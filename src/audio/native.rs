use anyhow::{Context, Result, bail, ensure};
use audioadapter_buffers::direct::SequentialSliceOfVecs;
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction};
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::mpsc,
};

const RATE: u32 = 44_100;
const CHUNK: usize = 256;
type Frame = [f32; 2];
type Message = io::Result<Vec<u8>>;

pub fn devices() -> Result<serde_json::Value> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.id().ok());
    let mut devices = Vec::new();
    for device in host.input_devices()? {
        let id = device.id()?;
        let config = device.default_input_config();
        devices.push(serde_json::json!({
            "id": id.to_string(), "name": device.description()?.name(),
            "default": default.as_ref() == Some(&id),
            "channels": config.as_ref().ok().map(|c| c.channels()),
            "sample_rate": config.as_ref().ok().map(|c| c.sample_rate()),
            "error": config.err().map(|e| e.to_string()),
        }));
    }
    Ok(serde_json::json!({"backend":host.id().name(), "inputs":devices}))
}

fn supported(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::I8
            | SampleFormat::I16
            | SampleFormat::I32
            | SampleFormat::I64
            | SampleFormat::U8
            | SampleFormat::U16
            | SampleFormat::U32
            | SampleFormat::U64
            | SampleFormat::F32
            | SampleFormat::F64
    )
}

fn configuration(device: &cpal::Device) -> Result<SupportedStreamConfig> {
    // Prefer native 44.1 kHz stereo, then mono. Never silently select surround channels.
    let ranges: Vec<_> = device
        .supported_input_configs()?
        .filter(|r| (1..=2).contains(&r.channels()) && supported(r.sample_format()))
        .collect();
    for channels in [2, 1] {
        for range in &ranges {
            if range.channels() == channels
                && let Some(config) = range.try_with_sample_rate(RATE)
            {
                return Ok(config);
            }
        }
    }
    let default = device.default_input_config()?;
    ensure!(
        (1..=2).contains(&default.channels()) && supported(default.sample_format()),
        "select a mono or stereo PCM input; this device's default format is unsupported"
    );
    ensure!(
        (8_000..=192_000).contains(&default.sample_rate()),
        "input rate must be 8..192 kHz"
    );
    Ok(default)
}

/// Bounded, non-blocking callback. It performs no allocation, I/O or locking.
fn enqueue<T: Sample>(
    data: &[T],
    channels: usize,
    producer: &mut rtrb::Producer<Frame>,
    fault: &AtomicU8,
) where
    f32: FromSample<T>,
{
    if fault.load(Ordering::Relaxed) != 0 {
        return;
    }
    for frame in data.chunks_exact(channels) {
        let left = frame[0].to_sample::<f32>();
        let right = frame[channels - 1].to_sample::<f32>();
        if !left.is_finite() || !right.is_finite() {
            fault.store(3, Ordering::Relaxed);
            return;
        }
        if producer.push([left, right]).is_err() {
            // Fail explicitly instead of dropping samples or accumulating unbounded delay.
            fault.store(1, Ordering::Relaxed);
            return;
        }
    }
}

fn build<T: SizedSample>(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    mut producer: rtrb::Producer<Frame>,
    fault: Arc<AtomicU8>,
) -> Result<cpal::Stream>
where
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels());
    let callback_fault = fault.clone();
    Ok(device.build_input_stream(
        config.config(),
        move |data: &[T], _| {
            enqueue(data, channels, &mut producer, &callback_fault);
        },
        move |_| {
            fault.store(2, Ordering::Relaxed);
        },
        Some(Duration::from_secs(5)),
    )?)
}

/// Conversion is off the realtime callback. No resampling at 44.1 kHz.
struct Converter {
    resampler: Option<Async<f32>>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
}
impl Converter {
    fn new(rate: u32) -> Result<Self> {
        let resampler = if rate == RATE {
            None
        } else {
            Some(Async::new_sinc(
                f64::from(RATE) / f64::from(rate),
                1.0,
                &SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2),
                CHUNK,
                2,
                FixedAsync::Input,
            )?)
        };
        let max = resampler.as_ref().map_or(CHUNK, |r| r.output_frames_max());
        Ok(Self {
            resampler,
            input: vec![vec![0.0; CHUNK]; 2],
            output: vec![vec![0.0; max]; 2],
        })
    }
    fn convert(&mut self, frames: &[Frame]) -> Result<Vec<u8>> {
        ensure!(frames.len() == CHUNK, "invalid native conversion block");
        if let Some(resampler) = &mut self.resampler {
            for (i, frame) in frames.iter().enumerate() {
                self.input[0][i] = frame[0];
                self.input[1][i] = frame[1];
            }
            let input = SequentialSliceOfVecs::new(&self.input, 2, CHUNK)?;
            let max = self.output[0].len();
            let mut output = SequentialSliceOfVecs::new_mut(&mut self.output, 2, max)?;
            let (_, produced) = resampler.process_into_buffer(&input, &mut output, None)?;
            let mut bytes = Vec::with_capacity(produced * 4);
            for i in 0..produced {
                bytes.extend(pcm(self.output[0][i]));
                bytes.extend(pcm(self.output[1][i]));
            }
            Ok(bytes)
        } else {
            Ok(frames
                .iter()
                .flat_map(|frame| frame.iter().flat_map(|sample| pcm(*sample)))
                .collect())
        }
    }
}
fn pcm(sample: f32) -> [u8; 2] {
    (sample
        .mul_add(32768.0, 0.0)
        .round()
        .clamp(-32768.0, 32767.0) as i16)
        .to_be_bytes()
}

pub struct Capture {
    rx: mpsc::Receiver<Message>,
    pending: Vec<u8>,
    position: usize,
    stop: Arc<AtomicBool>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl AsyncRead for Capture {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.position == self.pending.len() {
            match self.rx.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(bytes))) => {
                    self.pending = bytes;
                    self.position = 0;
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Err(error)),
                Poll::Ready(None) => {
                    return Poll::Ready(Err(io::Error::other("native capture ended")));
                }
            }
        }
        let count = buf.remaining().min(self.pending.len() - self.position);
        buf.put_slice(&self.pending[self.position..self.position + count]);
        self.position += count;
        Poll::Ready(Ok(()))
    }
}
impl Capture {
    pub fn open(id: String) -> Result<Self> {
        ensure!(!id.is_empty(), "input device must not be empty");
        let (tx, rx) = mpsc::channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let result = run(id, &tx, &worker_stop, ready_tx);
                if let Err(error) = result {
                    let _ = tx.blocking_send(Err(io::Error::other(format!("{error:#}"))));
                }
            })?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                rx,
                pending: Vec::new(),
                position: 0,
                stop,
            }),
            Ok(Err(error)) => {
                stop.store(true, Ordering::Relaxed);
                bail!(error)
            }
            Err(error) => {
                stop.store(true, Ordering::Relaxed);
                bail!("native capture initialization failed: {error}")
            }
        }
    }
}

fn run(
    id: String,
    tx: &mpsc::Sender<Message>,
    stop: &AtomicBool,
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<()> {
    // Construct, play and destroy the platform stream on its owning thread.
    let setup = (|| -> Result<_> {
        let host = cpal::default_host();
        let device = if id == "default" {
            host.default_input_device()
                .context("no default input device; check audio permissions")?
        } else {
            host.device_by_id(&id.parse().context("invalid input ID; use audio-inputs")?)
                .context("input device unavailable; run audio-inputs again")?
        };
        let config = configuration(&device)?;
        let rate = config.sample_rate();
        let converter = Converter::new(rate)?;
        // 50 ms capacity is a ceiling for scheduler stalls, not a prefill target.
        let (producer, consumer) = rtrb::RingBuffer::new(rate as usize / 20);
        let fault = Arc::new(AtomicU8::new(0));
        macro_rules! stream {
            ($t:ty) => {
                build::<$t>(&device, &config, producer, fault.clone())?
            };
        }
        let stream = match config.sample_format() {
            SampleFormat::I8 => stream!(i8),
            SampleFormat::I16 => stream!(i16),
            SampleFormat::I32 => stream!(i32),
            SampleFormat::I64 => stream!(i64),
            SampleFormat::U8 => stream!(u8),
            SampleFormat::U16 => stream!(u16),
            SampleFormat::U32 => stream!(u32),
            SampleFormat::U64 => stream!(u64),
            SampleFormat::F32 => stream!(f32),
            SampleFormat::F64 => stream!(f64),
            format => bail!("unsupported PCM format: {format}"),
        };
        stream
            .play()
            .context("start native input; check microphone permission and device availability")?;
        eprintln!(
            "Starting native capture on {} ({}, {rate} Hz, {} channels; PCM S16BE/44100/stereo)",
            device.description()?.name(),
            host.id().name(),
            config.channels()
        );
        Ok((stream, consumer, fault, converter))
    })();
    let (stream, mut consumer, fault, mut converter) = match setup {
        Ok(parts) => {
            let _ = ready.send(Ok(()));
            parts
        }
        Err(error) => {
            let _ = ready.send(Err(format!("{error:#}")));
            return Ok(());
        }
    };
    let mut frames = Vec::with_capacity(CHUNK);
    let mut last_input = Instant::now();
    while !stop.load(Ordering::Relaxed) && !tx.is_closed() {
        match fault.load(Ordering::Relaxed) {
            1 => bail!("native capture overrun; stopping instead of losing samples"),
            2 => bail!("native audio device failed or disconnected"),
            3 => bail!("native input contains non-finite samples"),
            _ => {}
        }
        while frames.len() < CHUNK {
            match consumer.pop() {
                Ok(frame) => {
                    frames.push(frame);
                    last_input = Instant::now();
                }
                Err(_) => break,
            }
        }
        if frames.len() == CHUNK {
            let bytes = converter.convert(&frames)?;
            if !bytes.is_empty() && tx.blocking_send(Ok(bytes)).is_err() {
                break;
            }
            frames.clear();
        } else {
            ensure!(
                last_input.elapsed() < Duration::from_secs(5),
                "native audio input stalled for five seconds"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    drop(stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_44100_preserves_pcm_and_channels() {
        let frames: Vec<_> = (0..CHUNK)
            .map(|i| [((i as i16) - 128) as f32 / 32768.0, -0.5])
            .collect();
        let bytes = Converter::new(RATE).unwrap().convert(&frames).unwrap();
        for (i, frame) in bytes.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(i16::from_be_bytes([frame[0], frame[1]]), i as i16 - 128);
            assert_eq!(i16::from_be_bytes([frame[2], frame[3]]), -16384);
        }
        assert_eq!(pcm(2.0), i16::MAX.to_be_bytes());
        assert_eq!(pcm(-2.0), i16::MIN.to_be_bytes());
    }
    #[test]
    fn mono_duplicates_and_overrun_is_reported() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(2);
        let fault = AtomicU8::new(0);
        enqueue(&[16384_i16, -16384, 1], 1, &mut producer, &fault);
        assert_eq!(consumer.pop().unwrap(), [0.5, 0.5]);
        assert_eq!(consumer.pop().unwrap(), [-0.5, -0.5]);
        assert_eq!(fault.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn resampling_48000_preserves_duration_pitch_and_stereo() {
        let mut converter = Converter::new(48000).unwrap();
        let mut bytes = Vec::new();
        let blocks = 375; // Exactly two seconds.
        for block in 0..blocks {
            let frames: Vec<_> = (0..CHUNK)
                .map(|i| {
                    let value = (std::f32::consts::TAU * 1000.0 * (block * CHUNK + i) as f32
                        / 48000.0)
                        .sin()
                        * 0.5;
                    [value, -value]
                })
                .collect();
            bytes.extend(converter.convert(&frames).unwrap());
        }
        let pcm: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                let l = i16::from_be_bytes([b[0], b[1]]);
                let r = i16::from_be_bytes([b[2], b[3]]);
                assert!((i32::from(l) + i32::from(r)).abs() <= 1);
                l
            })
            .collect();
        assert!(pcm.len().abs_diff(2 * RATE as usize) < 256);
        let crossings = pcm[1000..]
            .windows(2)
            .filter(|w| w[0] <= 0 && w[1] > 0)
            .count();
        let frequency = crossings as f64 * f64::from(RATE) / (pcm.len() - 1000) as f64;
        assert!((frequency - 1000.0).abs() < 2.0, "{frequency}");
    }
}
