# praxis-app: the Praxis Sound desktop app

A Tauri 2 shell (ADR-0002) around the conversation-first workstation in `app/` (ADR-0011). The
front end's `TauriEngine` adapter (`app/src/engine/tauri/`) calls the commands here over IPC;
each command is a thin wrapper over the shared command layer (`praxis_cli::ops`), the same
functions the `praxis` CLI and the MCP server call. A take made in the app and a take an agent
makes through the MCP server land in the same history.

## Run it (Windows)

```bash
cd app
npm install
npm run tauri -- dev                  # Vite dev server + the desktop window (debug build)
npm run tauri -- build --no-bundle    # release exe: target/release/praxis-app.exe
```

Set `PRAXIS_PACKS` to the instrument packs folder (for the felt piano) before starting it.
Sounds live in `PRAXIS_HOME/sounds/<name>` (default `%USERPROFILE%\Praxis Sound\sounds`, not Documents: Windows ransomware protection blocks writes there; if the folder cannot be created the app uses `%LOCALAPPDATA%\Praxis Sound` instead of closing); set
`ANTHROPIC_API_KEY` (or `praxis key set`) to plan with Claude, otherwise the offline rule
planner answers. Needs the WebView2 runtime (part of Windows 11).

## Commands

| Command | What it does |
| --- | --- |
| `app_info` | version, home, sounds folder, `PRAXIS_PACKS` |
| `list_sounds` | every sound folder, newest first (`SoundInfo`) |
| `new_sound_path {text}` | the folder a new sound will get (created by its first take) |
| `watch_sound {sound}` | follows the sound's `takes.json`; emits `praxis://takes {sound, revision}` on a new revision |
| `ask {req}` | `ops::ask` (a request, a scoped request or a hand edit with `patch`) -> the take with its score |
| `takes {sound}` | `ops::takes` with every take's score |
| `restore {req}` | `ops::restore` (going back is an undoable patch) |
| `edit {req}`, `variations {req}`, `keep_variation {req}`, `open_view {req}` | the matching ops |
| `new_sound {args}` | a blank sound (key, `bpm` or `seconds` for the duration solver, time signature, bars, loop or one-shot) in a new folder, with no take yet (`hand.rs`) |
| `make_take {req}` | renders, checks and records the live score as one take: a blank sound's first take, or every hand edit since the current take (its patch is the exact diff) |
| `presets` | the instrument presets a layer can use (`praxis presets`), each with whether it renders here |
| `apply_patch {sound, patch}` | a patch that makes no take (mute, solo, lock, and every hand edit), in the project's undo history |
| `undo {sound}`, `redo {sound}` | one step of the project's history; a step that lands on another take's score makes it current |
| `render_live {sound}` | renders and checks the live score to `<sound>/live/mix.wav` |
| `export {args}` | WAV 16/24/32f, FLAC, stems, MIDI or a `.praxis` copy of a take into a folder |
| `transport_sync {sound}` | the live score changed: plays it from now on (mute / solo / gain live, anything else rebuilt and swapped in); `null` closes |
| `transport_play`, `transport_stop`, `transport_seek {seconds}`, `transport_set_loop {range}` | the real-time transport (loop bars `start..=end`) |
| `transport_layer {layer, live}` | a live layer override (`gain_db` trim, `muted`, `solo`), heard at once, not saved |
| `transport_status` | device, rate, buffer, xruns and the last graph build (`TransportStatus`) |

Project writes take the sound's `.lock` like every other writer.

## Building a sound by hand (WP-U5)

A blank sound is a folder whose project holds a score with no layers and whose `takes.json` has
no take yet: it is listed, opened, watched and played like any other sound. Every control in the
Sound panel (settings, layers, notes in Rhythm and Notes) is a patch through `apply_patch`, so it
is heard at once through the real-time engine, undoable and held to the same rules (locks, key,
validation). "Make a take" calls `make_take`, which renders and checks the live score and records
it in the shared history with `provider: "hand"`: the first take of a blank sound freezes the
master gain as the take op does after a new sound's first render; a later take's patch is
`patch_between` the current take's score and the live score. Requests in words work from the
first take on (`ask` needs a take to edit).

The window makes a take only when asked, never on a pause or on leaving a view: every take is a
render plus checks and an entry in the history people and agents share, so a half-written motif
should not become one.

## Playback (real time)

`audio.rs` runs the real-time engine (`praxis_engine::realtime`) on the default output device
(`praxis_audio_io::device`: cpal, WASAPI shared mode, a 256-frame buffer asked for; Windows calls
back every 10 ms, 480 frames). A service thread owns the stream and the engine's control side; a
preparer thread builds each live score at the device's rate with the instrument registry the
renders use (`PRAXIS_PACKS`) and the master gain the current take's render applied. Every change
of the live score is synced: a change in layer mute, solo or gain alone goes in live (10 ms
ramp); anything else is built and cross-faded in over 30 ms while playing. Position, peak / RMS
meters, callback sizes, xruns and device errors arrive as `praxis://transport` about 30 times a
second; the Sound panel shows them in one line with a level meter.

The current take plays through the engine; earlier takes play their own render through
`<audio>` (asset protocol, scoped to the open sound's audio folders). When no output device
opens, or the score cannot be built for playback, the current take falls back to `<audio>` and
the line says why; the next Play tries the device again. cpal's `realtime` feature (thread
priority through an MPL-2.0 crate) stays off.

`PRAXIS_AUDIO_OUTPUT=fake` runs the same engine and commands on the fake output instead of the
speakers (48 kHz, 480-frame callbacks, paced by the clock): for silent manual runs. The fake
output cannot starve, so its xrun count is always 0; the render time per callback
(`transport_status`: `max_render_ms`, `p99_render_ms` against the 10 ms callback) is the margin.

## Tests

```bash
PRAXIS_PACKS="C:\path\to\packs" cargo test -p praxis-app     # every command, and the audio service on a fake output
PRAXIS_DEVICE_TEST=1 cargo test -p praxis-app --test audio device -- --nocapture   # the speakers
PRAXIS_PACKS="C:\path\to\packs" cargo test -p praxis-app --test hand   # a sound built by hand, and the Phase 3 exit sound
cd app && npx vitest run tests/unit/tauriEngine.test.ts      # the adapter through a fake bridge
```
