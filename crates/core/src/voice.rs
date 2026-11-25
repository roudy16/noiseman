use crate::{Kind, Noise};

/// How long every gain change takes, mute included; the daemon's stream-drop deadline is derived from it.
pub const FADE_SECS: f32 = 0.1;

/// A noise generator with click-free changes: gain moves to its target in exactly [`FADE_SECS`] along a smoothstep
/// S-curve, and a kind change fades to silence before switching generators.
pub struct Voice {
    noise: Noise,
    playing: Kind,
    level: f32,
    from: f32,
    target: f32,
    left: u32,
    len: u32,
    inv_len: f32,
}

impl Voice {
    pub fn new(sample_rate: u32, seed: u64, kind: Kind) -> Self {
        let len = ((FADE_SECS * sample_rate as f32).round() as u32).max(1);
        Voice {
            noise: Noise::new(sample_rate, seed),
            playing: kind,
            level: 0.0,
            from: 0.0,
            target: 0.0,
            left: 0,
            len,
            inv_len: 1.0 / len as f32,
        }
    }

    /// Writes one sample per item of `out` through `write`, fading towards `want` at `gain` as in [`FADE_SECS`]. The
    /// generator is chosen once per buffer, and again only where a kind change switches it mid-buffer.
    pub fn fill<O>(&mut self, out: impl IntoIterator<Item = O>, want: Kind, gain: f32, mut write: impl FnMut(O, f32)) {
        let mut out = out.into_iter();
        loop {
            let switched = match self.playing {
                Kind::Brown => self.render(&mut out, want, gain, &mut write, |n| n.sample(Kind::Brown)),
                Kind::White => self.render(&mut out, want, gain, &mut write, |n| n.sample(Kind::White)),
                Kind::Blue => self.render(&mut out, want, gain, &mut write, |n| n.sample(Kind::Blue)),
                Kind::Pink => self.render(&mut out, want, gain, &mut write, |n| n.sample(Kind::Pink)),
                Kind::Violet => self.render(&mut out, want, gain, &mut write, |n| n.sample(Kind::Violet)),
            };
            if !switched {
                return;
            }
        }
    }

    /// Renders with `sample`, the playing kind's generator, until `out` ends (false) or the fade switches kinds (true).
    #[inline(always)]
    fn render<O>(
        &mut self,
        out: &mut impl Iterator<Item = O>,
        want: Kind,
        gain: f32,
        write: &mut impl FnMut(O, f32),
        sample: impl Fn(&mut Noise) -> f32,
    ) -> bool {
        for slot in out {
            if self.left > 0 || want != self.playing || gain != self.target {
                let playing = self.playing;
                self.fade(want, gain);
                if self.playing != playing {
                    // The sample that switches already comes from the new generator.
                    write(slot, self.noise.next(self.playing) * self.level);
                    return true;
                }
            }
            write(slot, sample(&mut self.noise) * self.level);
        }
        false
    }

    /// One sample: the per-sample reference `fill` must match bit for bit.
    #[cfg(test)]
    fn step(&mut self, want: Kind, gain: f32) -> f32 {
        if self.left > 0 || want != self.playing || gain != self.target {
            self.fade(want, gain);
        }
        self.noise.next(self.playing) * self.level
    }

    fn fade(&mut self, want: Kind, gain: f32) {
        let target = if want == self.playing { gain } else { 0.0 };
        if target != self.target {
            (self.from, self.target, self.left) = (self.level, target, self.len);
        }
        self.left = self.left.saturating_sub(1);
        // Evaluated from the end (smoothstep is symmetric), so u = 0 on the last sample lands exactly on the target.
        // Direct evaluation, not forward differences: those drift by up to 1.6e-4 of the span in f32.
        let u = self.left as f32 * self.inv_len;
        self.level = target + (self.from - target) * (u * u * (3.0 - 2.0 * u));
        if want != self.playing && self.left == 0 {
            self.playing = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gain;

    fn run(v: &mut Voice, want: Kind, gain: f32, n: u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                v.step(want, gain);
                v.level
            })
            .collect()
    }

    #[test]
    fn fade_length_is_100_ms() {
        assert_eq!(Voice::new(48_000, 1, Kind::White).len, 4800);
        assert_eq!(Voice::new(44_100, 1, Kind::White).len, 4410);
    }

    #[test]
    fn volume_change_is_an_s_curve_that_lands_exactly() {
        let mut v = Voice::new(48_000, 1, Kind::White);
        let levels = run(&mut v, Kind::White, 0.7, 4800);
        assert_eq!(v.level, 0.7);
        let steps: Vec<f32> = levels.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.iter().all(|&d| d >= 0.0), "monotonic");
        let mid = steps[steps.len() / 2];
        assert!(steps[0] < mid / 100.0 && steps[steps.len() - 1] < mid / 100.0, "S-shaped");
    }

    #[test]
    fn kind_change_switches_at_silence() {
        let mut v = Voice::new(48_000, 1, Kind::White);
        run(&mut v, Kind::White, 1.0, 4800);
        for _ in 0..4800 {
            assert!(v.playing == Kind::White || v.level == 0.0);
            v.step(Kind::Brown, 1.0);
        }
        assert_eq!((v.playing, v.level), (Kind::Brown, 0.0));
        run(&mut v, Kind::Brown, 1.0, 4800);
        assert_eq!(v.level, 1.0);
    }

    #[test]
    fn fill_matches_step() {
        // (kind, gain, buffer size, buffers): fades land mid-buffer at 4800 samples unless noted.
        let script = [
            (Kind::Brown, 0.5, 4096, 3),
            (Kind::Brown, 0.2, 127, 10),
            (Kind::Brown, 0.9, 1, 50), // retarget mid-fade
            (Kind::Brown, 0.9, 128, 60),
            (Kind::Pink, 0.9, 4096, 4), // switches at sample 4800, mid second buffer
            (Kind::Violet, 0.9, 127, 100),
            (Kind::White, 0.0, 4096, 2), // kind change and mute at once
            (Kind::White, 0.0, 128, 50),
            (Kind::Blue, gain(70), 4096, 5), // muted, so the switch lands on the buffer's first sample
            (Kind::Blue, 0.0, 4096, 2),
            (Kind::Blue, 1.0, 4096, 3),
        ];
        let (mut r, mut m, mut st) = (
            Voice::new(48_000, 3, Kind::Brown),
            Voice::new(48_000, 3, Kind::Brown),
            Voice::new(48_000, 3, Kind::Brown),
        );
        let (mut mono, mut stereo) = ([0.0f32; 4096], [0.0f32; 8192]);
        for (want, g, n, count) in script {
            for _ in 0..count {
                m.fill(mono[..n].iter_mut(), want, g, |x, s| *x = s);
                st.fill(stereo[..2 * n].chunks_mut(2), want, g, |f, s| f.fill(s));
                for i in 0..n {
                    let s = r.step(want, g).to_bits();
                    assert_eq!([s; 3], [mono[i].to_bits(), stereo[2 * i].to_bits(), stereo[2 * i + 1].to_bits()]);
                }
            }
        }
    }

    #[test]
    fn retarget_mid_fade_is_continuous() {
        let mut v = Voice::new(48_000, 1, Kind::White);
        let mut levels = run(&mut v, Kind::White, 1.0, 2400);
        levels.extend(run(&mut v, Kind::White, 0.0, 4800));
        assert_eq!(v.level, 0.0);
        let max_step = 1.5 / 4800.0 + 1e-6; // smoothstep's peak slope is 1.5 spans per fade length
        assert!(levels.windows(2).all(|w| (w[1] - w[0]).abs() <= max_step));
    }
}
