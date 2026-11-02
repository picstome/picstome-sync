#!/usr/bin/env bash
set -euo pipefail

# Build the current platform with the local toolchain and upload the result to a
# draft GitHub release, without spending GitHub Actions minutes.
#
# Usage:
#   scripts/release-local.sh                 # version from tauri.conf.json
#   scripts/release-local.sh v0.2.0          # explicit tag
#   scripts/release-local.sh --no-build      # upload existing local bundles
#
# On macOS this builds a universal DMG. On Linux it builds deb + AppImage.
# Windows is intentionally not handled here; final releases use release.yml.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUILD=1
TAG=""
for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    -*) echo "Unknown option: $arg" >&2; exit 1 ;;
    *) TAG="$arg" ;;
  esac
done

VERSION="$(python3 - <<'PY'
import json
print(json.load(open("src-tauri/tauri.conf.json"))["version"])
PY
)"
TAG="${TAG:-v$VERSION}"

OS="$(uname -s)"
case "$OS" in
  Darwin)
    PLATFORM="macos"
    ARTIFACTS=(target/universal-apple-darwin/release/bundle/dmg/*.dmg)
    ;;
  Linux)
    PLATFORM="linux"
    ARTIFACTS=(target/release/bundle/deb/*.deb target/release/bundle/appimage/*.AppImage)
    ;;
  *)
    echo "Unsupported local platform: $OS (use release.yml for Windows/final releases)" >&2
    exit 1
    ;;
esac

if [[ "$BUILD" == "1" ]]; then
  echo "Building Picstome Sync $VERSION for $PLATFORM..."
  if [[ "$PLATFORM" == "macos" ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    npx --yes @tauri-apps/cli@2 build --target universal-apple-darwin
  else
    npx --yes @tauri-apps/cli@2 build
  fi
fi

echo "Checking draft release $TAG..."
if ! gh release view "$TAG" >/dev/null 2>&1; then
  gh release create "$TAG" \
    --draft \
    --title "Picstome Sync $TAG" \
    --notes "Local test build for $PLATFORM ($VERSION). Built and uploaded without GitHub Actions minutes."
fi

echo "Uploading $PLATFORM artifacts..."
UPLOAD_DIR="target/local-release"
rm -rf "$UPLOAD_DIR"
mkdir -p "$UPLOAD_DIR"
for file in "${ARTIFACTS[@]}"; do
  base="$(basename "$file")"
  # Keep asset names stable across Actions and local builds.
  normalized="${base// /.}"
  cp "$file" "$UPLOAD_DIR/$normalized"
done
gh release upload "$TAG" "$UPLOAD_DIR"/* --clobber
rm -rf "$UPLOAD_DIR"

echo "Done. Draft release: $(gh release view "$TAG" --json url --jq .url)"
