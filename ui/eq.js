// 10-band equalizer on Web Audio: <audio> → preamp → 10 biquad filters → speakers.
// The audio graph is only built the first time the EQ is switched on (a user gesture),
// so playback is untouched for people who never use it. The stream server sends CORS
// headers to the app origin, which is what lets Web Audio read the samples.
export const FREQS = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
export const PRESETS = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  bass: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0],
  vocal: [-2, -1, 0, 2, 4, 4, 3, 1, 0, -1],
  electronic: [5, 4, 1, 0, -2, 1, 0, 2, 4, 5],
  rock: [4, 3, 1, -1, -2, 0, 2, 3, 4, 4],
  acoustic: [3, 2, 1, 1, 2, 2, 3, 3, 2, 1],
};

const KEY = 'eq';
const load = () => {
  try {
    const s = JSON.parse(localStorage.getItem(KEY) || 'null');
    if (s && Array.isArray(s.gains) && s.gains.length === 10) return s;
  } catch { /* ignore */ }
  return { on: false, preset: 'flat', gains: PRESETS.flat.slice(), pre: 0 };
};
export const eqState = load();
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(eqState)); } catch { /* ignore */ } };

let ctx = null, pre = null, bands = [];
const db = (v) => Math.pow(10, v / 20);

function build(audio) {
  if (ctx) return;
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return;
  ctx = new AC();
  const src = ctx.createMediaElementSource(audio);
  pre = ctx.createGain();
  bands = FREQS.map((f, i) => {
    const b = ctx.createBiquadFilter();
    b.type = i === 0 ? 'lowshelf' : i === FREQS.length - 1 ? 'highshelf' : 'peaking';
    b.frequency.value = f;
    b.Q.value = 1.1;
    return b;
  });
  src.connect(pre);
  bands.reduce((node, b) => (node.connect(b), b), pre).connect(ctx.destination);
  // Resume with playback (contexts start suspended until a gesture on some platforms).
  audio.addEventListener('play', () => ctx.state === 'suspended' && ctx.resume());
}

function apply() {
  if (!ctx) return;
  const on = eqState.on;
  pre.gain.value = on ? db(eqState.pre) : 1;
  bands.forEach((b, i) => { b.gain.value = on ? eqState.gains[i] : 0; });
}

export const eq = {
  /** Call once at startup: rebuilds the graph if the EQ was left on. */
  init(audio) {
    this.audio = audio;
    if (eqState.on) {
      // Needs a gesture before the context can run; build lazily on the first play.
      const once = () => { build(audio); apply(); ctx?.resume(); audio.removeEventListener('play', once); };
      audio.addEventListener('play', once);
    }
  },
  setOn(on) {
    eqState.on = on;
    if (on) { build(this.audio); ctx?.resume(); }
    apply(); save();
  },
  setPreset(name) {
    if (!PRESETS[name]) return;
    eqState.preset = name;
    eqState.gains = PRESETS[name].slice();
    apply(); save();
  },
  setGain(i, v) { eqState.gains[i] = v; eqState.preset = 'custom'; apply(); save(); },
  setPreamp(v) { eqState.pre = v; apply(); save(); },
};
