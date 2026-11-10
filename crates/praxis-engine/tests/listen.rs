//! Writes the acceptance-1 bell render to WAV files for listening tests (not committed).
//!
//! `cargo test -p praxis-engine --release --test listen -- --ignored --nocapture`
//!
//! Output directory: `$PRAXIS_LISTEN_DIR`, or `<system temp>/praxis-engine-listen`. Files:
//! `acceptance1_bell_master.wav` (one loop cycle), `acceptance1_bell_x3.wav` (three cycles back
//! to back, to hear the seam), and one stem per layer. 32-bit float, 48 kHz, stereo.

mod common;

use common::*;
use praxis_dsp::StereoBuffer;
use std::path::PathBuf;

#[test]
#[ignore = "writes WAV files for listening; run explicitly"]
fn write_acceptance1_bell_wavs() {
    let dir = std::env::var_os("PRAXIS_LISTEN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("praxis-engine-listen"));
    let out = render(&bell_score(), &opts(true));
    let master = dir.join("acceptance1_bell_master.wav");
    write_wav_f32(&master, &out.master, SR).unwrap();
    let mut x3 = StereoBuffer::default();
    for _ in 0..3 {
        x3.left.extend_from_slice(&out.master.left);
        x3.right.extend_from_slice(&out.master.right);
    }
    write_wav_f32(&dir.join("acceptance1_bell_x3.wav"), &x3, SR).unwrap();
    for s in &out.stems {
        write_wav_f32(
            &dir.join(format!("acceptance1_bell_stem_{}.wav", s.layer_id)),
            &s.audio,
            SR,
        )
        .unwrap();
    }
    println!("wrote {} (and _x3, stems)", master.display());
    for w in &out.report.warnings {
        println!("warning: {w}");
    }
}
