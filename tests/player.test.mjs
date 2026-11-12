// Headless check of ui/player.js engine switching with a fake Tauri backend: node tests/player.test.mjs
import assert from 'node:assert/strict';

const calls = [];
const handlers = {};
const offline = new Set(['OFFLINE']);
let seq = 0, items = [], index = null;
const snap = () => ({ items, index, current: index === null ? null : items[index], shuffle: false, repeat: 'off', autoplay: false, context: null, play_seq: seq });
const backend = {
  stream_base: () => 'http://127.0.0.1:1/tok',
  get_settings: () => ({ web_player: true, web_player_supported: true }),
  player_state: () => snap(),
  player_play_list: ({ tracks, start }) => { items = tracks.map((t, i) => ({ uid: i + 1, track: t })); index = start; seq++; return snap(); },
  player_next: () => { if (index + 1 < items.length) { index++; seq++; return { step: 'play', state: snap() }; } return { step: 'stop', state: snap() }; },
  webplayer_load: ({ id }) => !offline.has(id),
  presence: () => null, media_update: () => null,
};
const el = { volume: 1, muted: false, paused: true, ended: false, currentTime: 0, duration: NaN, attrs: {},
  set src(v) { this.attrs.src = v; }, getAttribute(k) { return this.attrs[k] ?? null; }, removeAttribute(k) { delete this.attrs[k]; },
  load() {}, play() { this.paused = false; return Promise.resolve(); }, pause() { this.paused = true; }, addEventListener() {} };
globalThis.document = { getElementById: () => el };
globalThis.localStorage = { getItem: () => null, setItem() {} };
Object.defineProperty(globalThis, 'navigator', { value: { userAgent: 'Windows' }, configurable: true });
globalThis.window = { __TAURI__: {
  core: { invoke: async (cmd, args) => { calls.push([cmd, args]); return backend[cmd] ? backend[cmd](args || {}) : null; } },
  event: { listen: (ev, cb) => { handlers[ev] = cb; } },
} };
const tick = () => new Promise((r) => setTimeout(r, 5));
const fire = (p) => handlers['webplayer-state']({ payload: { ready: true, signed_in: true, want: '', id: '', position: 0, duration: 0, state: -1, ad: false, ends: 0, ...p } });
const count = (cmd) => calls.filter((c) => c[0] === cmd).length;

const root = new URL('../ui/', import.meta.url).href;
const { initApi } = await import(root + 'api.js');
await initApi();
const { player } = await import(root + 'player.js');
const events = [];
player.on('state', () => events.push('state'));
await player.restore();
assert.equal(player.webEngine, true);

const T = (id) => ({ id, title: id, artists: [], duration: 100 });
// guest (page not signed in): built-in engine
fire({ signed_in: false });
await player.playList([T('GUEST')], 0); await tick();
assert.ok(el.getAttribute('src').endsWith('/stream/GUEST'), 'guest plays through <audio>');
assert.equal(count('webplayer_load'), 0);
el.removeAttribute('src');
fire({ signed_in: true });
await player.playList([T('AAA'), T('OFFLINE'), T('CCC')], 0); await tick();
assert.deepEqual(calls.find((c) => c[0] === 'webplayer_load')[1], { id: 'AAA' });
assert.equal(el.getAttribute('src'), null, 'online track must not touch <audio>');
assert.equal(player.playing, true);

fire({ want: 'AAA', id: 'OLD', state: 1, position: 50, duration: 200 }); // previous video still reported
assert.equal(player.audio.currentTime, 0);
fire({ want: 'AAA', id: 'x', state: 1, ad: true, position: 3, duration: 15 });
assert.equal(player.ad, true); assert.ok(Number.isNaN(player.audio.duration));
fire({ want: 'AAA', id: 'AAA', state: 1, position: 12, duration: 180 });
assert.equal(player.ad, false); assert.equal(player.audio.currentTime, 12); assert.equal(player.audio.duration, 180);
fire({ want: 'AAA', id: 'AAA', state: 2, position: 13, duration: 180 });
assert.equal(player.playing, false);
player.toggle(); assert.equal(player.playing, true); assert.equal(count('webplayer_play'), 1);
player.seek(0.5); assert.deepEqual(calls.findLast((c) => c[0] === 'webplayer_seek')[1], { seconds: 90 });
player.setVolume(0.3); assert.deepEqual(calls.findLast((c) => c[0] === 'webplayer_volume')[1], { volume: 0.3, muted: false });

// track ended in the page -> Rust queue decides -> next track has an offline copy -> <audio>
fire({ want: 'AAA', id: 'AAA', state: 0, position: 180, duration: 180, ends: 1 }); await tick();
assert.equal(count('player_next'), 1);
assert.equal(player.current.id, 'OFFLINE');
assert.ok(el.getAttribute('src').endsWith('/stream/OFFLINE'), 'offline copy plays through <audio>');
assert.equal(count('webplayer_stop'), 1);
fire({ want: 'AAA', id: 'AAA', state: 0, ends: 1 }); await tick(); // stale page state is ignored
assert.equal(count('player_next'), 1);

await player.next(); await tick();
assert.equal(player.current.id, 'CCC');
assert.equal(el.getAttribute('src'), null);
assert.equal(calls.findLast((c) => c[0] === 'webplayer_load')[1].id, 'CCC');

// sign-in / sign-out reload the player page: the track is parked and play loads it again
player.suspend();
assert.equal(player.playing, false);
player.toggle(); await tick();
assert.equal(calls.filter((c) => c[0] === 'webplayer_load' && c[1].id === 'CCC').length, 2);

// switching the engine off moves the current track to <audio>
await player.setWebEngine(false); await tick();
assert.ok(el.getAttribute('src').endsWith('/stream/CCC'));
assert.equal(player.webEngine, false);
console.log('player.js engine switching: all checks passed');
process.exit(0);
