#!/usr/bin/env bash
# Cross-compile the Windows installer on Linux.
# Needs: rustup target add x86_64-pc-windows-gnu, mingw-w64 and nsis
#   (Debian/Ubuntu: sudo apt install mingw-w64 nsis)
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
makensis -V2 "-DVERSION=$VERSION" "-DEXE=../target/x86_64-pc-windows-gnu/release/WeeklyScheduler.exe" \
    "-DOUTDIR=../dist" installer/installer.nsi
ls -la dist/
