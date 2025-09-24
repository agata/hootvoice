# Sound Files

Place the following MP3 files in this directory:

- `start.mp3` — played when recording starts
- `processing.mp3` — played when recording stops and processing begins
- `complete.mp3` — played when transcription completes
- `fail.mp3` — played when recognition fails or ends due to silence

## Generate sample beeps with ffmpeg

You can quickly generate simple beeps with ffmpeg:

```bash
# Start (short high beep)
ffmpeg -f lavfi -i "sine=frequency=880:duration=0.2" -ac 2 -ar 44100 sounds/start.mp3

# Processing (short mid beep)
ffmpeg -f lavfi -i "sine=frequency=440:duration=0.3" -ac 2 -ar 44100 sounds/processing.mp3

# Complete (two rising tones)
ffmpeg -f lavfi -i "sine=frequency=440:duration=0.2,sine=frequency=880:duration=0.2" -filter_complex "[0][1]concat=n=2:v=0:a=1" -ac 2 -ar 44100 sounds/complete.mp3

# Fail (short low beep)
ffmpeg -f lavfi -i "sine=frequency=220:duration=0.25" -ac 2 -ar 44100 sounds/fail.mp3
```
