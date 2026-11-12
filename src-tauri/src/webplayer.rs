//! Desktop playback engine: YouTube Music's own web player in a hidden webview (issue #17).
//!
//! A second window (`ytm-player`, never shown except for sign-in) loads music.youtube.com.
//! A small script injected into the page (`PAGE_JS`) wraps the page's player object; Rust
//! drives it with `eval` and our UI only shows its state. Stream URLs, signatures and 18+
//! checks are the site's own business; ads play as on the site.
//!
//! How state gets back to Rust: Rust polls the page with `eval_with_callback` four times a
//! second and emits `webplayer-state` when something changed. Why not the alternatives:
//! - Tauri IPC from a remote origin needs a capability with `remote.urls`, and exposing one
//!   app command that way means declaring an app manifest, after which *every* app command
//!   needs an explicit permission; it would also put `__TAURI_INTERNALS__` within reach of
//!   third-party page scripts.
//! - `fetch` to our local server is an https → http://127.0.0.1 request: subject to the
//!   page's CSP, mixed-content rules (WebKit) and Chromium's local-network prompts.
//! - `eval` is run by the host, so the page's CSP / Trusted Types don't apply, it works the
//!   same in WebView2, WKWebView and WebKitGTK, and the page can't call into the app at all.
//!
//! Polling from Rust (rather than a timer in the page) also keeps working when the engine
//! throttles timers of a page that is not visible.
use crate::state::Core;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "ytm-player";
const HOME: &str = "https://music.youtube.com/";
const SETTING: &str = "web_player";
/// Set once the account was signed in through the player window (see `migrate`).
pub const WEB_LOGIN: &str = "web_login";

/// Runs in the page before its own scripts, on every navigation of the player window.
/// Everything that depends on the site's internals lives here (see ytmdesktop for the same
/// page API): `ytmusic-player-bar.playerApi` / `#movie_player` with `loadVideoById`,
/// `playVideo`, `pauseVideo`, `seekTo`, `setVolume`, `getPlayerState`, `getVideoData`.
const PAGE_JS: &str = r#"(function () {
  if (window.top !== window || window.__ytmc) return;
  var music = location.hostname === 'music.youtube.com';
  var want = '', pending = '', ends = 0, endSeen = false, sawWant = false, vol = null, muted = false;
  function api() {
    var bar = document.querySelector('ytmusic-player-bar');
    var p = (bar && bar.playerApi) || document.getElementById('movie_player');
    return p && typeof p.getPlayerState === 'function' ? p : null;
  }
  function isAd() {
    var el = document.getElementById('movie_player');
    return !!(el && (el.classList.contains('ad-showing') || el.classList.contains('ad-interrupting')));
  }
  function applyVolume(p) {
    if (vol === null) return;
    try { p.setVolume(Math.round(vol * 100)); if (muted) p.mute(); else p.unMute(); } catch (e) {}
  }
  function start(p, id) {
    endSeen = false; sawWant = false;
    applyVolume(p);
    p.loadVideoById(id);
  }
  window.__ytmc = {
    load: function (id) {
      want = id;
      var p = music && api();
      if (p) { pending = ''; start(p, id); } else pending = id;
    },
    play: function () { var p = api(); if (p) p.playVideo(); },
    pause: function () { var p = api(); if (p) p.pauseVideo(); },
    stop: function () { want = ''; pending = ''; var p = api(); if (p) p.pauseVideo(); },
    seek: function (s) { var p = api(); if (p) { p.seekTo(s, true); endSeen = false; } },
    volume: function (v, m) { vol = v; muted = !!m; var p = api(); if (p) applyVolume(p); },
    state: function () {
      var out = { ready: false, host: location.hostname, signed_in: false, id: '', want: want, position: 0, duration: 0, state: -1, ad: false, ends: ends };
      try {
        out.signed_in = !!(window.ytcfg && ytcfg.get && ytcfg.get('LOGGED_IN'));
        var p = music && api();
        if (!p) return out;
        out.ready = true;
        if (pending) { var id = pending; pending = ''; start(p, id); }
        var d = p.getVideoData ? p.getVideoData() : null;
        out.id = (d && d.video_id) || '';
        out.state = p.getPlayerState();
        out.ad = isAd();
        out.position = p.getCurrentTime() || 0;
        out.duration = p.getDuration() || 0;
        if (want && !out.ad) {
          if (out.id === want) {
            if (out.state === 1) { sawWant = true; endSeen = false; }
            // 0 = ended: counted once; our queue (not the site's) picks what plays next.
            if (out.state === 0 && !endSeen) { endSeen = true; ends++; }
          } else if (sawWant && out.id && out.state === 1 && !endSeen) {
            // The site moved on to a track of its own before we saw "ended".
            endSeen = true; ends++;
            p.pauseVideo();
          }
        }
        out.ends = ends;
      } catch (e) { out.error = String(e); }
      return out;
    }
  };
})();"#;

/// What Rust itself needs from the last poll (the UI gets the whole JSON).
#[derive(Default)]
struct Shared {
    last_json: String,
    signing_in: bool,
}
static SHARED: Mutex<Shared> = Mutex::new(Shared { last_json: String::new(), signing_in: false });
static POLLING: AtomicBool = AtomicBool::new(false);

pub fn enabled(core: &Core) -> bool {
    core.db.get_setting(SETTING).as_deref() != Some("0")
}

/// Only YouTube video ids ever reach the page script.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Create the hidden player window if it isn't there, and start polling it.
pub fn ensure(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    let url = Url::parse(HOME).map_err(|e| e.to_string())?;
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url));
    // Same browser arguments as every other window (see auth.rs): WebView2 requires it, and
    // they also turn off the webview's own media session (ours comes from media.rs).
    #[cfg(windows)]
    let builder = builder.additional_browser_args(crate::auth::MAIN_BROWSER_ARGS);
    let window = builder
        .title("YT Music: sign in")
        .visible(false)
        .inner_size(1000.0, 720.0)
        // Firefox's user agent, as in auth.rs: Google refuses sign-in in embedded
        // Chromium/WebKit shells. One user agent for the whole window, since it can't be
        // set per request.
        .user_agent(crate::auth::UA)
        .initialization_script(PAGE_JS)
        .build()
        .map_err(|e| e.to_string())?;
    // The window is the playback engine: closing it (after sign-in) only hides it.
    let w2 = window.clone();
    window.on_window_event(move |ev| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
            api.prevent_close();
            let _ = w2.hide();
            SHARED.lock().unwrap().signing_in = false;
            // lets Settings re-enable its sign-in button
            let _ = w2.app_handle().emit("google-login-closed", ());
        }
    });
    if !POLLING.swap(true, Ordering::SeqCst) {
        tauri::async_runtime::spawn(poll(app.clone()));
    }
    Ok(window)
}

async fn poll(app: AppHandle) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let Some(w) = app.get_webview_window(LABEL) else { break };
        let app2 = app.clone();
        let _ = w.eval_with_callback("window.__ytmc ? window.__ytmc.state() : null", move |json| on_state(&app2, json));
    }
    POLLING.store(false, Ordering::SeqCst);
}

fn on_state(app: &AppHandle, json: String) {
    // The result arrives JSON-encoded; some engines hand over a JSON string of JSON.
    let mut v: serde_json::Value = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(s) = v.as_str() {
        v = serde_json::from_str(s).unwrap_or(serde_json::Value::Null);
    }
    if !v.is_object() {
        return;
    }
    {
        let mut sh = SHARED.lock().unwrap();
        if sh.last_json == json {
            return;
        }
        sh.last_json = json;
    }
    let _ = app.emit("webplayer-state", v);
}

fn run(app: &AppHandle, js: String) -> Result<(), String> {
    ensure(app)?.eval(format!("window.__ytmc && window.__ytmc.{js}")).map_err(|e| e.to_string())
}

/// Start the engine at launch so the first song starts at once.
pub fn init(app: &AppHandle, core: &Arc<Core>) {
    if !enabled(core) {
        return;
    }
    if let Err(e) = ensure(app) {
        eprintln!("web player window failed: {e}");
        return;
    }
    tauri::async_runtime::spawn(migrate(app.clone(), core.clone()));
}

/// One time, after updating from a version with the cookie-only sign-in: the account is
/// signed in for the library but the player window has no session, and without one the
/// site stops songs part-way. Sign the account out so the next sign-in covers both.
/// Never repeats: afterwards either nothing is signed in or `WEB_LOGIN` is set.
async fn migrate(app: AppHandle, core: Arc<Core>) {
    if core.db.get_setting(WEB_LOGIN).is_some() || core.db.get_setting("logged_in").is_none() {
        return;
    }
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let Ok(cookies) = crate::auth::youtube_cookies(&app, LABEL).await else { return };
    if cookies.iter().any(|(n, _)| n == "SAPISID" || n == "__Secure-3PAPISID") {
        // the player window already has a session
        let _ = core.db.set_setting(WEB_LOGIN, "1");
        return;
    }
    if crate::commands::forget_account(&core).await.is_ok() {
        let _ = app.emit("signed-out", ());
    }
}

pub fn signing_in() -> bool {
    SHARED.lock().unwrap().signing_in
}

/// Show the player window on YouTube Music's own sign-in link.
pub fn sign_in(app: &AppHandle) -> Result<(), String> {
    let w = ensure(app)?;
    let url = Url::parse(crate::auth::START).map_err(|e| e.to_string())?;
    w.navigate(url).map_err(|e| e.to_string())?;
    SHARED.lock().unwrap().signing_in = true;
    w.show().map_err(|e| e.to_string())?;
    let _ = w.set_focus();
    Ok(())
}

/// Sign-in is over (auth.rs has the session): hide the window again.
pub fn finish_sign_in(app: &AppHandle) {
    let was = std::mem::replace(&mut SHARED.lock().unwrap().signing_in, false);
    if was && let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
}

/// Log out: remove Google's and YouTube's cookies from the player window and reload it.
pub async fn sign_out(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let w2 = w.clone();
    // WebView2 deadlocks if cookies are read on an event-handler thread; use a worker thread.
    if let Ok(Ok(all)) = tauri::async_runtime::spawn_blocking(move || w2.cookies()).await {
        for c in all {
            let d = c.domain().unwrap_or("").trim_start_matches('.').to_string();
            if ["youtube.com", "google.com"].iter().any(|s| d == *s || d.ends_with(&format!(".{s}"))) {
                let _ = w.delete_cookie(c);
            }
        }
    }
    if let Ok(url) = Url::parse(HOME) {
        let _ = w.navigate(url);
    }
}

// Commands are async: creating a window inside a sync command deadlocks on Windows.

/// Play `id` in the web player. `false` = not handled here (engine switched off, or the
/// track has an offline copy): the UI then plays it through the built-in engine.
#[tauri::command]
pub async fn webplayer_load(app: AppHandle, core: State<'_, Arc<Core>>, id: String) -> Result<bool, String> {
    if !enabled(&core) || !valid_id(&id) || core.db.download_path(&id).is_some() {
        return Ok(false);
    }
    run(&app, format!("load('{id}')"))?;
    Ok(true)
}

#[tauri::command]
pub async fn webplayer_play(app: AppHandle) -> Result<(), String> {
    run(&app, "play()".into())
}

#[tauri::command]
pub async fn webplayer_pause(app: AppHandle) -> Result<(), String> {
    run(&app, "pause()".into())
}

/// Pause and forget the track (the UI moved to the built-in engine or the queue ended).
#[tauri::command]
pub async fn webplayer_stop(app: AppHandle) -> Result<(), String> {
    match app.get_webview_window(LABEL) {
        Some(w) => w.eval("window.__ytmc && window.__ytmc.stop()").map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
pub async fn webplayer_seek(app: AppHandle, seconds: f64) -> Result<(), String> {
    if !seconds.is_finite() {
        return Ok(());
    }
    run(&app, format!("seek({})", seconds.max(0.0)))
}

#[tauri::command]
pub async fn webplayer_volume(app: AppHandle, volume: f64, muted: bool) -> Result<(), String> {
    if !volume.is_finite() {
        return Ok(());
    }
    run(&app, format!("volume({},{muted})", volume.clamp(0.0, 1.0)))
}

/// The engine switch in Settings. Off: the window is destroyed (frees its memory).
#[tauri::command]
pub async fn webplayer_enable(app: AppHandle, core: State<'_, Arc<Core>>, on: bool) -> Result<(), String> {
    core.db.set_setting(SETTING, if on { "1" } else { "0" }).map_err(|e| e.to_string())?;
    if on {
        ensure(&app)?;
    } else if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
        *SHARED.lock().unwrap() = Shared::default();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_video_ids_reach_the_page() {
        assert!(valid_id("dQw4w9WgXcQ"));
        assert!(valid_id("a-b_c"));
        assert!(!valid_id(""));
        assert!(!valid_id("x');alert(1)//"));
        assert!(!valid_id("a b"));
    }
}
