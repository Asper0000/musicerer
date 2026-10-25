#!/usr/bin/env bash
# Builds dist/linux/deb/musicerer_<version>_<arch>.deb from the staged payload. Sourced by build.sh.

package_deb() {
    need dpkg-deb "Install it with: sudo apt install dpkg"
    local out_dir="$DIST_LINUX/deb"
    local out="$out_dir/${APP_ID}_${VERSION}_${DEB_ARCH}.deb"
    local root="$WORK/deb"

    step "Packaging .deb"
    mkdir -p "$out_dir" && rm -f "$out"
    stage_payload "$root"
    mv "$root/usr/share/doc/$APP_ID/LICENSE" "$root/usr/share/doc/$APP_ID/copyright"

    SIZE_KB="$(du -sk "$root" | cut -f1)"
    install -d "$root/DEBIAN"
    render "$PKG_DIR/templates/deb-control.in" "$root/DEBIAN/control"

    dpkg-deb --root-owner-group -Zxz --build "$root" "$out" >/dev/null
    ok "$out  ($(human_size "$out"))"
}
