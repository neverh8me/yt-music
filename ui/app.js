import { api, initApi, listen } from './api.js';
import { icon, hydrateIcons, setIcon } from './icons.js';
import { player } from './player.js';
import { t, translateDom, LANGS, lang, setLang, locale } from './i18n.js';
import { eq, eqState, FREQS, PRESETS } from './eq.js';

// ---------------------------------------------------------------- helpers
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const fmt = (sec) => {
  if (!isFinite(sec) || sec < 0) return '0:00';
  sec = Math.floor(sec);
  const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
  return h ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
};
const totalTime = (tracks) => {
  const s = tracks.reduce((a, x) => a + (x.duration || 0), 0);
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return h ? t('time.hm', { h, m }) : t('time.m', { m });
};
const tracksLabel = (n) => t('tracks', { n });
const kindLabel = (k) => (k ? t(`kind.${k}`) : '');
const compact = (n) => new Intl.NumberFormat(locale(), { notation: 'compact', maximumFractionDigits: 1 }).format(n);
const subsLabel = (n) => (n == null ? '' : t('subscribers', { n: compact(n) }));
// Themes are token sets in styles.css: "glass" (default, PR #1 by @dopedasya) and "classic".
const getPref = (k, d) => { try { return localStorage.getItem(k) || d; } catch { return d; } };
function setPref(k, v) { try { localStorage.setItem(k, v); } catch { /* ignore */ } window.__applyTheme?.(); }
const regionName = (code) => {
  try { return new Intl.DisplayNames([locale()], { type: 'region' }).of(code) || code; } catch { return code; }
};
const COVER_PLACEHOLDER = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='-12 -12 48 48' fill='none' stroke='%236f6f7c' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Crect x='-12' y='-12' width='48' height='48' fill='%231c1c22' stroke='none'/%3E%3Cpath d='M9 18V5l12-2v13'/%3E%3Ccircle cx='6' cy='18' r='3'/%3E%3Ccircle cx='18' cy='16' r='3'/%3E%3C/svg%3E";
const GOOGLE_G = '<svg class="g-logo" viewBox="0 0 48 48" aria-hidden="true"><path fill="#EA4335" d="M24 9.5c3.5 0 6.6 1.2 9.1 3.6l6.8-6.8C35.8 2.4 30.3 0 24 0 14.6 0 6.6 5.4 2.6 13.3l7.9 6.2C12.4 13.7 17.7 9.5 24 9.5z"/><path fill="#4285F4" d="M46.1 24.5c0-1.6-.1-3.1-.4-4.5H24v9h12.4c-.5 2.9-2.2 5.4-4.6 7l7.1 5.5c4.2-3.9 7.2-9.6 7.2-17z"/><path fill="#FBBC05" d="M10.5 28.6A14.6 14.6 0 0 1 9.5 24c0-1.6.3-3.2.8-4.6l-7.9-6.2A24 24 0 0 0 0 24c0 3.9.9 7.5 2.6 10.7l7.9-6.1z"/><path fill="#34A853" d="M24 48c6.5 0 11.9-2.1 15.9-5.8l-7.1-5.5c-2 1.4-4.6 2.2-8.8 2.2-6.3 0-11.6-4.2-13.5-10l-7.9 6.1C6.6 42.6 14.6 48 24 48z"/></svg>';
const artistLinks = (artists = []) => artists.map((a) =>
  a.id ? `<a class="link" data-go="artist" data-id="${esc(a.id)}">${esc(a.name)}</a>` : esc(a.name)).join(', ');
const artistNames = (artists = []) => artists.map((a) => a.name).join(', ');
const errText = (e) => (typeof e === 'string' ? e : e?.message || JSON.stringify(e));
const debounce = (fn, ms) => { let h; return (...a) => { clearTimeout(h); h = setTimeout(() => fn(...a), ms); }; };
const isPhone = () => matchMedia('(max-width: 760px)').matches;
/** Android / iOS app: no yt-dlp, 18+ songs go through the signed-in stream and the JS solver. */
const MOBILE_APP = !!window.AndroidMedia || /iPhone|iPad|iPod/.test(navigator.userAgent);
/** The player alone on screen (narrow window, phone): Up next / Lyrics open as a sheet over it. */
const stageCompact = () => matchMedia('(max-width: 1199px)').matches;
/** googleusercontent covers come at 544 px; list rows only need a small one (less to load and decode). */
const thumb = (url, px = 120) => (url && /googleusercontent\.com|ggpht\.com/.test(url) ? url.replace(/=w\d+-h\d+[^/]*$/, `=w${px}-h${px}-l90-rj`) : url || '');
const reducedMotion = () => document.documentElement.dataset.motion === 'reduce';

function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.textContent = msg;
  $('#toasts').appendChild(el);
  setTimeout(() => el.classList.add('out'), 2800);
  setTimeout(() => el.remove(), 3200);
}

// ---------------------------------------------------------------- state
const state = {
  liked: new Set(),
  downloaded: new Set(),
  dlProgress: new Map(), // id -> {state, progress}
  playlists: [],
  remotePlaylists: [],
  settings: {},
  lists: new Map(), // list id -> tracks (for event delegation)
  listSeq: 0,
};

function registerList(tracks) {
  const id = `l${++state.listSeq}`;
  state.lists.set(id, tracks);
  if (state.lists.size > 60) state.lists.delete(state.lists.keys().next().value);
  return id;
}

async function refreshLibrary() {
  const [liked, dl, pls] = await Promise.all([api.likedIds(), api.downloadedIds(), api.playlists()]);
  state.liked = new Set(liked);
  state.downloaded = new Set(dl);
  state.playlists = pls;
  renderDock();
}

async function refreshRemote() {
  try { state.remotePlaylists = await api.remotePlaylists(); } catch { state.remotePlaylists = []; }
  renderDock();
}

// ---------------------------------------------------------------- stage ↔ browse
// "stage": the playing track fills the screen. "browse": a page is open over it and the
// stage is collapsed into the capsule player. The cover flies between the two (FLIP).
const mode = () => document.body.dataset.mode;
function setMode(m) {
  document.body.dataset.mode = m;
  if (m === 'browse') ensureBackEntry();
  markRail();
}
function morph(to) {
  if (mode() === to) return;
  const fromImg = to === 'browse' ? $('#stage-art') : $('#cap-art');
  const r1 = fromImg.getBoundingClientRect();
  setMode(to);
  const src = player.current?.cover;
  if (reducedMotion() || !r1.width || !src || document.body.classList.contains('idle')) return;
  let r2;
  if (to === 'browse') {
    // The capsule is still sliding in; use its resting place.
    const cap = $('#capsule'), app = $('#app').getBoundingClientRect(), art = $('#cap-art');
    const left = app.left + cap.offsetLeft + art.offsetLeft;
    r2 = { left, top: app.top + cap.offsetTop + art.offsetTop, width: art.offsetWidth, height: art.offsetHeight };
  } else {
    r2 = $('#stage-art').getBoundingClientRect();
  }
  const g = document.createElement('img');
  g.className = 'ghost-art';
  g.src = src;
  Object.assign(g.style, { left: `${r1.left}px`, top: `${r1.top}px`, width: `${r1.width}px`, height: `${r1.height}px` });
  document.body.appendChild(g);
  if (to === 'browse') closeSide();
  const hide = to === 'browse' ? $('#cap-art') : $('.stage-art-wrap');
  hide.style.visibility = 'hidden';
  const s = r2.width / r1.width;
  const rFrom = to === 'browse' ? 20 : r1.width / 2, rTo = to === 'browse' ? r2.width / 2 / s : 20 / s;
  const anim = g.animate([
    { transform: 'translate(0,0) scale(1)', borderRadius: `${rFrom}px` },
    { transform: `translate(${r2.left - r1.left}px,${r2.top - r1.top}px) scale(${s})`, borderRadius: `${rTo}px` },
  ], { duration: 320, easing: 'cubic-bezier(.2,.8,.2,1)', fill: 'forwards' });
  const done = () => { g.remove(); hide.style.visibility = ''; };
  anim.onfinish = done; anim.oncancel = done;
}
function toStage() {
  if (document.body.classList.contains('idle')) return;
  morph('stage');
  document.body.classList.remove('searching');
}

// Android back button / mouse back: one history entry while a layer or overlay is open.
let backArmed = false;
function ensureBackEntry() {
  if (backArmed) return;
  try { history.pushState({ ytm: true }, ''); backArmed = true; } catch { /* ignore */ }
}
window.addEventListener('popstate', () => {
  backArmed = false;
  if (closeOverlays()) { if (mode() === 'browse') ensureBackEntry(); return; }
  if (closeSide()) return;
  if (mode() === 'browse') {
    if (nav.pos > 0) { nav.pos--; render(); ensureBackEntry(); }
    else toStage();
  }
});

// ---------------------------------------------------------------- router (browse layer)
const nav = { stack: [], pos: -1 };
let renderToken = 0;
const LAYERS = ['library', 'history', 'downloads'];
const layerName = (v) => ({ home: t('layer.forYou'), library: t('nav.library'), history: t('lib.history'), downloads: t('lib.downloads') }[v]);

function go(view, params = {}, push = true) {
  if (push) {
    nav.stack = nav.stack.slice(0, nav.pos + 1);
    nav.stack.push({ view, params, scroll: 0 });
    nav.pos = nav.stack.length - 1;
  }
  if (mode() !== 'browse') morph('browse');
  render();
}

const skeleton = () => `<div class="loading" aria-busy="true"><div class="sk" style="height:24px;width:220px"></div>
  <div class="sk-grid">${'<div class="sk" style="aspect-ratio:1"></div>'.repeat(6)}</div>${'<div class="sk" style="height:48px"></div>'.repeat(4)}</div>`;
const layerBar = (active) => `<div class="layers-bar"><div class="tabs" role="tablist">${LAYERS.map((v) =>
  `<button class="tab" role="tab" data-nav="${v}" aria-selected="${v === active}">${esc(layerName(v))}</button>`).join('')}</div>
  ${player.current ? `<button class="link" data-act="to-stage">${esc(t('layer.collapse'))} ↓</button>` : ''}</div>`;

async function render() {
  const entry = nav.stack[nav.pos];
  if (!entry) return;
  const token = ++renderToken;
  const el = $('#view');
  markRail();
  const sk = setTimeout(() => { if (token === renderToken) el.innerHTML = skeleton(); }, 120);
  pageColor = null;
  try {
    const html = await (views[entry.view] || views.home)(entry.params);
    clearTimeout(sk);
    if (token !== renderToken) return;
    // The four layers share a tab bar at the top.
    el.innerHTML = (LAYERS.includes(entry.view) ? `<div class="page layer-head">${layerBar(entry.view)}</div>` : '') + html;
    $('#main').scrollTop = entry.scroll || 0;
    applyPageColor(token);
    if (['liked', 'local-playlist', 'playlist', 'album'].includes(entry.view) && autoDl.has(dlKey(pageCtx))) downloadTracks(pageTracks, true);
    afterRender[entry.view]?.(entry.params);
  } catch (e) {
    clearTimeout(sk);
    if (token !== renderToken) return;
    console.error(e);
    el.innerHTML = `<div class="page"><div class="empty">${icon('music')}<h2>${esc(t('err.title'))}</h2><p>${esc(errText(e))}</p>
      <button class="pill-btn primary" data-act="reload">${esc(t('err.reload'))}</button></div></div>`;
  }
}

$('#main').addEventListener('scroll', () => {
  const e = nav.stack[nav.pos];
  if (e) e.scroll = $('#main').scrollTop;
});

// ---------------------------------------------------------------- components
const dlMarkDone = () => `<span class="dl-mark" title="${esc(t('tr.downloaded'))}">${icon('downloaded')}</span>`;
const dlMarkBusy = (p) => `<span class="dl-mark busy" title="${esc(t('tr.downloading'))}">${Math.round((p || 0) * 100)}%</span>`;

function trackRows(tracks, opts = {}) {
  const lid = registerList(tracks);
  const cur = player.current?.id;
  const rows = tracks.map((x, i) => {
    const liked = state.liked.has(x.id);
    const prog = state.dlProgress.get(x.id);
    const dlIcon = state.downloaded.has(x.id) ? dlMarkDone() : prog && prog.state !== 'error' ? dlMarkBusy(prog.progress) : '';
    const playing = x.id === cur;
    return `<div class="trow ${playing ? 'playing' : ''} ${playing && !player.playing ? 'paused' : ''}" data-list="${lid}" data-idx="${i}" data-tid="${esc(x.id)}" tabindex="0">
      <div class="tnum"><span class="n">${opts.numbers === false ? '' : i + 1}</span><span class="eq"><b></b><b></b><b></b></span>${icon('play', 'pl')}</div>
      <div class="tmain">
        ${opts.covers === false ? '' : `<img class="tcover" loading="lazy" decoding="async" src="${esc(thumb(x.cover))}" alt="" data-onerr="hide">`}
        <div class="ttext"><div class="ttitle">${esc(x.title)}</div><div class="tartist">${dlIcon}${opts.badges?.[x.id] ? `<span class="qbadge">${esc(opts.badges[x.id])}</span>` : ''}<span>${artistLinks(x.artists)}</span></div></div>
      </div>
      ${opts.album === false ? '' : `<div class="talbum">${opts.plays && x.plays ? `<span class="plays">${esc(t('tr.plays', { n: compact(x.plays) }))}</span>` : x.album ? `<a class="link" data-go="album" data-id="${esc(x.album.id)}">${esc(x.album.name)}</a>` : ''}</div>`}
      <div class="tright">
        <button class="icon-btn like ${liked ? 'on' : ''}" data-act="like" aria-pressed="${liked}" title="${esc(t(liked ? 'menu.unlike' : 'tr.like'))}" aria-label="${esc(t(liked ? 'menu.unlike' : 'tr.like'))}">${icon(liked ? 'heart-fill' : 'heart')}</button>
        <span class="tdur">${x.duration ? fmt(x.duration) : ''}</span>
        <button class="icon-btn more" data-act="menu" aria-haspopup="menu" title="${esc(t('tr.more'))}" aria-label="${esc(t('tr.more'))}">${icon('more')}</button>
      </div>
    </div>`;
  }).join('');
  const noAlbum = opts.album === false;
  return `<div class="tracklist ${noAlbum ? 'noalbum' : ''}" data-ctx='${esc(JSON.stringify(opts.ctx || null))}'>${rows}</div>`;
}

function card(kind, item) {
  const round = kind === 'artist';
  const title = item.title || item.name;
  let sub = '';
  if (kind === 'album') sub = [item.year, kindLabel(item.kind), artistNames(item.artists)].filter(Boolean).join(' · ');
  if (kind === 'playlist') sub = item.author || (item.count ? tracksLabel(item.count) : t('kind.playlist'));
  if (kind === 'artist') sub = item.subscribers != null ? subsLabel(item.subscribers) : t('kind.artist');
  if (kind === 'local') sub = tracksLabel(item.count || 0);
  const cover = item.cover
    ? `<img loading="lazy" decoding="async" src="${esc(thumb(item.cover, 360))}" alt="" data-onerr="remove">`
    : `<div class="ph">${icon(kind === 'artist' ? 'user' : 'music')}</div>`;
  return `<div class="card" data-go="${kind === 'local' ? 'local-playlist' : kind}" data-id="${esc(item.id)}" tabindex="0">
    <div class="card-art ${round ? 'round' : ''}">${cover}
      ${kind !== 'artist' ? `<button class="card-play" data-act="play-collection" data-kind="${kind}" data-id="${esc(item.id)}" title="${esc(t('act.play'))}" aria-label="${esc(t('act.play'))} ${esc(title)}">${icon('play')}</button>` : ''}
    </div>
    <div class="card-title">${esc(title)}</div>
    <div class="card-sub">${esc(sub)}</div>
  </div>`;
}
/** A card for a built-in collection (liked, downloads, history): coloured tile instead of a cover. */
const specialCard = (navTo, cls, ic, title, sub) => `<div class="card" data-nav="${navTo}" tabindex="0">
  <div class="card-art"><div class="ph qc ${cls}">${icon(ic)}</div></div><div class="card-title">${esc(title)}</div><div class="card-sub">${esc(sub)}</div></div>`;

const secHead = (title, label = '', right = '') => `<div class="sec-head"><div>${label ? `<div class="sec-label">${esc(label)}</div>` : ''}<h2>${esc(title)}</h2></div>${right}</div>`;

const scrollBtns = () => `<div class="scroll-btns"><button class="iconbtn" data-act="scroll" data-dir="-1" aria-label="${esc(t('nav.back'))}">${icon('chev-left')}</button><button class="iconbtn" data-act="scroll" data-dir="1" aria-label="${esc(t('nav.forward'))}">${icon('chev-right')}</button></div>`;
/** One horizontally scrolling row of cards. */
const carousel = (title, items, kind) => !items?.length ? '' :
  `<section class="shelf">${secHead(title, '', scrollBtns())}<div class="carousel-track scroller">${items.map((i) => card(kind, i)).join('')}</div></section>`;
const shelf = (title, items, kind, label = '') => !items?.length ? '' :
  `<section class="shelf">${secHead(title, label)}<div class="grid">${items.map((i) => card(kind, i)).join('')}</div></section>`;

function collectionHeader({ cover, kind, title, meta, round = false, actions = '', phClass = '' }) {
  return `<div class="hero">
    <div class="hero-art ${round ? 'round' : ''}">${cover ? `<img src="${esc(cover)}" alt="" data-onerr="fallback" data-fallback="${esc(pageTracks[0]?.cover || '')}">` : `<div class="ph qc ${phClass}">${icon(phClass ? (phClass === 'liked-cover' ? 'heart-fill' : 'download') : 'music')}</div>`}</div>
    <div class="hero-text">
      <div class="hero-kind">${esc(kind)}</div>
      <h1 class="${title.length > 30 ? 'long' : ''}">${esc(title)}</h1>
      <div class="hero-meta">${meta}</div>
    </div>
  </div>
  <div class="actions">${actions}</div>`;
}

const playBtn = (act, extra = '') => `<button class="play-big" data-act="${act}" ${extra} title="${esc(t('act.play'))}" aria-label="${esc(t('act.play'))}">${icon('play')}</button>`;
const actBtn = (act, ic, title, extra = '') => `<button class="pill-btn" data-act="${act}" ${extra}>${icon(ic)}${esc(title)}</button>`;
const collectionActions = () => {
  const on = autoDl.has(dlKey(pageCtx));
  return playBtn('play-page') + actBtn('shuffle-page', 'shuffle', t('act.shuffle'))
    + `<button class="pill-btn dl-toggle ${on ? 'on' : ''}" data-act="download-page" aria-pressed="${on}">${dlToggleInner(on)}</button>`;
};

// cached data of the currently rendered collection page, used by action buttons
let pageTracks = [];
let pageCtx = null;
// recently played tracks shown on the home page (seed for "Start radio")
let homeRecent = [];
// Page tint: a cover URL (colour computed in Rust) or a fixed '#hex'; set by views.
let pageColor = null;

async function applyPageColor(token) {
  const main = $('#main');
  const src = pageColor;
  if (!src) { main.classList.remove('tinted'); main.style.removeProperty('--page-color'); return; }
  try {
    const c = src.startsWith('#') ? src : await api.coverColor(src);
    if (token !== renderToken) return;
    main.style.setProperty('--page-color', c);
    main.classList.add('tinted');
  } catch { main.classList.remove('tinted'); }
}

// ---------------------------------------------------------------- views (browse layer)
const views = {
  async home() {
    const [hist, home] = await Promise.all([
      api.history().catch(() => []),
      api.home().catch((e) => ({ sections: [], error: errText(e) })),
    ]);
    const recent = [];
    const seen = new Set();
    for (const x of hist) { if (!seen.has(x.id)) { seen.add(x.id); recent.push(x); } if (recent.length >= 16) break; }
    homeRecent = recent;
    const hour = new Date().getHours();
    const greet = t(hour < 5 ? 'greet.night' : hour < 12 ? 'greet.morning' : hour < 18 ? 'greet.day' : 'greet.evening');
    const name = state.settings.logged_in && state.settings.account_name ? `, ${state.settings.account_name.split(' ')[0]}` : '';
    const quick = [
      { nav: 'liked', title: t('lib.liked'), cls: 'liked-cover', ic: 'heart-fill' },
      ...state.playlists.slice(0, 5).map((p) => ({ go: 'local-playlist', id: p.id, title: p.name, cover: p.cover })),
    ];
    const sections = home.sections || [];
    const titleOf = (s) => t(`sec.${s.title}`, { name: s.param || '' });
    // Personal sections (built in Rust from your history and likes) become big "mix" cards.
    const PERSONAL = ['your_mix', 'artist_mix', 'more_like', 'your_artists', 'rediscover'];
    const mixes = sections.filter((s) => PERSONAL.includes(s.title) && s.tracks?.length);
    const MIX = ['#ff2e4d', '#5b8cff', '#9e73ff', '#ff8a3d', '#1f9d5a', '#e0457b'];
    let html = `<div class="page home">
      <div class="home-head"><h1>${esc(greet + name)}</h1>
        <div class="actions">
          <button class="pill-btn primary big" data-act="play-liked" ${state.liked.size ? '' : 'disabled'}>${icon('play')}${esc(t('home.playLiked'))}</button>
          ${recent.length ? `<button class="pill-btn big" data-act="radio-recent">${icon('radio')}${esc(t('home.startRadio'))}</button>` : ''}
        </div></div>
      <div class="quick">${quick.map((q) => `<div class="quick-item" ${q.nav ? `data-nav="${q.nav}"` : `data-go="${q.go}" data-id="${esc(q.id ?? '')}"`} tabindex="0">
        ${q.cover ? `<img src="${esc(thumb(q.cover))}" alt="">` : `<span class="qc ${q.cls || ''}">${icon(q.ic || 'music')}</span>`}
        <span>${esc(q.title)}</span></div>`).join('')}</div>`;
    if (mixes.length) {
      html += `<section class="shelf">${secHead(t('home.madeForYou'), '', scrollBtns())}<div class="mixes scroller">${mixes.map((s, i) => {
        const lid = registerList(s.tracks);
        const cover = s.tracks.find((x) => x.cover)?.cover;
        const artists = [...new Set(s.tracks.flatMap((x) => (x.artists || []).map((a) => a.name)))].slice(0, 3).join(', ');
        return `<div class="mix-card" style="--mix:${MIX[i % MIX.length]}" data-act="play-list" data-list="${lid}" data-title="${esc(titleOf(s))}" tabindex="0" role="button">
          ${cover ? `<img src="${esc(cover)}" alt="" data-onerr="remove">` : ''}<span class="mix-label">${esc(t('home.madeForYou'))}</span><b>${esc(titleOf(s))}</b><small>${esc(artists)}</small></div>`;
      }).join('')}</div></section>`;
    }
    // Quick picks: a swipeable grid of songs picked for you (first personal mix, else recent).
    const picks = mixes[0]?.tracks || [];
    if (picks.length) html += `<section class="shelf">${secHead(t('home.quick'), '', scrollBtns())}<div class="picks scroller">${trackRows(picks.slice(0, 20), { album: false })}</div></section>`;
    if (recent.length) html += `<section class="shelf">${secHead(t('home.recent'), '', scrollBtns())}<div class="picks scroller">${trackRows(recent, { album: false })}</div></section>`;
    for (const s of sections) {
      if (mixes.includes(s)) continue;
      const title = titleOf(s);
      if (s.tracks?.length) {
        html += s.title === 'top_tracks'
          ? `<section class="shelf chart">${secHead(title, t('home.chartLabel'))}${trackRows(s.tracks.slice(0, 10), { album: false })}</section>`
          : `<section class="shelf">${secHead(title, '', scrollBtns())}<div class="picks scroller">${trackRows(s.tracks.slice(0, 20), { album: false })}</div></section>`;
      }
      html += carousel(title, s.albums, 'album');
      html += carousel(title, s.playlists, 'playlist');
      html += carousel(title, s.artists, 'artist');
    }
    if (home.error) html += `<div class="note">${esc(t('home.recoFail', { e: home.error }))}</div>`;
    return html + '</div>';
  },

  async search({ q }) {
    if (!q) {
      const hist = (() => { try { return JSON.parse(localStorage.getItem('searches') || '[]'); } catch { return []; } })();
      return `<div class="page"><h1>${esc(t('search.title'))}</h1>
        ${hist.length ? `<section class="shelf"><h2>${esc(t('search.recent'))}</h2><div class="chips">${hist.map((s) => `<button class="chip" data-search="${esc(s)}">${esc(s)}</button>`).join('')}</div></section>`
        : `<p class="muted">${esc(t('search.hint'))}</p>`}</div>`;
    }
    const r = await api.search(q);
    const top = r.tracks.slice(0, 20);
    let html = `<div class="page"><h1 class="search-h">${esc(t('search.results', { q }))}</h1>`;
    if (!top.length && !r.albums.length && !r.artists.length && !r.playlists.length) {
      return html + `<div class="empty">${icon('search')}<h2>${esc(t('search.none'))}</h2></div></div>`;
    }
    if (r.artists[0]) {
      const a = r.artists[0];
      html += `<div class="top-result-row"><div class="top-result" data-go="artist" data-id="${esc(a.id)}" tabindex="0">
        ${a.cover ? `<img class="round" src="${esc(a.cover)}" alt="">` : ''}
        <div class="tr-name">${esc(a.name)}</div><div class="tr-kind">${esc(t('kind.artist'))}</div></div>
        <div class="top-tracks"><h2>${esc(t('search.songs'))}</h2>${trackRows(top.slice(0, 5), { album: false })}</div></div>`;
      if (top.length > 5) html += `<section class="shelf">${trackRows(top.slice(5))}</section>`;
    } else if (top.length) {
      html += `<section class="shelf"><h2>${esc(t('search.songs'))}</h2>${trackRows(top)}</section>`;
    }
    html += shelf(t('search.artists'), r.artists, 'artist');
    html += shelf(t('search.albums'), r.albums, 'album');
    html += shelf(t('search.playlists'), r.playlists, 'playlist');
    return html + '</div>';
  },

  async album({ id }) {
    const a = await api.album(id);
    pageTracks = a.tracks; pageCtx = { kind: 'album', id, title: a.title }; pageColor = a.cover;
    return `<div class="page">${collectionHeader({
      cover: a.cover, kind: kindLabel(a.kind) || t('kind.album'), title: a.title,
      meta: `${artistLinks(a.artists)}${a.year ? ` · ${a.year}` : ''} · ${tracksLabel(a.tracks.length)}, ${totalTime(a.tracks)}`,
      actions: collectionActions() + actBtn('save-page', 'plus', t('act.saveAsPlaylist')),
    })}${trackRows(a.tracks, { covers: false, album: false, ctx: pageCtx })}
    ${a.description ? `<p class="desc">${esc(a.description)}</p>` : ''}</div>`;
  },

  async artist({ id }) {
    const a = await api.artist(id);
    pageTracks = a.tracks; pageCtx = { kind: 'artist', id, title: a.name }; pageColor = a.cover;
    const plays = a.tracks.reduce((s, x) => s + (x.plays || 0), 0);
    // Full-bleed hero (edge to edge of the page area), then the page content.
    return `<div class="artist-hero" style="${a.cover ? `--bg-img:url('${esc(a.cover)}')` : ''}"><div class="artist-hero-in">
        <div class="hero-kind">${esc(t('kind.artist'))}</div><h1>${esc(a.name)}</h1>
        <div class="hero-meta">${[a.subscribers != null ? subsLabel(a.subscribers) : '', plays ? t('artist.topPlays', { n: compact(plays) }) : ''].filter(Boolean).map(esc).join(' · ')}</div>
        <div class="actions" style="margin-top:0">${playBtn('play-page')}${actBtn('radio-artist', 'radio', t('act.artistRadio'))}${actBtn('shuffle-page', 'shuffle', t('act.shuffle'))}</div>
      </div></div>
      <div class="page artist">
      ${a.tracks.length ? `<section class="shelf"><h2>${esc(t('artist.popular'))}</h2>${trackRows(a.tracks.slice(0, 10), { ctx: pageCtx, plays: true })}</section>` : ''}
      ${carousel(t('artist.albums'), a.albums, 'album')}
      ${carousel(t('artist.singles'), a.singles, 'album')}
      ${carousel(t('artist.playlists'), a.playlists, 'playlist')}
      ${carousel(t('artist.similar'), a.similar, 'artist')}
      ${a.description ? `<section class="shelf"><h2>${esc(t('artist.about'))}</h2><p class="desc">${esc(a.description)}</p></section>` : ''}
    </div>`;
  },

  async playlist({ id }) {
    const p = await api.playlist(id);
    pageTracks = p.tracks; pageCtx = { kind: 'playlist', id, title: p.title }; pageColor = p.cover;
    return `<div class="page">${collectionHeader({
      cover: p.cover, kind: t('kind.playlist'), title: p.title,
      meta: `${p.author ? `<b>${esc(p.author)}</b> · ` : ''}${tracksLabel(p.tracks.length)}, ${totalTime(p.tracks)}`,
      actions: collectionActions() + actBtn('save-page', 'plus', t('act.saveToLibrary')),
    })}${p.description ? `<p class="desc">${esc(p.description)}</p>` : ''}${trackRows(p.tracks, { ctx: pageCtx })}</div>`;
  },

  async 'local-playlist'({ id }) {
    const p = await api.localPlaylist(Number(id));
    pageTracks = p.tracks; pageCtx = { kind: 'local', id: Number(id), title: p.name }; pageColor = p.tracks[0]?.cover || null;
    return `<div class="page">${collectionHeader({
      cover: p.tracks[0]?.cover, kind: t('kind.playlist'), title: p.name,
      meta: `${tracksLabel(p.tracks.length)}${p.tracks.length ? `, ${totalTime(p.tracks)}` : ''}`,
      actions: collectionActions() + actBtn('rename-local', 'edit', t('act.rename')) + actBtn('delete-local', 'trash', t('act.deletePlaylist')),
    })}${p.tracks.length ? trackRows(p.tracks, { ctx: pageCtx })
      : `<div class="empty">${icon('music')}<h2>${esc(t('local.empty'))}</h2><button class="pill-btn primary" data-act="focus-search">${icon('search')}${esc(t('nav.search'))}</button></div>`}</div>`;
  },

  async liked() {
    const tracks = await api.likedTracks();
    pageTracks = tracks; pageCtx = { kind: 'liked', title: t('lib.liked') }; pageColor = '#5b6cff';
    return `<div class="page">${collectionHeader({
      cover: null, kind: t('kind.playlist'), title: t('lib.liked'), meta: tracksLabel(tracks.length),
      actions: collectionActions(), phClass: 'liked-cover',
    })}
    ${tracks.length ? trackRows(tracks, { ctx: pageCtx }) : `<div class="empty">${icon('heart')}<h2>${esc(t('liked.empty'))}</h2></div>`}</div>`;
  },

  async downloads() {
    const items = await api.downloads();
    const tracks = items.map((d) => d.track);
    pageTracks = tracks; pageCtx = { kind: 'downloads', title: t('lib.downloads') }; pageColor = '#1f9d5a';
    const size = items.reduce((a, d) => a + (d.size || 0), 0);
    const active = [...state.dlProgress.values()].filter((p) => p.state === 'queued' || p.state === 'downloading');
    return `<div class="page">${collectionHeader({
      cover: null, kind: t('kind.offline'), title: t('lib.downloads'),
      meta: `${tracksLabel(tracks.length)} · ${(size / 1048576).toFixed(0)} MB · ${icon('lock')} ${esc(t('dl.encrypted'))}`,
      actions: playBtn('play-page') + actBtn('shuffle-page', 'shuffle', t('act.shuffle')), phClass: 'dl-cover',
    })}
    ${active.length ? `<div class="note">${esc(t('dl.active', { n: active.length }))}</div>` : ''}
    ${tracks.length ? trackRows(tracks, { ctx: pageCtx }) : `<div class="empty">${icon('download')}<h2>${esc(t('dl.empty'))}</h2><p>${esc(t('set.dlHint'))}</p></div>`}</div>`;
  },

  async library() {
    await refreshLibrary().catch(() => {});
    return `<div class="page library-page">
      <div class="layers-bar"><div></div><div class="actions" style="margin-top:0">
        <button class="pill-btn" data-act="new-playlist">${icon('plus')}${esc(t('nav.newPlaylist'))}</button>
        ${state.settings.logged_in ? `<button class="pill-btn" data-act="sync">${icon('sync')}${esc(t('set.sync'))}</button>` : ''}</div></div>
      <div class="grid">
        ${specialCard('liked', 'liked-cover', 'heart-fill', t('lib.liked'), `${t('lib.playlist')} · ${tracksLabel(state.liked.size)}`)}
        ${specialCard('downloads', 'dl-cover', 'download', t('lib.downloads'), `${t('lib.offline')} · ${tracksLabel(state.downloaded.size)}`)}
        ${specialCard('history', 'hist-cover', 'history', t('lib.history'), t('lib.historySub'))}
        ${state.playlists.map((p) => card('local', p)).join('')}
      </div>
      ${state.remotePlaylists.length ? `<section class="shelf lib-sep">${secHead(t('lib.yt'))}<div class="grid">${state.remotePlaylists.map((p) => card('playlist', p)).join('')}</div></section>` : ''}
    </div>`;
  },

  async history() {
    const tracks = await api.history();
    pageTracks = tracks; pageCtx = null;
    return `<div class="page">${tracks.length ? trackRows(tracks, { numbers: false }) : `<div class="empty">${icon('history')}<h2>${esc(t('history.empty'))}</h2></div>`}</div>`;
  },

  async settings() {
    const s = await api.settings();
    state.settings = s;
    renderAccount();
    return `<div class="page settings"><h1>${esc(t('set.title'))}</h1>
      <section class="set-block">
        <h2>${esc(t('set.language'))}</h2>
        <div class="row"><select id="lang-select" class="text-in select" aria-label="${esc(t('set.language'))}">
          ${Object.entries(LANGS).map(([k, v]) => `<option value="${k}" ${k === lang ? 'selected' : ''}>${esc(v)}</option>`).join('')}
        </select></div>
        <h2 class="sub-h">${esc(t('set.theme'))}</h2>
        <div class="seg theme-seg" role="radiogroup">
          ${['system', 'dark', 'light'].map((m) => `<label class="seg-opt"><input type="radio" name="themeMode" value="${m}" ${getPref('themeMode', 'system') === m ? 'checked' : ''}>
            <b>${esc(t(`mode.${m}`))}</b><small>${esc(t(`mode.${m}Sub`))}</small></label>`).join('')}
        </div>
        <h2 class="sub-h">${esc(t('set.motion'))}</h2>
        <div class="seg" role="radiogroup">
          ${['on', 'system'].map((m) => `<label class="seg-opt"><input type="radio" name="motion" value="${m}" ${getPref('motion', 'on') === m ? 'checked' : ''}>
            <b>${esc(t(`motion.${m}`))}</b><small>${esc(t(`motion.${m}Sub`))}</small></label>`).join('')}
        </div>
        <h2 class="sub-h">${esc(t('set.region'))}</h2>
        <p class="muted small">${esc(t('set.regionHint'))}</p>
        <div class="row"><select id="region-select" class="text-in select" aria-label="${esc(t('set.region'))}">
          ${['auto', 'US', 'GB', 'DE', 'FR', 'PL', 'UA'].map((c) => `<option value="${c}" ${c === s.chart_region ? 'selected' : ''}>${esc(c === 'auto' ? t('set.regionAuto', { name: regionName(s.detected_region) }) : regionName(c))}</option>`).join('')}
        </select></div>
      </section>
      <section class="set-block">
        <h2>${esc(t('set.account'))}</h2>
        ${s.logged_in ? `<p>${esc(t('set.loggedIn'))}${s.account_name ? `: <b>${esc(s.account_name)}</b>` : ''}.</p>
          <div class="row"><button class="pill-btn primary" data-act="sync">${icon('sync')} ${esc(t('set.sync'))}</button>
          <button class="pill-btn" data-act="signout-confirm">${esc(t('set.logout'))}</button></div>`
        : `${s.google_login_supported ? `<p class="muted">${esc(t('set.googleHint'))}</p>
          <div class="row"><button class="google-btn" data-act="google-login">${GOOGLE_G}<span>${esc(t('set.google'))}</span></button></div>` : ''}
          ${s.google_login_supported ? '' : `<details class="advanced" open><summary>${esc(t('set.advanced'))}</summary>
          <p class="muted small">${esc(t('set.loginHint'))}</p>
          <ol class="muted small"><li>${esc(t('set.step1'))}</li><li>${esc(t('set.step2'))}</li><li>${esc(t('set.step3'))}</li></ol>
          <div class="row"><label class="pill-btn">${esc(t('set.pickFile'))}<input type="file" id="cookie-file" accept=".txt" hidden></label></div>
          <textarea id="cookie-text" placeholder="${esc(t('set.cookiePh'))}" aria-label="${esc(t('set.cookiePh'))}"></textarea>
          <div class="row"><button class="pill-btn" data-act="login">${esc(t('set.login'))}</button></div></details>`}`}
      </section>
      <section class="set-block">
        <h2>${esc(t('set.playback'))}</h2>
        <label class="check"><input type="checkbox" id="autoplay" ${player.autoplay ? 'checked' : ''}> ${esc(t('set.autoplay'))}</label>
        <div class="row"><button class="pill-btn" data-act="open-eq">${icon('sliders')}${esc(t('eq.title'))}</button></div>
      </section>
      <section class="set-block">
        <h2>${esc(t('set.downloads'))}</h2>
        <p class="muted small">${esc(t('set.dlHint'))}</p>
        <div class="seg" role="radiogroup">
          <label class="seg-opt"><input type="radio" name="dlq" value="opus" ${s.download_quality === 'opus' ? 'checked' : ''}>
            <b>${esc(t('set.qOpus'))}</b><small>${esc(t('set.qOpusSub'))}</small></label>
          <label class="seg-opt"><input type="radio" name="dlq" value="aac" ${s.download_quality === 'aac' ? 'checked' : ''}>
            <b>${esc(t('set.qAac'))}</b><small>${esc(t('set.qAacSub'))}</small></label>
        </div>
        <p class="muted small">${esc(t('set.dlDirHint'))}</p>
        <div class="row"><input id="dl-dir" class="text-in" value="${esc(s.download_dir)}" spellcheck="false" aria-label="${esc(t('set.dlDirHint'))}">
          <button class="pill-btn" data-act="save-dl-dir">${esc(t('set.save'))}</button></div>
      </section>
      ${s.web_player_supported ? `<section class="set-block">
        <h2>${esc(t('eng.title'))}</h2>
        <label class="check"><input type="checkbox" id="web-player" ${s.web_player ? 'checked' : ''}> ${esc(t('eng.web'))}</label>
        <p class="muted small">${esc(t('eng.hint'))}</p>
      </section>` : ''}
      <section class="set-block">
        <h2>${esc(t('int.title'))}</h2>
        <label class="check"><input type="checkbox" data-toggle="notify_tracks" ${s.notify_tracks ? 'checked' : ''}> ${esc(t('int.notify'))}</label>
        ${s.discord_available ? `<label class="check"><input type="checkbox" data-toggle="discord_presence" ${s.discord_presence ? 'checked' : ''}> ${esc(t('int.discord'))}</label>` : ''}
        ${s.hotkeys_supported ? `<label class="check"><input type="checkbox" data-toggle="global_hotkeys" ${s.global_hotkeys ? 'checked' : ''}> ${esc(t('int.hotkeys'))}</label>
          <p class="muted small"><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> ${esc(t('int.hkToggle'))} · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>→</kbd> ${esc(t('int.hkNext'))} · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>←</kbd> ${esc(t('int.hkPrev'))} · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>L</kbd> ${esc(t('int.hkLike'))}</p>` : ''}
      </section>
      ${/iPhone|iPad|iPod/.test(navigator.userAgent) ? '' : `<section class="set-block">
        <h2>${esc(t('upd.title'))}</h2>
        <p id="upd-status" class="muted">${esc(updateStatusText())}</p>
        <div class="row">
          <button class="pill-btn" data-act="check-update">${icon('sync')} ${esc(t('upd.check'))}</button>
          ${upd.info?.available ? `<button class="pill-btn primary" data-act="install-update">${esc(t(upd.info.kind === 'apk' ? 'upd.apk' : 'upd.install'))}</button>` : ''}
        </div>
      </section>`}
      ${s.ytdlp_supported === false ? '' : `<section class="set-block">
        <h2>${esc(t('set.ytdlp'))}</h2>
        <p class="muted small">${esc(t('set.ytdlpHint'))}</p>
        <p>${s.ytdlp ? esc(t('set.ytdlpFound')) : esc(t('set.ytdlpMissing'))}</p>
        <div class="row"><button class="pill-btn" data-act="install-ytdlp">${esc(t(s.ytdlp ? 'set.ytdlpUpdate' : 'set.ytdlpInstall'))}</button></div>
      </section>`}
    </div>`;
  },
};

const afterRender = {
  search({ q }) { if (q) $('#search-input').value = q; },
  settings() {
    $('#lang-select')?.addEventListener('change', (e) => setLang(e.target.value));
    $$('input[name=themeMode]').forEach((r) => r.addEventListener('change', () => setPref('themeMode', r.value)));
    $$('input[name=motion]').forEach((r) => r.addEventListener('change', () => setPref('motion', r.value)));
    $('#region-select')?.addEventListener('change', async (e) => {
      try { await api.setChartRegion(e.target.value); toast(t('toast.settingsSaved')); } catch (err) { toast(errText(err), 'err'); }
    });
    $('#cookie-file')?.addEventListener('change', async (e) => {
      const f = e.target.files[0];
      if (f) $('#cookie-text').value = await f.text();
    });
    $('#autoplay')?.addEventListener('change', (e) => player.setAutoplay(e.target.checked));
    $('#web-player')?.addEventListener('change', async (e) => {
      try { await player.setWebEngine(e.target.checked); state.settings = await api.settings(); } catch (err) { toast(errText(err), 'err'); }
      render();
    });
    $$('input[data-toggle]').forEach((cb) => cb.addEventListener('change', async () => {
      try {
        await api.setToggle(cb.dataset.toggle, cb.checked);
        if (cb.dataset.toggle === 'discord_presence') api.presence(player.playing, audio.currentTime || 0).catch(() => {});
      } catch (e) { toast(errText(e), 'err'); }
    }));
    $$('input[name=dlq]').forEach((r) => r.addEventListener('change', async () => {
      try { await api.setDownloadQuality(r.value); toast(t('toast.settingsSaved')); } catch (e) { toast(errText(e), 'err'); }
    }));
  },
};

// ---------------------------------------------------------------- dock + account
function renderDock() {
  // Library rail (wide screens): built-ins, your playlists, then YouTube playlists.
  const item = (attrs, cover, title, sub) => `<button class="rail-item" ${attrs} title="${esc(title)}">${cover}<div><b>${esc(title)}</b><small>${esc(sub)}</small></div></button>`;
  $('#rail-list').innerHTML = [
    item('data-nav="liked"', `<span class="qc liked-cover">${icon('heart-fill')}</span>`, t('lib.liked'), `${t('lib.playlist')} · ${tracksLabel(state.liked.size)}`),
    item('data-nav="downloads"', `<span class="qc dl-cover">${icon('download')}</span>`, t('lib.downloads'), `${t('lib.offline')} · ${tracksLabel(state.downloaded.size)}`),
    item('data-nav="history"', `<span class="qc hist-cover">${icon('history')}</span>`, t('lib.history'), t('lib.historySub')),
    ...state.playlists.map((p) => item(`data-go="local-playlist" data-id="${p.id}"`, p.cover ? `<img src="${esc(thumb(p.cover))}" alt="" loading="lazy" data-onerr="placeholder">` : `<span class="qc">${icon('music')}</span>`, p.name, `${t('lib.playlist')} · ${tracksLabel(p.count)}`)),
    state.remotePlaylists.length ? `<div class="mono rail-sep">${esc(t('lib.yt'))}</div>` : '',
    ...state.remotePlaylists.map((p) => item(`data-go="playlist" data-id="${esc(p.id)}"`, p.cover ? `<img src="${esc(thumb(p.cover))}" alt="" loading="lazy" data-onerr="placeholder">` : `<span class="qc">${icon('music')}</span>`, p.title, p.author || t('lib.playlist'))),
  ].join('');
  markRail();
}
/** Highlight the rail entry of the page that is open. */
function markRail() {
  const e = mode() === 'browse' ? nav.stack[nav.pos] : null;
  document.body.dataset.view = e ? e.view : 'stage';
  const top = e && (['library', 'history', 'downloads', 'liked', 'local-playlist', 'playlist'].includes(e.view) ? 'library' : e.view);
  $$('.rail-link[data-nav], .dock-link[data-nav]').forEach((b) => b.classList.toggle('active', !!e && b.dataset.nav === top));
  $$('.rail-item').forEach((b) => {
    const on = !!e && (b.dataset.nav ? b.dataset.nav === e.view : b.dataset.go === e.view && String(b.dataset.id) === String(e.params.id));
    b.classList.toggle('active', on);
  });
}

function renderAccount() {
  const a = $('#avatar');
  const s = state.settings;
  a.classList.toggle('guest', !s.logged_in);
  const name = s.account_name || t('top.account');
  a.setAttribute('aria-label', s.logged_in ? `${t('top.account')}: ${name}` : t('acct.guest'));
  a.title = a.getAttribute('aria-label');
  a.innerHTML = s.logged_in && s.account_avatar ? `<img src="${esc(s.account_avatar)}" alt="" data-onerr="remove">`
    : s.logged_in ? esc(name.trim()[0]?.toUpperCase() || 'Y') : icon('user');
}

// ---------------------------------------------------------------- popovers (account menu, track menu)
const overlay = $('#overlay');
let lastFocus = null;
function closeOverlays() {
  const any = !!overlay.firstChild || !!$('#modal-root').firstChild || !menu.classList.contains('hidden');
  overlay.innerHTML = '';
  if ($('#modal-root').firstChild) $('#modal-root').firstChild.dispatchEvent(new CustomEvent('dismiss'));
  hideMenu();
  $('#avatar').setAttribute('aria-expanded', 'false');
  if (any && lastFocus) { lastFocus.focus?.(); lastFocus = null; }
  return any;
}
function popover(html, anchor) {
  lastFocus = document.activeElement;
  hideMenu();
  overlay.innerHTML = `<div class="pop" role="menu">${html}</div>`;
  const p = overlay.firstChild, r = anchor.getBoundingClientRect();
  const w = p.offsetWidth;
  p.style.left = `${Math.max(12, Math.min(r.right - w, innerWidth - w - 12))}px`;
  let top = r.bottom + 8;
  if (top + p.offsetHeight > innerHeight - 12) top = Math.max(12, r.top - p.offsetHeight - 8);
  p.style.top = `${top}px`;
  p.querySelector('.mi')?.focus();
  ensureBackEntry();
}
function accountMenu() {
  const s = state.settings;
  const name = s.account_name || t('top.account');
  const head = s.logged_in
    ? `<div class="acct-head"><span class="avatar">${s.account_avatar ? `<img src="${esc(s.account_avatar)}" alt="">` : esc(name[0]?.toUpperCase() || 'Y')}</span><div><b>${esc(name)}</b><small>YouTube</small></div></div>`
    : `<div class="acct-head"><span class="avatar guest">${icon('user')}</span><div><b>${esc(t('acct.guest'))}</b><small>${esc(t('acct.guestSub'))}</small></div></div>`;
  const mi = (act, ic, label, cls = '') => `<button class="mi ${cls}" role="menuitem" data-act="${act}">${icon(ic)}<span>${esc(label)}</span></button>`;
  const items = s.logged_in
    ? mi('sync', 'sync', t('acct.sync')) + mi('switch-account', 'user-switch', t('acct.switch'))
    : (s.google_login_supported ? mi('google-login', 'user', t('acct.signIn')) : mi('open-settings', 'user', t('set.login')));
  $('#avatar').setAttribute('aria-expanded', 'true');
  popover(`${head}<div class="sep"></div>${items}${mi('open-settings', 'settings', t('nav.settings'))}${mi('check-update-menu', 'download', t('acct.update'))}
    ${s.logged_in ? `<div class="sep"></div>${mi('signout-confirm', 'logout', t('acct.signOut'), 'danger')}` : ''}`, $('#avatar'));
}

// ---------------------------------------------------------------- sheets (EQ, queue, confirm)
function sheet(id, title, body, actions = '') {
  lastFocus = document.activeElement;
  hideMenu();
  overlay.innerHTML = `<div class="scrim" data-scrim><div class="sheet" role="dialog" aria-modal="true" aria-labelledby="${id}-h" id="${id}">
    <div class="sheet-head"><h2 id="${id}-h">${esc(title)}</h2><div class="row">${actions}<button class="iconbtn" data-act="close-overlay" aria-label="${esc(t('rp.close'))}">${icon('close')}</button></div></div>
    ${body}</div></div>`;
  ensureBackEntry();
  return overlay.querySelector('.sheet');
}

function eqSheet() {
  const presets = Object.keys(PRESETS).map((p) => `<button class="tab" data-preset="${p}" aria-pressed="${eqState.preset === p}">${esc(t(`eq.p.${p}`))}</button>`).join('')
    + `<button class="tab" aria-pressed="${eqState.preset === 'custom'}" disabled>${esc(t('eq.custom'))}</button>`;
  const band = (k, v, label, cls = '') => `<div class="band ${cls}"><output id="eqo-${k}">${v > 0 ? '+' : ''}${v}</output>
    <input type="range" min="-12" max="12" step="1" value="${v}" data-band="${k}" aria-label="${esc(label)} dB" style="--p:${((v + 12) / 24) * 100}%"><span>${esc(label)}</span></div>`;
  const bands = band('pre', eqState.pre, t('eq.preamp'), 'pre')
    + FREQS.map((f, i) => band(i, eqState.gains[i], f >= 1000 ? `${f / 1000}k` : String(f))).join('');
  const el = sheet('eq-sheet', t('eq.title'),
    `<div class="tabs" id="eq-presets">${presets}</div><div class="bands">${bands}</div>
     <p class="muted small" style="margin:0">${esc(t('eq.hint'))}${window.AndroidMedia ? ` ${esc(t('eq.android'))}` : ''}</p>`,
    `<button class="switch" role="switch" id="eq-on" aria-checked="${eqState.on}" aria-label="${esc(t('eq.on'))}"></button>`);
  el.classList.toggle('eq-off', !eqState.on);
  $('#eq-on').focus();
  $('#eq-on').onclick = (e) => { eq.setOn(!eqState.on); e.currentTarget.setAttribute('aria-checked', eqState.on); el.classList.toggle('eq-off', !eqState.on); };
  $$('[data-preset]', el).forEach((b) => (b.onclick = () => { eq.setPreset(b.dataset.preset); eqSheet(); }));
  $$('[data-band]', el).forEach((r) => (r.oninput = () => {
    const k = r.dataset.band, v = Number(r.value);
    if (k === 'pre') eq.setPreamp(v); else {
      eq.setGain(Number(k), v);
      $$('#eq-presets .tab', el).forEach((b) => b.setAttribute('aria-pressed', !b.dataset.preset));
    }
    r.style.setProperty('--p', `${((v + 12) / 24) * 100}%`);
    $(`#eqo-${k}`).textContent = `${v > 0 ? '+' : ''}${v}`;
  }));
}

function queueMarkup() {
  const q = player.queue, i = player.index;
  if (!q.length) return `<p class="muted">${esc(t('rp.emptyQueue'))}</p>`;
  const lid = registerList(q);
  const row = (x, idx) => `<div class="qrow ${idx === i ? 'playing' : ''}" data-qidx="${idx}" data-list="${lid}" data-idx="${idx}" tabindex="0">
    <img src="${esc(thumb(x.cover))}" alt="" loading="lazy" decoding="async" data-onerr="hide"><div class="ttext"><div class="ttitle">${esc(x.title)}</div>
    <div class="tartist">${esc(artistNames(x.artists))}</div></div>
    ${idx !== i ? `<button class="icon-btn" data-act="q-remove" aria-label="${esc(t('rp.close'))}">${icon('close')}</button>` : '<span></span>'}</div>`;
  const nextTitle = player.context?.title ? t('rp.nextFrom', { title: player.context.title }) : t('rp.next');
  return `${q[i] ? `<div class="mono">${esc(t('rp.nowPlaying'))}</div>${row(q[i], i)}` : ''}
    ${q.length > i + 1 ? `<div class="mono" style="margin-top:8px">${esc(nextTitle)}</div>${q.slice(i + 1, i + 101).map((x, k) => row(x, i + 1 + k)).join('')}` : ''}`;
}
const queueSheet = () => { sheet('queue-sheet', t('rp.queue'), `<div>${queueMarkup()}</div>`); overlay.querySelector('[data-act="close-overlay"]').focus(); };

function confirmDialog(title, body, okLabel, danger = false) {
  return new Promise((resolve) => {
    lastFocus = document.activeElement;
    const root = $('#modal-root');
    root.innerHTML = `<div class="modal-bg"><div class="modal" role="alertdialog" aria-modal="true" aria-labelledby="cf-h"><h2 id="cf-h">${esc(title)}</h2>
      <p class="muted" style="margin:0">${esc(body)}</p>
      <div class="row end"><button class="pill-btn" id="cf-no">${esc(t('dlg.cancel'))}</button><button class="pill-btn ${danger ? 'danger' : 'primary'}" id="cf-ok">${esc(okLabel)}</button></div></div></div>`;
    const done = (v) => { root.innerHTML = ''; resolve(v); lastFocus?.focus?.(); };
    $('#cf-ok').onclick = () => done(true);
    $('#cf-no').onclick = () => done(false);
    root.firstChild.addEventListener('dismiss', () => done(false));
    $('.modal-bg', root).onclick = (e) => { if (e.target.classList.contains('modal-bg')) done(false); };
    $('#cf-ok').focus();
  });
}

// ---------------------------------------------------------------- context menu
const menu = $('#ctxmenu');
function showMenu(x, y, items) {
  menu.innerHTML = items.map((it, idx) => it === '-' ? '<div class="sep"></div>'
    : it.sub ? `<div class="mi has-sub" tabindex="0">${esc(it.label)}<span>${icon('chev-right')}</span><div class="submenu">${it.sub.map((s, k) =>
      `<div class="mi" role="menuitem" tabindex="0" data-mi="${k}" data-parent="${idx}">${esc(s.label)}</div>`).join('')}</div></div>`
    : `<div class="mi" role="menuitem" tabindex="0" data-mi="${idx}">${esc(it.label)}</div>`).join('');
  menu._items = items;
  overlay.innerHTML = '';
  menu.classList.remove('hidden');
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - r.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, innerHeight - r.height - 8))}px`;
  menu.classList.toggle('flip', x + r.width * 2 > innerWidth);
  menu.querySelector('.mi')?.focus();
}
function hideMenu() { menu.classList.add('hidden'); }
menu.addEventListener('click', (e) => {
  const mi = e.target.closest('.mi[data-mi]');
  if (!mi) {
    // touch screens have no hover: tap opens the submenu
    e.target.closest('.has-sub')?.classList.toggle('open');
    return;
  }
  const items = menu._items;
  const it = mi.dataset.parent != null ? items[mi.dataset.parent].sub[mi.dataset.mi] : items[mi.dataset.mi];
  hideMenu();
  it?.run();
});
menu.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.target.closest('.mi')?.click(); }
});
document.addEventListener('mousedown', (e) => {
  if (!menu.contains(e.target)) hideMenu();
  const p = overlay.querySelector('.pop');
  if (p && !p.contains(e.target) && !e.target.closest('#avatar')) closeOverlays();
});
window.addEventListener('blur', hideMenu);

function trackMenu(x, ctx, idx, px, py) {
  const items = [
    { label: t('menu.playNext'), run: () => { player.playNext([x]); toast(t('toast.playNext')); } },
    { label: t('menu.addQueue'), run: () => { player.addToQueue([x]); toast(t('toast.queued')); } },
    { label: t('menu.radio'), run: () => startRadio(x) },
    '-',
    { label: t('menu.addPlaylist'), sub: [
      { label: t('menu.newPlaylist'), run: () => newPlaylist([x]) },
      ...state.playlists.map((p) => ({ label: p.name, run: () => addTo(p, [x]) })),
    ] },
    { label: t(state.liked.has(x.id) ? 'menu.unlike' : 'menu.like'), run: () => toggleLike(x) },
    state.downloaded.has(x.id)
      ? { label: t('menu.removeDownload'), run: () => deleteDownload(x) }
      : { label: t('menu.download'), run: () => downloadTracks([x]) },
    '-',
  ];
  if (x.artists?.[0]?.id) items.push({ label: t('menu.artist'), run: () => go('artist', { id: x.artists[0].id }) });
  if (x.album?.id) items.push({ label: t('menu.album'), run: () => go('album', { id: x.album.id }) });
  items.push({ label: t('menu.copy'), run: () => { navigator.clipboard.writeText(`https://music.youtube.com/watch?v=${x.id}`).then(() => toast(t('toast.copied')), () => {}); } });
  if (ctx?.kind === 'local') {
    items.push('-', { label: t('menu.removeFromPlaylist'), run: async () => {
      await api.removeFromPlaylist(ctx.id, idx); await refreshLibrary(); render();
    } });
  }
  showMenu(px, py, items);
}

// ---------------------------------------------------------------- actions
async function toggleLike(x) {
  const liked = !state.liked.has(x.id);
  try {
    await api.setLiked(x, liked);
    liked ? state.liked.add(x.id) : state.liked.delete(x.id);
    if (liked && autoDl.has('liked')) downloadTracks([x], true);
    $$(`.trow[data-tid="${CSS.escape(x.id)}"] .like`).forEach((b) => {
      b.classList.toggle('on', liked); b.setAttribute('aria-pressed', liked); b.innerHTML = icon(liked ? 'heart-fill' : 'heart');
    });
    renderDock();
    updateNowPlaying();
    toast(t(liked ? 'toast.liked' : 'toast.unliked'));
  } catch (e) { toast(errText(e), 'err'); }
}

/** Downloading needs an account, as in YouTube Music. Says so and offers sign-in. */
function needSignIn() {
  if (state.settings.logged_in) return false;
  toast(t('toast.dlSignIn'), 'err');
  accountMenu();
  return true;
}
/** `quiet`: automatic downloads (no "already downloaded" / "downloading" toasts). */
async function downloadTracks(tracks, quiet = false) {
  if (!state.settings.logged_in) { if (!quiet) needSignIn(); return; }
  const todo = tracks.filter((x) => !state.downloaded.has(x.id) && !state.dlProgress.has(x.id));
  if (!todo.length) { if (!quiet) toast(t('toast.alreadyDl')); return; }
  for (const x of todo) state.dlProgress.set(x.id, { state: 'queued', progress: 0 });
  updateDlButtons();
  try { await api.download(todo); if (!quiet) toast(t('toast.dlStarted', { tracks: tracksLabel(todo.length) })); } catch (e) { toast(errText(e), 'err'); }
}
// Collections you chose to download (liked songs, a playlist, an album), kept on this device:
// everything in them is downloaded, and a song added later downloads by itself.
const autoDl = {
  get() { try { return new Set(JSON.parse(localStorage.getItem('autoDl') || '[]')); } catch { return new Set(); } },
  set(s) { try { localStorage.setItem('autoDl', JSON.stringify([...s])); } catch { /* ignore */ } },
  has(k) { return !!k && this.get().has(k); },
};
const dlKey = (ctx) => (!ctx ? null : ctx.kind === 'liked' ? 'liked' : ['local', 'playlist', 'album'].includes(ctx.kind) ? `${ctx.kind}:${ctx.id}` : null);
const dlToggleInner = (on) => `${icon(on ? 'downloaded' : 'download')}${esc(t(on ? 'act.downloaded' : 'act.download'))}`;
/** On start: catch up on the local collections marked for download. */
async function syncAutoDl() {
  if (!state.settings.logged_in) return;
  for (const k of autoDl.get()) {
    try {
      if (k === 'liked') await downloadTracks(await api.likedTracks(), true);
      else if (k.startsWith('local:')) await downloadTracks((await api.localPlaylist(Number(k.slice(6)))).tracks, true);
    } catch { /* e.g. a deleted playlist */ }
  }
}
/** Download button of the playing track (capsule + player). */
function updateDlButtons() {
  const cur = player.current;
  const done = !!cur && state.downloaded.has(cur.id);
  const prog = cur && state.dlProgress.get(cur.id);
  const label = done ? t('act.downloaded') : prog ? `${t('tr.downloading')} ${Math.round((prog.progress || 0) * 100)}%` : t('pl.download');
  const chip = $('#np-dl');
  for (const b of [chip]) { setIcon(b, done ? 'downloaded' : 'download'); b.classList.toggle('on', done); b.classList.toggle('busy', !!prog && !done); }
  chip.querySelector('span').textContent = label;
}

async function deleteDownload(x) {
  await api.deleteDownload(x.id);
  state.downloaded.delete(x.id);
  renderDock();
  if (nav.stack[nav.pos]?.view === 'downloads') render();
  toast(t('toast.dlRemoved'));
}

async function startRadio(x) {
  if (!x) return;
  toast(t('toast.radio'));
  try {
    const tracks = await api.radio(x.id);
    const list = [x, ...tracks.filter((y) => y.id !== x.id)];
    player.playList(list, 0, { kind: 'radio', title: t('radio.title', { title: x.title }) });
  } catch (e) { toast(errText(e), 'err'); }
}

function promptDialog(title, value = '') {
  return new Promise((resolve) => {
    lastFocus = document.activeElement;
    const root = $('#modal-root');
    root.innerHTML = `<div class="modal-bg"><div class="modal" role="dialog" aria-modal="true" aria-labelledby="pd-h"><h2 id="pd-h">${esc(title)}</h2>
      <input class="text-in" id="modal-in" value="${esc(value)}" spellcheck="false" aria-labelledby="pd-h">
      <div class="row end"><button class="pill-btn" id="m-cancel">${esc(t('dlg.cancel'))}</button><button class="pill-btn primary" id="m-ok">${esc(t('dlg.save'))}</button></div></div></div>`;
    const inp = $('#modal-in');
    inp.focus(); inp.select();
    const done = (v) => { root.innerHTML = ''; resolve(v); lastFocus?.focus?.(); };
    $('#m-ok').onclick = () => done(inp.value.trim() || null);
    $('#m-cancel').onclick = () => done(null);
    root.firstChild.addEventListener('dismiss', () => done(null));
    $('.modal-bg').onclick = (e) => { if (e.target.classList.contains('modal-bg')) done(null); };
    inp.onkeydown = (e) => { if (e.key === 'Enter') done(inp.value.trim() || null); if (e.key === 'Escape') { e.stopPropagation(); done(null); } };
  });
}

async function newPlaylist(tracks = []) {
  const name = await promptDialog(t('dlg.newPlaylist'), t('dlg.defaultName', { n: state.playlists.length + 1 }));
  if (!name) return;
  const id = await api.createPlaylist(name, tracks);
  await refreshLibrary();
  toast(t(tracks.length ? 'toast.createdAdded' : 'toast.created', { name }));
  if (!tracks.length) go('local-playlist', { id });
}

async function addTo(p, tracks) {
  await api.addToPlaylist(p.id, tracks);
  if (autoDl.has(`local:${p.id}`)) downloadTracks(tracks, true);
  await refreshLibrary();
  toast(t('toast.addedTo', { name: p.name }));
}

async function playCollection(kind, id) {
  try {
    let tracks = [], ctx = null;
    if (kind === 'album') { const a = await api.album(id); tracks = a.tracks; ctx = { kind, id, title: a.title }; }
    else if (kind === 'playlist') { const p = await api.playlist(id); tracks = p.tracks; ctx = { kind, id, title: p.title }; }
    else if (kind === 'local') { const p = await api.localPlaylist(Number(id)); tracks = p.tracks; ctx = { kind, id, title: p.name }; }
    if (tracks.length) player.playList(tracks, 0, ctx);
  } catch (e) { toast(errText(e), 'err'); }
}

async function logout() {
  player.suspend();
  await api.logout();
  state.settings = await api.settings();
  state.remotePlaylists = [];
  renderAccount(); renderDock();
  if (mode() === 'browse') render();
}

const actions = {
  like: (el) => { const r = trackFromEl(el); if (r) toggleLike(r.track); },
  menu: (el) => { const r = trackFromEl(el); const b = el.getBoundingClientRect(); if (r) trackMenu(r.track, r.ctx, r.idx, b.left - 200, b.bottom + 4); },
  'play-collection': (el) => playCollection(el.dataset.kind, el.dataset.id),
  'play-page': () => pageTracks.length && player.playList(pageTracks, 0, pageCtx),
  'shuffle-page': () => {
    if (!pageTracks.length) return;
    if (!player.shuffle) player.toggleShuffle();
    player.playList(pageTracks, Math.floor(Math.random() * pageTracks.length), pageCtx);
  },
  'download-page': (el) => {
    if (needSignIn()) return;
    const k = dlKey(pageCtx);
    if (!k) return downloadTracks(pageTracks);
    const s = autoDl.get();
    const on = !s.has(k);
    on ? s.add(k) : s.delete(k);
    autoDl.set(s);
    el.classList.toggle('on', on); el.setAttribute('aria-pressed', on); el.innerHTML = dlToggleInner(on);
    if (on) downloadTracks(pageTracks, true);
    toast(t(on ? 'toast.autoDlOn' : 'toast.autoDlOff'));
  },
  'save-page': async () => {
    await api.createPlaylist(pageCtx.title, pageTracks);
    await refreshLibrary();
    toast(t('toast.saved', { name: pageCtx.title }));
  },
  'rename-local': async () => {
    const name = await promptDialog(t('dlg.rename'), pageCtx.title);
    if (!name) return;
    await api.renamePlaylist(pageCtx.id, name);
    await refreshLibrary(); render();
  },
  'delete-local': async () => {
    if (!(await confirmDialog(t('dlg.delete', { name: pageCtx.title }), '', t('act.deletePlaylist'), true))) return;
    await api.deletePlaylist(pageCtx.id);
    await refreshLibrary();
    go('library');
  },
  'new-playlist': () => newPlaylist(),
  'radio-artist': () => { if (pageTracks[0]) startRadio(pageTracks[0]); },
  'radio-recent': () => { if (homeRecent[0]) startRadio(homeRecent[0]); },
  'play-liked': async () => {
    try {
      const tracks = await api.likedTracks();
      if (tracks.length) player.playList(tracks, 0, { kind: 'liked', title: t('lib.liked') });
    } catch (e) { toast(errText(e), 'err'); }
  },
  'q-remove': (el) => { player.removeAt(Number(el.closest('.qrow').dataset.qidx)); },
  'check-update': async (el) => {
    el.disabled = true;
    $('#upd-status').textContent = t('upd.checking');
    await checkForUpdate(true);
    render();
  },
  'check-update-menu': async () => {
    closeOverlays();
    toast(t('upd.checking'));
    await checkForUpdate(true);
    if (!upd.error) toast(updateStatusText());
  },
  'install-update': () => installUpdate(),
  reload: () => location.reload(),
  'google-login': async (el) => {
    closeOverlays();
    if (el.closest('.set-block')) { el.disabled = true; el.querySelector('span').textContent = t('set.googleWaiting'); }
    // Android: native sign-in screen (LoginActivity); it calls window.__androidLogin back.
    if (window.AndroidAuth) { window.AndroidAuth.login(); return; }
    player.suspend();
    try { await api.googleLogin(); } catch (e) { toast(errText(e), 'err'); if (mode() === 'browse') render(); }
  },
  'switch-account': async () => {
    closeOverlays();
    toast(t('acct.switchHint'));
    try { await logout(); await api.googleLogin(); } catch (e) { toast(errText(e), 'err'); }
  },
  'signout-confirm': async () => {
    closeOverlays();
    if (await confirmDialog(t('acct.signOutQ'), t('acct.signOutBody'), t('acct.signOut'), true)) logout();
  },
  'open-settings': () => { closeOverlays(); go('settings'); },
  'save-dl-dir': async () => {
    try { await api.setDownloadDir($('#dl-dir').value.trim()); toast(t('toast.settingsSaved')); render(); } catch (e) { toast(errText(e), 'err'); }
  },
  login: async (el) => {
    const text = $('#cookie-text').value.trim();
    if (!text) return toast(t('toast.pasteCookies'), 'err');
    el.disabled = true;
    try {
      await api.login(text);
      state.settings = await api.settings();
      renderAccount();
      toast(t('toast.loggedIn'));
      await sync();
      render();
    } catch (e) { toast(errText(e), 'err'); } finally { el.disabled = false; }
  },
  sync: async (el) => { closeOverlays(); el.disabled = true; await sync(); el.disabled = false; },
  'install-ytdlp': async (el) => {
    el.disabled = true; el.textContent = t('set.downloading');
    try { await api.installYtdlp(); toast(t('toast.ytdlpOk')); } catch (e) { toast(errText(e), 'err'); }
    render();
  },
  'play-list': (el) => {
    const list = state.lists.get(el.dataset.list);
    if (list?.length) player.playList(list, 0, { kind: 'mix', title: el.dataset.title });
  },
  scroll: (el) => {
    const box = el.closest('.shelf')?.querySelector('.scroller');
    if (box) box.scrollBy({ left: Number(el.dataset.dir) * box.clientWidth, // a page; snapping lines up the cards
      behavior: reducedMotion() ? 'auto' : 'smooth' });
  },
  'open-eq': () => eqSheet(),
  'close-overlay': () => closeOverlays(),
  'to-stage': () => toStage(),
  'to-browse': () => { if (!nav.stack.length) go('home'); else morph('browse'); },
  'focus-search': () => focusSearch(),
  'toggle-play': () => player.toggle(),
  'side-close': () => closeSide(),
};

async function sync() {
  try {
    const r = await api.syncAccount();
    await refreshLibrary();
    await refreshRemote();
    toast(t('toast.synced', { tracks: tracksLabel(r.liked), n: r.playlists }));
  } catch (e) { toast(t('toast.syncFail', { e: errText(e) }), 'err'); }
}

function trackFromEl(el) {
  const row = el.closest('[data-list]');
  if (!row) return null;
  const list = state.lists.get(row.dataset.list);
  const idx = Number(row.dataset.idx);
  const tl = row.closest('.tracklist');
  let ctx = null;
  try { ctx = tl ? JSON.parse(tl.dataset.ctx) : null; } catch { /* none */ }
  return list ? { list, idx, track: list[idx], ctx } : null;
}

// ---------------------------------------------------------------- global events
// Broken cover images (CSP forbids inline onerror=, so handle it here; errors don't bubble).
document.addEventListener('error', (e) => {
  const img = e.target;
  if (!(img instanceof HTMLImageElement)) return;
  // a playlist's own picture has a signed address that expires (a cached page may hold an old one):
  // show the first song's cover instead
  if (img.dataset.onerr === 'fallback') {
    img.dataset.onerr = 'hide';
    if (img.dataset.fallback) img.src = img.dataset.fallback; else img.style.visibility = 'hidden';
    return;
  }
  if (img.dataset.onerr === 'placeholder') { img.outerHTML = `<span class="qc">${icon('music')}</span>`; return; }
  if (img.dataset.onerr === 'remove') img.remove();
  else if (img.dataset.onerr === 'hide') img.style.visibility = 'hidden';
}, true);
document.addEventListener('click', (e) => {
  if (e.target.closest('[data-scrim]') && !e.target.closest('.sheet')) { closeOverlays(); return; }
  const act = e.target.closest('[data-act]');
  if (act) { e.stopPropagation(); actions[act.dataset.act]?.(act, e); return; }
  if (e.target.closest('.pop')) closeOverlays();
  const goEl = e.target.closest('[data-go]');
  // A click anywhere on a track row plays it (links inside the row still navigate).
  const row = e.target.closest('.trow');
  if (row && !goEl) {
    const r = trackFromEl(row);
    if (r) { if (player.current?.id === r.track.id) player.toggle(); else player.playList(r.list, r.idx, r.ctx); }
    return;
  }
  if (goEl) { const id = goEl.dataset.id; go(goEl.dataset.go, id ? { id } : {}); return; }
  const navEl = e.target.closest('[data-nav]');
  if (navEl) { go(navEl.dataset.nav); return; }
  const chip = e.target.closest('[data-search]');
  if (chip) { doSearch(chip.dataset.search); return; }
  const q = e.target.closest('.qrow');
  if (q) player.jumpTo(Number(q.dataset.qidx));
});
let hoverT = null;
document.addEventListener('pointerover', (e) => {
  const row = e.target.closest('.trow, .qrow');
  clearTimeout(hoverT);
  if (!row) return;
  const r = trackFromEl(row);
  if (r?.track && !state.downloaded.has(r.track.id)) hoverT = setTimeout(() => api.prefetch([r.track.id]).catch(() => {}), 250);
});
// Keyboard: Enter on a focused row / card activates it like a click.
document.addEventListener('keydown', (e) => {
  if ((e.key === 'Enter' || e.key === ' ') && e.target.matches('.trow, .card, .quick-item, .qrow, .top-result')) { e.preventDefault(); e.target.click(); }
});

document.addEventListener('contextmenu', (e) => {
  const row = e.target.closest('.trow, .qrow');
  if (row) {
    e.preventDefault();
    const r = trackFromEl(row);
    if (r) trackMenu(r.track, r.ctx, r.idx, e.clientX, e.clientY);
    return;
  }
  if (!e.target.closest('input, textarea')) e.preventDefault();
});

const navBack = () => { if (nav.pos > 0) { nav.pos--; render(); } else toStage(); };
$('#avatar').onclick = () => ($('#avatar').getAttribute('aria-expanded') === 'true' ? closeOverlays() : accountMenu());
document.addEventListener('click', (e) => {
  const line = e.target.closest('.ly-line');
  if (line) { player.seekTo(Number(line.dataset.t) / 1000); ly.userScrollAt = 0; }
});

// ---------------------------------------------------------------- search box
const sInput = $('#search-input');
const suggest = $('#suggest');
function focusSearch() {
  document.body.classList.add('searching');
  sInput.focus(); sInput.select();
  if (nav.stack[nav.pos]?.view !== 'search' || mode() !== 'browse') go('search', { q: sInput.value.trim() });
}
function doSearch(q) {
  q = q.trim();
  if (!q) return;
  suggest.classList.add('hidden');
  sInput.value = q;
  try {
    const h = JSON.parse(localStorage.getItem('searches') || '[]').filter((x) => x !== q);
    localStorage.setItem('searches', JSON.stringify([q, ...h].slice(0, 12)));
  } catch { /* ignore */ }
  const cur = nav.stack[nav.pos];
  if (cur?.view === 'search' && mode() === 'browse') { nav.stack[nav.pos] = { view: 'search', params: { q }, scroll: 0 }; render(); }
  else go('search', { q });
}
const loadSuggest = debounce(async (q) => {
  if (!q || document.activeElement !== sInput) return suggest.classList.add('hidden');
  try {
    const terms = await api.suggestions(q);
    if (sInput.value.trim() !== q) return;
    suggest.innerHTML = terms.slice(0, 8).map((s) => `<div class="sg" data-search="${esc(s)}">${icon('search')}<span>${esc(s)}</span></div>`).join('');
    suggest.classList.toggle('hidden', !terms.length);
  } catch { suggest.classList.add('hidden'); }
}, 180);
sInput.addEventListener('input', () => loadSuggest(sInput.value.trim()));
sInput.addEventListener('keydown', (e) => {
  const items = $$('.sg', suggest);
  const cur = items.findIndex((x) => x.classList.contains('sel'));
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    const n = items.length ? (cur + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length : -1;
    items.forEach((x, i) => x.classList.toggle('sel', i === n));
    if (items[n]) sInput.value = items[n].dataset.search;
  } else if (e.key === 'Enter') doSearch(sInput.value);
  else if (e.key === 'Escape') { e.stopPropagation(); suggest.classList.add('hidden'); sInput.blur(); }
});
sInput.addEventListener('blur', () => setTimeout(() => {
  suggest.classList.add('hidden');
  if (!sInput.value.trim()) document.body.classList.remove('searching');
}, 150));
sInput.addEventListener('focus', () => {
  if (sInput.value.trim()) loadSuggest(sInput.value.trim());
  if (mode() !== 'browse' || nav.stack[nav.pos]?.view !== 'search') go('search', { q: sInput.value.trim() });
});
$('#search-btn').onclick = focusSearch;

// ---------------------------------------------------------------- stream challenge solver
// Rust asks the webview's JS engine to run yt-dlp's solver (signed-in 18+ playback without
// yt-dlp, e.g. on Android); a worker keeps it off the UI thread. See jsplayer.rs.
// The worker is built from the solver's source (a blob): older Android WebViews don't serve a
// worker's own file requests from the app. If a worker can't start, the solver runs here on the
// main thread (loaded as plain scripts): a short pause beats Rust's slow QuickJS fallback.
// Older WebViews (Android 12 images ship Chromium 92) lack a few built-ins the solver uses.
function solverPolyfills() {
  const def = (o, k, v) => { if (!o[k]) Object.defineProperty(o, k, { configurable: true, writable: true, value: v }); };
  def(Object, "hasOwn", (o, k) => Object.prototype.hasOwnProperty.call(o, k));
  const at = function (i) { i = Math.trunc(i) || 0; if (i < 0) i += this.length; return this[i]; };
  def(Array.prototype, "at", at);
  def(String.prototype, "at", at);
  def(Array.prototype, "findLast", function (f, t) { for (let i = this.length - 1; i >= 0; i--) if (f.call(t, this[i], i, this)) return this[i]; return undefined; });
  def(Array.prototype, "findLastIndex", function (f, t) { for (let i = this.length - 1; i >= 0; i--) if (f.call(t, this[i], i, this)) return i; return -1; });
}
{
  const FILES = ['ejs/yt.solver.lib.min.js', 'ejs/yt.solver.core.min.js'];
  const reply = (id, out) => api.jsSolveDone(id, out).catch(() => {});
  const run = (jsc, input) => {
    try { return JSON.stringify(jsc(JSON.parse(input))); } catch (err) { return JSON.stringify({ type: 'error', error: String(err) }); }
  };
  let main = null;
  const mainJsc = () => main ??= (async () => {
    solverPolyfills();
    let loadErr = '';
    const onErr = (e) => { loadErr = e.message || String(e.error); };
    window.addEventListener('error', onErr);
    for (const src of FILES) {
      await new Promise((ok, fail) => { const s = document.createElement('script'); s.src = src; s.onload = ok; s.onerror = () => fail(new Error(src)); document.head.appendChild(s); });
      if (window.lib) { window.meriyah = window.lib.meriyah; window.astring = window.lib.astring; }
    }
    window.removeEventListener('error', onErr);
    if (typeof window.jsc !== 'function') { main = null; throw new Error(`solver did not load: ${loadErr || 'unknown'}`); }
    return window.jsc;
  })();
  const solveHere = async (id, input) => {
    try { reply(id, run(await mainJsc(), input)); } catch (err) { reply(id, JSON.stringify({ type: 'error', error: String(err) })); }
  };
  const pending = new Map();
  let worker = null; // Promise<Worker> | 'broken'
  const startWorker = async () => {
    const [lib, core] = await Promise.all(FILES.map((f) => fetch(f).then((r) => { if (!r.ok) throw new Error(f); return r.text(); })));
    const code = `(${solverPolyfills})();\n${lib}\nvar meriyah = lib.meriyah, astring = lib.astring;\n${core}\n`
      + `onmessage = (e) => { const { id, input } = e.data; let out; try { out = JSON.stringify(jsc(JSON.parse(input))); } catch (err) { out = JSON.stringify({ type: 'error', error: String(err) }); } postMessage({ id, out }); };`;
    const w = new Worker(URL.createObjectURL(new Blob([code], { type: 'text/javascript' })));
    w.onmessage = (e) => { pending.delete(e.data.id); reply(e.data.id, e.data.out); };
    w.onerror = (e) => {
      e.preventDefault?.();
      console.warn('solver worker failed:', e.message);
      worker = 'broken';
      for (const [id, input] of pending) solveHere(id, input);
      pending.clear();
    };
    return w;
  };
  listen('js-solve', async ({ id, input }) => {
    if (worker === 'broken') return solveHere(id, input);
    try {
      worker ??= startWorker();
      const w = await worker;
      pending.set(id, input);
      w.postMessage({ id, input });
      // a worker that never answers counts as broken too (Rust gives up at 90 s; parsing a new
      // player on a slow phone can take a while, so don't start a second copy too early)
      setTimeout(() => { if (pending.delete(id)) { worker = 'broken'; solveHere(id, input); } }, 70000);
    } catch {
      worker = 'broken';
      solveHere(id, input);
    }
  });
}

// ---------------------------------------------------------------- window frame (desktop)
// Windows/Linux: no system frame — our top bar drags the window and carries the buttons.
// macOS: the system traffic lights sit on our bar, so the brand moves right.
{
  const tw = window.__TAURI__?.window?.getCurrentWindow?.();
  const mobile = !!window.AndroidMedia || /Android|iPhone|iPad/.test(navigator.userAgent);
  const mac = /Mac/.test(navigator.platform || navigator.userAgent);
  if (tw && !mobile && mac) {
    document.body.classList.add('frame-mac');
    const fs = async () => document.body.classList.toggle('fullscreen', await tw.isFullscreen().catch(() => false));
    fs();
    window.addEventListener('resize', debounce(fs, 120));
  }
  if (tw && !mobile && !mac) {
    document.body.classList.add('frame-custom');
    $('#win-ctrls').hidden = false;
    $('#win-min').onclick = () => tw.minimize();
    $('#win-max').onclick = () => tw.toggleMaximize();
    $('#win-close').onclick = () => tw.close();
    const sync = async () => {
      const max = await tw.isMaximized().catch(() => false);
      setIcon($('#win-max'), max ? 'win-restore' : 'win-max');
      $('#win-max').title = t(max ? 'win.restore' : 'win.max');
      $('#win-max').setAttribute('aria-label', $('#win-max').title);
      document.body.classList.toggle('maximized', max);
    };
    sync();
    window.addEventListener('resize', debounce(sync, 120));
  }
}

// ---------------------------------------------------------------- stage + capsule
const audio = player.audio;
let sideMode = 'queue'; // 'queue' | 'lyrics'
const ly = { id: null, lines: [], active: -2, userScrollAt: 0 };

let lastShown = null;
function updateNowPlaying() {
  const cur = player.current;
  document.body.classList.toggle('idle', !cur);
  if (!cur) {
    $('#stage-art').src = COVER_PLACEHOLDER;
    $('#cap-art').src = COVER_PLACEHOLDER;
    document.title = 'YT Music';
    return;
  }
  $('#stage-art').src = cur.cover || COVER_PLACEHOLDER;
  $('#cap-art').src = cur.cover || COVER_PLACEHOLDER;
  if (lastShown !== cur.id) { lastShown = cur.id; for (const el of [$('.stage-art-wrap'), $('#stage-title'), $('#stage-artist')]) { el.classList.remove('swap'); void el.offsetWidth; el.classList.add('swap'); } }
  $('#stage-title').textContent = cur.title;
  $('#stage-artist').innerHTML = artistLinks(cur.artists) + (cur.album?.id ? ` · <a class="link" data-go="album" data-id="${esc(cur.album.id)}">${esc(cur.album.name)}</a>` : '');
  $('#stage-ctx').textContent = player.context?.title ? t('stage.from', { title: player.context.title }) : t('stage.now');
  $('#cap-title').textContent = cur.title;
  const liked = state.liked.has(cur.id);
  const like = $('#np-like');
  like.setAttribute('aria-pressed', liked);
  setIcon(like, liked ? 'heart-fill' : 'heart');
  like.querySelector('span').textContent = t(liked ? 'chip.liked' : 'chip.like');
  const cl = $('#cap-like');
  cl.classList.toggle('on', liked);
  setIcon(cl, liked ? 'heart-fill' : 'heart');
  cl.title = t(liked ? 'menu.unlike' : 'pl.like'); cl.setAttribute('aria-label', cl.title);
  document.title = `${cur.title} · ${artistNames(cur.artists)}`;
  updateDlButtons();
  // Ambient light takes the cover's colour (computed in Rust).
  if (cur.cover) api.coverColor(cur.cover).then((c) => { if (player.current?.id === cur.id) document.documentElement.style.setProperty('--tint-a', c); }).catch(() => {});
  // Offline badge (the encrypted downloaded copy is playing).
  const badge = $('#np-quality');
  badge.innerHTML = '';
  api.streamQuality(cur.id).then((q) => {
    if (player.current?.id !== cur.id) return;
    badge.innerHTML = q.offline ? `${icon('lock')}${esc(t('qp.offline'))}` : '';
    badge.title = q.offline ? t('quality.offline') : '';
  }).catch((e) => {
    if (MOBILE_APP && /LOGIN_REQUIRED|age|Sign in/i.test(errText(e)) && player.current?.id === cur.id) {
      toast(t(state.settings.logged_in ? 'toast.ageAndroid' : 'toast.ageSignIn'), 'err');
    }
  });
}

function updateState() {
  const playing = player.playing;
  for (const b of [$('#btn-play'), $('#cap-play'), $('#side-play')].filter(Boolean)) {
    setIcon(b, playing ? 'pause' : 'play');
    b.title = t(playing ? 'pl.pause' : 'pl.play'); b.setAttribute('aria-label', b.title);
  }
  document.body.classList.toggle('paused', !playing);
  for (const sh of [$('#btn-shuffle'), $('#cap-shuffle')]) sh.setAttribute('aria-pressed', player.shuffle);
  for (const rp of [$('#btn-repeat'), $('#cap-repeat')]) {
    rp.setAttribute('aria-pressed', player.repeat !== 'off');
    setIcon(rp, player.repeat === 'one' ? 'repeat-one' : 'repeat');
    rp.title = t({ off: 'pl.repeatOff', all: 'pl.repeatAll', one: 'pl.repeatOne' }[player.repeat] || 'pl.repeat'); rp.setAttribute('aria-label', rp.title);
  }
  const cur = player.current?.id;
  $$('.trow').forEach((r) => {
    r.classList.toggle('playing', r.dataset.tid === cur);
    r.classList.toggle('paused', r.dataset.tid === cur && !playing);
  });
  if ('mediaSession' in navigator) navigator.mediaSession.playbackState = playing ? 'playing' : 'paused';
}

let seeking = false;
function updateTime() {
  const d = audio.duration, c = audio.currentTime;
  const dur = isFinite(d) ? d : player.current?.duration;
  $('#t-dur').textContent = fmt(dur);
  const f = isFinite(d) && d > 0 ? c / d : 0;
  if (!seeking) {
    $('#t-cur').textContent = fmt(c);
    const s = $('#seek');
    s.value = Math.round(f * 1000);
    s.style.setProperty('--p', `${f * 100}%`);
    s.setAttribute('aria-valuetext', `${fmt(c)} / ${fmt(dur)}`);
  }
  if (!seeking) {
    const cs = $('#cap-seek');
    cs.value = Math.round(f * 1000);
    cs.style.setProperty('--p', `${f * 100}%`);
    cs.setAttribute('aria-valuetext', `${fmt(c)} / ${fmt(dur)}`);
  }
  const cur = player.current;
  if (cur) { const sub = $('#cap-sub'), names = player.ad ? t('pl.ad') : artistNames(cur.artists); if (sub.textContent !== names) sub.textContent = names; sub.title = `${fmt(c)} / ${fmt(dur)}`; }
  if ('mediaSession' in navigator && isFinite(d) && d > 0) {
    try { navigator.mediaSession.setPositionState({ duration: d, position: Math.min(c, d), playbackRate: 1 }); } catch { /* ignore */ }
  }
}
function updateVolume() {
  const v = audio.muted ? 0 : audio.volume;
  for (const r of [$('#vol'), $('#capvol-range')]) {
    r.value = Math.round(v * 100);
    r.style.setProperty('--p', `${v * 100}%`);
  }
  $('#capvol-val').textContent = Math.round(v * 100);
  setIcon($('#btn-mute'), v === 0 ? 'volume-off' : 'volume');
  setIcon($('#capvol-btn'), v === 0 ? 'volume-off' : 'volume');
}
// Both the stage slider and the capsule's bottom edge seek.
for (const s of [$('#seek'), $('#cap-seek')]) {
  s.addEventListener('input', () => {
    seeking = true;
    const f = s.value / 1000;
    s.style.setProperty('--p', `${f * 100}%`);
    if (isFinite(audio.duration)) $('#t-cur').textContent = fmt(f * audio.duration);
  });
  s.addEventListener('change', () => { player.seek(s.value / 1000); seeking = false; });
  s.addEventListener('pointerdown', (e) => e.stopPropagation());
}
{
  const wheel = (e) => { e.preventDefault(); player.setVolume(Math.max(0, Math.min(1, audio.volume - Math.sign(e.deltaY) * 0.05))); };
  for (const v of [$('#vol'), $('#capvol-range')]) {
    v.addEventListener('input', () => { if (audio.muted && v.value > 0) player.toggleMute(); player.setVolume(v.value / 100); });
  }
  $('#vol').addEventListener('wheel', wheel, { passive: false });
  // The bubble: a round button beside the capsule that grows up into a slider.
  const box = $('#capvol'), btn = $('#capvol-btn');
  const setOpen = (on) => { box.classList.toggle('open', on); btn.setAttribute('aria-expanded', on); };
  btn.onclick = () => setOpen(!box.classList.contains('open'));
  box.addEventListener('wheel', wheel, { passive: false });
  $('#capvol-val').onclick = () => player.toggleMute();
  document.addEventListener('pointerdown', (e) => { if (!e.target.closest('#capvol')) setOpen(false); });
}

// Side panel: up next, or synced lyrics.
async function renderSide() {
  const side = $('#side');
  const cur = player.current;
  if (!cur) { side.innerHTML = ''; return; }
  const head = `<div class="side-grab" aria-hidden="true"></div>
    <div class="side-mini"><img src="${esc(thumb(cur.cover))}" alt=""><div class="ttext"><div class="ttitle">${esc(cur.title)}</div><div class="tartist">${esc(artistNames(cur.artists))}</div></div>
      <button class="tbtn" id="side-play" data-act="toggle-play" title="${esc(t(player.playing ? 'pl.pause' : 'pl.play'))}">${icon(player.playing ? 'pause' : 'play')}</button>
      <button class="iconbtn side-close" data-act="side-close" title="${esc(t('stage.collapse'))}" aria-label="${esc(t('stage.collapse'))}">${icon('chev-down')}</button></div>`;
  const tabs = `${head}<div class="side-tabs" role="tablist">${[['queue', t('stage.upNext')], ['lyrics', t('rp.lyrics')]].map(([k, l]) =>
    `<button class="side-tab" role="tab" data-side="${k}" aria-selected="${sideMode === k}">${esc(l)}</button>`).join('')}</div>`;
  if (sideMode === 'queue') {
    const ctx = player.context?.title ? `<div class="mono">${esc(t('rp.nextFrom', { title: player.context.title }))}</div>` : '';
    side.innerHTML = `${tabs}<div class="side-body">${ctx}${queueMarkupUpNext()}</div>`;
    return;
  }
  if (ly.id === cur.id) { side.innerHTML = `${tabs}<div class="side-body">${ly.html}</div>`; syncLyrics(true); return; }
  side.innerHTML = `${tabs}<div class="side-body"><p class="ly-empty">${esc(t('lyrics.loading'))}</p></div>`;
  const l = await api.lyrics(cur).catch(() => null);
  if (player.current?.id !== cur.id || sideMode !== 'lyrics') return;
  ly.id = cur.id; ly.lines = l?.synced || []; ly.active = -2;
  ly.html = lyricsMarkup(l);
  side.innerHTML = `${tabs}<div class="side-body">${ly.html}</div>`;
  syncLyrics(true);
}
document.addEventListener('click', (e) => {
  const tab = e.target.closest('.side-tab');
  if (tab) { sideMode = tab.dataset.side; renderSide(); }
  const open = e.target.closest('[data-side-open]');
  if (open) openSide(open.dataset.sideOpen);
});

// Narrow window / phone: the player is alone on screen; Up next and Lyrics rise over it as a
// sheet (the player steps back) and go down with the chevron, Esc, or a swipe down.
function openSide(which) {
  if (which && which !== sideMode) { sideMode = which; renderSide(); }
  if (!stageCompact()) return;
  document.body.classList.add('side-open');
  ensureBackEntry();
}
function closeSide() {
  if (!document.body.classList.contains('side-open')) return false;
  document.body.classList.remove('side-open');
  return true;
}
{
  const side = $('#side');
  let y0 = null, dy = 0;
  side.addEventListener('pointerdown', (e) => {
    if (!document.body.classList.contains('side-open') || !e.target.closest('.side-grab, .side-mini, .side-tabs') || e.target.closest('button')) return;
    y0 = e.clientY; dy = 0; side.style.transition = 'none';
    side.setPointerCapture?.(e.pointerId);
  });
  side.addEventListener('pointermove', (e) => {
    if (y0 == null) return;
    dy = Math.max(0, e.clientY - y0);
    side.style.transform = `translateY(${dy}px)`;
  });
  const end = () => {
    if (y0 == null) return;
    y0 = null; side.style.transition = ''; side.style.transform = '';
    if (dy > 80) closeSide();
  };
  side.addEventListener('pointerup', end);
  side.addEventListener('pointercancel', end);
  // a body swipe down on the lyrics/queue (already at the top) closes it too
  let ty = null;
  side.addEventListener('touchstart', (e) => { const b = $('.side-body'); ty = b && b.scrollTop <= 0 && e.target.closest('.side-body') ? e.touches[0].clientY : null; }, { passive: true });
  side.addEventListener('touchmove', (e) => { if (ty != null && e.touches[0].clientY - ty > 110) { ty = null; closeSide(); } }, { passive: true });
  matchMedia('(max-width: 1199px)').addEventListener('change', (m) => { if (!m.matches) closeSide(); });
}
function queueMarkupUpNext() {
  const q = player.queue, i = player.index;
  const up = q.slice(i + 1, i + 51);
  if (!up.length) return `<p class="empty-line">${esc(t('rp.emptyQueue'))}</p>`;
  const lid = registerList(q);
  return up.map((x, k) => { const idx = i + 1 + k; return `<div class="qrow" data-qidx="${idx}" data-list="${lid}" data-idx="${idx}" tabindex="0">
    <img src="${esc(thumb(x.cover))}" alt="" loading="lazy" decoding="async" data-onerr="hide"><div class="ttext"><div class="ttitle">${esc(x.title)}</div>
    <div class="tartist">${esc(artistNames(x.artists))}</div></div>
    <button class="icon-btn" data-act="q-remove" aria-label="${esc(t('rp.close'))}">${icon('close')}</button></div>`; }).join('');
}
function lyricsMarkup(l) {
  if (!l) return `<p class="ly-empty">${esc(t('lyrics.none'))}</p>`;
  if (l.instrumental) return `<p class="ly-empty">♪ ${esc(t('lyrics.instrumental'))}</p>`;
  if (l.synced?.length) {
    return `<div class="ly-lines">${l.synced.map((x, i) =>
      `<p class="ly-line ${x.text ? '' : 'gap'}" data-i="${i}" data-t="${x.t}">${x.text ? esc(x.text) : '♪'}</p>`).join('')}</div>
      <p class="ly-source">${esc(t('lyrics.source', { src: l.source }))}</p>`;
  }
  return `<div class="ly-plain">${esc(l.plain || '').replace(/\n/g, '<br>')}</div>
    <p class="ly-source">${esc(t('lyrics.notSynced'))} · ${esc(t('lyrics.source', { src: l.source }))}</p>`;
}
function syncLyrics(force = false) {
  if (sideMode !== 'lyrics' || !ly.lines.length || ly.id !== player.current?.id) return;
  const ms = audio.currentTime * 1000 + 250; // small lead so the line lights up on the beat
  let lo = 0, hi = ly.lines.length - 1, idx = -1;
  while (lo <= hi) { const m = (lo + hi) >> 1; if (ly.lines[m].t <= ms) { idx = m; lo = m + 1; } else hi = m - 1; }
  if (idx === ly.active && !force) return;
  ly.active = idx;
  $$('.ly-line').forEach((el, i) => { el.classList.toggle('active', i === idx); el.classList.toggle('past', i < idx); });
  const el = $(`.ly-line[data-i="${idx}"]`), box = $('.side-body');
  if (el && box && Date.now() - ly.userScrollAt > 3000) {
    const top = box.scrollTop + el.getBoundingClientRect().top - box.getBoundingClientRect().top - box.clientHeight * 0.35;
    box.scrollTo({ top, behavior: force ? 'auto' : 'smooth' });
  }
}
document.addEventListener('wheel', (e) => { if (e.target.closest('.side-body')) ly.userScrollAt = Date.now(); }, { passive: true });
document.addEventListener('touchmove', (e) => { if (e.target.closest('.side-body')) ly.userScrollAt = Date.now(); }, { passive: true });

$('#btn-play').onclick = $('#cap-play').onclick = () => player.toggle();
$('#btn-next').onclick = $('#cap-next').onclick = () => player.next();
$('#btn-prev').onclick = $('#cap-prev').onclick = () => player.prev();
$('#btn-shuffle').onclick = $('#cap-shuffle').onclick = () => player.toggleShuffle();
$('#btn-repeat').onclick = $('#cap-repeat').onclick = () => player.cycleRepeat();
$('#btn-mute').onclick = () => player.toggleMute();
$('#np-like').onclick = $('#cap-like').onclick = () => player.current && toggleLike(player.current);
$('#np-radio').onclick = () => startRadio(player.current);
$('#np-dl').onclick = () => player.current && downloadTracks([player.current]);
$('#cap-lyrics').onclick = () => { toStage(); openSide('lyrics'); };
$('#cap-queue').onclick = () => queueSheet();
$('#cap-open').onclick = $('#cap-art').onclick = () => toStage();
$('#np-add').onclick = $('#cap-add').onclick = (e) => {
  const above = e.currentTarget.id === 'cap-add'; // the capsule sits at the bottom: open upwards
  const cur = player.current;
  if (!cur) return;
  const r = e.currentTarget.getBoundingClientRect();
  const items = [{ label: t('menu.newPlaylist'), run: () => newPlaylist([cur]) },
    ...state.playlists.map((p) => ({ label: p.name, run: () => addTo(p, [cur]) }))];
  showMenu(r.left, r.bottom + 6, items);
  if (above) menu.style.top = `${Math.max(8, r.top - menu.offsetHeight - 10)}px`;
};
// Drag the stage cover down → open the layers; swipe the capsule up → back to the stage.
function swipe(el, dir, fn) {
  let y0 = null;
  el.addEventListener('pointerdown', (e) => { if (!e.target.closest('button:not(.cap-meta)')) y0 = e.clientY; });
  window.addEventListener('pointerup', () => { y0 = null; });
  el.addEventListener('pointermove', (e) => {
    if (y0 == null) return;
    const dy = e.clientY - y0;
    if (dir > 0 ? dy > 70 : dy < -30) { y0 = null; fn(); }
  });
}
swipe($('.stage-art-wrap'), 1, () => actions['to-browse']());
swipe($('#capsule'), -1, toStage);

player.on('track', () => { updateNowPlaying(); renderSide(); if (mode() === 'browse') updateState(); });
player.on('queue', () => { if (sideMode === 'queue') renderSide(); if ($('#queue-sheet')) $('#queue-sheet > div:last-child').innerHTML = queueMarkup(); });
player.on('state', updateState);
player.on('time', () => { updateTime(); syncLyrics(); });
player.on('volume', updateVolume);
player.on('error', (x) => toast(t('toast.playFail', { title: x.title }), 'err'));

// keyboard shortcuts
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    if (closeOverlays() || closeSide()) return;
    if (document.activeElement === sInput) return;
    if (mode() === 'browse') toStage();
    return;
  }
  if ((e.key === 'k' && (e.ctrlKey || e.metaKey)) || (e.key === '/' && !e.target.closest('input, textarea, select'))) { e.preventDefault(); focusSearch(); return; }
  if (e.target.closest('input, textarea, select')) return;
  if (e.code === 'Space' && !e.target.closest('button, [tabindex], a')) { e.preventDefault(); player.toggle(); }
  else if (e.code === 'ArrowRight' && e.ctrlKey) player.next();
  else if (e.code === 'ArrowLeft' && e.ctrlKey) player.prev();
  else if (e.altKey && e.key === 'ArrowLeft') navBack();
  else if (e.code === 'ArrowRight' && !e.target.closest('[role=menu]')) audio.currentTime = Math.min(audio.duration || 0, audio.currentTime + 5);
  else if (e.code === 'ArrowLeft' && !e.target.closest('[role=menu]')) audio.currentTime = Math.max(0, audio.currentTime - 5);
  else if (e.code === 'ArrowUp' && e.ctrlKey) player.setVolume(audio.volume + 0.05);
  else if (e.code === 'ArrowDown' && e.ctrlKey) player.setVolume(audio.volume - 0.05);
  else if (e.key === 'l' && e.altKey) { if (player.current) toggleLike(player.current); }
});
// Arrow keys move between items of an open menu.
document.addEventListener('keydown', (e) => {
  if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
  const box = e.target.closest('.pop, .ctxmenu');
  if (!box) return;
  e.preventDefault();
  const items = $$('.mi', box).filter((m) => m.offsetParent);
  const i = items.indexOf(document.activeElement);
  items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus();
});
window.addEventListener('mouseup', (e) => {
  // With a history entry armed, the webview's own back navigation fires popstate instead.
  if (e.button === 3 && !backArmed) navBack();
  if (e.button === 4 && nav.pos < nav.stack.length - 1) { nav.pos++; if (mode() !== 'browse') morph('browse'); render(); }
});

// ---------------------------------------------------------------- backend events
listen('download-progress', (p) => {
  state.dlProgress.set(p.id, p);
  if (p.id === player.current?.id) setTimeout(updateDlButtons, 0);
  if (p.state === 'done') {
    state.downloaded.add(p.id);
    state.dlProgress.delete(p.id);
    renderDock();
  }
  if (p.state === 'error') { state.dlProgress.delete(p.id); toast(t('toast.dlError', { e: p.error || '' }), 'err'); }
  $$(`.trow[data-tid="${CSS.escape(p.id)}"] .tartist`).forEach((el) => {
    const mark = el.querySelector('.dl-mark');
    const html = p.state === 'done' ? dlMarkDone() : p.state === 'error' ? '' : dlMarkBusy(p.progress);
    if (mark) mark.outerHTML = html; else el.insertAdjacentHTML('afterbegin', html);
  });
});

// ---------------------------------------------------------------- sleep timer (logic in Rust)
let sleepTick = null;
function showSleep(st) {
  const btn = $('#btn-sleep');
  const left = $('#sleep-left');
  clearInterval(sleepTick);
  const active = st.remaining_s != null || st.end_of_track;
  btn.classList.toggle('on', active);
  if (st.end_of_track) { left.textContent = '♪'; return; }
  if (st.remaining_s == null) { left.textContent = ''; return; }
  let s = st.remaining_s;
  const draw = () => { left.textContent = s >= 60 ? `${Math.ceil(s / 60)}′` : `${s}″`; };
  draw();
  sleepTick = setInterval(() => { s = Math.max(0, s - 1); draw(); if (!s) clearInterval(sleepTick); }, 1000);
}
async function setSleep(minutes, endOfTrack = false) {
  try {
    const st = await api.setSleepTimer(minutes, endOfTrack);
    showSleep(st);
    toast(minutes ? t('sleep.set', { n: minutes }) : endOfTrack ? t('sleep.endSet') : t('sleep.off'));
  } catch (e) { toast(errText(e), 'err'); }
}
$('#btn-sleep').onclick = (e) => {
  const r = e.currentTarget.getBoundingClientRect();
  const items = [15, 30, 45, 60, 90].map((m) => ({ label: t('sleep.min', { n: m }), run: () => setSleep(m) }));
  items.push({ label: t('sleep.end'), run: () => setSleep(null, true) }, '-', { label: t('sleep.cancel'), run: () => setSleep(null, false) });
  showMenu(r.left, r.bottom + 6, items);
};
// Fire: fade out over ~4 s, pause, restore volume.
listen('sleep-timer', () => {
  showSleep({ remaining_s: null, end_of_track: false });
  const start = audio.volume;
  let k = 0;
  const fade = setInterval(() => {
    k++;
    audio.volume = Math.max(0, start * (1 - k / 20));
    if (k >= 20) { clearInterval(fade); audio.pause(); audio.volume = start; toast(t('sleep.fired')); }
  }, 200);
});

// ---------------------------------------------------------------- global hotkeys (desktop)
listen('hotkey', (action) => {
  if (action === 'toggle') player.toggle();
  else if (action === 'next') player.next();
  else if (action === 'prev') player.prev();
  else if (action === 'like' && player.current) toggleLike(player.current);
});

// ---------------------------------------------------------------- updates
const upd = { info: null, error: null, busy: false };

function updateStatusText() {
  const i = upd.info;
  if (upd.error) return t('upd.fail', { e: upd.error });
  if (!i) return '';
  return i.available ? t('upd.available', { v: i.version, cur: i.current }) : t('upd.latest', { v: i.current });
}

function showUpdateButton() {
  const b = $('#update-btn');
  const i = upd.info;
  b.classList.toggle('hidden', !i?.available);
  if (i?.available && !upd.busy) b.textContent = t('upd.short', { v: i.version });
}

async function checkForUpdate(loud = false) {
  try {
    upd.info = await api.checkUpdate();
    upd.error = null;
  } catch (e) {
    upd.error = errText(e);
    if (loud) toast(t('upd.fail', { e: upd.error }), 'err');
  }
  showUpdateButton();
}

async function installUpdate() {
  if (upd.busy || !upd.info?.available) return;
  upd.busy = true;
  const b = $('#update-btn');
  b.disabled = true;
  b.textContent = t('upd.downloading', { p: 0 });
  try {
    await api.installUpdate(); // desktop: app restarts when done; Android: browser opens the APK
    if (upd.info.kind === 'apk') toast(t('upd.apkHint'));
  } catch (e) {
    toast(t('upd.fail', { e: errText(e) }), 'err');
  } finally {
    upd.busy = false;
    b.disabled = false;
    showUpdateButton();
  }
}

listen('update-progress', (p) => {
  const pct = p.total ? Math.round((p.downloaded / p.total) * 100) : 0;
  $('#update-btn').textContent = t('upd.downloading', { p: pct });
});

listen('google-login', async (r) => {
  if (r.ok) {
    toast(t('toast.loggedIn'));
    state.settings = await api.settings();
    renderAccount();
    await sync();
  } else {
    toast(t('toast.googleFail', { e: r.error || '' }), 'err');
  }
  if (nav.stack[nav.pos]?.view === 'settings') render();
});
// One time after an update: the old sign-in did not cover the web player (webplayer.rs).
listen('signed-out', async () => {
  state.settings = await api.settings();
  state.remotePlaylists = [];
  renderAccount(); renderDock();
  if (mode() === 'browse') render();
  toast(t('eng.signInAgain'));
});
listen('google-login-closed', () => { if (nav.stack[nav.pos]?.view === 'settings') render(); });
// Android sign-in result: the YouTube cookies from LoginActivity (null if cancelled).
window.__androidLogin = async (cookie) => {
  if (!cookie) { if (nav.stack[nav.pos]?.view === 'settings') render(); return; }
  try {
    await api.login(cookie);
    state.settings = await api.settings();
    renderAccount();
    api.warmSolver().catch(() => {});
    toast(t('toast.loggedIn'));
    await sync();
  } catch (e) { toast(t('toast.googleFail', { e: errText(e) }), 'err'); }
  if (nav.stack[nav.pos]?.view === 'settings') render();
};

// ---------------------------------------------------------------- boot
(async function boot() {
  translateDom();
  hydrateIcons();
  eq.init(player.element);
  await initApi();
  state.settings = await api.settings().catch(() => ({}));
  renderAccount();
  await refreshLibrary().catch((e) => console.error(e));
  refreshRemote();
  if (state.settings.logged_in) api.syncAccount().then(refreshRemote).catch(() => {});
  await player.restore().catch((e) => console.error(e));
  updateNowPlaying();
  updateState();
  renderSide();
  go(navigator.onLine ? 'home' : 'downloads');
  api.sleepState().then(showSleep).catch(() => {});
  setTimeout(syncAutoDl, 8000);
  // Signed in before account info existed (or the avatar changed): fetch it once per start.
  // Android (no yt-dlp): prepare YouTube's player for 18+ songs before the first one is played.
  if (state.settings.logged_in && MOBILE_APP) setTimeout(() => api.warmSolver().catch(() => {}), 5000);
  if (state.settings.logged_in) {
    api.refreshAccount()
      .then(async () => { state.settings = await api.settings(); renderAccount(); })
      .catch(() => {});
  }
  // Quietly look for a new version a few seconds after start.
  setTimeout(() => checkForUpdate(false), 4000);
})();
