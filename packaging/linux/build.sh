#!/usr/bin/env bash
# Linux packaging entry point. Normally started by build.bat through WSL, but works on any Linux box.
#
#   packaging/linux/build.sh [appimage] [deb] [rpm]     (no argument = all three)
#
# Needs: a Linux Rust toolchain, the five binaries in ./linux_tools, and per format:
#   deb: dpkg-deb    rpm: rpmbuild    appimage: curl (to fetch appimagetool once)
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
source "$PKG_DIR/appimage.sh"
source "$PKG_DIR/deb.sh"
source "$PKG_DIR/rpm.sh"

targets=("$@")
((${#targets[@]})) || targets=(appimage deb rpm)
for t in "${targets[@]}"; do
    case "$t" in appimage|deb|rpm) ;; *) die "Unknown Linux target '$t' (use appimage, deb or rpm)." ;; esac
done

# Fail fast, before the slow compile.
check_tools
require_linux_cargo
check_build_deps
for t in "${targets[@]}"; do
    case "$t" in
        deb) need dpkg-deb "Install it with: sudo apt install dpkg" ;;
        rpm) need rpmbuild "Install it with: sudo apt install rpm   (Fedora: sudo dnf install rpm-build)" ;;
        appimage) [[ -x "$TOOLS_SRC/appimagetool" ]] || need curl "Install it with: sudo apt install curl" ;;
    esac
done

WORK="$(mktemp -d)"   # native Linux filesystem: fast, and keeps file modes intact (unlike /mnt/c)
trap 'rm -rf "$WORK"' EXIT

build_binary
for t in "${targets[@]}"; do "package_$t"; done

step "Done"
say "Output: ${DIST_LINUX#"$ROOT"/}/"
