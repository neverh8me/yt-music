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
    /// Tracks whose fast (built-in) stream URL died while playing: resolve these with yt-dlp.
    distrust: Mutex<std::collections::HashSet<String>>,
    /// Tracks resolved by the fast path that have no yt-dlp backup yet (see backup()).
    fast: Mutex<std::collections::HashSet<String>>,
    /// Format (itag) each track started playing in: a replacement URL must be the same file,
    /// or the bytes already played wouldn't line up with the rest.
    itags: Mutex<HashMap<String, String>>,
    /// YouTube player challenge solver (signed-in playback without yt-dlp, e.g. Android).
    solver: crate::jsplayer::Solver,
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

/// Songs only: the app plays music, not music videos (or podcast episodes). Applied to what
/// the app picks (search, charts, artist pages, radio), not to the user's own playlists.
fn song(t: &TrackItem) -> bool {
    t.track_type.is_track()
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
            .storage_dir(&storage)
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
        let solver_dir = storage.parent().map(|p| p.join("jsplayer")).unwrap_or_else(|| storage.join("jsplayer"));
        let solver = crate::jsplayer::Solver::new(reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?, solver_dir);
        Ok(Self {
            rp,
            http,
            streams: Mutex::new(HashMap::new()),
            visitor: Mutex::new(None),
            session,
            ytdlp_dir,
            ytdlp_slots: tokio::sync::Semaphore::new(3),
            inflight: Mutex::new(HashMap::new()),
            distrust: Mutex::new(std::collections::HashSet::new()),
            fast: Mutex::new(std::collections::HashSet::new()),
            itags: Mutex::new(HashMap::new()),
            solver,
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
            tracks: tracks.items.items.into_iter().filter(song).map(track).collect(),
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
                    tracks: c.top_tracks.into_iter().filter(song).map(track).collect(),
                    ..Default::default()
                });
            }
            if !c.trending_tracks.is_empty() {
                sections.push(HomeSection {
                    title: "trending".into(),
                    tracks: c.trending_tracks.into_iter().filter(song).map(track).collect(),
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
            tracks: a.tracks.into_iter().filter(song).map(track).collect(),
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
        // whole playlist (YouTube Music itself caps playlists at 5000 songs)
        p.tracks.extend_limit(&qr, 5000).await.ok();
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
        p.videos.extend_limit(&qr, 5000).await.ok();
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
        Ok(p.items.into_iter().filter(song).map(track).collect())
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

    /// Get YouTube's current player parsed for the challenge solver ahead of time, so the first
    /// signed-in (18+) song on Android doesn't wait for it.
    pub async fn warm_solver(&self) -> Result<()> {
        self.solver.player().await.map(|_| ())
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
    /// The stream server calls this when a URL turned out dead or forbidden: forget it, and
    /// resolve this track the slow, sure way (yt-dlp) next time.
    pub fn invalidate(&self, id: &str) {
        self.streams.lock().unwrap().retain(|(k, _), _| k != id);
        let mut d = self.distrust.lock().unwrap();
        if d.len() > 500 {
            d.clear();
        }
        d.insert(id.to_string());
    }

    fn cached(&self, key: &(String, Codec)) -> Option<StreamInfo> {
        self.streams.lock().unwrap().get(key).filter(|s| s.expires > Instant::now()).cloned()
    }

    /// Resolve an audio URL in the requested codec (falls back to whatever exists),
    /// cached until shortly before it expires.
    ///
    /// Fast start, sure finish. The built-in resolvers answer in about a second, so a track
    /// starts from them: the anonymous clients, then (sign-in-only songs such as 18+) a signed
    /// request as your account with the player challenges solved in the app. Their URLs can
    /// die mid-song, so on desktop the track that is actually playing also gets a yt-dlp URL
    /// for the *same* format in the background (see backup()), which quietly replaces the
    /// fast one; and if the fast URL dies first, yt-dlp resolves that same format, so the
    /// song goes on from where it was. yt-dlp is also the fallback when the fast path fails.
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
        let ytdlp = cfg!(desktop) && ytdlp::find(&self.ytdlp_dir).is_some();
        let sure_first = ytdlp && self.distrust.lock().unwrap().contains(id);
        let fast = async {
            match self.stream_native_race(id, codec).await {
                Ok(s) => Ok(s),
                Err(e) => match self.session_cookie() {
                    Some(cookie) => self.stream_signed(id, codec, &cookie).await.with_context(|| format!("{e:#}")),
                    None => Err(e),
                },
            }
        };
        let resolved = if sure_first {
            let itag = self.itags.lock().unwrap().get(id).cloned();
            match self.stream_ytdlp(id, codec, itag.as_deref()).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("yt-dlp failed for {id}: {e:#}; using the built-in clients");
                    fast.await.context("couldn't get an audio stream")?
                }
            }
        } else {
            match fast.await {
                Ok(s) => {
                    if ytdlp && let Some(itag) = url_param(&s.url, "itag") {
                        let mut f = self.fast.lock().unwrap();
                        let mut t = self.itags.lock().unwrap();
                        if f.len() > 500 {
                            f.clear();
                        }
                        if t.len() > 500 {
                            t.clear();
                        }
                        f.insert(id.to_string());
                        t.insert(id.to_string(), itag);
                    }
                    s
                }
                Err(e) if ytdlp => {
                    eprintln!("built-in clients failed for {id}: {e:#}; using yt-dlp");
                    self.stream_ytdlp(id, codec, None).await.context("couldn't get an audio stream")?
                }
                Err(e) => return Err(e.context("couldn't get an audio stream")),
            }
        };
        self.streams.lock().unwrap().insert(key, resolved.clone());
        self.inflight.lock().unwrap().remove(id);
        Ok(resolved)
    }

    /// Signed-in player request with clients that accept the account session (TV, embedded),
    /// then the "n" and signature challenges solved from YouTube's player JS
    /// (see jsplayer.rs). This is what plays 18+ songs where yt-dlp isn't available.
    pub async fn stream_signed(&self, id: &str, codec: Codec, cookie: &str) -> Result<StreamInfo> {
        let player = self.solver.player().await.context("YouTube player")?;
        let visitor = self.visitor_id().await.unwrap_or_default();
        let auth = crate::account::authorization(cookie, "https://www.youtube.com").ok_or_else(|| anyhow!("no SAPISID in the session"))?;
        // Client configs copied from yt-dlp's INNERTUBE_CLIENTS (they accept the account cookies):
        // (clientName, header id, version, user agent, extra client fields)
        const SAFARI: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15,gzip(gfe)";
        let clients: [(&str, &str, &str, &str, serde_json::Value); 3] = [
            // Premium accounts: no PO token needed
            ("WEB_CREATOR", "62", "1.20260708.06.00", SAFARI, serde_json::json!({})),
            ("TVHTML5", "7", "5.20260707",
             "Mozilla/5.0 (SMART-TV; Linux; Tizen 2.4.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/2.4.0 TV Safari/538.1",
             serde_json::json!({ "deviceMake": "Samsung", "deviceModel": "UKS9800", "osName": "Tizen", "osVersion": "2.4.0" })),
            ("TVHTML5", "7", "7.20260707.07.00",
             "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)",
             serde_json::json!({})),
        ];
        let mut errors: Vec<String> = vec![];
        for (name, num, version, ua, extra) in clients {
            let mut context = serde_json::json!({ "client": { "clientName": name, "clientVersion": version, "userAgent": ua, "hl": "en", "gl": "US" } });
            if let Some(m) = extra.as_object() {
                for (k, v) in m {
                    context["client"][k] = v.clone();
                }
            }
            if !visitor.is_empty() {
                context["client"]["visitorData"] = visitor.clone().into();
            }
            let body = serde_json::json!({
                "context": context,
                "videoId": id,
                "playbackContext": { "contentPlaybackContext": { "signatureTimestamp": player.sts, "html5Preference": "HTML5_PREF_WANTS" } },
                "contentCheckOk": true,
                "racyCheckOk": true,
            });
            let mut req = self
                .http
                .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
                .header("X-YouTube-Client-Name", num)
                .header("X-YouTube-Client-Version", version)
                .header("Origin", "https://www.youtube.com")
                .header("X-Origin", "https://www.youtube.com")
                .header(reqwest::header::USER_AGENT, ua)
                .header("Cookie", cookie)
                .header("Authorization", &auth)
                .header("X-Goog-AuthUser", "0");
            if !visitor.is_empty() {
                req = req.header("X-Goog-Visitor-Id", &visitor);
            }
            let resp: serde_json::Value = match req.json(&body).send().await.and_then(|r| r.error_for_status()) {
                Ok(r) => r.json().await?,
                Err(e) => { errors.push(format!("{name} {version}: {e}")); continue; }
            };
            let status = resp["playabilityStatus"]["status"].as_str().unwrap_or("");
            if status != "OK" {
                errors.push(format!("{name} {version}: {status}: {}", resp["playabilityStatus"]["reason"].as_str().unwrap_or("")));
                continue;
            }
            match self.pick_signed(&resp, codec, &player).await {
                Ok(s) => return Ok(s),
                Err(e) => errors.push(format!("{name}: {e:#}")),
            }
        }
        Err(anyhow!("{}", errors.join(" | ")))
    }

    /// Best audio format of a signed player response, with its URL deciphered.
    async fn pick_signed(&self, resp: &serde_json::Value, codec: Codec, player: &crate::jsplayer::Player) -> Result<StreamInfo> {
        let formats = resp["streamingData"]["adaptiveFormats"].as_array().ok_or_else(|| anyhow!("no formats"))?;
        let audio = formats.iter().filter(|f| {
            f["mimeType"].as_str().is_some_and(|m| m.starts_with("audio/"))
                && (f["url"].is_string() || f["signatureCipher"].is_string())
                && f["drmFamilies"].is_null()
                && f["audioTrack"]["audioIsDefault"].as_bool().unwrap_or(true)
        });
        let bitrate = |f: &&serde_json::Value| f["bitrate"].as_u64().unwrap_or(0);
        let best = audio
            .clone()
            .filter(|f| f["mimeType"].as_str().is_some_and(|m| m.starts_with(codec.mime_prefix())))
            .max_by_key(bitrate)
            .or_else(|| audio.max_by_key(bitrate))
            .ok_or_else(|| anyhow!("no audio stream"))?;
        // Plain URL, or a cipher: url + s (encrypted signature) + sp (its query name).
        let (base, sig, sp) = match best["url"].as_str() {
            Some(u) => (u.to_string(), None, String::new()),
            None => {
                let mut url = String::new();
                let mut s = None;
                let mut sp = "signature".to_string();
                for (k, v) in url::form_urlencoded::parse(best["signatureCipher"].as_str().unwrap_or("").as_bytes()) {
                    match k.as_ref() {
                        "url" => url = v.into_owned(),
                        "s" => s = Some(v.into_owned()),
                        "sp" => sp = v.into_owned(),
                        _ => {}
                    }
                }
                (url, s, sp)
            }
        };
        let mut u = url::Url::parse(&base)?;
        let n = u.query_pairs().find(|(k, _)| k == "n").map(|(_, v)| v.into_owned());
        let (nm, sm) = self
            .solver
            .solve(player, &n.iter().cloned().collect::<Vec<_>>(), &sig.iter().cloned().collect::<Vec<_>>())
            .await?;
        let pairs: Vec<(String, String)> = u
            .query_pairs()
            .map(|(k, v)| {
                let v = if k == "n" { nm.get(v.as_ref()).cloned().unwrap_or_else(|| v.into_owned()) } else { v.into_owned() };
                (k.into_owned(), v)
            })
            .collect();
        {
            let mut q = u.query_pairs_mut();
            q.clear();
            for (k, v) in &pairs {
                q.append_pair(k, v);
            }
            if let Some(s) = &sig {
                q.append_pair(&sp, sm.get(s).ok_or_else(|| anyhow!("signature not solved"))?);
            }
        }
        let url = u.to_string();
        let mime_full = best["mimeType"].as_str().unwrap_or("audio/mp4");
        let expires = Instant::now() + url_ttl(&url);
        Ok(StreamInfo {
            url,
            mime: mime_full.split(';').next().unwrap_or("audio/mp4").to_string(),
            codec: Codec::from_mime(mime_full),
            kbps: best["averageBitrate"].as_u64().or_else(|| best["bitrate"].as_u64()).map(|b| (b / 1000) as u32),
            size: best["contentLength"].as_str().and_then(|s| s.parse().ok()),
            expires,
        })
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
    /// True once per fast-resolved track: the caller (the stream server, i.e. the track is
    /// really being played) should then run backup() in the background.
    pub fn take_backup_job(&self, id: &str) -> bool {
        self.fast.lock().unwrap().remove(id)
    }

    /// Get a yt-dlp URL for the same format as the fast URL in the cache and put it in its
    /// place: later requests for this track (seeks, the next chunk after a hiccup) then use
    /// the sure URL. Different format → leave things alone (the bytes wouldn't line up).
    pub async fn backup(&self, id: &str, codec: Codec) {
        let key = (id.to_string(), codec);
        let Some(cur) = self.cached(&key) else { return };
        let Some(itag) = url_param(&cur.url, "itag") else { return };
        match self.stream_ytdlp(id, codec, Some(&itag)).await {
            Ok(mut s) if url_param(&s.url, "itag").as_deref() == Some(itag.as_str()) => {
                s.size = cur.size;
                s.kbps = cur.kbps;
                s.mime = cur.mime.clone();
                s.codec = cur.codec;
                let mut m = self.streams.lock().unwrap();
                if m.get(&key).is_some_and(|c| c.url == cur.url) {
                    m.insert(key, s);
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("yt-dlp backup for {id} failed: {e:#}"),
        }
    }

    /// `itag`: ask for exactly this format first (to continue a track that already plays).
    async fn stream_ytdlp(&self, id: &str, codec: Codec, itag: Option<&str>) -> Result<StreamInfo> {
        let format = match itag {
            Some(i) => format!("{i}/{}", codec.ytdlp_format()),
            None => codec.ytdlp_format().to_string(),
        };
        let format = format.as_str();
        let _slot = self.ytdlp_slots.acquire().await?;
        let url = match self.session_cookie() {
            Some(cookie) => {
                let tmp = self.ytdlp_dir.parent().map(|p| p.join("tmp")).unwrap_or_else(std::env::temp_dir);
                match ytdlp::get_url_signed_in(&self.ytdlp_dir, &tmp, id, format, &cookie).await {
                    Ok(u) => u,
                    Err(e) => {
                        eprintln!("signed-in yt-dlp failed for {id}: {e:#}; trying anonymously");
                        ytdlp::get_url(&self.ytdlp_dir, id, format).await?
                    }
                }
            }
            None => ytdlp::get_url(&self.ytdlp_dir, id, format).await?,
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

/// A query parameter of a stream URL (e.g. "itag").
pub fn url_param(url: &str, name: &str) -> Option<String> {
    url.split(['?', '&']).find_map(|p| p.strip_prefix(name)?.strip_prefix('=')).map(str::to_string)
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
        for p in pls.into_iter().take(0) {
            let t = std::time::Instant::now();
            let r = yt.playlist_any(&p.id, true).await;
            println!("playlist {:<12}: {} in {:?}", &p.id[..p.id.len().min(12)],
                match &r { Ok(x) => format!("OK {} tracks", x.tracks.len()), Err(e) => format!("ERR {e:#}").chars().take(100).collect() },
                t.elapsed());
        }
        // rustypipe can panic on a flaky visitor-id fetch; retry a few times
        let mut r = None;
        for _ in 0..5 {
            use futures_util::FutureExt;
            if let Ok(Ok(x)) = std::panic::AssertUnwindSafe(yt.search("так похуй")).catch_unwind().await { r = Some(x); break; }
        }
        let r = r.expect("search failed 5 times");
        // Signed-in playback without yt-dlp (Android path): resolve, then probe the tail.
        let at = std::time::Instant::now();
        let p = yt.solver.player().await;
        println!("player: {:?} in {:?}", p.as_ref().map(|p| (p.id.clone(), p.sts)).map_err(|e| format!("{e:#}")), at.elapsed());
        for t in r.tracks.iter().take(3) {
            let at = std::time::Instant::now();
            match yt.stream_signed(&t.id, super::Codec::Aac, cookie.as_deref().unwrap()).await {
                Ok(s) => {
                    let total = s.size.unwrap_or(2_000_000);
                    let probe = reqwest::Client::new().get(&s.url).header("Range", format!("bytes={}-{}", total.saturating_sub(4096), total - 1)).send().await;
                    println!("signed {} [{}]: OK {} in {:?}, tail: {:?}", t.title, t.id, s.mime, at.elapsed(), probe.map(|r| r.status()));
                }
                Err(e) => println!("signed {} [{}]: ERR {}", t.title, t.id, format!("{e:#}").chars().take(400).collect::<String>()),
            }
        }
        // Which client does yt-dlp use successfully? (verbose run; cookie file deleted right after)
        if std::env::var("YTDLP_VERBOSE").is_ok() {
            let c = cookie.as_deref().unwrap();
            let file = std::env::temp_dir().join("ytm-ck-test.txt");
            let mut txt = String::from("# Netscape HTTP Cookie File
");
            for kv in c.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((k, v)) = kv.split_once('=') { txt.push_str(&format!(".youtube.com	TRUE	/	TRUE	2147483647	{k}	{v}
")); }
            }
            std::fs::write(&file, txt).unwrap();
            let out = std::process::Command::new(data.join("bin").join("yt-dlp.exe"))
                .args(["-v", "-f", "bestaudio", "--get-url", "--no-playlist", "--cookies"]).arg(&file)
                .arg(format!("https://music.youtube.com/watch?v={}", r.tracks[0].id)).output().unwrap();
            let _ = std::fs::remove_file(&file);
            for l in String::from_utf8_lossy(&out.stderr).lines().filter(|l| l.contains("player") || l.contains("client") || l.contains("WARNING") || l.contains("ERROR") || l.contains("sig") || l.contains("challenge")) {
                println!("ytdlp: {}", l.chars().take(220).collect::<String>());
            }
            println!("ytdlp ok: {}", out.status.success());
        }
        for t in r.tracks.iter().take(3) {
            let s = crate::ytdlp::get_url_signed_in(&data.join("bin"), &std::env::temp_dir(), &t.id, "bestaudio[ext=m4a]/bestaudio", cookie.as_deref().unwrap()).await;
            println!("stream {} [{}]: {}", t.title, t.id, match s { Ok(u) => format!("OK url len {}", u.len()), Err(e) => format!("ERR {e:#}").chars().take(300).collect() });
        }
    }
}