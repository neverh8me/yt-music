//! Windows media overlay (volume flyout, lock screen, media keys) as "YT Music".
//!
//! The WebView's own media session shows up as "Unknown app" (it belongs to the
//! msedgewebview2 process). So the app registers its own identity (AppUserModelID with a
//! display name and icon) and drives the system media controls natively; the web layer
//! reports the current track and receives play/pause/next/previous/seek back as events.
#![cfg(windows)]

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};
use std::cell::RefCell;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Same id as the bundle identifier, so the installer's Start-menu shortcut matches.
pub const AUMID: &str = "io.github.neverh8me.ytmusic";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Register "YT Music" (name + icon) for our AppUserModelID under HKCU and adopt that id
/// for this process. Call before any window is created.
pub fn register_identity() {
    use windows_sys::Win32::System::Registry::*;
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    let key_path = wide(&format!("Software\\Classes\\AppUserModelId\\{AUMID}"));
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegCreateKeyExW(HKEY_CURRENT_USER, key_path.as_ptr(), 0, std::ptr::null(), 0, KEY_SET_VALUE, std::ptr::null(), &mut hkey, std::ptr::null_mut()) == 0 {
            for (name, value) in [("DisplayName", "YT Music".to_string()), ("IconUri", exe)] {
                let (n, v) = (wide(name), wide(&value));
                RegSetValueExW(hkey, n.as_ptr(), 0, REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32);
            }
            RegCloseKey(hkey);
        }
        let id = wide(AUMID);
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
}

thread_local! {
    // The WinRT objects behind the controls aren't Send: they live on the main thread.
    static CONTROLS: RefCell<Option<MediaControls>> = const { RefCell::new(None) };
}

/// Create the system media controls for the main window (call on the main thread).
pub fn init(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else { return };
    let Ok(hwnd) = win.hwnd() else { return };
    let config = PlatformConfig { display_name: "YT Music", dbus_name: "ytmusic", hwnd: Some(hwnd.0 as *mut std::ffi::c_void) };
    let Ok(mut controls) = MediaControls::new(config) else { return };
    let handle = app.clone();
    let attached = controls.attach(move |ev| {
        let (action, arg) = match ev {
            MediaControlEvent::Play => ("play", 0.0),
            MediaControlEvent::Pause => ("pause", 0.0),
            MediaControlEvent::Toggle => ("toggle", 0.0),
            MediaControlEvent::Next => ("next", 0.0),
            MediaControlEvent::Previous => ("prev", 0.0),
            MediaControlEvent::Stop => ("pause", 0.0),
            MediaControlEvent::SetPosition(MediaPosition(d)) => ("seek", d.as_secs_f64()),
            _ => return,
        };
        let _ = handle.emit("media-control", (action, arg));
    });
    if attached.is_ok() {
        CONTROLS.with(|c| *c.borrow_mut() = Some(controls));
    }
}

#[derive(serde::Deserialize)]
pub struct NowPlaying {
    title: String,
    artist: String,
    album: String,
    cover: String,
    duration_ms: u64,
    position_ms: u64,
    playing: bool,
}

/// Update the overlay with the current track and playback state.
pub fn update(app: &AppHandle, np: Option<NowPlaying>) {
    let _ = app.run_on_main_thread(move || {
        CONTROLS.with(|c| {
            let mut c = c.borrow_mut();
            let Some(controls) = c.as_mut() else { return };
            let Some(np) = np else {
                let _ = controls.set_playback(MediaPlayback::Stopped);
                return;
            };
            let _ = controls.set_metadata(MediaMetadata {
                title: Some(&np.title),
                artist: Some(&np.artist),
                album: Some(&np.album).filter(|a| !a.is_empty()),
                cover_url: Some(&np.cover).filter(|u| u.starts_with("https://")),
                duration: Some(Duration::from_millis(np.duration_ms)).filter(|d| !d.is_zero()),
            });
            let progress = Some(MediaPosition(Duration::from_millis(np.position_ms)));
            let _ = controls.set_playback(if np.playing { MediaPlayback::Playing { progress } } else { MediaPlayback::Paused { progress } });
        });
    });
}
