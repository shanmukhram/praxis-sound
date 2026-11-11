# Brief: WP-AG2 assistant polish from the first live run

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules),
`docs/agents/live-run-results.md` (what the live run found), `docs/agents/live-run.md`,
`docs/contracts/cli.md` ("Takes" and "Assistant agent"), `docs/contracts/patch.md`, the agent in
`crates/praxis-cli/src/agent/`, the take ops in `crates/praxis-cli/src/ops/` (`takes.rs`,
`changes.rs`, `history.rs`), the MCP `praxis_ask` tool, the desktop shell's `ask` command in
`crates/praxis-app/src/commands.rs`, the adapter `app/src/engine/tauri/`, and how the thread draws
takes in `app/src/ui/shell.ts`. ADR-0011 for the wording style.

## Fix (all found in the live run or left as follow-ups)

1. **A turn without a take is a reply, not an error.** Undo, a refused locked layer, a
   clarifying question (`ask_user`) and opening a view returned `ok: false` (`patch_rejected` /
   `error`) with the reply as the message. Make `ops::ask` return `ok: true` with `take: null`,
   the reply, the steps and why no take was made (`no_take: "question" | "refused" | "undo" |
   "view" | "nothing_to_change"`). CLI exit code 0; MCP `praxis_ask` the same shape; the app's
   thread shows it as an assistant reply (a question, a refusal, "take 2 is current again"),
   never as a red error. Real failures (API down, no key with `provider: claude`, a render error)
   stay errors.
2. **Titles.** Take titles were clipped from the reply ("I made a 16-bar"). The agent's final
   step gives a short title (3-6 words, sentence case, no "I"); add it to the finishing tool or
   the reply protocol, with a fallback that summarises instead of clipping.
3. **"Identical" wording after reverb.** When a scoped edit leaves no bar bit-identical only
   because a recirculating reverb carries rounding-level differences (contracts/patch.md allows
   below -120 dBFS), the take says "Bars 1-12 identical note for note; only reverb rounding
   differs" (measure the largest difference and use the patch contract's threshold), not nothing.
4. **Locks in the CLI.** `praxis ask --patch` with only lock/mute/solo changes records a take
   today ("Kick locked"). Like the app, these are live-score changes without a take: commit them
   to the project (undoable) and bump `revision`, no render, no take.
5. **Tone edits through "point at bars".** A bar-limited tone or space edit is a layer-wide
   patch held to its bars by automation (the frozen `apply` refuses effect/automation changes
   under `scope.bars`), so the app's scoped path (`changed_layers`) counts the whole layer and
   refuses it. Teach `changed_layers` (and the agent's draft scope check) to read automation and
   effect changes bar by bar: a change whose effect outside the scope is nil (automation flat at
   the old value, a new effect bypassed or at zero mix there) counts only for the scoped bars.
   Prove it with a render: bars outside the scope stay identical within the patch contract.
6. **The assistant's live steps in the app.** Pass the agent's progress callback through the
   Tauri `ask` command as events, and show them under the request while it works ("Reading the
   sound", "Changing the felt piano's tone", "Rendering", "Checking: 8 of 8 passed"), replaced by
   the take or reply when done. Plain words, the existing style.
7. **Key safety.** `PRAXIS_CLAUDE_URL` (the API address override used by tests) must only accept
   loopback hosts (`127.0.0.1`, `::1`, `localhost`); anything else is refused with a clear error,
   so the key can never be sent to another server by an environment variable.

## You own

`crates/praxis-cli/**`, `crates/praxis-mcp/**`, `crates/praxis-app/src/{commands,shell}.rs` and
`crates/praxis-app/tests/**`, `app/src/engine/**`, `app/src/ui/**`, `app/tests/**`,
`docs/agents/**`, and the "Takes" and "Assistant agent" sections of `docs/contracts/cli.md`.

## Rules for this package

- Never call the real Claude API: tests use the scripted model and replay fixtures; every
  binary or app run sets `PRAXIS_KEYCHAIN=off` (the owner's key is in the keychain).
- Never play audio on the owner's speakers (fake output, muted Chromium).
- Build limits: 5 jobs / 5 test threads via `.claude/worktrees/.cargo/config.toml`; never override.

## Done when

- Tests for each item: the four no-take turns return `ok: true` with `no_take` set (CLI exit 0,
  MCP shape, app shows a reply); titles are short and never clipped; the reverb wording appears
  with a measured residue; a lock via `--patch` makes no take but is undoable; a bar-limited tone
  edit passes the app's scoped path and keeps the other bars identical; the app shows the steps
  (Vitest with the fake bridge); `PRAXIS_CLAUDE_URL=https://example.com` is refused.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-cli -p praxis-mcp -p praxis-app --release`, the acceptance test, the wasm
  build, `npx tsc --noEmit`, `npx vitest run` and the Playwright e2e (muted) pass.
- Update `docs/agents/live-run.md` so the next live run checks these.
- Report as in docs/BRIEFING.md.
