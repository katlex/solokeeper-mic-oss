"""
SoloKeeper Mic — Transcription Sidecar

HTTP server that wraps faster-whisper for local transcription.
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

app = FastAPI(title="SoloKeeper Transcription Sidecar")

# Global model reference (loaded lazily)
_model = None
_model_size = "small"


def get_model():
    global _model
    if _model is None:
        from faster_whisper import WhisperModel

        print(f"Loading Whisper model: {_model_size}...", flush=True)
        _model = WhisperModel(
            _model_size,
            device="cpu",
            compute_type="int8",
        )
        print("Model loaded.", flush=True)
    return _model


class TranscribeRequest(BaseModel):
    audio_path: str
    output_dir: str
    language: str | None = None


class TranscribeResponse(BaseModel):
    text: str
    segments: list[dict]
    language: str
    duration: float


@app.get("/health")
async def health():
    return {"status": "ok", "model": _model_size}


@app.post("/transcribe")
async def transcribe(req: TranscribeRequest):
    audio_path = Path(req.audio_path)
    output_dir = Path(req.output_dir)

    if not audio_path.exists():
        raise HTTPException(status_code=404, detail=f"Audio file not found: {audio_path}")

    if not output_dir.exists():
        raise HTTPException(status_code=404, detail=f"Output directory not found: {output_dir}")

    try:
        model = get_model()
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

        # Save transcript.json
        transcript_json = {
            "text": full_text,
            "language": detected_language,
            "duration": duration,
            "segments": segments,
            "transcribed_at": datetime.now().isoformat(),
            "model": _model_size,
        }
        transcript_json_path = output_dir / "transcript.json"
        with open(transcript_json_path, "w", encoding="utf-8") as f:
            json.dump(transcript_json, f, indent=2, ensure_ascii=False)

        # Save transcript.md
        transcript_md_path = output_dir / "transcript.md"
        with open(transcript_md_path, "w", encoding="utf-8") as f:
            # Read meta.json for date info
            meta_path = output_dir / "meta.json"
            date_str = ""
            duration_str = ""
            if meta_path.exists():
                with open(meta_path) as mf:
                    meta = json.load(mf)
                date_str = f"{meta.get('date', '')} {meta.get('time', '')}"
                dur_secs = meta.get("duration_secs")
                if dur_secs:
                    mins = dur_secs // 60
                    duration_str = f"{mins} min"

            f.write(f"# Meeting — {date_str}\n")
            if duration_str:
                f.write(f"Duration: {duration_str}\n")
            f.write(f"\n")

            for seg in segments:
                start_mins = int(seg["start"]) // 60
                start_secs = int(seg["start"]) % 60
                timestamp = f"{start_mins:02d}:{start_secs:02d}"
                f.write(f"**[{timestamp}]** {seg['text']}\n\n")

        return TranscribeResponse(
            text=full_text,
            segments=segments,
            language=detected_language,
            duration=duration,
        )

    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


def main():
    global _model_size

    port = int(os.environ.get("SOLOKEEPER_SIDECAR_PORT", "8384"))
    _model_size = os.environ.get("SOLOKEEPER_WHISPER_MODEL", "small")

    print(f"Starting SoloKeeper transcription sidecar on port {port}...", flush=True)
    print(f"Whisper model: {_model_size}", flush=True)

    uvicorn.run(app, host="127.0.0.1", port=port, log_level="info")


if __name__ == "__main__":
    main()
