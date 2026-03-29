import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Recording {
  id: string;
  folder_name: string;
  date: string;
  time: string;
  duration_secs: number | null;
  has_transcript: boolean;
  transcript_text: string | null;
  audio_path: string;
}

function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

function App() {
  const [isRecording, setIsRecording] = useState(false);
  const [audioLevel, setAudioLevel] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [recordings, setRecordings] = useState<Recording[]>([]);
  const [selectedRecording, setSelectedRecording] = useState<Recording | null>(null);
  const [transcribing, setTranscribing] = useState<string | null>(null);
  const [sidecarOnline, setSidecarOnline] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pollRef = useRef<number | null>(null);

  const loadRecordings = useCallback(async () => {
    try {
      const recs = await invoke<Recording[]>("get_recordings");
      setRecordings(recs);
    } catch (e) {
      console.error("Failed to load recordings:", e);
    }
  }, []);

  const checkSidecar = useCallback(async () => {
    try {
      const online = await invoke<boolean>("check_sidecar_status");
      setSidecarOnline(online);
    } catch {
      setSidecarOnline(false);
    }
  }, []);

  useEffect(() => {
    loadRecordings();
    checkSidecar();
    const interval = setInterval(checkSidecar, 5000);
    return () => clearInterval(interval);
  }, [loadRecordings, checkSidecar]);

  useEffect(() => {
    if (isRecording) {
      pollRef.current = window.setInterval(async () => {
        try {
          const [recording, level, secs] = await invoke<[boolean, number, number]>(
            "get_recording_status"
          );
          setAudioLevel(level);
          setElapsed(secs);
          if (!recording) {
            setIsRecording(false);
          }
        } catch (e) {
          console.error(e);
        }
      }, 100);
    } else {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    }
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [isRecording]);

  const handleStartRecording = async () => {
    try {
      setError(null);
      await invoke("start_recording");
      setIsRecording(true);
      setElapsed(0);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleStopRecording = async () => {
    try {
      setError(null);
      await invoke("stop_recording");
      setIsRecording(false);
      setAudioLevel(0);
      setElapsed(0);
      await loadRecordings();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleTranscribe = async (rec: Recording) => {
    if (!sidecarOnline) {
      setError("Transcription service is offline. Start the Python sidecar first.");
      return;
    }
    try {
      setError(null);
      const folderPath = rec.audio_path.replace("/audio.wav", "");
      setTranscribing(rec.id);
      await invoke("transcribe_recording", { folderPath });
      await loadRecordings();
      setTranscribing(null);
    } catch (e) {
      setError(String(e));
      setTranscribing(null);
    }
  };

  const levelWidth = Math.min(audioLevel * 300, 100);

  return (
    <div className="flex flex-col h-screen bg-bg-primary">
      {/* Header */}
      <header className="flex items-center justify-between px-6 py-4 bg-bg-secondary border-b border-border">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded-lg bg-accent flex items-center justify-center text-white font-bold text-sm">
            SK
          </div>
          <h1 className="text-lg font-semibold text-text-primary">SoloKeeper Mic</h1>
        </div>
        <div className="flex items-center gap-2">
          <div
            className={`w-2 h-2 rounded-full ${sidecarOnline ? "bg-success" : "bg-danger"}`}
            title={sidecarOnline ? "Transcription service online" : "Transcription service offline"}
          />
          <span className="text-xs text-text-secondary">
            {sidecarOnline ? "AI Ready" : "AI Offline"}
          </span>
        </div>
      </header>

      {/* Recording Controls */}
      <div className="flex flex-col items-center py-8 bg-bg-secondary border-b border-border">
        {isRecording && (
          <div className="mb-4 text-center">
            <span className="text-3xl font-mono text-text-primary">{formatDuration(elapsed)}</span>
          </div>
        )}

        {/* Audio Level */}
        {isRecording && (
          <div className="w-64 h-2 bg-bg-tertiary rounded-full mb-4 overflow-hidden">
            <div
              className="h-full bg-accent rounded-full transition-all duration-75"
              style={{ width: `${levelWidth}%` }}
            />
          </div>
        )}

        <button
          onClick={isRecording ? handleStopRecording : handleStartRecording}
          className={`w-20 h-20 rounded-full flex items-center justify-center transition-all duration-200 cursor-pointer ${
            isRecording
              ? "bg-danger hover:bg-danger-hover"
              : "bg-accent hover:bg-accent-hover"
          }`}
        >
          {isRecording ? (
            <div className="w-7 h-7 bg-white rounded-sm" />
          ) : (
            <div className="w-7 h-7 bg-white rounded-full" />
          )}
        </button>
        <span className="mt-3 text-sm text-text-secondary">
          {isRecording ? "Recording... Click to stop" : "Click to start recording"}
        </span>

        {error && (
          <div className="mt-3 px-4 py-2 bg-danger/20 border border-danger/30 rounded text-danger text-sm max-w-md text-center">
            {error}
          </div>
        )}
      </div>

      {/* Recordings List */}
      <div className="flex-1 overflow-y-auto">
        <div className="px-6 py-4">
          <h2 className="text-sm font-semibold text-text-secondary uppercase tracking-wider mb-3">
            Recordings ({recordings.length})
          </h2>

          {recordings.length === 0 ? (
            <div className="text-center py-12 text-text-secondary">
              <p className="text-lg mb-2">No recordings yet</p>
              <p className="text-sm">Press the record button to start your first recording</p>
            </div>
          ) : (
            <div className="space-y-2">
              {recordings.map((rec) => (
                <div
                  key={rec.id}
                  className={`p-4 rounded-lg border cursor-pointer transition-colors ${
                    selectedRecording?.id === rec.id
                      ? "bg-bg-tertiary border-accent/50"
                      : "bg-bg-secondary border-border hover:border-border"
                  }`}
                  onClick={() => setSelectedRecording(rec)}
                >
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium text-text-primary">
                        {rec.date} at {rec.time}
                      </div>
                      <div className="text-sm text-text-secondary mt-1">
                        {rec.duration_secs != null
                          ? `Duration: ${formatDuration(rec.duration_secs)}`
                          : "Duration unknown"}
                        {rec.has_transcript && (
                          <span className="ml-3 text-success">Transcribed</span>
                        )}
                      </div>
                    </div>
                    <div className="flex gap-2">
                      {!rec.has_transcript && (
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            handleTranscribe(rec);
                          }}
                          disabled={transcribing === rec.id}
                          className={`px-3 py-1 rounded text-sm transition-colors ${
                            transcribing === rec.id
                              ? "bg-bg-tertiary text-text-secondary cursor-wait"
                              : "bg-accent hover:bg-accent-hover text-white cursor-pointer"
                          }`}
                        >
                          {transcribing === rec.id ? "Transcribing..." : "Transcribe"}
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Transcript Preview */}
                  {selectedRecording?.id === rec.id && rec.transcript_text && (
                    <div className="mt-4 p-4 bg-bg-primary rounded border border-border">
                      <h3 className="text-sm font-semibold text-text-secondary mb-2">
                        Transcript
                      </h3>
                      <pre className="text-sm text-text-primary whitespace-pre-wrap font-sans leading-relaxed max-h-64 overflow-y-auto">
                        {rec.transcript_text}
                      </pre>
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default App;
