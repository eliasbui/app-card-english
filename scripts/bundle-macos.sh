#!/bin/sh
# Builds "dist/Card English.app": the release binary, the icon and the microphone permission text.
#
#   scripts/bundle-macos.sh               # for this Mac
#   UNIVERSAL=1 scripts/bundle-macos.sh   # Apple silicon and Intel in one binary (what a release ships)
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="dist/Card English.app"

if [ "${UNIVERSAL:-}" = 1 ]; then
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    BIN=$(mktemp -d)/card-english
    lipo -create -output "$BIN" target/aarch64-apple-darwin/release/card-english target/x86_64-apple-darwin/release/card-english
else
    cargo build --release
    BIN=target/release/card-english
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/card-english"

ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z "$s" "$s" assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat >"$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Card English</string>
    <key>CFBundleDisplayName</key><string>Card English</string>
    <key>CFBundleIdentifier</key><string>com.eliasbui.cardenglish</string>
    <key>CFBundleExecutable</key><string>card-english</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Card English nghe bạn đọc từ để chấm điểm phát âm. Âm thanh chỉ xử lý trên máy.</string>
</dict>
</plist>
EOF

# An ad-hoc signature, so that macOS remembers the microphone permission between launches.
codesign --force --deep --sign - "$APP"
echo "$APP ($VERSION)"
