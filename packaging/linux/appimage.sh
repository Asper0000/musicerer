#!/usr/bin/env bash
# Builds dist/linux/appimage/Musicerer-<version>-<arch>.AppImage from the staged payload. Sourced by build.sh.

# Downloads and caches appimagetool as a build-time dependency; it is not shipped with Musicerer.
get_appimagetool() {
    APPIMAGETOOL="$CACHE_DIR/appimagetool-$RPM_ARCH.AppImage"
    if [[ ! -x "$APPIMAGETOOL" ]]; then
        need curl "Install it with: sudo apt install curl"
        say "Downloading appimagetool (cached in $CACHE_DIR)..."
        mkdir -p "$CACHE_DIR"
        curl -fsSL -o "$APPIMAGETOOL.part" \
            "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$RPM_ARCH.AppImage" \
            || die "Could not download appimagetool. check your network connection or install appimagetool manually in $CACHE_DIR."
        mv "$APPIMAGETOOL.part" "$APPIMAGETOOL" && chmod +x "$APPIMAGETOOL"
    fi
}

package_appimage() {
    local out_dir="$DIST_LINUX/appimage"
    local out="$out_dir/$APP_NAME-$VERSION-$RPM_ARCH.AppImage"
    local appdir="$WORK/AppDir"

    step "Packaging AppImage"
    mkdir -p "$out_dir" && rm -f "$out"
    get_appimagetool
    stage_payload "$appdir"

    install -m755 "$PKG_DIR/AppRun" "$appdir/AppRun"
    cp "$appdir/usr/share/applications/$APP_ID.desktop" "$appdir/$APP_ID.desktop"
    ln -s "usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" "$appdir/$APP_ID.svg"
    ln -s "usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" "$appdir/.DirIcon"

    # --appimage-extract-and-run: WSL (and containers) usually have no FUSE.
    ARCH="$RPM_ARCH" "$APPIMAGETOOL" --appimage-extract-and-run "$appdir" "$out" >/dev/null 2>"$WORK/appimagetool.log" \
        || { cat "$WORK/appimagetool.log" >&2; die "appimagetool failed."; }
    chmod +x "$out"
    ok "$out  ($(human_size "$out"))"
}
