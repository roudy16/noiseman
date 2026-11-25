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
        let next = self.level + (target - self.level) * self.smoothing;
        // Rounding stalls the fade just short of high targets, so also snap once a step stops changing the level.
        self.level = if next == self.level || (target - next).abs() < 1e-6 { target } else { next };
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
        for _ in 0..9_600 {
            v.step(Kind::White, 1.0);
        }
        assert_eq!(v.level, 1.0, "the fade settles exactly");
        let mut quiet = false;
        for _ in 0..9_600 {
            v.step(Kind::Brown, 1.0);
            quiet |= v.level < 1e-4;
        }
        assert!(quiet && v.playing == Kind::Brown && (v.level - 1.0).abs() < 1e-2);
    }
}
