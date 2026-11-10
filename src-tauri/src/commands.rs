//! Tauri commands invoked from the frontend.
use crate::models::*;
use crate::state::Core;
use crate::yt::Codec;
use crate::{color, downloads, lyrics, ytdlp};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};

type Res<T> = Result<T, String>;
type St<'a> = State<'a, Arc<Core>>;

fn e<E: std::fmt::Display>(err: E) -> String {
    format!("{err:#}")
}

#[tauri::command]
pub fn stream_base(core: St) -> String {
    format!("http://127.0.0.1:{}/{}", core.port.load(Ordering::Relaxed), core.stream_token)
}

// ------------------------------------------------------------- YouTube Music
#[tauri::command]
pub async fn search(core: St<'_>, query: String) -> Res<serde_json::Value> {
    let key = format!("search:{}", query.trim().to_lowercase());
    page(core.inner(), key, 10 * MIN, move |c| async move { c.yt.search(&query).await }).await
}

#[tauri::command]
pub async fn search_suggestions(core: St<'_>, query: String) -> Res<Vec<String>> {
    core.yt.suggestions(&query).await.map_err(e)
}

#[tauri::command]
pub async fn home(core: St<'_>) -> Res<Home> {
    // "Made for you" first (from your history/likes), then charts for your region.
    let region = effective_region(&core);
    let general = page(core.inner(), format!("home:{region}"), 60 * MIN, move |c| async move { c.yt.home(&region).await });
    let (personal, general) = tokio::join!(crate::recommend::personal(&core), general);
    let mut sections = personal;
    match general.and_then(|v| serde_json::from_value::<Home>(v).map_err(e)) {
        Ok(h) => sections.extend(h.sections),
        Err(err) if sections.is_empty() => return Err(err),
        Err(_) => {}
    }
    Ok(Home { sections })
}

// ------------------------------------------------------------- page cache
/// Prefix of cached page keys in the DB cache table (pruned after a week, see lib.rs).
pub const PAGE_PREFIX: &str = "page:";
const MIN: i64 = 60 * 1000;

/// Pages from YouTube (albums, artists, playlists, charts, search) are kept in the DB, so
/// opening one you've seen before is instant: a fresh copy is returned as is, a stale one
/// is returned right away and refreshed in the background for next time. Only a page
/// never opened before (or not opened for a week) waits for the network.
async fn page<T, F, Fut>(core: &Arc<Core>, key: String, fresh_ms: i64, fetch: F) -> Res<serde_json::Value>
where
    T: serde::Serialize,
    F: FnOnce(Arc<Core>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
{
    fn store(core: &Core, key: &str, v: &serde_json::Value) {
        let entry = serde_json::json!({ "ts": crate::db::now(), "v": v });
        let _ = core.db.cache_put(key, &entry.to_string());
    }
    let key = format!("{PAGE_PREFIX}{key}");
    let old = core
        .db
        .cache_get(&key)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|mut o| Some((o["ts"].as_i64()?, o.get_mut("v")?.take())));
    match old {
        Some((ts, v)) if crate::db::now() - ts < fresh_ms => Ok(v),
        Some((_, v)) => {
            let c = core.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(new) = fetch(c.clone()).await
                    && let Ok(new) = serde_json::to_value(new)
                {
                    store(&c, &key, &new);
                }
            });
            Ok(v)
        }
        None => {
            let v = serde_json::to_value(fetch(core.clone()).await.map_err(e)?).map_err(e)?;
            store(core, &key, &v);
            Ok(v)
        }
    }
}

/// Stored setting: "auto" (default) or a country code.
fn chart_region(core: &Core) -> String {
    core.db.get_setting("chart_region").unwrap_or_else(|| "auto".into())
}

/// Where the user is, from OS settings, US if unknown.
/// Windows: the "Country or region" home location (independent of UI language, so an
/// English Windows in Ukraine still gives "UA"). Elsewhere: the locale's region ("uk-UA").
fn detected_region() -> String {
    #[cfg(windows)]
    if let Some(c) = windows_home_location() {
        return c;
    }
    sys_locale::get_locale()
        .and_then(|l| l.replace('_', "-").split('-').nth(1).map(str::to_uppercase))
        .filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()))
        .unwrap_or_else(|| "US".into())
}

#[cfg(windows)]
fn windows_home_location() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultGeoName;
    let mut buf = [0u16; 16];
    // SAFETY: buffer pointer and length are valid; returns chars written incl. NUL (0 on error).
    let n = unsafe { GetUserDefaultGeoName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(n as usize - 1)]);
    // Two-letter ISO codes only (numeric UN M.49 codes are returned for regions like "World").
    (s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic())).then(|| s.to_uppercase())
}

fn effective_region(core: &Core) -> String {
    match chart_region(core).as_str() {
        "auto" => detected_region(),
        r => r.to_string(),
    }
}

#[tauri::command]
pub fn set_chart_region(core: St, region: String) -> Res<()> {
    const ALLOWED: [&str; 7] = ["auto", "US", "GB", "DE", "FR", "PL", "UA"];
    if !ALLOWED.contains(&region.as_str()) {
        return Err("unsupported region".into());
    }
    core.db.set_setting("chart_region", &region).map_err(e)
}

#[tauri::command]
pub async fn get_album(core: St<'_>, id: String) -> Res<serde_json::Value> {
    page(core.inner(), format!("album:{id}"), 6 * 60 * MIN, move |c| async move { c.yt.album(&id).await }).await
}

#[tauri::command]
pub async fn get_artist(core: St<'_>, id: String) -> Res<serde_json::Value> {
    page(core.inner(), format!("artist:{id}"), 60 * MIN, move |c| async move { c.yt.artist(&id).await }).await
}

#[tauri::command]
pub async fn get_playlist(core: St<'_>, id: String) -> Res<serde_json::Value> {
    let logged_in = core.db.get_setting("logged_in").is_some();
    page(core.inner(), format!("playlist:{id}"), 10 * MIN, move |c| async move { c.yt.playlist_any(&id, logged_in).await }).await
}

#[tauri::command]
pub async fn get_radio(core: St<'_>, id: String) -> Res<Vec<Track>> {
    core.yt.radio(&id).await.map_err(e)
}

/// Synced lyrics (LRCLIB) with YouTube Music's plain lyrics as fallback.
/// Results are cached in the DB, so lyrics of offline tracks work without internet.
#[tauri::command]
pub async fn get_lyrics(core: St<'_>, track: Track) -> Res<Option<lyrics::Lyrics>> {
    let key = format!("lyrics:{}", track.id);
    if let Some(cached) = core.db.cache_get(&key) {
        return Ok(serde_json::from_str(&cached).ok().flatten());
    }
    let (lrc, yt) = tokio::join!(lyrics::lrclib(&core.http, &track), core.yt.lyrics(&track.id));
    // Both sources answered (even "nothing found") → the result is worth caching.
    let definite = lrc.is_ok() && yt.is_ok();
    let found = match lrc {
        Ok(Some(l)) if !l.synced.is_empty() || l.instrumental => Some(l),
        other => {
            let plain_lrc = other.ok().flatten();
            match yt {
                Ok(Some(y)) => Some(lyrics::Lyrics {
                    synced: vec![],
                    plain: Some(y.body),
                    source: "YouTube Music".into(),
                    instrumental: false,
                }),
                _ => plain_lrc,
            }
        }
    };
    // Don't cache after a network error, so it's retried next time.
    if found.is_some() || definite {
        let _ = core.db.cache_put(&key, &serde_json::to_string(&found).unwrap_or_default());
    }
    Ok(found)
}

/// Accent colour of a cover image (`#rrggbb`), computed in Rust and cached.
#[tauri::command]
pub async fn cover_color(core: St<'_>, url: String) -> Res<String> {
    // Only fetch cover art from Google's image hosts; never arbitrary URLs (no SSRF
    // into the local network through this command).
    if !is_cover_url(&url) {
        return Err("not a cover image URL".into());
    }
    let key = format!("color:{url}");
    if let Some(c) = core.db.cache_get(&key) {
        return Ok(c);
    }
    let bytes = core.http.get(&url).send().await.map_err(e)?.error_for_status().map_err(e)?.bytes().await.map_err(e)?;
    let c = tokio::task::spawn_blocking(move || color::dominant(&bytes)).await.map_err(e)?.map_err(e)?;
    let _ = core.db.cache_put(&key, &c);
    Ok(c)
}

fn is_cover_url(u: &str) -> bool {
    const HOSTS: [&str; 3] = ["googleusercontent.com", "ggpht.com", "ytimg.com"];
    let Ok(url) = url::Url::parse(u) else { return false };
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && url.port().is_none()
        && HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

#[cfg(test)]
mod tests {
    use super::is_cover_url;

    #[test]
    fn cover_url_allowlist() {
        assert!(is_cover_url("https://lh3.googleusercontent.com/abc=w544-h544"));
        assert!(is_cover_url("https://i.ytimg.com/vi/x/hqdefault.jpg"));
        assert!(!is_cover_url("http://lh3.googleusercontent.com/abc"), "https only");
        assert!(!is_cover_url("https://192.168.1.1/admin"));
        assert!(!is_cover_url("https://evilgoogleusercontent.com/x"));
        assert!(!is_cover_url("https://googleusercontent.com.evil.io/x"));
        assert!(!is_cover_url("https://lh3.googleusercontent.com:8443/x"));
    }
}

/// The current track for the system media overlay (Windows; other platforms ignore it).
#[tauri::command]
pub fn media_update(app: AppHandle, np: Option<serde_json::Value>) {
    #[cfg(windows)]
    crate::media::update(&app, np.and_then(|v| serde_json::from_value(v).ok()));
    #[cfg(not(windows))]
    let _ = (app, np);
}

/// Resolve stream URLs in the background (e.g. the track under the pointer), so a click plays at once.
#[tauri::command]
pub fn prefetch(core: St, ids: Vec<String>) {
    let ok = |id: &String| id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    for id in ids.into_iter().filter(ok).take(3) {
        let core = core.inner().clone();
        tauri::async_runtime::spawn(async move {
            if core.db.download_path(&id).is_none() {
                let _ = core.yt.stream(&id, Codec::Aac).await;
            }
        });
    }
}

/// Quality of what is (about to be) played: the offline file, or the AAC stream.
#[tauri::command]
pub async fn stream_quality(core: St<'_>, id: String) -> Res<StreamQuality> {
    if let Some((codec, kbps)) = core.db.download_quality(&id) {
        return Ok(StreamQuality { codec, kbps, offline: true });
    }
    let s = core.yt.stream(&id, Codec::Aac).await.map_err(e)?;
    Ok(StreamQuality { codec: Some(s.codec.label().into()), kbps: s.kbps, offline: false })
}

#[tauri::command]
pub fn set_download_quality(core: St, quality: String) -> Res<()> {
    if quality != "opus" && quality != "aac" {
        return Err("quality must be opus or aac".into());
    }
    core.db.set_setting("download_quality", &quality).map_err(e)
}

// ------------------------------------------------------------- library
#[tauri::command]
pub fn liked_tracks(core: St) -> Res<Vec<Track>> {
    core.db.liked_tracks().map_err(e)
}

#[tauri::command]
pub fn liked_ids(core: St) -> Res<Vec<String>> {
    core.db.liked_ids().map_err(e)
}

#[tauri::command]
pub fn set_liked(core: St, track: Track, liked: bool) -> Res<()> {
    core.db.set_liked(&track, liked).map_err(e)
}

#[tauri::command]
pub fn list_playlists(core: St) -> Res<Vec<LocalPlaylistSummary>> {
    core.db.list_playlists().map_err(e)
}

#[tauri::command]
pub fn get_local_playlist(core: St, id: i64) -> Res<LocalPlaylist> {
    core.db.get_playlist(id).map_err(e)
}

#[tauri::command]
pub fn create_playlist(core: St, name: String, tracks: Vec<Track>) -> Res<i64> {
    core.db.create_playlist(&name, &tracks).map_err(e)
}

#[tauri::command]
pub fn rename_playlist(core: St, id: i64, name: String) -> Res<()> {
    core.db.rename_playlist(id, &name).map_err(e)
}

#[tauri::command]
pub fn delete_playlist(core: St, id: i64) -> Res<()> {
    core.db.delete_playlist(id).map_err(e)
}

#[tauri::command]
pub fn add_to_playlist(core: St, id: i64, tracks: Vec<Track>) -> Res<()> {
    core.db.add_to_playlist(id, &tracks).map_err(e)
}

#[tauri::command]
pub fn remove_from_playlist(core: St, id: i64, pos: usize) -> Res<()> {
    core.db.remove_from_playlist(id, pos).map_err(e)
}

#[tauri::command]
pub fn get_history(core: St) -> Res<Vec<Track>> {
    core.db.history(200).map_err(e)
}

// ------------------------------------------------------------- downloads
#[tauri::command]
pub fn download_tracks(app: AppHandle, core: St, tracks: Vec<Track>) -> Res<()> {
    // as in YouTube Music: downloads are for signed-in listeners
    if core.db.get_setting("logged_in").is_none() {
        return Err("Sign in to download songs".into());
    }
    downloads::enqueue(app, core.inner().clone(), tracks);
    Ok(())
}

#[tauri::command]
pub fn list_downloads(core: St) -> Res<Vec<DownloadEntry>> {
    let mut list = core.db.downloads().map_err(e)?;
    // Drop entries whose file was deleted outside the app.
    list.retain(|d| {
        let ok = std::path::Path::new(&d.path).is_file();
        if !ok {
            let _ = core.db.remove_download(&d.track.id);
        }
        ok
    });
    Ok(list)
}

#[tauri::command]
pub fn downloaded_ids(core: St) -> Res<Vec<String>> {
    core.db.downloaded_ids().map_err(e)
}

#[tauri::command]
pub fn delete_download(core: St, id: String) -> Res<()> {
    if let Some(path) = core.db.remove_download(&id).map_err(e)? {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

// ------------------------------------------------------------- settings & account
#[tauri::command]
pub async fn get_settings(core: St<'_>) -> Res<Settings> {
    // Cached by the startup task; fall back to just the path (no slow process spawn here).
    let ytdlp = core
        .ytdlp_info
        .lock()
        .unwrap()
        .clone()
        .or_else(|| ytdlp::find(&core.ytdlp_dir()).map(|b| b.display().to_string()));
    Ok(Settings {
        download_dir: core.download_dir().to_string_lossy().into_owned(),
        logged_in: core.db.get_setting("logged_in").is_some(),
        account_name: core.db.get_setting("account_name"),
        account_avatar: core.db.get_setting("account_avatar"),
        ytdlp,
        ytdlp_supported: cfg!(desktop),
        download_quality: match core.download_codec() {
            Codec::Aac => "aac".into(),
            Codec::Opus => "opus".into(),
        },
        google_login_supported: true, // desktop window, Android activity, iOS in the main view
        chart_region: chart_region(&core),
        detected_region: detected_region(),
        notify_tracks: crate::integrations::enabled(&core, "notify_tracks"),
        discord_presence: crate::integrations::enabled(&core, "discord_presence"),
        #[cfg(desktop)]
        discord_available: !crate::integrations::DISCORD_CLIENT_ID.is_empty(),
        #[cfg(mobile)]
        discord_available: false,
        global_hotkeys: crate::integrations::enabled(&core, "global_hotkeys"),
        hotkeys_supported: cfg!(desktop),
    })
}

#[tauri::command]
pub fn set_download_dir(core: St, path: String) -> Res<()> {
    let p = std::path::PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err("An absolute path is required, e.g. D:\\YT Music offline".into());
    }
    std::fs::create_dir_all(&p).map_err(e)?;
    core.db.set_setting("download_dir", &p.to_string_lossy()).map_err(e)
}

#[tauri::command]
pub async fn login_cookies(core: St<'_>, text: String) -> Res<()> {
    core.yt.login(&text).await.map_err(e)?;
    core.db.set_setting("logged_in", "1").map_err(e)?;
    // name and avatar for the account menu (Android sign-in and pasted cookies come this way)
    let _ = refresh_account_info(&core).await;
    Ok(())
}

/// Fetch the YouTube account's name and avatar and remember them.
pub async fn refresh_account_info(core: &Core) -> Result<(), String> {
    let cookie = core.yt.session_cookie().ok_or("not signed in")?;
    let acc = crate::account::fetch(&core.http, &cookie).await.map_err(e)?;
    core.db.set_setting("account_name", &acc.name).map_err(e)?;
    match &acc.avatar {
        Some(a) => core.db.set_setting("account_avatar", a).map_err(e)?,
        None => core.db.del_setting("account_avatar").map_err(e)?,
    }
    Ok(())
}

/// Android: parse YouTube's player for 18+ playback in the background (see Yt::warm_solver).
#[tauri::command]
pub async fn warm_solver(core: St<'_>) -> Res<()> {
    core.yt.warm_solver().await.map_err(e)
}

#[tauri::command]
pub async fn refresh_account(core: St<'_>) -> Res<()> {
    refresh_account_info(&core).await
}

#[tauri::command]
pub async fn logout(core: St<'_>) -> Res<()> {
    core.yt.logout().await.map_err(e)?;
    core.db.del_setting("logged_in").map_err(e)?;
    core.db.del_setting("account_name").map_err(e)?;
    core.db.del_setting("account_avatar").map_err(e)?;
    core.db.del_setting("remote_playlists").map_err(e)?;
    Ok(())
}

#[tauri::command]
pub async fn sync_account(core: St<'_>) -> Res<SyncResult> {
    if core.db.get_setting("logged_in").is_none() {
        return Err("Log in first".into());
    }
    let (liked, pls) = tokio::join!(core.yt.liked_tracks(), core.yt.saved_playlists());
    let liked = liked.map_err(|err| format!("liked songs: {err:#}"))?;
    core.db.import_likes(&liked).map_err(e)?;
    let pls = pls.unwrap_or_default();
    let json = serde_json::to_string(&pls).map_err(e)?;
    core.db.set_setting("remote_playlists", &json).map_err(e)?;
    Ok(SyncResult { liked: liked.len(), playlists: pls.len() })
}

#[tauri::command]
pub fn remote_playlists(core: St) -> Res<Vec<PlaylistCard>> {
    let all: Vec<PlaylistCard> = core
        .db
        .get_setting("remote_playlists")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // "SE" (podcast "episodes for later") can't be opened by clients; hide it.
    Ok(all.into_iter().filter(|p| p.id != "SE").collect())
}

#[tauri::command]
pub async fn install_ytdlp(core: St<'_>) -> Res<String> {
    let bin = ytdlp::install(&core.ytdlp_dir(), &core.http).await.map_err(e)?;
    let info = match ytdlp::version(&bin).await {
        Some(v) => format!("{} ({v})", bin.display()),
        None => bin.display().to_string(),
    };
    *core.ytdlp_info.lock().unwrap() = Some(info.clone());
    Ok(info)
}
