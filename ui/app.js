import { api, initApi, listen } from './api.js';
import { icon, hydrateIcons, setIcon } from './icons.js';
import { player } from './player.js';
import { t, translateDom, LANGS, lang, setLang, locale } from './i18n.js';

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
// Themes: "classic" (default) and "glass" (design by @dopedasya). theme-boot.js applies the
// saved one before first paint; this switches live.
const currentTheme = () => document.documentElement.dataset.theme || 'classic';
function applyTheme(theme) {
  if (theme !== 'glass') theme = 'classic';
  try { localStorage.setItem('theme', theme); } catch { /* ignore */ }
  document.documentElement.dataset.theme = theme;
  $('#css-classic').disabled = theme !== 'classic';
  $('#css-glass').disabled = theme !== 'glass';
}
const regionName = (code) => {
  try { return new Intl.DisplayNames([locale()], { type: 'region' }).of(code) || code; } catch { return code; }
};
const COVER_PLACEHOLDER = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='-12 -12 48 48' fill='none' stroke='%236f6f7c' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Crect x='-12' y='-12' width='48' height='48' fill='%231c1c22' stroke='none'/%3E%3Cpath d='M9 18V5l12-2v13'/%3E%3Ccircle cx='6' cy='18' r='3'/%3E%3Ccircle cx='18' cy='16' r='3'/%3E%3C/svg%3E";
const qualityLabel = (codec, kbps) => [codec, kbps ? `${kbps} kbps` : ''].filter(Boolean).join(' · ');
const GOOGLE_G = '<svg class="g-logo" viewBox="0 0 48 48" aria-hidden="true"><path fill="#EA4335" d="M24 9.5c3.5 0 6.6 1.2 9.1 3.6l6.8-6.8C35.8 2.4 30.3 0 24 0 14.6 0 6.6 5.4 2.6 13.3l7.9 6.2C12.4 13.7 17.7 9.5 24 9.5z"/><path fill="#4285F4" d="M46.1 24.5c0-1.6-.1-3.1-.4-4.5H24v9h12.4c-.5 2.9-2.2 5.4-4.6 7l7.1 5.5c4.2-3.9 7.2-9.6 7.2-17z"/><path fill="#FBBC05" d="M10.5 28.6A14.6 14.6 0 0 1 9.5 24c0-1.6.3-3.2.8-4.6l-7.9-6.2A24 24 0 0 0 0 24c0 3.9.9 7.5 2.6 10.7l7.9-6.1z"/><path fill="#34A853" d="M24 48c6.5 0 11.9-2.1 15.9-5.8l-7.1-5.5c-2 1.4-4.6 2.2-8.8 2.2-6.3 0-11.6-4.2-13.5-10l-7.9 6.1C6.6 42.6 14.6 48 24 48z"/></svg>';
const artistLinks = (artists = []) => artists.map((a) =>
  a.id ? `<a class="link" data-go="artist" data-id="${esc(a.id)}">${esc(a.name)}</a>` : esc(a.name)).join(', ');
const artistNames = (artists = []) => artists.map((a) => a.name).join(', ');
const errText = (e) => (typeof e === 'string' ? e : e?.message || JSON.stringify(e));
const debounce = (fn, ms) => { let h; return (...a) => { clearTimeout(h); h = setTimeout(() => fn(...a), ms); }; };

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
  renderSidebar();
}

async function refreshRemote() {
  try { state.remotePlaylists = await api.remotePlaylists(); } catch { state.remotePlaylists = []; }
  renderSidebar();
}

// ---------------------------------------------------------------- router
const nav = { stack: [], pos: -1 };
let renderToken = 0;

function go(view, params = {}, push = true) {
  if (push) {
    nav.stack = nav.stack.slice(0, nav.pos + 1);
    nav.stack.push({ view, params, scroll: 0 });
    nav.pos = nav.stack.length - 1;
  }
  render();
}

async function render() {
  const entry = nav.stack[nav.pos];
  const token = ++renderToken;
  const el = $('#view');
  $$('.nav-item, .tab').forEach((a) => a.classList.toggle('active', a.dataset.nav === entry.view
    || (entry.view === 'local-playlist' && a.dataset.pl == entry.params.id)
    || (entry.view === 'playlist' && a.dataset.rpl === entry.params.id)));
  $('#nav-back').disabled = nav.pos <= 0;
  $('#nav-fwd').disabled = nav.pos >= nav.stack.length - 1;
  el.innerHTML = '<div class="loading">' + icon('spinner', 'spin') + '</div>';
  pageColor = null;
  $('#main').classList.toggle('lyrics-mode', entry.view === 'lyrics');
  $('#btn-lyrics').classList.toggle('on', entry.view === 'lyrics');
  try {
    const html = await (views[entry.view] || views.home)(entry.params);
    if (token !== renderToken) return;
    el.innerHTML = html;
    $('#main').scrollTop = entry.scroll || 0;
    applyPageColor(token);
    afterRender[entry.view]?.(entry.params);
  } catch (e) {
    if (token !== renderToken) return;
    console.error(e);
    el.innerHTML = `<div class="empty"><h2>${esc(t('err.title'))}</h2><p>${esc(errText(e))}</p>
      <button class="pill-btn" data-act="reload">${esc(t('err.reload'))}</button></div>`;
  }
}

$('#main').addEventListener('scroll', () => {
  const e = nav.stack[nav.pos];
  if (e) e.scroll = $('#main').scrollTop;
  $('#topbar').classList.toggle('scrolled', $('#main').scrollTop > 10);
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
    return `<div class="trow ${x.id === cur ? 'playing' : ''}" data-list="${lid}" data-idx="${i}" data-tid="${esc(x.id)}">
      <div class="tnum"><span class="n">${opts.numbers === false ? '' : i + 1}</span><span class="eq"><b></b><b></b><b></b></span>${icon('play', 'pl')}</div>
      <div class="tmain">
        ${opts.covers === false ? '' : `<img class="tcover" loading="lazy" src="${esc(x.cover || '')}" alt="" data-onerr="hide">`}
        <div class="ttext"><div class="ttitle">${esc(x.title)}</div><div class="tartist">${dlIcon}${opts.badges?.[x.id] ? `<span class="qbadge">${esc(opts.badges[x.id])}</span>` : ''}${artistLinks(x.artists)}</div></div>
      </div>
      ${opts.album === false ? '' : `<div class="talbum">${x.album ? `<a class="link" data-go="album" data-id="${esc(x.album.id)}">${esc(x.album.name)}</a>` : ''}</div>`}
      <div class="tright">
        <button class="icon-btn like ${liked ? 'on' : ''}" data-act="like" title="${esc(t('tr.like'))}">${icon(liked ? 'heart-fill' : 'heart')}</button>
        <span class="tdur">${x.duration ? fmt(x.duration) : ''}</span>
        <button class="icon-btn more" data-act="menu" title="${esc(t('tr.more'))}">${icon('more')}</button>
      </div>
    </div>`;
  }).join('');
  const noAlbum = opts.album === false;
  const head = `<div class="thead ${noAlbum ? 'noalbum' : ''}"><div>#</div><div>${esc(t('col.title'))}</div>${noAlbum ? '' : `<div>${esc(t('col.album'))}</div>`}<div class="tright">${icon('clock')}</div></div>`;
  return `<div class="tracklist ${noAlbum ? 'noalbum' : ''}" data-ctx='${esc(JSON.stringify(opts.ctx || null))}'>${opts.header === false ? '' : head}${rows}</div>`;
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
    ? `<img loading="lazy" src="${esc(item.cover)}" alt="" data-onerr="remove">`
    : `<div class="ph">${icon(kind === 'artist' ? 'user' : 'music')}</div>`;
  return `<div class="card" data-go="${kind === 'local' ? 'local-playlist' : kind}" data-id="${esc(item.id)}">
    <div class="card-art ${round ? 'round' : ''}">${cover}
      ${kind !== 'artist' ? `<button class="card-play" data-act="play-collection" data-kind="${kind}" data-id="${esc(item.id)}" title="${esc(t('act.play'))}">${icon('play')}</button>` : ''}
    </div>
    <div class="card-title">${esc(title)}</div>
    <div class="card-sub">${esc(sub)}</div>
  </div>`;
}

/** Section heading: optional mono "// label" above the title, optional right-hand content. */
const secHead = (title, label = '', right = '') => `<div class="sec-head"><div>${label ? `<div class="sec-label">// ${esc(label)}</div>` : ''}<h2>${esc(title)}</h2></div>${right}</div>`;

const shelf = (title, items, kind, label = '') => !items?.length ? '' :
  `<section class="shelf">${secHead(title, label)}<div class="grid">${items.map((i) => card(kind, i)).join('')}</div></section>`;

function collectionHeader({ cover, kind, title, meta, round = false, actions = '', phClass = '' }) {
  return `<div class="hero">
    <div class="hero-art ${round ? 'round' : ''}">${cover ? `<img src="${esc(cover)}" alt="">` : `<div class="ph ${phClass}">${icon(phClass ? (phClass === 'liked-cover' ? 'heart-fill' : 'download') : 'music')}</div>`}</div>
    <div class="hero-text">
      <div class="hero-kind">${esc(kind)}</div>
      <h1 class="${title.length > 30 ? 'long' : ''}">${esc(title)}</h1>
      <div class="hero-meta">${meta}</div>
    </div>
  </div>
  <div class="actions">${actions}</div>`;
}

const playBtn = (act, extra = '') => `<button class="play-big" data-act="${act}" ${extra} title="${esc(t('act.play'))}">${icon('play')}</button>`;
const actBtn = (act, ic, title, extra = '') => `<button class="icon-btn big" data-act="${act}" ${extra} title="${esc(title)}">${icon(ic)}</button>`;
const collectionActions = () => playBtn('play-page') + actBtn('shuffle-page', 'shuffle', t('act.shuffle'))
  + actBtn('download-page', 'download', t('act.downloadAll'));

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

// ---------------------------------------------------------------- lyrics (Spotify-style)
const ly = { id: null, lines: [], active: -2, userScrollAt: 0 };

function lyricsMarkup(l, cur) {
  const head = `<div class="ly-head"><img src="${esc(cur.cover || '')}" alt=""><div><b>${esc(cur.title)}</b>
    <span>${esc(artistNames(cur.artists))}</span></div></div>`;
  if (!l) return `${head}<p class="ly-empty">${esc(t('lyrics.none'))}</p>`;
  if (l.instrumental) return `${head}<p class="ly-empty">♪ ${esc(t('lyrics.instrumental'))}</p>`;
  if (l.synced?.length) {
    return `${head}<div class="ly-lines">${l.synced.map((x, i) =>
      `<p class="ly-line ${x.text ? '' : 'gap'}" data-i="${i}" data-t="${x.t}">${x.text ? esc(x.text) : '♪'}</p>`).join('')}</div>
      <p class="ly-source">${esc(t('lyrics.source', { src: l.source }))}</p>`;
  }
  return `${head}<div class="ly-plain">${esc(l.plain || '').replace(/\n/g, '<br>')}</div>
    <p class="ly-source">${esc(t('lyrics.notSynced'))} · ${esc(t('lyrics.source', { src: l.source }))}</p>`;
}

function syncLyrics(force = false) {
  if (nav.stack[nav.pos]?.view !== 'lyrics' || !ly.lines.length || ly.id !== player.current?.id) return;
  const ms = audio.currentTime * 1000 + 250; // small lead so the line lights up on the beat
  let lo = 0, hi = ly.lines.length - 1, idx = -1;
  while (lo <= hi) { const m = (lo + hi) >> 1; if (ly.lines[m].t <= ms) { idx = m; lo = m + 1; } else hi = m - 1; }
  if (idx === ly.active && !force) return;
  ly.active = idx;
  $$('.ly-line').forEach((el, i) => { el.classList.toggle('active', i === idx); el.classList.toggle('past', i < idx); });
  const el = $(`.ly-line[data-i="${idx}"]`);
  if (el && Date.now() - ly.userScrollAt > 3000) {
    const main = $('#main');
    const top = el.offsetTop - main.clientHeight * 0.38;
    main.scrollTo({ top, behavior: force ? 'auto' : 'smooth' });
  }
}

// ---------------------------------------------------------------- views
const views = {
  async home() {
    const [hist, home] = await Promise.all([
      api.history().catch(() => []),
      api.home().catch((e) => ({ sections: [], error: errText(e) })),
    ]);
    const hour = new Date().getHours();
    const greet = t(hour < 5 ? 'greet.night' : hour < 12 ? 'greet.morning' : hour < 18 ? 'greet.day' : 'greet.evening');
    const recent = [];
    const seen = new Set();
    for (const x of hist) { if (!seen.has(x.id)) { seen.add(x.id); recent.push(x); } if (recent.length >= 8) break; }
    const quick = [
      { go: 'liked', title: t('lib.liked'), cls: 'liked-cover', ic: 'heart-fill' },
      ...state.playlists.slice(0, 5).map((p) => ({ go: 'local-playlist', id: p.id, title: p.name, cover: p.cover })),
    ];
    homeRecent = recent;
    const name = state.settings.account_name;
    const heroLabel = name ? `${greet}, ${name}` : greet;
    // Both themes' markup is rendered; each theme's stylesheet hides the other's parts.
    let html = `<div class="page home">
      <h1 class="greet">${esc(greet)}</h1>
      <section class="home-hero">
        <div class="sec-label">// ${esc(heroLabel)}</div>
        <h1>${esc(t('home.heroTitle'))}</h1>
        <p>${esc(t('home.heroSub'))}</p>
        <div class="status-chip"><i class="dot"></i>${esc(t('home.heroChip', { liked: state.liked.size, offline: state.downloaded.size }))}</div>
        <div class="hero-ctas">
          <button class="pill-btn primary big" data-act="play-liked" ${state.liked.size ? '' : 'disabled'}>${icon('play')}${esc(t('home.playLiked'))}</button>
          ${recent.length ? `<button class="pill-btn big" data-act="radio-recent">${icon('radio')}${esc(t('home.startRadio'))}</button>` : ''}
        </div>
      </section>
      <section class="shelf quick-shelf">${secHead(t('home.quick'), t('home.lbl.quick'))}
      <div class="quick">${quick.map((q) => `<div class="quick-item" data-go="${q.go}" data-id="${esc(q.id ?? '')}">
        ${q.cover ? `<img src="${esc(q.cover)}" alt="">` : `<span class="qc ${q.cls || ''}">${icon(q.ic || 'music')}</span>`}
        <span>${esc(q.title)}</span></div>`).join('')}</div></section>`;
    if (recent.length) html += `<section class="shelf">${secHead(t('home.recent'), t('home.lbl.history'))}${trackRows(recent, { header: false, album: false })}</section>`;
    for (const s of home.sections || []) {
      const title = t(`sec.${s.title}`, { name: s.param || '' });
      if (s.tracks?.length) html += `<section class="shelf">${secHead(title, t('home.lbl.tracks'))}${trackRows(s.tracks.slice(0, 10), { header: false })}</section>`;
      html += shelf(title, s.albums, 'album', t('home.lbl.albums'));
      html += shelf(title, s.playlists, 'playlist', t('home.lbl.playlists'));
      html += shelf(title, s.artists, 'artist', t('home.lbl.artists'));
    }
    if (home.error) html += `<div class="note">${esc(t('home.recoFail', { e: home.error }))}</div>`;
    return html + '</div>';
  },

  async search({ q }) {
    if (!q) {
      const hist = (() => { try { return JSON.parse(localStorage.getItem('searches') || '[]'); } catch { return []; } })();
      return `<div class="page"><h1>${esc(t('search.title'))}</h1>
        ${hist.length ? `<h2>${esc(t('search.recent'))}</h2><div class="chips">${hist.map((s) => `<button class="chip" data-search="${esc(s)}">${esc(s)}</button>`).join('')}</div>`
        : `<p class="muted">${esc(t('search.hint'))}</p>`}</div>`;
    }
    const r = await api.search(q);
    const top = r.tracks.slice(0, 20);
    let html = `<div class="page"><h1 class="search-h">${esc(t('search.results', { q }))}</h1>`;
    if (!top.length && !r.albums.length && !r.artists.length && !r.playlists.length) {
      return html + `<p class="muted">${esc(t('search.none'))}</p></div>`;
    }
    if (r.artists[0]) {
      const a = r.artists[0];
      html += `<div class="top-result-row"><div class="top-result" data-go="artist" data-id="${esc(a.id)}">
        ${a.cover ? `<img class="round" src="${esc(a.cover)}" alt="">` : ''}
        <div class="tr-name">${esc(a.name)}</div><div class="tr-kind">${esc(t('kind.artist'))}</div></div>
        <div class="top-tracks"><h2>${esc(t('search.songs'))}</h2>${trackRows(top.slice(0, 5), { header: false, album: false })}</div></div>`;
      if (top.length > 5) html += `<section class="shelf">${trackRows(top.slice(5), { header: false })}</section>`;
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
    return `<div class="page artist">
      <div class="artist-hero" style="${a.cover ? `--bg-img:url('${esc(a.cover)}')` : ''}">
        <div class="hero-kind">${esc(t('kind.artist'))}</div><h1>${esc(a.name)}</h1>
        ${a.subscribers != null ? `<div class="hero-meta">${esc(subsLabel(a.subscribers))}</div>` : ''}
      </div>
      <div class="actions">${playBtn('play-page')}${actBtn('radio-artist', 'radio', t('act.artistRadio'))}</div>
      ${a.tracks.length ? `<section class="shelf"><h2>${esc(t('artist.popular'))}</h2>${trackRows(a.tracks.slice(0, 10), { header: false, ctx: pageCtx })}</section>` : ''}
      ${shelf(t('artist.albums'), a.albums, 'album')}
      ${shelf(t('artist.singles'), a.singles, 'album')}
      ${shelf(t('artist.playlists'), a.playlists, 'playlist')}
      ${shelf(t('artist.similar'), a.similar, 'artist')}
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
    })}${p.description ? `<p class="desc top">${esc(p.description)}</p>` : ''}${trackRows(p.tracks, { ctx: pageCtx })}</div>`;
  },

  async 'local-playlist'({ id }) {
    const p = await api.localPlaylist(Number(id));
    pageTracks = p.tracks; pageCtx = { kind: 'local', id: Number(id), title: p.name }; pageColor = p.tracks[0]?.cover || null;
    return `<div class="page">${collectionHeader({
      cover: p.tracks[0]?.cover, kind: t('kind.playlist'), title: p.name,
      meta: `${tracksLabel(p.tracks.length)}${p.tracks.length ? `, ${totalTime(p.tracks)}` : ''}`,
      actions: collectionActions() + actBtn('rename-local', 'edit', t('act.rename')) + actBtn('delete-local', 'trash', t('act.deletePlaylist')),
    })}${p.tracks.length ? trackRows(p.tracks, { ctx: pageCtx }) : `<p class="muted pad">${esc(t('local.empty'))}</p>`}</div>`;
  },

  async liked() {
    const tracks = await api.likedTracks();
    pageTracks = tracks; pageCtx = { kind: 'liked', title: t('lib.liked') }; pageColor = '#4a2fa8';
    return `<div class="page">${collectionHeader({
      cover: null, kind: t('kind.playlist'), title: t('lib.liked'), meta: tracksLabel(tracks.length),
      actions: collectionActions(), phClass: 'liked-cover',
    })}
    ${tracks.length ? trackRows(tracks, { ctx: pageCtx }) : `<p class="muted pad">${esc(t('liked.empty'))}</p>`}</div>`;
  },

  async downloads() {
    const items = await api.downloads();
    const tracks = items.map((d) => d.track);
    pageTracks = tracks; pageCtx = { kind: 'downloads', title: t('lib.downloads') }; pageColor = '#11603f';
    const size = items.reduce((a, d) => a + (d.size || 0), 0);
    const active = [...state.dlProgress.values()].filter((p) => p.state === 'queued' || p.state === 'downloading');
    const badges = Object.fromEntries(items.map((d) => [d.track.id, qualityLabel(d.codec, d.kbps)]));
    return `<div class="page">${collectionHeader({
      cover: null, kind: t('kind.offline'), title: t('lib.downloads'),
      meta: `${tracksLabel(tracks.length)} · ${(size / 1048576).toFixed(0)} MB · ${icon('lock')} ${esc(t('dl.encrypted'))}`,
      actions: playBtn('play-page') + actBtn('shuffle-page', 'shuffle', t('act.shuffle')), phClass: 'dl-cover',
    })}
    ${active.length ? `<div class="note">${esc(t('dl.active', { n: active.length }))}</div>` : ''}
    ${tracks.length ? trackRows(tracks, { ctx: pageCtx, badges }) : `<p class="muted pad">${esc(t('dl.empty'))}</p>`}</div>`;
  },

  async lyrics() {
    const cur = player.current;
    ly.id = null; ly.lines = []; ly.active = -2;
    if (!cur) return `<div class="page lyrics-page"><p class="ly-empty">${esc(t('rp.nothing'))}</p></div>`;
    pageColor = cur.cover || '#3a3a48';
    const l = await api.lyrics(cur).catch(() => null);
    ly.id = cur.id;
    ly.lines = l?.synced || [];
    return `<div class="page lyrics-page">${lyricsMarkup(l, cur)}</div>`;
  },

  // Phone-only screen: on desktop the same list lives in the sidebar.
  async library() {
    await refreshLibrary().catch(() => {});
    const item = (attrs, cover, title, sub) => `<a class="nav-item lib-item" ${attrs}>
      <span class="lib-cover ${cover.cls || ''}">${cover.img ? `<img src="${esc(cover.img)}" alt="">` : icon(cover.ic || 'music')}</span>
      <span class="lib-text"><b>${esc(title)}</b><small>${esc(sub)}</small></span></a>`;
    return `<div class="page library-page">
      <div class="lib-page-head"><h1>${esc(t('nav.library'))}</h1>
        <button class="icon-btn big" data-act="new-playlist" title="${esc(t('nav.newPlaylist'))}">${icon('plus')}</button></div>
      ${item('data-nav="liked"', { cls: 'liked-cover', ic: 'heart-fill' }, t('lib.liked'), `${t('lib.playlist')} · ${tracksLabel(state.liked.size)}`)}
      ${item('data-nav="downloads"', { cls: 'dl-cover', ic: 'download' }, t('lib.downloads'), `${t('lib.offline')} · ${tracksLabel(state.downloaded.size)}`)}
      ${item('data-nav="history"', { cls: 'hist-cover', ic: 'history' }, t('lib.history'), t('lib.historySub'))}
      ${state.playlists.map((p) => item(`data-nav="local-playlist" data-pl="${p.id}"`, { img: p.cover }, p.name, `${t('lib.playlist')} · ${tracksLabel(p.count)}`)).join('')}
      ${state.remotePlaylists.length ? '<div class="lib-sep">YouTube Music</div>' : ''}
      ${state.remotePlaylists.map((p) => item(`data-nav="playlist" data-rpl="${esc(p.id)}"`, { img: p.cover }, p.title, p.author || t('lib.playlist'))).join('')}
    </div>`;
  },

  async history() {
    const tracks = await api.history();
    pageTracks = tracks; pageCtx = null;
    return `<div class="page"><h1>${esc(t('lib.history'))}</h1>${tracks.length ? trackRows(tracks, { numbers: false }) : `<p class="muted">${esc(t('history.empty'))}</p>`}</div>`;
  },

  async settings() {
    const s = await api.settings();
    state.settings = s;
    return `<div class="page settings"><h1>${esc(t('set.title'))}</h1>
      <section class="set-block">
        <h2>${esc(t('set.language'))}</h2>
        <div class="row"><select id="lang-select" class="text-in select">
          ${Object.entries(LANGS).map(([k, v]) => `<option value="${k}" ${k === lang ? 'selected' : ''}>${esc(v)}</option>`).join('')}
        </select></div>
        <h2 class="sub-h">${esc(t('set.theme'))}</h2>
        <div class="seg theme-seg" role="radiogroup">
          ${['classic', 'glass'].map((th) => `<label class="seg-opt"><input type="radio" name="theme" value="${th}" ${currentTheme() === th ? 'checked' : ''}>
            <b>${esc(t(`theme.${th}`))}</b><small>${esc(t(`theme.${th}Sub`))}</small></label>`).join('')}
        </div>
        <h2 class="sub-h">${esc(t('set.region'))}</h2>
        <p class="muted small">${esc(t('set.regionHint'))}</p>
        <div class="row"><select id="region-select" class="text-in select">
          ${['auto', 'US', 'GB', 'DE', 'FR', 'PL', 'UA'].map((c) => `<option value="${c}" ${c === s.chart_region ? 'selected' : ''}>${esc(c === 'auto' ? t('set.regionAuto', { name: regionName(s.detected_region) }) : regionName(c))}</option>`).join('')}
        </select></div>
      </section>
      <section class="set-block">
        <h2>${esc(t('set.account'))}</h2>
        ${s.logged_in ? `<p>${esc(t('set.loggedIn'))}${s.account_name ? `: <b>${esc(s.account_name)}</b>` : ''}.</p>
          <div class="row"><button class="pill-btn primary" data-act="sync">${icon('sync')} ${esc(t('set.sync'))}</button>
          <button class="pill-btn" data-act="logout">${esc(t('set.logout'))}</button></div>`
        : `${s.google_login_supported ? `<p class="muted">${esc(t('set.googleHint'))}</p>
          <div class="row"><button class="google-btn" data-act="google-login">${GOOGLE_G}<span>${esc(t('set.google'))}</span></button></div>` : ''}
          <details class="advanced" ${s.google_login_supported ? '' : 'open'}><summary>${esc(t('set.advanced'))}</summary>
          <p class="muted small">${esc(t('set.loginHint'))}</p>
          <ol class="muted small"><li>${esc(t('set.step1'))}</li><li>${esc(t('set.step2'))}</li><li>${esc(t('set.step3'))}</li></ol>
          <div class="row"><label class="pill-btn">${esc(t('set.pickFile'))}<input type="file" id="cookie-file" accept=".txt" hidden></label></div>
          <textarea id="cookie-text" placeholder="${esc(t('set.cookiePh'))}"></textarea>
          <div class="row"><button class="pill-btn" data-act="login">${esc(t('set.login'))}</button></div></details>`}
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
        <div class="row"><input id="dl-dir" class="text-in" value="${esc(s.download_dir)}" spellcheck="false">
          <button class="pill-btn" data-act="save-dl-dir">${esc(t('set.save'))}</button></div>
      </section>
      <section class="set-block">
        <h2>${esc(t('set.playback'))}</h2>
        <label class="check"><input type="checkbox" id="autoplay" ${player.autoplay ? 'checked' : ''}> ${esc(t('set.autoplay'))}</label>
      </section>
      <section class="set-block">
        <h2>${esc(t('int.title'))}</h2>
        <label class="check"><input type="checkbox" data-toggle="notify_tracks" ${s.notify_tracks ? 'checked' : ''}> ${esc(t('int.notify'))}</label>
        ${s.discord_available ? `<label class="check"><input type="checkbox" data-toggle="discord_presence" ${s.discord_presence ? 'checked' : ''}> ${esc(t('int.discord'))}</label>` : ''}
        ${s.hotkeys_supported ? `<label class="check"><input type="checkbox" data-toggle="global_hotkeys" ${s.global_hotkeys ? 'checked' : ''}> ${esc(t('int.hotkeys'))}</label>
          <p class="muted small hotkey-list"><kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> ${esc(t('int.hkToggle'))} · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>→</kbd> ${esc(t('int.hkNext'))} · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>←</kbd> ${esc(t('int.hkPrev'))} · <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>L</kbd> ${esc(t('int.hkLike'))}</p>` : ''}
      </section>
      <section class="set-block">
        <h2>${esc(t('upd.title'))}</h2>
        <p id="upd-status" class="muted">${esc(updateStatusText())}</p>
        <div class="row">
          <button class="pill-btn" data-act="check-update">${icon('sync')} ${esc(t('upd.check'))}</button>
          ${upd.info?.available ? `<button class="pill-btn primary" data-act="install-update">${esc(t(upd.info.kind === 'apk' ? 'upd.apk' : 'upd.install'))}</button>` : ''}
        </div>
      </section>
      ${s.ytdlp_supported === false ? '' : `<section class="set-block">
        <h2>${esc(t('set.ytdlp'))}</h2>
        <p class="muted small">${esc(t('set.ytdlpHint'))}</p>
        <p>${s.ytdlp ? `${esc(t('set.ytdlpFound'))} <code>${esc(s.ytdlp)}</code>` : esc(t('set.ytdlpMissing'))}</p>
        <div class="row"><button class="pill-btn" data-act="install-ytdlp">${esc(t(s.ytdlp ? 'set.ytdlpUpdate' : 'set.ytdlpInstall'))}</button></div>
      </section>`}
    </div>`;
  },
};

const afterRender = {
  search({ q }) { if (q) $('#search-input').value = q; },
  lyrics() { syncLyrics(true); },
  settings() {
    $('#lang-select')?.addEventListener('change', (e) => setLang(e.target.value));
    $$('input[name=theme]').forEach((r) => r.addEventListener('change', () => applyTheme(r.value)));
    $('#region-select')?.addEventListener('change', async (e) => {
      try { await api.setChartRegion(e.target.value); toast(t('toast.settingsSaved')); } catch (err) { toast(errText(err), 'err'); }
    });
    $('#cookie-file')?.addEventListener('change', async (e) => {
      const f = e.target.files[0];
      if (f) $('#cookie-text').value = await f.text();
    });
    $('#autoplay')?.addEventListener('change', (e) => player.setAutoplay(e.target.checked));
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

// ---------------------------------------------------------------- sidebar
function renderSidebar() {
  $('#liked-count').textContent = `${t('lib.playlist')} · ${tracksLabel(state.liked.size)}`;
  $('#dl-count').textContent = `${t('lib.offline')} · ${tracksLabel(state.downloaded.size)}`;
  $('#playlist-list').innerHTML = state.playlists.map((p) => `
    <a class="nav-item lib-item" data-nav="local-playlist" data-pl="${p.id}">
      <span class="lib-cover">${p.cover ? `<img src="${esc(p.cover)}" alt="">` : icon('music')}</span>
      <span class="lib-text"><b>${esc(p.name)}</b><small>${esc(t('lib.playlist'))} · ${tracksLabel(p.count)}</small></span></a>`).join('');
  $('#remote-playlists').innerHTML = state.remotePlaylists.length
    ? '<div class="lib-sep">YouTube Music</div>' + state.remotePlaylists.map((p) => `
    <a class="nav-item lib-item" data-nav="playlist" data-rpl="${esc(p.id)}">
      <span class="lib-cover">${p.cover ? `<img src="${esc(p.cover)}" alt="">` : icon('music')}</span>
      <span class="lib-text"><b>${esc(p.title)}</b><small>${esc(p.author || t('lib.playlist'))}</small></span></a>`).join('') : '';
  $('#account-btn').textContent = state.settings.logged_in ? (state.settings.account_name || t('top.account')) : t('top.login');
}

// ---------------------------------------------------------------- right panel
let rpTab = 'queue';
function openPanel(tab) {
  const rp = $('#rightpanel');
  if (!rp.classList.contains('hidden') && rpTab === tab) { rp.classList.add('hidden'); syncPanelBtns(); return; }
  rpTab = tab;
  rp.classList.remove('hidden');
  $$('.rp-tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === tab));
  renderPanel();
  syncPanelBtns();
}
function syncPanelBtns() {
  const open = !$('#rightpanel').classList.contains('hidden');
  $('#btn-queue').classList.toggle('on', open && rpTab === 'queue');
  $('#btn-lyrics').classList.toggle('on', open && rpTab === 'lyrics');
}

async function renderPanel() {
  if ($('#rightpanel').classList.contains('hidden')) return;
  const body = $('#rp-body');
  if (rpTab === 'queue') {
    const q = player.queue, i = player.index;
    const lid = registerList(q);
    const row = (x, idx) => `<div class="qrow ${idx === i ? 'playing' : ''}" data-qidx="${idx}" data-list="${lid}" data-idx="${idx}">
      <img src="${esc(x.cover || '')}" alt="" loading="lazy"><div class="ttext"><div class="ttitle">${esc(x.title)}</div>
      <div class="tartist">${esc(artistNames(x.artists))}</div></div>
      ${idx !== i ? `<button class="icon-btn" data-act="q-remove">${icon('close')}</button>` : ''}</div>`;
    const nextTitle = player.context?.title ? t('rp.nextFrom', { title: player.context.title }) : t('rp.next');
    body.innerHTML = !q.length ? `<p class="muted pad">${esc(t('rp.emptyQueue'))}</p>`
      : `<h3>${esc(t('rp.nowPlaying'))}</h3>${q[i] ? row(q[i], i) : ''}
        ${q.length > i + 1 ? `<h3>${esc(nextTitle)}</h3>${q.slice(i + 1, i + 101).map((x, k) => row(x, i + 1 + k)).join('')}` : ''}`;
  }
}

// ---------------------------------------------------------------- context menu
const menu = $('#ctxmenu');
function showMenu(x, y, items) {
  menu.innerHTML = items.map((it, idx) => it === '-' ? '<div class="sep"></div>'
    : it.sub ? `<div class="mi has-sub">${esc(it.label)}<span>${icon('chev-right')}</span><div class="submenu">${it.sub.map((s, k) =>
      `<div class="mi" data-mi="${k}" data-parent="${idx}">${esc(s.label)}</div>`).join('')}</div></div>`
    : `<div class="mi" data-mi="${idx}">${esc(it.label)}</div>`).join('');
  menu._items = items;
  menu.classList.remove('hidden');
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - r.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, innerHeight - r.height - 8))}px`;
  menu.classList.toggle('flip', x + r.width * 2 > innerWidth);
}
const hideMenu = () => menu.classList.add('hidden');
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
document.addEventListener('mousedown', (e) => { if (!menu.contains(e.target)) hideMenu(); });
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
  items.push({ label: t('menu.copy'), run: () => { navigator.clipboard.writeText(`https://music.youtube.com/watch?v=${x.id}`); toast(t('toast.copied')); } });
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
    $$(`.trow[data-tid="${CSS.escape(x.id)}"] .like`).forEach((b) => { b.classList.toggle('on', liked); b.innerHTML = icon(liked ? 'heart-fill' : 'heart'); });
    renderSidebar();
    updateNowPlaying();
    toast(t(liked ? 'toast.liked' : 'toast.unliked'));
  } catch (e) { toast(errText(e), 'err'); }
}

async function downloadTracks(tracks) {
  const todo = tracks.filter((x) => !state.downloaded.has(x.id));
  if (!todo.length) return toast(t('toast.alreadyDl'));
  for (const x of todo) state.dlProgress.set(x.id, { state: 'queued', progress: 0 });
  try { await api.download(todo); toast(t('toast.dlStarted', { tracks: tracksLabel(todo.length) })); } catch (e) { toast(errText(e), 'err'); }
}

async function deleteDownload(x) {
  await api.deleteDownload(x.id);
  state.downloaded.delete(x.id);
  renderSidebar();
  if (nav.stack[nav.pos].view === 'downloads') render();
  toast(t('toast.dlRemoved'));
}

async function startRadio(x) {
  toast(t('toast.radio'));
  try {
    const tracks = await api.radio(x.id);
    const list = [x, ...tracks.filter((y) => y.id !== x.id)];
    player.playList(list, 0, { kind: 'radio', title: t('radio.title', { title: x.title }) });
  } catch (e) { toast(errText(e), 'err'); }
}

function promptDialog(title, value = '') {
  return new Promise((resolve) => {
    const root = $('#modal-root');
    root.innerHTML = `<div class="modal-bg"><div class="modal"><h2>${esc(title)}</h2>
      <input class="text-in" id="modal-in" value="${esc(value)}" spellcheck="false">
      <div class="row end"><button class="pill-btn" id="m-cancel">${esc(t('dlg.cancel'))}</button><button class="pill-btn primary" id="m-ok">${esc(t('dlg.save'))}</button></div></div></div>`;
    const inp = $('#modal-in');
    inp.focus(); inp.select();
    const done = (v) => { root.innerHTML = ''; resolve(v); };
    $('#m-ok').onclick = () => done(inp.value.trim() || null);
    $('#m-cancel').onclick = () => done(null);
    $('.modal-bg').onclick = (e) => { if (e.target.classList.contains('modal-bg')) done(null); };
    inp.onkeydown = (e) => { if (e.key === 'Enter') done(inp.value.trim() || null); if (e.key === 'Escape') done(null); };
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

const actions = {
  like: (el) => { const r = trackFromEl(el); if (r) toggleLike(r.track); },
  menu: (el, e) => { const r = trackFromEl(el); if (r) trackMenu(r.track, r.ctx, r.idx, e.clientX, e.clientY); },
  'play-collection': (el) => playCollection(el.dataset.kind, el.dataset.id),
  'play-page': () => pageTracks.length && player.playList(pageTracks, 0, pageCtx),
  'shuffle-page': () => {
    if (!pageTracks.length) return;
    if (!player.shuffle) player.toggleShuffle();
    player.playList(pageTracks, Math.floor(Math.random() * pageTracks.length), pageCtx);
  },
  'download-page': () => downloadTracks(pageTracks),
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
    if (!confirm(t('dlg.delete', { name: pageCtx.title }))) return;
    await api.deletePlaylist(pageCtx.id);
    await refreshLibrary();
    go('home');
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
  'install-update': () => installUpdate(),
  reload: () => location.reload(),
  'google-login': async (el) => {
    el.disabled = true;
    el.querySelector('span').textContent = t('set.googleWaiting');
    try { await api.googleLogin(); } catch (e) { toast(errText(e), 'err'); render(); }
  },
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
      renderSidebar();
      toast(t('toast.loggedIn'));
      await sync();
      render();
    } catch (e) { toast(errText(e), 'err'); } finally { el.disabled = false; }
  },
  logout: async () => { await api.logout(); state.settings = await api.settings(); state.remotePlaylists = []; renderSidebar(); render(); },
  sync: async (el) => { el.disabled = true; await sync(); el.disabled = false; },
  'install-ytdlp': async (el) => {
    el.disabled = true; el.textContent = t('set.downloading');
    try { await api.installYtdlp(); toast(t('toast.ytdlpOk')); } catch (e) { toast(errText(e), 'err'); }
    render();
  },
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
  if (img.dataset.onerr === 'remove') img.remove();
  else if (img.dataset.onerr === 'hide') img.style.visibility = 'hidden';
}, true);
document.addEventListener('click', (e) => {
  const act = e.target.closest('[data-act]');
  if (act) { e.stopPropagation(); actions[act.dataset.act]?.(act, e); return; }
  const num = e.target.closest('.trow .tnum');
  if (num) {
    const r = trackFromEl(num);
    if (r) { if (player.current?.id === r.track.id) player.toggle(); else player.playList(r.list, r.idx, r.ctx); }
    return;
  }
  const goEl = e.target.closest('[data-go]');
  if (goEl) { const id = goEl.dataset.id; go(goEl.dataset.go, id ? { id } : {}); return; }
  const navEl = e.target.closest('[data-nav]');
  if (navEl) {
    const v = navEl.dataset.nav;
    if (v === 'local-playlist') go(v, { id: navEl.dataset.pl });
    else if (v === 'playlist') go(v, { id: navEl.dataset.rpl });
    else if (v === 'search') { $('#search-input').focus(); go('search', { q: '' }); }
    else go(v);
    return;
  }
  const chip = e.target.closest('[data-search]');
  if (chip) { doSearch(chip.dataset.search); return; }
  const q = e.target.closest('.qrow');
  if (q) player.jumpTo(Number(q.dataset.qidx));
});

document.addEventListener('dblclick', (e) => {
  const row = e.target.closest('.trow');
  if (!row || e.target.closest('button, a')) return;
  const r = trackFromEl(row);
  if (r) player.playList(r.list, r.idx, r.ctx);
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

$('#nav-back').onclick = () => { if (nav.pos > 0) { nav.pos--; render(); } };
$('#nav-fwd').onclick = () => { if (nav.pos < nav.stack.length - 1) { nav.pos++; render(); } };
$('#new-playlist').onclick = () => newPlaylist();
$('#rp-close').onclick = () => { $('#rightpanel').classList.add('hidden'); syncPanelBtns(); };
// Lyrics open as a full Spotify-style screen; pressing the button again goes back.
function toggleLyrics() {
  if (nav.stack[nav.pos]?.view === 'lyrics') $('#nav-back').click();
  else go('lyrics');
}
$$('.rp-tab').forEach((b) => (b.onclick = () => (b.dataset.tab === 'lyrics' ? toggleLyrics() : openPanel(b.dataset.tab))));
$('#btn-queue').onclick = () => openPanel('queue');
$('#btn-lyrics').onclick = toggleLyrics;
document.addEventListener('click', (e) => {
  const line = e.target.closest('.ly-line');
  if (line) { player.seekTo(Number(line.dataset.t) / 1000); ly.userScrollAt = 0; }
});
// Manual scrolling pauses auto-follow for a few seconds, like Spotify.
['wheel', 'touchmove'].forEach((ev) => $('#main').addEventListener(ev, () => { ly.userScrollAt = Date.now(); }, { passive: true }));

// ---------------------------------------------------------------- search box
const sInput = $('#search-input');
const suggest = $('#suggest');
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
  if (cur?.view === 'search') { nav.stack[nav.pos] = { view: 'search', params: { q }, scroll: 0 }; render(); }
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
  else if (e.key === 'Escape') { suggest.classList.add('hidden'); sInput.blur(); }
});
sInput.addEventListener('blur', () => setTimeout(() => suggest.classList.add('hidden'), 150));
sInput.addEventListener('focus', () => { if (sInput.value.trim()) loadSuggest(sInput.value.trim()); });

// ---------------------------------------------------------------- player bar
const audio = player.audio;
function updateNowPlaying() {
  const cur = player.current;
  $('#player').classList.toggle('idle', !cur);
  if (!cur) {
    // Idle player stays visible (like Spotify) with a placeholder.
    $('#np-cover').src = COVER_PLACEHOLDER;
    $('#np-title').textContent = t('pl.idle');
    $('#np-artist').textContent = t('pl.idleSub');
    $('#np-quality').textContent = '';
    return;
  }
  $('#np-cover').src = cur.cover || '';
  $('#np-title').innerHTML = cur.album?.id ? `<a class="link" data-go="album" data-id="${esc(cur.album.id)}">${esc(cur.title)}</a>` : esc(cur.title);
  $('#np-artist').innerHTML = artistLinks(cur.artists);
  const liked = state.liked.has(cur.id);
  $('#np-like').classList.toggle('on', liked);
  setIcon($('#np-like'), liked ? 'heart-fill' : 'heart');
  document.title = `${cur.title} · ${artistNames(cur.artists)}`;
  // Quality badge (codec · kbps, lock icon when playing the encrypted offline copy).
  const badge = $('#np-quality');
  badge.textContent = '';
  badge.className = 'qpill';
  api.streamQuality(cur.id).then((q) => {
    if (player.current?.id !== cur.id) return;
    badge.innerHTML = `${q.offline ? icon('lock') : ''}${esc(qualityLabel(q.codec, q.kbps))}`;
    badge.title = q.offline ? t('quality.offline') : t('quality.stream');
    badge.classList.toggle('offline', q.offline);
  }).catch(() => {});
}
function updateState() {
  setIcon($('#btn-play'), player.playing ? 'pause' : 'play');
  $('#btn-shuffle').classList.toggle('on', player.shuffle);
  $('#btn-repeat').classList.toggle('on', player.repeat !== 'off');
  setIcon($('#btn-repeat'), player.repeat === 'one' ? 'repeat-one' : 'repeat');
  const cur = player.current?.id;
  $$('.trow').forEach((r) => {
    r.classList.toggle('playing', r.dataset.tid === cur);
    r.classList.toggle('paused', r.dataset.tid === cur && !player.playing);
  });
  if ('mediaSession' in navigator) navigator.mediaSession.playbackState = player.playing ? 'playing' : 'paused';
}
let dragging = null;
function updateTime() {
  const d = audio.duration, c = audio.currentTime;
  $('#t-dur').textContent = fmt(isFinite(d) ? d : player.current?.duration);
  if (dragging === 'seek') return;
  $('#t-cur').textContent = fmt(c);
  const f = isFinite(d) && d > 0 ? c / d : 0;
  $('#seek-fill').style.width = `${f * 100}%`;
  $('#seek-knob').style.left = `${f * 100}%`;
  if (audio.buffered.length && isFinite(d)) $('#seek-buf').style.width = `${(audio.buffered.end(audio.buffered.length - 1) / d) * 100}%`;
  if ('mediaSession' in navigator && isFinite(d) && d > 0) {
    try { navigator.mediaSession.setPositionState({ duration: d, position: Math.min(c, d), playbackRate: 1 }); } catch { /* ignore */ }
  }
}
function updateVolume() {
  const v = audio.muted ? 0 : audio.volume;
  $('#vol-fill').style.width = `${v * 100}%`;
  $('#vol-knob').style.left = `${v * 100}%`;
  setIcon($('#btn-mute'), v === 0 ? 'volume-off' : 'volume');
}

function bindBar(el, kind, onMove, onEnd) {
  const frac = (e) => { const r = el.getBoundingClientRect(); return Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)); };
  el.addEventListener('pointerdown', (e) => {
    dragging = kind;
    el.setPointerCapture(e.pointerId);
    el.classList.add('drag');
    onMove(frac(e));
    const move = (ev) => onMove(frac(ev));
    const up = (ev) => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
      el.classList.remove('drag');
      dragging = null;
      onEnd?.(frac(ev));
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
  });
}
bindBar($('#seekbar'), 'seek', (f) => {
  $('#seek-fill').style.width = `${f * 100}%`;
  $('#seek-knob').style.left = `${f * 100}%`;
  if (isFinite(audio.duration)) $('#t-cur').textContent = fmt(f * audio.duration);
}, (f) => player.seek(f));
bindBar($('#volbar'), 'vol', (f) => player.setVolume(f));
$('#volbar').addEventListener('wheel', (e) => { e.preventDefault(); player.setVolume(audio.volume - Math.sign(e.deltaY) * 0.05); }, { passive: false });

$('#btn-play').onclick = () => player.toggle();
$('#btn-next').onclick = () => player.next();
$('#btn-prev').onclick = () => player.prev();
$('#btn-shuffle').onclick = () => player.toggleShuffle();
$('#btn-repeat').onclick = () => player.cycleRepeat();
$('#btn-mute').onclick = () => player.toggleMute();
$('#np-like').onclick = () => player.current && toggleLike(player.current);
$('#np-cover').onclick = () => openPanel('queue');

player.on('track', () => {
  updateNowPlaying();
  renderPanel();
  if (nav.stack[nav.pos]?.view === 'lyrics') render();
});
player.on('queue', () => { if (rpTab === 'queue') renderPanel(); });
player.on('state', updateState);
player.on('time', () => { updateTime(); syncLyrics(); });
player.on('volume', updateVolume);
player.on('error', (x) => toast(t('toast.playFail', { title: x.title }), 'err'));

// keyboard shortcuts
document.addEventListener('keydown', (e) => {
  if (e.target.closest('input, textarea, select')) return;
  if (e.code === 'Space') { e.preventDefault(); player.toggle(); }
  else if (e.code === 'ArrowRight' && e.ctrlKey) player.next();
  else if (e.code === 'ArrowLeft' && e.ctrlKey) player.prev();
  else if (e.altKey && e.key === 'ArrowLeft') $('#nav-back').click();
  else if (e.code === 'ArrowRight') audio.currentTime = Math.min(audio.duration || 0, audio.currentTime + 5);
  else if (e.code === 'ArrowLeft') audio.currentTime = Math.max(0, audio.currentTime - 5);
  else if (e.code === 'ArrowUp' && e.ctrlKey) player.setVolume(audio.volume + 0.05);
  else if (e.code === 'ArrowDown' && e.ctrlKey) player.setVolume(audio.volume - 0.05);
  else if ((e.key === 'k' && e.ctrlKey) || e.key === '/') { e.preventDefault(); sInput.focus(); sInput.select(); }
  else if (e.key === 'l' && e.altKey) { if (player.current) toggleLike(player.current); }
});
window.addEventListener('mouseup', (e) => { if (e.button === 3) $('#nav-back').click(); if (e.button === 4) $('#nav-fwd').click(); });

// ---------------------------------------------------------------- backend events
listen('download-progress', (p) => {
  state.dlProgress.set(p.id, p);
  if (p.state === 'done') {
    state.downloaded.add(p.id);
    state.dlProgress.delete(p.id);
    renderSidebar();
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
  showMenu(r.left, r.top - 8 - items.length * 36, items);
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

// "Add to playlist" from the player bar.
$('#np-add').onclick = (e) => {
  const cur = player.current;
  if (!cur) return;
  const r = e.currentTarget.getBoundingClientRect();
  const items = [{ label: t('menu.newPlaylist'), run: () => newPlaylist([cur]) },
    ...state.playlists.map((p) => ({ label: p.name, run: () => addTo(p, [cur]) }))];
  showMenu(r.left, r.top - 8 - Math.min(items.length, 8) * 36, items);
};

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
    renderSidebar();
    await sync();
  } else {
    toast(t('toast.googleFail', { e: r.error || '' }), 'err');
  }
  if (nav.stack[nav.pos]?.view === 'settings') render();
});
listen('google-login-closed', () => { if (nav.stack[nav.pos]?.view === 'settings') render(); });

// ---------------------------------------------------------------- boot
(async function boot() {
  translateDom();
  hydrateIcons();
  await initApi();
  state.settings = await api.settings().catch(() => ({}));
  await refreshLibrary().catch((e) => console.error(e));
  refreshRemote();
  await player.restore().catch((e) => console.error(e));
  updateNowPlaying();
  go('home');
  api.sleepState().then(showSleep).catch(() => {});
  // Quietly look for a new version a few seconds after start.
  setTimeout(() => checkForUpdate(false), 4000);
})();
