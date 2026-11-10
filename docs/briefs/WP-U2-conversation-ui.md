# Brief: WP-U2 conversation-first workstation

Branch: continue on `wp/u1-frontend` after `git merge build/integration`, or `wp/u2-conversation-ui`
from `build/integration`. Read first: `docs/BRIEFING.md` (binding), `PRD.md` section 3D,
`docs/adr/0011-conversation-first-workstation.md` (accepted by the owner), `docs/adr/0010-agents-first-class.md`,
and **`docs/design/workstation-concept.html`**, the approved visual and interaction reference.
Screenshots: `docs/design/screenshots/`.

## Goal and why it matters
The owner wants the app "as simple as possible", so beginners use it like a pro, and agents drive
it too. The owner approved the concept: a conversation about one sound, where each request makes a
playable take with plain-language checks and an honest "what changed / what stayed identical", and
a Sound panel whose change map shows the edit. Deeper views open on demand, by the person or by the
assistant/agent. Rebuild `app/` around it; the existing timeline, step sequencer, piano roll,
waveform/spectrogram, history and export pieces become the panel's views, not the main screen.

## You own
`app/**` only.

## Build
- **Match the concept.** Tokens, type (Segoe UI Variable / system stack, no outside loads), the
  three-column shell (sounds list 248 | conversation | Sound panel 456), the take card, compact
  earlier takes with Go back, the composer with scope chip and clip drop, the suggestion chips,
  panel tabs, the pinned change map, and the "opened by <who>" flash. **Themed scrollbars**
  everywhere (`color-scheme: dark`, slim grey thumb, transparent track): the owner asked for this.
  Phone: full-width thread, sticky composer, Sound panel as a bottom sheet.
- **Take model in the adapter.** Extend `EngineAdapter` with
  `ask({ text, scope?: {layer, bars}, clipFile? }) -> Take`, `takes()`, `restore(n)` (an undoable
  patch), `onTake`, `onOpenView({ view, by })` and `openView(view)`.
  `Take = { n, who: 'You' | 'Agent · <name>', ask, title, say, score, patch, changed: {layer: [from, to]},
  untouched: string, render: { audioUrl, report }, checks: VerifyReport, opens? }`, mirroring the
  Rust `VerifyReport` (`docs/contracts/sound-profile.md`) and the CLI payloads (`docs/contracts/cli.md`),
  so a real adapter can fill it from `praxis generate|edit|check` without changing the UI.
- **MockEngine** implements it: scoped asks produce scoped patches through the existing patch
  builder and mock apply (lock and scope rules hold), and the change map and waveform highlight
  come from `changed`, never from separate state. Keep the honest mock labels.
- **Agents.** A connected-agents block in the shell, and agent takes in the thread under their name.
  The mock can inject an agent take for the e2e test.
- **Restraint.** Colour only on the sound (the waveform's changed bars, the change map, moved
  notes, the playhead): one amber accent, a white inverted primary, true-black flat surfaces.

## Tests
- **Vitest:**
  - a take's `changed` drives the change-map cells;
  - a scoped ask produces a patch whose scope is exactly the dragged bars;
  - restore is undoable;
  - the checks summary counts are computed from the report.
- **Playwright:**
  - load the acceptance-1 sound and see the current take as the largest block;
  - a suggestion chip makes take 4;
  - drag bars 13-16 on the piano lane, then ask, and the patch scope shows bars 13-16 / piano;
  - an agent-opened view shows "opened by";
  - Go back to take 1, then Undo;
  - the phone bottom sheet opens;
  - scrollbar colour is not the default (check the computed `scrollbar-color`).
- **Screenshots** at 1440 and 390 in `app/screenshots/`.

## Git and report
Same rules as before. Commit with
`git -c user.name=shanmukhram -c user.email=51820274+shanmukhram@users.noreply.github.com commit -F <unique msg file>`.
No AI attribution, run both checks after each commit, stage explicit paths, and don't push.
Report: what you built, what you verified, what is mocked, contract issues, next steps, and the commit hash.
