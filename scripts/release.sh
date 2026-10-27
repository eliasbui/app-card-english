#!/bin/sh
# Releases a version: bumps it, builds the macOS app here (Apple silicon and Intel in one), tags and pushes,
# publishes the GitHub release with the macOS zip, and points the Homebrew cask at it. The release workflow then
# builds the Windows zip and adds it to the same release.
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

sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml && rm Cargo.toml.bak
cargo update --workspace --offline -q

# Built and tested before anything is pushed, so that a broken build never leaves a tag behind.
cargo test --quiet
UNIVERSAL=1 scripts/bundle-macos.sh
ZIP="dist/CardEnglish-$VERSION-macos.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "dist/Card English.app" "$ZIP"

# Releasing the version Cargo.toml already has leaves nothing to commit.
git diff --quiet Cargo.toml Cargo.lock || git commit -q -m "Release $VERSION" Cargo.toml Cargo.lock
git tag "v$VERSION"
git push origin HEAD "v$VERSION"

# The workflow may have made the release first; then the zip is added to it.
gh release create "v$VERSION" "$ZIP" --title "v$VERSION" --generate-notes 2>/dev/null ||
    gh release upload "v$VERSION" "$ZIP" --clobber
echo "published v$VERSION with the macOS zip; the release workflow adds the Windows zip"

scripts/update-cask.sh "$VERSION" "$ZIP" ||
    echo "the Homebrew cask was not updated; run scripts/update-cask.sh $VERSION $ZIP once the tap can be pushed to" >&2
