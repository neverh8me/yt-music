//! "Sign in with Google": a separate private window shows Google's own sign-in page;
//! once it lands on music.youtube.com we read the session cookies from the webview and
//! hand them to rustypipe. The user never sees or copies cookies.
//!
//! The window is incognito, so the Google session is not kept in the webview profile:
//! only the YouTube cookies we hand to rustypipe stay, and logging out removes them.
use crate::state::Core;
use std::sync::Arc;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "google-login";
const START: &str = "https://accounts.google.com/ServiceLogin?service=youtube&passive=true&continue=https%3A%2F%2Fmusic.youtube.com%2F";
/// YouTube's own sign-in hand-off (what its "Sign in" button does): after Google, youtube.com/signin
/// sets the YouTube session cookies, then goes on to YouTube Music. WebKit (macOS, Linux) doesn't
/// always get the YouTube cookies from the shorter START redirect, so it starts here and, if the
/// page still arrives signed out, retries once (Google's own session is there by then).
#[cfg(not(windows))]
const YT_SIGNIN: &str = "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";
#[cfg(not(windows))]
static RETRIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A regular desktop browser UA: Google refuses sign-in in "embedded" browsers it can detect.
/// Must stay identical to `additionalBrowserArgs` of the main window in tauri.conf.json.
#[cfg(windows)]
const MAIN_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,HardwareMediaKeyHandling,MediaSessionService --autoplay-policy=no-user-gesture-required";
/// It must match the engine actually rendering the page: Google compares the UA with what
/// the browser can do and blocks sign-in ("this browser may not be secure") on a mismatch.
/// Windows renders with WebView2 (Chromium/Edge); macOS and Linux with WebKit (Safari).
#[cfg(windows)]
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0";
#[cfg(not(windows))]
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15";

#[derive(Clone, serde::Serialize)]
struct LoginResult {
    ok: bool,
    error: Option<String>,
}

fn done(app: &AppHandle, ok: bool, error: Option<String>) {
    let _ = app.emit("google-login", LoginResult { ok, error });
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.close();
    }
}

/// Only one cookie poller at a time (page loads on music.youtube.com can fire repeatedly).
static POLLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Diagnostics for sign-in problems: steps and cookie *names* only, never values.
fn log(core: &Core, msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(core.data_dir.join("login.log")) {
        let _ = writeln!(f, "[{}] {msg}", crate::db::now());
    }
}

/// Session cookies for YouTube, from the login window's cookie store.
async fn youtube_cookies(app: &AppHandle) -> Result<Vec<(String, String)>, String> {
    let w = app.get_webview_window(LABEL).ok_or("login window closed")?;
    // WebView2 deadlocks if cookies are read on an event-handler thread; use a worker thread.
    let all = tauri::async_runtime::spawn_blocking(move || w.cookies())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(all
        .into_iter()
        .filter(|c| {
            let d = c.domain().unwrap_or("").trim_start_matches('.');
            d == "youtube.com" || d.ends_with(".youtube.com")
        })
        .map(|c| (c.name().to_string(), c.value().to_string()))
        .collect())
}

/// After the window reaches music.youtube.com, poll its cookies until the signed-in
/// session (SAPISID) shows up — it can arrive a moment after the page "finishes"
/// loading — then log rustypipe in with them.
async fn capture(app: AppHandle, core: Arc<Core>) {
    use std::sync::atomic::Ordering;
    if POLLING.swap(true, Ordering::SeqCst) {
        return;
    }
    log(&core, "sign-in window open, waiting for YouTube session cookies");
    let mut last_names = String::new();
    for _ in 0..400 {
        // ~10 minutes; stops earlier if the window is closed
        let cookies = match youtube_cookies(&app).await {
            Ok(c) => c,
            Err(e) => {
                log(&core, &format!("stop: {e}"));
                break;
            }
        };
        let names = cookies.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(",");
        if names != last_names {
            log(&core, &format!("cookie names: {names}"));
            last_names = names;
        }
        // SAPISID is what authenticated YouTube requests are signed with.
        if cookies.iter().any(|(n, _)| n == "SAPISID" || n == "__Secure-3PAPISID") {
            let header = cookies.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ");
            match core.yt.login(&header).await {
                Ok(()) => {
                    log(&core, "signed in");
                    let _ = core.db.set_setting("logged_in", "1");
                    if let Err(e) = crate::commands::refresh_account_info(&core).await {
                        log(&core, &format!("account info unavailable: {e}"));
                    }
                    done(&app, true, None);
                }
                Err(e) => {
                    log(&core, &format!("rustypipe rejected the session: {e:#}"));
                    done(&app, false, Some(format!("{e:#}")));
                }
            }
            POLLING.store(false, Ordering::SeqCst);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }
    log(&core, "gave up waiting for session cookies");
    POLLING.store(false, Ordering::SeqCst);
}

/// WebKit: the page reached YouTube Music but without a session → run YouTube's sign-in
/// hand-off once more (Google is signed in by now, so it passes straight through).
#[cfg(not(windows))]
async fn retry_signin(app: AppHandle) {
    use std::sync::atomic::Ordering;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    let signed_in = youtube_cookies(&app).await.map(|c| c.iter().any(|(n, _)| n == "SAPISID" || n == "__Secure-3PAPISID"));
    if signed_in != Ok(false) || RETRIED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let (Some(w), Ok(url)) = (app.get_webview_window(LABEL), Url::parse(YT_SIGNIN)) {
        let _ = w.navigate(url);
    }
}

#[tauri::command]
pub async fn google_login(app: AppHandle, core: State<'_, Arc<Core>>) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_focus();
        return Ok(());
    }
    let core = core.inner().clone();
    let poll_core = core.clone();
    let handler_app = app.clone();
    #[cfg(windows)]
    let url = Url::parse(START).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let url = {
        RETRIED.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = START;
        Url::parse(YT_SIGNIN).map_err(|e| e.to_string())?
    };
    let builder = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(url));
    // WebView2 shares one browser process per data folder; every window must use the same
    // browser arguments as the main window (tauri.conf.json), or creation fails with
    // 0x8007139F "group or resource is not in the correct state".
    #[cfg(windows)]
    let builder = builder.additional_browser_args(MAIN_BROWSER_ARGS);
    let window = builder
        .title("Sign in with Google")
        // automated tests create it off-screen
        .visible(std::env::var_os("YTUBEM_HIDDEN").is_none())
        .inner_size(480.0, 720.0)
        .resizable(true)
        .incognito(true)
        .user_agent(UA)
        .on_page_load(move |_w, payload| {
            let host = payload.url().host_str().unwrap_or("");
            if payload.event() == PageLoadEvent::Finished && host == "music.youtube.com" {
                tauri::async_runtime::spawn(capture(handler_app.clone(), core.clone()));
                #[cfg(not(windows))]
                tauri::async_runtime::spawn(retry_signin(handler_app.clone()));
            }
        })
        .build()
        .map_err(|e| e.to_string())?;
    // Closing the window without finishing counts as cancel.
    let cancel_app = app.clone();
    window.on_window_event(move |ev| {
        if matches!(ev, tauri::WindowEvent::Destroyed) {
            let _ = cancel_app.emit("google-login-closed", ());
        }
    });
    // Watch for the session from the start, not only on page-load events
    // (music.youtube.com is a single-page app; the session may appear without a reload).
    tauri::async_runtime::spawn(capture(app.clone(), poll_core));
    Ok(())
}
