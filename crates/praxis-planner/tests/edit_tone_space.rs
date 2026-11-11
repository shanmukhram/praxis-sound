//! Tone and space edits (WP-P2b): "make the piano muffled" keeps every note and changes the
//! sound; "darker in the last four bars" changes only those bars of the render; "more room"
//! raises the reverb; an instruction the rules cannot read changes nothing and says what is
//! possible; the Feel sliders' eight phrases map to their documented intents; a layer id selects
//! exactly that layer.
//!
//! Renders use the real engine (effects, mix bus, limiter) with the engine's test instruments.
//! The sampled `felt_piano` needs praxis-instruments and its sample pack, which this crate does
//! not depend on, so a test stand-in plays the piano layer: the engine's test bell through the
//! felt-hammer low-pass that `sampler/presets.rs` documents (cutoff `850 Hz x 2^(3 tone) x
//! 2^-(felt - 0.5)`), honouring the same `tone` / `felt` / `release_s` parameters.

mod common;

use common::SR;
use praxis_engine::testing::{TestBell, TestFactory};
use praxis_engine::{
    Instrument, InstrumentContext, InstrumentError, InstrumentFactory, NoteOn, PresetInfo,
    PresetParam, RenderOptions, RenderOutput, render_offline,
};
use praxis_planner::edit::resolve::layers_matching;
use praxis_planner::edit::{
    EditIntent, SpaceChange, ToneChange, compose_edit, plan_edit, plan_project_edit,
};
use praxis_planner::{Catalogue, EditOutcome, RuleProvider, plan_brief};
use praxis_score::apply::apply;
use praxis_score::patch::PatchOp;
use praxis_score::project::Project;
use praxis_score::timeline::{TimeMap, expand_notes, score_length_samples};
use praxis_score::{
    EffectKind, GeneratorClass, InstrumentKind, InstrumentRef, Score, validate::validate,
};
use serde_json::json;

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

fn acceptance1() -> Score {
    plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score
}

fn edit(score: &Score, instruction: &str) -> EditOutcome {
    plan_edit(
        score,
        instruction,
        &RuleProvider::new(),
        &Catalogue::standard(),
        5,
    )
    .unwrap_or_else(|e| panic!("{instruction:?}: {e}"))
}

/// Notes are identical, the edited Score is valid and the inverse restores it exactly.
fn keeps_notes_and_undoes(before: &Score, out: &EditOutcome) {
    assert_eq!(
        expand_notes(before, SR),
        expand_notes(&out.score, SR),
        "{}: the notes changed",
        out.summary
    );
    assert_eq!(before.patterns, out.score.patterns);
    for (a, b) in before.layers.iter().zip(&out.score.layers) {
        assert_eq!(a.clips, b.clips, "clips of `{}` changed", a.id);
    }
    let report = validate(&out.score);
    assert!(report.is_valid(), "{:?}", report.errors);
    assert_eq!(
        apply(&out.score, &out.inverse).expect("undo").score,
        *before
    );
}

// ---- rendering ----

/// `felt_piano` stand-in: the test bell through two one-pole low-passes at the felt-hammer
/// cutoff of `felt_voicing` (middle C, mid velocity).
struct FeltStandIn {
    bell: TestBell,
    sr: f32,
    tone: f32,
    felt: f32,
    coef: f32,
    st: [[f32; 2]; 2],
}

impl FeltStandIn {
    fn new(sr: f32, tone: f32, felt: f32, release_s: f32) -> FeltStandIn {
        let mut s = FeltStandIn {
            bell: TestBell::new(sr, 1.0, release_s),
            sr,
            tone,
            felt,
            coef: 0.0,
            st: [[0.0; 2]; 2],
        };
        s.update();
        s
    }

    fn cutoff(&self) -> f32 {
        850.0 * 2f32.powf(3.0 * self.tone) * 2f32.powf(-(self.felt - 0.5))
    }

    fn update(&mut self) {
        let fc = self.cutoff().min(self.sr * 0.45);
        self.coef = 1.0 - (-std::f32::consts::TAU * fc / self.sr).exp();
    }
}

impl Instrument for FeltStandIn {
    fn note_on(&mut self, note: &NoteOn) {
        self.bell.note_on(note);
    }
    fn note_off(&mut self, id: u64) {
        self.bell.note_off(id);
    }
    fn all_notes_off(&mut self) {
        self.bell.all_notes_off();
    }
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value.clamp(0.0, 1.0),
            1 => self.felt = value.clamp(0.0, 1.0),
            2 => self.bell.set_param(1, value),
            _ => {}
        }
        self.update();
    }
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.bell.process(left, right);
        let a = self.coef;
        for (ch, buf) in [left, right].into_iter().enumerate() {
            let st = &mut self.st[ch];
            for x in buf.iter_mut() {
                st[0] += a * (*x - st[0]);
                st[1] += a * (st[0] - st[1]);
                *x = if st[1].abs() > 1e-30 { st[1] } else { 0.0 };
            }
        }
    }
    fn tail_samples(&self) -> usize {
        self.bell.tail_samples() + (0.02 * self.sr) as usize
    }
    fn reset(&mut self) {
        self.bell.reset();
        self.st = [[0.0; 2]; 2];
    }
}

fn felt_info() -> PresetInfo {
    let p = |name: &str, min: f64, max: f64, default: f64| PresetParam {
        name: name.into(),
        min,
        max,
        default,
        unit: String::new(),
    };
    PresetInfo {
        kind: InstrumentKind::Sampler,
        name: "felt_piano".into(),
        class: GeneratorClass::Motif,
        pitched: true,
        sustaining: true,
        range: (21, 108),
        params: vec![
            p("tone", 0.0, 1.0, 0.5),
            p("felt", 0.0, 1.0, 0.7),
            p("release_s", 0.05, 4.0, 1.2),
        ],
        source: "test stand-in for felt_piano (not a product preset)".into(),
    }
}

/// The engine's test instruments plus the `felt_piano` stand-in.
struct StandIns(TestFactory);

impl InstrumentFactory for StandIns {
    fn create(
        &self,
        spec: &InstrumentRef,
        ctx: &InstrumentContext,
    ) -> Result<Box<dyn Instrument>, InstrumentError> {
        if spec.kind == InstrumentKind::Sampler && spec.preset == "felt_piano" {
            let get = |n: &str, d: f64| spec.params.get(n).copied().unwrap_or(d) as f32;
            return Ok(Box::new(FeltStandIn::new(
                ctx.sample_rate,
                get("tone", 0.5),
                get("felt", 0.7),
                get("release_s", 1.2),
            )));
        }
        self.0.create(spec, ctx)
    }
    fn describe(&self, kind: InstrumentKind, preset: &str) -> Option<PresetInfo> {
        if kind == InstrumentKind::Sampler && preset == "felt_piano" {
            return Some(felt_info());
        }
        self.0.describe(kind, preset)
    }
    fn presets(&self) -> Vec<PresetInfo> {
        let mut v = self.0.presets();
        v.push(felt_info());
        v
    }
}

/// Drums play the engine's test kick (no parameters); the felt piano stays itself.
fn on_stand_ins(score: &mut Score) {
    for l in &mut score.layers {
        if l.instrument.kind == InstrumentKind::Drum {
            l.instrument.preset = "soft_kick".into();
            l.instrument.params.clear();
        }
    }
}

/// Every pitched layer plays the test bell (for isolation tests: the instrument is not edited).
fn on_test_instruments(score: &mut Score) {
    for l in &mut score.layers {
        if l.instrument.kind == InstrumentKind::Drum {
            l.instrument.preset = "soft_kick".into();
        } else {
            l.instrument.kind = InstrumentKind::Synth;
            l.instrument.preset = "bell".into();
        }
        l.instrument.params.clear();
    }
}

fn opts() -> RenderOptions {
    RenderOptions {
        sample_rate: SR,
        stems: true,
        ..RenderOptions::default()
    }
}

fn render(score: &Score, factory: &dyn InstrumentFactory) -> RenderOutput {
    render_offline(score, factory, &opts()).expect("render")
}

/// Mono mix of a layer's stem.
fn stem_mono(out: &RenderOutput, id: &str) -> Vec<f32> {
    let s = out
        .stems
        .iter()
        .find(|s| s.layer_id == id)
        .unwrap_or_else(|| panic!("no stem {id}"));
    s.audio
        .left
        .iter()
        .zip(&s.audio.right)
        .map(|(l, r)| 0.5 * (l + r))
        .collect()
}

/// Spectral centroid in Hz: magnitude-weighted mean frequency over Hann-windowed 2048-sample
/// frames (a plain DFT), skipping near-silent frames.
fn centroid(x: &[f32]) -> f64 {
    const N: usize = 2048;
    let tau = std::f64::consts::TAU;
    let cos: Vec<f64> = (0..N).map(|i| (tau * i as f64 / N as f64).cos()).collect();
    let sin: Vec<f64> = (0..N).map(|i| (tau * i as f64 / N as f64).sin()).collect();
    let hann: Vec<f64> = (0..N).map(|i| 0.5 - 0.5 * cos[i]).collect();
    let frames = 24;
    let stride = (x.len().saturating_sub(N) / frames).max(1);
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for f in 0..frames {
        let start = f * stride;
        if start + N > x.len() {
            break;
        }
        let frame: Vec<f64> = (0..N).map(|i| x[start + i] as f64 * hann[i]).collect();
        if frame.iter().map(|v| v * v).sum::<f64>() < 1e-9 {
            continue;
        }
        for k in 1..N / 2 {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, v) in frame.iter().enumerate() {
                let j = (k * n) % N;
                re += v * cos[j];
                im -= v * sin[j];
            }
            let mag = (re * re + im * im).sqrt();
            num += mag * k as f64 * SR as f64 / N as f64;
            den += mag;
        }
    }
    assert!(den > 0.0, "the stem is silent");
    num / den
}

// ---- tone ----

#[test]
fn make_the_piano_muffled_keeps_every_note_and_darkens_the_render() {
    let before = acceptance1();
    let out = edit(&before, "make the piano muffled");
    println!("make the piano muffled -> {}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::ChangeTone);
    assert_eq!(out.plan.params.tone, Some(ToneChange::Darker));
    assert!(out.understood() && out.changed());
    assert_eq!(out.patch.scope.bars, None);
    assert_eq!(out.patch.scope.layers, ["piano"]);
    keeps_notes_and_undoes(&before, &out);
    assert_eq!(out.score.layer("kick"), before.layer("kick"));
    // The felt piano's own controls moved: darker filter, more felt.
    let params = &out.score.layer("piano").expect("piano").instrument.params;
    assert_eq!(params.get("tone"), Some(&0.2));
    assert_eq!(params.get("felt"), Some(&1.0));
    assert!(
        out.summary.contains("darker") && out.summary.contains("notes are unchanged"),
        "{}",
        out.summary
    );
    assert!(!out.summary.contains("motif"), "{}", out.summary);

    let factory = StandIns(TestFactory::new());
    let (mut a, mut b) = (before.clone(), out.score.clone());
    on_stand_ins(&mut a);
    on_stand_ins(&mut b);
    let c0 = centroid(&stem_mono(&render(&a, &factory), "piano"));
    let c1 = centroid(&stem_mono(&render(&b, &factory), "piano"));
    println!("piano stem spectral centroid: {c0:.0} Hz -> {c1:.0} Hz (muffled)");
    assert!(c1 < 0.95 * c0, "centroid {c0:.0} -> {c1:.0} Hz");

    // "brighter" from the original raises it; from the muffled take it moves back up.
    let bright = edit(&before, "make the piano brighter");
    assert_eq!(bright.plan.params.tone, Some(ToneChange::Brighter));
    keeps_notes_and_undoes(&before, &bright);
    let mut c = bright.score.clone();
    on_stand_ins(&mut c);
    let c2 = centroid(&stem_mono(&render(&c, &factory), "piano"));
    println!("piano stem spectral centroid: {c0:.0} Hz -> {c2:.0} Hz (brighter)");
    assert!(c2 > 1.03 * c0, "centroid {c0:.0} -> {c2:.0} Hz");
    let back = edit(&out.score, "make the piano brighter");
    let p = &back.score.layer("piano").expect("piano").instrument.params;
    assert_eq!((p.get("tone"), p.get("felt")), (Some(&0.5), Some(&0.7)));
}

#[test]
fn a_preset_without_tone_controls_gets_a_low_pass_and_brighter_takes_it_off() {
    let mut s = acceptance1();
    // A drone has no tone parameters.
    let l = s
        .layers
        .iter_mut()
        .find(|l| l.id == "piano")
        .expect("piano");
    l.instrument = InstrumentRef {
        kind: InstrumentKind::Sfx,
        preset: "drone".into(),
        params: Default::default(),
    };
    l.class = GeneratorClass::Drone;
    let out = edit(&s, "make the piano a lot darker");
    keeps_notes_and_undoes(&s, &out);
    let fx = &out.score.layer("piano").expect("piano").effects[0];
    assert_eq!((fx.id.as_str(), fx.kind), ("tone_lp", EffectKind::Filter));
    assert_eq!(fx.params.get("cutoff_hz"), Some(&1500.0));
    // Brighter opens the low-pass, and takes it off once it is wide open.
    let cutoff = |s: &Score| {
        s.layer("piano")
            .expect("piano")
            .effects
            .iter()
            .find(|e| e.id == "tone_lp")
            .and_then(|e| e.params.get("cutoff_hz").copied())
    };
    let mut cur = out.score.clone();
    let mut seen = vec![cutoff(&cur)];
    for _ in 0..3 {
        cur = edit(&cur, "make the piano much brighter").score;
        seen.push(cutoff(&cur));
    }
    println!("tone low-pass cutoffs: {seen:?}");
    assert!(seen[1] > seen[0]);
    assert_eq!(seen.last(), Some(&None));
}

/// Window bookkeeping of contracts/patch.md "Audio-level isolation" for an edit of `layer` in
/// bars from `bar`: the end of the wrapped window and the first sample (less the limiter
/// lookahead) of the edited bars.
fn isolation_window(
    after: &Score,
    factory: &dyn InstrumentFactory,
    layer_id: &str,
    bar: u32,
) -> (usize, usize) {
    let block = opts().block_size;
    let len = score_length_samples(after, SR);
    let li = after
        .layers
        .iter()
        .position(|l| l.id == layer_id)
        .expect("layer");
    let overhang = expand_notes(after, SR)
        .into_iter()
        .filter(|n| n.layer == li)
        .map(|n| (n.start_sample + n.length_samples).saturating_sub(len))
        .max()
        .unwrap_or(0) as usize;
    let layer = after.layer(layer_id).expect("layer");
    let ctx = InstrumentContext {
        sample_rate: SR as f32,
        max_block: block,
        key: after.key,
        bpm: after.tempo.bpm,
        seed: 0,
    };
    let inst_tail = factory
        .create(&layer.instrument, &ctx)
        .expect("instrument")
        .tail_samples();
    let mut w = Vec::new();
    let chain = praxis_engine::effects::EffectChain::build(
        &layer.effects,
        &praxis_engine::effects::BuiltinEffects,
        SR as f32,
        block,
        after.tempo.bpm as f32,
        layer_id,
        &mut w,
    );
    let master_tail = praxis_engine::master::isolation_tail_samples(
        &after.master,
        SR,
        block,
        after.tempo.bpm as f32,
    ) as usize;
    let lookahead =
        praxis_engine::master::limiter_latency_samples(&after.master.loudness, SR, block) as usize;
    let window_end =
        overhang + inst_tail + chain.tail_samples() + chain.latency_samples() + master_tail;
    let start = TimeMap::new(after, SR).bar_start_sample(bar) as usize;
    (window_end, start - lookahead)
}

fn same(a: &RenderOutput, b: &RenderOutput, i: usize) -> bool {
    a.master.left[i].to_bits() == b.master.left[i].to_bits()
        && a.master.right[i].to_bits() == b.master.right[i].to_bits()
}

/// Largest absolute master difference over samples `from..to`.
fn max_diff(a: &RenderOutput, b: &RenderOutput, from: usize, to: usize) -> f32 {
    (from..to.min(a.master.len()))
        .map(|i| {
            (a.master.left[i] - b.master.left[i])
                .abs()
                .max((a.master.right[i] - b.master.right[i]).abs())
        })
        .fold(0.0, f32::max)
}

#[test]
fn darker_in_the_last_four_bars_leaves_bars_1_to_12_of_the_master_bit_identical() {
    let mut before = acceptance1();
    let out = edit(&before, "make the piano darker in the last four bars");
    println!("{}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::ChangeTone);
    assert_eq!(out.bars, praxis_score::BarRange { start: 13, end: 16 });
    // Layer settings cannot change in a bar-scoped patch, so the patch has no bars; the step
    // lane holds the low-pass to bars 13-16.
    assert_eq!(out.patch.scope.bars, None);
    keeps_notes_and_undoes(&before, &out);
    let piano = out.score.layer("piano").expect("piano");
    let lane = &piano.automation[0];
    assert_eq!(lane.target, "effects.tone_b13_16.mix");
    let points: Vec<(f64, f64)> = lane.points.iter().map(|p| (p.bar, p.value)).collect();
    assert_eq!(points, [(1.0, 0.0), (13.0, 1.0)]);
    assert_eq!(piano.effects[0].params.get("mix"), Some(&0.0));
    assert!(
        out.summary.contains("Bars 1-12 are unchanged"),
        "{}",
        out.summary
    );

    let mut after = out.score.clone();
    on_test_instruments(&mut before);
    on_test_instruments(&mut after);
    let factory = TestFactory::new();
    let first = render(&before, &factory);
    let gain = first.report.applied_gain_db;
    before.master.frozen_gain_db = Some(gain);
    after.master.frozen_gain_db = Some(gain);
    let a = render(&before, &factory);
    let b = render(&after, &factory);
    let kick = |o: &RenderOutput| {
        let s = o.stems.iter().find(|s| s.layer_id == "kick").expect("kick");
        (s.audio.left.clone(), s.audio.right.clone())
    };
    assert_eq!(kick(&a), kick(&b), "the kick stem changed");

    let (window_end, bar13) = isolation_window(&after, &factory, "piano", 13);
    assert!(window_end < bar13);
    let first_diff = (window_end..a.master.len()).find(|&i| !same(&a, &b, i));
    let peak = max_diff(&a, &b, window_end, bar13);
    println!(
        "darker in bars 13-16: window wraps to sample {window_end}; first master difference \
         after it at {first_diff:?}; bar 13 (less lookahead) at {bar13}; largest difference \
         between them {:.1} dBFS",
        20.0 * peak.max(1e-30).log10()
    );
    assert!(first_diff.is_some(), "bars 13-16 sound darker");
    // The piano's room reverb is a feedback network: what the filter changed in bars 13-16
    // recirculates past its -90 dB tail, which contracts/patch.md allows below -120 dBFS.
    assert!(
        peak < 1e-6,
        "master differs by {peak:e} inside bars 1-12 and outside the window"
    );

    // Without the recirculating reverb the filter is exactly transparent: bit-identical.
    let dry = |s: &Score| {
        let mut s = s.clone();
        for l in &mut s.layers {
            l.effects.retain(|e| e.kind != EffectKind::Reverb);
        }
        s
    };
    let (a2, b2) = (
        render(&dry(&before), &factory),
        render(&dry(&after), &factory),
    );
    let (window2, _) = isolation_window(&dry(&after), &factory, "piano", 13);
    let first2 = (window2..a2.master.len()).find(|&i| !same(&a2, &b2, i));
    println!("without the reverb: window ends at {window2}; first difference at {first2:?}");
    let first2 = first2.expect("bars 13-16 sound darker");
    assert!(
        first2 >= bar13,
        "master differs at sample {first2}, inside bars 1-12 and outside the window"
    );
    // And the darker bars really are darker.
    let tail = |o: &RenderOutput| stem_mono(o, "piano")[bar13..].to_vec();
    let (c0, c1) = (centroid(&tail(&a)), centroid(&tail(&b)));
    println!("piano stem centroid over bars 13-16: {c0:.0} Hz -> {c1:.0} Hz");
    assert!(c1 < 0.95 * c0, "centroid {c0:.0} -> {c1:.0} Hz");
}

// ---- space ----

fn room_mix(s: &Score, layer: &str) -> Option<f64> {
    s.layer(layer)?
        .effects
        .iter()
        .find(|e| e.kind == EffectKind::Reverb)
        .map(|e| e.params.get("mix").copied().unwrap_or(0.18))
}

#[test]
fn more_room_raises_the_reverb_and_changes_no_notes() {
    let before = acceptance1();
    let m0 = room_mix(&before, "piano").expect("the arranger gives the piano a room");
    let out = edit(&before, "Give it more room");
    println!("Give it more room -> {}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::ChangeSpace);
    assert_eq!(out.plan.params.space, Some(SpaceChange::Wetter));
    keeps_notes_and_undoes(&before, &out);
    let m1 = room_mix(&out.score, "piano").expect("room");
    assert!(m1 > m0, "mix {m0} -> {m1}");
    // The kick has no reverb and is a drum: it is not given one unless named.
    assert_eq!(out.score.layer("kick"), before.layer("kick"));

    let drier = edit(&before, "Make it drier");
    keeps_notes_and_undoes(&before, &drier);
    assert!(room_mix(&drier.score, "piano").expect("room") < m0);
    let dry = edit(&before, "make the piano completely dry");
    keeps_notes_and_undoes(&before, &dry);
    assert_eq!(room_mix(&dry.score, "piano"), None);
    // Nothing to dry on a layer without reverb: no change, and it says so.
    let none = edit(&dry.score, "make it drier");
    assert!(!none.changed(), "{}", none.summary);
    assert!(
        none.summary.contains("reverb to reduce"),
        "{}",
        none.summary
    );
    // A named drum gets a room.
    let kick = edit(&before, "give the kick more room");
    assert!(room_mix(&kick.score, "kick").is_some(), "{}", kick.summary);
}

#[test]
fn more_room_in_some_bars_is_a_lane_on_the_reverb_mix() {
    let before = acceptance1();
    let out = edit(&before, "more reverb on the piano in the last four bars");
    assert_eq!(out.plan.intent, EditIntent::ChangeSpace);
    keeps_notes_and_undoes(&before, &out);
    let piano = out.score.layer("piano").expect("piano");
    let m0 = room_mix(&before, "piano").expect("room");
    let lane = piano.automation.last().expect("lane");
    assert!(lane.target.ends_with(".mix"), "{}", lane.target);
    assert_eq!(lane.points[0].value, m0);
    assert_eq!(lane.points[1].bar, 13.0);
    assert!(lane.points[1].value > m0);
    assert_eq!(room_mix(&out.score, "piano"), Some(m0));
}

// ---- no silent rewrites ----

#[test]
fn an_instruction_the_rules_cannot_read_changes_nothing_and_says_what_is_possible() {
    let before = acceptance1();
    for instruction in ["make it pop", "piano", "change the piano"] {
        let out = edit(&before, instruction);
        println!("{instruction:?} -> {}", out.summary);
        assert_eq!(out.plan.intent, EditIntent::NotUnderstood);
        assert!(!out.understood());
        assert!(!out.changed());
        assert!(out.patch.ops.is_empty() && out.inverse.ops.is_empty());
        assert_eq!(out.score, before);
        assert!(
            out.summary.starts_with("Nothing changed"),
            "{}",
            out.summary
        );
        assert!(out.summary.contains(instruction), "{}", out.summary);
        assert!(
            out.summary.contains("darker") && out.summary.contains("more or less room"),
            "{}",
            out.summary
        );
    }
    // A project is not committed to.
    let mut project = Project::new(before.clone());
    let out = plan_project_edit(
        &mut project,
        "make it pop",
        &RuleProvider::new(),
        &Catalogue::standard(),
        0,
    )
    .expect("edit");
    assert!(!out.changed());
    assert!(!project.can_undo());
    // A plan written with `not_understood` is answered the same way.
    let out = compose_edit(
        &json!({"intent": "not_understood", "notes": ["no edit asked for"]}),
        &before,
        &Catalogue::standard(),
        0,
    )
    .expect("compose");
    assert!(!out.changed());
    assert!(out.summary.contains("no edit asked for"), "{}", out.summary);
}

// ---- layer references ----

#[test]
fn a_layer_id_selects_only_that_layer() {
    let mut s = acceptance1();
    let mut upright = s.layer("piano").expect("piano").clone();
    upright.id = "upright_piano".into();
    upright.name = "Upright piano".into();
    upright.instrument.preset = "upright_piano".into();
    s.layers.push(upright);
    let ids = |r: &str| -> Vec<String> {
        layers_matching(r, &s)
            .iter()
            .map(|l| l.id.clone())
            .collect()
    };
    assert_eq!(ids("piano"), ["piano"]);
    assert_eq!(ids("Piano"), ["piano"]);
    assert_eq!(ids("upright_piano"), ["upright_piano"]);
    assert_eq!(ids("upright piano"), ["upright_piano"]);
    assert_eq!(ids("upright"), ["upright_piano"]);
    // Loose references still reach every layer they describe.
    assert_eq!(ids("pianos"), ["piano", "upright_piano"]);
    assert_eq!(ids("keys"), ["piano", "upright_piano"]);
    // So "make the piano muffled" leaves the upright alone.
    let out = edit(&s, "make the piano muffled");
    assert_eq!(out.patch.scope.layers, ["piano"]);
    assert_eq!(out.score.layer("upright_piano"), s.layer("upright_piano"));
}

// ---- the Feel sliders ----

/// Every phrase the app's Feel sliders send (app/src/ui/views/overview.ts `MACROS`), on the
/// acceptance Score: Brightness changes tone, Space changes reverb, Mood and Energy change
/// dynamics. None of them changes a note.
#[test]
fn every_feel_slider_phrase_makes_its_documented_change() {
    let before = acceptance1();
    let cases = [
        ("Mood", "Make the mood calmer", EditIntent::ChangeDynamics),
        (
            "Mood",
            "Make the mood more tense",
            EditIntent::ChangeDynamics,
        ),
        (
            "Brightness",
            "Make it a little darker",
            EditIntent::ChangeTone,
        ),
        (
            "Brightness",
            "Make it a little brighter",
            EditIntent::ChangeTone,
        ),
        ("Space", "Make it drier", EditIntent::ChangeSpace),
        ("Space", "Give it more room", EditIntent::ChangeSpace),
        ("Energy", "Calmer, less energy", EditIntent::ChangeDynamics),
        ("Energy", "More energy", EditIntent::ChangeDynamics),
    ];
    for (slider, phrase, intent) in cases {
        let out = edit(&before, phrase);
        println!("{slider}: {phrase:?} -> {}", out.summary);
        assert_eq!(out.plan.intent, intent, "{phrase}");
        assert!(out.changed(), "{phrase}: {}", out.summary);
        assert_eq!(out.patch.scope.bars, None, "{phrase}");
        keeps_notes_and_undoes(&before, &out);
        assert!(
            out.patch
                .ops
                .iter()
                .all(|op| matches!(op, PatchOp::UpdateLayer { .. })),
            "{phrase}: {:?}",
            out.patch.ops
        );
    }
}
