![Lyssna playing a song, with its Spotify Canvas behind word-synced lyrics](.github/assets/app.avif)

<div align="center">

# Lyssna

**Spotify on your desktop, with Spicy Lyrics built in.**

[![Release](https://img.shields.io/badge/release-0.2.0-1ed760?style=for-the-badge)](#get-started)
[![Windows · macOS](https://img.shields.io/badge/Windows%20%C2%B7%20macOS-2b2b2b?style=for-the-badge)](#build-from-source)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white)](Cargo.toml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue?style=for-the-badge)](LICENSE)

[Download](https://updates.lyssna.org/latest/lyssna.exe) · [Build from source](#build-from-source) · [Credits](#credits)

</div>

> [!IMPORTANT]
> **Spotify Premium is required.** librespot, which Lyssna streams through, cannot play on free accounts.

> [!WARNING]
> **Newer Spotify accounts may not play.** Spotify changed how it hands out audio decryption keys, and accounts created after an apparent cutoff (around 2023–2024) no longer get them through librespot. Songs fail with an audio key error. This is a Spotify-side restriction that affects every librespot version, and no workaround is known; see [librespot #1649](https://github.com/librespot-org/librespot/issues/1649).
>
> On Windows and Apple silicon Macs, Lyssna can play these accounts through Spotify Soloist, Spotify's official background player. Turn on **Play through Spotify Soloist** in Settings. It needs a free Soloist API key from Spotify for Developers and a one-time pick of the device in the Spotify app. See [FAQ: Spotify Soloist](#faq-spotify-soloist).

## What it is

A Spotify client written in Rust. It streams through [librespot](https://github.com/librespot-org/librespot) and draws its own interface in Spotify's layout, so it needs neither the Spotify app nor Spicetify.

| Feature | What you get |
| --- | --- |
| **Lyrics** | Word- and syllable-synced lyrics from six sources (Spicy Lyrics, Musixmatch, Apple Music, Spotify, LRCLIB, Netease), the Spicy Lyrics animator, Space Gravity, full screen (F11), your own LRC and TTML files, and community credits |
| **Canvas** | The song's Spotify Canvas beside the lyrics, tinting the background and the Now Playing panel |
| **Listening** | Gapless playback, crossfade from 1 to 12 seconds, media keys, Spotify Connect, and optional playback from the tray after closing the window |
| **Library** | Every playlist and Spotify folder, plus saved albums and followed artists, in Spotify's Recents order |
| **Updates** | Signed automatic updates on Windows, checked against an Ed25519 signature and SHA-256 hash before installing |

Press **F11** for full-screen lyrics over the Canvas. The controls fade out until you move the mouse:

![Full-screen lyrics over a Spotify Canvas](.github/assets/fullscreen.avif)

> [!NOTE]
> **Early development.** Some of Spotify's and Spicy Lyrics' features are still missing.

## Get started

1. **Install.** On Windows 10 or 11 (x64), download [the latest version](https://updates.lyssna.org/latest/lyssna.exe). On macOS 12 or later, [build from source](#build-from-source).
2. **Sign in.** Choose **Sign in with Spotify** and approve the connection in your browser. The setup window stays up until Spotify confirms the account.
3. **Play something.** Your playlists, albums and artists load into the library on the left.
4. **Make it yours.** Open **Settings** from the account menu for lyrics sources, the animator, crossfade and tray behaviour.

Sign-in uses OAuth with PKCE through a state-checked loopback callback. No Spotify developer app is needed, and later launches reuse the saved session.

On macOS, Lyssna sends stereo playback through AVFoundation with stereo spatialization enabled. On an Apple silicon Mac with compatible AirPods, play a song, open the AirPods menu in the menu bar, and choose **Fixed** or **Head Tracked** under **Spatialize Stereo**. macOS controls the effect; the decoded music remains stereo. Availability depends on your Mac, headphones, and system settings. See [Apple's Spatial Audio guide](https://support.apple.com/guide/airpods/control-spatial-audio-and-head-tracking-dev00eb7e0a3/web).

## FAQ: Spotify Soloist

> [!CAUTION]
> **Use Soloist at your own risk.** Lyssna is not affiliated with, endorsed by or supported by Spotify. Soloist is a Spotify developer product. Lyssna uses it with the API key you create, under your Spotify account, so you are responsible for following the [Spotify Developer Terms](https://developer.spotify.com/terms) and Spotify's Terms of Use.
>
> - Spotify can change, limit or withdraw Soloist, its API keys or its downloads at any time, without notice. Playback through Soloist can stop working until Lyssna catches up, or for good.
> - Spotify decides what an account may do. Lyssna can't promise that using Soloist this way won't lead Spotify to restrict your API key or your account.
> - Lyssna doesn't control Soloist. Bugs, outages and audio problems inside Soloist are Spotify's, and Lyssna's maintainers can't fix them.
> - Lyssna is provided as is, with no warranty, under the [GNU AGPL v3](LICENSE).
>
> If any of that isn't acceptable to you, leave **Play through Spotify Soloist** off.

<details>
<summary>Why do songs fail with an audio key error?</summary>

Spotify stopped handing out audio decryption keys to librespot for accounts created after an apparent cutoff around 2023–2024. Lyssna's own player is built on librespot, so those accounts can browse, search and see lyrics, but songs won't play. The block is on Spotify's side and affects every librespot client; see [librespot #1649](https://github.com/librespot-org/librespot/issues/1649). Older accounts usually play fine without Soloist.

</details>

<details>
<summary>What is Spotify Soloist?</summary>

Soloist is Spotify's official background player for developers. Spotify unlocks songs for it the same way it does for its own apps, so accounts refused by librespot play through it. Lyssna starts Soloist in the background and drives it: play, pause, seek, skip, queue, shuffle, repeat and volume all work from Lyssna's own controls.

Soloist isn't part of Lyssna. Lyssna downloads it from Spotify's servers onto your computer, checks that Spotify signed it, and keeps it up to date. Nothing from Soloist ships in Lyssna's download.

</details>

<details>
<summary>Which computers can run it?</summary>

- Windows 10 or 11, x64.
- Apple silicon Macs (M1 or newer) on macOS 13 or later.

Intel Macs can't run Soloist. On those, play through the Spotify app instead: open it, then pick its device from Lyssna's **Devices** button.

</details>

<details>
<summary>How do I set it up?</summary>

1. Open **Settings**, then **General**, and turn on **Play through Spotify Soloist** under **Playback**. Lyssna downloads Soloist from Spotify.
2. Create a Soloist API key at [Spotify for Developers](https://developer.spotify.com/), then paste it into the **Soloist API key** field. It's free. Lyssna keeps it in Windows Credential Manager, or in its encrypted credential store on macOS.
3. On your phone, open the Spotify app on the same Wi-Fi, tap the devices icon and pick **Lyssna Player**. You only do this once.

When the status reads **Ready**, songs play through Soloist.

</details>

<details>
<summary>Do I still need Spotify Premium?</summary>

Yes. Soloist plays for Premium accounts only, the same as librespot.

</details>

<details>
<summary>What works differently while Soloist plays?</summary>

- **Audio output:** Soloist plays through your system's default output, so Lyssna's output picker is greyed out. Change the output in your operating system's sound settings.
- **Visualiser bars:** the playing bars don't move.
- **Crossfade:** **Crossfade songs** and its length work. Soloist only reads them when it starts, so a change takes effect the next time playback is paused. **Crossfade when you pick or skip** is unavailable.
- **Volume normalization:** always off under Soloist.
- **DJ:** hidden while Soloist is on.

</details>

## Privacy

Spotify credentials stay on your computer: Windows Credential Manager on Windows, AES-256-GCM encrypted files on macOS. Lyrics requests carry the Spotify track ID, and Spotify tokens are never sent to Spicy Lyrics.

<details>
<summary>Lyrics lookups: who receives what</summary>

Every build includes Lyssna's publishable Spicy Lyrics client key, so you do not need a Spicy Lyrics account. Lyric sources receive the track ID, title and artist they need to find a match. Answers are cached on disk for three days, separately for each source; **Settings > Storage** clears them.

Community credits show the maker's Discord username by looking up their public spicylyrics.org profile. **Settings > Display** turns this off.

Self-hosted lyrics deployments can still be provisioned through standard input. That legacy mode asks for explicit consent before it sends a short-lived Spotify access token to the configured host, and it never sends refresh tokens or client secrets:

```powershell
Read-Host 'HTTPS lyrics host' | .\target\debug\lyssna.exe --set-lyrics-host --allow-spotify-access-token
```

The host is kept in the credential store, not in source. Your DNS and network traffic still reveal where requests go.

</details>

<details>
<summary>Spotify services and the optional developer connection</summary>

Browsing, library changes, audio and Connect control go through the librespot session, using the same internal services as Spotify's desktop client. Those services are undocumented and can change; this is not an approved Spotify SDK integration.

An optional developer connection keeps the older Web API login. Create an app in the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard), register `http://127.0.0.1:43821/callback`, and enter its Client ID in setup. Spotify's development-mode allowlist and quotas apply to it. No client secret is needed.

</details>

## Build from source

Windows needs Rust, Node.js 24, WebView2, the Visual Studio C++ build tools, CMake and LLVM (for libclang). Then:

```powershell
.\build.ps1          # build and install the latest main
.\build.ps1 local    # build the current checkout, uncommitted changes included
```

On macOS, install the Xcode Command Line Tools, Rustup, Node.js 24 and CMake (`brew install cmake`), then run `./build.sh` or `./build.sh local`.

<details>
<summary>What the build scripts do</summary>

By default each script requires a clean worktree, switches to `main`, fast-forwards from `origin/main`, restores npm dependencies, and builds a release. It installs the Rust version that `Cargo.toml` requires unless `RUSTUP_TOOLCHAIN` is set, and restores your original branch afterwards, including on failure. Nothing is pushed, and credentials and app data are left alone.

- **Windows** installs to `%LOCALAPPDATA%\Programs\Lyssna\lyssna.exe`, closes a running copy first, and starts the new one. `-NoLaunch` installs without starting it. Builds from this script turn automatic updates off; run it again to update. If PowerShell blocks the script, use `powershell -ExecutionPolicy Bypass -File .\build.ps1`. The script finds Visual Studio's bundled CMake and LLVM's default folder; otherwise put CMake on `PATH` and set `LIBCLANG_PATH` to the folder that contains `libclang.dll`.
- **macOS** signs the bundle with a reusable self-signed certificate (an explicit `APPLE_SIGNING_IDENTITY` or `APPLE_CERTIFICATE` takes precedence, and `LYSSNA_ADHOC_SIGNING=1` opts out), replaces `/Applications/Lyssna.app` (and removes an old `/Applications/Lyssna.app`) and opens it. Credentials live encrypted under `~/Library/Application Support/Lyssna/credentials/`; keep `device.key` with them when backing up.
- `.\build.ps1 <branch>` or `./build.sh <branch>` builds another branch; a commit hash builds that commit.

To build by hand:

```powershell
npm ci
npm run build
cargo build --release --locked
.\target\release\lyssna.exe
```

</details>

<details>
<summary>Releases and CI</summary>

The release workflow builds and tests the client, signs the update manifest, uploads the executable and source archive to the `spotiflex-releases` bucket (it keeps the old name for compatibility), and activates the stable channel last. [docs/UPDATES.md](docs/UPDATES.md) has the maintainer steps. The **Build macOS app** workflow produces ad-hoc signed Apple Silicon and Intel `.app.zip` files when dispatched by hand; they are not notarized, and macOS builds do not update automatically.

</details>

## Development

```powershell
npm test                 # frontend unit tests (Vitest)
npm run test:e2e         # browser tests against a mocked desktop bridge (Playwright)
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

At the time of writing, 234 Rust tests, 185 frontend unit tests and 183 browser tests pass. The browser tests use a mocked bridge, so they do not prove live Spotify behaviour. Keep `Cargo.lock` when updating dependencies: it pins `vergen` to 9.0.6 because librespot 0.8's build script breaks with vergen 9.1.0.

<details>
<summary>Current limits</summary>

- Remote-device commands and audio quality on other computers have not been verified by hand.
- Sign-in has not been tested end to end on an account outside the developer app's allowlist.
- Library ordering uses the last 50 played contexts from Spotify's play history; older items are ordered by when they were added.
- Spotify's public API does not expose folder metadata, so folders are read from Spotify's rootlist service.

</details>

## Credits

<div align="center">
<table>
  <tr>
    <td align="center" valign="top" width="200">
      <a href="https://github.com/Stormanzanii"><img src="https://images.weserv.nl/?url=github.com/Stormanzanii.png&w=200&h=200&fit=cover&mask=circle&output=png" width="96" alt="Arashii"></a><br>
      <b>Arashii</b><br>
      <sub>Creator of Lyssna</sub>
    </td>
    <td align="center" valign="top" width="200">
      <a href="https://github.com/iPixelGalaxy"><img src="https://images.weserv.nl/?url=github.com/iPixelGalaxy.png&w=200&h=200&fit=cover&mask=circle&output=png" width="96" alt="iPixelGalaxy"></a><br>
      <b>iPixelGalaxy</b><br>
      <sub>Spicy Lyrics fork</sub>
    </td>
    <td align="center" valign="top" width="200">
      <a href="https://github.com/Spikerko"><img src="https://images.weserv.nl/?url=github.com/Spikerko.png&w=200&h=200&fit=cover&mask=circle&output=png" width="96" alt="Spikerko"></a><br>
      <b>Spikerko</b><br>
      <sub>Creator of Spicy Lyrics</sub>
    </td>
  </tr>
</table>
</div>

**Built on**

- **[Spicy Lyrics](https://github.com/iPixelGalaxy/spicy-lyrics)** · Spikerko, fork by iPixelGalaxy · AGPL-3.0<br>Lyrics display, Space Gravity, word sync and the TTML tools.
- **[librespot](https://github.com/librespot-org/librespot)** · librespot contributors · MIT<br>Spotify playback and Connect.
- **[spr](https://github.com/Fraktality/spr)** · Fraktality · MIT<br>The spring that animates lyrics.
- **[Kawarp](https://github.com/better-lyrics/kawarp)** · Better Lyrics · MIT<br>The warped artwork and Canvas ambience.

Lyssna reimplements Spicy Lyrics' behaviour. The only Spicy Lyrics code it copies is the Gibberish word filter's dictionary and rules, under the same AGPL-3.0 licence; [NOTICE.md](NOTICE.md) has the details.

## License

Lyssna is free software under the [GNU AGPL v3](LICENSE). [NOTICE.md](NOTICE.md) records each upstream project Lyssna references, and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) holds the licences of bundled dependencies. Spotify is a trademark of Spotify AB; Lyssna is not affiliated with Spotify.
