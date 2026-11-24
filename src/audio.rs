use std::fmt;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU8, AtomicU32};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, FromSample, OutputCallbackInfo, SampleFormat, SampleRate, SizedSample, Stream,
    StreamConfig, SupportedBufferSize,
};

use crate::noise::{Kind, Noise};

static GAIN: AtomicU32 = AtomicU32::new(0);
static KIND: AtomicU8 = AtomicU8::new(Kind::Brown as u8);

const BUFFER_FRAMES: u32 = 4096;
const FADE_SECS: f32 = 0.01;

pub fn kind() -> Kind {
    Kind::from_u8(KIND.load(Relaxed))
}

pub fn set_kind(k: Kind) {
    KIND.store(k as u8, Relaxed)
}

pub fn set_gain(g: f32) {
    GAIN.store(g.to_bits(), Relaxed)
}

#[derive(Debug)]
pub enum AudioError {
    NoDevice,
    Format(SampleFormat),
    Cpal(cpal::Error),
}

impl From<cpal::Error> for AudioError {
    fn from(e: cpal::Error) -> Self {
        AudioError::Cpal(e)
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::NoDevice => f.write_str("no default output device"),
            AudioError::Format(format) => write!(f, "unsupported sample format {format}"),
            AudioError::Cpal(e) => write!(f, "{e}"),
        }
    }
}

pub struct Output {
    stream: Stream,
    rate: SampleRate,
}

impl Output {
    /// One device buffer, truncated to whole milliseconds.
    pub fn buffer_time(&self) -> Duration {
        let frames = self.stream.buffer_size().unwrap_or_else(|e| {
            eprintln!("noiseman: stream buffer size unavailable, assuming {BUFFER_FRAMES}: {e}");
            BUFFER_FRAMES
        });
        Duration::from_millis(u64::from(frames) * 1000 / u64::from(self.rate))
    }
}

pub fn start(on_lost: impl Fn() + Send + 'static) -> Result<Output, AudioError> {
    let device = cpal::default_host().default_output_device().ok_or(AudioError::NoDevice)?;
    let supported = device.default_output_config()?;
    let mut config = supported.config();
    config.buffer_size = match *supported.buffer_size() {
        SupportedBufferSize::Range { min, max } => BufferSize::Fixed(BUFFER_FRAMES.max(min).min(max)),
        SupportedBufferSize::Unknown => BufferSize::Default,
    };
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, on_lost)?,
        SampleFormat::I16 => build::<i16>(&device, config, on_lost)?,
        SampleFormat::U16 => build::<u16>(&device, config, on_lost)?,
        SampleFormat::I32 => build::<i32>(&device, config, on_lost)?,
        SampleFormat::I24 => build::<cpal::I24>(&device, config, on_lost)?,
        other => return Err(AudioError::Format(other)),
    };
    stream.play()?;
    Ok(Output { stream, rate: config.sample_rate })
}

struct Voice {
    noise: Noise,
    playing: Kind,
    level: f32,
    smoothing: f32,
}

impl Voice {
    #[inline]
    fn step(&mut self, want: Kind, gain: f32) -> f32 {
        // A kind change fades to silence before switching generators, so it does not click.
        let target = if want == self.playing { gain } else { 0.0 };
        self.level += (target - self.level) * self.smoothing;
        if (target - self.level).abs() < 1e-6 {
            self.level = target;
        }
        if want != self.playing && self.level < 1e-4 {
            self.playing = want;
        }
        self.noise.next(self.playing) * self.level
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: StreamConfig,
    on_lost: impl Fn() + Send + 'static,
) -> Result<Stream, cpal::Error> {
    let channels = config.channels as usize;
    let sr = config.sample_rate;
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut v = Voice {
        noise: Noise::new(sr, seed),
        playing: kind(),
        level: 0.0,
        smoothing: 1.0 - (-1.0 / (FADE_SECS * sr as f32)).exp(),
    };
    let data = move |data: &mut [T], _: &OutputCallbackInfo| {
        let want = kind();
        let gain = f32::from_bits(GAIN.load(Relaxed));
        for frame in data.chunks_mut(channels) {
            frame.fill(T::from_sample(v.step(want, gain)));
        }
    };
    let error = move |e: cpal::Error| {
        eprintln!("noiseman: audio stream error ({:?}): {e}", e.kind());
        if matches!(e.kind(), ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated) {
            on_lost();
        }
    };
    device.build_output_stream(config, data, error, None)
}
