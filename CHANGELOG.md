# Changelog

## v0.2.0

The owl picked up a few new tricks. 🦉✨ Whisper transcripts can now take a quick detour through your favorite local LLM to get cleaned up, summarized, or remixed before they land in your app.

- Added an opt-in LLM post-processing engine with a local-only HTTP client, presets for formatting or summarizing, and full custom prompt support.
- Expanded Settings with a dedicated LLM tab: configure base URL/model, control auto-paste behavior, and inspect recent history saved to disk.
- Published a new `llm-postprocess` guide and refreshed the manual so you can get up and running with Ollama, LM Studio, and friends.
- Trimmed unused diff logging code to keep builds leaner.

## v0.1.0

HootVoice takes the stage for the very first time — cue the confetti! 🎉
This debut release is fresh, friendly, and ready to turn your speech into text
with a single hotkey. If anything feels off, send us a hoot — we’re listening.

- Real‑time transcription: on‑device Whisper via whisper‑rs (no cloud, no fees)
- Local‑first & private: everything runs on your device — no audio leaves your
  machine, and there are no API usage fees. Your wallet (and privacy) will thank you.
- Global hotkey: start/stop recording anywhere; tiny floating toolbar
- Auto copy/paste: copies to clipboard and can paste into the front app
- Model manager: download and switch Whisper models with progress
- User dictionary: teach HootVoice your names and favorite phrases
- Cross‑platform builds: Linux, macOS, Windows
- Sounds and status: clear cues for start/processing/complete
- Under‑the‑hood polish and performance tweaks

P.S. Want tips, setup guides, and FAQs? Check the docs: https://hootvoice.com

Thanks for being here for 0.1 — you’re the very best 🥳
