#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    White,
    Pink,
    Brown,
    Blue,
    Violet,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::White, Kind::Pink, Kind::Brown, Kind::Blue, Kind::Violet];

    pub fn label(self) -> &'static str {
        match self {
            Kind::White => "White",
            Kind::Pink => "Pink",
            Kind::Brown => "Brown",
            Kind::Blue => "Blue",
            Kind::Violet => "Violet",
        }
    }

    pub fn from_u8(v: u8) -> Kind {
        Self::ALL.get(v as usize).copied().unwrap_or(Kind::White)
    }
}

const WHITE_GAIN: f32 = 0.218;
const PINK_GAIN: f32 = 0.0715;
const BLUE_GAIN: f32 = 0.120;
const VIOLET_GAIN: f32 = 0.154;

pub struct Noise {
    rng: u64,
    pink: [f32; 7],
    brown: f32,
    brown_a: f32,
    brown_b: f32,
    prev: f32,
}

impl Noise {
    pub fn new(sample_rate: u32, seed: u64) -> Self {
        let a = (-2.0 * std::f32::consts::PI * 15.3 / sample_rate as f32).exp();
        Noise {
            rng: seed | 1,
            pink: [0.0; 7],
            brown: 0.0,
            brown_a: a,
            brown_b: 0.0138 * ((1.0 - a * a) / (1.0 - 0.998_f32 * 0.998)).sqrt(),
            prev: 0.0,
        }
    }

    fn white(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u32 << 23) as f32 - 1.0
    }

    // Paul Kellet's "refined" pink filter.
    fn pink_raw(&mut self) -> f32 {
        let w = self.white();
        let b = &mut self.pink;
        b[0] = 0.99886 * b[0] + w * 0.0555179;
        b[1] = 0.99332 * b[1] + w * 0.0750759;
        b[2] = 0.96900 * b[2] + w * 0.153852;
        b[3] = 0.86650 * b[3] + w * 0.3104856;
        b[4] = 0.55000 * b[4] + w * 0.5329522;
        b[5] = -0.7616 * b[5] - w * 0.0168980;
        let p = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
        b[6] = w * 0.115926;
        p
    }

    pub fn next(&mut self, kind: Kind) -> f32 {
        let s = match kind {
            Kind::White => self.white() * WHITE_GAIN,
            Kind::Pink => self.pink_raw() * PINK_GAIN,
            Kind::Brown => {
                let w = self.white();
                self.brown = self.brown_a * self.brown + self.brown_b * w;
                self.brown
            }
            Kind::Blue => {
                let p = self.pink_raw();
                let d = p - self.prev;
                self.prev = p;
                d * BLUE_GAIN
            }
            Kind::Violet => {
                let w = self.white();
                let d = w - self.prev;
                self.prev = w;
                d * VIOLET_GAIN
            }
        };
        s.clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_and_bounds() {
        const SR: u32 = 48_000;
        for kind in Kind::ALL {
            let mut n = Noise::new(SR, 0x9E37_79B9_7F4A_7C15);
            let (mut sum, mut sq) = (0.0f64, 0.0f64);
            let len = SR as usize * 10;
            for _ in 0..len {
                let s = n.next(kind);
                assert!(s.is_finite() && (-1.0..=1.0).contains(&s), "{kind:?}: {s}");
                sum += s as f64;
                sq += (s as f64).powi(2);
            }
            let mean = sum / len as f64;
            let db = 10.0 * (sq / len as f64).log10();
            let mean_bound = if kind == Kind::Brown { 0.03 } else { 0.01 };
            assert!((db + 18.0).abs() <= 1.5, "{kind:?}: {db:.2} dBFS");
            assert!(mean.abs() < mean_bound, "{kind:?}: mean {mean}");
        }
    }
}
