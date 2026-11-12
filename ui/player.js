// Thin playback layer. The queue, shuffle/repeat, autoplay radio, history and prefetching
// all live in Rust (src-tauri/src/player.rs + playback.rs). This file only plays the
// current track and reports user/audio events back to Rust.
//
// Two engines sit behind one `audio` facade (issue #17):
// - the <audio> element fed by our local stream server: phones, offline copies, fallback;
// - on desktop, YouTube Music's own web player in a hidden webview (src-tauri/src/webplayer.rs):
//   Rust drives the page and sends its state as `webplayer-state` events.
import { api, listen, streamUrl } from './api.js';

const el = document.getElementById('audio');
const listeners = {};
const emit = (ev, data) => (listeners[ev] || []).forEach((f) => f(data));

// ---- web player engine state
let webOn = false;      // engine switched on (desktop + setting)
let web = false;        // the current track plays in the web player
let webPlaying = false;
let ws = null;          // last state from the page, only while it is about our track
let lastEnds = null;    // the page's "track ended" counter
let lazyId = null;      // restored track not loaded into any engine yet (loads on play)
let watchdog = 0;
const WEB_START_TIMEOUT = 12000;
// Without an account the site stops songs part-way (seen at ~0:39, cause unknown), so the
// web player is used only while its page is signed in; guests keep the built-in engine.
let webSignedIn = false;
// Safety net for any other stall: playing, not an ad, but the position stands still.
const WEB_STALL_MS = 8000;
let progressAt = 0;
const pushVolume = () => api.wpVolume(el.volume, el.muted).catch(() => {});

// What the rest of the UI (and this file) uses instead of the <audio> element.
const audio = {
  get currentTime() { return web ? (ws && !ws.ad ? ws.position : 0) : el.currentTime; },
  set currentTime(v) {
    if (!web) { el.currentTime = v; return; }
    if (!ws || ws.ad) return; // can't seek an ad
    ws = { ...ws, position: v };
    api.wpSeek(v).catch(() => {});
    emit('time');
  },
  get duration() { return web ? (ws && !ws.ad && ws.duration > 0 ? ws.duration : NaN) : el.duration; },
  get paused() { return web ? !webPlaying : el.paused; },
  get ended() { return web ? false : el.ended; },
  get volume() { return el.volume; },
  set volume(v) { el.volume = v; if (web) pushVolume(); },
  get muted() { return el.muted; },
  set muted(m) { el.muted = m; if (web) pushVolume(); },
  play() {
    if (lazyId) { const id = lazyId; lazyId = null; load(id, true); return Promise.resolve(); }
    if (!web) return el.play();
    webPlaying = true; progressAt = Date.now(); emit('state');
    return api.wpPlay();
  },
  pause() {
    if (!web) { el.pause(); return; }
    webPlaying = false; emit('state');
    api.wpPause().catch(() => {});
  },
};

function playBuiltIn(id, autoplay) {
  if (web) api.wpStop().catch(() => {});
  web = false; ws = null;
  el.src = streamUrl(id);
  if (autoplay) el.play().catch((e) => console.warn('play()', e));
}

/** Put the current track into the right engine. */
async function load(id, autoplay) {
  const seq = loadedSeq;
  clearTimeout(watchdog);
  lazyId = null;
  if (!webOn || !webSignedIn) { playBuiltIn(id, autoplay); return; }
  // The web player starts playing as soon as it loads, so a restored (paused) track waits.
  if (!autoplay) { lazyId = id; return; }
  let handled = false;
  try { handled = await api.wpLoad(id); } catch (e) { console.warn('web player', e); }
  if (seq !== loadedSeq) return; // another track was started meanwhile
  if (!handled) { playBuiltIn(id, true); emit('state'); return; } // offline copy
  web = true; ws = null; webPlaying = true; progressAt = Date.now();
  if (el.getAttribute('src')) { el.pause(); el.removeAttribute('src'); el.load(); }
  pushVolume();
  // If the site doesn't start the song (page broken, blocked, no network to it), play it
  // with the built-in engine rather than sit silent.
  watchdog = setTimeout(() => {
    if (seq !== loadedSeq || !web || ws) return;
    console.warn('web player did not start, using the built-in engine');
    playBuiltIn(id, true); emit('state');
  }, WEB_START_TIMEOUT);
  emit('state'); emit('time');
}

/** The song stands still in the web player: carry on from the same place in <audio>. */
function checkStall() {
  if (!web || !ws || ws.ad || !webPlaying || Date.now() - progressAt < WEB_STALL_MS) return;
  const id = snap.current?.track.id, at = ws.position;
  if (!id) return;
  console.warn('web player stalled, using the built-in engine');
  playBuiltIn(id, true);
  el.addEventListener('loadedmetadata', () => { if (at > 1 && snap.current?.track.id === id) el.currentTime = at; }, { once: true });
  emit('state');
}

function onWebState(s) {
  webSignedIn = !!s.signed_in;
  if (!ws || s.position !== ws.position || s.ad || s.state === 2) progressAt = Date.now();
  const id = snap.current?.track.id;
  if (lastEnds === null || s.ends < lastEnds) lastEnds = s.ends; // first report / page reloaded
  if (!web || !id || s.want !== id) { lastEnds = s.ends; return; }
  const ended = s.ends > lastEnds;
  lastEnds = s.ends;
  if (ended) { webPlaying = false; player.next(true); return; }
  // The page reports the previous video until ours has loaded; an ad counts as ours.
  if (!s.ad && s.id !== id) return;
  const wasAd = !!ws?.ad, wasPlaying = webPlaying, first = !ws;
  ws = s;
  if (s.state === 1 || s.state === 3) webPlaying = true;       // playing / buffering
  else if (s.state === 2 || s.state === 0) webPlaying = false; // paused / ended
  if (first) failures = 0;
  if (first || wasAd !== s.ad || wasPlaying !== webPlaying) { emit('state'); reportAll(); }
  emit('time');
}

let snap = { items: [], index: null, current: null, shuffle: false, repeat: 'off', autoplay: true, context: null, play_seq: 0 };
let loadedSeq = -1;
let failures = 0;

function apply(s, { autoplayAudio = true } = {}) {
  const prevUid = snap.current?.uid;
  snap = s;
  const cur = s.current;
  if (cur && s.play_seq !== loadedSeq) {
    loadedSeq = s.play_seq;
    load(cur.track.id, autoplayAudio);
    updateSession();
  }
  if (!cur) {
    clearTimeout(watchdog); lazyId = null;
    if (web) { api.wpStop().catch(() => {}); web = false; ws = null; }
    el.removeAttribute('src'); el.load();
  }
  if (cur?.uid !== prevUid) emit('track', cur?.track || null);
  emit('queue');
  emit('state');
}

function updateSession() {
  if (!('mediaSession' in navigator) || !snap.current) return;
  const t = snap.current.track;
  navigator.mediaSession.metadata = new MediaMetadata({
    title: t.title,
    artist: (t.artists || []).map((a) => a.name).join(', '),
    album: t.album?.name || '',
    artwork: t.cover ? [{ src: t.cover, sizes: '544x544', type: 'image/jpeg' }] : [],
  });
}

async function step(reply) {
  apply(reply.state);
  if (reply.step === 'restart') { audio.currentTime = 0; audio.play().catch(() => {}); }
  else if (reply.step === 'stop') { audio.pause(); emit('state'); }
}

export const player = {
  on(ev, fn) { (listeners[ev] ||= []).push(fn); },
  /** Engine-independent stand-in for the <audio> element (time, volume, play/pause). */
  get audio() { return audio; },
  /** The real <audio> element (built-in engine only), e.g. for the equalizer. */
  get element() { return el; },
  /** YouTube is playing an ad in the web player. */
  get ad() { return web && !!ws?.ad; },
  get webEngine() { return webOn; },
  /** The player page is about to reload (sign-in, sign-out): park the track; play starts it again. */
  suspend() {
    if (!web) return;
    api.wpStop().catch(() => {});
    web = false; ws = null; lazyId = snap.current?.track.id || null;
    emit('state');
  },
  /** Switch between the web player and the built-in engine (Settings). */
  async setWebEngine(on) {
    await api.wpEnable(on);
    const cur = snap.current?.track.id, was = this.playing;
    audio.pause();
    if (web) { web = false; ws = null; }
    if (on && !webOn) listen('webplayer-state', onWebState);
    webOn = on;
    if (cur) await load(cur, was);
    emit('state'); emit('time');
  },
  get queue() { return snap.items.map((i) => i.track); },
  get index() { return snap.index ?? -1; },
  get current() { return snap.current?.track || null; },
  get shuffle() { return snap.shuffle; },
  get repeat() { return snap.repeat; },
  get autoplay() { return snap.autoplay; },
  get context() { return snap.context; },
  get playing() { return !audio.paused && !audio.ended; },

  async playList(tracks, start = 0, context = null) {
    if (!tracks.length) return;
    apply(await api.playList(tracks, start, context));
  },
  async playTrack(t) { return this.playList([t], 0, null); },
  async playNext(tracks) { apply(await api.playNext(tracks)); },
  async addToQueue(tracks) { apply(await api.addQueue(tracks)); },
  async removeAt(i) { apply(await api.queueRemove(i)); },
  async jumpTo(i) { apply(await api.queueJump(i)); },
  async next(auto = false) { await step(await api.next(auto)); },
  async prev() { await step(await api.prev(audio.currentTime || 0)); },
  async toggleShuffle() { apply(await api.toggleShuffle()); },
  async cycleRepeat() { apply(await api.cycleRepeat()); },
  async setAutoplay(on) { apply(await api.setAutoplay(on)); },

  toggle() {
    if (!snap.current) return;
    if (audio.paused) Promise.resolve(audio.play()).catch(() => {});
    else audio.pause();
  },
  seek(frac) {
    if (!isFinite(audio.duration)) return;
    audio.currentTime = Math.max(0, Math.min(1, frac)) * audio.duration;
  },
  seekTo(sec) { if (isFinite(sec)) audio.currentTime = Math.max(0, sec); },
  setVolume(v) {
    el.volume = Math.max(0, Math.min(1, v));
    el.muted = false;
    if (web) pushVolume();
    try { localStorage.setItem('volume', String(el.volume)); } catch { /* ignore */ }
    emit('volume');
  },
  toggleMute() { audio.muted = !audio.muted; emit('volume'); },

  /** Load the saved queue from Rust without auto-starting playback. */
  async restore() {
    try { el.volume = Number(localStorage.getItem('volume') ?? 0.8); } catch { el.volume = 0.8; }
    el.preload = 'metadata';
    try { const s = await api.settings(); webOn = !!(s.web_player_supported && s.web_player); } catch { webOn = false; }
    if (webOn) listen('webplayer-state', onWebState);
    setInterval(checkStall, 2000);
    apply(await api.playerState(), { autoplayAudio: false });
    emit('volume');
    // Changes made elsewhere in Rust (e.g. background radio top-up) arrive as events.
    listen('player-state', (s) => apply(s));
  },
};

// Tell Rust what's playing (Discord "Listening to"); it reads the track from its queue.
const reportPresence = () => api.presence(!audio.paused && !audio.ended, audio.currentTime || 0).catch(() => {});

// Android: mirror state into the native MediaSession (lock screen / notification player,
// headset buttons). `AndroidMedia` is injected by MainActivity.kt; absent elsewhere.
const androidMedia = window.AndroidMedia;
function reportAndroid() {
  if (!androidMedia) return;
  const t = snap.current?.track;
  if (!t) { androidMedia.stop(); return; }
  androidMedia.update(JSON.stringify({
    title: t.title,
    artist: (t.artists || []).map((a) => a.name).join(', '),
    album: t.album?.name || '',
    cover: t.cover || '',
    durationMs: Math.round((isFinite(audio.duration) ? audio.duration : t.duration || 0) * 1000),
    positionMs: Math.round((audio.currentTime || 0) * 1000),
    playing: !audio.paused && !audio.ended,
  }));
}
// Lock-screen / headset actions arrive here (called via evaluateJavascript).
window.__androidMedia = (action, arg) => {
  if (action === 'play') audio.play().catch(() => {});
  else if (action === 'pause') audio.pause();
  else if (action === 'next') player.next();
  else if (action === 'prev') player.prev();
  else if (action === 'seek') audio.currentTime = Number(arg) / 1000;
};

// Windows: the system media overlay is driven natively by Rust (named "YT Music");
// tell it what's playing, and take its play/pause/next/previous/seek back.
const isWindows = navigator.userAgent.includes('Windows');
function reportWindows() {
  if (!isWindows) return;
  const t = snap.current?.track;
  api.mediaUpdate(t ? {
    title: t.title,
    artist: (t.artists || []).map((a) => a.name).join(', '),
    album: t.album?.name || '',
    cover: t.cover || '',
    duration_ms: Math.round((isFinite(audio.duration) ? audio.duration : t.duration || 0) * 1000),
    position_ms: Math.round((audio.currentTime || 0) * 1000),
    playing: !audio.paused && !audio.ended,
  } : null).catch(() => {});
}
if (isWindows) {
  listen('media-control', ([action, arg]) => {
    if (action === 'play') audio.play().catch(() => {});
    else if (action === 'pause') audio.pause();
    else if (action === 'toggle') player.toggle();
    else if (action === 'next') player.next();
    else if (action === 'prev') player.prev();
    else if (action === 'seek') audio.currentTime = Number(arg);
  });
}

const reportAll = () => { reportPresence(); reportAndroid(); reportWindows(); };
el.addEventListener('play', () => emit('state'));
el.addEventListener('pause', () => { emit('state'); reportAll(); });
el.addEventListener('seeked', reportAll);
el.addEventListener('durationchange', () => { reportAndroid(); reportWindows(); });
el.addEventListener('playing', () => { failures = 0; emit('state'); reportAll(); });
el.addEventListener('timeupdate', () => emit('time'));
el.addEventListener('progress', () => emit('time'));
el.addEventListener('durationchange', () => emit('time'));
el.addEventListener('ended', () => player.next(true));
el.addEventListener('error', () => {
  const t = player.current;
  if (!t || !el.getAttribute('src')) return;
  failures++;
  emit('error', t);
  // Skip broken tracks, but don't spin forever if everything fails (e.g. offline).
  if (failures < 4) setTimeout(() => player.next(true), 800);
});

if ('mediaSession' in navigator) {
  const ms = navigator.mediaSession;
  ms.setActionHandler('play', () => audio.play());
  ms.setActionHandler('pause', () => audio.pause());
  ms.setActionHandler('nexttrack', () => player.next());
  ms.setActionHandler('previoustrack', () => player.prev());
  try { ms.setActionHandler('seekto', (d) => { audio.currentTime = d.seekTime; }); } catch { /* unsupported */ }
}
