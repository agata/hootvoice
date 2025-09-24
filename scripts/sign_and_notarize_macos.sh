#!/usr/bin/env bash
set -euo pipefail

# Sign and optionally notarize the .app or .dmg in dist/
# Env vars:
#   MACOS_SIGN_IDENTITY   -> e.g. "Developer ID Application: Your Name (TEAMID)"
#   MACOS_TEAM_ID         -> e.g. ABCDE12345
#   MACOS_NOTARIZE_APPLE_ID -> Apple ID email (for notarytool, when using password)
#   MACOS_NOTARIZE_PWD      -> App-specific password (recommended)
#   MACOS_NOTARIZE_PROFILE  -> Keychain profile name (optional alternative to APPLE_ID/PWD)
#
# Usage:
#   scripts/sign_and_notarize_macos.sh [path_to_app_or_dmg]

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"

OS_TYPE="${OSTYPE:-}"
if [[ "${OS_TYPE}" != darwin* ]]; then
  echo "[sign_and_notarize] This script must run on macOS. Skipping." >&2
  exit 0
fi

TARGET_PATH="${1:-}"
if [[ -z "${TARGET_PATH}" ]]; then
  # Prefer DMG if present (new naming with arch suffix)
  if ls "$DIST_DIR"/HootVoice-macos-*.dmg >/dev/null 2>&1; then
    TARGET_PATH=$(ls "$DIST_DIR"/HootVoice-macos-*.dmg | head -n 1)
  else
    for candidate in "HootVoice.app" "hootvoice.app"; do
      if [[ -d "$DIST_DIR/$candidate" ]]; then
        TARGET_PATH="$DIST_DIR/$candidate"
        break
      fi
    done
  fi
fi

if [[ -z "$TARGET_PATH" ]]; then
  echo "[sign_and_notarize] No .app or .dmg found to sign/notarize." >&2
  exit 1
fi

EXT="${TARGET_PATH##*.}"

sign_app() {
  local app_path="$1"
  if [[ -z "${MACOS_SIGN_IDENTITY:-}" ]]; then
    echo "[sign_and_notarize] MACOS_SIGN_IDENTITY not set. Skipping codesign." >&2
    return 0
  fi

  echo "[sign_and_notarize] Codesigning $app_path ..."
  if [[ -f "$ROOT_DIR/packaging/macos/entitlements.plist" ]]; then
    codesign --force --options runtime --deep --timestamp \
      --entitlements "$ROOT_DIR/packaging/macos/entitlements.plist" \
      -s "$MACOS_SIGN_IDENTITY" "$app_path"
  else
    codesign --force --options runtime --deep --timestamp \
      -s "$MACOS_SIGN_IDENTITY" "$app_path"
  fi

  codesign --verify --deep --strict --verbose=2 "$app_path"
  echo "✅ Signed: $app_path"
}

sign_dmg() {
  local dmg_path="$1"
  if [[ -z "${MACOS_SIGN_IDENTITY:-}" ]]; then
    echo "[sign_and_notarize] MACOS_SIGN_IDENTITY not set. Skipping DMG codesign." >&2
    return 0
  fi
  echo "[sign_and_notarize] Codesigning DMG $dmg_path ..."
  codesign --force --timestamp -s "$MACOS_SIGN_IDENTITY" "$dmg_path"
  echo "✅ Signed: $dmg_path"
}

notarize() {
  local path="$1"
  if [[ -n "${MACOS_NOTARIZE_PROFILE:-}" ]]; then
    echo "[sign_and_notarize] Submitting to notarization with keychain profile: $path"
    xcrun notarytool submit "$path" --keychain-profile "$MACOS_NOTARIZE_PROFILE" --wait
  else
    if [[ -z "${MACOS_NOTARIZE_APPLE_ID:-}" || -z "${MACOS_NOTARIZE_PWD:-}" || -z "${MACOS_TEAM_ID:-}" ]]; then
      echo "[sign_and_notarize] Notarization credentials missing. Skipping notarize." >&2
      return 0
    fi
    echo "[sign_and_notarize] Submitting to notarization: $path"
    xcrun notarytool submit "$path" --apple-id "$MACOS_NOTARIZE_APPLE_ID" --team-id "$MACOS_TEAM_ID" --password "$MACOS_NOTARIZE_PWD" --wait
  fi
  echo "[sign_and_notarize] Stapling ticket..."
  xcrun stapler staple "$path" || true
  # If path is DMG, also staple the .app inside dist if present
  if [[ "$path" == *.dmg ]]; then
    for candidate in "$DIST_DIR/HootVoice.app" "$DIST_DIR/hootvoice.app"; do
      if [[ -d "$candidate" ]]; then
        xcrun stapler staple "$candidate" || true
      fi
    done
  fi
  echo "✅ Notarized: $path"
}

case "$EXT" in
  app)
    sign_app "$TARGET_PATH"
    notarize "$TARGET_PATH"
    ;;
  dmg)
    # Ideally sign the app first if present, then DMG, then notarize DMG
    for candidate in "$DIST_DIR/HootVoice.app" "$DIST_DIR/hootvoice.app"; do
      if [[ -d "$candidate" ]]; then
        sign_app "$candidate"
      fi
    done
    sign_dmg "$TARGET_PATH"
    notarize "$TARGET_PATH"
    ;;
  *)
    echo "[sign_and_notarize] Unsupported target: $TARGET_PATH" >&2
    exit 1
    ;;
esac

echo "Done."
