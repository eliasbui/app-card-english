#!/bin/sh
# Builds the Windows app here, in Docker, and zips it as dist/CardEnglish-<version>-windows-x64.zip with
# CardEnglish.exe in it. It is cross-compiled for MSVC with cargo-xwin: the sherpa-onnx libraries that the app links
# on Windows are built by MSVC, so MinGW cannot link them. Nothing of it is run here, so the Windows tests are not.
#
#   scripts/package-windows.sh
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ZIP="CardEnglish-$VERSION-windows-x64.zip"

docker info >/dev/null 2>&1 || { echo "start Docker first; it builds the Windows app" >&2; exit 1; }
mkdir -p dist

# clang runs as clang-cl for the C of sqlite and oniguruma. The Windows SDK is kept in the image. onnxruntime asks for
# PathCch.lib, which the SDK names pathcch.lib, and Linux tells the two apart.
docker build -q -t cardenglish-win - >/dev/null <<'EOF'
FROM rust:1-trixie
RUN apt-get update && apt-get install -y --no-install-recommends clang lld llvm zip \
    && apt-get clean \
    && rustup target add x86_64-pc-windows-msvc \
    && cargo install cargo-xwin --locked \
    && ln -s "$(command -v clang)" /usr/local/bin/clang-cl \
    && XWIN_ACCEPT_LICENSE=1 cargo xwin cache xwin \
    && ln -s pathcch.lib /root/.cache/cargo-xwin/xwin/sdk/lib/um/x86_64/PathCch.lib
EOF

# crt-static, because the sherpa-onnx libraries for Windows use the static C runtime and the C of sqlite has to agree
# with them; it also leaves no Visual C++ redistributable to install. ponytail: 2 jobs fit the 3.5 GB that Docker
# Desktop gives by default, four rustc processes at once are killed for memory; raise both when Docker has more.
docker run --rm \
    -v "$PWD":/src:ro -v "$PWD/dist":/out \
    -v cardenglish-win-target:/target -v cardenglish-win-cargo:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS=2 -e RUSTFLAGS="-C target-feature=+crt-static" \
    -w /src cardenglish-win sh -euc '
        cargo xwin build --release --locked --target x86_64-pc-windows-msvc
        cp /target/x86_64-pc-windows-msvc/release/card-english.exe /tmp/CardEnglish.exe
        rm -f /out/'"$ZIP"'
        zip -qj /out/'"$ZIP"' /tmp/CardEnglish.exe
    '
echo "dist/$ZIP"
