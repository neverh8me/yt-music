# YT Music redesign in Figma (issue #7)

**Primary goal:** The owner opens a Figma file and sees three distinct, original layout concepts for YT Music on desktop and phone. All three are built on the existing PR #1 glass tokens. The owner picks one, and that one becomes the hi-fi design and then code.

Source audit: `DESIGN-IS-2026-11-08/`. Its verdict is REDESIGN (11/30).

## Phase 0: Discovery (done)
**How this serves the primary goal:** We know which tools and limits we are working with before building anything.
- **Figma MCP account:** Starter plan, View seat. Read tools (get_screenshot, get_metadata, get_design_context …) are limited to **20 calls/month**. Write tools (create_new_file, whoami and the like) are exempt. Rule: verify with `node.screenshot()` inside `use_figma` instead of calling `get_screenshot`.
- **Skills loaded:** figma-use, figma-generate-design and figma-generate-library. Rules to follow:
  - colours in 0–1
  - load fonts before writing text
  - `createAutoLayout`
  - explicit variable scopes
  - return the IDs of everything created
  - run calls one after another, never in parallel
- **Tokens file:** I can't look up the key of the "YTM Tokens" Figma file through MCP. The code says `ui/theme-glass.css:6-41` mirrors it, so I recreate the tokens as variables from that file.

## Phase 1: File and foundations
**How this serves the primary goal:** Every concept uses the same preserved tokens.
- `create_new_file` "YT Music — Redesign" in the drafts of team::1701068436203893395.
- Variable collection "YTM Tokens" with:
  - **color:** bg, surface, surface-hover, surface-dense, stroke, stroke-strong, text, muted, dim, accent, accent-hi, glow-warm, glow-cool, on-light, danger
  - **radius:** xs 8, sm 10, md 14, lg 20, xl 24
  - **spacing:** 4-px scale (4…48)
- Seven text styles in Geist: Display, Title, Heading, Body, Body-strong, Caption, Mono.
- **Check:** the returned variable and style counts.

## Phase 2: Three concepts (low-to-mid fidelity, desktop 1280×800 + phone 390×844)
**How this serves the primary goal:** The owner has real alternatives to choose from.
- **Stage:** the playing track is the home canvas, with library, search and recommendations as layers.
- **Command:** a single command bar runs search, actions and navigation; the library is one filterable list.
- **Timeline:** queue, history and up-next form one horizontal river docked to the player.
- Every concept must:
  - play a track in ≤1 click from home and from results
  - play on a click anywhere on a track row
  - put the account menu on the avatar
  - keep blur to ≤3 layers
  - avoid signature elements of YouTube Music, Spotify and Apple Music
- **Check:** one `node.screenshot()` per concept frame, with no clipped text and the Geist font.

## Phase 3: Owner picks; hi-fi for the chosen concept (later, after approval)
**How this serves the primary goal:** The chosen concept becomes buildable.
- Components:
  - track row
  - card
  - player
  - slider
  - menu
  - sheet
  - toast
  - account menu (#5)
  - EQ (#6)
- All 12 routes, plus every state (empty, loading, error, success, focus, disabled).

## Phase 4: Code, one PR per screen (later)
**How this serves the primary goal:** Users get the new design.

## Verify
- Screenshots of each frame.
- A contrast check that text tokens on surfaces reach AA, with `--dim` used only for non-essential text.
- No hard-coded colours outside the variables.
