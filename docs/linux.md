# Linux Setup (Vulkan/Wayland)

HootVoice uses `whisper-rs` with GPU acceleration where available. On Linux,
Vulkan is recommended for best performance.

## Dependencies (Ubuntu 24.04)

```
sudo apt update
sudo apt install glslc libvulkan-dev vulkan-tools mesa-vulkan-drivers libopenblas-dev
vulkaninfo | head -n 20 # verify
```

- Wayland auto‑paste: install `wtype`
- X11 auto‑paste: install `xdotool`

## Wayland Floating Window (optional)

Build with the `wayland_layer` feature to enable the tiny floating window via a sidecar:

```
cargo build --release --features wayland_layer --bins
```

## Troubleshooting

- No Vulkan device found: make sure GPU drivers and Vulkan ICD are installed
- Auto‑paste not working: verify `wtype` (Wayland) or `xdotool` (X11) is installed
- No microphones: check `arecord -l` / `pactl list sources` and audio server
- Vulkan build/runtime issues: ensure Vulkan runtime/driver and `glslc` are installed; verify with `vulkaninfo`
- Model download is slow: first download can take several minutes; models are cached per user
