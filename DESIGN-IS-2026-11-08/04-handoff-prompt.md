````
/make-plan Redesign the YT Music app UI (desktop Tauri/WebView2 + Android WebView, static frontend in ui/), GitHub issue #7. Current design failed audit at 11/30 with critical gaps in principles #1 innovative, #3 aesthetic, #4 understandable, #8 thorough (all scored 1/3).

Verdict paragraph (quoted from 03-verdict.md):
> The current UI is the standard music-player layout in a glass skin, scoring 11/30 against Rams's ten principles, so it should be redesigned from purpose while keeping the PR #1 glass tokens.

Why redesign and not refine: total 11/30 is below the 20 threshold. The owner also requires that it look like neither YouTube Music, Spotify nor Apple Music, and the borrowed part is the layout itself (left library sidebar + top search bar + bottom player bar + right queue/lyrics panel + phone tab bar, ui/index.html:37-148).

Primary user: a daily listener on a Windows PC and an Android phone, in Ukrainian, Russian or English, usually signed in to YouTube.
Primary task: find a song, album, artist or playlist, start it in as few steps as possible, and control playback (queue, lyrics, like, offline).
Constraints:
- Vanilla JS with no build step. Rust owns the logic; JS only renders.
- Must look unlike YouTube Music, Spotify and Apple Music.
- Name "YT Music"; never reuse the YouTube Music logo.
- uk/ru/en i18n.
- WCAG AA contrast for body text.
- Must run smoothly in Android WebView.

Preserve from current design:
- PR #1 glass theme tokens, ui/theme-glass.css:6-41. They mirror the Figma file "YTM Tokens":
  - colours: --bg #050507, --surface rgba(255,255,255,.04), --stroke rgba(255,255,255,.09), --text #f5f5f7, --muted #9a9aa6, --accent #3ddc84, --glow-warm #ff8a3d, --glow-cool #5b8cff, --danger #ff5c6c
  - radii: --r-xs 8, --r-sm 10, --r-md 14, --r-lg 20, --radius 24
  - --blur: blur(32px) saturate(140%)
  - fonts: Geist / Geist Mono (ui/fonts)
- The classic theme (ui/styles.css:1-22) stays selectable as the second theme.
- Keyboard shortcuts: Space, Ctrl+→/←, ←/→ seek, Ctrl+↑/↓ volume, Alt+L, / and Ctrl+K (ui/app.js:1069-1082), plus OS media hotkeys.
- 1-click play affordances on home: Play liked, Start radio, card play (ui/app.js:293-294, :182).
- Full uk/ru/en string set (ui/i18n.js, 231 keys).
- The cover-colour page tint (--page-color, ui/app.js:115/223).

Discard:
- The three-column shell plus right side panel. Evidence: ui/index.html:37-148. Caused the failure on #1.
- The account button routing to Settings. Evidence: ui/index.html:96 → ui/app.js:863-869. Caused the failure on #4.
- Blur on every content card plus the 4-filter glow SVG plus eq-bar neon. Evidence: ui/theme-glass.css:94-324, ui/index.html:13-35. Caused the failures on #5, #7 and #9.
- Duplicated elements: brand blocks (ui/index.html:39-53), greeting (ui/app.js:286-289), library list built twice, the dead Lyrics panel tab (ui/app.js:593, :905). Caused the failure on #10.
- Untokenised literals: about 48 colours, about 72 radii, about 25 font sizes. Evidence: ui/theme-glass.css throughout. Caused the failure on #3.
- Play only from the tiny .tnum cell (ui/app.js:855); div seek/volume bars (ui/index.html:131, :141); <a> elements without href for nav. Caused the failures on #2 and #8.

Top 5 moves from the audit (verbatim):
1. #1 innovative / #2 useful: new information architecture, not the three-column shell.
   - Develop 2–3 distinct concepts in Figma and let the owner pick:
     - (a) "Stage": what's playing is the home screen, and library/search/recommendations slide in as layers over it.
     - (b) "Command": one universal command/search bar (Ctrl+K, "/" on phone) does search, actions and navigation, and the library is a single filterable list instead of a sidebar.
     - (c) "Timeline": queue, history and next-up form one horizontal timeline docked to the player.
   - Every concept must start a song in at most 1 click/tap from home and at most 1 click after search results.
   - A single click anywhere on a track row must play it (fixes ui/app.js:855).
2. #4 understandable: every label says what it does.
   - The account button opens an account menu (issue #5).
   - Toggles carry state (aria-pressed, plus tooltips like "Pause" or "Repeat: one").
   - Plain words instead of jargon: yt-dlp → "Backup playback helper"; cookies → "Sign in with a browser file (advanced)"; Opus/AAC/kbps → "High" / "Standard".
   - Evidence: ui/app.js:863-869, :988-1017; ui/i18n.js:55-88.
3. #3 aesthetic / #10 as little design as possible: enforce the token system and keep one copy of everything.
   - At most 7 font sizes, a 4-px spacing scale as tokens, radii only from --r-*, colours only from :root.
   - Remove the duplicate brand block and greeting, the dead Lyrics panel tab and the double-built library.
   - Evidence: ui/index.html:39-53; ui/app.js:286-289, :593.
4. #8 thorough: design every state.
   - Focus-visible ring in the accent colour, skeleton loaders, empty states with one action.
   - Toasts as an aria-live region, real role=slider for seek and volume.
   - Focus moves into Now Playing and into menus.
5. #5 unobtrusive / #7 long-lasting / #9 environmentally friendly: keep glass but budget it.
   - Blur only on persistent chrome (player, navigation, overlays; at most 3 layers). Content cards sit on a flat --surface.
   - Drop the eq neon and the "powered by rust" wordmark.
   - Gate all motion behind reduced-motion, and fix the idle eq animation (ui/app.js:150 vs :994).

Redesign principles in priority order:
1. #2 Useful: play in at most 1 click from home or from search results; queue, lyrics and like are reachable in 1 step from anywhere.
2. #4 Understandable: a first-time user names every primary control correctly; no technical jargon in the UI.
3. #1 Innovative: the layout is recognisably its own, not a recolour of the sidebar + player-bar pattern, and still behaves conventionally where conventions help (media keys, Space, seek).
4. #10 As little design as possible: one entry point per action, one brand mark, no duplicated lists.

Feature coverage to design (on par with mainstream players, own interaction design):
- home and recommendations; search with suggestions
- album, artist and playlist pages
- library (liked, playlists, downloads, history); create and edit playlists; add to playlist
- queue with drag & drop; track radio
- lyrics (synced); full-screen Now Playing; mini player
- sleep timer, crossfade, volume normalisation, equalizer (issue #6)
- account menu (issue #5); settings; offline downloads; update flow; sign-in and onboarding

Deliverables for the plan:
- Phase 0: read the existing Figma "YTM Tokens" file (owner to provide the link) via the Figma MCP get_variable_defs / get_design_context, and read the Figma MCP skill (/figma-use) before any use_figma call.
- New information architecture as a screen map for desktop and phone, not derived from the old one.
- 2–3 concept directions, built in Figma as low-fi frames, compared side by side with the current layout; the owner picks one.
- Design system page in Figma: tokens bound as variables (preserved glass values), type scale ≤7, spacing scale, components (track row, card, button, slider, menu, sheet, toast, player).
- Hi-fi screens for desktop (1280) and phone (390), plus all states: empty, loading, error, success, focus, disabled.
- Honesty pass on every string: drop "Listen to Everything", "fast", "like Spotify" and "best quality" (ui/i18n.js:32-33, :60, :87).
- Migration path: build the new shell behind a third theme/layout option, ship screen by screen with one PR each, keep classic and glass themes selectable.
- Cutover criteria: the new layout becomes the default once every route in ui/app.js:266-526 has a new screen and the issue #5/#6 features are in. The old shell is then removed (no permanent flag).

Anti-patterns to guard against (specific to REDESIGN):
- Porting old structure under new styling (the sidebar + bottom bar + right panel must not come back renamed).
- Keeping both layouts behind a flag indefinitely.
- Redesigning to follow a trend rather than the principles above (no new glow, neon or blur-everything).
- Treating the Preserve list as optional: the PR #1 glass tokens are a hard constraint from the owner.
- Copying signature elements of YouTube Music, Spotify or Apple Music (e.g. a green circular play FAB on cards, Apple's large-title library, YouTube Music's chip rows).
````
