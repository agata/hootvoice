set shell := ["bash", "-Eeuo", "pipefail", "-c"]

# Default: list tasks
default:
    @just --list

# Clean build artifacts
clean:
    cargo clean

# Run (default binary)
run:
    cargo run

# Build (use existing build.sh)
build:
    ./build.sh

# Update third-party license file
third-party-licenses:
    cargo about generate about-md.hbs > assets/THIRD_PARTY_LICENSES.md

# Package
# - Run `just build` first
# - On macOS, best-effort codesign/notarize if env vars are set
# - Archive dist/ into tar.gz and zip under artifacts/ with versioned name
package:
    #!/usr/bin/env bash
    set -Eeuo pipefail

    # 1) build
    just build

    # 2) detect os/arch
    os_raw=$(uname -s | tr '[:upper:]' '[:lower:]')
    case "$os_raw" in
      darwin*) os="macos" ;;
      linux*) os="linux" ;;
      msys*|mingw*|cygwin*) os="windows" ;;
      *) os="$os_raw" ;;
    esac
    arch=$(uname -m)

    # 3) version from Cargo.toml (BSD sed friendly)
    #    Use awk + POSIX character class to avoid \s which BSD sed doesn't support
    version=$(awk -F '"' '/^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)
    if [[ -z "${version:-}" ]]; then
      echo "[package] Failed to get version (check [package].version in Cargo.toml)" >&2
      exit 1
    fi

    # 4) macOS signing & notarization
    if [[ "$os" == "macos" ]]; then
      if [[ -f .env ]]; then
        set -a
        # shellcheck disable=SC1091
        source .env
        set +a
      elif [[ -f .secure/set_env.sh ]]; then
        # shellcheck disable=SC1091
        source .secure/set_env.sh
      fi

      if command -v xcrun >/dev/null 2>&1; then
        app_path=""
        if [[ -d dist/HootVoice.app ]]; then app_path="dist/HootVoice.app"; fi
        if [[ -z "$app_path" && -d dist/hootvoice.app ]]; then app_path="dist/hootvoice.app"; fi

        # Codesign .app
        if [[ -n "$app_path" && -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
          echo "[package] Codesigning .app ... ($app_path)"
          codesign --force --deep --timestamp --options runtime \
                   --entitlements packaging/macos/entitlements.plist \
                   -s "$MACOS_SIGN_IDENTITY" "$app_path" || true
          codesign --verify --deep --strict -v "$app_path" || true
        fi

        # Build artifact DMG path if exists (new naming with arch suffix)
        dmg_path=""
        if ls dist/HootVoice-macos-*.dmg >/dev/null 2>&1; then dmg_path=$(ls dist/HootVoice-macos-*.dmg | head -n 1); fi

        # Codesign DMG
        if [[ -n "$dmg_path" && -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
          echo "[package] Codesigning DMG ... ($dmg_path)"
          codesign --force --timestamp -s "$MACOS_SIGN_IDENTITY" "$dmg_path" || true
        fi

        # Notarization (prefer notarizing the .app, then DMG)
        if [[ -n "${MACOS_NOTARIZE_APPLE_ID:-}" && -n "${MACOS_TEAM_ID:-}" && -n "${MACOS_NOTARIZE_PWD:-}" ]]; then
          if [[ -n "$app_path" ]]; then
            echo "[package] Notarizing .app via notarytool ..."
            xcrun notarytool submit "$app_path" \
              --apple-id "$MACOS_NOTARIZE_APPLE_ID" \
              --team-id "$MACOS_TEAM_ID" \
              --password "$MACOS_NOTARIZE_PWD" \
              --wait || echo "[package] .app notarization failed or skipped"
            xcrun stapler staple "$app_path" || true
          fi
          if [[ -n "$dmg_path" ]]; then
            echo "[package] Notarizing DMG via notarytool ..."
            xcrun notarytool submit "$dmg_path" \
              --apple-id "$MACOS_NOTARIZE_APPLE_ID" \
              --team-id "$MACOS_TEAM_ID" \
              --password "$MACOS_NOTARIZE_PWD" \
              --wait || echo "[package] DMG notarization failed or skipped"
            xcrun stapler staple "$dmg_path" || true
          fi
        else
          echo "[package] Notarization skipped (credentials not set)"
        fi
      else
        echo "[package] xcrun not found; skip macOS notarization"
      fi
    fi

    # 5) package dist -> artifacts
    if [[ ! -d dist ]]; then
      echo "dist/ not found. Run 'just build' first." >&2
      exit 1
    fi

    mkdir -p artifacts
    base="hootvoice-v${version}-${os}-${arch}"
    echo "[package] Creating artifacts: artifacts/${base}.{tar.gz,zip} ..."
    tar -czf "artifacts/${base}.tar.gz" -C dist .

    if command -v zip >/dev/null 2>&1; then
      (cd dist && zip -r "../artifacts/${base}.zip" .)
    elif [[ "$os" == "macos" ]] && command -v ditto >/dev/null 2>&1; then
      ditto -c -k --sequesterRsrc --keepParent dist "artifacts/${base}.zip"
    elif [[ "$os" == "windows" ]]; then
      powershell -Command "Compress-Archive -Path dist\\* -DestinationPath artifacts\\${base}.zip" || true
    else
      echo "zip command not found; skipping .zip artifact."
    fi

    echo "[package] Done. Outputs in ./artifacts"
