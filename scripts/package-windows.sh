#!/bin/sh
# Builds the Windows app here and zips it as dist/CardEnglish-<version>-windows-x64.zip with CardEnglish.exe in it.
# It is cross-compiled for MSVC with cargo-xwin, which fetches the MSVC CRT and the Windows SDK once into its cache:
# the sherpa-onnx libraries that the app links on Windows are built by MSVC, so MinGW cannot link them. Nothing of it
# is run here, so the Windows tests are not.
#
#   scripts/package-windows.sh
#   CHECK=1 scripts/package-windows.sh   # only checks the tools, so that a release stops before it builds anything
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ZIP="CardEnglish-$VERSION-windows-x64.zip"

# Homebrew's LLVM is keg-only, so its bin goes first on the PATH of this script alone: clang-cl compiles the C of
# sqlite and oniguruma, llvm-lib archives it, llvm-rc puts the icon in the exe, and lld-link (a formula of its own)
# links. Apple's clang has none of these.
LLVM=$(brew --prefix llvm 2>/dev/null || :)/bin
LLD=$(brew --prefix lld 2>/dev/null || :)/bin
[ -x "$LLVM/clang-cl" ] && [ -x "$LLVM/llvm-lib" ] && [ -x "$LLVM/llvm-rc" ] && [ -x "$LLD/lld-link" ] ||
    { echo "the Windows app needs Homebrew's LLVM; install it once: brew install llvm lld" >&2; exit 1; }
command -v cargo-xwin >/dev/null ||
    { echo "the Windows app needs cargo-xwin; install it once: cargo install --locked cargo-xwin" >&2; exit 1; }
rustup target list --installed | grep -qx x86_64-pc-windows-msvc ||
    { echo "the Windows app needs Rust for Windows; add it once: rustup target add x86_64-pc-windows-msvc" >&2; exit 1; }
[ "${CHECK:-}" = 1 ] && exit 0
PATH="$LLVM:$LLD:$PATH"

# crt-static, because the sherpa-onnx libraries for Windows use the static C runtime and the C of sqlite has to agree
# with them; it also leaves no Visual C++ redistributable to install. /ignore:4099 quiets the linker's hundred
# warnings that Microsoft's CRT ships without its PDB files. The OAuth client of the Google Drive sync goes in from
# this shell's environment, when it is set there. The Windows SDK names it pathcch.lib where onnxruntime asks for
# PathCch.lib, which only a case-sensitive disk tells apart, and a Mac's is not.
RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/ignore:4099" \
    cargo xwin build --release --locked --target x86_64-pc-windows-msvc
OUT=${CARGO_TARGET_DIR:-target}/x86_64-pc-windows-msvc/release
cp "$OUT/card-english.exe" "$OUT/CardEnglish.exe"
mkdir -p dist
rm -f "dist/$ZIP"
zip -qj "dist/$ZIP" "$OUT/CardEnglish.exe"
echo "dist/$ZIP"
