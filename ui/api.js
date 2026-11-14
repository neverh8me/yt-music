// Thin wrapper over Tauri commands. Every backend call goes through here.
const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
export const listen = (ev, cb) => T.event.listen(ev, (e) => cb(e.payload));

let base = '';
export async function initApi() {
  base = await invoke('stream_base');
}
export const streamUrl = (id) => `${base}/stream/${encodeURIComponent(id)}`;

export const api = {
  // YouTube Music
  search: (query) => invoke('search', { query }),
  suggestions: (query) => invoke('search_suggestions', { query }),
  home: () => invoke('home'),
  album: (id) => invoke('get_album', { id }),
  artist: (id) => invoke('get_artist', { id }),
  artistReleases: (id) => invoke('get_artist_releases', { id }),
  playlist: (id) => invoke('get_playlist', { id }),
  radio: (id) => invoke('get_radio', { id }),
  lyrics: (track) => invoke('get_lyrics', { track }),
  coverColor: (url) => invoke('cover_color', { url }),
  streamQuality: (id) => invoke('stream_quality', { id }),
  prefetch: (ids) => invoke('prefetch', { ids }),
  mediaUpdate: (np) => invoke('media_update', { np }),
  jsSolveDone: (id, out) => invoke('js_solve_done', { id, out }),
  warmSolver: () => invoke('warm_solver'),

  // Player (queue engine lives in Rust)
  playerState: () => invoke('player_state'),
  playList: (tracks, start, context) => invoke('player_play_list', { tracks, start, context }),
  playNext: (tracks) => invoke('player_play_next', { tracks }),
  addQueue: (tracks) => invoke('player_add_queue', { tracks }),
  queueRemove: (index) => invoke('player_remove', { index }),
  queueJump: (index) => invoke('player_jump', { index }),
  next: (auto) => invoke('player_next', { auto }),
  prev: (position) => invoke('player_prev', { position }),
  toggleShuffle: () => invoke('player_toggle_shuffle'),
  cycleRepeat: () => invoke('player_cycle_repeat'),
  setAutoplay: (on) => invoke('player_set_autoplay', { on }),
  setSleepTimer: (minutes, endOfTrack = false) => invoke('set_sleep_timer', { minutes, endOfTrack }),
  sleepState: () => invoke('sleep_timer_state'),
  presence: (playing, position) => invoke('presence', { playing, position }),
  setToggle: (key, on) => invoke('set_toggle', { key, on }),

  // Web player engine (desktop): YouTube Music's own player in a hidden webview
  wpLoad: (id) => invoke('webplayer_load', { id }),
  wpPlay: () => invoke('webplayer_play'),
  wpPause: () => invoke('webplayer_pause'),
  wpStop: () => invoke('webplayer_stop'),
  wpSeek: (seconds) => invoke('webplayer_seek', { seconds }),
  wpVolume: (volume, muted) => invoke('webplayer_volume', { volume, muted }),
  wpEnable: (on) => invoke('webplayer_enable', { on }),

  // Library
  likedTracks: () => invoke('liked_tracks'),
  likedIds: () => invoke('liked_ids'),
  setLiked: (track, liked) => invoke('set_liked', { track, liked }),
  playlists: () => invoke('list_playlists'),
  localPlaylist: (id) => invoke('get_local_playlist', { id }),
  createPlaylist: (name, tracks = []) => invoke('create_playlist', { name, tracks }),
  renamePlaylist: (id, name) => invoke('rename_playlist', { id, name }),
  deletePlaylist: (id) => invoke('delete_playlist', { id }),
  addToPlaylist: (id, tracks) => invoke('add_to_playlist', { id, tracks }),
  removeFromPlaylist: (id, pos) => invoke('remove_from_playlist', { id, pos }),
  history: () => invoke('get_history'),

  // Downloads
  download: (tracks) => invoke('download_tracks', { tracks }),
  downloads: () => invoke('list_downloads'),
  downloadedIds: () => invoke('downloaded_ids'),
  deleteDownload: (id) => invoke('delete_download', { id }),

  // Account & settings
  settings: () => invoke('get_settings'),
  setDownloadDir: (path) => invoke('set_download_dir', { path }),
  setDownloadQuality: (quality) => invoke('set_download_quality', { quality }),
  setChartRegion: (region) => invoke('set_chart_region', { region }),
  googleLogin: () => invoke('google_login'),
  login: (text) => invoke('login_cookies', { text }),
  logout: () => invoke('logout'),
  refreshAccount: () => invoke('refresh_account'),
  syncAccount: () => invoke('sync_account'),
  remotePlaylists: () => invoke('remote_playlists'),
  installYtdlp: () => invoke('install_ytdlp'),

  // Updates (desktop: verified self-update; Android: opens the new APK)
  checkUpdate: () => invoke('check_update'),
  installUpdate: () => invoke('install_update'),
};
