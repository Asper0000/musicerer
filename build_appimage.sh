#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Musicerer"
APP_DIR="AppDir"
APPIMAGE_PATH="${APP_NAME}-x86_64.AppImage"

say() { printf '[%s] %s\n' "$APP_NAME" "$*"; }
fail() { printf '[%s] ERROR: %s\n' "$APP_NAME" "$*" >&2; exit 1; }

command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo is required to build the application."
command -v wget >/dev/null 2>&1 || fail "wget is required to download the AppImage tools."
command -v tar >/dev/null 2>&1 || fail "tar is required."
command -v yt-dlp >/dev/null 2>&1 || fail "Install yt-dlp and make it available on PATH."
command -v ffprobe >/dev/null 2>&1 || fail "Install ffprobe and make it available on PATH."

say "Building release binary..."
cargo build --release

say "Preparing AppDir..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/usr/bin/assets" "$APP_DIR/usr/share/icons/hicolor/scalable/apps"
cp target/release/musicerer "$APP_DIR/usr/bin/"
cp assets/icon.ico "$APP_DIR/usr/bin/assets/icon.ico"

cat > "$APP_DIR/AppRun" <<'APPRUN'
#!/usr/bin/env bash
set -e
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/musicerer" "$@"
APPRUN
chmod +x "$APP_DIR/AppRun"

# Bundle ffmpeg with the app; yt-dlp and ffprobe remain system prerequisites.
if [[ ! -f ffmpeg ]]; then
    say "Downloading static FFmpeg..."
    wget -O ffmpeg-release-amd64-static.tar.xz \
        https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz
    extracted_dir="$(tar -tJf ffmpeg-release-amd64-static.tar.xz | sed -n '1s,/.*,,p')"
    [[ -n "$extracted_dir" ]] || fail "Could not determine the FFmpeg archive directory."
    tar -xJf ffmpeg-release-amd64-static.tar.xz
    [[ -f "$extracted_dir/ffmpeg" ]] || fail "The FFmpeg archive did not contain the expected executable."
    cp "$extracted_dir/ffmpeg" ffmpeg
    chmod +x ffmpeg
    rm -rf "$extracted_dir" ffmpeg-release-amd64-static.tar.xz
fi
cp ffmpeg "$APP_DIR/usr/bin/ffmpeg"
chmod +x "$APP_DIR/usr/bin/ffmpeg"

say "Installing the project icon..."
cp assets/icon.svg "$APP_DIR/usr/share/icons/hicolor/scalable/apps/musicerer.svg"
ln -sf usr/share/icons/hicolor/scalable/apps/musicerer.svg "$APP_DIR/musicerer.svg"
ln -sf usr/share/icons/hicolor/scalable/apps/musicerer.svg "$APP_DIR/.DirIcon"

cat > "$APP_DIR/musicerer.desktop" <<'DESKTOP'
[Desktop Entry]
Name=Musicerer
Exec=musicerer
Icon=musicerer
Type=Application
Categories=AudioVideo;Audio;Player;
Terminal=false
DESKTOP

if [[ ! -x appimagetool ]]; then
    say "Downloading appimagetool..."
    wget -O appimagetool \
        https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x appimagetool
fi

say "Packaging AppImage..."
ARCH=x86_64 ./appimagetool "$APP_DIR" "$APPIMAGE_PATH"
say "Created $APPIMAGE_PATH"
