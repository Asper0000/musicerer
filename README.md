<p align="center">
  <img src="assets/icon.svg" width="160" height="160" alt="Musicerer logo">
</p>

<h1 align="center">Musicerer</h1>

<p align="center">
  <strong>A native queue-based audio downloader that saves tagged AAC <code>.m4a</code> files.</strong><br>
  Built with Rust and Iced.
</p>

<p align="center">
  <a href="#"><img src="https://img.shields.io/badge/Version-0.3.0-6f42c1" alt="Version 0.3.0"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.95%2B-orange?logo=rust&logoColor=white" alt="Rust"></a>
  <a href="#"><img src="https://img.shields.io/badge/Platform-Windows%2010%2F11%20%7C%20Linux-0078D6?logo=windows&logoColor=white" alt="Platform"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPLv3-blue.svg" alt="License"></a>
</p>

---

## Contents

- [Overview](#overview)
- [Features](#features)
- [Supported services](#supported-services)
- [How it works](#how-it-works)
- [Runtime tools](#runtime-tools)
- [Building from source](#building-from-source)
- [Configuration](#configuration)
- [Technical details](#technical-details)
- [License](#license)
- [Credits](#credits)

---

## Overview

**Musicerer** is a desktop application for building a queue of music links and downloading their audio as AAC `.m4a` files. It combines media extraction with metadata lookup and tagging, then organizes completed files in the selected output directory.

The application is written in Rust and uses Iced with the `tiny-skia` renderer. Its interface includes queue progress, track previews, download logs, settings, and an Installation tab for Musicerer and its runtime tools.

Musicerer does not embed third-party media-tool binaries in its executable or release packages. Required tools are installed and updated separately by the application.

## Features

- **Queue-based downloads:** Add links, preview tracks, monitor progress, inspect logs, and run multiple downloads concurrently.
- **Multi-service support:** Accepts links from YouTube, YouTube Music, SoundCloud, Spotify, Deezer, and Apple Music.
- **Metadata matching:** Uses source metadata and cross-service lookups to improve track information and find playable audio for metadata-based sources.
- **Audio conversion:** Produces AAC `.m4a` files using FFmpeg.
- **Metadata tagging:** Writes available track and album information, including artist, album, release year, track and disc numbers, genre, label, ISRC, BPM, and ReplayGain.
- **Cover art:** Retrieves available artwork and converts it to square cover art for the output file.
- **Playlist organization:** Can place playlist and album downloads into their own folders.
- **Download controls:** Supports stopping the queue and optionally skipping files that already exist in the destination.
- **Installation management:** Checks for Musicerer and tool updates, and provides controls to install, reinstall, or remove managed tools.
- **Persistent settings:** Remembers the output directory, cookies file, and download preferences between runs.

## Supported services

Musicerer accepts links from:

- YouTube
- YouTube Music
- SoundCloud
- Spotify
- Deezer
- Apple Music

YouTube and SoundCloud provide the media sources used for extraction. Spotify, Deezer, and Apple Music links provide track metadata and are matched to an available audio source.

## How it works

1. Add a supported link to the queue. Musicerer identifies the service and resolves track information.
2. For metadata-based services, Musicerer looks for a matching playable audio source.
3. The scheduler runs downloads concurrently, with bounded worker counts and retry handling.
4. `yt-dlp` extracts the audio and FFmpeg converts it to AAC `.m4a`.
5. Musicerer gathers and merges available metadata, adds cover art, writes the tags, and moves the completed file into the output directory.
6. Temporary download data is cleaned up after processing.

## Runtime tools

Musicerer requires external media tools, but **does not bundle them** in the application executable, Windows installer, AppImage, DEB, or RPM packages.

The Installation tab manages these tools:

| Tool | Purpose |
|---|---|
| yt-dlp | Media extraction |
| FFmpeg | Audio conversion and media processing |
| FFprobe | Media information and duration checks |
| Deno | JavaScript runtime used by supported extraction workflows |

On supported systems, Musicerer can download and update managed copies automatically. Managed tools are stored in the user's application configuration directory and take priority over other copies.

If a managed copy is not present, Musicerer can also find tools in a local `tools` directory or through the system `PATH`. The Installation tab reports detected tool status and provides controls for managed installations.

Automatic tool downloads are supported on Windows x64, Linux x64, and Linux ARM64. Musicerer itself is packaged for Windows and Linux.

---

## Building from source

### Prerequisites

- Rust 1.95 or newer.
- Windows: Inno Setup 6 or 7 to build the setup executable.
- Windows-to-Linux builds: WSL with a Linux Rust toolchain.
- Linux builds: a C toolchain, `pkg-config`, the X11/Wayland development libraries, and the packaging tools for the selected format.

### Windows

Run `build.bat` from the project directory:

```bat
build.bat
build.bat portable
build.bat setup
build.bat both
```

| Target | Result |
|---|---|
| `portable` | Builds `dist\portable\musicerer.exe` |
| `setup` | Builds the portable executable, then creates the Windows installer |
| `both` | Builds the portable executable and Windows installer |
| *(no target)* | Same as `both` |

The setup executable is written to `dist\installer\Musicerer_Setup_v<version>.exe`. The version is read from `Cargo.toml`. The installer contains Musicerer itself; runtime media tools are installed separately by the application.

### Linux

Linux packages can be built inside WSL when invoked from Windows:

```bat
build.bat appimage
build.bat deb
build.bat rpm
build.bat linux
```

`linux` builds all three formats. `all` builds the Windows portable executable, Windows setup, and all Linux packages.

On a native Linux system, run:

```bash
packaging/linux/build.sh
packaging/linux/build.sh appimage
packaging/linux/build.sh deb
packaging/linux/build.sh rpm
```

Linux packages contain the Musicerer application and desktop integration. They do not contain runtime media tools or require a project-level `linux_tools` directory. The AppImage build downloads and caches `appimagetool` as a build-time dependency; it is not included in the resulting application.

### Plain Cargo build

To build only the application executable:

```bash
cargo build --release
```

The executable is written to `target/release/musicerer` on Linux or `target/release/musicerer.exe` on Windows. No third-party runtime media tools are embedded in the executable.

---

## Configuration

Settings are available from the **Settings** tab:

- **Output folder:** Destination for completed `.m4a` files.
- **Cookies file:** Optional Netscape-format `cookies.txt` file.
- **Skip existing files:** Skips a track when a file with the same output name already exists.
- **Playlist folders:** Saves playlist and album tracks in their own subfolders.
- **VPN warning:** The startup warning can be dismissed or suppressed.

Settings are saved automatically to the operating system's user configuration directory, in `Musicerer/config.json` (for example, `%APPDATA%\Musicerer\config.json` on Windows and `~/.config/Musicerer/config.json` on Linux).

---

## Technical details

- **Language:** Rust, edition 2021.
- **GUI:** Iced 0.14 with the `tiny-skia` renderer.
- **Download process:** External tools are launched as child processes. Windows subprocesses are configured to avoid opening extra console windows.
- **Concurrency:** Queue downloads and metadata resolution use worker threads and communicate progress and results back to the UI.
- **Metadata:** Separate service modules handle Spotify, Deezer, Apple Music/iTunes, and SoundCloud metadata. A merge layer resolves competing metadata values.
- **Updates:** Musicerer checks GitHub releases for application and managed-tool updates. Tool downloads are installed into the managed tools directory.

---

## License

Musicerer is licensed under the **GNU General Public License v3.0** (GPLv3).

The full license text is available in [`LICENSE`](LICENSE).

---

## Credits

- **[Iced](https://github.com/iced-rs/iced):** Cross-platform GUI library for Rust.
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp):** Media extraction.
- **[FFmpeg](https://ffmpeg.org/):** Audio conversion and media processing.
- **[audiotags](https://github.com/ttscoff/audiotags)** and **[mp4ameta](https://github.com/jdecourcy/mp4ameta):** Audio metadata reading and writing.
- **[tiny-skia](https://github.com/RazrFalcon/tiny-skia):** 2D rendering library.

---

<p align="center">
  <em>If Musicerer is useful to you, consider starring the repository.</em>
</p>
