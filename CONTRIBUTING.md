## Contributing to HootVoice

Thanks for your interest in contributing!

### Development

- Prerequisites: Rust stable, OS‑specific deps (see README)
- Clone and build:

```
git clone https://github.com/agata/hootvoice.git
cd hootvoice
cargo build
```

### Pull Requests

- Create a feature branch from `main`
- Keep the scope focused and include a clear description
- Run `cargo build` locally and ensure it succeeds cross‑platform if possible
- Prefer English for code, comments, commit messages, and issues

### Issues

- Bug reports: include OS, steps to reproduce, expected vs actual behavior, and logs if available
- Feature requests: describe the problem and proposed UX/behavior

### Code Style

- Follow existing patterns and keep the code simple and robust
- User‑facing text should go through i18n resources when practical

### Release Notes & Releases

- Changelog style: Keep `CHANGELOG.md` in English with a short, friendly tone. A celebratory vibe + a few crisp bullets work well.
- Local‑first message: Highlight that HootVoice runs entirely on your device,
  keeps audio private, and has no API usage fees.
- Before tagging: Add a new section headed `## vX.Y.Z` with your notes under it.
  The GitHub Release workflow extracts exactly that section and uses it as the
  release description.
- Create a release: Tag and push:

  ```bash
  git tag vX.Y.Z
  git push origin vX.Y.Z
  ```

- CI/Release artifacts: Linux/macOS/Windows builds are produced and uploaded.
  On Linux, GPU builds of `whisper-rs` with `vulkan` require a shader compiler
  and Vulkan libs (Ubuntu 24.04 example):

  ```bash
  sudo apt install glslc libvulkan-dev vulkan-tools mesa-vulkan-drivers libopenblas-dev
  ```

- Local checks: Run formatting and a quick build before PRs:

  ```bash
  cargo fmt --all
  cargo build
  ```
