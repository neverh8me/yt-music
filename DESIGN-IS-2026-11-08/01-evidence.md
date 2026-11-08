# Evidence (static source audit, v0.3.8, 2026-11-08)

Four subagents collected this evidence: Structural + Accessibility, Visual, Copy & Honesty, and Weight & Friction. Paths are relative to `ui/`. Visual numbers are INFERRED from source; nothing was rendered.

## E1 Structure
- **Shell layout:**
  - Sidebar with library on the left (index.html:37-80) and top bar with back/forward, search and account (:84-96).
  - Bottom player bar (:111-141) and a right panel for queue/lyrics (:102-106).
  - Bottom tab bar on phones (:144-148) and a full-screen Now Playing sheet (:153-182).
- **Controls and routes:** 46 static interactive controls; 12 routes (app.js:266-526).
- **Duplicated affordances:**
  - 3 ways to open Settings (index.html:80, :96, :148).
  - 2 brand blocks, with CSS hiding one (index.html:39-53).
  - 2 home greetings rendered (app.js:286-289).
  - The library list is built twice (index.html:64-77 + app.js:558-566 vs app.js:431-442).
  - New playlist ×4, queue ×3, lyrics ×3.
- **Dead code:**
  - `player.playTrack` (player.js:63).
  - Icon `check` (icons.js:32).
  - `.lyrics` CSS rule and `.glass` utility class.
  - Unreachable `rpTab==='lyrics'` branch (app.js:593). The Lyrics tab in the right panel never shows a panel (app.js:599, :905).
- **Starting a song:**
  - From home: 1 click (hero "Play liked", card play overlay, `.tnum`).
  - From search: 2 clicks + typing.
  - Single-clicking a track row's title does nothing. Only the small `.tnum` cell or a double-click plays (app.js:855, :878).
- **Phone layout:** shuffle/prev/repeat, the extra controls, account and update are all hidden (styles.css:345, :355, :363).

## E2 Accessibility
- 0 `aria-label` (titles only); 0 `aria-pressed` on toggles; no `aria-live` on toasts (index.html:189); no skip link.
- The seek and volume bars are `<div>`s with no slider role (index.html:131, :141).
- These are `<a>` with no href and can't be reached by keyboard: sidebar and tab bar links, context-menu items (`div.mi`, app.js:616), queue rows, and track play cells.
- Focus:
  - No `:focus-visible` rule in either theme.
  - `outline:none` on inputs (styles.css:121, :292, :311; theme-glass.css:165, :367, :389).
  - Opening Now Playing doesn't move focus into it (app.js:1103-1110).
- The global Space key toggles playback even while focus is on another button (app.js:1069-1071).

## E3 Visual (INFERRED)
- **Tokens vs literals:**
  - Color literals outside the token set: ≈47 (classic), ≈48 (glass).
  - Radius: 7 token uses vs ≈72 literals (glass); 18 token uses vs 39 literals (classic).
  - Glass has a single spacing token, `--gap` (theme-glass.css:36); classic has none.
  - Font sizes: 23 (classic) and ≈25 (glass), including half-pixel steps 10.5/11.5/12.5/13.5.
- **Contrast:**
  - `--dim` is under 4.5:1 on every surface; worst is 2.60 on glass `--active`.
  - White on the classic accent `#ff4d5e` is 3.24; white on the classic toast `#3a7bff` is 3.85.
- **States:**

  | State | Status |
  |---|---|
  | Empty | Bare muted text, no action (app.js:388, 398, 414, 449, 607) |
  | Loading | Spinner only, no skeletons |
  | Error | Full-page view + toast |
  | Success | Toasts |
  | Disabled | Present |
  | Focus | **Missing** |

- **Theme and motion:**
  - Dark-only, with no `prefers-color-scheme`.
  - Reduced-motion covers only `.ly-line` and `.np-sheet`, not the spinner, eq bars or toasts.

## E4 Copy and honesty
- **Inflated claims:**
  - "Listen to Everything" (i18n.js:32).
  - "one fast, ad-free player" (i18n.js:33).
  - "like Spotify" (i18n.js:60).
  - "best quality" (i18n.js:87).
- **Dark patterns:** none.
- **Jargon shown to users:**
  - yt-dlp: `set.ytdlp*` (i18n.js:63, 75), plus a raw file path (app.js:529).
  - cookies, Netscape format, `Cookie:` header (i18n.js:55-59, 86).
  - Opus/AAC/kbps in the quality badge (app.js:39).
  - "powered by Rust 🦀" (index.html:42-51).
- **Labels that don't match behavior:**
  - The account button opens the full Settings page (index.html:96 → app.js:863-869).
  - The Play tooltip says "Play" while music is playing (app.js:988).
  - The Mute tooltip doesn't change once muted (app.js:1017).
  - The Repeat mode is shown only by its icon (app.js:990).
  - The Like title stays "Save to Liked Songs" when the track is already liked.
- **Untranslated strings:**
  - Raw backend errors go straight into toasts (app.js:44 errText, used in 8+ places).
  - The "YouTube Music" divider label is hardcoded (app.js:441, :563).
- **Translations:** all 231 keys exist in uk/ru/en.

## E5 Weight and friction
- **JS:** 122 KB raw, with no build step. CSS: 37 KB (classic) or 46 KB (glass). Glass also loads 141 KB of Geist fonts.
- **Startup:** 12-15 IPC invokes. 4 of them run back-to-back before home renders (app.js:1317-1321), and the main view stays a spinner until the YouTube Music home request returns.
- **Glass rendering cost:**
  - Likely 15-40+ blurred layers at once on the home screen (every `.card`, `.quick-item` and `.tracklist` is blurred: theme-glass.css:94-324).
  - A full-screen SVG glow behind them, with 4 Gaussian-blur filters (index.html:13-35).
  - `prefers-reduced-transparency` doesn't remove the blur.
- **Probable idle bug:** the eq bars on a restored current track keep animating while paused, because render never sets `.paused` (app.js:150 vs :994).
- **Idle timers:** none, and no requestAnimationFrame loop.
