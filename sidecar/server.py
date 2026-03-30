"""
SoloKeeper Mic — Transcription & Diarization Sidecar

HTTP server that wraps faster-whisper and pyannote-audio for
local transcription and speaker diarization.
Launched by the Tauri app, communicates via localhost HTTP.
"""

import json
import os
import sys
from datetime import datetime
from pathlib import Path

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
import uvicorn

app = FastAPI(title="SoloKeeper Sidecar")

# Global model references (loaded lazily)
_whisper_model = None
_whisper_model_size = "small"
_diarization_pipeline = None
_hf_token: str | None = None


def get_whisper_model():
    global _whisper_model
    if _whisper_model is None:
        from faster_whisper import WhisperModel

        print(f"Loading Whisper model: {_whisper_model_size}...", flush=True)
        _whisper_model = WhisperModel(
            _whisper_model_size,
            device="cpu",
            compute_type="int8",
        )
        print("Whisper model loaded.", flush=True)
    return _whisper_model


def get_diarization_pipeline():
    global _diarization_pipeline
    if _diarization_pipeline is None:
        if not _hf_token:
            raise RuntimeError(
                "HF_TOKEN is required for speaker diarization. "
                "Set it in Settings or as an environment variable."
            )
        from pyannote.audio import Pipeline

        import torch

        print("Loading pyannote diarization pipeline...", flush=True)
        _diarization_pipeline = Pipeline.from_pretrained(
            "pyannote/speaker-diarization-3.1",
            token=_hf_token,
        )

        # Use Apple Silicon GPU (Metal) if available for ~3-5x speedup
        if torch.backends.mps.is_available():
            _diarization_pipeline = _diarization_pipeline.to(torch.device("mps"))
            print("Diarization pipeline loaded (MPS/Metal GPU).", flush=True)
        else:
            print("Diarization pipeline loaded (CPU).", flush=True)
    return _diarization_pipeline


class TranscribeRequest(BaseModel):
    audio_path: str
    output_dir: str
    language: str | None = None
    diarize: bool = True


class TranscribeResponse(BaseModel):
    text: str
    segments: list[dict]
    language: str
    duration: float
    speakers: list[str]


class ConfigUpdate(BaseModel):
    whisper_model: str | None = None
    hf_token: str | None = None


@app.get("/health")
async def health():
    return {
        "status": "ok",
        "model": _whisper_model_size,
        "has_hf_token": bool(_hf_token),
    }


@app.post("/config")
async def update_config(req: ConfigUpdate):
    global _whisper_model, _whisper_model_size, _diarization_pipeline, _hf_token

    if req.whisper_model and req.whisper_model != _whisper_model_size:
        _whisper_model_size = req.whisper_model
        _whisper_model = None  # Force reload on next transcription

    if req.hf_token is not None:
        _hf_token = req.hf_token if req.hf_token else None
        _diarization_pipeline = None  # Force reload

    return {"status": "ok"}


def _merge_diarization_with_segments(
    segments: list[dict], diarization
) -> tuple[list[dict], list[str]]:
    """Merge whisper segments with pyannote diarization to assign speakers."""
    speaker_set: set[str] = set()

    for seg in segments:
        seg_mid = (seg["start"] + seg["end"]) / 2.0
        best_speaker = "Unknown"
        best_overlap = 0.0

        for turn, _, speaker in diarization.itertracks(yield_label=True):
            # Calculate overlap between whisper segment and diarization turn
            overlap_start = max(seg["start"], turn.start)
            overlap_end = min(seg["end"], turn.end)
            overlap = max(0, overlap_end - overlap_start)

            if overlap > best_overlap:
                best_overlap = overlap
                best_speaker = speaker

            # Fallback: if segment midpoint falls within this turn
            if best_overlap == 0 and turn.start <= seg_mid <= turn.end:
                best_speaker = speaker

        seg["speaker"] = best_speaker
        speaker_set.add(best_speaker)

    # Create stable sorted speaker list
    speakers = sorted(speaker_set)
    return segments, speakers


@app.post("/transcribe")
async def transcribe(req: TranscribeRequest):
    audio_path = Path(req.audio_path)
    output_dir = Path(req.output_dir)

    if not audio_path.exists():
        raise HTTPException(status_code=404, detail=f"Audio file not found: {audio_path}")

    if not output_dir.exists():
        raise HTTPException(status_code=404, detail=f"Output directory not found: {output_dir}")

    try:
        # --- Step 1: Transcription ---
        model = get_whisper_model()
        segments_iter, info = model.transcribe(
            str(audio_path),
            language=req.language,
            beam_size=5,
            vad_filter=True,
        )

        segments = []
        full_text_parts = []
        for segment in segments_iter:
            seg_data = {
                "start": round(segment.start, 2),
                "end": round(segment.end, 2),
                "text": segment.text.strip(),
            }
            segments.append(seg_data)
            full_text_parts.append(segment.text.strip())

        full_text = " ".join(full_text_parts)
        detected_language = info.language
        duration = info.duration

        # --- Step 2: Save transcript immediately (before diarization) ---
        speakers: list[str] = []

        # Read meta.json for date info and speaker names
        meta_path = output_dir / "meta.json"
        meta = {}
        speaker_names: dict[str, str] = {}
        if meta_path.exists():
            with open(meta_path) as mf:
                meta = json.load(mf)
            speaker_names = meta.get("speaker_names", {})

        # Update meta.json with discovered speakers
        if speakers:
            meta["speakers"] = speakers
            # Initialize speaker_names for new speakers
            if "speaker_names" not in meta:
                meta["speaker_names"] = {}
            for spk in speakers:
                if spk not in meta["speaker_names"]:
                    meta["speaker_names"][spk] = spk  # Default: use raw label
            speaker_names = meta["speaker_names"]
            with open(meta_path, "w", encoding="utf-8") as mf:
                json.dump(meta, mf, indent=2, ensure_ascii=False)

        # Save transcript.json
        transcript_json = {
            "text": full_text,
            "language": detected_language,
            "duration": duration,
            "segments": segments,
            "speakers": speakers,
            "transcribed_at": datetime.now().isoformat(),
            "model": _whisper_model_size,
        }
        transcript_json_path = output_dir / "transcript.json"
        with open(transcript_json_path, "w", encoding="utf-8") as f:
            json.dump(transcript_json, f, indent=2, ensure_ascii=False)

        # Save transcript.md
        _write_transcript_md(output_dir, meta, segments, duration, speaker_names)

        # --- Step 3: Kick off diarization in background ---
        if req.diarize and _hf_token:
            import asyncio
            asyncio.get_event_loop().run_in_executor(
                None, _run_diarization_background, str(audio_path), str(output_dir)
            )

        return TranscribeResponse(
            text=full_text,
            segments=segments,
            language=detected_language,
            duration=duration,
            speakers=speakers,
        )

    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


def _run_diarization_background(audio_path: str, output_dir: str):
    """Run diarization in background and update transcript files."""
    try:
        pipeline = get_diarization_pipeline()
        print(f"[diarize] Running diarization on {audio_path}...", flush=True)
        diarize_result = pipeline(audio_path)

        if hasattr(diarize_result, 'speaker_diarization'):
            diarization = diarize_result.speaker_diarization
        else:
            diarization = diarize_result

        # Load existing transcript
        output_path = Path(output_dir)
        transcript_path = output_path / "transcript.json"
        with open(transcript_path) as f:
            transcript = json.load(f)

        segments, speakers = _merge_diarization_with_segments(transcript["segments"], diarization)
        transcript["segments"] = segments
        transcript["speakers"] = speakers

        # Save updated transcript.json
        with open(transcript_path, "w", encoding="utf-8") as f:
            json.dump(transcript, f, indent=2, ensure_ascii=False)

        # Update meta.json with speakers
        meta_path = output_path / "meta.json"
        meta = {}
        if meta_path.exists():
            with open(meta_path) as f:
                meta = json.load(f)
        meta["speakers"] = speakers
        if "speaker_names" not in meta:
            meta["speaker_names"] = {}
        for spk in speakers:
            if spk not in meta["speaker_names"]:
                meta["speaker_names"][spk] = spk
        with open(meta_path, "w", encoding="utf-8") as f:
            json.dump(meta, f, indent=2, ensure_ascii=False)

        # Regenerate transcript.md
        _write_transcript_md(output_path, meta, segments, transcript.get("duration", 0), meta.get("speaker_names", {}))

        print(f"[diarize] Complete! Found {len(speakers)} speakers.", flush=True)
    except Exception as e:
        print(f"[diarize] Failed: {e}", flush=True)


def _write_transcript_md(
    output_dir: Path,
    meta: dict,
    segments: list[dict],
    duration: float,
    speaker_names: dict[str, str],
):
    """Write human-readable transcript.md with optional speaker labels."""
    transcript_md_path = output_dir / "transcript.md"
    with open(transcript_md_path, "w", encoding="utf-8") as f:
        date_str = f"{meta.get('date', '')} {meta.get('time', '')}".strip()
        dur_secs = meta.get("duration_secs")
        duration_str = ""
        if dur_secs:
            mins = dur_secs // 60
            duration_str = f"{mins} min"

        # Collect display names for header
        unique_speakers = []
        seen = set()
        for seg in segments:
            spk = seg.get("speaker")
            if spk and spk not in seen:
                seen.add(spk)
                display = speaker_names.get(spk, spk)
                unique_speakers.append(display)

        f.write(f"# Meeting — {date_str}\n")
        if duration_str:
            f.write(f"Duration: {duration_str}")
            if unique_speakers:
                f.write(f" | Speakers: {', '.join(unique_speakers)}")
            f.write("\n")
        f.write("\n")

        for seg in segments:
            start_mins = int(seg["start"]) // 60
            start_secs = int(seg["start"]) % 60
            timestamp = f"{start_mins:02d}:{start_secs:02d}"
            speaker = seg.get("speaker")
            if speaker:
                display_name = speaker_names.get(speaker, speaker)
                f.write(f"**[{timestamp}] {display_name}:** {seg['text']}\n\n")
            else:
                f.write(f"**[{timestamp}]** {seg['text']}\n\n")


@app.get("/diarize-status/{recording_id}")
async def diarize_status(recording_id: str):
    """Check if diarization has completed for a recording."""
    settings = load_settings_file()
    storage = settings.get("storage_path", "")
    if not storage:
        return {"status": "unknown"}
    transcript_path = Path(storage) / recording_id / "transcript.json"
    if transcript_path.exists():
        with open(transcript_path) as f:
            t = json.load(f)
        if t.get("speakers"):
            return {"status": "complete", "speakers": t["speakers"]}
    return {"status": "pending"}


def load_settings_file():
    settings_path = Path.home() / "SoloKeeper" / "settings.json"
    if settings_path.exists():
        with open(settings_path) as f:
            return json.load(f)
    return {}


@app.post("/regenerate-md")
async def regenerate_md(req: dict):
    """Regenerate transcript.md after speaker names change."""
    output_dir = Path(req["output_dir"])
    transcript_json_path = output_dir / "transcript.json"
    meta_path = output_dir / "meta.json"

    if not transcript_json_path.exists():
        raise HTTPException(status_code=404, detail="transcript.json not found")

    with open(transcript_json_path) as f:
        transcript = json.load(f)
    meta = {}
    speaker_names = {}
    if meta_path.exists():
        with open(meta_path) as f:
            meta = json.load(f)
        speaker_names = meta.get("speaker_names", {})

    _write_transcript_md(
        output_dir,
        meta,
        transcript["segments"],
        transcript.get("duration", 0),
        speaker_names,
    )
    return {"status": "ok"}


def main():
    global _whisper_model_size, _hf_token

    port = int(os.environ.get("SOLOKEEPER_SIDECAR_PORT", "8384"))
    _whisper_model_size = os.environ.get("SOLOKEEPER_WHISPER_MODEL", "small")
    _hf_token = os.environ.get("HF_TOKEN", "") or None

    print(f"Starting SoloKeeper sidecar on port {port}...", flush=True)
    print(f"Whisper model: {_whisper_model_size}", flush=True)
    print(f"HF token: {'set' if _hf_token else 'not set (diarization disabled)'}", flush=True)

    uvicorn.run(app, host="127.0.0.1", port=port, log_level="info")


if __name__ == "__main__":
    main()
