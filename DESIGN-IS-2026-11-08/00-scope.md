# Scope: YT Music UI audit (issue #7)

**What is audited:** the shipped UI of YT Music v0.3.8, the static frontend in `ui/`:
- `index.html`
- `app.js` (views, routing, rendering)
- `player.js`
- `styles.css` (the "classic" theme)
- `theme-glass.css` (the "glass" theme from PR #1, @dopedasya)
- `i18n.js`
- `icons.js`

Both surfaces are covered: desktop (Tauri/WebView2) and phone (Android WebView, phone CSS).

**How:** a static source audit. I didn't start the app: the owner games on this PC, so no windows or audio without asking. Visual facts are inferred from the CSS and marked INFERRED.

**Primary user:** someone who listens to music every day on a PC and an Android phone, in Ukrainian, Russian or English, often with a YouTube account signed in.

**Primary task:** find a song, album, artist or playlist, start it in as few steps as possible, and control playback (queue, lyrics, likes, offline).

**Constraints from the owner (2026-11-08):**
- It must not look like YouTube Music, Spotify or Apple Music. It should be unique and very convenient.
- **The PR #1 theme stays:** the glass tokens in `ui/theme-glass.css`, which mirror the Figma file "YTM Tokens":
  - `--bg #050507`
  - `--accent #3ddc84`
  - glows `#ff8a3d` / `#5b8cff`
  - Geist and Geist Mono fonts
  - `blur(32px)` surfaces
  - radii 8/10/14/20/24
- Feature coverage at least on par with mainstream players.
- Stack: vanilla JS with no build step; Rust owns the logic and JS only renders.
- Name "YT Music"; don't reuse the YouTube Music logo.

**References (negative):** YouTube Music, Spotify and Apple Music. Avoid their layouts, signature components and brand cues.
