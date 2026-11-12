//! In-app updates.
//! Desktop: tauri-plugin-updater fetches `latest.json` from the GitHub release, downloads
//! the installer for this OS, verifies its minisign signature against the public key
//! baked into the app, installs it and restarts.
//! Android: apps can't replace themselves, so we compare versions with the latest GitHub
//! release and open the new APK's download link; Android then shows its installer.
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    /// "install" (desktop self-update) or "apk" (Android download).
    pub kind: &'static str,
}

#[derive(Serialize, Clone)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

fn current(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

#[cfg(desktop)]
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(UpdateInfo {
        available: update.is_some(),
        current: current(&app),
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.and_then(|u| u.body),
        kind: "install",
    })
}

/// Download (with `update-progress` events), verify, install, restart.
#[cfg(desktop)]
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Err("already up to date".into());
    };
    let mut downloaded = 0u64;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // On Windows the NSIS installer takes over and exits the app itself.
    app.restart();
}

#[cfg(mobile)]
const LATEST: &str = "https://api.github.com/repos/neverh8me/yt-music/releases/latest";

#[cfg(mobile)]
async fn latest_apk(app: &AppHandle) -> Result<(semver::Version, Option<String>, Option<String>), String> {
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        body: Option<String>,
        assets: Vec<Asset>,
    }
    let _ = app;
    let r: Release = reqwest::Client::new()
        .get(LATEST)
        .header("User-Agent", "YT Music updater")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let v = semver::Version::parse(r.tag_name.trim_start_matches('v')).map_err(|e| e.to_string())?;
    let apk = r.assets.into_iter().find(|a| a.name.ends_with(".apk")).map(|a| a.browser_download_url);
    Ok((v, r.body, apk))
}

#[cfg(mobile)]
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let cur = current(&app);
    let (latest, notes, apk) = latest_apk(&app).await?;
    let newer = semver::Version::parse(&cur).map(|c| latest > c).unwrap_or(false);
    Ok(UpdateInfo {
        // iOS has no self-update: the new .ipa is side-loaded by hand (README).
        available: newer && apk.is_some() && cfg!(target_os = "android"),
        current: cur,
        version: Some(latest.to_string()),
        notes,
        kind: "apk",
    })
}

#[cfg(mobile)]
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let (_, _, apk) = latest_apk(&app).await?;
    let url = apk.ok_or("no APK in the latest release")?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}
