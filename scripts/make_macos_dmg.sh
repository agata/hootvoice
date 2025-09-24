#!/usr/bin/env bash
set -euo pipefail

# Create a compressed DMG from the built .app in dist/

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"

OS_TYPE="${OSTYPE:-}"
if [[ "${OS_TYPE}" != darwin* ]]; then
  echo "[make_macos_dmg] This script must run on macOS. Skipping." >&2
  exit 0
fi

APP_IN_DIST=""
for candidate in "HootVoice.app" "hootvoice.app"; do
  if [[ -d "$DIST_DIR/$candidate" ]]; then
    APP_IN_DIST="$DIST_DIR/$candidate"
    break
  fi
done

if [[ -z "$APP_IN_DIST" ]]; then
  echo "[make_macos_dmg] .app not found in $DIST_DIR" >&2
  ls -la "$DIST_DIR" || true
  exit 1
fi

VOL_NAME="HootVoice"

# Normalize arch for filename suffix
ARCH_UNAME=$(uname -m)
case "$ARCH_UNAME" in
  x86_64|amd64) OUT_ARCH="x86_64" ;;
  arm64|aarch64) OUT_ARCH="arm64" ;;
  *) OUT_ARCH="$ARCH_UNAME" ;;
esac

DMG_PATH="$DIST_DIR/HootVoice-macos-${OUT_ARCH}.dmg"
STAGE_DIR="$DIST_DIR/.dmg_stage"

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

echo "[make_macos_dmg] Staging files..."
cp -R "$APP_IN_DIST" "$STAGE_DIR/"
ln -s /Applications "$STAGE_DIR/Applications"

echo "[make_macos_dmg] Creating DMG at $DMG_PATH ..."
hdiutil create -volname "$VOL_NAME" -srcfolder "$STAGE_DIR" -ov -format UDZO -imagekey zlib-level=9 "$DMG_PATH"

rm -rf "$STAGE_DIR"
echo "✅ DMG created: $DMG_PATH"
