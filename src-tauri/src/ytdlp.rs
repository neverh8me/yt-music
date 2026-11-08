//! Optional yt-dlp fallback: locate/install the binary and ask it for stream URLs or files.
use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;

const EXE: &str = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
const RELEASE_URL: &str = if cfg!(windows) {
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
} else {
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp"
};

/// App-local copy first, then PATH.
pub fn find(dir: &Path) -> Option<PathBuf> {
    let local = dir.join(EXE);
    if local.is_file() {
        return Some(local);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(EXE))
            .find(|p| p.is_file())
    })
}

fn cmd(bin: &Path) -> Command {
    let mut c = Command::new(bin);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c.kill_on_drop(true);
    c
}

/// Only the audio URL is needed: skip HLS/DASH manifests and subtitle translations,
/// and fail fast on a dead connection instead of hanging.
const FAST: [&str; 4] = ["--extractor-args", "youtube:skip=hls,dash,translated_subs", "--socket-timeout", "10"];

fn watch_url(id: &str) -> String {
    format!("https://music.youtube.com/watch?v={id}")
}

pub async fn get_url(dir: &Path, id: &str, format: &str) -> Result<String> {
    let bin = find(dir).ok_or_else(|| anyhow!("yt-dlp is not installed"))?;
    let out = cmd(&bin)
        .args(["-f", format, "--get-url", "--no-playlist", "--no-warnings"])
        .args(FAST)
        .arg(watch_url(id))
        .output()
        .await?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http"))
        .ok_or_else(|| anyhow!("yt-dlp returned no URL"))
}

/// Signed-in variant for age-restricted songs: the session is written to a short-lived,
/// owner-only Netscape cookie file (yt-dlp's format), deleted right after the call.
pub async fn get_url_signed_in(dir: &Path, tmp_dir: &Path, id: &str, format: &str, cookie_header: &str) -> Result<String> {
    let bin = find(dir).ok_or_else(|| anyhow!("yt-dlp is not installed"))?;
    let mut rnd = [0u8; 8];
    getrandom::fill(&mut rnd).map_err(|e| anyhow!("rng: {e}"))?;
    let name: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
    let file = tmp_dir.join(format!("ck-{name}.txt"));
    let mut txt = String::from("# Netscape HTTP Cookie File\n");
    for kv in cookie_header.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((k, v)) = kv.split_once('=') {
            txt.push_str(&format!(".youtube.com\tTRUE\t/\tTRUE\t2147483647\t{k}\t{v}\n"));
        }
    }
    std::fs::create_dir_all(tmp_dir)?;
    std::fs::write(&file, txt)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    }
    let mut c = cmd(&bin);
    c.args(["-f", format, "--get-url", "--no-playlist", "--no-warnings"]).args(FAST).arg("--cookies").arg(&file);
    // yt-dlp needs a JS runtime for YouTube's web clients; use Node if installed.
    if let Some(node) = which_node() {
        c.arg("--js-runtimes").arg(format!("node:{}", node.display()));
    }
    let out = c.arg(watch_url(id)).output().await;
    let _ = std::fs::remove_file(&file);
    let out = out?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http"))
        .ok_or_else(|| anyhow!("yt-dlp returned no URL"))
}

fn which_node() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "node.exe" } else { "node" };
    let on_path = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe)).find(|p| p.is_file()));
    // A freshly installed Node isn't on the PATH of already-running sessions on Windows.
    #[cfg(windows)]
    let on_path = on_path.or_else(|| {
        let p = PathBuf::from(r"C:\Program Files\nodejs\node.exe");
        p.is_file().then_some(p)
    });
    on_path
}

const SUMS_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";

/// Expected SHA-256 of `name` from the release's SHA2-256SUMS file.
async fn expected_sha256(http: &reqwest::Client, name: &str) -> Result<String> {
    let sums = http.get(SUMS_URL).send().await?.error_for_status()?.text().await?;
    sums.lines()
        .find_map(|l| {
            let (hash, file) = l.split_once(char::is_whitespace)?;
            (file.trim().trim_start_matches('*') == name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| anyhow!("{name} not listed in SHA2-256SUMS"))
}

/// Download the latest release binary, verify it against SHA2-256SUMS, then install it.
pub async fn install(dir: &Path, http: &reqwest::Client) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    tokio::fs::create_dir_all(dir).await?;
    let (bytes, expected) = tokio::try_join!(
        async { Ok::<_, anyhow::Error>(http.get(RELEASE_URL).send().await?.error_for_status()?.bytes().await?) },
        expected_sha256(http, EXE),
    )?;
    let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        bail!("yt-dlp checksum mismatch (expected {expected}, got {actual}); not installing");
    }
    let tmp = dir.join(format!("{EXE}.tmp"));
    tokio::fs::write(&tmp, &bytes).await?;
    let dest = dir.join(EXE);
    tokio::fs::rename(&tmp, &dest).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(dest)
}

/// Make sure a usable yt-dlp exists: install the app-local copy if there is none,
/// and self-update it every few days (YouTube changes often, yt-dlp keeps up).
pub async fn ensure(dir: &Path, http: &reqwest::Client) -> Result<()> {
    let local = dir.join(EXE);
    match find(dir) {
        None => {
            install(dir, http).await?;
        }
        Some(bin) if bin == local => {
            let age = std::fs::metadata(&bin)?.modified()?.elapsed().unwrap_or_default();
            if age > std::time::Duration::from_secs(3 * 24 * 3600) {
                let out = cmd(&bin).arg("-U").output().await?;
                if !out.status.success() {
                    // standalone exe can't always self-update (e.g. locked); re-download instead
                    install(dir, http).await?;
                }
                // bump mtime so we don't retry on every start when already up to date
                let f = std::fs::File::options().write(true).open(&bin)?;
                f.set_modified(std::time::SystemTime::now())?;
            }
        }
        Some(_) => {} // system-wide install, managed by the user
    }
    Ok(())
}

pub async fn version(bin: &Path) -> Option<String> {
    let out = cmd(bin).arg("--version").output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
