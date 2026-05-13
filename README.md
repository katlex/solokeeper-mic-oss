# SoloKeeper Mic

Desktop meeting recorder with **local** AI transcription and speaker diarization. Captures your microphone and system audio (remote call participants), transcribes with [faster-whisper](https://github.com/SYSTRAN/faster-whisper), and labels speakers with [pyannote-audio](https://github.com/pyannote/pyannote-audio) — all on-device, nothing leaves your machine.

Built with Tauri 2 + React + a Python sidecar. See [`docs/spec.md`](docs/spec.md) for the full product vision.

> Status: early MVP. Tested on macOS (Apple Silicon). Windows / Linux not yet validated by maintainers — patches welcome.

## Features

- Microphone + system-audio (loopback) capture into a stereo WAV (left = mic, right = system)
- Local transcription via faster-whisper (model selectable: small / medium / large-v3)
- Speaker diarization via pyannote-audio 3.1, with Apple Silicon GPU (Metal/MPS) acceleration
- Recordings saved to `~/SoloKeeper/meetings/<timestamp>/` as `audio.wav`, `transcript.json`, `transcript.md`, `meta.json`
- Live VU meters for mic + system audio

## Prerequisites

- **Node.js 20+** and **pnpm**
- **Rust toolchain** (install via [rustup](https://rustup.rs))
- **Python 3.10+** and **[uv](https://docs.astral.sh/uv/)**
- **macOS only:** Xcode Command Line Tools, plus [BlackHole 2ch](https://existential.audio/blackhole/) and an Aggregate Device — see [`docs/SETUP_MACOS.md`](docs/SETUP_MACOS.md)
- **HuggingFace token** (needed for pyannote diarization). Create a token at <https://huggingface.co/settings/tokens> and accept the model license at <https://huggingface.co/pyannote/speaker-diarization-3.1>. You'll paste the token in the app's Settings.

## Install

```bash
pnpm install
cd sidecar && uv sync && cd ..
```

## Run (development)

The app and the Python sidecar run as two processes. Start the sidecar first:

```bash
# Terminal 1 — Python sidecar (HTTP server on 127.0.0.1:8384)
cd sidecar
uv run solokeeper-sidecar
```

```bash
# Terminal 2 — Tauri app (opens the desktop window)
pnpm tauri dev
```

On first launch, open **Settings** in the app and paste your HuggingFace token. The Whisper model is downloaded automatically on first transcription.

Override the sidecar port via `SOLOKEEPER_SIDECAR_PORT` (default `8384`).

## Build

```bash
pnpm tauri build
```

Bundles land in `src-tauri/target/release/bundle/` (`.dmg` on macOS, `.msi` on Windows, `.deb` / `.AppImage` on Linux). Note: the current build does **not** package the Python sidecar — you still need to run it separately. Sidecar bundling is on the roadmap.

## Storage layout

```
~/SoloKeeper/meetings/
  2026-03-29_15-30_meeting/
    audio.wav         # stereo: left = mic, right = system
    transcript.json   # structured segments with timestamps + speakers
    transcript.md     # human-readable
    meta.json         # date, duration, speakers, etc.
```

## Project layout

```
src/          React frontend (Vite + Tailwind)
src-tauri/    Tauri Rust backend — audio capture (cpal), file I/O, sidecar bridge
sidecar/      Python HTTP server — faster-whisper + pyannote-audio
docs/         Spec and platform setup notes
```

## Platform notes

- **macOS:** Requires *Screen Recording* permission for system-audio loopback. The recommended setup uses an Aggregate Device combining your mic + BlackHole 2ch into a single input stream — see [`docs/SETUP_MACOS.md`](docs/SETUP_MACOS.md).
- **Windows:** WASAPI loopback should work natively (untested by maintainers).
- **Linux:** Use a PulseAudio/PipeWire monitor source as the system-audio device (untested).

## Contributing

Issues and PRs welcome, especially for Windows/Linux validation and sidecar packaging.
