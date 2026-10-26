#!/bin/sh
# Bumps the version, tags it and pushes; the release workflow then builds and publishes the app.
#
#   scripts/release.sh 0.2.0
set -eu
VERSION=${1:?usage: scripts/release.sh <version, such as 0.2.0>}
cd "$(dirname "$0")/.."

echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "a version is three numbers, such as 0.2.0" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit your changes first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null && { echo "v$VERSION is already tagged" >&2; exit 1; }

sed -i.bak "1,/^version = /s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml && rm Cargo.toml.bak
cargo update --workspace --offline -q
git commit -q -m "Release $VERSION" Cargo.toml Cargo.lock
git tag "v$VERSION"
git push origin HEAD "v$VERSION"
echo "pushed v$VERSION; the release workflow builds and publishes it"
