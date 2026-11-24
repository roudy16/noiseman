use std::fmt;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU8, AtomicU32};
use std::time::{SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, FromSample, OutputCallbackInfo, SampleFormat, SampleRate, SizedSample, Stream,
    StreamConfig, SupportedBufferSize,
};

use crate::noise::{Kind, Noise};

pub static GAIN: AtomicU32 = AtomicU32::new(0);
pub static KIND: AtomicU8 = AtomicU8::new(Kind::Brown as u8);

pub const BUFFER_FRAMES: u32 = 4096;

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
            AudioError::Format(fmt) => write!(f, "unsupported sample format {fmt}"),
            AudioError::Cpal(e) => write!(f, "{e}"),
        }
    }
}

struct Voice {
    noise: Noise,
    cur: Kind,
    g: f32,
    k: f32,
}

pub fn start(on_lost: impl Fn() + Send + 'static) -> Result<(Stream, SampleRate), AudioError> {
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
    Ok((stream, config.sample_rate))
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
        cur: Kind::from_u8(KIND.load(Relaxed)),
        g: 0.0,
        k: 1.0 - (-1.0 / (0.01 * sr as f32)).exp(),
    };
    let data = move |data: &mut [T], _: &OutputCallbackInfo| {
        let want = Kind::from_u8(KIND.load(Relaxed));
        let gain = f32::from_bits(GAIN.load(Relaxed));
        for frame in data.chunks_mut(channels) {
            let target = if want != v.cur { 0.0 } else { gain };
            v.g += (target - v.g) * v.k;
            if (target - v.g).abs() < 1e-6 {
                v.g = target;
            }
            if want != v.cur && v.g < 1e-4 {
                v.cur = want;
            }
            frame.fill(T::from_sample(v.noise.next(v.cur) * v.g));
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
