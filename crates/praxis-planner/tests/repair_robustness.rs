//! Generated broken plans: deterministic repairs always yield a Score that validates, or an
//! `Invalid` error with issues; repairs are idempotent. (A seeded xorshift generator stands in for
//! proptest, which is not a dependency of this crate.)

use praxis_planner::plan::{PlanContext, validate_plan};
use praxis_planner::repair::fix_plan;
use praxis_planner::{Catalogue, PlanError, ScorePlan, compose_plan};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

fn example() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("prompts")
        .join("acceptance1.plan.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("plan")).expect("json")
}

fn mutate(rng: &mut Rng, plan: &mut Value) {
    let presets = [
        "felt_piano",
        "bell",
        "warm_bass",
        "soft_keys",
        "pluck",
        "kick",
        "hat_closed",
        "granular_texture",
        "riser",
        "nope",
    ];
    let kinds = ["sampler", "synth", "drum", "sfx"];
    let classes = [
        "motif", "melody", "bass", "drums", "pad", "chords", "riser", "arp",
    ];
    let grids = [
        "quarter",
        "eighth",
        "sixteenth",
        "eighth_triplet",
        "sixteenth_triplet",
    ];
    let spokens = [
        "DON din don din TIN din don din",
        "DON ~ ~ - don",
        "BOOM din",
        "~ DON",
        "- - - -",
        "",
        "DON din DON don TIN DON din DON don TIN DON din DON don TIN DON din DON",
    ];
    for _ in 0..(1 + rng.below(6)) {
        match rng.below(14) {
            0 => plan["tempo_bpm"] = json!(*rng.pick(&[0.0, 10.0, 90.0, 128.0, 500.0, -3.0])),
            1 => plan["duration_s"] = json!(*rng.pick(&[1.0, 30.0, 60.0, 600.0])),
            2 => plan["bars"] = json!(*rng.pick(&[0u32, 1, 7, 16, 40, 9000])),
            3 => {
                plan["time_signature"] = json!({
                    "numerator": *rng.pick(&[0u8, 3, 4, 5, 6, 7, 40]),
                    "denominator": *rng.pick(&[3u8, 4, 8, 16]),
                })
            }
            4 => plan["emotion"] = json!(*rng.pick(&["calm", "very sad and happy", "", "x1"])),
            5 => {
                let li = rng.below(2) as usize;
                plan["layers"][li]["preset"] =
                    json!({"kind": *rng.pick(&kinds), "name": *rng.pick(&presets)});
            }
            6 => {
                let li = rng.below(2) as usize;
                plan["layers"][li]["class"] = json!(*rng.pick(&classes));
            }
            7 => {
                let li = rng.below(2) as usize;
                plan["layers"][li]["motif"]["grid"] = json!(*rng.pick(&grids));
                plan["layers"][li]["motif"]["length_bars"] = json!(rng.below(12));
            }
            8 => {
                let li = rng.below(2) as usize;
                plan["layers"][li]["motif"]["spoken"] = json!(*rng.pick(&spokens));
            }
            9 => {
                let li = rng.below(2) as usize;
                let notes: Vec<Value> = (0..rng.below(5))
                    .map(|_| {
                        json!({
                            "step": rng.below(40) as f64 + if rng.chance(30) { 0.5 } else { 0.0 },
                            "pitch": rng.below(140) as i64 - 6,
                            "velocity": *rng.pick(&[0.5, 0.0, 1.5]),
                        })
                    })
                    .collect();
                plan["layers"][li]["motif"]["notes"] = json!(notes);
            }
            10 => {
                let a = rng.below(20) as u32;
                let b = rng.below(20) as u32;
                let si = rng.below(2) as usize;
                plan["sections"][si]["bars"] = json!({"start": a, "end": b});
                plan["sections"][si]["intensity"] = json!(*rng.pick(&[0.2, 3.0, -1.0]));
            }
            11 => {
                let si = rng.below(2) as usize;
                plan["sections"][si]["layers"] = json!([*rng.pick(&["piano", "kick", "ghost"])]);
            }
            12 => {
                plan["chords"] = json!([
                    {"bars": {"start": rng.below(20), "end": rng.below(20)},
                     "symbol": *rng.pick(&["Am", "C#", "F", "Xy", "Bdim", "Eb/G"])}
                ])
            }
            _ => {
                let li = rng.below(2) as usize;
                plan["layers"][li]["id"] = json!(*rng.pick(&["Piano!", "kick", "piano", ""]));
                plan["layers"][li]["motif"]["sustain_steps"] = json!(*rng.pick(&[0.5, 1.0, 99.0]));
                plan["layers"][li]["motif"]["degrees"] =
                    json!([rng.below(60) as i64 - 30, rng.below(5)]);
            }
        }
    }
}

#[test]
fn repairs_always_end_valid_or_explain() {
    let catalogue = Catalogue::standard();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut ok, mut invalid) = (0, 0);
    for case in 0..400 {
        let mut plan = example();
        mutate(&mut rng, &mut plan);
        match compose_plan(&plan, &catalogue, case) {
            Ok(out) => {
                let report = praxis_score::validate::validate(&out.score);
                assert!(
                    report.is_valid(),
                    "case {case}: {:?}\nplan: {plan}",
                    report.errors
                );
                ok += 1;
                // Idempotent: the repaired plan needs no further repairs.
                let requested: BTreeSet<_> = out.plan.requested.iter().copied().collect();
                let ctx = PlanContext {
                    catalogue: &catalogue,
                    requested: &requested,
                };
                assert!(validate_plan(&out.plan, &ctx).is_empty(), "case {case}");
                let (again, repairs): (ScorePlan, _) = fix_plan(&out.plan, &ctx);
                assert!(repairs.is_empty(), "case {case}: {repairs:?}");
                assert_eq!(again, out.plan);
            }
            Err(PlanError::Invalid(issues)) => {
                assert!(!issues.is_empty(), "case {case}");
                assert!(issues.iter().all(|i| !i.message.is_empty()));
                println!(
                    "case {case} invalid: {}",
                    praxis_planner::plan::issues_text(&issues)
                );
                invalid += 1;
            }
            Err(e) => panic!("case {case}: unexpected error {e}\nplan: {plan}"),
        }
    }
    println!("repaired to valid: {ok}, explained as invalid: {invalid}");
    assert!(ok >= 380, "most broken plans should be repairable ({ok})");
}
