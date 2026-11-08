# Verdict: REDESIGN (11/30)

**The current UI is the standard music-player layout in a glass skin, scoring 11/30 against Rams's ten principles, so it should be redesigned from purpose while keeping the PR #1 glass tokens.**

The rule applied: a total below 20 means REDESIGN. No principle scored 0. The owner's constraint (not like YouTube Music, Spotify or Apple Music) independently rules out a refine, because the layout itself is the borrowed part.

## Top 5 moves
1. **#1 innovative / #2 useful: a new information architecture, not the three-column shell.** The current layout is sidebar + top bar + bottom player + right panel (index.html:37-148). Develop 2–3 distinct concepts in Figma and let the owner pick one. Candidate directions:
   - (a) **"Stage"**: what's playing is the home screen. Library, search and recommendations come in as layers over it.
   - (b) **"Command"**: a single universal command/search bar (Ctrl+K, and "/" on phone) handles search, actions and navigation. The library is one filterable list, not a sidebar.
   - (c) **"Timeline"**: the queue, history and next-up form one horizontal timeline docked to the player.

   Every concept must start a song in at most 1 click/tap from home and at most 1 click after search results, and a single click anywhere on a track row must play it (fixes app.js:855).
2. **#4 understandable: every label says what it does.**
   - The account button opens an account menu (issue #5).
   - Toggles carry state: aria-pressed, plus tooltips like "Pause" or "Repeat: one".
   - Jargon becomes plain words: yt-dlp → "Backup playback helper"; cookies → "Sign in with a browser file (advanced)"; Opus/AAC/kbps → "High" / "Standard".

   Evidence: app.js:863-869, :988-1017, i18n.js:55-88.
3. **#3 aesthetic / #10 as little design as possible: enforce the token system and keep one copy of everything.**
   - Limits: at most 7 font sizes, a 4-px spacing scale as tokens, radii only from `--r-*`, colours only from `:root`.
   - Remove the duplicate brand block and greeting, the dead Lyrics panel tab and the double-built library.

   Evidence: E3 counts; index.html:39-53; app.js:286-289, :593.
4. **#8 thorough: design every state.**
   - focus-visible ring in the accent colour
   - skeleton loaders
   - empty states with one action
   - toasts as an aria-live region
   - real `role=slider` for seek and volume
   - focus moves into Now Playing and menus

   Evidence: E2, E3 states table.
5. **#5 unobtrusive / #7 long-lasting / #9 environmentally friendly: keep glass but budget it.**
   - Blur only on persistent chrome: player, navigation and overlays (at most 3 layers). Content cards sit on a flat `--surface`.
   - Drop the neon glow on the eq bars and the "powered by rust" wordmark.
   - Gate all motion behind reduced-motion, and fix the idle eq animation.

   Evidence: theme-glass.css:94-324, index.html:13-35, app.js:150 vs :994.
