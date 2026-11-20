#!/usr/bin/env bash
# Package a built Linux binary as a .deb and a .tar.gz.
#   scripts/package-linux.sh [path/to/WeeklyScheduler] [deb-arch]
# deb-arch defaults to this machine's (amd64, arm64, ...). Output goes to dist/.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${1:-target/release/WeeklyScheduler}"
ARCH="${2:-$(dpkg --print-architecture 2>/dev/null || uname -m)}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
case "$ARCH" in amd64|x86_64) TAR_ARCH=x86_64 ;; arm64|aarch64) TAR_ARCH=aarch64 ;; *) TAR_ARCH="$ARCH" ;; esac
mkdir -p dist
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ── .tar.gz: run from the folder or ./install.sh for the current user
T="$WORK/weekly-scheduler-$VERSION"
mkdir -p "$T"
install -m 755 "$BIN" "$T/weekly-scheduler"
install -m 644 assets/icon.png "$T/weekly-scheduler.png"
install -m 644 packaging/linux/weekly-scheduler.desktop "$T/"
install -m 755 packaging/linux/install.sh "$T/"
install -m 644 packaging/linux/README.txt "$T/"
tar -C "$WORK" -czf "dist/weekly-scheduler-$VERSION-linux-$TAR_ARCH.tar.gz" "weekly-scheduler-$VERSION"

# ── .deb: system-wide install with the package manager
if command -v dpkg-deb >/dev/null; then
    D="$WORK/deb"
    install -Dm 755 "$BIN" "$D/usr/bin/weekly-scheduler"
    install -Dm 644 packaging/linux/weekly-scheduler.desktop "$D/usr/share/applications/weekly-scheduler.desktop"
    install -Dm 644 assets/icon.png "$D/usr/share/icons/hicolor/256x256/apps/weekly-scheduler.png"
    install -Dm 644 packaging/linux/README.txt "$D/usr/share/doc/weekly-scheduler/README.txt"
    # the newest glibc symbol the program uses is the oldest glibc it runs on
    GLIBC="$( (objdump -T "$BIN" 2>/dev/null || true) | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/GLIBC_//' | sort -V | tail -1)"
    GLIBC="${GLIBC:-2.31}"
    SIZE="$(du -sk "$D/usr" | cut -f1)"
    mkdir -p "$D/DEBIAN"
    cat > "$D/DEBIAN/control" <<EOF
Package: weekly-scheduler
Version: $VERSION
Section: education
Priority: optional
Architecture: $ARCH
Depends: libc6 (>= $GLIBC)
Recommends: xdg-utils
Installed-Size: $SIZE
Maintainer: Teacher Resource <96256497+HandyFox@users.noreply.github.com>
Homepage: https://github.com/HandyFox/weekly-scheduler-desktop
Description: Weekly Assignment Scheduler
 Plan each week in a grid and print a sheet for every student plus a
 teacher copy. Opens in your web browser and runs on this computer only.
EOF
    dpkg-deb --root-owner-group --build "$D" "dist/weekly-scheduler_${VERSION}_${ARCH}.deb" >/dev/null
fi
ls -la dist/
