# Contract: `praxis` CLI JSON (FR-33)

With `--json`, every command prints exactly one JSON object on stdout and nothing else:

```json
{"ok": true, "command": "<name>", ...payload}
{"ok": false, "command": "<name>", "error": {"code": "<code>", "message": "<text>"}}
```

Exit code 0 on success, 1 on error. Human-readable output (without `--json`) may change freely;
the JSON payloads below only gain fields, never lose or rename them.

| Command | Input | Payload |
| --- | --- | --- |
| `render <score\|project> -o out.wav [--format 16\|24\|32f\|flac\|flac16\|flac24] [--sample-rate N] [--stems DIR] [--seed N] [--skip-missing]` | score JSON or `.praxis` | `output, stems[], sample_rate, frames, duration_s, is_loop, loudness_lufs, true_peak_dbtp, applied_gain_db, blocked_layers[], render_seconds, realtime_factor, format, skipped_layers[], warnings[], check{VerifyReport fields, passed, failed[]}` |
| `validate <score>` | score JSON | `valid, errors[], warnings[]` (issues: `code, path, message`) |
| `schema [--write]` | - | `score, patch, plan` schemas, or `written[]` |
| `generate (--prompt TEXT [--provider offline\|claude] \| --plan FILE\|-) [--seed N] -o project.praxis [--render out.wav]` | brief or agent-written ScorePlan | `project, score_summary{key, bpm, bars, layers[], sections[]}, provider, provider_note, attempts, repairs[], render?` |
| `edit <project> --prompt TEXT [--provider ...]` | project + instruction | `project, patch, scope, changed_bars, changed_layers` |
| `analyze <clip> [-o profile.json]` | audio/video file | the Sound Profile |
| `check <wav> --score <score>` | render + score | `VerifyReport` (`checks[], tempo_bpm, key, grid_deviation_ms, onsets_on_grid, integrated_lufs, true_peak_dbtp, seam_*`) and `passed` |
| `match <clip> [--prompt TEXT] [--bars N] [--key K] [--bpm N] -o project.praxis` | reference clip | `project, similarity, iterations[], originality` |
| `export-midi <score\|project> -o out.mid` | score | `output, tracks` |
| `packs list\|install <name>` | - | `packs[]` / `installed` |
| `key set\|clear\|status` | key on stdin only (set) | `stored` / `cleared` / `available, source` |
| `presets` | - | `presets[]` (name, kind, class, renderable here) |

The command layer is the library `praxis_cli::ops` (one function per command, typed request in,
payload JSON out); the MCP server and the desktop app call it directly. `edit` lands with WP-P2,
`match` with WP-M.

Error codes: `io`, `bad_json`, `bad_score`, `invalid_score`, `bad_format`, `render_failed`,
`no_api_key`, `provider_failed`, `pack_missing`, `decode_failed`, `patch_rejected`, `error`.
