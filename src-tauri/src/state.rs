use crate::db::Db;
use crate::player::Queue;
use crate::vault;
use crate::yt::{Codec, Yt};
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicU16;
use tokio::sync::Semaphore;

pub struct Core {
    pub db: Db,
    pub yt: Yt,
    pub http: reqwest::Client,
    pub data_dir: PathBuf,
    pub default_download_dir: PathBuf,
    pub port: AtomicU16,
    pub download_slots: Semaphore,
    pub downloading: Mutex<HashSet<String>>,
    pub queue: tokio::sync::Mutex<Queue>,
    /// Random per-run secret in every stream URL, so other programs on the machine can't
    /// use the local server (e.g. to pull decrypted offline tracks).
    pub stream_token: String,
    pub sleep: Mutex<SleepTimer>,
    /// "path (version)" of yt-dlp, filled in the background at startup: running
    /// `yt-dlp --version` takes seconds, so Settings must never wait for it.
    pub ytdlp_info: Mutex<Option<String>>,
    pub discord: crate::integrations::Discord,
    vault_key: Mutex<Option<vault::Key>>,
}

/// Sleep timer: stop after a deadline, or when the current track ends.
#[derive(Default)]
pub struct SleepTimer {
    /// Bumped on every change so stale timer tasks know they were cancelled.
    pub generation: u64,
    pub deadline_ms: Option<i64>,
    pub end_of_track: bool,
}

fn random_token() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("system rng");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Core {
    pub fn new(db: Db, yt: Yt, http: reqwest::Client, data_dir: PathBuf, default_download_dir: PathBuf) -> Self {
        let queue = db
            .get_setting("queue")
            .and_then(|s| serde_json::from_str::<Queue>(&s).ok())
            .unwrap_or_else(Queue::new);
        Core {
            db,
            yt,
            http,
            data_dir,
            default_download_dir,
            port: AtomicU16::new(0),
            download_slots: Semaphore::new(3),
            downloading: Default::default(),
            queue: tokio::sync::Mutex::new(queue),
            stream_token: random_token(),
            sleep: Mutex::new(SleepTimer::default()),
            ytdlp_info: Mutex::new(None),
            discord: crate::integrations::Discord::start(),
            vault_key: Mutex::new(None),
        }
    }

    pub fn download_dir(&self) -> PathBuf {
        self.db
            .get_setting("download_dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_download_dir.clone())
    }

    /// Opus by default (better quality); AAC on request.
    pub fn download_codec(&self) -> Codec {
        match self.db.get_setting("download_quality").as_deref() {
            Some("aac") => Codec::Aac,
            _ => Codec::Opus,
        }
    }

    pub fn ytdlp_dir(&self) -> PathBuf {
        self.data_dir.join("bin")
    }

    /// Per-install key for the offline vault, kept in the OS secret store.
    /// v0.3.0–0.3.2 stored it in the DB; it is moved over once and removed from the DB.
    pub fn vault_key(&self) -> Result<vault::Key> {
        let mut slot = self.vault_key.lock().unwrap();
        if let Some(k) = slot.as_ref() {
            return Ok(k.clone());
        }
        let legacy = self.db.get_setting("vault_key").and_then(|h| crate::secrets::from_hex(&h).ok());
        let raw = crate::secrets::get_or_create(&self.data_dir, "vault", legacy)?;
        if legacy.is_some() {
            if legacy == Some(raw) {
                self.db.del_setting("vault_key")?;
            } else {
                anyhow::bail!("vault key mismatch between DB and secret store");
            }
        }
        let key = vault::Key::from_raw(raw);
        *slot = Some(key.clone());
        Ok(key)
    }
}

#[cfg(test)]
pub fn test_core() -> std::sync::Arc<Core> {
    let dir = std::env::temp_dir().join("ytubem-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::sync::Arc::new(Core::new(
        Db::open(&dir.join("test.db")).unwrap(),
        Yt::new(dir.join("rp"), dir.join("bin"), &[0u8; 32]).unwrap(),
        reqwest::Client::new(),
        dir.clone(),
        dir.join("dl"),
    ))
}
