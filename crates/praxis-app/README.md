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
Sounds live in `PRAXIS_HOME/sounds/<name>` (default `Documents\Praxis Sound\sounds`); set
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
| `apply_patch {sound, patch}` | a patch that makes no take (mute, solo, lock), in the project's undo history |
| `undo {sound}`, `redo {sound}` | one step of the project's history; a step that lands on another take's score makes it current |
| `render_live {sound}` | renders and checks the live score to `<sound>/live/mix.wav` |
| `export {args}` | WAV 16/24/32f, FLAC, stems, MIDI or a `.praxis` copy of a take into a folder |

Project writes take the sound's `.lock` like every other writer. Takes play through the WebView's
`<audio>` element from `takes/<n>/mix.wav` (asset protocol, scoped to the sounds folder and the
open sound); the real-time engine replaces that later.

## Tests

```bash
PRAXIS_PACKS="C:\path\to\packs" cargo test -p praxis-app     # every command on a real sound folder
cd app && npx vitest run tests/unit/tauriEngine.test.ts      # the adapter through a fake bridge
```
