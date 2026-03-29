# SoloKeeper Mic — Product Spec

## Vision

Desktop app for solopreneurs: automatic recording, transcription, and analysis of work meetings. Fully local, private, cross-platform. Part of the SoloKeeper ecosystem — future sync with SoloKeeper bot for follow-ups, meeting prep, and obligation tracking.

## Target User

Solopreneur, freelancer, consultant who:
- Has 3-10 calls per week (clients, partners, contractors)
- Doesn't want to pay $20/mo for Otter.ai/Fireflies
- Doesn't want recordings leaking to the cloud
- Wants to later find "what did I promise client X?"

## Core Features (MVP)

### 1. Audio Capture
- **System audio (loopback)** — hears what you hear (remote participants)
- **Microphone** — your voice
- **Both channels** → stereo WAV (left = mic, right = system audio)
- Works with any call platform: Zoom, Meet, Teams, Discord, phone via computer
- **Auto-start recording** (configurable): detect call start via audio activity. Manual start also available. Controlled via settings.

### 2. Transcription
- **Engine:** Whisper (faster-whisper / whisper.cpp) — local, offline
- **Models:** small (fast, 1-2GB RAM) / medium (accurate, 5GB) / large-v3 (maximum, 10GB)
- Model selection in settings, auto-download on first launch
- **Languages:** auto-detect or manual selection (en, ru, es, etc.)

### 3. Speaker Diarization
- **Engine:** pyannote-audio 3.1 (state-of-the-art, open source)
- Splits into Speaker 1, Speaker 2, ...
- Assign names to speakers after recording
- **Voice enrollment (v2):** auto-recognize familiar voices

### 4. Storage
- Local folder: `~/SoloKeeper/meetings/`
- Structure:
  ```
  ~/SoloKeeper/meetings/
    2026-03-29_15-30_zoom-call/
      audio.wav          # full recording
      transcript.json    # structured transcription
      transcript.md      # human-readable format
      meta.json          # metadata (date, duration, speakers, tags)
  ```
- `transcript.md` format:
  ```markdown
  # Meeting — 2026-03-29 15:30
  Duration: 47 min | Speakers: Alex, Peter

  **[00:00:12] Alex:** Let's discuss the integration status...
  **[00:00:35] Peter:** Sure, so we've completed the API layer...
  ```

### 5. UI (Desktop App)
- **Framework:** Electron + React (or Tauri + React for lightweight option)
- **System tray** — lives in tray, minimal interface
- **Main screen:** recording list, search, filters
- **Recording:** big Record / Stop button, audio level indicator
- **Viewer:** transcript with clickable timestamps for audio playback
- **Settings:** input/output devices, Whisper model, language, storage path

## Tech Stack

| Component | Technology | Why |
|-----------|-----------|-----|
| Desktop shell | **Tauri 2.0** | Lightweight (~5MB vs 150MB Electron), native webview, Rust backend |
| UI | **React + Tailwind** | Fast development, works great with Tauri |
| Audio capture | **cpal** (Rust) | Native cross-platform audio I/O |
| System loopback | **Platform-specific** | macOS: ScreenCaptureKit, Windows: WASAPI, Linux: PipeWire/Pulse monitor |
| Transcription | **faster-whisper** (Python sidecar) | CTranslate2, 4x faster than original, CPU and GPU |
| Diarization | **pyannote-audio 3.1** (Python sidecar) | Best open-source diarization in 2026 |
| Backend bridge | **Python sidecar** | Tauri spawns Python process, communicates via local HTTP |
| Storage | **SQLite + files** | Metadata in SQLite, audio/transcripts on disk |
| Packaging | **tauri-bundler** | .dmg / .msi / .deb / .AppImage |

**Decision:** Tauri 2.0 — lighter, faster, native feel. Python sidecar for ML workloads.

## Architecture

```
┌──────────────────────────────────────┐
│           Tauri 2.0 (Rust core)       │
│  ┌──────────┐  ┌──────────────────┐  │
│  │  React UI │  │  Audio Engine    │  │
│  │  (webview)│  │  (cpal + loopback│  │
│  │           │  │   per-platform)  │  │
│  └─────┬─────┘  └────────┬─────────┘  │
│        │    Tauri IPC     │            │
│        │                  │            │
│  ┌─────┴──────────────────┴─────────┐  │
│  │  Rust Backend                     │  │
│  │  ┌──────────┐  ┌──────────────┐  │  │
│  │  │  SQLite   │  │  File I/O    │  │  │
│  │  └──────────┘  └──────────────┘  │  │
│  │  ┌──────────────────────────────┐ │  │
│  │  │  Python Sidecar (HTTP)       │ │  │
│  │  │  ├─ faster-whisper           │ │  │
│  │  │  ├─ pyannote diarization     │ │  │
│  │  │  └─ action item extraction   │ │  │
│  │  └──────────────────────────────┘ │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────┘
         │
         ▼
   ~/SoloKeeper/meetings/
```

## Platform-Specific Notes

### macOS
- System audio: `ScreenCaptureKit` (macOS 12.3+) via electron-audio-loopback
- Requires permission: Screen Recording (for audio capture)
- Microphone: standard permission
- Apple Silicon: faster-whisper works on CPU (Metal support in development)

### Windows
- System audio: WASAPI loopback — natively supported
- Easiest of all three platforms
- GPU: CUDA for faster-whisper on Nvidia

### Linux
- System audio: PulseAudio/PipeWire monitor source
- May require manual audio source configuration
- Primary audience — developers, tolerant of setup

## SoloKeeper Sync (Future — v2+)

### Concept
Local transcripts → sync with SoloKeeper account (Telegram/WhatsApp bot). Bot gets full context of all meetings and can:

1. **Follow-up emails** — "Write a follow-up for yesterday's call with Peter"
2. **Obligations** — track who promised what, deadlines
3. **Meeting prep** — "What did we discuss with Peter last time?"
4. **CRM-like features** — interaction history per contact
5. **Weekly digest** — summary: 7 meetings, 12 action items, 3 overdue

### Sync Architecture
```
SoloKeeper Mic (desktop)
    │
    ├── transcript.json (structured)
    ├── meta.json (tags, speakers, action items)
    │
    ▼ [sync daemon — encrypted]
SoloKeeper Cloud (API)
    │
    ▼
SoloKeeper Bot (Telegram/WhatsApp)
    → context-aware responses
    → follow-up generation
    → obligation tracking
```

### Privacy Model
- Audio NEVER leaves the device
- Only synced: transcripts, metadata, extracted action items
- End-to-end encryption for sync
- User controls what to sync (everything / action items only / nothing)

## Competitive Landscape

| Product | Price | Local? | Diarization | Cross-platform |
|---------|-------|--------|-------------|----------------|
| Otter.ai | $17/mo | ❌ Cloud | ✅ | Web only |
| Fireflies.ai | $19/mo | ❌ Cloud | ✅ | Web only |
| Granola | $12/mo | ❌ Cloud | ✅ | Mac only |
| Meetily | Free | ✅ | ⚠️ Beta | Mac/Win/Linux |
| TranscriptionSuite | Free | ✅ | ✅ | Mac/Win/Linux |
| **SoloKeeper Mic** | **Freemium** | **✅** | **✅** | **Mac/Win/Linux** |

### Our Edge
- **Not just transcription — part of a working system** (SoloKeeper ecosystem)
- Bot knows your meeting context → smart follow-ups, prep, tracking
- Privacy-first: everything local, sync is optional and encrypted
- Built for solopreneurs, not enterprise

## Monetization

- **Free forever:** recording + transcription + diarization — all local, no account needed
- **SoloKeeper ($100/mo):** the full personal assistant package — meeting sync + AI bot (Telegram/WhatsApp) + follow-ups + obligation tracking + meeting prep + weekly digests. The desktop app is the capture layer; the bot is the brain.

The desktop app is a **lead magnet** — free, genuinely useful standalone. Once solopreneurs feel the pain of manually extracting action items and writing follow-ups, the $100/mo bot package sells itself.

## MVP Roadmap

### Phase 1 — Audio + Transcription (2 weeks)
- [ ] Electron app scaffold (tray icon, basic UI)
- [ ] Audio capture: mic + system loopback
- [ ] Python sidecar: faster-whisper transcription
- [ ] Save to local folder (audio + transcript.md)
- [ ] Basic UI: record button, list of recordings, transcript viewer

### Phase 2 — Diarization + Polish (2 weeks)
- [ ] Pyannote integration for speaker separation
- [ ] Speaker naming UI
- [ ] Search across all transcripts (full-text)
- [ ] Audio playback synced with transcript
- [ ] Settings: devices, model, language, storage path
- [ ] Auto-update

### Phase 3 — Smart Features (2 weeks)
- [ ] AI summary (local via Ollama or cloud API)
- [ ] Action items extraction
- [ ] Auto-detect meeting start/end
- [ ] Calendar integration (show meeting title)
- [ ] Export: PDF, DOCX, Notion, Obsidian

### Phase 4 — SoloKeeper Sync (4 weeks)
- [ ] Sync daemon
- [ ] SoloKeeper API integration
- [ ] Bot context enrichment
- [ ] Follow-up generation
- [ ] Obligation tracking

## Open Questions

1. ~~Electron vs Tauri?~~ → **Tauri 2.0** ✅
2. **Python sidecar packaging** — PyInstaller? Embedded Python? conda-pack?
3. **Pyannote license** — MIT, but models require HuggingFace accept. Need HF token on first launch?
4. **Branding** — "SoloKeeper Mic"? "SoloKeeper Meetings"? "SoloKeeper Listen"?
5. ~~Auto-record~~ → **Configurable** (auto or manual per settings) ✅
