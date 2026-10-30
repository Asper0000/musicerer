<p align="center">
  <img src="assets/icon.svg" width="160" height="160" alt="Musicerer logo">
</p>

<h1 align="center">Musicerer</h1>

<p align="center">
  <strong>A sleek, native queue downloader for extracting audio into fully tagged AAC <code>.m4a</code> files.</strong><br>
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

**Musicerer** is a lightweight, native GUI application designed to download audio from various supported platforms. It takes a queue of URLs, extracts the best available audio, and saves it as a standardized `.m4a` (AAC) file complete with advanced iTunes-style metadata, high-resolution cover art, and smart file organization.

Unlike traditional downloaders that rely on bulky web frameworks, Musicerer is built entirely in Rust using **Iced** and **tiny-skia**. It renders purely on the CPU, requires no GPU, and features a completely custom, borderless dark UI with native window resizing.

- **Convenient to run.** Officially compiled releases come bundled with all necessary media tools, requiring zero setup from the user.
- **Smart & Clean.** Cross-references multiple music databases to fill in missing tags, automatically groups playlist downloads into safe subfolders, and runs isolated temporary downloads.
- **Low-impact.** It keeps preferences in the current user’s config directory and does not modify system registries.

> **Side Note:** The officially bundled Windows executable is large because it embeds full, uncompressed builds of the required media tools (FFmpeg and Deno are the biggest).

---

## Features

- 🎵 **Multi-Platform Support**: Add individual tracks, albums, and playlists from YouTube, SoundCloud, Spotify, and Deezer. Spotify and Deezer act as metadata-first sources, resolving to the highest quality YouTube or SoundCloud audio.
- 🧠 **Intelligent Metadata Resolution**: Automatically cross-references multiple public APIs to fetch and merge the most accurate metadata (canonical artist, album, year, genre, label, ISRC, BPM, and ReplayGain) regardless of the original source.
- 🎨 **Custom UI**: Frameless window, dark monochrome theme with blue accents, dynamic status indicators, and draggable/resizable edges.
- 🖼️ **Advanced Tagging**: Writes standard tags via `audiotags` and advanced freeform M4A atoms (ISRC, Label, BPM) via `mp4ameta`, complete with center-cropped 1:1 cover art.
- 📊 **Queue Management**: Adaptive parallel downloads, real-time progress bars, track previews, and detailed log output.
- 📂 **Smart Organization**: Playlist and album tracks are automatically sorted into their own subfolders, preventing collisions when multiple playlists share the same name.
- 💾 **Persistent Settings**: Saves your output directory, cookies path, and UI preferences automatically.
- 🔓 **DRM Support**: Officially compiled releases include built-in support for extracting DRM-protected SoundCloud songs.

---

## How it works

Musicerer abstracts away the complexity of command-line media extraction.

1. When a URL is added, Musicerer detects the source. YouTube and SoundCloud URLs are prepared for direct audio extraction. Spotify and Deezer URLs act as metadata layers, searching for a matching YouTube or SoundCloud audio source.
2. The metadata engine extracts native information, hydrates hidden metadata, and cross-queries other backends with strict fuzzy-matching to fill in any missing gaps (album, genre, track number, etc.). A precedence merge table ensures the most authoritative data wins.
3. Downloads run concurrently in isolated temporary directories with adaptive worker limits. The scheduler automatically handles rate limits and parallel versions of the same song.
4. `yt-dlp` extracts the audio and converts it to `.m4a` using FFmpeg.
5. The application downloads and center-crops the best available cover art, writes the merged tags to the file, and moves it to the output folder (or a specific playlist subfolder). Temporary data is cleaned up automatically.

---

## What doesn't work: Go+

**SoundCloud Go+ tracks** may return a short preview instead of the full stream when the current session is not entitled to the complete track. When `ffprobe` is available and the extracted SoundCloud audio is shorter than 45 seconds, Musicerer marks the download as failed rather than silently saving a likely preview. This duration check is a heuristic; a genuinely short track can also trigger it.

For SoundCloud items that `yt-dlp` reports as DRM-protected, officially compiled releases include a built-in fallback mechanism to attempt extraction. This is best-effort and does not guarantee access to restricted content. YouTube DRM-protected content is skipped.

---

## Building from source

To build Musicerer, install the Rust toolchain and the required runtime dependencies.

### Prerequisites

- **Rust** 1.95 or newer
- **yt-dlp**, **ffmpeg**/**ffprobe**, and **Deno** (required for some YouTube JavaScript challenges). These must be installed on your system and accessible via your `PATH`.

### Build (plain cargo)

~~~bash
cargo build --release
~~~

The executable will be at `target/release/musicerer`. 

*(Note: Building from source does not include the custom SoundCloud DRM fallback binary; that capability is exclusive to the officially compiled releases.)*

---

## Configuration

You can configure Musicerer directly from the **Settings** tab:
- **Output folder:** Where finished `.m4a` files are saved.
- **Cookies file:** An optional Netscape-format `cookies.txt` file. Highly recommended for bypassing bot-checks or downloading age-gated content.
- **Playlist folders:** Toggle whether playlist tracks are saved in their own subfolders.

These settings are saved automatically to your OS config directory (e.g., `%APPDATA%\Musicerer\config.json` on Windows, `~/.config/Musicerer/config.json` on Linux).

---

## Technical details

### Architecture
- **UI:** Built with `iced` using the `tiny-skia` backend for pure CPU rendering. The UI uses a custom `stack!` layout to overlay invisible `MouseArea` handles on the edges of the window for native-feeling resizing.
- **Metadata Engine:** A strictly typed precedence merge system (`tagging/merge.rs`) resolves conflicts between native `yt-dlp` data, SoundCloud hydration, Spotify/Deezer APIs, and the enrichment layer. Results are cached using `LazyLock` and parallel-safe mutexes to prevent redundant network requests.
- **Concurrent backend:** Downloads, playlist expansion, and metadata enrichment run on bounded worker threads. Workers communicate with the UI through an `Arc<Mutex<Vec<WorkerEvent>>>` inbox, polled by an Iced subscription.
- **Subprocess Management:** External tools are spawned using `std::process::Command`. On Windows, `CREATE_NO_WINDOW` flags are applied to prevent console windows from flashing. On Linux, tools run in their own process groups to allow clean termination of the process tree.

---

## License

Musicerer is licensed under the **GNU General Public License v3.0** (GPLv3).

This is an original, standalone project. It is not a fork or modification of any existing codebase. By using, modifying, or redistributing this project, you agree to the terms of the GPLv3.

The full license text is in [`LICENSE`](LICENSE).

---

## Credits

- **[Iced](https://github.com/iced-rs/iced)**: The cross-platform GUI library for Rust.
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)**: The backbone of the media extraction process.
- **[FFmpeg](https://ffmpeg.org/)**: For audio transcoding and thumbnail conversion.
- **[audiotags](https://github.com/ttscoff/audiotags)** & **[mp4ameta](https://github.com/jdecourcy/mp4ameta)**: For reading and writing standard and advanced M4A metadata.
- **[tiny-skia](https://github.com/RazrFalcon/tiny-skia)**: The pure-Rust CPU rasteriser.

---

<p align="center">
  <em>If Musicerer makes your music library better, consider starring the repo ⭐</em>
</p>