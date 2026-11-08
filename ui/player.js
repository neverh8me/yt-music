// Thin playback layer. The queue, shuffle/repeat, autoplay radio, history and prefetching
// all live in Rust (src-tauri/src/player.rs + playback.rs). This file only plays the
// current track in <audio> and reports user/audio events back to Rust.
import { api, listen, streamUrl } from './api.js';

const audio = document.getElementById('audio');
const listeners = {};
const emit = (ev, data) => (listeners[ev] || []).forEach((f) => f(data));

let snap = { items: [], index: null, current: null, shuffle: false, repeat: 'off', autoplay: true, context: null, play_seq: 0 };
let loadedSeq = -1;
let failures = 0;

function apply(s, { autoplayAudio = true } = {}) {
  const prevUid = snap.current?.uid;
  snap = s;
  const cur = s.current;
  if (cur && s.play_seq !== loadedSeq) {
    loadedSeq = s.play_seq;
    audio.src = streamUrl(cur.track.id);
    if (autoplayAudio) audio.play().catch((e) => console.warn('play()', e));
    updateSession();
  }
  if (!cur) { audio.removeAttribute('src'); audio.load(); }
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
  get audio() { return audio; },
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
    if (audio.paused) audio.play().catch(() => {});
    else audio.pause();
  },
  seek(frac) {
    if (!isFinite(audio.duration)) return;
    audio.currentTime = Math.max(0, Math.min(1, frac)) * audio.duration;
  },
  seekTo(sec) { if (isFinite(sec)) audio.currentTime = Math.max(0, sec); },
  setVolume(v) {
    audio.volume = Math.max(0, Math.min(1, v));
    audio.muted = false;
    try { localStorage.setItem('volume', String(audio.volume)); } catch { /* ignore */ }
    emit('volume');
  },
  toggleMute() { audio.muted = !audio.muted; emit('volume'); },

  /** Load the saved queue from Rust without auto-starting playback. */
  async restore() {
    try { audio.volume = Number(localStorage.getItem('volume') ?? 0.8); } catch { audio.volume = 0.8; }
    audio.preload = 'metadata';
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
audio.addEventListener('play', () => emit('state'));
audio.addEventListener('pause', () => { emit('state'); reportAll(); });
audio.addEventListener('seeked', reportAll);
audio.addEventListener('durationchange', () => { reportAndroid(); reportWindows(); });
audio.addEventListener('playing', () => { failures = 0; emit('state'); reportAll(); });
audio.addEventListener('timeupdate', () => emit('time'));
audio.addEventListener('progress', () => emit('time'));
audio.addEventListener('durationchange', () => emit('time'));
audio.addEventListener('ended', () => player.next(true));
audio.addEventListener('error', () => {
  const t = player.current;
  if (!t || !audio.getAttribute('src')) return;
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
