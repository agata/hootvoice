# HootVoice — Real‑time voice input (Whisper)

Website: https://hootvoice.com

Latest downloads: https://github.com/agata/hootvoice/releases/latest

HootVoice records your microphone and transcribes speech to text in real time using Whisper. It ships a simple floating toolbar and a settings UI with model/download management, device controls, and a user dictionary.

For end users, see the [User Manual](docs/manual.html) (also available on the website).
For Japanese, see `README.ja.md` and the [ユーザーマニュアル](docs/manual.ja.html).

## Highlights

- Global hotkey: toggle recording anywhere (default Ctrl+Shift+R; configurable)
- Accurate transcription: whisper-rs backend with GPU acceleration (Vulkan/Metal)
- Auto copy/paste: copies to clipboard and optionally pastes into the front app
- Cross‑platform: Linux, macOS, Windows
- Floating UI: tiny always‑on‑top toolbar; proper Wayland layer‑shell sidecar
- Model manager: download official ggml models with progress, stored per‑user
- User dictionary: replace words/phrases with aliases, applied contextually
- Status sounds: start/processing/complete audio cues with adjustable volume
- Fully local: on‑device transcription — no cloud, no API fees

## Local processing (no cloud)

HootVoice runs entirely on your device. Audio is not uploaded to any server.

- Privacy: content never leaves your machine.
- Performance: no network latency; speed depends on CPU/GPU and chosen model.
- Cost: no API usage fees. Only the first-time model download needs network; you can pre-seed `models/` for offline use.

### Whisper model licensing

HootVoice downloads Whisper models on demand. Whisper (and the ggml conversions it uses) are released under the MIT License by OpenAI. See the [Whisper repository](https://github.com/openai/whisper#license) for full terms before redistributing models with your own builds.

## Requirements

- Rust stable (1.70+) and a working C/C++ toolchain
- Microphone input device; 4 GB+ RAM (8 GB recommended)

Platform notes:
- Linux
  - Audio: `libasound2-dev` (ALSA)
  - Clipboard/paste: Wayland → `wl-clipboard` + `wtype`; X11 → `xclip` + `xdotool`
  - Copy-only: requires `xclip` (X11) or `wl-copy` (Wayland)
  - Proper Wayland floating (optional feature `wayland_layer`): GTK4 + `gtk4-layer-shell`
  - GPU (Vulkan) build/runtime deps for whisper-rs:
    - Ubuntu 24.04: `sudo apt install glslc libvulkan-dev vulkan-tools mesa-vulkan-drivers libopenblas-dev`
    - Arch: `vulkan-headers vulkan-icd-loader vulkan-radeon (or nvidia-utils) vulkan-tools` (+ `openblas` optional)
- macOS
  - Xcode Command Line Tools (`xcode-select --install`), macOS 10.15+
  - Accessibility permission is required for auto‑paste (see Troubleshooting)
- Windows
  - Visual Studio Build Tools (C++), Windows 10/11
  - Vulkan SDK recommended for GPU builds

## Build

Clone and build:

```bash
git clone https://github.com/agata/hootvoice.git
cd hootvoice
cargo build --release

# Optional: Wayland layer‑shell floating sidecar (builds hootvoice-float)
cargo build --release --features wayland_layer --bins
```

Binaries:
- Linux/macOS: `target/release/hootvoice`
- Windows: `target/release/hootvoice.exe`
- Wayland sidecar: `target/release/hootvoice-float` (when `wayland_layer` is enabled)

Note (Linux): enabling `whisper-rs`’s `vulkan` feature requires a shader compiler (`glslc`). On Ubuntu 24.04 install: `glslc libvulkan-dev vulkan-tools mesa-vulkan-drivers libopenblas-dev`.

## Run

```bash
./target/release/hootvoice
```

On first launch the selected Whisper model is downloaded to the per‑user config dir (default Large‑v3 ~3.1 GB). Change the model in Settings → Speech Model; downloads are saved under `models/` inside the app config directory.

Controls:
- Toggle: global hotkey (default Ctrl+Shift+R; configurable in Settings)
- Signals (Linux/macOS): `SIGUSR1` toggles recording; `SIGUSR2` opens Settings

## App Data

Per‑user directory for settings/models/dictionary:
- Linux: `~/.config/HootVoice`
- macOS: `~/Library/Application Support/HootVoice`
- Windows: `%APPDATA%\HootVoice`

Contents:
- `settings.toml`: UI settings (hotkey, devices, language, behavior)
- `config.toml`: app config (stores `whisper.model_path` for compatibility)
- `models/`: downloaded ggml models (`ggml-*.bin`)
- `dictionary.yaml`: user dictionary (editable in Settings → Dictionary)

## Packaging

Use the helper script for local packaging:

```bash
./build.sh
```

Outputs are written to `dist/` (and archives to the project root). On Linux, an AppImage is produced if tools are available. On macOS, `.app`/DMG are created when `cargo-bundle` and platform tools are present.

Justfile tasks:

```bash
just clean   # cargo clean
just run     # cargo run
just build   # ./build.sh
just package # dist -> artifacts with versioned archives
```

## Wayland (Hyprland)

When built with `wayland_layer`, HootVoice launches a sidecar (`hootvoice-float`) for a native layer‑shell floating toolbar. You can add Hyprland rules to keep the base window hidden in a special workspace and the toolbar always on top.

Optional icons: place 16–24 px PNGs and set `HOOTVOICE_ICON_DIR` (expects `record.png`, `stop.png`, `loader.png`, `settings.png`, `grip-vertical.png`).

## Documentation

- Website (Docs): https://hootvoice.com
- Linux Setup: docs/linux.md

## Waybar (sample custom module)

HootVoice writes `status.json` next to your app settings in the OS‑standard config directory (e.g., on Linux: `~/.config/HootVoice/status.json`). You can use a Waybar custom module to display and control it.

Waybar config (e.g. `~/.config/waybar/config`):

```json
{
  "custom/hootvoice": {
    "return-type": "json",
    "interval": 1,
    "exec": "cat ~/.config/HootVoice/status.json || echo '{\"text\":\"○\",\"tooltip\":\"idle\",\"class\":\"idle\",\"alt\":\"idle\",\"color\":\"#22aa22\"}'",
    "on-click": "pkill -USR1 hootvoice",
    "on-click-right": "pkill -USR2 hootvoice"
  }
}
```

Waybar style (e.g. `~/.config/waybar/style.css`):

```css
#custom-hootvoice.idle { color: #22aa22; }
#custom-hootvoice.recording { color: #dd3333; }
#custom-hootvoice.processing { color: #d0c000; }
```

Notes:
- Left click toggles recording (SIGUSR1), right click opens Settings (SIGUSR2).
- The JSON fields `text`, `tooltip`, `class`, `alt`, `color` are produced by HootVoice.

## Troubleshooting

- Vulkan build errors (Linux): install `glslc libvulkan-dev vulkan-tools mesa-vulkan-drivers` (Ubuntu 24.04). Ensure GPU drivers are present.
- Auto‑paste on macOS: grant Accessibility and Automation permissions to “HootVoice” (System Settings → Privacy & Security). The app logs hints if blocked.
- Auto‑paste on Linux: install `wtype` on Wayland or `xdotool` on X11. If not available, the app falls back to copy‑only.
- No microphone devices: check ALSA devices (`aplay -l`, `arecord -l`) and audio server.

## Third‑party licenses

The app bundles `assets/THIRD_PARTY_LICENSES.md` generated via `cargo-about` for in‑app viewing (Settings → General).

## License

MIT

Runtime Whisper models downloaded through HootVoice are provided by OpenAI under the MIT License; review the upstream [Whisper license](https://github.com/openai/whisper#license) before redistributing bundled models.

## Contributing

PRs are welcome. For large changes, please open an issue to discuss first.

## Acknowledgements

- OpenAI Whisper
- whisper.cpp / whisper‑rs

—

Japanese docs: see `README.ja.md` and [docs/manual.ja.html](docs/manual.ja.html).
