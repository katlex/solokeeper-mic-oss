use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use hound::{WavSpec, WavWriter};
use std::io::BufWriter;
use std::path::PathBuf;
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

    pub fn start_recording(&self, output_path: PathBuf) -> Result<(), String> {
        if *self.is_recording.lock().unwrap() {
            return Err("Already recording".to_string());
        }

        // Set up WAV writer and device config on the main thread so we can
        // return errors synchronously.
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No input device available")?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();
        let stream_config: StreamConfig = config.into();

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
