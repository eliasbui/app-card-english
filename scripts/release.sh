#!/bin/sh
# Releases a version: bumps it, builds the macOS app (Apple silicon and Intel in one) and the Windows app (x64 and
# arm64) here, tags and pushes, publishes the GitHub release with the three zips, and points the Homebrew cask at the
# macOS one.
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
CHECK=1 scripts/package-windows.sh
# The Google Drive sync signs in with the app's OAuth client, baked in at build time; without it the apps still
# build, with no Google sign-in. Exported, so that cargo reads them.
OAUTH="$HOME/.config/card-english/google-oauth.env"
if [ -f "$OAUTH" ]; then set -a; . "$OAUTH"; set +a; fi
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
X64ZIP="dist/CardEnglish-$VERSION-windows-x64.zip"
ARM64ZIP="dist/CardEnglish-$VERSION-windows-arm64.zip"

# Releasing the version Cargo.toml already has leaves nothing to commit.
git diff --quiet Cargo.toml Cargo.lock || git commit -q -m "Release $VERSION" Cargo.toml Cargo.lock
git tag "v$VERSION"
# To main whatever branch this runs on, so that a release can be made from a worktree while main is checked out
# elsewhere; a push that is not a fast-forward of main is refused.
git push origin HEAD:main "v$VERSION"

gh release create "v$VERSION" "$ZIP" "$X64ZIP" "$ARM64ZIP" --title "v$VERSION" --generate-notes
echo "published v$VERSION with the macOS and Windows zips"

# What the app reads to find this release (src/update.rs): the API's answer in shape, fetched through github.com's
# download links, which the API's limit of sixty calls an hour for a whole network does not touch.
asset() {
    printf '{"name": "%s", "size": %s, "digest": "sha256:%s", "browser_download_url": "https://github.com/eliasbui/app-card-english/releases/download/v%s/%s"}' \
        "$(basename "$1")" "$(stat -f %z "$1")" "$(shasum -a 256 "$1" | cut -d ' ' -f 1)" "$VERSION" "$(basename "$1")"
}
NOTES=$(gh release view "v$VERSION" --json body --jq '.body | tojson')
printf '{"tag_name": "v%s", "html_url": "https://github.com/eliasbui/app-card-english/releases/tag/v%s", "body": %s, "assets": [%s, %s, %s]}\n' \
    "$VERSION" "$VERSION" "$NOTES" "$(asset "$ZIP")" "$(asset "$X64ZIP")" "$(asset "$ARM64ZIP")" > dist/latest.json
gh release upload "v$VERSION" dist/latest.json

scripts/update-cask.sh "$VERSION" "$ZIP" ||
    echo "the Homebrew cask was not updated; run scripts/update-cask.sh $VERSION $ZIP once the tap can be pushed to" >&2
