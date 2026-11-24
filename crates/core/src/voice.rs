use crate::{Kind, Noise};

const FADE_SECS: f32 = 0.01;

/// A noise generator with click-free changes: gain moves with a ~10 ms one-pole fade, and a kind change fades to
/// silence before switching generators.
pub struct Voice {
    noise: Noise,
    playing: Kind,
    level: f32,
    smoothing: f32,
}

impl Voice {
    pub fn new(sample_rate: u32, seed: u64, kind: Kind) -> Self {
        Voice {
            noise: Noise::new(sample_rate, seed),
            playing: kind,
            level: 0.0,
            smoothing: 1.0 - (-1.0 / (FADE_SECS * sample_rate as f32)).exp(),
        }
    }

    #[inline]
    pub fn step(&mut self, want: Kind, gain: f32) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_change_fades_through_silence() {
        let mut v = Voice::new(48_000, 1, Kind::White);
        for _ in 0..4_800 {
            v.step(Kind::White, 1.0);
        }
        assert!((v.level - 1.0).abs() < 1e-3);
        let mut quiet = false;
        for _ in 0..9_600 {
            v.step(Kind::Brown, 1.0);
            quiet |= v.level < 1e-4;
        }
        assert!(quiet && v.playing == Kind::Brown && (v.level - 1.0).abs() < 1e-2);
    }
}
