import { useState, useEffect, useRef, useCallback } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

// ─── Types ───────────────────────────────────────────────────────────────────

interface Segment {
  start: number;
  end: number;
  text: string;
  speaker?: string;
}

interface TranscriptJson {
  text: string;
  language: string;
  duration: number;
  segments: Segment[];
  speakers: string[];
}

interface Recording {
  id: string;
  folder_name: string;
  folder_path: string;
  date: string;
  time: string;
  duration_secs: number | null;
  has_transcript: boolean;
  transcript_text: string | null;
  transcript_json: TranscriptJson | null;
  audio_path: string;
  speakers: string[];
  speaker_names: Record<string, string>;
}

interface AppSettings {
  audio_input_device: string | null;
  system_audio_device: string | null;
  whisper_model: string;
  language: string;
  storage_path: string;
  recording_mode: string;
  hf_token: string;
}

type Page = "recordings" | "settings";

// ─── Helpers ─────────────────────────────────────────────────────────────────

function formatDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

function formatTimestamp(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

function audioFileUrl(path: string): string {
  return convertFileSrc(path);
}

function highlightMatch(text: string, query: string): React.ReactNode {
  if (!query) return text;
  const idx = text.toLowerCase().indexOf(query.toLowerCase());
  if (idx === -1) return text;
  return (
    <>
      {text.slice(0, idx)}
      <mark className="bg-accent/30 text-text-primary rounded px-0.5">{text.slice(idx, idx + query.length)}</mark>
      {text.slice(idx + query.length)}
    </>
  );
}

// ─── App ─────────────────────────────────────────────────────────────────────

function App() {
  const [page, setPage] = useState<Page>("recordings");
  const [isRecording, setIsRecording] = useState(false);
  const [micLevel, setMicLevel] = useState(0);
  const [systemLevel, setSystemLevel] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [recordings, setRecordings] = useState<Recording[]>([]);
  const [selectedRecording, setSelectedRecording] = useState<Recording | null>(null);
  const [transcribing, setTranscribing] = useState<string | null>(null);
  const [sidecarOnline, setSidecarOnline] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<Recording[] | null>(null);
  const pollRef = useRef<number | null>(null);

  // Audio playback
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackTime, setPlaybackTime] = useState(0);



  // Speaker naming
  const [editingSpeakers, setEditingSpeakers] = useState(false);
  const [speakerDraft, setSpeakerDraft] = useState<Record<string, string>>({});

  // Settings
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [inputDevices, setInputDevices] = useState<string[]>([]);

  // ─── Data Loading ──────────────────────────────────────────────────────────

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

  // Recording status poll
  useEffect(() => {
    if (isRecording) {
      pollRef.current = window.setInterval(async () => {
        try {
          const [recording, mic, sys, secs] = await invoke<[boolean, number, number, number]>(
            "get_recording_status"
          );
          setMicLevel(mic);
          setSystemLevel(sys);
          setElapsed(secs);
          if (!recording) setIsRecording(false);
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

  // Audio time tracking
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    const onTime = () => setPlaybackTime(audio.currentTime);
    const onPlay = () => setIsPlaying(true);
    const onPause = () => setIsPlaying(false);
    const onEnded = () => setIsPlaying(false);
    audio.addEventListener("timeupdate", onTime);
    audio.addEventListener("play", onPlay);
    audio.addEventListener("pause", onPause);
    audio.addEventListener("ended", onEnded);
    return () => {
      audio.removeEventListener("timeupdate", onTime);
      audio.removeEventListener("play", onPlay);
      audio.removeEventListener("pause", onPause);
      audio.removeEventListener("ended", onEnded);
    };
  }, [selectedRecording]);

  // ─── Handlers ──────────────────────────────────────────────────────────────

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
      setMicLevel(0);
      setSystemLevel(0);
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
      setTranscribing(rec.id);
      await invoke("transcribe_recording", { folderPath: rec.folder_path });
      await loadRecordings();
      setTranscribing(null);
      // Refresh selected recording
      const updated = (await invoke<Recording[]>("get_recordings")).find(r => r.id === rec.id);
      if (updated) setSelectedRecording(updated);
    } catch (e) {
      setError(String(e));
      setTranscribing(null);
    }
  };

  const handleSearch = async (query: string) => {
    setSearchQuery(query);
    if (!query.trim()) {
      setSearchResults(null);
      return;
    }
    try {
      const results = await invoke<Recording[]>("search_transcripts", { query: query.trim() });
      setSearchResults(results);
    } catch (e) {
      console.error("Search failed:", e);
    }
  };

  const handleSeekTo = (seconds: number) => {
    if (audioRef.current) {
      audioRef.current.currentTime = seconds;
      audioRef.current.play();
    }
  };

  const handleSaveSpeakerNames = async () => {
    if (!selectedRecording) return;
    try {
      await invoke("save_speaker_names", {
        folderPath: selectedRecording.folder_path,
        speakerNames: speakerDraft,
      });
      setEditingSpeakers(false);
      await loadRecordings();
      const updated = (await invoke<Recording[]>("get_recordings")).find(r => r.id === selectedRecording.id);
      if (updated) setSelectedRecording(updated);
    } catch (e) {
      setError(String(e));
    }
  };

  const loadSettings = async () => {
    const s = await invoke<AppSettings>("get_settings");
    setSettings(s);
    const devices = await invoke<string[]>("get_input_devices");
    setInputDevices(devices);
  };

  const handleSaveSettings = async () => {
    if (!settings) return;
    setSettingsSaving(true);
    try {
      await invoke("save_app_settings", { settings });
      setSettingsSaving(false);
    } catch (e) {
      setError(String(e));
      setSettingsSaving(false);
    }
  };

  useEffect(() => {
    if (page === "settings") loadSettings();
  }, [page]);

  const displayRecordings = searchResults ?? recordings;

  // ─── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col h-screen bg-bg-primary">
      {/* Header */}
      <header className="flex items-center justify-between px-6 py-3 bg-bg-secondary border-b border-border">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded-lg bg-accent flex items-center justify-center text-white font-bold text-sm">
            SK
          </div>
          <h1 className="text-lg font-semibold text-text-primary">SoloKeeper Mic</h1>
        </div>
        <div className="flex items-center gap-4">
          <nav className="flex gap-1">
            <button
              onClick={() => setPage("recordings")}
              className={`px-3 py-1.5 rounded text-sm transition-colors cursor-pointer ${
                page === "recordings" ? "bg-accent text-white" : "text-text-secondary hover:text-text-primary"
              }`}
            >
              Recordings
            </button>
            <button
              onClick={() => setPage("settings")}
              className={`px-3 py-1.5 rounded text-sm transition-colors cursor-pointer ${
                page === "settings" ? "bg-accent text-white" : "text-text-secondary hover:text-text-primary"
              }`}
            >
              Settings
            </button>
          </nav>
          <div className="flex items-center gap-2">
            <div
              className={`w-2 h-2 rounded-full ${sidecarOnline ? "bg-success" : "bg-danger"}`}
              title={sidecarOnline ? "Transcription service online" : "Transcription service offline"}
            />
            <span className="text-xs text-text-secondary">
              {sidecarOnline ? "AI Ready" : "AI Offline"}
            </span>
          </div>
        </div>
      </header>

      {error && (
        <div className="mx-6 mt-3 px-4 py-2 bg-danger/20 border border-danger/30 rounded text-danger text-sm">
          {error}
          <button onClick={() => setError(null)} className="ml-2 underline cursor-pointer">dismiss</button>
        </div>
      )}

      {page === "recordings" ? (
        <RecordingsPage
          isRecording={isRecording}
          elapsed={elapsed}
          micLevel={micLevel}
          systemLevel={systemLevel}
          recordings={displayRecordings}
          selectedRecording={selectedRecording}
          transcribing={transcribing}
          sidecarOnline={sidecarOnline}
          searchQuery={searchQuery}
          isSearching={searchResults !== null}
          onStartRecording={handleStartRecording}
          onStopRecording={handleStopRecording}
          onSelectRecording={(rec) => {
            setSelectedRecording(rec);
            setEditingSpeakers(false);
          }}
          onTranscribe={handleTranscribe}
          onSearch={handleSearch}
          onSeekTo={handleSeekTo}
          audioRef={audioRef}
          isPlaying={isPlaying}
          playbackTime={playbackTime}
          editingSpeakers={editingSpeakers}
          speakerDraft={speakerDraft}
          onStartEditSpeakers={() => {
            if (selectedRecording) {
              setSpeakerDraft({ ...selectedRecording.speaker_names });
              setEditingSpeakers(true);
            }
          }}
          onCancelEditSpeakers={() => setEditingSpeakers(false)}
          onUpdateSpeakerDraft={(key, val) => setSpeakerDraft({ ...speakerDraft, [key]: val })}
          onSaveSpeakerNames={handleSaveSpeakerNames}
        />
      ) : (
        <SettingsPage
          settings={settings}
          inputDevices={inputDevices}
          saving={settingsSaving}
          onUpdate={(patch) => settings && setSettings({ ...settings, ...patch })}
          onSave={handleSaveSettings}
        />
      )}
    </div>
  );
}

// ─── Recordings Page ─────────────────────────────────────────────────────────

function RecordingsPage({
  isRecording, elapsed, micLevel, systemLevel, recordings, selectedRecording,
  transcribing, sidecarOnline, searchQuery, isSearching,
  onStartRecording, onStopRecording, onSelectRecording, onTranscribe, onSearch,
  onSeekTo, audioRef, isPlaying, playbackTime,
  editingSpeakers, speakerDraft, onStartEditSpeakers, onCancelEditSpeakers,
  onUpdateSpeakerDraft, onSaveSpeakerNames,
}: {
  isRecording: boolean;
  elapsed: number;
  micLevel: number;
  systemLevel: number;
  recordings: Recording[];
  selectedRecording: Recording | null;
  transcribing: string | null;
  sidecarOnline: boolean;
  searchQuery: string;
  isSearching: boolean;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onSelectRecording: (r: Recording) => void;
  onTranscribe: (r: Recording) => void;
  onSearch: (q: string) => void;
  onSeekTo: (secs: number) => void;
  audioRef: React.RefObject<HTMLAudioElement | null>;
  isPlaying: boolean;
  playbackTime: number;
  editingSpeakers: boolean;
  speakerDraft: Record<string, string>;
  onStartEditSpeakers: () => void;
  onCancelEditSpeakers: () => void;
  onUpdateSpeakerDraft: (key: string, val: string) => void;
  onSaveSpeakerNames: () => void;
}) {
  return (
    <div className="flex flex-1 overflow-hidden">
      {/* Left panel: controls + list */}
      <div className="w-80 min-w-72 flex flex-col border-r border-border bg-bg-secondary">
        {/* Recording Controls */}
        <div className="flex flex-col items-center py-6 border-b border-border">
          {isRecording && (
            <div className="mb-3 text-center">
              <span className="text-2xl font-mono text-text-primary">{formatDuration(elapsed)}</span>
            </div>
          )}
          {isRecording && (
            <div className="flex items-end gap-3 mb-3">
              <VuMeter label="Mic" level={micLevel} />
              <VuMeter label="System" level={systemLevel} />
            </div>
          )}
          <button
            onClick={isRecording ? onStopRecording : onStartRecording}
            className={`w-16 h-16 rounded-full flex items-center justify-center transition-all duration-200 cursor-pointer ${
              isRecording ? "bg-danger hover:bg-danger-hover" : "bg-accent hover:bg-accent-hover"
            }`}
          >
            {isRecording ? (
              <div className="w-6 h-6 bg-white rounded-sm" />
            ) : (
              <div className="w-6 h-6 bg-white rounded-full" />
            )}
          </button>
          <span className="mt-2 text-xs text-text-secondary">
            {isRecording ? "Recording..." : "Record"}
          </span>
        </div>

        {/* Search */}
        <div className="px-3 py-3 border-b border-border">
          <input
            type="text"
            placeholder="Search transcripts..."
            value={searchQuery}
            onChange={(e) => onSearch(e.target.value)}
            className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary placeholder-text-secondary focus:outline-none focus:border-accent"
          />
          {isSearching && (
            <div className="mt-1 text-xs text-text-secondary">
              {recordings.length} result{recordings.length !== 1 ? "s" : ""}
              <button onClick={() => onSearch("")} className="ml-2 text-accent cursor-pointer">clear</button>
            </div>
          )}
        </div>

        {/* Recordings List */}
        <div className="flex-1 overflow-y-auto">
          {recordings.length === 0 ? (
            <div className="text-center py-8 text-text-secondary text-sm">
              {isSearching ? "No results" : "No recordings yet"}
            </div>
          ) : (
            <div className="divide-y divide-border">
              {recordings.map((rec) => (
                <div
                  key={rec.id}
                  className={`px-3 py-3 cursor-pointer transition-colors ${
                    selectedRecording?.id === rec.id ? "bg-bg-tertiary" : "hover:bg-bg-primary"
                  }`}
                  onClick={() => onSelectRecording(rec)}
                >
                  <div className="font-medium text-sm text-text-primary">
                    {rec.date} at {rec.time}
                  </div>
                  <div className="text-xs text-text-secondary mt-0.5 flex items-center gap-2">
                    {rec.duration_secs != null && <span>{formatDuration(rec.duration_secs)}</span>}
                    {rec.has_transcript && <span className="text-success">Transcribed</span>}
                    {rec.speakers.length > 0 && (
                      <span>{rec.speakers.length} speaker{rec.speakers.length !== 1 ? "s" : ""}</span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Right panel: transcript viewer */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {selectedRecording ? (
          <>
            {/* Recording header */}
            <div className="px-6 py-4 border-b border-border bg-bg-secondary">
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-lg font-semibold text-text-primary">
                    {selectedRecording.date} at {selectedRecording.time}
                  </h2>
                  <div className="text-sm text-text-secondary mt-0.5">
                    {selectedRecording.duration_secs != null && (
                      <span>Duration: {formatDuration(selectedRecording.duration_secs)}</span>
                    )}
                    {selectedRecording.speakers.length > 0 && (
                      <span className="ml-3">
                        Speakers: {selectedRecording.speakers.map(s =>
                          selectedRecording.speaker_names[s] || s
                        ).join(", ")}
                      </span>
                    )}
                  </div>
                </div>
                <div className="flex gap-2">
                  {selectedRecording.speakers.length > 0 && !editingSpeakers && (
                    <button
                      onClick={onStartEditSpeakers}
                      className="px-3 py-1.5 rounded text-sm bg-bg-tertiary hover:bg-border text-text-secondary hover:text-text-primary transition-colors cursor-pointer"
                    >
                      Rename Speakers
                    </button>
                  )}
                  {selectedRecording.has_transcript && (
                    <button
                      onClick={() => {
                        const mdPath = selectedRecording.folder_path + "/transcript.md";
                        navigator.clipboard.writeText(mdPath);
                      }}
                      className="px-3 py-1.5 rounded text-sm bg-bg-tertiary hover:bg-border text-text-secondary hover:text-text-primary transition-colors cursor-pointer"
                      title="Copy path to transcript.md"
                    >
                      📋 Copy path
                    </button>
                  )}
                  {!selectedRecording.has_transcript && (
                    <button
                      onClick={() => onTranscribe(selectedRecording)}
                      disabled={transcribing === selectedRecording.id || !sidecarOnline}
                      className={`px-3 py-1.5 rounded text-sm transition-colors ${
                        transcribing === selectedRecording.id
                          ? "bg-bg-tertiary text-text-secondary cursor-wait"
                          : !sidecarOnline
                          ? "bg-bg-tertiary text-text-secondary cursor-not-allowed"
                          : "bg-accent hover:bg-accent-hover text-white cursor-pointer"
                      }`}
                    >
                      {transcribing === selectedRecording.id ? "Transcribing..." : "Transcribe"}
                    </button>
                  )}
                </div>
              </div>

              {/* Speaker naming editor */}
              {editingSpeakers && (
                <div className="mt-3 p-3 bg-bg-tertiary rounded border border-border">
                  <div className="text-sm font-medium text-text-primary mb-2">Rename Speakers</div>
                  <div className="space-y-2">
                    {selectedRecording.speakers.map((spk) => (
                      <div key={spk} className="flex items-center gap-2">
                        <span className="text-xs text-text-secondary w-24 shrink-0">{spk}</span>
                        <input
                          type="text"
                          value={speakerDraft[spk] || ""}
                          onChange={(e) => onUpdateSpeakerDraft(spk, e.target.value)}
                          className="flex-1 px-2 py-1 bg-bg-primary border border-border rounded text-sm text-text-primary focus:outline-none focus:border-accent"
                          placeholder="Enter name..."
                        />
                      </div>
                    ))}
                  </div>
                  <div className="flex gap-2 mt-3">
                    <button
                      onClick={onSaveSpeakerNames}
                      className="px-3 py-1 rounded text-sm bg-accent hover:bg-accent-hover text-white cursor-pointer"
                    >
                      Save
                    </button>
                    <button
                      onClick={onCancelEditSpeakers}
                      className="px-3 py-1 rounded text-sm bg-bg-primary hover:bg-border text-text-secondary cursor-pointer"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </div>

            {/* Audio Player */}
            {selectedRecording.audio_path && (
              <div className="px-6 py-3 border-b border-border bg-bg-secondary flex items-center gap-3">
                <audio ref={audioRef} src={audioFileUrl(selectedRecording.audio_path)} preload="metadata" />
                <button
                  onClick={() => {
                    if (audioRef.current) {
                      if (isPlaying) audioRef.current.pause();
                      else audioRef.current.play();
                    }
                  }}
                  className="w-8 h-8 rounded-full bg-accent hover:bg-accent-hover flex items-center justify-center cursor-pointer transition-colors"
                >
                  {isPlaying ? (
                    <svg width="12" height="14" viewBox="0 0 12 14" fill="white">
                      <rect x="0" y="0" width="4" height="14" />
                      <rect x="8" y="0" width="4" height="14" />
                    </svg>
                  ) : (
                    <svg width="12" height="14" viewBox="0 0 12 14" fill="white">
                      <polygon points="0,0 12,7 0,14" />
                    </svg>
                  )}
                </button>
                <span className="text-sm font-mono text-text-secondary">{formatTimestamp(playbackTime)}</span>
                <input
                  type="range"
                  min={0}
                  max={selectedRecording.duration_secs || 0}
                  value={playbackTime}
                  onChange={(e) => {
                    if (audioRef.current) audioRef.current.currentTime = Number(e.target.value);
                  }}
                  className="flex-1 accent-accent h-1"
                />
                <span className="text-sm font-mono text-text-secondary">
                  {selectedRecording.duration_secs != null ? formatTimestamp(selectedRecording.duration_secs) : "--:--"}
                </span>
              </div>
            )}

            {/* Transcript */}
            <div className="flex-1 overflow-y-auto px-6 py-4">
              {selectedRecording.transcript_json ? (
                <TranscriptView
                  segments={selectedRecording.transcript_json.segments}
                  speakerNames={selectedRecording.speaker_names}
                  searchQuery={searchQuery}
                  playbackTime={playbackTime}
                  onSeekTo={onSeekTo}
                />
              ) : selectedRecording.transcript_text ? (
                <pre className="text-sm text-text-primary whitespace-pre-wrap font-sans leading-relaxed">
                  {selectedRecording.transcript_text}
                </pre>
              ) : (
                <div className="text-center py-12 text-text-secondary">
                  <p className="text-lg mb-2">No transcript yet</p>
                  <p className="text-sm">Click "Transcribe" to generate a transcript</p>
                </div>
              )}
            </div>
          </>
        ) : (
          <div className="flex-1 flex items-center justify-center text-text-secondary">
            <p>Select a recording to view its transcript</p>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── VU Meter ───────────────────────────────────────────────────────────────

function VuMeter({ label, level }: { label: string; level: number }) {
  const clampedLevel = Math.max(0, Math.min(1, level));
  const db = clampedLevel > 0 ? 20 * Math.log10(clampedLevel) : -60;
  const pct = ((db + 60) / 60) * 100; // -60dB = 0%, 0dB = 100%
  const clampedPct = Math.max(0, Math.min(100, pct));

  // Color of the top of the filled region
  const barColor = db > -6 ? "#ef4444" : db > -12 ? "#eab308" : "#22c55e";

  return (
    <div className="flex flex-col items-center gap-1">
      <div className="relative w-5 h-[120px] bg-bg-tertiary rounded overflow-hidden border border-border">
        {/* dB scale marks */}
        {[-6, -12, -24, -48].map((mark) => {
          const markPct = ((mark + 60) / 60) * 100;
          return (
            <div
              key={mark}
              className="absolute left-0 w-full border-t border-white/10"
              style={{ bottom: `${markPct}%` }}
            />
          );
        })}
        {/* Filled bar */}
        <div
          className="absolute bottom-0 left-0 w-full rounded-t-sm"
          style={{
            height: `${clampedPct}%`,
            background: `linear-gradient(to top, #22c55e 0%, #22c55e 60%, #eab308 80%, #ef4444 100%)`,
            transition: "height 75ms ease-out",
            opacity: clampedLevel > 0 ? 1 : 0.3,
          }}
        />
        {/* Peak indicator line */}
        <div
          className="absolute left-0 w-full h-0.5"
          style={{
            bottom: `${clampedPct}%`,
            backgroundColor: barColor,
            transition: "bottom 75ms ease-out",
            opacity: clampedLevel > 0 ? 1 : 0,
          }}
        />
      </div>
      <span className="text-[10px] font-mono text-text-secondary">
        {clampedLevel > 0 ? `${Math.round(db)}` : "-∞"}
      </span>
      <span className="text-[10px] text-text-secondary">{label}</span>
    </div>
  );
}

// ─── Transcript Segment View ─────────────────────────────────────────────────

function TranscriptView({
  segments, speakerNames, searchQuery, playbackTime, onSeekTo,
}: {
  segments: Segment[];
  speakerNames: Record<string, string>;
  searchQuery: string;
  playbackTime: number;
  onSeekTo: (secs: number) => void;
}) {
  let lastSpeaker = "";

  return (
    <div className="space-y-1">
      {segments.map((seg, i) => {
        const isCurrent = playbackTime >= seg.start && playbackTime < seg.end;
        const speaker = seg.speaker ? (speakerNames[seg.speaker] || seg.speaker) : null;
        const showSpeaker = speaker && speaker !== lastSpeaker;
        if (speaker) lastSpeaker = speaker;

        return (
          <div key={i}>
            {showSpeaker && (
              <div className="mt-4 mb-1 text-xs font-semibold text-accent uppercase tracking-wide">
                {speaker}
              </div>
            )}
            <div
              className={`flex gap-3 py-1 px-2 rounded cursor-pointer transition-colors group ${
                isCurrent ? "bg-accent/10" : "hover:bg-bg-tertiary"
              }`}
              onClick={() => onSeekTo(seg.start)}
            >
              <span className="text-xs font-mono text-text-secondary shrink-0 pt-0.5 group-hover:text-accent">
                {formatTimestamp(seg.start)}
              </span>
              <span className="text-sm text-text-primary leading-relaxed">
                {highlightMatch(seg.text, searchQuery)}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ─── Settings Page ───────────────────────────────────────────────────────────

function SettingsPage({
  settings, inputDevices, saving, onUpdate, onSave,
}: {
  settings: AppSettings | null;
  inputDevices: string[];
  saving: boolean;
  onUpdate: (patch: Partial<AppSettings>) => void;
  onSave: () => void;
}) {
  if (!settings) return <div className="flex-1 flex items-center justify-center text-text-secondary">Loading...</div>;

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-2xl mx-auto px-6 py-8">
        <h2 className="text-xl font-semibold text-text-primary mb-6">Settings</h2>

        <div className="space-y-6">
          {/* Audio Input */}
          <SettingsField label="Microphone Device">
            <select
              value={settings.audio_input_device || ""}
              onChange={(e) => onUpdate({ audio_input_device: e.target.value || null })}
              className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary focus:outline-none focus:border-accent"
            >
              <option value="">System Default</option>
              {inputDevices.map((d) => (
                <option key={d} value={d}>{d}</option>
              ))}
            </select>
          </SettingsField>

          {/* System Audio */}
          <SettingsField
            label="Capture System Audio"
            hint="Records what you hear (other people on the call) using macOS ScreenCaptureKit. Requires Screen Recording permission."
          >
            <label className="flex items-center gap-3 cursor-pointer">
              <div
                className={`relative w-11 h-6 rounded-full transition-colors ${
                  settings.system_audio_device ? "bg-accent" : "bg-bg-tertiary border border-border"
                }`}
                onClick={() => onUpdate({ system_audio_device: settings.system_audio_device ? null : "screencapturekit" })}
              >
                <div
                  className={`absolute top-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform ${
                    settings.system_audio_device ? "translate-x-[22px]" : "translate-x-[2px]"
                  }`}
                />
              </div>
              <span className="text-sm text-text-primary">
                {settings.system_audio_device ? "Enabled" : "Disabled"}
              </span>
            </label>
          </SettingsField>

          {/* Whisper Model */}
          <SettingsField label="Whisper Model" hint="Larger models are more accurate but slower and use more memory.">
            <select
              value={settings.whisper_model}
              onChange={(e) => onUpdate({ whisper_model: e.target.value })}
              className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary focus:outline-none focus:border-accent"
            >
              <option value="tiny">tiny (fastest, ~1GB)</option>
              <option value="base">base (fast, ~1GB)</option>
              <option value="small">small (balanced, ~2GB)</option>
              <option value="medium">medium (accurate, ~5GB)</option>
              <option value="large-v3">large-v3 (best, ~10GB)</option>
            </select>
          </SettingsField>

          {/* Language */}
          <SettingsField label="Language" hint="Auto-detect works well for most cases.">
            <select
              value={settings.language}
              onChange={(e) => onUpdate({ language: e.target.value })}
              className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary focus:outline-none focus:border-accent"
            >
              <option value="auto">Auto-detect</option>
              <option value="en">English</option>
              <option value="ru">Russian</option>
              <option value="es">Spanish</option>
              <option value="fr">French</option>
              <option value="de">German</option>
              <option value="zh">Chinese</option>
              <option value="ja">Japanese</option>
              <option value="ko">Korean</option>
              <option value="pt">Portuguese</option>
              <option value="it">Italian</option>
              <option value="nl">Dutch</option>
              <option value="pl">Polish</option>
              <option value="uk">Ukrainian</option>
            </select>
          </SettingsField>

          {/* Storage Path */}
          <SettingsField label="Storage Path">
            <input
              type="text"
              value={settings.storage_path}
              onChange={(e) => onUpdate({ storage_path: e.target.value })}
              className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary focus:outline-none focus:border-accent"
            />
          </SettingsField>

          {/* Recording Mode */}
          <SettingsField label="Recording Mode">
            <select
              value={settings.recording_mode}
              onChange={(e) => onUpdate({ recording_mode: e.target.value })}
              className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary focus:outline-none focus:border-accent"
            >
              <option value="manual">Manual (press to record)</option>
              <option value="auto">Auto-detect (start on audio activity)</option>
            </select>
          </SettingsField>

          {/* HuggingFace Token */}
          <SettingsField
            label="HuggingFace Token"
            hint="Required for speaker diarization. Get one at huggingface.co/settings/tokens. You must accept the pyannote model terms first."
          >
            <input
              type="password"
              value={settings.hf_token}
              onChange={(e) => onUpdate({ hf_token: e.target.value })}
              placeholder="hf_..."
              className="w-full px-3 py-2 bg-bg-tertiary border border-border rounded text-sm text-text-primary placeholder-text-secondary focus:outline-none focus:border-accent"
            />
          </SettingsField>

          {/* Save */}
          <div className="pt-4">
            <button
              onClick={onSave}
              disabled={saving}
              className={`px-6 py-2 rounded text-sm font-medium transition-colors cursor-pointer ${
                saving ? "bg-bg-tertiary text-text-secondary" : "bg-accent hover:bg-accent-hover text-white"
              }`}
            >
              {saving ? "Saving..." : "Save Settings"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function SettingsField({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="block text-sm font-medium text-text-primary mb-1">{label}</label>
      {hint && <p className="text-xs text-text-secondary mb-2">{hint}</p>}
      {children}
    </div>
  );
}

export default App;
