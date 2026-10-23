<p align="center">
  <img src="assets/icon.svg" width="160" height="160" alt="Musicerer logo">
</p>

<h1 align="center">Musicerer</h1>

<p align="center">
  <strong>A sleek, native queue downloader for extracting audio into AAC <code>.m4a</code> files.</strong><br>
  Built in Rust with Iced and tiny-skia.
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.99.1%2B-orange?logo=rust&logoColor=white" alt="Rust"></a>
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

- **Portable.** On Windows, `yt-dlp`, `ffmpeg`, and the fallback tool are compiled directly into the executable.
- **Safe & clean.** Downloads run in isolated temporary directories and move to your output folder atomically.
- **Reversible.** It does not modify system registries or leave permanent traces.

> **Side Note:** The current Windows executable is quite large because it bundles a full, uncompressed build of FFmpeg. A future update will strip down unnecessary FFmpeg components to significantly reduce the final binary size.

---

## Features

- 🎵 **Extensive Platform Support**: Paste URLs from a wide variety of supported media platforms and hit Add.
- 📦 **Fully portable (Windows)**: Embeds `ffmpeg.exe`, `ffprobe.exe`, and the fallback tool directly into the binary.
- 🎨 **Custom UI**: Frameless window, dark monochrome theme with blue accents, and draggable/resizable edges.
- 🖼️ **Perfect metadata**: Downloads and center-crops cover art to a 1:1 square, writing it to the file tags.
- 📊 **Queue management**: Real-time progress bars, track previews, and detailed log output.
- 💾 **Persistent settings**: Saves your output directory and cookies path automatically in your OS config directory.
- 💽 **Storage monitor**: Displays live free/total disk space for your output drive.

---

## How it works

Musicerer abstracts away the complexity of command-line media extraction. 

1. When you add a URL, it queries the platform's public API (or `yt-dlp`) to fetch the title and thumbnail immediately.
2. The queue is processed sequentially. For each track, a temporary directory is created.
3. `yt-dlp` extracts the best audio stream and saves it as `.m4a`. 
4. A custom Rust module crops the cover art into a square, and `audiotags` injects the metadata.
5. The file is moved to your chosen output folder, and the temp directory is cleaned up.

---

## What doesn't work: Go+

**SoundCloud Go+ tracks** are heavily restricted. While `yt-dlp` can download Go+ tracks, it often grabs a ~30-second preview clip because the session isn't entitled to the full stream. Musicerer actively detects these short previews and fails the download intentionally to prevent you from saving incomplete audio. **Musicerer does not bypass Go+ protection.**

*(Note: For standard tracks and DRM-protected tracks, Musicerer bypasses DRM protection automatically using its integrated fallback tool, allowing seamless downloading of otherwise locked tracks.)*

---

## Building from source

**Note:** No build scripts are shipped by default. To build Musicerer, you must have the Rust toolchain and the required external tools installed.

### Prerequisites

- **Rust** 1.99.1 or newer
- **yt-dlp** on your system `PATH`
- **deno** (Required for certain JavaScript challenges)

### Setup (Windows)

To embed the tools into the `.exe`, place the following files in a `tools/` folder at the root of the project before compiling:
- `soundcloud.exe` (or your specific fallback binary name)
- `ffmpeg.exe`
- `ffprobe.exe`

### Setup (Linux)

> **⚠️ Warning:** The Linux build has not been tested at all. While the codebase includes Linux-specific configurations (such as skipping embedded binaries and providing an AppImage build script), it has not been verified on a real Linux machine. Proceed at your own risk.

On Linux, the embedded tools are skipped. You must install them via your package manager:
~~~bash
sudo apt update
sudo apt install build-essential pkg-config libx11-dev libxcursor-dev libxrandr-dev libxi-dev libxkbcommon-dev libwayland-dev ffmpeg yt-dlp
~~~

### Build

Open your terminal in the project root and run:

~~~bash
cargo build --release
~~~

The executable will be located at `target/release/musicerer` (or `musicerer.exe` on Windows).

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
- **Asynchronous Backend:** Downloads and previews run in separate threads. They communicate back to the UI via an `Arc<Mutex<Vec<WorkerEvent>>>` inbox, polled by an Iced subscription every 120ms.
- **Subprocess Management:** External tools (`yt-dlp`, `ffmpeg`, etc.) are spawned using `std::process::Command`. On Windows, `CREATE_NO_WINDOW` flags are applied to prevent console windows from flashing.
- **Fallback Extraction:** If DRM is detected, a custom Rust module invokes the fallback binary, searches the temp directory for the resulting audio file, and transcodes it using the embedded `ffmpeg` with `-vn` flags to prevent AAC mapping errors.

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
