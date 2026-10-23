# Musicerer

SoundCloud + YouTube â†’ native AAC `.m4a` queue downloader.
Rust + Iced + tiny-skia (pure CPU rendering, no GPU required).

## What it does

- Accepts a queue of SoundCloud and YouTube URLs.
- Uses `yt-dlp` for extraction and format selection.
- Saves each track as M4A with iTunes-style metadata and a 1:1 cover.
- Skips DRM-protected tracks with a clear status (no DRM circumvention).
- Sequential download with per-track tempdir so the final move is atomic.
- Optional Netscape `cookies.txt` for age-gated or private YouTube content.

## Prerequisites

| Tool   | Why                                       | Install                                   |
| ------ | ----------------------------------------- | ----------------------------------------- |
| Rust   | build                                     | https://rustup.rs                         |
| yt-dlp | extraction                                | `winget install yt-dlp`                   |
| ffmpeg | audio transcode + thumbnail conversion    | `winget install Gyan.FFmpeg`              |
| deno   | YouTube JS challenge solver               | `winget install DenoLand.Deno`            |

Verify:

    rustc --version && cargo --version
    yt-dlp --version
    ffmpeg -version
    deno --version

## Build & run

    cargo run --release

Release profile is tuned for size (`opt-level = "z"`, fat LTO, strip).

## Configuration

The UI exposes three paths at runtime:

- **Output** â€” where finished M4A files land (defaults to `%USERPROFILE%\Downloads\Musicerer`).
- **Cookies** â€” optional `cookies.txt` (Netscape format).
- **Queue** â€” paste one URL per line, press *Add*.

## Layout

    src/main.rs     â€” Iced UI, state machine, event polling
    src/backend.rs  â€” yt-dlp subprocess, metadata write, cover crop
    src/theme.rs    â€” monochrome dark palette

## Notes on DRM

DRM-protected tracks (SoundCloud Go/Go+, some YouTube Music) are
detected and skipped. `yt-dlp` does not bypass DRM, and neither does
this tool. If a track is DRM-protected it cannot be saved as a
portable audio file by any lawful desktop tool.