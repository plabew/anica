use std::{
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use cpal::{
    Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

pub struct AudioRecorder {
    stream: Option<Stream>,
    writer: Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>,
    sample_count: Arc<AtomicU64>,
    channels: u16,
    sample_rate: u32,
    started_at: Instant,
    output_path: PathBuf,
    device_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AudioRecordingResult {
    pub path: PathBuf,
    pub duration: Duration,
    pub device_name: Option<String>,
}

impl AudioRecorder {
    pub fn start(output_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(output_dir)
            .map_err(|err| format!("Failed to create recording directory: {err}"))?;

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "No default microphone input device found.".to_string())?;
        let device_name = device.name().ok();
        let supported = device
            .default_input_config()
            .map_err(|err| format!("Failed to read microphone config: {err}"))?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let channels = config.channels.max(1);
        let sample_rate = config.sample_rate.0.max(1);

        let output_path = output_dir.join(format!("voice_recording_{}.wav", unix_millis()));
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(&output_path, spec)
            .map_err(|err| format!("Failed to create WAV file: {err}"))?;
        let writer = Arc::new(Mutex::new(Some(writer)));
        let sample_count = Arc::new(AtomicU64::new(0));

        let err_fn = |err| eprintln!("[AudioRecorder] input stream error: {err}");
        let stream = match sample_format {
            SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, &writer, &sample_count, err_fn)
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, &writer, &sample_count, err_fn)
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, &writer, &sample_count, err_fn)
            }
            other => Err(format!("Unsupported microphone sample format: {other:?}")),
        }?;
        stream
            .play()
            .map_err(|err| format!("Failed to start microphone stream: {err}"))?;

        Ok(Self {
            stream: Some(stream),
            writer,
            sample_count,
            channels,
            sample_rate,
            started_at: Instant::now(),
            output_path,
            device_name,
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn stop(mut self) -> Result<AudioRecordingResult, String> {
        self.stream.take();
        let writer = self
            .writer
            .lock()
            .map_err(|_| "Failed to lock WAV writer.".to_string())?
            .take();
        if let Some(writer) = writer {
            writer
                .finalize()
                .map_err(|err| format!("Failed to finalize WAV file: {err}"))?;
        }

        let frames = self.sample_count.load(Ordering::Relaxed) / u64::from(self.channels.max(1));
        let duration = if frames > 0 {
            Duration::from_secs_f64(frames as f64 / self.sample_rate as f64)
        } else {
            self.elapsed()
        };

        Ok(AudioRecordingResult {
            path: self.output_path,
            duration: duration.max(Duration::from_millis(1)),
            device_name: self.device_name,
        })
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    writer: &Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>,
    sample_count: &Arc<AtomicU64>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, String>
where
    T: Sample + SizedSample + Send + 'static,
    i16: cpal::FromSample<T>,
{
    let writer = Arc::clone(writer);
    let sample_count = Arc::clone(sample_count);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if let Ok(mut guard) = writer.lock()
                    && let Some(writer) = guard.as_mut()
                {
                    for sample in data {
                        let value: i16 = sample.to_sample::<i16>();
                        let _ = writer.write_sample(value);
                    }
                }
                sample_count.fetch_add(data.len() as u64, Ordering::Relaxed);
            },
            err_fn,
            None,
        )
        .map_err(|err| format!("Failed to build microphone input stream: {err}"))
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
