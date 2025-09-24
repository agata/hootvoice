#!/bin/bash

# HootVoice - Build script
# Create cross-platform release builds

set -e

echo "🔨 Starting HootVoice release build..."

# Detect platform
OS="unknown"
ARCH=$(uname -m)

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
    OS="windows"
fi

echo "📦 Platform: $OS-$ARCH"

# Release build
echo "🚀 Building release..."
cargo build --release

# Check build status
if [ $? -ne 0 ]; then
    echo "❌ Build failed"
    exit 1
fi

# Binary name
BINARY_GUI="hootvoice"
if [ "$OS" == "windows" ]; then
    BINARY_GUI="hootvoice.exe"
fi

# Create output dir
OUTPUT_DIR="dist"
mkdir -p $OUTPUT_DIR

# Resolve Cargo target dir (supports Windows short path via CARGO_TARGET_DIR)
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
# Normalize backslashes to forward slashes for MSYS/bash compatibility
TARGET_DIR="${TARGET_DIR//\\/\/}"

# Copy binary
SRC_BIN="$TARGET_DIR/release/$BINARY_GUI"
echo "📄 Copying binary from: $SRC_BIN"
cp "$SRC_BIN" "$OUTPUT_DIR/"

# Copy resources
echo "📄 Copying resources..."
cp -r sounds $OUTPUT_DIR/ 2>/dev/null || echo "⚠️  sounds directory not found"
# Root-level config.toml / prompt.txt / dictionary.txt are deprecated (not bundled)
cp docs/manual.html $OUTPUT_DIR/ 2>/dev/null || true
cp docs/manual.ja.html $OUTPUT_DIR/ 2>/dev/null || true

# Per-platform handling
if [ "$OS" == "linux" ] || [ "$OS" == "macos" ]; then
    # Ensure executable bit
    chmod +x $OUTPUT_DIR/$BINARY_GUI
    
    # Strip debug symbols (smaller size)
    echo "🔧 Stripping debug symbols..."
    strip $OUTPUT_DIR/$BINARY_GUI || echo "⚠️  strip command not found"
fi

# No generic archives are created here.

# Show sizes
echo ""
echo "📊 Build artifacts:"
ls -lh "$OUTPUT_DIR/$BINARY_GUI"

echo ""
echo "✨ Build complete!"
echo "📍 Executable: $OUTPUT_DIR/$BINARY_GUI"
echo "📦 Additional platform packages will be created separately (AppImage/DMG)"

# Additional packaging (platform specific)
if [ "$OS" == "linux" ]; then
  echo ""
  echo "📦 Linux: Building AppImage..."
  bash ./scripts/make_appimage.sh || echo "⚠️ AppImage build failed or was skipped"
  if ls dist/HootVoice-linux-*.AppImage >/dev/null 2>&1; then
    ls -lh dist/HootVoice-linux-*.AppImage
  fi
elif [ "$OS" == "macos" ]; then
  echo ""
  echo "📦 macOS: Creating .app bundle..."
  bash ./scripts/make_macos_app.sh || echo "⚠️ .app build failed or was skipped"
  if [ -d dist/HootVoice.app ]; then
    echo "✅ .app output: dist/HootVoice.app"
  fi
  echo ""
  echo "📦 macOS: Creating DMG..."
  bash ./scripts/make_macos_dmg.sh || echo "⚠️ DMG build failed or was skipped"
  if ls dist/HootVoice-macos-*.dmg >/dev/null 2>&1; then
    echo "✅ DMG output:" && ls -lh dist/HootVoice-macos-*.dmg
  fi
fi
