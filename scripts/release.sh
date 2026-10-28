#!/bin/sh
# Releases a version: bumps it, builds the macOS app (Apple silicon and Intel in one) and the Windows app here,
# tags and pushes, publishes the GitHub release with both zips, and points the Homebrew cask at the macOS one.
#
#   scripts/release.sh 0.2.0
set -eu
VERSION=${1:?usage: scripts/release.sh <version, such as 0.2.0>}
cd "$(dirname "$0")/.."

echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "a version is three numbers, such as 0.2.0" >&2; exit 1; }
[ "$(uname)" = Darwin ] || { echo "the macOS app is built here, so a release is made on a Mac" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit your changes first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null && { echo "v$VERSION is already tagged" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "sign in to GitHub first: gh auth login" >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "start Docker first; it builds the Windows app" >&2; exit 1; }
# The Google Drive sync signs in with the app's OAuth client, baked in at build time; without it the apps still
# build, with no Google sign-in.
[ -n "${CARD_ENGLISH_GOOGLE_CLIENT_ID:-}" ] && [ -n "${CARD_ENGLISH_GOOGLE_CLIENT_SECRET:-}" ] ||
    echo "warning: CARD_ENGLISH_GOOGLE_CLIENT_ID or CARD_ENGLISH_GOOGLE_CLIENT_SECRET is not set; this release has no Google Drive sync" >&2

sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml && rm Cargo.toml.bak
cargo update --workspace --offline -q

# Built and tested before anything is pushed, so that a broken build never leaves a tag behind.
cargo test --quiet
UNIVERSAL=1 scripts/bundle-macos.sh
ZIP="dist/CardEnglish-$VERSION-macos.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "dist/Card English.app" "$ZIP"
scripts/package-windows.sh
WINZIP="dist/CardEnglish-$VERSION-windows-x64.zip"

# Releasing the version Cargo.toml already has leaves nothing to commit.
git diff --quiet Cargo.toml Cargo.lock || git commit -q -m "Release $VERSION" Cargo.toml Cargo.lock
git tag "v$VERSION"
git push origin HEAD "v$VERSION"

gh release create "v$VERSION" "$ZIP" "$WINZIP" --title "v$VERSION" --generate-notes
echo "published v$VERSION with the macOS and Windows zips"

scripts/update-cask.sh "$VERSION" "$ZIP" ||
    echo "the Homebrew cask was not updated; run scripts/update-cask.sh $VERSION $ZIP once the tap can be pushed to" >&2
