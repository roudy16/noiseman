//! noiseman's sound, shared by the native player and the web demo: what each noise kind sounds like, how a
//! kind change and a volume change are smoothed, and how a 0-100 volume maps to gain. No I/O, no allocation.

mod noise;
mod voice;

pub use noise::Noise;
pub use voice::Voice;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    Brown,
    White,
    Blue,
    Pink,
    Violet,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Brown, Kind::White, Kind::Blue, Kind::Pink, Kind::Violet];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Brown => "Brown",
            Kind::White => "White",
            Kind::Blue => "Blue",
            Kind::Pink => "Pink",
            Kind::Violet => "Violet",
        }
    }

    pub fn from_u8(v: u8) -> Kind {
        Self::ALL.get(v as usize).copied().unwrap_or(Kind::Brown)
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Self::ALL.into_iter().find(|k| k.label().eq_ignore_ascii_case(s))
    }
}

/// Linear gain for a 0-100 volume: a square law, so the low end of the range stays usable.
pub fn gain(vol: i32) -> f32 {
    let x = vol.clamp(0, 100) as f32 / 100.0;
    x * x
}
