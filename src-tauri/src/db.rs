//! Local library: likes, playlists, history, downloads, settings (SQLite).
use crate::models::*;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn track_from_json(s: String) -> Option<Track> {
    serde_json::from_str(&s).ok()
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS tracks (id TEXT PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS likes (track_id TEXT PRIMARY KEY, ts INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS playlists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                created INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                pos INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                PRIMARY KEY (playlist_id, pos)
            );
            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                track_id TEXT NOT NULL,
                ts INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS history_ts ON history(ts);
            CREATE TABLE IF NOT EXISTS downloads (
                track_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                size INTEGER NOT NULL,
                ts INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS cache (key TEXT PRIMARY KEY, value TEXT NOT NULL, ts INTEGER NOT NULL);
            "#,
        )?;
        // v0.3: audio quality of downloads. ALTER fails harmlessly if the column exists.
        let _ = conn.execute("ALTER TABLE downloads ADD COLUMN codec TEXT", []);
        let _ = conn.execute("ALTER TABLE downloads ADD COLUMN kbps INTEGER", []);
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn upsert_track(conn: &Connection, t: &Track) -> Result<()> {
        conn.execute(
            "INSERT INTO tracks(id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![t.id, serde_json::to_string(t)?],
        )?;
        Ok(())
    }

    fn tracks_query(&self, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Track>> {
        let conn = self.c();
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map(p, |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(track_from_json).collect())
    }

    // ---- likes
    pub fn set_liked(&self, t: &Track, liked: bool) -> Result<()> {
        let conn = self.c();
        if liked {
            Self::upsert_track(&conn, t)?;
            conn.execute(
                "INSERT OR IGNORE INTO likes(track_id, ts) VALUES (?1, ?2)",
                params![t.id, now()],
            )?;
        } else {
            conn.execute("DELETE FROM likes WHERE track_id = ?1", [&t.id])?;
        }
        Ok(())
    }

    /// Import liked tracks (oldest first so the newest ends up on top).
    pub fn import_likes(&self, tracks: &[Track]) -> Result<usize> {
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let base = now() - tracks.len() as i64;
        let mut added = 0;
        for (i, t) in tracks.iter().rev().enumerate() {
            Self::upsert_track(&tx, t)?;
            added += tx.execute(
                "INSERT OR IGNORE INTO likes(track_id, ts) VALUES (?1, ?2)",
                params![t.id, base + i as i64],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn liked_ids(&self) -> Result<Vec<String>> {
        let conn = self.c();
        let mut st = conn.prepare("SELECT track_id FROM likes")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn liked_tracks(&self) -> Result<Vec<Track>> {
        self.tracks_query(
            "SELECT t.data FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY l.ts DESC",
            [],
        )
    }

    // ---- playlists
    pub fn list_playlists(&self) -> Result<Vec<LocalPlaylistSummary>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT p.id, p.name,
                    (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id),
                    (SELECT t.data FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id
                       WHERE pt.playlist_id = p.id ORDER BY pt.pos LIMIT 1)
             FROM playlists p ORDER BY p.created DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok(LocalPlaylistSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                count: r.get(2)?,
                cover: r
                    .get::<_, Option<String>>(3)?
                    .and_then(track_from_json)
                    .and_then(|t| t.cover),
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn create_playlist(&self, name: &str, tracks: &[Track]) -> Result<i64> {
        let id = {
            let conn = self.c();
            conn.execute(
                "INSERT INTO playlists(name, created) VALUES (?1, ?2)",
                params![name, now()],
            )?;
            conn.last_insert_rowid()
        };
        self.add_to_playlist(id, tracks)?;
        Ok(id)
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        self.c()
            .execute("UPDATE playlists SET name = ?1 WHERE id = ?2", params![name, id])?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.c().execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn add_to_playlist(&self, id: i64, tracks: &[Track]) -> Result<()> {
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let mut pos: i64 = tx.query_row(
            "SELECT COALESCE(MAX(pos) + 1, 0) FROM playlist_tracks WHERE playlist_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        for t in tracks {
            Self::upsert_track(&tx, t)?;
            tx.execute(
                "INSERT INTO playlist_tracks(playlist_id, pos, track_id) VALUES (?1, ?2, ?3)",
                params![id, pos, t.id],
            )?;
            pos += 1;
        }
        tx.commit()?;
        Ok(())
    }

    /// `index` is the position in the list as shown (0-based), not the stored `pos`.
    pub fn remove_from_playlist(&self, id: i64, index: usize) -> Result<()> {
        let conn = self.c();
        let pos: Option<i64> = conn
            .query_row(
                "SELECT pos FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY pos LIMIT 1 OFFSET ?2",
                params![id, index as i64],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(pos) = pos {
            conn.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND pos = ?2",
                params![id, pos],
            )?;
        }
        Ok(())
    }

    pub fn get_playlist(&self, id: i64) -> Result<LocalPlaylist> {
        let name: String = self
            .c()
            .query_row("SELECT name FROM playlists WHERE id = ?1", [id], |r| r.get(0))?;
        let tracks = self.tracks_query(
            "SELECT t.data FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1 ORDER BY pt.pos",
            [id],
        )?;
        Ok(LocalPlaylist { id, name, tracks })
    }

    // ---- history
    pub fn record_play(&self, t: &Track) -> Result<()> {
        let conn = self.c();
        Self::upsert_track(&conn, t)?;
        conn.execute(
            "INSERT INTO history(track_id, ts) VALUES (?1, ?2)",
            params![t.id, now()],
        )?;
        // keep the table bounded
        conn.execute(
            "DELETE FROM history WHERE id < (SELECT MIN(id) FROM (SELECT id FROM history ORDER BY id DESC LIMIT 5000))",
            [],
        )?;
        Ok(())
    }

    /// Most played tracks since `since_ms`, with play counts (taste signal for recommendations).
    pub fn top_played(&self, since_ms: i64, limit: usize) -> Result<Vec<(Track, u32)>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT t.data, COUNT(*) AS c FROM history h JOIN tracks t ON t.id = h.track_id
             WHERE h.ts >= ?1 GROUP BY h.track_id ORDER BY c DESC, MAX(h.ts) DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![since_ms, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|(d, c)| Some((track_from_json(d)?, c))).collect())
    }

    /// A random handful of liked tracks.
    pub fn liked_sample(&self, limit: usize) -> Result<Vec<Track>> {
        self.tracks_query(
            "SELECT t.data FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY RANDOM() LIMIT ?1",
            [limit as i64],
        )
    }

    /// Liked tracks that haven't been played since `since_ms` ("rediscover").
    pub fn liked_not_played_since(&self, since_ms: i64, limit: usize) -> Result<Vec<Track>> {
        self.tracks_query(
            "SELECT t.data FROM likes l JOIN tracks t ON t.id = l.track_id
             WHERE NOT EXISTS (SELECT 1 FROM history h WHERE h.track_id = l.track_id AND h.ts >= ?1)
             ORDER BY RANDOM() LIMIT ?2",
            params![since_ms, limit as i64],
        )
    }

    pub fn history(&self, limit: usize) -> Result<Vec<Track>> {
        self.tracks_query(
            "SELECT t.data FROM history h JOIN tracks t ON t.id = h.track_id ORDER BY h.id DESC LIMIT ?1",
            [limit as i64],
        )
    }

    // ---- downloads
    pub fn add_download(&self, t: &Track, path: &str, size: u64, codec: &str, kbps: Option<u32>) -> Result<()> {
        let conn = self.c();
        Self::upsert_track(&conn, t)?;
        conn.execute(
            "INSERT OR REPLACE INTO downloads(track_id, path, size, ts, codec, kbps) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![t.id, path, size as i64, now(), codec, kbps],
        )?;
        Ok(())
    }

    /// Quality of a downloaded file: (codec, kbps).
    pub fn download_quality(&self, id: &str) -> Option<(Option<String>, Option<u32>)> {
        self.c()
            .query_row("SELECT codec, kbps FROM downloads WHERE track_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten()
    }

    // ---- cache (cover colours, lyrics): small JSON values keyed by string
    pub fn cache_get(&self, key: &str) -> Option<String> {
        self.c()
            .query_row("SELECT value FROM cache WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn cache_put(&self, key: &str, value: &str) -> Result<()> {
        self.c().execute(
            "INSERT OR REPLACE INTO cache(key, value, ts) VALUES (?1, ?2, ?3)",
            params![key, value, now()],
        )?;
        Ok(())
    }

    /// Drop cached entries whose key starts with `prefix` and that are older than `max_age_ms`.
    pub fn cache_prune(&self, prefix: &str, max_age_ms: i64) -> Result<()> {
        self.c().execute(
            "DELETE FROM cache WHERE substr(key, 1, length(?1)) = ?1 AND ts < ?2",
            params![prefix, now() - max_age_ms],
        )?;
        Ok(())
    }

    pub fn download_path(&self, id: &str) -> Option<String> {
        self.c()
            .query_row("SELECT path FROM downloads WHERE track_id = ?1", [id], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn remove_download(&self, id: &str) -> Result<Option<String>> {
        let path = self.download_path(id);
        self.c().execute("DELETE FROM downloads WHERE track_id = ?1", [id])?;
        Ok(path)
    }

    pub fn downloads(&self) -> Result<Vec<DownloadEntry>> {
        let conn = self.c();
        let mut st = conn.prepare(
            "SELECT t.data, d.path, d.size, d.ts, d.codec, d.kbps FROM downloads d JOIN tracks t ON t.id = d.track_id
             ORDER BY d.ts DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<u32>>(5)?,
            ))
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter_map(|(d, path, size, ts, codec, kbps)| {
                // Older downloads (before quality tracking) were always AAC m4a.
                let codec = codec.or_else(|| path.ends_with(".m4a").then(|| "AAC".to_string()));
                Some(DownloadEntry { track: track_from_json(d)?, path, size: size as u64, ts, codec, kbps })
            })
            .collect())
    }

    pub fn downloaded_ids(&self) -> Result<Vec<String>> {
        let conn = self.c();
        let mut st = conn.prepare("SELECT track_id FROM downloads")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ---- settings
    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.c()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.c().execute(
            "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn del_setting(&self, key: &str) -> Result<()> {
        self.c().execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }
}
