//! Two raw exports for `site/worklet.js`, no wasm-bindgen: the worklet scope lacks the APIs its glue needs. One synth
//! per module instance in static memory, with no heap, so the module stays a few KB.

use std::cell::UnsafeCell;

use noiseman_core::{Kind, Voice, gain};

/// Largest block `synth_render` fills; Web Audio's render quantum is 128 frames.
const MAX_FRAMES: usize = 1024;

struct Slot<T>(UnsafeCell<T>);

// SAFETY: wasm32-unknown-unknown has one thread, and no reference from `get()` outlives an export call.
unsafe impl<T> Sync for Slot<T> {}

static VOICE: Slot<Option<Voice>> = Slot(UnsafeCell::new(None));
// Kept apart from VOICE, whose `None` is not all zero bytes: an all-zero static costs no bytes in the module.
static BUF: Slot<[f32; MAX_FRAMES]> = Slot(UnsafeCell::new([0.0; MAX_FRAMES]));

#[allow(clippy::mut_from_ref)]
fn get<T>(slot: &'static Slot<T>) -> &'static mut T {
    // SAFETY: see `Slot`; each export takes each reference once and drops it on return.
    unsafe { &mut *slot.0.get() }
}

/// Starts (or restarts) the module's synth.
#[unsafe(no_mangle)]
pub extern "C" fn synth_init(sample_rate: u32, seed: u32, kind: u32) {
    let voice = Voice::new(sample_rate, u64::from(seed) << 16 | 1, Kind::from_u8(kind as u8));
    *get(&VOICE) = Some(voice);
}

/// Renders `frames` (at most MAX_FRAMES) samples of `kind` at volume `vol` (0-100) and returns where they start;
/// silence before `synth_init`.
#[unsafe(no_mangle)]
pub extern "C" fn synth_render(kind: u32, vol: u32, frames: u32) -> *const f32 {
    let buf = get(&BUF);
    let (want, g) = (Kind::from_u8(kind as u8), gain(vol as i32));
    let block = &mut buf[..(frames as usize).min(MAX_FRAMES)];
    match get(&VOICE) {
        Some(v) => v.fill(block, want, g, |x, s| *x = s),
        None => block.fill(0.0),
    }
    buf.as_ptr()
}

#[cfg(test)]
mod tests {
    use noiseman_core::Kind;

    /// app.js sends a kind as its index in KINDS, which `synth_init`/`synth_render` decode with `Kind::from_u8`.
    #[test]
    fn app_js_kinds_match_decode_order() {
        let labels: Vec<String> =
            (0..Kind::ALL.len() as u8).map(|i| format!("\"{}\"", Kind::from_u8(i).label())).collect();
        let line = format!("const KINDS = [{}];", labels.join(", "));
        assert!(include_str!("../site/app.js").contains(&line), "web/site/app.js lacks {line}");
    }
}
