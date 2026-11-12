//! Data shapes shared with the frontend. Kept small and flat so they serialize fast.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ArtistRef {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AlbumRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Track {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration: Option<u32>,
    pub cover: Option<String>,
    /// Play count (YouTube views), when YouTube reports it (artist pages, charts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plays: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AlbumCard {
    pub id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub year: Option<u16>,
    pub cover: Option<String>,
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ArtistCard {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub subscribers: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlaylistCard {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub cover: Option<String>,
    pub count: Option<u64>,
}

#[derive(Serialize, Default)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumCard>,
    pub artists: Vec<ArtistCard>,
    pub playlists: Vec<PlaylistCard>,
}

#[derive(Serialize)]
pub struct AlbumPage {
    pub id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub year: Option<u16>,
    pub cover: Option<String>,
    pub kind: String,
    pub description: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Serialize)]
pub struct ArtistPage {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub subscribers: Option<u64>,
    pub description: Option<String>,
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumCard>,
    pub singles: Vec<AlbumCard>,
    pub playlists: Vec<PlaylistCard>,
    pub similar: Vec<ArtistCard>,
}

#[derive(Serialize)]
pub struct PlaylistPage {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub cover: Option<String>,
    pub description: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct HomeSection {
    /// i18n key, e.g. "your_mix"; `param` fills `{name}` ("More like {name}").
    pub title: String,
    pub param: Option<String>,
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumCard>,
    pub playlists: Vec<PlaylistCard>,
    pub artists: Vec<ArtistCard>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Home {
    pub sections: Vec<HomeSection>,
}

#[derive(Serialize)]
pub struct LyricsOut {
    pub body: String,
    pub footer: String,
}

#[derive(Serialize)]
pub struct LocalPlaylistSummary {
    pub id: i64,
    pub name: String,
    pub count: u32,
    pub cover: Option<String>,
}

#[derive(Serialize)]
pub struct LocalPlaylist {
    pub id: i64,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Serialize)]
pub struct DownloadEntry {
    pub track: Track,
    pub path: String,
    pub size: u64,
    pub ts: i64,
    pub codec: Option<String>,
    pub kbps: Option<u32>,
}

/// What is playing right now and in what quality (shown as a badge in the player).
#[derive(Serialize)]
pub struct StreamQuality {
    pub codec: Option<String>,
    pub kbps: Option<u32>,
    pub offline: bool,
}

#[derive(Serialize)]
pub struct Settings {
    pub download_dir: String,
    pub logged_in: bool,
    pub account_name: Option<String>,
    pub account_avatar: Option<String>,
    pub ytdlp: Option<String>,
    /// yt-dlp only exists on desktop; the UI hides its section elsewhere.
    pub ytdlp_supported: bool,
    /// "opus" or "aac".
    pub download_quality: String,
    /// The Google sign-in window is desktop-only for now.
    pub google_login_supported: bool,
    /// "auto" or a country code used for charts ("US", "GB", ...).
    pub chart_region: String,
    /// Country detected from the OS regional settings (used when "auto").
    pub detected_region: String,
    pub notify_tracks: bool,
    pub discord_presence: bool,
    /// False when this build has no Discord application id.
    pub discord_available: bool,
    pub global_hotkeys: bool,
    pub hotkeys_supported: bool,
    /// Desktop: play through YouTube Music's web player (webplayer.rs).
    pub web_player: bool,
    pub web_player_supported: bool,
}

#[derive(Serialize)]
pub struct SyncResult {
    pub liked: usize,
    pub playlists: usize,
}

#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub id: String,
    pub state: &'static str,
    pub progress: f32,
    pub error: Option<String>,
}
