#!/usr/bin/env bash
set -euo pipefail

# Build an AppImage for the GUI binary (hootvoice) on Linux.
#
# Requirements:
# - appimagetool (auto-downloaded to packaging/tools if missing)
# - bash, coreutils, tar, gzip
#
# Result:
# - dist/HootVoice-linux-<arch>.AppImage

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
TOOLS_DIR="$ROOT_DIR/packaging/tools"
APP_DIR_BASE="$ROOT_DIR/packaging/appimage"
APP_DIR="$APP_DIR_BASE/AppDir"

OS_TYPE="${OSTYPE:-}"
if [[ "${OS_TYPE}" != linux* ]]; then
  echo "[make_appimage] This script must run on Linux. Skipping." >&2
  exit 0
fi

ARCH_UNAME=$(uname -m)
# Normalize output arch (x86_64/arm64) while keeping AppImage tool arch (x86_64/aarch64)
case "$ARCH_UNAME" in
  x86_64|amd64)
    OUT_ARCH="x86_64"
    APPIMAGE_ARCH="x86_64"
    TOOL_SUFFIX="x86_64"
    ;;
  aarch64|arm64)
    OUT_ARCH="arm64"
    APPIMAGE_ARCH="aarch64"
    TOOL_SUFFIX="aarch64"
    ;;
  *)
    OUT_ARCH="$ARCH_UNAME"
    APPIMAGE_ARCH="x86_64"
    TOOL_SUFFIX="x86_64"
    ;;
esac

GUI_BIN="$ROOT_DIR/target/release/hootvoice"
if [[ ! -f "$GUI_BIN" ]]; then
  echo "[make_appimage] GUI binary not found: $GUI_BIN" >&2
  echo "Run: cargo build --release first." >&2
  exit 1
fi

mkdir -p "$DIST_DIR" "$TOOLS_DIR" "$APP_DIR"
rm -rf "$APP_DIR" && mkdir -p "$APP_DIR"

# AppDir skeleton
mkdir -p \
  "$APP_DIR/usr/bin" \
  "$APP_DIR/usr/share/applications" \
  "$APP_DIR/usr/share/icons/hicolor/256x256/apps"

# Binary and runtime resources (place next to the exe)
install -m 0755 "$GUI_BIN" "$APP_DIR/usr/bin/hootvoice"
if [[ -d "$ROOT_DIR/sounds" ]]; then
  cp -a "$ROOT_DIR/sounds" "$APP_DIR/usr/bin/"
fi

# .desktop entry
cat > "$APP_DIR/usr/share/applications/hootvoice.desktop" << 'EOF'
[Desktop Entry]
Type=Application
Name=HootVoice
Comment=High‑accuracy voice input (Whisper)
Exec=hootvoice
Icon=hootvoice
Terminal=false
Categories=AudioVideo;
EOF
cp "$APP_DIR/usr/share/applications/hootvoice.desktop" "$APP_DIR/hootvoice.desktop"

# Icon (use provided one if exists, otherwise a tiny placeholder)
ICON_SRC_PNG="$ROOT_DIR/packaging/icons/hootvoice.png"
ICON_THEMED_PNG="$APP_DIR/usr/share/icons/hicolor/256x256/apps/hootvoice.png"
ICON_TOPLEVEL_PNG="$APP_DIR/hootvoice.png"
if [[ -f "$ICON_SRC_PNG" ]]; then
  install -m 0644 "$ICON_SRC_PNG" "$ICON_THEMED_PNG"
  # Also place at top-level so appimagetool surely finds it
  install -m 0644 "$ICON_SRC_PNG" "$ICON_TOPLEVEL_PNG"
else
  echo "[make_appimage] packaging/icons/hootvoice.png not found; generating placeholder icon."
  # 1x1 transparent PNG
  base64 -d > "$ICON_THEMED_PNG" << 'B64'
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNgYAAAAAMAASsJTYQAAAAASUVORK5CYII=
B64
  cp "$ICON_THEMED_PNG" "$ICON_TOPLEVEL_PNG"
fi

# AppRun launcher
cat > "$APP_DIR/AppRun" << 'EOF'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/hootvoice" "$@"
EOF
chmod +x "$APP_DIR/AppRun"

# Fetch appimagetool if needed
APPIMAGETOOL="$TOOLS_DIR/appimagetool.AppImage"
if [[ ! -x "$APPIMAGETOOL" ]]; then
  echo "[make_appimage] Downloading appimagetool..."
  URL="https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${TOOL_SUFFIX}.AppImage"
  curl -fsSL "$URL" -o "$APPIMAGETOOL"
  chmod +x "$APPIMAGETOOL"
fi

OUT_NAME="HootVoice-linux-${OUT_ARCH}.AppImage"
pushd "$APP_DIR_BASE" >/dev/null
# Use extract-and-run to avoid FUSE dependency on CI runners
if ARCH="$APPIMAGE_ARCH" "$APPIMAGETOOL" --appimage-extract-and-run --version >/dev/null 2>&1; then
  echo "[make_appimage] Using --appimage-extract-and-run"
  ARCH="$APPIMAGE_ARCH" "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream AppDir "$OUT_NAME"
else
  echo "[make_appimage] Fallback: manual --appimage-extract"
  "$APPIMAGETOOL" --appimage-extract
  if [[ -x squashfs-root/usr/bin/appimagetool ]]; then
    ARCH="$APPIMAGE_ARCH" squashfs-root/usr/bin/appimagetool --no-appstream AppDir "$OUT_NAME"
  else
    ARCH="$APPIMAGE_ARCH" squashfs-root/AppRun --no-appstream AppDir "$OUT_NAME"
  fi
fi
popd >/dev/null

install -m 0755 "$APP_DIR_BASE/$OUT_NAME" "$DIST_DIR/$OUT_NAME"
echo "✅ AppImage created: $DIST_DIR/$OUT_NAME"
