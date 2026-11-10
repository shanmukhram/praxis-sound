# Brief: WP-K cli (first full pass)

Branch: `wp/k-cli`. Read first: `docs/BRIEFING.md` (binding), `PRD.md` (FR-33, FR-34),
`docs/PLAN.md` (WP-K), `docs/contracts/cli.md` (the JSON contract you implement exactly),
and the merged public APIs of praxis-planner, praxis-analyze, praxis-audio-io, praxis-score
(apply, project), praxis-instruments (Registry, packs install), praxis-engine.

## Goal and why it matters
"Scriptable. A command-line tool and an open project file, so agents and video pipelines can drive
it without the window" (PRD goal 6). Every command takes JSON-friendly input and with `--json`
prints exactly one JSON object.

## You own
`crates/praxis-cli/**` (main.rs, commands.rs and new modules, tests under
`crates/praxis-cli/tests/`). Keep the existing `render`, `validate`, `schema` behaviour and
payload fields; only add fields.

## Build
Commands from docs/contracts/cli.md: `generate` (planner: `--provider offline|claude`, default
claude when a key is available else offline with a note in the payload; writes a `.praxis`
project; optional `--render`), `edit` (only if WP-P2 has merged; otherwise leave it out and say so),
`analyze` (decode via audio-io, verify/profile via analyze; until the full profile exists, print the
feature set available), `check <wav> --score <score>` (`verify_render`, exit 1 if any check
fails, payload has `passed`), `export-midi`, `packs list|install <name>` (uses instruments'
`install` feature; `install` downloads only packs listed in `packs/index.json`), `render --format
flac`, `key set|clear` (stores the Anthropic key in the OS keychain via the planner's helper;
reads the key from stdin, never from argv, never prints it). Error codes from the contract.

## Tests
Integration tests running the built binary (`env!("CARGO_BIN_EXE_praxis")`) for every command's
JSON success and error shapes, including: render the bell fixture and `check` it (loop seam and
grid pass), generate acceptance test 1 with `--provider offline` and render it, export MIDI and
re-parse. Tests needing the piano pack skip with a message when absent.

## Report
As in docs/BRIEFING.md "Report back".
