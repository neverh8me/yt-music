<p align="center"><img src="src-tauri/icons/128x128@2x.png" alt="YT Music" width="96"></p>

<h1 align="center">YT Music</h1>

<p align="center"><b>English</b> · <a href="README.uk.md">Українська</a> · <a href="README.ru.md">Русский</a></p>

A fast, unofficial YouTube Music client for Windows, macOS, Linux and Android, written in Rust (Tauri 2). Search, stream, build your library and download music for offline listening — including 18+ songs on your own account.

> Unofficial project. Not affiliated with, endorsed by, or sponsored by Google LLC. YouTube and YouTube Music are trademarks of Google LLC.

![Home: mixes made for you, quick picks, your playlists in the sidebar](docs/screenshots/home.png)

| Player with synced lyrics | Phone |
|---|---|
| ![Player with synced lyrics](docs/screenshots/lyrics.png) | <img src="docs/screenshots/mobile.png" alt="Phone layout" width="300"> |

<details><summary>More screenshots</summary>

![Album](docs/screenshots/album.png)
![Artist](docs/screenshots/artist.png)
![Search](docs/screenshots/search.png)

</details>

## Features

**Listening**
- Home made for you: your mix and a mix for each favourite artist, from your history and likes; quick picks, top songs chart, new releases, recently played
- Search songs, albums, artists and playlists with suggestions; album, artist (with play counts) and playlist pages
- Songs only: music videos are left out of search, charts, artist pages and radio
- A player that expands from the bottom bar to full screen: big cover, controls, and *Up next* / *Lyrics* right beside it
- Queue, shuffle, repeat (all / one), seeking from the bar, volume, sleep timer, media keys
- Autoplay: when the queue runs out, similar songs (radio) keep playing
- Time-synced lyrics (the current line lights up, click a line to jump), from [LRCLIB](https://lrclib.net) with YouTube Music lyrics as fallback
- 10-band equalizer with presets
- 18+ songs play when you're signed in (age-verified account)
- On Windows, macOS and Linux, signed-in playback goes through YouTube Music's own web player: songs start at once and don't stop half-way

**Library**
- Sidebar with your library and playlists, always at hand
- Liked songs, your own playlists and history, stored locally in SQLite
- Sign in with Google to bring your YouTube likes and playlists; account menu on your avatar (sync, switch account, sign out)
- Select several songs, or all of them, on any page: play next, add to a playlist, download or remove at once
- Likes and unlikes reach your YouTube Music account
- Offline downloads (high or standard quality), stored encrypted in the app's own `.ytmc` format, so they play only inside YT Music

**App**
- Glass design (by [@dopedasya](https://github.com/dopedasya)) in dark, light or system mode, with the cover's colour as ambient light
- Own title bar on Windows/Linux; shows up as "YT Music" in the Windows media overlay
- English, Ukrainian and Russian
- One-click updates: desktop installs a signed update and restarts; Android downloads the new APK

## Install

Grab the file for your platform from [Releases](../../releases). Each release lists what changed.

> **1.1.0 is out for Windows and Android.** The macOS, Linux and iPhone builds come back in a later release; until then macOS and Linux users can keep [1.0.1](../../releases/tag/v1.0.1).

| Platform | File |
|---|---|
| Windows | `*_Windows-x64-setup.exe` |
| macOS, Apple Silicon (M1/M2/M3/M4) | `*_macOS-Apple-Silicon.dmg` |
| macOS, Intel | `*_macOS-Intel.dmg` |
| Linux (Debian/Ubuntu) | `*_Linux-x64.deb` |
| Android | `*_Android.apk` (allow installing from unknown sources) |
| iPhone (**beta**) | `*-beta_iOS-unsigned.ipa`, installed with [AltStore](https://altstore.io) or [Sideloadly](https://sideloadly.io) |

The iPhone build is a **beta** and is not in the App Store. The `.ipa` is unsigned: AltStore or Sideloadly signs it with your own Apple ID and installs it. With a free Apple ID the app works for 7 days, then has to be refreshed the same way (AltStore can do it by itself). It does not update itself: install the new `.ipa` over the old one.

macOS builds aren't notarized: on first launch right-click the app and choose **Open**.

### System requirements

| Platform | Minimum |
|---|---|
| Windows | Windows 10 or 11, 64-bit (WebView2 is built in) |
| macOS | macOS 11 Big Sur or later (Apple Silicon or Intel) |
| Linux | Ubuntu 22.04+, Debian 12+ or another distro with glibc 2.35 and WebKitGTK 4.1; x86-64 |
| Android | Android 7.0+ on a 64-bit ARM phone, with Android System WebView kept up to date (Google Play) |
| iPhone (beta) | iOS 14+, a computer with AltStore or Sideloadly, and an Apple ID |

The desktop apps download [yt-dlp](https://github.com/yt-dlp/yt-dlp) on first start (about 18 MB) to play music. An internet connection is needed except for downloaded songs.

## Build from source

Requirements: [Rust](https://rustup.rs) (stable). On Windows you also need the MSVC Build Tools and WebView2 (preinstalled on Windows 10/11). On Linux you need the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
cd src-tauri
cargo run                          # dev build
cargo install tauri-cli --version "^2"
cargo tauri build                  # installer in target/release/bundle/
```

The frontend is plain HTML/CSS/JS in `ui/`, so no Node.js or bundler is needed.

## How it works

The logic is Rust; the web layer renders the interface and plays `<audio>`.

```
ui/                 interface (no build step): app.js, player.js, api.js, eq.js, i18n.js, icons.js,
                    ejs/ (stream challenge solver, see below)
src-tauri/src/
  lib.rs            startup, command registration (desktop + Android entry point)
  player.rs         queue engine: shuffle, repeat, autoplay radio, persistence
  playback.rs       player commands; history, prefetching the next streams
  yt.rs             YouTube Music metadata (rustypipe) + audio stream resolution
  ytdlp.rs          yt-dlp (desktop): installed and updated automatically, checksum-verified
  jsplayer.rs       solves YouTube's stream challenges without yt-dlp (Android)
  server.rs         local 127.0.0.1 server for <audio> (Range/seeking, decrypts offline files)
  vault.rs          encrypted offline container (.ytmc, ChaCha20, seekable)
  downloads.rs      chunked downloads straight into the vault
  lyrics.rs         LRCLIB lookup, fuzzy matching, LRC parsing
  recommend.rs      "made for you" mixes from your history and likes
  color.rs          dominant cover colour for tints
  auth.rs / account.rs   "Sign in with Google", account name and avatar
  webplayer.rs      desktop: YouTube Music's web player in a hidden window, driven from Rust
  media.rs          Windows media overlay (system media controls)
  db.rs             SQLite: likes, playlists, history, downloads, cache, settings
```

- **Metadata** (search, albums, artists, charts, radio, account) comes from [rustypipe](https://codeberg.org/ThetaDev/rustypipe).
- **Audio on desktop:** once you're signed in, songs play through YouTube Music's own web player in a hidden window (`webplayer.rs`): our interface drives it and shows its state. Songs start at once, 18+ songs play, and YouTube shows its ads as on the site. Guests, downloaded songs and the fallback (if the site doesn't start a song) use the built-in player below, with [yt-dlp](https://github.com/yt-dlp/yt-dlp) resolving the streams.
- **Audio on Android** (no yt-dlp): a direct InnerTube `player` request with the visionOS client races rustypipe. For 18+ songs the app makes a signed request and solves the stream URL challenges with yt-dlp's own solver ([yt-dlp/ejs](https://github.com/yt-dlp/ejs), Unlicense) running in the webview's JS engine; the parsed player is cached on disk.
- googlevideo rejects open-ended ranges, so the local proxy fetches upstream in bounded 4 MB chunks and stitches them into one stream. Seeking works, and stream URLs never reach the web layer.

## Tests

```sh
cargo test                            # offline tests
cargo test -- --ignored --nocapture   # live tests against real YouTube
node tests/player.test.mjs            # player engine switching (UI, fake backend)
```

## Keyboard shortcuts

| Key | Action |
|---|---|
| Space | Play / pause |
| ← / → | Seek −5 / +5 s |
| Ctrl+← / Ctrl+→ | Previous / next |
| Ctrl+↑ / Ctrl+↓ | Volume |
| Ctrl+K or / | Search |
| Alt+L | Like current song |
| Esc | Close the player or a menu |
| Alt+← | Back |

## License

[GPL-3.0](LICENSE). The project depends on rustypipe, which is GPL-3.0, so the app as a whole is distributed under the same license. The vendored stream challenge solver (`ui/ejs`) is from yt-dlp/ejs (Unlicense).
