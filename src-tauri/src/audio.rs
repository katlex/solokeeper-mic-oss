use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use hound::{WavReader, WavSpec, WavWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

/// Lock-free ring buffer for audio samples (SPSC: single producer, single consumer)
struct RingBuffer {
    buffer: Box<[std::sync::atomic::AtomicI16]>,
    write_pos: AtomicU32,
    read_pos: AtomicU32,
    capacity: u32,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        let mut v = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            v.push(std::sync::atomic::AtomicI16::new(0));
        }
        Self {
            buffer: v.into_boxed_slice(),
            write_pos: AtomicU32::new(0),
            read_pos: AtomicU32::new(0),
            capacity: capacity as u32,
        }
    }

    fn push(&self, sample: i16) -> bool {
        let wp = self.write_pos.load(Ordering::Relaxed);
        let next_wp = (wp + 1) % self.capacity;
        if next_wp == self.read_pos.load(Ordering::Acquire) {
            return false; // full — drop sample
        }
        self.buffer[wp as usize].store(sample, Ordering::Relaxed);
        self.write_pos.store(next_wp, Ordering::Release);
        true
    }

    fn pop(&self) -> Option<i16> {
        let rp = self.read_pos.load(Ordering::Relaxed);
        if rp == self.write_pos.load(Ordering::Acquire) {
            return None; // empty
        }
        let sample = self.buffer[rp as usize].load(Ordering::Relaxed);
        self.read_pos.store((rp + 1) % self.capacity, Ordering::Release);
        Some(sample)
    }
}

/// Thread-safe audio recorder using lock-free ring buffer.
/// Audio callback writes to ring buffer (NO locks, NO disk I/O).
/// Separate writer thread drains ring buffer to disk.
pub struct AudioRecorder {
    is_recording: Arc<AtomicBool>,
    level: Arc<AtomicU32>, // f32 bits stored as u32 (atomic)
    ring_buffer: Arc<RingBuffer>,
}

unsafe impl Send for AudioRecorder {}
unsafe impl Sync for AudioRecorder {}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            is_recording: Arc::new(AtomicBool::new(false)),
            level: Arc::new(AtomicU32::new(0)),
            // 10 seconds at 48kHz — plenty of headroom
            ring_buffer: Arc::new(RingBuffer::new(48000 * 10)),
        }
    }

    pub fn get_input_devices() -> Vec<String> {
        let host = cpal::default_host();
        host.input_devices()
            .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
            .unwrap_or_default()
    }

    pub fn start_recording(&self, output_path: PathBuf, device_name: Option<&str>) -> Result<(), String> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err("Already recording".to_string());
        }

        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => {
                match find_input_device_by_name(name) {
                    Some(d) => d,
                    None => {
                        eprintln!("[audio] Device '{}' not found, falling back to system default", name);
                        host.default_input_device()
                            .ok_or("No input device available")?
                    }
                }
            },
            None => host
                .default_input_device()
                .ok_or("No input device available")?,
        };

        let default_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels() as usize;
        let sample_format = default_config.sample_format();
        let stream_config: StreamConfig = default_config.into();
        eprintln!("[audio] Recording device at {}Hz, {}ch, {:?}", sample_rate, channels, sample_format);

        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let wav_writer = WavWriter::create(&output_path, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        self.is_recording.store(true, Ordering::SeqCst);

        // Writer thread: drains ring buffer to disk (owns the WavWriter)
        let is_rec_writer = self.is_recording.clone();
        let ring_writer = self.ring_buffer.clone();
        thread::spawn(move || {
            let mut writer = wav_writer;
            loop {
                let mut count = 0;
                while let Some(sample) = ring_writer.pop() {
                    let _ = writer.write_sample(sample);
                    count += 1;
                }
                if !is_rec_writer.load(Ordering::SeqCst) {
                    // Drain remaining samples
                    while let Some(sample) = ring_writer.pop() {
                        let _ = writer.write_sample(sample);
                    }
                    let _ = writer.finalize();
                    eprintln!("[audio] Writer thread done");
                    return;
                }
                if count == 0 {
                    thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        });

        // Audio capture thread (owns the cpal Stream)
        let is_rec = self.is_recording.clone();
        let level = self.level.clone();
        let ring = self.ring_buffer.clone();

        thread::spawn(move || {
            let err_fn = |err| eprintln!("Audio stream error: {}", err);

            let stream_result = match sample_format {
                SampleFormat::F32 => build_stream::<f32>(&device, &stream_config, ring, is_rec.clone(), level, channels, err_fn),
                SampleFormat::I16 => build_stream::<i16>(&device, &stream_config, ring, is_rec.clone(), level, channels, err_fn),
                SampleFormat::U16 => build_stream::<u16>(&device, &stream_config, ring, is_rec.clone(), level, channels, err_fn),
                _ => {
                    eprintln!("Unsupported sample format: {:?}", sample_format);
                    is_rec.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to build stream: {}", e);
                    is_rec.store(false, Ordering::SeqCst);
                    return;
                }
            };

            if let Err(e) = stream.play() {
                eprintln!("Failed to play stream: {}", e);
                is_rec.store(false, Ordering::SeqCst);
                return;
            }

            while is_rec.load(Ordering::SeqCst) {
                thread::sleep(std::time::Duration::from_millis(50));
            }

            drop(stream);
        });

        Ok(())
    }

    pub fn stop_recording(&self) -> Result<(), String> {
        self.is_recording.store(false, Ordering::SeqCst);
        // Give writer thread time to drain ring buffer and finalize WAV
        thread::sleep(std::time::Duration::from_millis(500));
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    pub fn get_level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

fn find_input_device_by_name(name: &str) -> Option<Device> {
    let host = cpal::default_host();
    host.input_devices()
        .ok()?
        .find(|d| d.name().ok().as_deref() == Some(name))
}

fn build_stream<T: cpal::Sample + cpal::SizedSample + Send + 'static>(
    device: &Device,
    config: &StreamConfig,
    ring: Arc<RingBuffer>,
    is_recording: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    channels: usize,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                if !is_recording.load(Ordering::Relaxed) {
                    return;
                }
                let mut max_level: f32 = 0.0;
                for frame in data.chunks(channels) {
                    let mut sum: f32 = 0.0;
                    for sample in frame {
                        let s: f32 = cpal::Sample::from_sample(*sample);
                        sum += s;
                    }
                    let mono = sum / channels as f32;
                    let amplitude = mono.abs();
                    if amplitude > max_level {
                        max_level = amplitude;
                    }
                    let sample_i16 = (mono * i16::MAX as f32) as i16;
                    let _ = ring.push(sample_i16); // lock-free!
                }
                level.store(max_level.to_bits(), Ordering::Relaxed);
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("Failed to build input stream: {}", e))?;

    Ok(stream)
}

/// Resample mono i16 samples using linear interpolation.
fn resample_samples(samples: &[i16], from_rate: u32, to_rate: u32) -> Result<Vec<i16>, String> {
    if from_rate == to_rate || samples.is_empty() {
        return Ok(samples.to_vec());
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;

        let s0 = samples[idx] as f64;
        let s1 = if idx + 1 < samples.len() {
            samples[idx + 1] as f64
        } else {
            s0
        };

        let interpolated = s0 + frac * (s1 - s0);
        output.push(interpolated.clamp(-32768.0, 32767.0) as i16);
    }

    eprintln!("[audio] Resampled {} -> {} samples ({}Hz -> {}Hz)", samples.len(), output.len(), from_rate, to_rate);
    Ok(output)
}

/// Merge mic.wav and system.wav into mono audio.wav (mixed).
/// Resamples system audio to match mic sample rate if they differ.
/// Keeps mic.wav and system.wav for debugging.
pub fn merge_to_audio_wav(folder: &Path) -> Result<(), String> {
    let mic_path = folder.join("mic.wav");
    let system_path = folder.join("system.wav");
    let audio_path = folder.join("audio.wav");

    if !mic_path.exists() {
        return Err("Mic recording not found".to_string());
    }

    if system_path.exists() {
        let mic_reader = WavReader::open(&mic_path)
            .map_err(|e| format!("Failed to open mic.wav: {}", e))?;
        let sys_reader = WavReader::open(&system_path)
            .map_err(|e| format!("Failed to open system.wav: {}", e))?;

        let mic_rate = mic_reader.spec().sample_rate;
        let sys_rate = sys_reader.spec().sample_rate;

        let spec = WavSpec {
            channels: 1,
            sample_rate: mic_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = WavWriter::create(&audio_path, spec)
            .map_err(|e| format!("Failed to create audio.wav: {}", e))?;

        let mic_samples: Vec<i16> = mic_reader
            .into_samples::<i16>()
            .filter_map(|s| s.ok())
            .collect();
        let raw_sys_samples: Vec<i16> = sys_reader
            .into_samples::<i16>()
            .filter_map(|s| s.ok())
            .collect();

        let sys_samples = if sys_rate != mic_rate {
            eprintln!(
                "[audio] Resampling system audio from {}Hz to {}Hz ({} samples)",
                sys_rate, mic_rate, raw_sys_samples.len()
            );
            resample_samples(&raw_sys_samples, sys_rate, mic_rate)?
        } else {
            raw_sys_samples
        };

        let max_len = mic_samples.len().max(sys_samples.len());
        for i in 0..max_len {
            let mic = mic_samples.get(i).copied().unwrap_or(0) as i32;
            let sys = sys_samples.get(i).copied().unwrap_or(0) as i32;
            // Sum and soft-clip instead of dividing by 2 (preserves volume)
            let mixed = (mic + sys).clamp(-32768, 32767) as i16;
            writer.write_sample(mixed).map_err(|e| format!("Write error: {}", e))?;
        }

        writer.finalize().map_err(|e| format!("Failed to finalize audio.wav: {}", e))?;
        eprintln!("[audio] Merged to mono audio.wav: {} samples @ {}Hz", max_len, mic_rate);
    } else {
        std::fs::copy(&mic_path, &audio_path)
            .map_err(|e| format!("Failed to copy mic.wav to audio.wav: {}", e))?;
    }

    Ok(())
}
