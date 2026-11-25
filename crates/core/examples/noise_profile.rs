use std::hint::black_box;
use std::process::ExitCode;
use std::time::Instant;

use noiseman_core::{Kind, Voice, gain};

const SR: u32 = 48_000;
const BUFFER_FRAMES: usize = 4096;

fn run(kind: Kind, secs: u64) {
    let mut v = Voice::new(SR, 0x9E37_79B9_7F4A_7C15, kind);
    let mut buf = [0.0f32; BUFFER_FRAMES];
    let total = secs * u64::from(SR);
    let buffers = total.div_ceil(BUFFER_FRAMES as u64);
    let start = Instant::now();
    for _ in 0..buffers {
        // The callback loads KIND and GAIN once per buffer from atomics; black_box stands in for those loads.
        let (k, g) = black_box((kind, gain(50)));
        for s in &mut buf {
            *s = v.step(k, g);
        }
        black_box(&mut buf);
    }
    let elapsed = start.elapsed();
    let samples = buffers * BUFFER_FRAMES as u64;
    let ns = elapsed.as_nanos() as f64 / samples as f64;
    let realtime = samples as f64 / f64::from(SR) / elapsed.as_secs_f64();
    println!("{:<6} {samples} samples in {elapsed:.3?}: {ns:.3} ns/sample, {realtime:.0}x realtime", kind.label());
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let kinds: Vec<Kind> = match args.get(1).map_or("all", String::as_str) {
        "all" => Kind::ALL.to_vec(),
        name => match Kind::parse(name) {
            Some(k) => vec![k],
            None => {
                eprintln!("noise_profile: unknown kind {name:?}; expected white|pink|brown|blue|violet|all");
                return ExitCode::FAILURE;
            }
        },
    };
    let secs = match args.get(2).map(|s| s.parse::<u64>()) {
        None => 600,
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            eprintln!("noise_profile: bad seconds {:?}: {e}", args[2]);
            return ExitCode::FAILURE;
        }
    };
    for kind in kinds {
        run(kind, secs);
    }
    ExitCode::SUCCESS
}
