//! Offline downloads: fetch the audio stream in bounded chunks and store it encrypted in
//! our `.ytmc` container (see `vault`). Track metadata lives in the library DB, files are
//! named by a hash of the video id, so the download folder reveals nothing.
use crate::models::{DownloadProgress, Track};
use crate::state::Core;
use crate::vault;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

const CHUNK: u64 = 4 * 1024 * 1024;

fn emit(app: &AppHandle, id: &str, state: &'static str, progress: f32, error: Option<String>) {
    let _ = app.emit(
        "download-progress",
        DownloadProgress { id: id.to_string(), state, progress, error },
    );
}

pub fn enqueue(app: AppHandle, core: Arc<Core>, tracks: Vec<Track>) {
    for t in tracks {
        {
            let mut busy = core.downloading.lock().unwrap();
            if core.db.download_path(&t.id).is_some() || !busy.insert(t.id.clone()) {
                continue;
            }
        }
        emit(&app, &t.id, "queued", 0.0, None);
        let (app, core) = (app.clone(), core.clone());
        tauri::async_runtime::spawn(async move {
            let _permit = core.download_slots.acquire().await;
            emit(&app, &t.id, "downloading", 0.0, None);
            let progress = |p: f32| emit(&app, &t.id, "downloading", p, None);
            let res = download_one(&core, &t, progress).await;
            core.downloading.lock().unwrap().remove(&t.id);
            match res {
                Ok(()) => emit(&app, &t.id, "done", 1.0, None),
                Err(e) => {
                    eprintln!("download {} failed: {e:#}", t.id);
                    emit(&app, &t.id, "error", 0.0, Some(format!("{}: {e:#}", t.title)));
                }
            }
        });
    }
}

/// `<dir>/<first 16 bytes of sha256(id) as hex>.ytmc`
pub fn file_for(dir: &Path, id: &str) -> PathBuf {
    let h = Sha256::digest(id.as_bytes());
    let name: String = h[..16].iter().map(|b| format!("{b:02x}")).collect();
    dir.join(name).with_extension(vault::EXT)
}

async fn download_one(core: &Core, t: &Track, progress: impl Fn(f32) + Sync) -> Result<()> {
    let dir = core.download_dir();
    tokio::fs::create_dir_all(&dir).await.context("couldn't create the download folder")?;
    let dest = file_for(&dir, &t.id);
    let key = core.vault_key()?;
    // googlevideo occasionally refuses a freshly issued URL (403); like the playback
    // proxy, drop the cached URL and try once more with a new one.
    let mut attempt = 0;
    let info = loop {
        let info = core.yt.stream(&t.id, core.download_codec()).await?;
        match fetch_encrypted(core, &info.url, info.size, &info.mime, &key, &dest, &progress).await {
            Ok(()) => break info,
            Err(e) if attempt == 0 => {
                eprintln!("download {} attempt 1 failed ({e:#}); retrying with a fresh URL", t.id);
                core.yt.invalidate(&t.id);
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    };
    let size = tokio::fs::metadata(&dest).await?.len();
    core.db.add_download(t, &dest.to_string_lossy(), size, info.codec.label(), info.kbps)?;
    Ok(())
}

/// Download in bounded Range chunks (googlevideo rejects open-ended/huge ranges),
/// encrypting on the fly. Writes to `.part` and renames on success.
async fn fetch_encrypted(
    core: &Core,
    url: &str,
    size: Option<u64>,
    mime: &str,
    key: &vault::Key,
    dest: &Path,
    progress: impl Fn(f32),
) -> Result<()> {
    let part = dest.with_extension("part");
    let mut w = vault::Writer::create(&part, key, mime).await?;
    let mut total = size;
    let mut done: u64 = 0;
    let mut last_emit = Instant::now();

    let result: Result<()> = async {
        loop {
            let end = done + CHUNK - 1;
            let end = total.map_or(end, |t| end.min(t.saturating_sub(1)));
            let resp = core
                .http
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={done}-{end}"))
                .send()
                .await?
                .error_for_status()?;
            if total.is_none() {
                total = resp
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.rsplit('/').next())
                    .and_then(|v| v.parse().ok());
            }
            let full_body = resp.status() == reqwest::StatusCode::OK; // server ignored Range
            let mut resp = resp;
            let mut got = 0u64;
            while let Some(chunk) = resp.chunk().await? {
                w.write(&chunk).await?;
                got += chunk.len() as u64;
                done += chunk.len() as u64;
                if last_emit.elapsed().as_millis() > 300 {
                    last_emit = Instant::now();
                    progress(total.map_or(0.0, |t| done as f32 / t as f32).min(0.99));
                }
            }
            if got == 0 || full_body || total.is_some_and(|t| done >= t) {
                break;
            }
        }
        if done == 0 {
            bail!("empty response");
        }
        if let Some(t) = total.filter(|t| done < *t) {
            bail!("incomplete download: {done} of {t} bytes");
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            w.finish().await?;
            tokio::fs::rename(&part, dest).await?;
            Ok(())
        }
        Err(e) => {
            drop(w);
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::yt::Codec;

    #[tokio::test]
    #[ignore]
    async fn live_download_encrypted() {
        let core = crate::state::test_core();
        let t = core.yt.search("Okean Elzy Obiymy").await.unwrap().tracks.remove(0);
        let dir = core.download_dir();
        tokio::fs::create_dir_all(&dir).await.unwrap();
        for codec in [Codec::Aac, Codec::Opus] {
            let info = core.yt.stream(&t.id, codec).await.unwrap();
            let dest = dir.join(format!("test-{codec:?}.ytmc"));
            let key = core.vault_key().unwrap();
            let start = Instant::now();
            fetch_encrypted(&core, &info.url, info.size, &info.mime, &key, &dest, |_| {}).await.unwrap();
            let len = std::fs::metadata(&dest).unwrap().len();
            println!("{codec:?}: {} {:?} kbps, {} bytes in {:?}", info.mime, info.kbps, len, start.elapsed());
            // decrypting the first bytes must give the container signature back
            let mut f = tokio::fs::File::open(&dest).await.unwrap();
            let h = vault::read_header(&mut f).await.unwrap();
            let mut r = vault::RangeReader::open(f, &h, &key, 0, 16).await.unwrap();
            let head = r.next_chunk().await.unwrap().unwrap();
            match codec {
                Codec::Aac => assert_eq!(&head[4..8], b"ftyp", "MP4 signature"),
                Codec::Opus => assert_eq!(&head[..4], [0x1A, 0x45, 0xDF, 0xA3], "WebM/EBML signature"),
            }
            assert!(info.size.is_none_or(|s| len >= s), "complete file");
            std::fs::remove_file(&dest).ok();
        }
    }
}
