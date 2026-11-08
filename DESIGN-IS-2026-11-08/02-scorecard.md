# Scorecard (Dieter Rams, 0–3 each; ties go to the lower score; the worst instance counts)

1. Good design is innovative: **1/3**
   - Evidence: E1 shell = left library sidebar, top search bar, bottom player bar, right queue/lyrics panel, phone tab bar. The copy even benchmarks against Spotify (E4).
   - Why this score: it is the mainstream music-player layout with a glass skin. It isn't a wholesale copy of one flow, so not 0. It isn't a refreshed pattern either, so not 2.

2. Good design makes a product useful: **2/3**
   - Evidence (E1): 1-click play from home; search takes 2 clicks plus typing. A single click on a row title does nothing. The account button detours through Settings.
   - Why this score: the primary task completes in a few steps, but adjacent surfaces add detours. It isn't blocked, so not 1.

3. Good design is aesthetic: **1/3**
   - Evidence (E3): about 48 untokenised colours, about 72 literal radii against 7 token uses, about 25 font sizes including half-pixel steps.
   - Why this score: a token system exists but most rules bypass it. Far more than 2 inconsistencies rules out 2. The system is still visible, so not 0.

4. Good design makes a product understandable: **1/3**
   - Evidence (E4): the account button opens Settings; Play, Mute, Repeat and Like tooltips don't reflect state; yt-dlp, cookies and Opus/kbps jargon is visible to users.
   - Why this score: 2–3 controls are unclear and jargon is present. The primary action (play) is still identifiable, so not 0.

5. Good design is unobtrusive: **1/3**
   - Evidence (E5, E1): in glass, every card, quick item and tracklist is blurred over a 4-filter glow, and the eq bars have their own glow. There are two brand blocks and two greetings, plus "powered by Rust 🦀".
   - Why this score: in the worst theme (glass), decoration competes with the cover art. Content is still the figure, so not 0.

6. Good design is honest: **1/3**
   - Evidence (E4): "Listen to Everything", "fast", "like Spotify", "best quality". No dark patterns.
   - Why this score: 2 or more inflated claims. No deceptive flow, so not 0.

7. Good design is long-lasting: **1/3**
   - Evidence (E3, E5):
     - glassmorphism blur on every surface
     - neon glow on the eq bars and ambient arcs
     - the "// powered by rust" dev-terminal wordmark
   - Why this score: 3 trend markers that date the UI to 2021–2026. The base type (Geist) and the dark palette are timeless, so not 0.

8. Good design is thorough down to the last detail: **1/3**
   - Evidence (E2, E3):
     - no focus style at all
     - loading is a spinner only, with no skeletons
     - empty states are bare text with no action
     - toasts have no aria-live
     - seek and volume aren't sliders
   - Why this score: 1 state is missing and 2 are rough, which is more than one rough state, so not 2. Disabled, error and success exist, so not 0.

9. Good design is environmentally friendly: **1/3**
   - Evidence (E5, E3):
     - 122 KB JS (under 500 KB)
     - spinner and eq animation aren't gated by reduced motion
     - a probable idle eq animation while paused
     - 15–40 composited blur layers in glass
   - Why this score: weight is fine, but motion and GPU cost aren't gated. That rules out 2. Weight is under 2 MB with no autoplay, so not 0.

10. Good design is as little design as possible: **1/3**
    - Evidence (E1): duplicate brand block, duplicate greeting, a dead Lyrics panel tab, 3 Settings entry points, the library built twice, dead code.
    - Why this score: 3–5 removable elements. Not dominated by decoration, so not 0.

**Total: 11/30.** No principle scored 0.
