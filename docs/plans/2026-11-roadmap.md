# YT Music: roadmap, November 2026

**Primary goal:** Any song starts quickly on PC and Android, including 18+ songs on a signed-in account. The app is identified as "YT Music" everywhere, has an account menu and an equalizer, and gets a new Spotify-level design from Figma.

Each phase is one GitHub issue. Phases are worked one at a time, only after the owner approves each one. Each phase ends in its own commit or PR.

## Phase 0: Discovery (done)
**How this serves the primary goal:** It establishes which APIs actually exist, so later phases don't invent any.
- Today yt-dlp is called only in `Yt::stream_ytdlp` (`src-tauri/src/yt.rs`). Search, metadata, playlists, account and lyrics are already Rust.
- 18+ path: `Yt::stream` waits for both native resolvers (`select_ok`) to fail, then runs yt-dlp with a temporary cookie file plus Node. That takes 3–5 s.
- yt-dlp clients (`_base.py`, master, 2026-11-08) split into two groups, and no client is in both. android, ios and visionos work without JS but don't support cookies. web, web_embedded, tv and mweb support cookies but need the JS player. Signed-in defaults are `web_embedded`, `tv_samsung`, `web`; neither web_embedded nor tv needs a GVS PO token.
- On Windows, `navigator.mediaSession` in WebView2 has no AppUserModelID, so the overlay shows "Unknown app".
- `#account-btn` (`ui/app.js`) opens Settings.

## Phase 1: Windows overlay name (#2)
**How this serves the primary goal:** Windows shows "YT Music" in its media overlay.
- Set an explicit AUMID (`SetCurrentProcessExplicitAppUserModelID`) and match it to the installer shortcut. If that isn't enough, switch to native SMTC and turn off WebView mediaSession on Windows.
- Verify in the installed build that the overlay shows the name and icon and that its buttons work.
- Guard: don't create a second media session alongside the WebView one.

## Phase 2: Fast 18+ on desktop (#3)
**How this serves the primary goal:** Removes the wait before an 18+ track starts.
- Remember which ids need sign-in (DB) and send them straight to yt-dlp.
- When visionOS returns `LOGIN_REQUIRED`, start yt-dlp immediately.
- Prefetch the next track's URL.
- Tune the yt-dlp args to use only the needed player clients.
- Verify with timings before and after on the three test songs ("так похуй").
- Guard: no new yt-dlp call sites outside stream resolution.

## Phase 3: 18+ on Android (#4)
**How this serves the primary goal:** 18+ tracks play on the phone.
- Step 1: Android Google sign-in through a Kotlin CookieManager plugin.
- Step 2: signed `web_embedded`/`tv_samsung` player request in Rust, plus signature/`n` deobfuscation in rquickjs with the player JS cached.
- Fallback: a clear "18+ not available" message.
- Verify on a device, using the build from CI.

## Phase 4: Account menu (#5)
**How this serves the primary goal:** Account actions are one click away.
- Dropdown with: avatar and name, Switch account, Sign in / Sign out, Sync library, Settings, Updates.
- Closes on Esc or an outside click. Shows as a bottom sheet on phones.

## Phase 5: Equalizer (#6)
**How this serves the primary goal:** Users can shape the sound.
- 10 bands of Web Audio `BiquadFilterNode` plus a preamp, presets, and settings stored in the Rust DB.
- Verify that background playback still works on Android.

## Phase 6: Figma redesign (#7)
**How this serves the primary goal:** Every feature becomes easy to reach on PC and phone.
- Audit, design system, screens, missing features, then implementation with one PR per screen.
- Needs the owner's Figma file link.

## Final: Verify
- `cargo test` (including `-- --ignored` live tests for streams), plus a desktop smoke test when the owner says the PC is free, plus an Android APK from CI.
