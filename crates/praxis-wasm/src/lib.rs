//! praxis-wasm: the browser build of the engine and analyzer (Phase 8, WP-W).
//!
//! The same crates the desktop runs, compiled to WebAssembly and exported through
//! `wasm-bindgen` (`wasm-bindgen --target web`; the app loads the glue from
//! `app/src/engine-wasm/pkg/`):
//!
//! - free functions: [`validate_score`], [`plan_brief`] and [`compose_plan`] (the offline
//!   planner), [`plan_edit`] (the offline edit rules, optionally held to bars of one layer),
//!   [`apply_patch`], [`verify`] and [`analyse`];
//! - [`WasmEngine`] (`Engine` in JS): pack files in memory, presets and offline renders
//!   ([`Rendered`]) with their checks;
//! - [`WasmRealtime`] (`Realtime` in JS): the real-time engine an AudioWorklet runs, block by
//!   block, with no allocation per block.
//!
//! Strings in and out are JSON. Results are `{ok: true, ...}` or `{ok: false, message}`; nothing
//! throws across the boundary, so the same functions run natively in tests (`ops`, `realtime`).

#![forbid(unsafe_code)]

pub mod library;
pub mod ops;
pub mod realtime;

use library::Library;
use realtime::Realtime;
use wasm_bindgen::prelude::*;

/// A JS number as a seed (negative or not finite: the default seed).
fn seed_of(seed: f64) -> u64 {
    if seed.is_finite() && seed >= 0.0 {
        seed as u64
    } else {
        ops::DEFAULT_SEED
    }
}

/// The crate version (`Cargo.toml`).
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validates a score: `{ok, errors[], warnings[]}`.
#[wasm_bindgen]
pub fn validate_score(json: &str) -> String {
    ops::validate(json).to_string()
}

/// A brief in words -> `{ok, score, title, summary, plan, repairs, lint}` (offline planner).
#[wasm_bindgen]
pub fn plan_brief(brief: &str, seed: f64) -> String {
    ops::plan_brief(brief, seed_of(seed)).to_string()
}

/// A ScorePlan (JSON) -> the same payload as [`plan_brief`].
#[wasm_bindgen]
pub fn compose_plan(plan: &str, seed: f64) -> String {
    ops::compose_plan(plan, seed_of(seed)).to_string()
}

/// An edit in words on a score -> `{ok, changed, understood, patch, inverse, score, summary,
/// title, bars, refused, new_layers}`. `scope` is `null` or `{layer, bars: {start, end}}`.
#[wasm_bindgen]
pub fn plan_edit(score: &str, instruction: &str, scope: &str, seed: f64) -> String {
    ops::plan_edit(score, instruction, scope, seed_of(seed)).to_string()
}

/// Applies a patch: `{ok, score, inverse}` or `{ok: false, message, error}`.
#[wasm_bindgen]
pub fn apply_patch(score: &str, patch: &str) -> String {
    ops::apply_patch(score, patch).to_string()
}

/// Checks a master render against its score: the `VerifyReport` plus `passed` and `summary`.
#[wasm_bindgen]
pub fn verify(score: &str, left: &[f32], right: &[f32], sample_rate: u32) -> String {
    ops::verify(score, left, right, sample_rate).to_string()
}

/// Measures a clip's PCM: tempo, key, loudness, onsets, loop seam.
#[wasm_bindgen]
pub fn analyse(left: &[f32], right: &[f32], sample_rate: u32, beats_per_bar: u8) -> String {
    ops::analyse(left, right, sample_rate, beats_per_bar).to_string()
}

/// The offline engine: pack files held in memory, presets, renders.
#[wasm_bindgen(js_name = Engine)]
#[derive(Debug, Default)]
pub struct WasmEngine {
    lib: Library,
}

#[wasm_bindgen(js_class = Engine)]
impl WasmEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmEngine {
        WasmEngine::default()
    }

    /// Stores one pack file under its pack-relative path ("upright-piano-kw/pack.json").
    pub fn insert_pack_file(&mut self, path: &str, bytes: Vec<u8>) {
        self.lib.insert(path, bytes);
    }

    /// `{ok, needs: [{pack, preset, complete}]}` for the sampled layers of a score.
    pub fn pack_needs(&mut self, score: &str) -> String {
        ops::pack_needs(&mut self.lib, score).to_string()
    }

    /// `{ok, missing: [path]}`: files still to fetch for `preset` of `pack`.
    pub fn missing_pack_files(&self, pack: &str, preset: &str) -> String {
        ops::missing_pack_files(&self.lib, pack, preset).to_string()
    }

    /// `{ok, presets[], engine_only[]}` (`praxis presets`).
    pub fn presets(&mut self) -> String {
        ops::presets(&mut self.lib).to_string()
    }

    /// `[files, bytes]` held.
    pub fn pack_size(&self) -> Vec<f64> {
        let (n, b) = self.lib.size();
        vec![n as f64, b as f64]
    }

    /// Renders a score offline and checks it. `settings`: `{sample_rate?, layers?}` or `null`.
    pub fn render(&mut self, score: &str, settings: &str) -> Rendered {
        let s = ops::parse_settings(settings);
        match ops::render(&mut self.lib, score, &s, None) {
            Ok(r) => Rendered {
                ok: true,
                left: r.audio.left,
                right: r.audio.right,
                result: serde_json::json!({
                    "ok": true,
                    "report": r.report,
                    "checks": r.checks,
                    "played": r.played,
                })
                .to_string(),
            },
            Err(e) => Rendered {
                ok: false,
                left: Vec::new(),
                right: Vec::new(),
                result: e.to_string(),
            },
        }
    }
}

/// One offline render: the master's two channels and `{ok, report, checks, played}` (or
/// `{ok: false, message}`).
#[wasm_bindgen]
#[derive(Debug)]
pub struct Rendered {
    ok: bool,
    left: Vec<f32>,
    right: Vec<f32>,
    result: String,
}

#[wasm_bindgen]
impl Rendered {
    pub fn ok(&self) -> bool {
        self.ok
    }

    /// The JSON result.
    pub fn result(&self) -> String {
        self.result.clone()
    }

    /// Moves the left channel out (a `Float32Array`); a second call returns an empty array.
    pub fn take_left(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.left)
    }

    pub fn take_right(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.right)
    }
}

/// The real-time engine of an AudioWorklet (see [`realtime`]).
#[wasm_bindgen(js_name = Realtime)]
#[derive(Debug)]
pub struct WasmRealtime {
    inner: Realtime,
}

#[wasm_bindgen(js_class = Realtime)]
impl WasmRealtime {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: u32) -> WasmRealtime {
        WasmRealtime {
            inner: Realtime::new(sample_rate),
        }
    }

    pub fn max_frames() -> usize {
        realtime::MAX_FRAMES
    }

    /// The status slot names, comma separated, in [`WasmRealtime::status`] order.
    pub fn status_fields() -> String {
        realtime::STATUS_FIELDS.join(",")
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    pub fn insert_pack_file(&mut self, path: &str, bytes: Vec<u8>) {
        self.inner.library().insert(path, bytes);
    }

    pub fn missing_pack_files(&mut self, pack: &str, preset: &str) -> String {
        ops::missing_pack_files(self.inner.library(), pack, preset).to_string()
    }

    /// Prepares a score and swaps it in. `master_gain_db`: the last offline render's applied
    /// gain (`NaN`: none, plays at 0 dB unless the score has a frozen gain).
    pub fn load(&mut self, score: &str, master_gain_db: f64) -> String {
        let gain = master_gain_db.is_finite().then_some(master_gain_db);
        self.inner.load(score, gain).to_string()
    }

    pub fn play(&mut self) -> bool {
        self.inner.play()
    }

    pub fn stop(&mut self) -> bool {
        self.inner.stop()
    }

    /// Musical position in samples.
    pub fn seek(&mut self, position: f64) -> bool {
        self.inner.seek(position.max(0.0) as u64)
    }

    /// Loop range in musical samples; `end <= start` clears it.
    pub fn set_loop(&mut self, start: f64, end: f64) -> bool {
        self.inner.set_loop(start.max(0.0) as u64, end.max(0.0) as u64)
    }

    pub fn set_layer_mute(&mut self, layer: &str, muted: bool) -> bool {
        self.inner.set_layer_mute(layer, muted)
    }

    pub fn set_layer_solo(&mut self, layer: &str, solo: bool) -> bool {
        self.inner.set_layer_solo(layer, solo)
    }

    pub fn set_layer_gain(&mut self, layer: &str, gain_db: f32) -> bool {
        self.inner.set_layer_gain(layer, gain_db)
    }

    pub fn set_master_gain(&mut self, gain_db: f32) -> bool {
        self.inner.set_master_gain(gain_db)
    }

    /// Renders `frames` frames into the engine's buffers (read them through
    /// [`WasmRealtime::left_ptr`]). Returns the frames rendered. Allocates nothing.
    pub fn render(&mut self, frames: usize) -> usize {
        self.inner.render(frames)
    }

    /// Renders into the caller's arrays (`process(left, right)`).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.inner.process(left, right);
    }

    pub fn left_ptr(&self) -> usize {
        self.inner.left_ptr()
    }

    pub fn right_ptr(&self) -> usize {
        self.inner.right_ptr()
    }

    /// Address of the status slots (`f64`, [`WasmRealtime::status_fields`] order).
    pub fn status_ptr(&self) -> usize {
        self.inner.status_ptr()
    }

    /// A copy of the status slots.
    pub fn status(&self) -> Vec<f64> {
        self.inner.status().to_vec()
    }
}
