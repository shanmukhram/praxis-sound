# Praxis Sound: PRD & Build Plan

Nov 10, 2026 · @Bunny

## 1. Overview

Praxis Sound (working name) is a Rust sound-design workstation that turns a text brief or a dropped reference clip into editable, layered audio: background music, scores, tonal loops and sound effects. It renders by synthesis and sampling from a structured score, so any result can be changed bar by bar and layer by layer instead of being regenerated from scratch.

It grows out of an earlier browser prototype. Four lessons from that prototype drive the design:

- **Mood words mislead generators.** "Cinematic" and "mystery" produced a riser that cut off abruptly. The app needs a structured score and an enforced avoid-list, not a free-text prompt handed to a black box.
- **Rhythm comes first.** Notes sit on a fixed grid in a repeating motif, with accents (DON / din / TIN) and overlapping sustain. Isolated notes with gaps between them sound out of time.
- **Arrangement is layering.** Bars 9–16 keep the piano untouched and add a soft kick underneath. A new section adds layers; it does not rewrite the ones already there.
- **A synthesized piano is not good enough.** Realistic instruments need samples.

**Users.** Video creators and editors who need original, in-key music and sound design that fits a specific cut. Our own team is the first user.

**Why Rust.** One small native binary, sample-accurate real-time audio with no garbage-collection pauses, offline rendering far faster than real time, and the same engine compiled to WebAssembly for a browser build.

## 2. Goals, non-goals and success criteria

### Goals

1. **Prompt to score.** A plain-language brief produces music in the stated key, tempo, bar count, emotion and structure.
2. **Reference to similar.** A dropped audio or video clip produces new sound with the same character.
3. **Surgical edits.** A change touches only the bars and layers named; everything else stays sample-identical.
4. **Film-grade output.** Sampled instruments, a proper mix bus, loudness targets and seamless loops. Restrained and intentional, never a stock whoosh.
5. **Light and fast.** Small install, instant start, renders many times faster than real time.
6. **Scriptable.** A command-line tool and an open project file, so agents and video pipelines can drive it without the window.

### Non-goals for the first release

- A full DAW: no microphone recording, no hosting of third-party plugins.
- Training or shipping a neural text-to-audio model. The engine is procedural; a language model only writes the score.
- Reproducing a reference clip's melody or its recording.
- Accounts, cloud sync, real-time collaboration, mobile apps.

### Acceptance test 1: the loop from the thread

One prompt must produce this, first try, with no manual fixing:

- 16 bars, 4/4, A minor, 30 seconds (so 128 BPM), seamless loop.
- Bars 1–8: one soft felt piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping sustain.
- Bars 9–16: the identical piano part, plus a soft muted kick layered underneath. No new melody, no fills.
- No risers, sweeps, impacts or pads anywhere. The tail of bar 16 flows into bar 1 with no click or fade.

### Success criteria

The numbers are proposed targets to confirm, not measurements.

| Area | Criterion |
| --- | --- |
| Correctness | Acceptance test 1 passes from a single prompt |
| Determinism | Same project and seed render a bit-identical WAV on the same platform |
| Edit isolation | Editing bars 13–16 of one layer leaves every other bar and layer sample-identical, effect tails aside |
| Loop seam | No discontinuity above the noise floor at the loop point; reverb and release tails wrap around |
| Render speed | A 2-minute, 8-layer piece renders at 20× real time or better on a mid-range laptop |
| Real-time preview | No dropouts at a 256-sample buffer |
| Footprint | Installer under 30 MB without instrument packs; cold start under 1 second; idle memory under 200 MB |
| Reference analysis | Tempo within 2 BPM and correct key on at least 90% of a labelled test set |
| Reference match | Generated sound scores closer to its reference than to any other clip in the test set, and passes the originality check |
| Loudness | Master output hits its loudness target within 0.5 LU and never exceeds the true-peak ceiling |

## 3. Features and requirements

Every sound in the app is a **Score**: a structured, human-readable document holding key, tempo, bars, sections, layers, patterns, instruments, effects and automation. Prompts, reference clips and manual edits all write to the Score; the engine only ever renders the Score. That one rule is what makes bar-level edits, undo, variations and determinism possible.

### A. Prompt to music and score

| ID | Requirement |
| --- | --- |
| FR-1 | A text brief becomes a Score: key and scale, tempo, time signature, bar count, sections, emotion, instruments, rhythm, intensity, loop or one-shot |
| FR-2 | Duration solver: fit a bar count to a target length by solving tempo (16 bars in 30 seconds gives 128 BPM), or solve bars from a fixed tempo |
| FR-3 | Avoid-list enforced by the engine: "no risers, no impacts, no pads" switches those generator classes off, rather than hoping a prompt is obeyed |
| FR-4 | Rhythm-first motifs: a step grid (eighths, sixteenths, triplets) with per-step pitch, velocity, accent and note length, including overlap above 100% |
| FR-5 | Spoken rhythm input: "DON din DON don TIN" maps to strong, soft and bright steps on the grid |
| FR-6 | Layered arrangement: sections add or remove whole layers; a locked layer cannot be altered by any later generation or edit |
| FR-7 | Scoped conversational edits: "keep the first half, change the last four bars to add a subtle melody" changes only those bars |
| FR-8 | Variations: several alternates of any selection, each with its own seed, auditioned A/B and kept or discarded |
| FR-9 | Scoring depth: chord progressions, multi-section form (intro, build, peak, release), tension curves, transitions and emotion presets |
| FR-10 | Hit points: markers at timestamps (a cut, a reveal) that the arrangement lands on, for scoring to picture |

### B. Inspiration clips

| ID | Requirement |
| --- | --- |
| FR-11 | Drag and drop audio (WAV, MP3, FLAC, OGG, M4A) or video (MP4, MOV, WebM) onto the window; the audio track is extracted automatically |
| FR-12 | Analysis produces a Sound Profile: tempo and beat grid, key, loudness and dynamics, spectral balance, groove and onset pattern, pitch contour, amplitude envelope, tonal-to-noise ratio, stereo width, reverb decay, section boundaries |
| FR-13 | The clip is classified as music, sound effect or ambience, and routed to the matching generator |
| FR-14 | "Make something like this": the profile becomes a new Score with the same character, in any key, tempo or length the user asks for |
| FR-15 | Match loop: render, compare against the reference, adjust parameters, repeat until the difference stops shrinking; the user sees the similarity score |
| FR-16 | Effect recreation: whooshes, impacts, risers, drones, glitches and ticks are rebuilt as procedural patches with macro knobs for length, brightness, weight and pitch |
| FR-17 | Mix and match: groove from clip A, timbre from clip B, mood from a text brief |
| FR-18 | Originality guard: output shares character, never the reference's melody or audio. A melodic-similarity check blocks close copies; reference audio is never mixed into an export |
| FR-19 | Reference library: profiles are saved, tagged and reusable as named styles |

### C. Sound engine

| ID | Requirement |
| --- | --- |
| FR-20 | Sampled instruments through open sample formats (SFZ, SoundFont): felt piano first, then strings, keys, mallets, bass |
| FR-21 | Synthesis: subtractive, FM, wavetable, plucked and struck physical models, granular, noise |
| FR-22 | Drums and percussion: synthesized kick, snare, hats and toms, plus sample kits |
| FR-23 | Effects: EQ, filters, compressor with sidechain, saturation, delay, algorithmic and convolution reverb, chorus, stereo width, limiter |
| FR-24 | Automation of any parameter over bars |
| FR-25 | Master bus with a loudness target and true-peak ceiling |
| FR-26 | Seamless loop rendering: tails wrap around the loop point |
| FR-27 | Deterministic offline render and low-latency real-time preview from the same engine code |

### D. Workstation

| ID | Requirement |
| --- | --- |
| FR-28 | Timeline of bars, sections and layers with mute, solo and lock |
| FR-29 | Step sequencer and piano roll for direct editing of any pattern |
| FR-30 | Waveform and spectrogram views; loudness-matched A/B against the reference clip |
| FR-31 | Full undo and redo; named versions of a project |
| FR-32 | Export: WAV (16-bit, 24-bit, 32-bit float), FLAC, per-layer stems, MIDI, and the project file |
| FR-33 | Headless CLI: generate from a prompt, analyse a clip, render a project, all with JSON in and out |
| FR-34 | The language model sits behind a provider interface with the user's own API key. Everything except prompt interpretation works offline |

## 4. Architecture and stack

&#91;embedded content: architecture · three inputs, one Score, one engine, a match loop\]

Prompts and clips never touch audio directly: the planner and analyzer write to the Score, and the dashed match loop feeds each render back through the analyzer until it sounds like the reference.

### Design rules

- **The model writes Scores, never audio.** The planner returns a Score, or a patch to one, through a schema-constrained tool call. A validator rejects off-scale notes, off-grid timing and banned generator classes, then asks for a repair.
- **Edits are patches.** Every change, human or model, is a small diff against the Score with a bar and layer scope. That gives undo, versions and edit isolation.
- **One engine, two modes.** The same DSP code runs offline (deterministic, seeded, fixed block size) and in real time. The audio thread never allocates or locks; it talks to the UI over lock-free queues.
- **Matching is analysis by synthesis.** The analyzer measures the reference and the render with the same features, and an optimizer adjusts patch parameters to close the gap.
- **Core first, shell second.** All logic lives in UI-free Rust crates that also compile to WebAssembly. The desktop window is a thin Tauri shell with a web front end, so a browser build reuses the same UI and engine.

### Workspace crates

| Crate | Responsibility |
| --- | --- |
| `sf-score` | Score types, JSON schema, validation, music theory (scales, chords, quantising), patch and diff operations |
| `sf-dsp` | Oscillators, filters, envelopes, effects, sampler voices; no I/O, WebAssembly-safe |
| `sf-engine` | Sequencer, graph scheduler, mixer, automation, offline renderer, loop wrap, loudness |
| `sf-instruments` | SFZ and SoundFont loading, synth patches, drum models, instrument pack management |
| `sf-audio-io` | Real-time device output, file decoding and encoding |
| `sf-analyze` | Feature extraction, Sound Profile, similarity metrics, originality check |
| `sf-match` | Profile-to-Score mapping and the analysis-by-synthesis optimizer |
| `sf-planner` | Model provider interface, brief-to-Score, edit-to-patch, validation and repair loop |
| `sf-cli` | Headless generate, analyse and render commands |
| `sf-app` | Desktop shell and the workstation UI |
| `sf-wasm` | Browser build of the engine and analyzer |

### Candidate libraries

Phase 0 confirms each one for maintenance status, licence and WebAssembly support before it is adopted.

| Need | Candidate |
| --- | --- |
| Audio device output | cpal |
| Decoding audio and video containers | symphonia, with ffmpeg as an optional fallback |
| WAV writing | hound |
| FFT | rustfft, realfft |
| Resampling | rubato |
| DSP building blocks | fundsp, or in-house code where it falls short |
| Loudness metering | ebur128 |
| MIDI | midly |
| Score schema and serialisation | serde, schemars |
| Optional embedding and stem-separation models | ort (ONNX Runtime) |
| Desktop shell | Tauri 2 |
| Language model | Claude API, behind the provider interface |

## 5. Build plan

Nine phases, ordered by dependency and closed by exit criteria, not dates. After Phase 1, Phases 2, 3 and 5 can run in parallel; Phase 4 needs 2; Phase 6 needs 4 and 5.

### Phase 0: Foundations

- Cargo workspace with the crates from section 4; CI on Windows, macOS and Linux; formatting, lint and test gates.
- Decision records for the UI shell, the DSP library, sample formats and licence.
- Score schema v0, published as JSON Schema, with validation and round-trip tests.
- Golden-audio test harness: render a Score, then compare by hash and by feature tolerance.

**Exit:** CI is green on all three systems, and the CLI renders an empty Score to a silent WAV.

### Phase 1: Engine core

- Oscillators, envelopes, filters, mixer and a grid sequencer with velocity, accent and note overlap.
- Offline renderer, WAV export, seeded determinism, loop-wrap rendering.

**Exit:** a hand-written Score of acceptance test 1, using a simple bell tone, renders with a clean loop seam and is bit-identical across reruns.

### Phase 2: Instruments, effects and mix

- Sampler with SFZ and SoundFont support and a licence-cleared felt piano pack.
- Drum synthesis, the remaining synth types, the full effects set, sidechain, automation.
- Master bus with loudness target and true-peak limiter.

**Exit:** acceptance test 1 renders with the sampled felt piano and soft kick at its loudness target, and you sign off on how it sounds.

### Phase 3: Real-time engine and workstation

- Device output, transport, lock-free bridge between UI and audio thread.
- Timeline, layers with mute, solo and lock, step sequencer, piano roll, waveform and spectrogram.
- Project save and load, undo and redo, versions, stem and MIDI export.

**Exit:** acceptance test 1 can be built by hand in the window, played without dropouts and exported.

### Phase 4: Planner

- Brief to Score through the model provider interface; duration solver; enforced avoid-list; spoken-rhythm input.
- Scoped edits as patches, locked layers, variations with seeds.
- A prompt test suite with automatic validators for key, grid, structure and banned elements.

**Exit:** acceptance test 1 passes from one prompt; "change the last four bars" leaves bars 1–12 sample-identical; the prompt suite passes.

### Phase 5: Reference analysis

- Drag and drop for audio and video, decoding, audio-track extraction.
- Sound Profile extraction, clip classification, profile view, loudness-matched A/B, reference library.

**Exit:** tempo and key accuracy meet the section 2 targets on the labelled test set, and profiles display for a music clip, an effect and an ambience.

### Phase 6: Similar-sound generation

- Profile to Score for music; procedural patch families for whooshes, impacts, risers, drones, glitches and ticks, each with macro knobs.
- Match loop with a visible similarity score; mix and match across clips and text; originality guard.

**Exit:** on the reference test set every generated sound is closest to its own reference, passes the originality check, and survives your blind listening review.

### Phase 7: Scoring depth

- Chord progressions, multi-section form, tension curves, transitions, emotion presets.
- Hit points that the arrangement lands on; longer cues with evolving layers.

**Exit:** a 2-minute score generated from a brief with three hit points lands each one on its marker and passes listening review.

### Phase 8: Performance and release

- Profiling against the speed, memory and start-up targets.
- Installers for Windows and macOS, the WebAssembly browser build, instrument pack downloader.
- CLI documentation, crash reporting, licence audit of every dependency and sample.

**Exit:** every success criterion in section 2 passes on release builds.

## 6. Risks and open questions

### Risks

| Risk | Mitigation |
| --- | --- |
| Procedural audio sounds only as good as its samples and mix chain | Phase 2 exits on your listening sign-off; budget for licensed instrument packs |
| Matching a dense, fully mixed track is much harder than matching a single effect or texture | Match character (tempo, key, groove, instrumentation, tonal balance) rather than the waveform; optional stem separation; show the similarity score honestly |
| The planner writes bland or theory-wrong music | Validators, curated motif and progression libraries, a prompt test suite that grows with every rejected result |
| Reference clips are usually copyrighted | Only measurements are kept; the originality guard blocks close melodic copies; reference audio never reaches an export |
| Real-time audio behaves differently across Windows and macOS drivers | Device tests in CI where possible, manual checks on real hardware at Phase 3 exit |
| The browser build has tighter limits on threads, files and model size | Core crates stay WebAssembly-safe from Phase 0; heavy optional models are desktop-only |

### Open questions

- [ ] Final name. "Sonic Foundry" is also the name of the company that originally made Sound Forge, so it needs a trademark check before anything public.
- [ ] UI shell: Tauri with a web front end (recommended, and what section 4 assumes) or a pure-Rust GUI such as egui.
- [ ] Open source or closed, and under which licence.
- [ ] Which instrument packs ship by default, and whether to pay for any.
- [ ] Whether a local model is needed as an offline fallback for the planner.
- [ ] Whether a neural audio generator should plug in later as an optional layer source.
- [ ] Linux as a supported platform in the first release, or CI-only.
