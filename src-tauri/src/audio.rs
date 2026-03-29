use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use hound::{WavReader, WavSpec, WavWriter};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

/// Thread-safe audio recorder. The cpal Stream lives on a dedicated thread
/// since it isn't Send/Sync on all platforms.
pub struct AudioRecorder {
    is_recording: Arc<Mutex<bool>>,
    level: Arc<Mutex<f32>>,
    writer: Arc<Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>>,
}

unsafe impl Send for AudioRecorder {}
unsafe impl Sync for AudioRecorder {}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            is_recording: Arc::new(Mutex::new(false)),
            level: Arc::new(Mutex::new(0.0)),
            writer: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get_input_devices() -> Vec<String> {
        let host = cpal::default_host();
        host.input_devices()
            .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
            .unwrap_or_default()
    }

    /// Start recording from the specified device (or default if None).
    pub fn start_recording(&self, output_path: PathBuf, device_name: Option<&str>) -> Result<(), String> {
        if *self.is_recording.lock().unwrap() {
            return Err("Already recording".to_string());
        }

        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => find_input_device_by_name(name)
                .ok_or_else(|| format!("Audio device '{}' not found", name))?,
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
        eprintln!("[audio] Recording from device at {}Hz, {} channels", sample_rate, channels);

        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let wav_writer = WavWriter::create(&output_path, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        *self.writer.lock().unwrap() = Some(wav_writer);
        *self.is_recording.lock().unwrap() = true;

        let is_rec = self.is_recording.clone();
        let level = self.level.clone();
        let writer = self.writer.clone();

        // Spawn a thread that owns the cpal Stream (not Send on some platforms)
        thread::spawn(move || {
            let err_fn = |err| eprintln!("Audio stream error: {}", err);

            let stream_result = match sample_format {
                SampleFormat::F32 => {
                    build_stream::<f32>(&device, &stream_config, writer, is_rec.clone(), level, channels, err_fn)
                }
                SampleFormat::I16 => {
                    build_stream::<i16>(&device, &stream_config, writer, is_rec.clone(), level, channels, err_fn)
                }
                SampleFormat::U16 => {
                    build_stream::<u16>(&device, &stream_config, writer, is_rec.clone(), level, channels, err_fn)
                }
                _ => {
                    eprintln!("Unsupported sample format");
                    *is_rec.lock().unwrap() = false;
                    return;
                }
            };

            let stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to build stream: {}", e);
                    *is_rec.lock().unwrap() = false;
                    return;
                }
            };

            if let Err(e) = stream.play() {
                eprintln!("Failed to play stream: {}", e);
                *is_rec.lock().unwrap() = false;
                return;
            }

            // Keep thread alive while recording — stream is dropped when we exit
            while *is_rec.lock().unwrap() {
                thread::sleep(std::time::Duration::from_millis(50));
            }

            drop(stream);
        });

        Ok(())
    }

    pub fn stop_recording(&self) -> Result<(), String> {
        *self.is_recording.lock().unwrap() = false;
        // Give the recording thread time to stop and flush
        thread::sleep(std::time::Duration::from_millis(300));
        // Finalize the WAV file
        if let Some(writer) = self.writer.lock().unwrap().take() {
            writer
                .finalize()
                .map_err(|e| format!("Failed to finalize WAV: {}", e))?;
        }
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        *self.is_recording.lock().unwrap()
    }

    pub fn get_level(&self) -> f32 {
        *self.level.lock().unwrap()
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
    writer: Arc<Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>>,
    is_recording: Arc<Mutex<bool>>,
    level: Arc<Mutex<f32>>,
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
                if !*is_recording.lock().unwrap() {
                    return;
                }
                let mut max_level: f32 = 0.0;
                if let Some(ref mut writer) = *writer.lock().unwrap() {
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
                        let _ = writer.write_sample(sample_i16);
                    }
                }
                *level.lock().unwrap() = max_level;
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("Failed to build input stream: {}", e))?;

    Ok(stream)
}

/// Resample mono i16 samples using linear interpolation (simple, robust).
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

/// Merge mic.wav and system.wav into a stereo audio.wav (left=mic, right=system).
/// Resamples system audio to match mic sample rate if they differ.
/// If only mic.wav exists, copies it to audio.wav (mono).
/// Keeps mic.wav and system.wav for debugging.
pub fn merge_to_audio_wav(folder: &Path) -> Result<(), String> {
    let mic_path = folder.join("mic.wav");
    let system_path = folder.join("system.wav");
    let audio_path = folder.join("audio.wav");

    if !mic_path.exists() {
        return Err("Mic recording not found".to_string());
    }

    if system_path.exists() {
        // Merge to stereo: left=mic, right=system
        let mic_reader = WavReader::open(&mic_path)
            .map_err(|e| format!("Failed to open mic.wav: {}", e))?;
        let sys_reader = WavReader::open(&system_path)
            .map_err(|e| format!("Failed to open system.wav: {}", e))?;

        let mic_rate = mic_reader.spec().sample_rate;
        let sys_rate = sys_reader.spec().sample_rate;

        let spec = WavSpec {
            channels: 2,
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

        // Resample system audio to match mic sample rate if needed
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
            let mic = mic_samples.get(i).copied().unwrap_or(0);
            let sys = sys_samples.get(i).copied().unwrap_or(0);
            writer.write_sample(mic).map_err(|e| format!("Write error: {}", e))?;
            writer.write_sample(sys).map_err(|e| format!("Write error: {}", e))?;
        }

        writer.finalize().map_err(|e| format!("Failed to finalize audio.wav: {}", e))?;

        // Keep mic.wav and system.wav for debugging
    } else {
        // No system audio — copy mic.wav to audio.wav (keep original for debugging)
        std::fs::copy(&mic_path, &audio_path)
            .map_err(|e| format!("Failed to copy mic.wav to audio.wav: {}", e))?;
    }

    Ok(())
}
