#!/usr/bin/env bash
set -euo pipefail

# Build a macOS .app bundle for the GUI binary using cargo-bundle.
# If cargo-bundle is not installed, print a hint and skip without failing the build.

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"

OS_TYPE="${OSTYPE:-}"
if [[ "${OS_TYPE}" != darwin* ]]; then
  echo "[make_macos_app] This script must run on macOS. Skipping." >&2
  exit 0
fi

if ! command -v cargo >/dev/null; then
  echo "[make_macos_app] cargo not found. Install Rust toolchain first." >&2
  exit 1
fi

if ! cargo bundle -V >/dev/null 2>&1; then
  echo "[make_macos_app] cargo-bundle is not installed."
  echo "  Install: cargo install cargo-bundle"
  echo "  Skipping .app bundling for now."
  exit 0
fi

echo "[make_macos_app] Bundling .app with cargo-bundle..."
cargo bundle --release --bin hootvoice

# Resolve actual output path from cargo-bundle (handle case differences)
APP_BASE="$ROOT_DIR/target/release/bundle/osx"
APP_PATH=""
for candidate in "HootVoice.app" "hootvoice.app"; do
  if [[ -d "$APP_BASE/$candidate" ]]; then
    APP_PATH="$APP_BASE/$candidate"
    break
  fi
done

if [[ -z "$APP_PATH" ]]; then
  echo "[make_macos_app] .app bundle not found under $APP_BASE" >&2
  echo "[make_macos_app] Contents of $APP_BASE:" >&2
  ls -la "$APP_BASE" >&2 || true
  exit 1
fi

# Copy runtime resources next to the executable inside the .app
MACOS_DIR="$APP_PATH/Contents/MacOS"
mkdir -p "$MACOS_DIR"
if [[ -d "$ROOT_DIR/sounds" ]]; then
  rsync -a --delete "$ROOT_DIR/sounds/" "$MACOS_DIR/sounds/"
fi

PLIST="$APP_PATH/Contents/Info.plist"
if [[ -f "$PLIST" ]] && command -v /usr/libexec/PlistBuddy >/dev/null 2>&1; then
  # Privacy usage description (microphone)
  /usr/libexec/PlistBuddy -c "Print :NSMicrophoneUsageDescription" "$PLIST" >/dev/null 2>&1 || \
  /usr/libexec/PlistBuddy -c "Add :NSMicrophoneUsageDescription string Microphone access is required for voice input." "$PLIST" || true
  # Apple Events usage description (Automation prompt message)
  /usr/libexec/PlistBuddy -c "Print :NSAppleEventsUsageDescription" "$PLIST" >/dev/null 2>&1 || \
  /usr/libexec/PlistBuddy -c "Add :NSAppleEventsUsageDescription string Controls other apps (System Events) to enable auto‑paste." "$PLIST" || true

  # Ensure bundle metadata is consistent
  /usr/libexec/PlistBuddy -c "Set :CFBundleName HootVoice" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundleName string HootVoice" "$PLIST" || true
  /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName HootVoice" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundleDisplayName string HootVoice" "$PLIST" || true
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier com.hootvoice.HootVoice" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string com.hootvoice.HootVoice" "$PLIST" || true
  /usr/libexec/PlistBuddy -c "Set :LSMinimumSystemVersion 10.15" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :LSMinimumSystemVersion string 10.15" "$PLIST" || true

  # App icon
  RES_DIR="$APP_PATH/Contents/Resources"
  mkdir -p "$RES_DIR"
  if [[ -f "$ROOT_DIR/packaging/icons/hootvoice.icns" ]]; then
    install -m 0644 "$ROOT_DIR/packaging/icons/hootvoice.icns" "$RES_DIR/AppIcon.icns"
    /usr/libexec/PlistBuddy -c "Set :CFBundleIconFile AppIcon" "$PLIST" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string AppIcon" "$PLIST" || true
  fi

  # Remove legacy/undesired keys that may cause odd behaviors
  /usr/libexec/PlistBuddy -c "Delete :LSRequiresCarbon" "$PLIST" >/dev/null 2>&1 || true
fi

mkdir -p "$DIST_DIR"
rsync -a --delete "$APP_PATH" "$DIST_DIR/"
echo "✅ macOS app created: $DIST_DIR/$(basename "$APP_PATH")"

# Optional: codesign the .app here so that the DMG (built later) contains a signed app
if command -v xcrun >/dev/null 2>&1; then
  # Load signing env if present
  if [[ -f "$ROOT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$ROOT_DIR/.env"
    set +a
  elif [[ -f "$ROOT_DIR/.secure/set_env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$ROOT_DIR/.secure/set_env.sh"
  fi

  if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    echo "[make_macos_app] Codesigning app before DMG ..."
    codesign --force --deep --timestamp --options runtime \
             --entitlements "$ROOT_DIR/packaging/macos/entitlements.plist" \
             -s "$MACOS_SIGN_IDENTITY" "$DIST_DIR/$(basename "$APP_PATH")" || true
    codesign --verify --deep --strict -v "$DIST_DIR/$(basename "$APP_PATH")" || true
  fi
fi
