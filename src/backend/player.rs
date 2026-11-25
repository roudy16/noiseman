use std::sync::Arc;
use std::time::{Duration, Instant};

use super::audio;
use crate::protocol::{Command, State};

/// Playback state that every frontend sees: volume, mute and the audio stream's lifetime.
pub struct Player {
    on_lost: Arc<dyn Fn() + Send + Sync>,
    stream: Option<audio::Output>,
    drop_stream_at: Option<Instant>,
    vol: i32,
    muted: bool,
    error: Option<String>,
}

impl Player {
    /// Starts playing at once unless `muted`. `on_lost` runs on the audio thread when the device or stream goes away;
    /// it must get [`Player::audio_lost`] called on the player's thread.
    pub fn new(on_lost: Arc<dyn Fn() + Send + Sync>, muted: bool) -> Player {
        let mut p = Player { on_lost, stream: None, drop_stream_at: None, vol: 50, muted: true, error: None };
        p.set_muted(muted);
        p
    }

    pub fn state(&self) -> State {
        State { kind: audio::kind(), vol: self.vol, muted: self.muted, error: self.error.clone() }
    }

    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Kind(k) => audio::set_kind(k),
            Command::Vol(v) => self.set_vol(v),
            Command::VolBy(d) => self.set_vol(self.vol.saturating_add(d)),
            Command::Mute(m) => self.set_muted(m),
            Command::ToggleMute => self.set_muted(!self.muted),
            Command::Quit => self.set_muted(true),
            Command::Status | Command::Watch | Command::Protocol => {}
        }
    }

    pub fn audio_lost(&mut self) {
        self.stream = None;
        if !self.muted {
            self.play();
        }
    }

    /// When [`Player::tick`] next has work: dropping the stream once a mute has faded out.
    pub fn deadline(&self) -> Option<Instant> {
        self.drop_stream_at
    }

    pub fn tick(&mut self, now: Instant) {
        if self.drop_stream_at.is_some_and(|t| now >= t) {
            self.stream = None;
            self.drop_stream_at = None;
        }
    }

    fn play(&mut self) {
        let on_lost = self.on_lost.clone();
        match audio::start(move || on_lost()) {
            Ok(s) => {
                self.stream = Some(s);
                self.error = None;
            }
            Err(e) => {
                log!("noiseman: cannot start audio: {e}");
                self.error = Some(format!("audio error: {e}"));
                self.muted = true;
                self.apply_gain();
            }
        }
    }

    fn apply_gain(&self) {
        audio::set_gain(if self.muted { 0.0 } else { noiseman_core::gain(self.vol) });
    }

    fn set_vol(&mut self, v: i32) {
        self.vol = v.clamp(0, 100);
        self.apply_gain();
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_gain();
        if muted {
            if self.stream.is_some() && self.drop_stream_at.is_none() {
                let queued = self.stream.as_ref().map_or(Duration::ZERO, audio::Output::buffer_time);
                // The callback picks up the gain within a buffer, fades for FADE_SECS, and up to 3 more buffers are
                // queued in the device; 20 ms of slack for scheduling.
                let fade = Duration::from_secs_f32(noiseman_core::FADE_SECS) + Duration::from_millis(20);
                self.drop_stream_at = Some(Instant::now() + fade + 4 * queued);
            }
        } else {
            self.drop_stream_at = None;
            if self.stream.is_none() {
                self.play();
            }
        }
    }
}
