#!/usr/bin/env bash
# Shared helpers for the Linux packaging scripts. Sourced, never run directly.

# ---- logging ---------------------------------------------------------------
if [[ -t 1 ]]; then _B=$'\033[1m'; _R=$'\033[31m'; _G=$'\033[32m'; _Y=$'\033[33m'; _0=$'\033[0m'; else _B=; _R=; _G=; _Y=; _0=; fi
step() { printf '\n%s=== %s ===%s\n' "$_B" "$*" "$_0"; }
say()  { printf '  %s\n' "$*"; }
ok()   { printf '  %s%s%s\n' "$_G" "$*" "$_0"; }
warn() { printf '  %swarning:%s %s\n' "$_Y" "$_0" "$*" >&2; }
die()  { printf '\n%serror:%s %s\n' "$_R" "$_0" "$*" >&2; exit 1; }

# ---- layout ----------------------------------------------------------------
PKG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$PKG_DIR/../.." && pwd)"
DIST_LINUX="$ROOT/dist/linux"
TOOLS_SRC="${MUSICERER_LINUX_TOOLS:-$ROOT/linux_tools}"
CARGO_TARGET="$ROOT/target/linux"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/musicerer-build"

APP_ID="musicerer"
APP_NAME="Musicerer"
APP_SUMMARY="Queue downloader that extracts audio into tagged AAC .m4a files"
MAINTAINER="${MAINTAINER:-Musicerer <noreply@example.invalid>}"
# Tools shipped inside every Linux package. Names match src/backend/tools.rs.
TOOLS=(yt-dlp ffmpeg ffprobe deno soundcloud)

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
[[ -n "$VERSION" ]] || die "Could not read the version from Cargo.toml."

case "$(uname -m)" in
    x86_64)        RPM_ARCH=x86_64;  DEB_ARCH=amd64 ;;
    aarch64|arm64) RPM_ARCH=aarch64; DEB_ARCH=arm64 ;;
    *) die "Unsupported CPU architecture: $(uname -m)" ;;
esac

umask 022

# ---- checks ----------------------------------------------------------------
need() { command -v "$1" >/dev/null 2>&1 || die "'$1' is required. $2"; }

is_elf() { [[ "$(head -c4 "$1" 2>/dev/null | od -An -tx1 | tr -d ' \n')" == 7f454c46 ]]; }

# Finds a *Linux* cargo. Inside WSL the Windows PATH is appended to $PATH, so a plain
# `command -v cargo` can silently pick up cargo.exe from /mnt/c, which must not be used.
require_linux_cargo() {
    # shellcheck disable=SC1091
    [[ -r "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
    local c; c="$(command -v cargo || true)"
    case "$c" in
        ""|/mnt/*|*.exe)
            die "No Linux Rust toolchain found${c:+ (only $c, which is the Windows one)}.
       Install it inside WSL:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
    esac
}

# Every tool must exist and be a Linux ELF binary (catches Windows .exe files dropped in by mistake).
check_tools() {
    [[ -d "$TOOLS_SRC" ]] || die "Tools folder not found: $TOOLS_SRC"
    local missing=() t f
    for t in "${TOOLS[@]}"; do
        f="$TOOLS_SRC/$t"
        if [[ ! -f "$f" ]]; then missing+=("$t"); continue; fi
        is_elf "$f" || die "$f is not a Linux executable (ELF). Windows .exe files cannot be packaged here."
    done
    ((${#missing[@]} == 0)) || die "Missing in ${TOOLS_SRC##*/}/: ${missing[*]}"
}

# ---- build -----------------------------------------------------------------
# The GUI stack links against X11/Wayland/xkbcommon dev packages; catch that before a long compile.
check_build_deps() {
    command -v cc >/dev/null 2>&1 && command -v pkg-config >/dev/null 2>&1 \
        || die "A C toolchain and pkg-config are required. Install them with:
       sudo apt install build-essential pkg-config"
    local missing=() lib
    for lib in wayland-client xkbcommon x11 xcursor xrandr xi; do
        pkg-config --exists "$lib" 2>/dev/null || missing+=("$lib")
    done
    ((${#missing[@]} == 0)) || die "Missing development libraries (${missing[*]}). Install them with:
       sudo apt install libx11-dev libxcursor-dev libxrandr-dev libxi-dev libxkbcommon-dev libwayland-dev
       (Fedora: sudo dnf install libX11-devel libXcursor-devel libXrandr-devel libXi-devel libxkbcommon-devel wayland-devel)"
}

build_binary() {
    step "Compiling musicerer (release, $RPM_ARCH)"
    (cd "$ROOT" && cargo build --release --target-dir "$CARGO_TARGET")
    BINARY="$CARGO_TARGET/release/$APP_ID"
    [[ -x "$BINARY" ]] || die "Build finished but $BINARY is missing."
}

# ---- staging ---------------------------------------------------------------
# Lays out the installed file tree once; every package format starts from a copy of it.
#   /usr/bin/musicerer                      -> symlink to the real binary
#   /usr/lib/musicerer/musicerer            real binary
#   /usr/lib/musicerer/tools/<tool>         bundled tools, found by src/backend/tools.rs
#                                           (it looks in "tools" next to the real executable)
#   /usr/share/applications, icons, doc     desktop integration
stage_payload() {
    local root="$1" t
    install -d "$root/usr/bin" "$root/usr/lib/$APP_ID/tools" \
               "$root/usr/share/applications" \
               "$root/usr/share/icons/hicolor/scalable/apps" \
               "$root/usr/share/doc/$APP_ID"
    install -m755 "$BINARY" "$root/usr/lib/$APP_ID/$APP_ID"
    for t in "${TOOLS[@]}"; do
        install -m755 "$TOOLS_SRC/$t" "$root/usr/lib/$APP_ID/tools/$t"
    done
    ln -s "../lib/$APP_ID/$APP_ID" "$root/usr/bin/$APP_ID"
    install -m644 "$PKG_DIR/$APP_ID.desktop" "$root/usr/share/applications/$APP_ID.desktop"
    install -m644 "$ROOT/assets/icon.svg"    "$root/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    install -m644 "$ROOT/LICENSE"            "$root/usr/share/doc/$APP_ID/LICENSE"
}

# Replaces @VERSION@-style placeholders in a template and writes the result.
render() {
    sed -e "s|@VERSION@|$VERSION|g" -e "s|@DEB_ARCH@|$DEB_ARCH|g" -e "s|@RPM_ARCH@|$RPM_ARCH|g" \
        -e "s|@MAINTAINER@|$MAINTAINER|g" -e "s|@SUMMARY@|$APP_SUMMARY|g" \
        -e "s|@SIZE_KB@|${SIZE_KB:-0}|g" -e "s|@STAGE@|${STAGE:-}|g" "$1" > "$2"
}

human_size() { du -h "$1" | cut -f1; }
