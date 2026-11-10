# Contract: engine, instruments and rendering

Code (frozen): `crates/praxis-engine/src/api.rs`, `instrument.rs`;
`crates/praxis-instruments/src/registry.rs`, `packs.rs`; `crates/praxis-dsp/src/traits.rs`.

## Offline render

`render_offline(score, factory, opts) -> RenderOutput { master, stems, report }`.

Pipeline per layer: notes (from `timeline::expand_notes`) -> instrument -> layer effects (in order)
-> layer automation -> gain/pan (equal-power) -> stem. Sum of stems -> master effects -> loudness
normalisation (or frozen gain) -> true-peak limiter at `master.loudness.true_peak_dbtp`.

- **Block size** is fixed (`opts.block_size`, default 128). Notes start exactly at their sample:
  the engine splits blocks at event boundaries, so instruments always receive `note_on` at the
  start of a `process` call.
- **Order is fixed**: layers render in `score.layers` order; sums are accumulated in that order.
  Multi-threaded rendering is allowed only if each layer renders independently and the sum order
  is unchanged.
- **Mute/solo**: muted layers are skipped; if any layer is solo, only solo layers render.
- **Avoid-list**: layers whose class, or whose preset's intrinsic class, is in
  `constraints.avoid` are not rendered and are listed in `report.blocked_layers`.
- **Sidechain**: a compressor with `sidechain: <layer id>` is keyed by that layer's instrument
  output (pre its effects). Layers are rendered so the key is available first; a sidechain cycle
  is a render error.
- **Automation**: evaluated per block at the block's first sample (blocks are 128 samples, so
  ~2.7 ms resolution); processors smooth.
- **Latency compensation**: effects report `latency_samples`; the engine delays other paths so
  every stem stays sample-aligned with the grid.

## Loop wrap (FR-26)

For `playback: loop`, let L = loop length in samples. The engine renders the arrangement
continuously for `k + 1` cycles, where `k = ceil(max_tail / L)` (at least 1), and returns the last
cycle `[k·L, (k+1)·L)`. Because every cycle is identical, the captured cycle already contains the
tails of the previous one, so sample `L-1` flows into sample `0` exactly as it would in a looping
player. Effect and compressor state is also in steady state. Loudness is measured on the captured
cycle, and the limiter runs in the same warm-up manner. `report.warmup_cycles = k`.

Verification: `praxis_analyze::verify` checks the seam; a click at the loop point is a failure.

## Instruments

`InstrumentFactory::create(spec, ctx)` builds one `Instrument` per layer. Instruments overwrite
their output buffers in `process`, receive `NoteOn {id, pitch, velocity, brightness, seed,
length_samples}` and `note_off(id)`. Voice stealing takes the quietest voice. Sustaining
instruments ring past `note_off` (release), which is how overlap above 100% sounds connected.

`PresetInfo` declares `class`, `pitched`, `sustaining`, `range` and parameters. Preset names are
stable API (scores refer to them).

## Real time (Phase 3)

The same graph runs in a `RealtimeEngine` owned by the audio callback. The UI talks to it only
through `rtrb` single-producer/single-consumer queues: commands in (play, stop, seek, set param,
swap prepared graph), meters and position out. Graph changes are prepared off the audio thread and
swapped in by pointer exchange through the queue; the old graph is returned through another queue
to be dropped off the audio thread. No allocation, locks or I/O in the callback.
