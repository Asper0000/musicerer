<p align="center">
  <img src="assets/icon.svg" width="160" height="160" alt="Musicerer logo">
</p>

<h1 align="center">Musicerer</h1>

<p align="center">
  <strong>A sleek, native queue downloader for extracting audio into AAC <code>.m4a</code> files.</strong><br>
  Built in Rust with Iced and tiny-skia.
</p>

<p align="center">
  <a href="#"><img src="https://img.shields.io/badge/Version-0.2-6f42c1" alt="Version 0.2"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.95%2B-orange?logo=rust&logoColor=white" alt="Rust"></a>
  <a href="#"><img src="https://img.shields.io/badge/Platform-Windows%2010%2F11%20%7C%20Linux-0078D6?logo=windows&logoColor=white" alt="Platform"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPLv3-blue.svg" alt="License"></a>
</p>

---

## Contents

- [What is Musicerer?](#what-is-musicerer)
- [Features](#features)
- [How it works](#how-it-works)
- [What doesn't work: Go+](#what-doesnt-work-go)
- [Building from source](#building-from-source)
- [Configuration](#configuration)
- [Technical details](#technical-details)
- [License](#license)
- [Credits](#credits)

---

## What is Musicerer?

**Musicerer** is a lightweight, native GUI application designed to download audio from various supported platforms. It takes a queue of URLs, extracts the best available audio, and saves it as a standardized `.m4a` (AAC) file complete with iTunes-style metadata and 1:1 cover art.

Unlike traditional downloaders that rely on bulky web frameworks, Musicerer is built entirely in Rust using **Iced** and **tiny-skia**. It renders purely on the CPU, requires no GPU, and features a completely custom, borderless dark UI with native window resizing.

- **Convenient to run.** The Windows release comes in two flavours: a **bundled** `.exe` with `yt-dlp`, `ffmpeg`, `ffprobe`, Deno and the optional SoundCloud fallback tool all packed inside, and a slim **portable** `.exe` that uses a `tools` folder or your `PATH`.
- **Safe & clean.** Downloads run in isolated temporary directories and move to your output folder atomically.
- **Low-impact.** It keeps preferences in the current user’s config directory and does not modify system registries.

> **Side Note:** The bundled Windows executable is large because it embeds full, uncompressed builds of the tools above (FFmpeg and Deno are the biggest).

---

## Features

- 🎵 **Platform support**: Add individual tracks or playlists from YouTube and SoundCloud.
- 📦 **Two Windows builds**: `bundled` embeds all five tools (`yt-dlp`, `ffmpeg`, `ffprobe`, `deno`, `soundcloud`); `portable` embeds none, looks in a `tools` folder (current directory, then next to the exe), then on `PATH`, and shows a warning listing whatever is missing.
- 🎨 **Custom UI**: Frameless window, dark monochrome theme with blue accents, and draggable/resizable edges.
- 🖼️ **Track metadata**: Embeds available title, artist, album, and center-cropped 1:1 cover art into the saved file.
- 📊 **Queue management**: Real-time progress bars, track previews, and detailed log output.
- 🎧 **Playlist support**: Playlists are fully supported for all platforms.
- 💾 **Persistent settings**: Saves your output directory and cookies path automatically in your OS config directory.
- 💽 **Storage monitor**: Shows available and total disk space for the output drive.

---

## How it works

Musicerer abstracts away the complexity of command-line media extraction.

1. When a URL is added, `yt-dlp` fetches track metadata and artwork. SoundCloud playlists are expanded first; tracks with incomplete playlist metadata are then resolved individually.
2. Downloads run concurrently in isolated temporary directories, with up to six workers. When a server rate-limits requests, the scheduler reduces concurrency and retries within a bounded limit.
3. `yt-dlp` extracts audio and converts it to `.m4a` with FFmpeg.
4. Rust code crops available cover art to a square, and `audiotags` writes the available title, artist, album, and artwork metadata.
5. Completed audio is moved into the chosen output folder. Temporary download data is cleaned up automatically.

---

## What doesn't work: Go+

**SoundCloud Go+ tracks** may return a short preview instead of the full stream when the current session is not entitled to the complete track. When `ffprobe` is available and the extracted SoundCloud audio is shorter than 45 seconds, Musicerer marks the download as failed rather than silently saving a likely preview. This duration check is a heuristic; a genuinely short track can also trigger it.

For SoundCloud items that `yt-dlp` reports as protected, Musicerer can try the configured fallback tool. This is best-effort and does not guarantee access to restricted content. YouTube DRM-protected content is skipped.

---

## Building from source

To build Musicerer, install the Rust toolchain and the runtime tools required by your target platform.

### Prerequisites

- **Rust** 1.95 or newer (required by the atomic `try_update` API used by the scheduler; Iced 0.14 itself requires 1.88)
- **yt-dlp**, **ffmpeg**/**ffprobe** and **Deno** (required for some YouTube JavaScript challenges), either in a `tools` folder or on `PATH`

### Setup (Windows)

Put these files in a `tools/` folder at the project root:
`yt-dlp.exe`, `ffmpeg.exe`, `ffprobe.exe`, `deno.exe`, `soundcloud.exe`

Then run:

~~~bat
build.bat            :: both builds
build.bat bundled    :: only the all-in-one exe
build.bat portable   :: only the slim exe
~~~

Results:

- `dist\bundled\musicerer.exe` - everything embedded; extracted on first use to `%TEMP%\musicerer-tools-<version>`.
- `dist\portable\musicerer.exe` - nothing embedded. Place the tools in `dist\portable\tools\` (or put them on `PATH`). If required tools are missing, the app still opens and shows a warning dialog listing them. `soundcloud.exe` is optional (DRM fallback only).

The `bundled` build needs all five files present at compile time; the `portable` build compiles without them.

### Setup (Linux)

Linux packages are built from Windows through WSL, or directly on any Linux machine. Tools are never embedded in the executable on Linux; instead the packages ship them in a `tools` folder next to the real binary (`/usr/lib/musicerer/tools`), which Musicerer already searches.

Put these **Linux** binaries in a `linux_tools/` folder at the project root:
`yt-dlp`, `ffmpeg`, `ffprobe`, `deno`, `soundcloud` (the build refuses Windows `.exe` files).

One-time setup inside WSL (Ubuntu/Debian shown; use your distro's equivalents):

~~~bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh     # Linux Rust toolchain (the Windows one is not used)
sudo apt install build-essential pkg-config libx11-dev libxcursor-dev libxrandr-dev \
                 libxi-dev libxkbcommon-dev libwayland-dev           # GUI build libraries
sudo apt install rpm curl                                           # for .rpm and the AppImage tool download
~~~

Then, from Windows:

~~~bat
build.bat linux        :: AppImage + .deb + .rpm
build.bat appimage     :: or deb / rpm, individually
build.bat all          :: both Windows builds and all Linux packages
build.bat portable deb :: targets can be combined
~~~

On a native Linux machine run `packaging/linux/build.sh [appimage] [deb] [rpm]` instead.

### Build (plain cargo)

~~~bash
cargo build --release
~~~

The executable will be at `target/release/musicerer`. Without a package it looks for tools in a `tools` folder, then on `PATH`.

### Output layout

~~~text
dist/
  bundled/musicerer.exe
  portable/musicerer.exe  (+ tools/)
  linux/
    appimage/Musicerer-<version>-<arch>.AppImage
    deb/musicerer_<version>_<arch>.deb
    rpm/musicerer-<version>-1.<arch>.rpm
~~~

### Project layout

~~~text
build.bat                Windows entry point; also drives the Linux build through WSL
packaging/linux/         build.sh (entry), common.sh, appimage.sh, deb.sh, rpm.sh,
                         AppRun, musicerer.desktop, templates/ (deb control, rpm spec)
tools/                   Windows tool binaries (git-ignored)
linux_tools/             Linux tool binaries (git-ignored)
src/  assets/            application source and icons
~~~

---

## Configuration

You can configure Musicerer directly from the **Settings** tab:
- **Output folder:** Where finished `.m4a` files are saved.
- **Cookies file:** An optional Netscape-format `cookies.txt` file. This is highly recommended for bypassing bot-checks or downloading age-gated content.

These settings are saved automatically to your OS config directory (e.g., `%APPDATA%\Musicerer\config.json` on Windows, `~/.config/Musicerer/config.json` on Linux).

---

## Technical details

### Architecture
- **UI:** Built with `iced` 0.14 using the `tiny-skia` backend for pure CPU rendering. The UI uses a custom `stack!` layout to overlay invisible `MouseArea` handles on the edges of the window for native-feeling resizing.
- **Concurrent backend:** Downloads, playlist expansion, thumbnails, and previews run on bounded worker threads. Workers communicate with the UI through an `Arc<Mutex<Vec<WorkerEvent>>>` inbox, polled by an Iced subscription every 120 ms.
- **Subprocess Management:** External tools (`yt-dlp`, `ffmpeg`, etc.) are spawned using `std::process::Command`. On Windows, `CREATE_NO_WINDOW` flags are applied to prevent console windows from flashing.
- **Fallback Extraction:** If a SoundCloud track is DRM-protected and the optional `soundcloud.exe` is present, a custom Rust module invokes it, searches the temp directory for the resulting audio file, and transcodes it using FFmpeg with `-vn` flags to prevent AAC mapping errors. Without that optional binary, the DRM fallback is skipped with a clear log message; regular downloads remain unaffected.

### Window API
Musicerer utilizes `iced::window::latest()` to get the current window ID, allowing it to call `iced::window::drag_resize()` and `iced::window::drag()` for custom topbar interactions.

---

## License

Musicerer is licensed under the **GNU General Public License v3.0** (GPLv3).

This is an original, standalone project. It is not a fork or modification of any existing codebase. By using, modifying, or redistributing this project, you agree to the terms of the GPLv3:

- You may **use** Musicerer for any purpose.
- You may **study and modify** the source.
- You may **redistribute** modified versions **only under the same GPLv3 license** and **only with source code available**.
- You may **not** add restrictions that limit others' freedoms under GPLv3.

The full license text is in [`LICENSE`](LICENSE).

---

## Credits

- **[Iced](https://github.com/iced-rs/iced)**: The cross-platform GUI library for Rust.
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)**: The backbone of the media extraction process.
- **[FFmpeg](https://ffmpeg.org/)**: For audio transcoding and thumbnail conversion.
- **[audiotags](https://github.com/ttscoff/audiotags)**: For reading and writing ID3/M4A metadata.
- **[tiny-skia](https://github.com/RazrFalcon/tiny-skia)**: The pure-Rust CPU rasteriser.

---

<p align="center">
  <em>If Musicerer makes your music library better, consider starring the repo ⭐</em>
</p>
