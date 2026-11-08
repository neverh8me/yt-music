// Runs yt-dlp's stream-challenge solver (ui/ejs, Unlicense) in this webview's JS engine
// (V8 / JavaScriptCore): far faster than QuickJS for parsing YouTube's player.
// Rust sends {id, input} via the "js-solve" event; app.js forwards it here and the result back.
/* global lib, jsc */
importScripts('ejs/yt.solver.lib.min.js');
Object.assign(self, lib);
importScripts('ejs/yt.solver.core.min.js');
onmessage = (e) => {
  const { id, input } = e.data;
  let out;
  try { out = JSON.stringify(jsc(JSON.parse(input))); } catch (err) { out = JSON.stringify({ type: 'error', error: String(err) }); }
  postMessage({ id, out });
};
