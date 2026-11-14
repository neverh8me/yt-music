//! YT Music — app entry point (desktop `main.rs` and the Android activity both call `run`).
mod account;
mod auth;
mod color;
mod commands;
mod db;
mod downloads;
mod integrations;
mod jsplayer;
mod lyrics;
#[cfg(windows)]
mod media;
mod models;
mod playback;
mod player;
mod recommend;
mod secrets;
mod secure_store;
mod server;
mod state;
mod update;
mod vault;
#[cfg(desktop)]
mod webplayer;
mod yt;
mod ytdlp;

use state::Core;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Name the app "YT Music" in the Windows media overlay / notifications.
    #[cfg(windows)]
    media::register_identity();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(integrations::hotkeys_plugin());
    builder
        .setup(|app| {
            // iOS: the window is made here (tauri.ios.conf.json has none) so it can carry Mobile
            // Safari's user agent for this iOS version: Google's sign-in opens in it (auth.rs).
            #[cfg(target_os = "ios")]
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
                .user_agent(&auth::ios::user_agent())
                .build()?;
            // YTUBEM_DATA_DIR isolates automated test runs from the user's real library.
            let data_dir = match std::env::var_os("YTUBEM_DATA_DIR") {
                Some(d) => std::path::PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            std::fs::create_dir_all(&data_dir)?;
            // Offline tracks are encrypted `.ytmc` files, so they live with the app data
            // rather than in the user's music folder.
            let default_download_dir = data_dir.join("offline");

            let db = db::Db::open(&data_dir.join("library.db"))?;
            // cached pages (commands::page) live a week
            let _ = db.cache_prune(commands::PAGE_PREFIX, 7 * 24 * 3600 * 1000);
            let session_key = secrets::get_or_create(&data_dir, "session", None)?;
            let yt = yt::Yt::new(data_dir.join("rustypipe"), data_dir.join("bin"), &session_key)?;
            let http = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(20))
                .build()?;

            let core = Arc::new(Core::new(db, yt, http, data_dir, default_download_dir));
            let port = tauri::async_runtime::block_on(server::start(core.clone()))?;
            core.port.store(port, Ordering::Relaxed);

            // yt-dlp is a desktop-only fallback for stream resolution.
            #[cfg(desktop)]
            {
                let bg = core.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = ytdlp::ensure(&bg.ytdlp_dir(), &bg.http).await {
                        eprintln!("yt-dlp setup failed: {e:#}");
                    }
                    // Cache "path (version)" for Settings.
                    if let Some(bin) = ytdlp::find(&bg.ytdlp_dir()) {
                        let info = match ytdlp::version(&bin).await {
                            Some(v) => format!("{} ({v})", bin.display()),
                            None => bin.display().to_string(),
                        };
                        *bg.ytdlp_info.lock().unwrap() = Some(info);
                    }
                });
            }
            #[cfg(desktop)]
            webplayer::init(app.handle(), &core);
            app.manage(core);
            jsplayer::attach(app.handle().clone());
            // Our own title bar (ui: #topbar + window buttons) instead of the system frame.
            // macOS keeps its traffic lights, overlaid on the bar (titleBarStyle: Overlay).
            #[cfg(any(windows, target_os = "linux"))]
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_decorations(false);
            }
            #[cfg(windows)]
            media::init(app.handle());
            #[cfg(desktop)]
            integrations::register_hotkeys(app.handle());

            // The window starts hidden (no white flash); show it once set up.
            // YTUBEM_HIDDEN keeps it hidden for automated UI tests.
            if std::env::var_os("YTUBEM_HIDDEN").is_none()
                && let Some(w) = app.get_webview_window("main")
            {
                w.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(desktop)]
            webplayer::webplayer_load,
            #[cfg(desktop)]
            webplayer::webplayer_play,
            #[cfg(desktop)]
            webplayer::webplayer_pause,
            #[cfg(desktop)]
            webplayer::webplayer_stop,
            #[cfg(desktop)]
            webplayer::webplayer_seek,
            #[cfg(desktop)]
            webplayer::webplayer_volume,
            #[cfg(desktop)]
            webplayer::webplayer_enable,
            commands::stream_base,
            commands::search,
            commands::search_suggestions,
            commands::home,
            commands::get_album,
            commands::get_artist,
            commands::get_artist_releases,
            commands::get_playlist,
            commands::get_radio,
            commands::get_lyrics,
            commands::cover_color,
            commands::stream_quality,
            commands::prefetch,
            commands::media_update,
            jsplayer::js_solve_done,
            commands::warm_solver,
            commands::liked_tracks,
            commands::liked_ids,
            commands::set_liked,
            commands::list_playlists,
            commands::get_local_playlist,
            commands::create_playlist,
            commands::rename_playlist,
            commands::delete_playlist,
            commands::add_to_playlist,
            commands::remove_from_playlist,
            commands::get_history,
            commands::download_tracks,
            commands::list_downloads,
            commands::downloaded_ids,
            commands::delete_download,
            commands::get_settings,
            commands::set_download_dir,
            commands::set_download_quality,
            commands::set_chart_region,
            commands::login_cookies,
            commands::logout,
            commands::refresh_account,
            commands::sync_account,
            commands::remote_playlists,
            commands::install_ytdlp,
            playback::player_state,
            playback::player_play_list,
            playback::player_play_next,
            playback::player_add_queue,
            playback::player_remove,
            playback::player_jump,
            playback::player_next,
            playback::player_prev,
            playback::player_toggle_shuffle,
            playback::player_cycle_repeat,
            playback::player_set_autoplay,
            playback::set_sleep_timer,
            playback::sleep_timer_state,
            playback::presence,
            playback::set_toggle,
            auth::google_login,
            update::check_update,
            update::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running YT Music");
}
