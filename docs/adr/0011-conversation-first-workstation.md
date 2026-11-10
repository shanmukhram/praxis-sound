# ADR-0011: Conversation-first workstation

Status: accepted (owner, 2026-11-11: "the UI is good"; scrollbars must match the theme)

## Context

The owner wants the UI "as simple as possible": a beginner should be able to use it like a pro,
and agents will drive it too. A chat assistant was given as a reference for the feel (say what you
want, get it), not as a design to copy. The first front-end pass (WP-U1) built a classic
workstation (timeline, step sequencer, piano roll as the main screen), which is the expert's view.

## Decision

The workstation opens on a conversation about one sound, with a Sound panel beside it.

- **Takes, not messages.** Every request (from the person or an agent) produces a take: a playable
  waveform, the facts (key, tempo, bars, length), the measured checks in plain words ("5 of 5
  checks passed: in key, on the beat, -16.0 LUFS, peak -1.1 dB, seamless loop"), what changed and
  what stayed sample-identical, and the next actions (try variations, export). Earlier takes fold
  into one-line rows with "Go back"; the current take is always the biggest thing on screen.
  Takes are project versions (FR-31); going back is an undoable patch.
- **The change map is the signature.** The panel always starts with "Where each layer plays": a
  lane per layer over the bars, sections above, and the bars the current take changed in the one
  accent colour, with a sentence saying what is untouched. It makes scoped edits (FR-7) and
  layering (FR-6) visible without vocabulary. Dragging across bars sets the scope of the next
  request ("Bars 13-16 · Felt piano"), so a beginner makes surgical edits by pointing.
- **Depth on demand.** The panel's views are Overview (layers with Mute / Solo / Lock in words,
  and four plain-language macros: Mood, Brightness, Space, Energy), Rhythm (the step grid in
  DON / din / don / TIN), Notes (piano roll plus a "What moved" table), Mix (loudness and peak
  against their targets) and Export. The person opens them, or the assistant or an agent opens the
  one that shows what it just did ("Notes opened by cut-assistant"). WP-U1's timeline, step
  sequencer and piano roll become these views.
- **Agents are visible participants.** Connected agents are listed in the shell; their requests
  and takes appear in the same thread with their name, and they can open panel views through the
  MCP server (WP-X: a `praxis_open_view` tool and a `praxis_takes` stream).
- **Inputs.** One composer: text, a quoted spoken rhythm, or a dropped clip (FR-11); suggestion
  chips under it are generated from the current sound.
- **Style.** True-black flat (taste-skill app rules): elevation by lightness, 1px rules, colour
  only on the sound (waveform of the changed bars, change map, moved notes, playhead) in one amber
  accent; a white inverted primary button; Segoe UI Variable / system UI (no outside font loads);
  sentence case; no gradients, glass, emoji, icon tiles or tracked caps. Scrollbars are themed
  (slim grey thumb on a transparent track, `color-scheme: dark`), never the default white bar.
- **Phone.** Conversation full width with a sticky composer; the Sound panel is a bottom sheet
  opened from the top bar or by the assistant.

## Consequences

WP-U is re-briefed (`docs/briefs/WP-U2-conversation-ui.md`). The engine adapter interface
gains take-level calls (request -> take with render report and checks; open view). The planner
needs a short, plain-language reply per take (`PlanOutcome` summary), and suggestion chips from
the current Score. The CLI and MCP keep the same take model so agents and people share history.
