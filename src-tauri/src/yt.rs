//! YouTube Music access via rustypipe, with yt-dlp as a fallback for stream URLs.
use crate::models::*;
use crate::ytdlp;
use anyhow::{Context, Result, anyhow, bail};
use rustypipe::client::RustyPipe;
use rustypipe::model::richtext::ToPlaintext;
use rustypipe::model::{
    AlbumItem, AlbumType, ArtistId, ArtistItem, AudioFormat, AudioStream, MusicPlaylistItem,
    Thumbnail, TrackItem,
};
use std::collections::HashMap;
use std::path::PathBuf;
use futures_util::FutureExt;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Audio codec of a YouTube stream. AAC (m4a) plays everywhere; Opus (webm) sounds
/// better at the same bitrate (~160 kbps vs 128 kbps AAC).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Aac,
    Opus,
}

impl Codec {
    fn from_mime(mime: &str) -> Self {
        if mime.contains("opus") || mime.contains("webm") { Codec::Opus } else { Codec::Aac }
    }
    fn mime_prefix(self) -> &'static str {
        match self {
            Codec::Aac => "audio/mp4",
            Codec::Opus => "audio/webm",
        }
    }
    fn ytdlp_format(self) -> &'static str {
        match self {
            Codec::Aac => "bestaudio[ext=m4a]/bestaudio",
            Codec::Opus => "bestaudio[acodec=opus]/bestaudio",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Codec::Aac => "AAC",
            Codec::Opus => "Opus",
        }
    }
}

#[derive(Clone)]
pub struct StreamInfo {
    pub url: String,
    pub mime: String,
    pub codec: Codec,
    pub kbps: Option<u32>,
    pub size: Option<u64>,
    pub expires: Instant,
}

pub struct Yt {
    pub rp: RustyPipe,
    http: reqwest::Client,
    streams: Mutex<HashMap<(String, Codec), StreamInfo>>,
    visitor: Mutex<Option<(String, Instant)>>,
    /// Second handle on the encrypted session cache, to read the cookie for account info.
    session: crate::secure_store::SecureStorage,
    pub ytdlp_dir: PathBuf,
    /// At most a few yt-dlp processes at once (prefetches must not starve the track you clicked).
    ytdlp_slots: tokio::sync::Semaphore,
    /// One resolve per track id at a time; a second caller waits and reuses the cached URL.
    inflight: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

// ------------------------------------------------------------------ conversions
pub fn best_thumb(thumbs: &[Thumbnail]) -> Option<String> {
    let t = thumbs.iter().max_by_key(|t| t.width * t.height)?;
    Some(upscale(&t.url))
}

/// googleusercontent covers carry their size in the URL (`=w120-h120-...`); ask for a bigger one.
fn upscale(url: &str) -> String {
    if (url.contains("googleusercontent.com") || url.contains("ggpht.com"))
        && let Some(i) = url.rfind("=w")
    {
        return format!("{}=w544-h544-l90-rj", &url[..i]);
    }
    url.to_string()
}

fn artists(a: &[ArtistId]) -> Vec<ArtistRef> {
    a.iter()
        .map(|a| ArtistRef { id: a.id.clone(), name: a.name.clone() })
        .collect()
}

pub fn track(t: TrackItem) -> Track {
    Track {
        cover: best_thumb(&t.cover),
        artists: artists(&t.artists),
        album: t.album.map(|a| AlbumRef { id: a.id, name: a.name }),
        duration: t.duration,
        plays: t.view_count,
        title: t.name,
        id: t.id,
    }
}

fn album_kind(k: AlbumType) -> &'static str {
    match k {
        AlbumType::Ep => "ep",
        AlbumType::Single => "single",
        AlbumType::Audiobook => "audiobook",
        AlbumType::Show => "show",
        _ => "album",
    }
}

fn album(a: AlbumItem) -> AlbumCard {
    AlbumCard {
        cover: best_thumb(&a.cover),
        artists: artists(&a.artists),
        year: a.year,
        kind: album_kind(a.album_type).to_string(),
        title: a.name,
        id: a.id,
    }
}

fn artist(a: ArtistItem) -> ArtistCard {
    ArtistCard {
        cover: best_thumb(&a.avatar),
        subscribers: a.subscriber_count,
        name: a.name,
        id: a.id,
    }
}

pub fn playlist_card(p: MusicPlaylistItem) -> PlaylistCard {
    PlaylistCard {
        cover: best_thumb(&p.thumbnail),
        author: p.channel.map(|c| c.name),
        count: p.track_count,
        title: p.name,
        id: p.id,
    }
}

fn pick_audio(streams: &[AudioStream], codec: Codec) -> Option<&AudioStream> {
    let usable = || {
        streams
            .iter()
            .filter(|s| s.drm_systems.is_empty())
            // multi-language videos: skip dubbed tracks
            .filter(|s| s.track.as_ref().is_none_or(|t| t.is_default))
    };
    let wanted = match codec {
        Codec::Aac => AudioFormat::M4a,
        Codec::Opus => AudioFormat::Webm,
    };
    usable()
        .filter(|s| s.format == wanted)
        .max_by_key(|s| s.bitrate)
        .or_else(|| usable().max_by_key(|s| s.bitrate))
}

/// Parse a two-letter country code into rustypipe's `Country` (serialized as e.g. "US").
fn serde_plain_country(code: &str) -> Result<rustypipe::param::Country> {
    Ok(serde_json::from_value(serde_json::Value::String(code.to_uppercase()))?)
}

impl Yt {
    /// `session_key` encrypts rustypipe's cache (it holds the YouTube login session).
    pub fn new(storage: PathBuf, ytdlp_dir: PathBuf, session_key: &crate::secrets::Key) -> Result<Self> {
        std::fs::create_dir_all(&storage)?;
        let secure = crate::secure_store::SecureStorage::new(storage.clone(), session_key);
        let session = crate::secure_store::SecureStorage::new(storage.clone(), session_key);
        let mut b = RustyPipe::builder()
            .storage_dir(storage)
            .storage(Box::new(secure))
            .timeout(Duration::from_secs(20));
        // YTUBEM_COUNTRY (e.g. "US") overrides the region used for charts/new releases.
        if let Ok(c) = std::env::var("YTUBEM_COUNTRY")
            && let Ok(country) = serde_plain_country(&c)
        {
            b = b.country(country);
        }
        let rp = b.build()?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
        Ok(Self {
            rp,
            http,
            streams: Mutex::new(HashMap::new()),
            visitor: Mutex::new(None),
            session,
            ytdlp_dir,
            ytdlp_slots: tokio::sync::Semaphore::new(3),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    pub async fn search(&self, q: &str) -> Result<SearchResults> {
        let qr = self.rp.query();
        let (tracks, albums, artists, playlists) = tokio::join!(
            qr.music_search_tracks(q),
            qr.music_search_albums(q),
            qr.music_search_artists(q),
            qr.music_search_playlists(q, false),
        );
        // Tracks are the critical part; other categories degrade gracefully.
        let tracks = tracks.context("track search failed")?;
        Ok(SearchResults {
            tracks: tracks.items.items.into_iter().map(track).collect(),
            albums: albums
                .map(|r| r.items.items.into_iter().map(album).take(12).collect())
                .unwrap_or_default(),
            artists: artists
                .map(|r| r.items.items.into_iter().map(artist).take(12).collect())
                .unwrap_or_default(),
            playlists: playlists
                .map(|r| r.items.items.into_iter().map(playlist_card).take(12).collect())
                .unwrap_or_default(),
        })
    }

    pub async fn suggestions(&self, q: &str) -> Result<Vec<String>> {
        Ok(self.rp.query().music_search_suggestion(q).await?.terms)
    }

    /// Home feed. Charts follow chart_region (e.g. "US"); new releases follow YouTube's own geo.
    pub async fn home(&self, chart_region: &str) -> Result<Home> {
        let qr = self.rp.query();
        let (charts, new) = tokio::join!(qr.music_charts(serde_plain_country(chart_region).ok()), qr.music_new_albums());
        let mut sections = Vec::new();
        if let Ok(n) = new
            && !n.is_empty()
        {
            sections.push(HomeSection {
                title: "new_releases".into(),
                albums: n.into_iter().take(12).map(album).collect(),
                ..Default::default()
            });
        }
        if let Ok(c) = charts {
            if !c.top_tracks.is_empty() {
                sections.push(HomeSection {
                    title: "top_tracks".into(),
                    tracks: c.top_tracks.into_iter().map(track).collect(),
                    ..Default::default()
                });
            }
            if !c.trending_tracks.is_empty() {
                sections.push(HomeSection {
                    title: "trending".into(),
                    tracks: c.trending_tracks.into_iter().map(track).collect(),
                    ..Default::default()
                });
            }
            if !c.artists.is_empty() {
                sections.push(HomeSection {
                    title: "popular_artists".into(),
                    artists: c.artists.into_iter().take(12).map(artist).collect(),
                    ..Default::default()
                });
            }
            if !c.playlists.is_empty() {
                sections.push(HomeSection {
                    title: "charts".into(),
                    playlists: c.playlists.into_iter().take(12).map(playlist_card).collect(),
                    ..Default::default()
                });
            }
        }
        if sections.is_empty() {
            return Err(anyhow!("can't reach YouTube Music"));
        }
        Ok(Home { sections })
    }

    pub async fn album(&self, id: &str) -> Result<AlbumPage> {
        let a = self.rp.query().music_album(id).await?;
        let cover = best_thumb(&a.cover);
        let album_ref = AlbumRef { id: a.id.clone(), name: a.name.clone() };
        let tracks = a
            .tracks
            .into_iter()
            .map(|t| {
                let mut t = track(t);
                // album tracks come without their own cover/album; fill them in
                if t.cover.is_none() {
                    t.cover = cover.clone();
                }
                if t.album.is_none() {
                    t.album = Some(album_ref.clone());
                }
                t
            })
            .collect();
        Ok(AlbumPage {
            title: a.name,
            artists: artists(&a.artists),
            year: a.year,
            kind: album_kind(a.album_type).to_string(),
            description: a.description.map(|d| d.to_plaintext()).filter(|s| !s.is_empty()),
            tracks,
            cover,
            id: a.id,
        })
    }

    pub async fn artist(&self, id: &str) -> Result<ArtistPage> {
        let a = self.rp.query().music_artist(id, false).await?;
        let (singles, albums): (Vec<_>, Vec<_>) = a
            .albums
            .into_iter()
            .partition(|x| matches!(x.album_type, AlbumType::Single | AlbumType::Ep));
        Ok(ArtistPage {
            cover: best_thumb(&a.header_image),
            subscribers: a.subscriber_count,
            description: a.description,
            tracks: a.tracks.into_iter().map(track).collect(),
            albums: albums.into_iter().map(album).collect(),
            singles: singles.into_iter().map(album).collect(),
            playlists: a.playlists.into_iter().map(playlist_card).collect(),
            similar: a.similar_artists.into_iter().map(artist).collect(),
            name: a.name,
            id: a.id,
        })
    }

    pub async fn playlist(&self, id: &str, authenticated: bool) -> Result<PlaylistPage> {
        let mut qr = self.rp.query();
        if authenticated {
            qr = qr.authenticated();
        }
        let mut p = qr.music_playlist(id).await?;
        p.tracks.extend_limit(&qr, 500).await.ok();
        Ok(PlaylistPage {
            cover: best_thumb(&p.thumbnail),
            author: p.channel.map(|c| c.name),
            description: p.description.map(|d| d.to_plaintext()).filter(|s| !s.is_empty()),
            tracks: p.tracks.items.into_iter().map(track).collect(),
            title: p.name,
            id: p.id,
        })
    }

    /// Plain YouTube playlist endpoint: works for private/personal playlists that the
    /// YouTube Music parser in rustypipe currently chokes on.
    pub async fn playlist_yt(&self, id: &str, authenticated: bool) -> Result<PlaylistPage> {
        let mut qr = self.rp.query();
        if authenticated {
            qr = qr.authenticated();
        }
        let mut p = qr.playlist(id).await?;
        p.videos.extend_limit(&qr, 500).await.ok();
        Ok(PlaylistPage {
            cover: best_thumb(&p.thumbnail),
            author: p.channel.map(|c| c.name),
            description: p.description.map(|d| d.to_plaintext()).filter(|s| !s.is_empty()),
            tracks: p
                .videos
                .items
                .into_iter()
                .map(|v| Track {
                    cover: best_thumb(&v.thumbnail),
                    artists: v
                        .channel
                        .map(|c| vec![ArtistRef { id: Some(c.id), name: c.name.trim_end_matches(" - Topic").to_string() }])
                        .unwrap_or_default(),
                    album: None,
                    duration: v.duration,
                    plays: v.view_count,
                    title: v.name,
                    id: v.id,
                })
                .collect(),
            title: p.name,
            id: p.id,
        })
    }

    /// Load a playlist trying the fastest working route first, each attempt isolated from
    /// rustypipe panics: YT Music without auth (public) → with auth → plain YouTube API.
    pub async fn playlist_any(&self, id: &str, logged_in: bool) -> Result<PlaylistPage> {
        async fn guarded<F: Future<Output = Result<PlaylistPage>>>(f: F) -> Result<PlaylistPage> {
            std::panic::AssertUnwindSafe(f).catch_unwind().await.unwrap_or_else(|_| Err(anyhow!("rustypipe panicked")))
        }
        let mut last = match guarded(self.playlist(id, false)).await {
            Ok(p) if !p.tracks.is_empty() => return Ok(p),
            Ok(_) => anyhow!("empty playlist"),
            Err(e) => e,
        };
        let attempts: Vec<(bool, bool)> = if logged_in {
            vec![(true, true), (false, true), (false, false)] // (music api?, auth?)
        } else {
            vec![(false, false)]
        };
        for (music, auth) in attempts {
            let r = if music { guarded(self.playlist(id, auth)).await } else { guarded(self.playlist_yt(id, auth)).await };
            match r {
                Ok(p) if !p.tracks.is_empty() => return Ok(p),
                Ok(_) => {}
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub async fn radio(&self, id: &str) -> Result<Vec<Track>> {
        let qr = self.rp.query();
        let mut p = qr.music_radio_track(id).await?;
        p.extend_limit(&qr, 50).await.ok();
        Ok(p.items.into_iter().map(track).collect())
    }

    pub async fn lyrics(&self, id: &str) -> Result<Option<LyricsOut>> {
        let qr = self.rp.query();
        let d = qr.music_details(id).await?;
        let Some(lid) = d.lyrics_id else { return Ok(None) };
        match qr.music_lyrics(lid).await {
            Ok(l) => Ok(Some(LyricsOut { body: l.body, footer: l.footer })),
            Err(rustypipe::error::Error::Extraction(
                rustypipe::error::ExtractionError::NotFound { .. },
            )) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // ------------------------------------------------------------ account
    pub async fn login(&self, text: &str) -> Result<()> {
        let text = text.trim();
        let text = text.strip_prefix("Cookie:").map(str::trim).unwrap_or(text);
        if text.contains('\t') {
            self.rp.user_auth_set_cookie_txt(text).await?;
        } else {
            self.rp.user_auth_set_cookie(text).await?;
        }
        Ok(())
    }

    /// Cookie header of the signed-in session (from the encrypted cache), if any.
    pub fn session_cookie(&self) -> Option<String> {
        use rustypipe::cache::CacheStorage;
        crate::account::cookie_from_cache(&self.session.read()?)
    }

    pub async fn logout(&self) -> Result<()> {
        self.rp.user_auth_remove_cookie().await.ok();
        Ok(())
    }

    pub async fn liked_tracks(&self) -> Result<Vec<Track>> {
        let qr = self.rp.query().authenticated();
        let mut p = qr.music_liked_tracks().await?;
        p.tracks.extend_limit(&qr, 5000).await.ok();
        Ok(p.tracks.items.into_iter().map(track).collect())
    }

    pub async fn saved_playlists(&self) -> Result<Vec<PlaylistCard>> {
        let qr = self.rp.query().authenticated();
        let mut p = qr.music_saved_playlists().await?;
        p.extend_limit(&qr, 500).await.ok();
        Ok(p.items
            .into_iter()
            // "LM" (liked music) is synced into local likes separately; "SE" (episodes for
            // later) is a podcast queue YouTube doesn't let clients view.
            .filter(|p| p.id != "LM" && p.id != "SE")
            .map(playlist_card)
            .collect())
    }

    // ------------------------------------------------------------ streams
    pub fn invalidate(&self, id: &str) {
        self.streams.lock().unwrap().retain(|(k, _), _| k != id);
    }

    fn cached(&self, key: &(String, Codec)) -> Option<StreamInfo> {
        self.streams.lock().unwrap().get(key).filter(|s| s.expires > Instant::now()).cloned()
    }

    /// Resolve an audio URL in the requested codec (falls back to whatever exists),
    /// cached until shortly before it expires.
    ///
    /// Desktop: every track goes through yt-dlp, the one path that plays everything
    /// (18+ songs included, signed in), so there is no switching between methods.
    /// The next tracks are resolved ahead of time, so skipping doesn't wait.
    /// Android (no yt-dlp): the pure-Rust resolvers.
    pub async fn stream(&self, id: &str, codec: Codec) -> Result<StreamInfo> {
        let key = (id.to_string(), codec);
        if let Some(s) = self.cached(&key) {
            return Ok(s);
        }
        let lock = self.inflight.lock().unwrap().entry(id.to_string()).or_default().clone();
        let _guard = lock.lock().await;
        if let Some(s) = self.cached(&key) {
            return Ok(s);
        }
        let resolved = if cfg!(desktop) && ytdlp::find(&self.ytdlp_dir).is_some() {
            match self.stream_ytdlp(id, codec).await {
                Ok(s) => s,
                Err(e) => {
                    // Only if yt-dlp itself is broken (e.g. mid-update): keep the music going.
                    eprintln!("yt-dlp failed for {id}: {e:#}; using the built-in client");
                    self.stream_native_race(id, codec).await.context("couldn't get an audio stream")?
                }
            }
        } else {
            self.stream_native_race(id, codec).await.context("couldn't get an audio stream")?
        };
        self.streams.lock().unwrap().insert(key, resolved.clone());
        self.inflight.lock().unwrap().remove(id);
        Ok(resolved)
    }

    /// The pure-Rust resolvers: visionOS innertube and rustypipe race; first *verified* URL wins.
    async fn stream_native_race(&self, id: &str, codec: Codec) -> Result<StreamInfo> {
        type Fut<'a> = Pin<Box<dyn Future<Output = Result<StreamInfo>> + Send + 'a>>;
        // Pure-Rust resolvers first (~0.1 s); yt-dlp (~3 s, desktop only) is the safety net
        // for when YouTube breaks them and yt-dlp has already caught up.
        // rustypipe can panic internally (e.g. unwrap on a failed visitor-id fetch);
        // catch that so it only counts as a failed resolver, not a dropped request.
        let rp_guarded = async {
            match std::panic::AssertUnwindSafe(self.stream_rustypipe(id, codec)).catch_unwind().await {
                Ok(r) => r,
                Err(_) => Err(anyhow!("rustypipe panicked")),
            }
        };
        let native: Vec<Fut> = vec![Box::pin(self.stream_native(id, codec)), Box::pin(rp_guarded)];
        Ok(futures_util::future::select_ok(native).await?.0)
    }

    async fn stream_rustypipe(&self, id: &str, codec: Codec) -> Result<StreamInfo> {
        let p = self.rp.query().player(id).await?;
        let s = pick_audio(&p.audio_streams, codec).ok_or_else(|| anyhow!("no audio stream"))?;
        // Without a PO token YouTube serves only the first ~1 MB and then 403s,
        // which would cut playback mid-track. Probe the tail before trusting the URL.
        if s.size > 1_200_000 {
            let tail = format!("bytes={}-{}", s.size - 4096, s.size - 1);
            let st = self.http.get(&s.url).header(reqwest::header::RANGE, tail).send().await?.status();
            if st != reqwest::StatusCode::PARTIAL_CONTENT {
                bail!("rustypipe: truncated stream ({st})");
            }
        }
        let ttl = (p.expires_in_seconds as u64).saturating_sub(600).max(300);
        let mime = s.mime.split(';').next().unwrap_or("audio/mp4").to_string();
        Ok(StreamInfo {
            url: s.url.clone(),
            codec: Codec::from_mime(&s.mime),
            kbps: Some(s.average_bitrate.max(s.bitrate) / 1000).filter(|&k| k > 0),
            mime,
            size: Some(s.size).filter(|&n| n > 0),
            expires: Instant::now() + Duration::from_secs(ttl),
        })
    }

    /// YouTube visitor id for anonymous InnerTube requests, cached for 6 hours.
    /// Fetched directly (rustypipe's own fetch panics when the request fails).
    async fn visitor_id(&self) -> Option<String> {
        if let Some((v, at)) = self.visitor.lock().unwrap().clone()
            && at.elapsed() < Duration::from_secs(6 * 3600)
        {
            return Some(v);
        }
        let body = serde_json::json!({ "context": { "client": {
            "clientName": "WEB", "clientVersion": "2.20250101.00.00", "hl": "en", "gl": "US" } } });
        let resp: serde_json::Value = self
            .http
            .post("https://www.youtube.com/youtubei/v1/visitor_id?prettyPrint=false")
            .header("X-YouTube-Client-Name", "1")
            .header("X-YouTube-Client-Version", "2.20250101.00.00")
            .header("Origin", "https://www.youtube.com")
            .json(&body)
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        let v = resp["responseContext"]["visitorData"].as_str()?.to_string();
        *self.visitor.lock().unwrap() = Some((v.clone(), Instant::now()));
        Some(v)
    }

    /// Direct innertube `player` call with the visionOS client — the same trick yt-dlp uses.
    /// This client needs no JS player (no signature/n deobfuscation) and no PO token,
    /// so it works in pure Rust everywhere, including Android.
    async fn stream_native(&self, id: &str, codec: Codec) -> Result<StreamInfo> {
        self.stream_native_as(id, codec, None).await
    }

    /// Same request signed as the logged-in user (cookies + SAPISIDHASH): needed for
    /// age-restricted songs, which YouTube only serves to signed-in, age-verified accounts.
    pub async fn stream_native_as(&self, id: &str, codec: Codec, cookie: Option<&str>) -> Result<StreamInfo> {
        const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
        let visitor = self.visitor_id().await.unwrap_or_default();
        let body = serde_json::json!({
            "context": { "client": {
                "clientName": "VISIONOS",
                "clientVersion": "1.02",
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "hl": "en",
                "gl": "US",
                "visitorData": visitor,
            }},
            "videoId": id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        let mut req = self
            .http
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .header("X-YouTube-Client-Name", "101")
            .header("X-YouTube-Client-Version", "1.02")
            .header("Origin", "https://www.youtube.com")
            .header(reqwest::header::USER_AGENT, UA);
        if !visitor.is_empty() {
            req = req.header("X-Goog-Visitor-Id", &visitor);
        }
        if let Some(c) = cookie
            && let Some(auth) = crate::account::authorization(c, "https://www.youtube.com")
        {
            req = req.header("Cookie", c).header("Authorization", auth).header("X-Goog-AuthUser", "0");
        }
        let resp: serde_json::Value = req
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let status = resp["playabilityStatus"]["status"].as_str().unwrap_or("");
        if status != "OK" {
            let reason = resp["playabilityStatus"]["reason"].as_str().unwrap_or("unknown");
            if status == "LOGIN_REQUIRED" {
                // a stale visitor id can trigger the bot check; get a fresh one next time
                *self.visitor.lock().unwrap() = None;
            }
            bail!("visionOS: {status}: {reason}");
        }
        let formats = resp["streamingData"]["adaptiveFormats"].as_array().ok_or_else(|| anyhow!("no formats"))?;
        let audio = formats.iter().filter(|f| {
            f["mimeType"].as_str().is_some_and(|m| m.starts_with("audio/"))
                && f["url"].is_string()
                && f["drmFamilies"].is_null()
                // skip dubbed tracks on multi-language videos
                && f["audioTrack"]["audioIsDefault"].as_bool().unwrap_or(true)
        });
        let bitrate = |f: &&serde_json::Value| f["bitrate"].as_u64().unwrap_or(0);
        let wanted = codec.mime_prefix();
        let best = audio
            .clone()
            .filter(|f| f["mimeType"].as_str().is_some_and(|m| m.starts_with(wanted)))
            .max_by_key(bitrate)
            .or_else(|| audio.max_by_key(bitrate))
            .ok_or_else(|| anyhow!("visionOS: no audio stream"))?;
        let ttl = resp["streamingData"]["expiresInSeconds"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(21540)
            .saturating_sub(600)
            .max(300);
        let mime_full = best["mimeType"].as_str().unwrap_or("audio/mp4");
        let avg = best["averageBitrate"].as_u64().or_else(|| best["bitrate"].as_u64());
        Ok(StreamInfo {
            url: best["url"].as_str().unwrap_or_default().to_string(),
            mime: mime_full.split(';').next().unwrap_or("audio/mp4").to_string(),
            codec: Codec::from_mime(mime_full),
            kbps: avg.map(|b| (b / 1000) as u32),
            size: best["contentLength"].as_str().and_then(|s| s.parse().ok()),
            expires: Instant::now() + Duration::from_secs(ttl),
        })
    }

    /// yt-dlp fallback. Signed in when a session exists: that's what makes age-restricted
    /// (18+) songs play, which YouTube only serves to signed-in, age-verified accounts.
    async fn stream_ytdlp(&self, id: &str, codec: Codec) -> Result<StreamInfo> {
        let _slot = self.ytdlp_slots.acquire().await?;
        let url = match self.session_cookie() {
            Some(cookie) => {
                let tmp = self.ytdlp_dir.parent().map(|p| p.join("tmp")).unwrap_or_else(std::env::temp_dir);
                match ytdlp::get_url_signed_in(&self.ytdlp_dir, &tmp, id, codec.ytdlp_format(), &cookie).await {
                    Ok(u) => u,
                    Err(e) => {
                        eprintln!("signed-in yt-dlp failed for {id}: {e:#}; trying anonymously");
                        ytdlp::get_url(&self.ytdlp_dir, id, codec.ytdlp_format()).await?
                    }
                }
            }
            None => ytdlp::get_url(&self.ytdlp_dir, id, codec.ytdlp_format()).await?,
        };
        let expires = Instant::now() + url_ttl(&url);
        let mime = if url.contains("mime=audio%2Fwebm") { "audio/webm" } else { "audio/mp4" };
        Ok(StreamInfo {
            url,
            codec: Codec::from_mime(mime),
            kbps: None,
            mime: mime.into(),
            size: None,
            expires,
        })
    }
}

/// How long a googlevideo URL stays valid: its `expire=` timestamp minus a safety margin.
fn url_ttl(url: &str) -> Duration {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    url.split(['?', '&'])
        .find_map(|p| p.strip_prefix("expire="))
        .and_then(|v| v.parse::<u64>().ok())
        .map(|exp| Duration::from_secs(exp.saturating_sub(now).saturating_sub(600).max(300)))
        .unwrap_or(Duration::from_secs(3 * 3600))
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Hits real YouTube. Run with: cargo test live -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_smoke() {
        let dir = std::env::temp_dir().join("ytubem-test");
        let yt = Yt::new(dir.join("rp"), dir.join("bin"), &[0u8; 32]).unwrap();
        let r = yt.search("Океан Ельзи").await.unwrap();
        println!("tracks={} albums={} artists={} playlists={}", r.tracks.len(), r.albums.len(), r.artists.len(), r.playlists.len());
        let t = &r.tracks[0];
        println!("first: {} ({:?})", t.title, t.duration);
        println!("suggest: {:?}", yt.suggestions("океан").await.unwrap());
        let a = yt.album(&r.albums[0].id).await.unwrap();
        println!("album: {} tracks={}", a.title, a.tracks.len());
        let ar = yt.artist(&r.artists[0].id).await.unwrap();
        println!("artist: {} top={} albums={} singles={}", ar.name, ar.tracks.len(), ar.albums.len(), ar.singles.len());
        println!("radio: {}", yt.radio(&t.id).await.unwrap().len());
        println!("lyrics: {}", yt.lyrics(&t.id).await.unwrap().is_some());
        let home = yt.home("US").await.unwrap();
        println!("home: {:?}", home.sections.iter().map(|s| &s.title).collect::<Vec<_>>());
        let pl = yt.playlist(&r.playlists[0].id, false).await.unwrap();
        println!("playlist: {} tracks={}", pl.title, pl.tracks.len());

        let start = Instant::now();
        let s = yt.stream(&t.id, Codec::Aac).await.unwrap();
        println!("stream mime={} size={:?} in {:?}", s.mime, s.size, start.elapsed());
        let start = Instant::now();
        yt.stream(&t.id, Codec::Aac).await.unwrap();
        println!("cached in {:?}", start.elapsed());
    }
}
#[cfg(test)]
mod native_tests {
    use super::*;
    #[tokio::test]
    #[ignore]
    async fn live_native_stream() {
        let dir = std::env::temp_dir().join("ytubem-test");
        let yt = Yt::new(dir.join("rp"), dir.join("bin"), &[0u8; 32]).unwrap();
        let c = reqwest::Client::new();
        for id in ["8wWrzr3_46Y", "1BdGqKO3ERY", "dQw4w9WgXcQ"] {
            let t = Instant::now();
            match yt.stream_native(id, Codec::Opus).await {
                Ok(s) => {
                    let el = t.elapsed();
                    let size = s.size.unwrap_or(0);
                    let open = c.get(&s.url).header("Range", "bytes=0-").send().await.unwrap().status();
                    let tail = c.get(&s.url).header("Range", format!("bytes={}-{}", size - 4096, size - 1)).send().await.unwrap().status();
                    println!("{id}: {} size={size} in {el:?} open={open} tail={tail}", s.mime);
                }
                Err(e) => println!("{id}: ERROR {e:#}"),
            }
        }
    }
}

#[cfg(test)]
mod debug_user_session {
    /// Reproduce issues with the real signed-in session (read-only requests only).
    /// Run: cargo test --lib debug_user_session -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn debug_user_session() {
        let data = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("io.github.neverh8me.ytmusic");
        let key = crate::secrets::get_or_create(&data, "session", None).unwrap();
        let yt = super::Yt::new(data.join("rustypipe"), data.join("bin"), &key).unwrap();
        let db = crate::db::Db::open(&data.join("library.db")).unwrap();
        let cookie = yt.session_cookie();
        println!("session cookie present: {}", cookie.is_some());
        if let Some(c) = &cookie {
            match crate::account::fetch(&reqwest::Client::new(), c).await {
                Ok(a) => println!("account: name ok ({} chars), avatar: {}", a.name.chars().count(), a.avatar.is_some()),
                Err(e) => println!("account: ERR {e:#}"),
            }
        }
        let pls: Vec<crate::models::PlaylistCard> =
            serde_json::from_str(&db.get_setting("remote_playlists").unwrap_or("[]".into())).unwrap();
        for p in pls {
            let t = std::time::Instant::now();
            let r = yt.playlist_any(&p.id, true).await;
            println!("playlist {:<12}: {} in {:?}", &p.id[..p.id.len().min(12)],
                match &r { Ok(x) => format!("OK {} tracks", x.tracks.len()), Err(e) => format!("ERR {e:#}").chars().take(100).collect() },
                t.elapsed());
        }
        let r = yt.search("так похуй").await.unwrap();
        for t in r.tracks.iter().take(3) {
            let s = crate::ytdlp::get_url_signed_in(&data.join("bin"), &std::env::temp_dir(), &t.id, "bestaudio[ext=m4a]/bestaudio", cookie.as_deref().unwrap()).await;
            println!("stream {} [{}]: {}", t.title, t.id, match s { Ok(u) => format!("OK url len {}", u.len()), Err(e) => format!("ERR {e:#}").chars().take(300).collect() });
        }
    }
}