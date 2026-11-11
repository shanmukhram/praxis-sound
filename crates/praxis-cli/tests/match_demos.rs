//! Writes the Phase 6 blind-listening pairs to `demos/phase-6/` (ignored; run on purpose):
//!
//! ```text
//! cargo test -p praxis-cli --release --test match_demos -- --ignored --nocapture
//! ```
//!
//! For every reference of the matcher's set: the reference render and the matched result, as
//! `pair-NN-a.flac` / `pair-NN-b.flac` in an order shuffled per pair (deterministically), the
//! answer key in `answers/ANSWER_KEY.md` and the scores in `answers/`. Never plays audio.

use praxis_dsp::StereoBuffer;
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_match::analysis::{profile_of, similarity};
use praxis_match::{MatchRequest, match_reference, refset};
use praxis_score::Score;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const SR: u32 = 48_000;

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demos/phase-6")
}

fn render(score: &Score, reg: &Registry) -> StereoBuffer {
    let opts = RenderOptions {
        sample_rate: SR,
        ..RenderOptions::default()
    };
    render_offline(score, reg, &opts)
        .unwrap_or_else(|e| panic!("render {}: {e}", score.title))
        .master
}

/// Whether the reference plays first in pair `i` (a fixed, unpatterned order).
fn reference_first(i: usize) -> bool {
    // Bits of a fixed constant, so the order has no visible rule.
    (0b1011_0010_0110_1101u32 >> (i % 16)) & 1 == 1
}

#[test]
#[ignore = "writes demos/phase-6; run on purpose"]
fn write_blind_review_pairs() {
    let reg = Registry::with_default_packs();
    let dir = out_dir();
    let answers = dir.join("answers");
    std::fs::create_dir_all(&answers).unwrap();
    let refs = refset::references();
    let profiles: Vec<_> = refs
        .iter()
        .map(|r| {
            let audio = render(&r.score, &reg);
            (
                audio.clone(),
                profile_of(&audio, SR, &format!("{}.wav", r.name)).unwrap(),
            )
        })
        .collect();
    let mut key = String::from(
        "# Answer key (Phase 6 blind review)\n\nRead this only after listening.\n\n\
         | Pair | Reference clip | The reference is | Result made with | Similarity to own \
         reference | Nearest other reference | Originality |\n\
         | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for (i, r) in refs.iter().enumerate() {
        let mut req = MatchRequest::new(profiles[i].1.clone());
        req.seed = 7;
        let out = match_reference(&req, &reg).unwrap();
        let result_audio = render(&out.score, &reg);
        let res = profile_of(&result_audio, SR, "result.wav").unwrap();
        let sims: Vec<f32> = profiles.iter().map(|(_, p)| similarity(p, &res)).collect();
        let (oi, other) = sims
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(j, s)| (j, *s))
            .unwrap();
        let pair = i + 1;
        let ref_first = reference_first(i);
        let (a, b) = if ref_first {
            (&profiles[i].0, &result_audio)
        } else {
            (&result_audio, &profiles[i].0)
        };
        praxis_audio_io::encode::write_flac(&dir.join(format!("pair-{pair:02}-a.flac")), a, SR, 24)
            .unwrap();
        praxis_audio_io::encode::write_flac(&dir.join(format!("pair-{pair:02}-b.flac")), b, SR, 24)
            .unwrap();
        std::fs::write(
            answers.join(format!("pair-{pair:02}-reference.score.json")),
            praxis_score::to_json(&r.score),
        )
        .unwrap();
        std::fs::write(
            answers.join(format!("pair-{pair:02}-result.score.json")),
            praxis_score::to_json(&out.score),
        )
        .unwrap();
        writeln!(
            key,
            "| {pair:02} | {} | {} | {} | {:.3} | {:.3} ({}) | {} (melodic {:.2}) |",
            r.name,
            if ref_first { "A" } else { "B" },
            out.made_with,
            sims[i],
            other,
            refs[oi].name,
            if out.originality.passed {
                "passed"
            } else {
                "failed"
            },
            out.originality.melodic_similarity,
        )
        .unwrap();
        println!(
            "pair {pair:02} {:<14} -> {:<32} own {:.3}, nearest other {:.3} ({}), {} renders",
            r.name,
            out.made_with,
            sims[i],
            other,
            refs[oi].name,
            out.iterations.len()
        );
    }
    std::fs::write(answers.join("ANSWER_KEY.md"), key).unwrap();
}
