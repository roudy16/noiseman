//! Two raw exports for `site/worklet.js`, no wasm-bindgen: the worklet scope lacks the APIs its glue needs. One synth
//! per module instance in static memory, with no heap, so the module stays a few KB.

use std::cell::UnsafeCell;

use noiseman_core::{Kind, Voice, gain};

/// Largest block `synth_render` fills; Web Audio's render quantum is 128 frames.
const MAX_FRAMES: usize = 1024;

struct Synth {
    voice: Option<Voice>,
    buf: [f32; MAX_FRAMES],
}

struct Slot(UnsafeCell<Synth>);

// SAFETY: wasm32-unknown-unknown has one thread, and no reference from `synth()` outlives an export call.
unsafe impl Sync for Slot {}

static SYNTH: Slot = Slot(UnsafeCell::new(Synth { voice: None, buf: [0.0; MAX_FRAMES] }));

#[allow(clippy::mut_from_ref)]
fn synth() -> &'static mut Synth {
    // SAFETY: see `Slot`; each export takes this reference once and drops it on return.
    unsafe { &mut *SYNTH.0.get() }
}

/// Starts (or restarts) the module's synth.
#[unsafe(no_mangle)]
pub extern "C" fn synth_init(sample_rate: u32, seed: u32, kind: u32) {
    let voice = Voice::new(sample_rate, u64::from(seed) << 16 | 1, Kind::from_u8(kind as u8));
    synth().voice = Some(voice);
}

/// Renders `frames` (at most MAX_FRAMES) samples of `kind` at volume `vol` (0-100) and returns where they start;
/// silence before `synth_init`.
#[unsafe(no_mangle)]
pub extern "C" fn synth_render(kind: u32, vol: u32, frames: u32) -> *const f32 {
    let s = synth();
    let (want, g) = (Kind::from_u8(kind as u8), gain(vol as i32));
    let block = &mut s.buf[..(frames as usize).min(MAX_FRAMES)];
    match &mut s.voice {
        Some(v) => block.iter_mut().for_each(|x| *x = v.step(want, g)),
        None => block.fill(0.0),
    }
    s.buf.as_ptr()
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
