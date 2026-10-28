#!/bin/sh
# Builds the Windows app here and zips it as dist/CardEnglish-<version>-windows-x64.zip (Intel and AMD PCs) and
# dist/CardEnglish-<version>-windows-arm64.zip (Snapdragon PCs), each with CardEnglish.exe in it.
# It is cross-compiled for MSVC with cargo-xwin, which fetches the MSVC CRT and the Windows SDK once into its cache:
# the sherpa-onnx libraries that the app links on Windows are built by MSVC, so MinGW cannot link them. Nothing of it
# is run here, so the Windows tests are not.
#
#   scripts/package-windows.sh
#   ARCH=arm64 scripts/package-windows.sh   # one of the two, x64 or arm64
#   CHECK=1 scripts/package-windows.sh      # only checks the tools, so that a release stops before it builds anything
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case "${ARCH:-both}" in
    both) ARCHS="x64 arm64" ;;
    x64 | arm64) ARCHS=$ARCH ;;
    *) echo "ARCH is x64 or arm64, or left out for both" >&2; exit 1 ;;
esac
rust_target() { if [ "$1" = arm64 ]; then echo aarch64-pc-windows-msvc; else echo x86_64-pc-windows-msvc; fi; }

# Homebrew's LLVM is keg-only, so its bin goes first on the PATH of this script alone: clang-cl compiles the C of
# sqlite and oniguruma, llvm-lib archives it, llvm-rc puts the icon in the exe, and lld-link (a formula of its own)
# links. Apple's clang has none of these.
LLVM=$(brew --prefix llvm 2>/dev/null || :)/bin
LLD=$(brew --prefix lld 2>/dev/null || :)/bin
[ -x "$LLVM/clang-cl" ] && [ -x "$LLVM/llvm-lib" ] && [ -x "$LLVM/llvm-rc" ] && [ -x "$LLD/lld-link" ] ||
    { echo "the Windows app needs Homebrew's LLVM; install it once: brew install llvm lld" >&2; exit 1; }
command -v cargo-xwin >/dev/null ||
    { echo "the Windows app needs cargo-xwin; install it once: cargo install --locked cargo-xwin" >&2; exit 1; }
for arch in $ARCHS; do
    rustup target list --installed | grep -qx "$(rust_target "$arch")" ||
        { echo "the Windows app needs Rust for Windows; add it once: rustup target add $(rust_target "$arch")" >&2; exit 1; }
done
[ "${CHECK:-}" = 1 ] && exit 0
PATH="$LLVM:$LLD:$PATH"

# crt-static, because the sherpa-onnx libraries for Windows use the static C runtime and the C of sqlite has to agree
# with them; it also leaves no Visual C++ redistributable to install. /ignore:4099 quiets the linker's hundred
# warnings that Microsoft's CRT ships without its PDB files. The OAuth client of the Google Drive sync goes in from
# this shell's environment, when it is set there. The Windows SDK names it pathcch.lib where onnxruntime asks for
# PathCch.lib, which only a case-sensitive disk tells apart, and a Mac's is not.
export RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/ignore:4099"
mkdir -p dist
for arch in $ARCHS; do
    target=$(rust_target "$arch")
    # The sherpa-onnx libraries for ARM64 call parts of Microsoft's C++ standard library that the CRT cargo-xwin
    # fetches by default (MSVC 14.44) does not have for ARM64, and the link fails on __std_find_trivial_1 and the like;
    # Visual Studio 2026's CRT (14.51) has them. cargo-xwin keeps one version per cache folder, so that one has a
    # folder of its own, with the ARM64 half only. x64 keeps the CRT it has always been built with.
    (
        if [ "$arch" = arm64 ]; then
            export XWIN_VERSION=18 XWIN_ARCH=aarch64 XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-$HOME/Library/Caches/cargo-xwin}-18"
        fi
        cargo xwin build --release --locked --target "$target"
    )
    out=${CARGO_TARGET_DIR:-target}/$target/release
    name="CardEnglish-$VERSION-windows-$arch.zip"
    cp "$out/card-english.exe" "$out/CardEnglish.exe"
    rm -f "dist/$name"
    zip -qj "dist/$name" "$out/CardEnglish.exe"
    echo "dist/$name"
done
