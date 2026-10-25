#!/usr/bin/env bash
# Builds dist/linux/rpm/musicerer-<version>-1.<arch>.rpm from the staged payload. Sourced by build.sh.

package_rpm() {
    need rpmbuild "Install it with: sudo apt install rpm   (Fedora: sudo dnf install rpm-build)"
    local out_dir="$DIST_LINUX/rpm"
    local top="$WORK/rpm"
    export STAGE="$WORK/rpm-stage"

    step "Packaging .rpm"
    mkdir -p "$out_dir" "$top"/{BUILD,RPMS,SPECS,SRPMS} && rm -f "$out_dir/$APP_ID-$VERSION-"*.rpm
    stage_payload "$STAGE"
    render "$PKG_DIR/templates/rpm.spec.in" "$top/SPECS/$APP_ID.spec"

    rpmbuild -bb "$top/SPECS/$APP_ID.spec" \
        --define "_topdir $top" \
        --define "_binary_payload w7.xzdio" \
        --define "_build_id_links none" \
        --quiet >/dev/null

    local built; built="$(find "$top/RPMS" -name '*.rpm' | head -n1)"
    [[ -n "$built" ]] || die "rpmbuild produced no package."
    cp "$built" "$out_dir/"
    ok "$out_dir/$(basename "$built")  ($(human_size "$built"))"
}
