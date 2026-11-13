# 04 — Готовий запит для /make-plan

````
/make-plan Redesign the YT Music app interface (ui/ in neverh8me/yt-music: index.html, styles.css, app.js, player.js, i18n.js). Current design failed a Dieter Rams audit at 13/30 with the weakest scores (1/3) in principles #4 understandable, #6 honest, #3 aesthetic, #10 as little design as possible, #7 long-lasting, #9 environmentally friendly, #1 innovative.

Verdict paragraph (quoted from 03-verdict.md):
> REDESIGN. The total of 13 out of 30 is below the threshold of 20, so by the audit rule this is a rework from purpose, not polishing — although no principle scored zero and the base (tokens, the "player + layers" model) is worth keeping.

Why redesign and not refine: the total is below the 20-point threshold; no load-bearing principle scored 0.

Context: Tauri 2 app, vanilla-JS UI with no build step, the same files run on Windows, macOS, Linux, Android and iPhone. Primary user: a person listening to YouTube Music. Primary task: find a song and play it, then control playback without thinking about the app. Owner's requirement: fast and stable. Reply to the owner in Ukrainian; code, comments and commit messages in English.

Preserve from current design:
- The token system in ui/styles.css `:root`: 8 spacing steps (4–48 px), 5 type sizes (11, 12, 14, 18, 26 px), 6 radii, the colour tokens (--bg #050507, --text #f5f5f7, --muted #9a9aa6, --accent #ff2e4d), dark and light themes.
- The "stage + layers" model: the playing track is the home screen, pages open as layers over it, the capsule player collapses the stage (ui/app.js, `setMode` / `morph` / `go`).
- All existing states: skeleton loading (.sk), empty (.empty), toasts for errors and success, 8 :focus-visible rules.
- Fonts Geist and Geist Mono (ui/fonts).

Discard:
- Icon-only controls with a tooltip as the only label: 25 elements with `data-i18n-title` in ui/index.html; touch screens show no tooltips. Caused failure on principle #4.
- The header tagline "powered by Rust 🦀" (ui/index.html:21). Caused failure on principles #10 and #5.
- The always-running ambient background animation `#ambient { animation: drift 18s ... infinite }` (ui/styles.css:580). Caused failure on principles #9, #10 and #7.
- Type sizes outside the scale: 28, 22, 18, 16, 15, 11 px and clamp(44px, 7vw, 88px) in ui/styles.css; 26 distinct hex and 32 distinct rgb/rgba literals against ~18 colour tokens. Caused failure on principle #3.
- Unmeasured speed claims: "Fast unofficial YouTube Music client" (src-tauri/tauri.conf.json shortDescription) and "Songs start faster…" (ui/i18n.js, key eng.hint), while issue #15 "tracks start slowly" is open. Caused failure on principle #6.
- Technical blocks in the main settings: yt-dlp status and the playback-engine switch among 8 `set-block` sections (ui/app.js). Caused failure on principle #10.

Top 5 moves from the audit (verbatim):
1. #4 understandable: give text or a clear menu to the 25 icon-only buttons, first of all on phones: sleep timer, equalizer, "add to playlist", "queue". Evidence: 25 `data-i18n-title` elements in ui/index.html.
2. #10 as little design as possible: remove the "powered by Rust 🦀" tagline (ui/index.html:21) and the background drift (ui/styles.css:580); move the yt-dlp block and the engine switch into "Advanced". Evidence: 8 settings sections in ui/app.js.
3. #3 aesthetic: fold the 7 orphan type sizes and the colours outside tokens into the existing system. Evidence: font-size literals and 58 colour literals in ui/styles.css.
4. #6 honest: remove "Fast" and "start faster" until start time is measured; after measuring, state the number. Evidence: src-tauri/tauri.conf.json, ui/i18n.js eng.hint, issue #15.
5. #9 environmentally friendly and #7 long-lasting: tie motion to the system "reduce motion" setting (ui/styles.css has no prefers-reduced-motion query; ui/app.js:46 uses its own data-motion flag), cut the 10 backdrop-filter uses down to those that carry meaning, and measure the memory of the hidden music.youtube.com window on desktop.

Redesign principles in priority order:
1. #4 understandable — a first-time user names every primary control correctly on a phone, with no tooltips.
2. #10 as little design as possible — removing any remaining element breaks a task; nothing decorative moves on an idle screen.
3. #6 honest — every label and claim matches measured behaviour; the play button, volume and "Downloaded" always reflect the real state.

Deliverables for the plan:
- New information architecture (not derived from old)
- New primary flow (low-fi, labeled, compared side-by-side to current)
- States checklist (empty, loading, error, success, focus, disabled)
- Migration path for users currently on the old design
- Cutover criteria (when is the old design retired)

Anti-patterns to guard against (specific to REDESIGN):
- Porting old structure under new styling
- Keeping both designs behind a flag indefinitely
- Redesigning to follow a trend rather than the principles above
- Treating the Preserve list as optional — it must be filled before this handoff is valid
````
