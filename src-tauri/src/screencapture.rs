//! System audio capture via macOS ScreenCaptureKit (macOS 13+).
//!
//! Wraps a small Objective-C bridge (`screencapture_bridge.m`) that uses
//! SCStream to capture all system audio, excluding the current process.
//! Audio is written directly to a mono 16-bit PCM WAV file at 48 kHz.
//!
//! No BlackHole or virtual audio device required.

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

// FFI declarations for the Objective-C bridge
extern "C" {
    /// Start capturing system audio. Writes mono 16-bit WAV to `output_path`.
    /// Returns 0 on success, -1 on error, -2 if macOS < 13.
    fn scb_start_capture(output_path: *const std::os::raw::c_char) -> i32;

    /// Stop capture and finalize the WAV file.
    fn scb_stop_capture();

    /// Returns current peak audio level (0.0 – 1.0).
    fn scb_get_level() -> f32;
}

/// Captures system audio using ScreenCaptureKit.
///
/// Drop-in replacement for the cpal-based system `AudioRecorder`.
/// The ObjC bridge handles the SCStream lifecycle and WAV writing internally.
pub struct SystemRecorder {
    is_recording: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
}

unsafe impl Send for SystemRecorder {}
unsafe impl Sync for SystemRecorder {}

impl SystemRecorder {
    pub fn new() -> Self {
        Self {
            is_recording: Arc::new(AtomicBool::new(false)),
            level: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Start capturing system audio to `output_path` (a .wav file).
    pub fn start_recording(&self, output_path: PathBuf) -> Result<(), String> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err("Already recording system audio".to_string());
        }

        let path_str = output_path
            .to_str()
            .ok_or("Invalid output path")?;
        let c_path = CString::new(path_str)
            .map_err(|_| "Path contains null byte")?;

        let rc = unsafe { scb_start_capture(c_path.as_ptr()) };
        match rc {
            0 => {
                self.is_recording.store(true, Ordering::SeqCst);
                eprintln!("[screencapture] System audio capture started");

                // Spawn a lightweight thread to poll the ObjC-side level
                let is_rec = self.is_recording.clone();
                let level = self.level.clone();
                std::thread::spawn(move || {
                    while is_rec.load(Ordering::SeqCst) {
                        let l = unsafe { scb_get_level() };
                        level.store(l.to_bits(), Ordering::Relaxed);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                });

                Ok(())
            }
            -2 => Err("ScreenCaptureKit requires macOS 13.0 or later".to_string()),
            _ => Err("Failed to start system audio capture (check screen recording permission)".to_string()),
        }
    }

    /// Stop capturing and finalize the WAV file.
    pub fn stop_recording(&self) -> Result<(), String> {
        self.is_recording.store(false, Ordering::SeqCst);
        unsafe { scb_stop_capture() };
        eprintln!("[screencapture] System audio capture stopped");
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn get_level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}
