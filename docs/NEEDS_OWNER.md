# Needs the owner

Only things the owner can give. Work continues on everything they do not block.

| # | Item | Blocks | Status |
| --- | --- | --- | --- |
| 1 | **Anthropic API key** for the Claude planner (`ANTHROPIC_API_KEY` env var, or `praxis key set` once built). The offline rule planner works without it, so CI and acceptance test 1 do not wait. | Live Claude planning and its prompt-suite run | open |
| 2 | **Listening sign-off, Phase 2**: acceptance test 1 with the felt piano and soft kick (`demos/phase-2/`). | Phase 2 exit (not the next phases) | pending demos |
| 3 | **Licence** for Praxis Sound (open source or closed, which licence). Repo is private meanwhile; all deps permissive (ADR-0008). | Public release | open |
| 4 | **Trademark check** of "Praxis Sound". | Anything public | open |
| 5 | **Paid sample packs?** Default piano is CC0 Upright Piano KW voiced soft. A dedicated felt piano with redistribution rights would likely sound better; tell me if you want to buy one and which. | Nothing now | open |
| 6 | **MPL-2.0 acceptable?** symphonia would decode AAC/MP4 natively; without it we call ffmpeg when installed (ADR-0004). | Native video/AAC decoding without ffmpeg | open |
| 7 | **Linux** a supported platform for release, or CI-only (PRD open question)? | Release scope | open |
| 8 | **Labelled reference clips**: any clips you own with known tempo/key help the Phase 5 test set (we also make our own). | Nothing (we self-make) | optional |
| 9 | **Real-hardware check** of real-time playback on Windows and macOS at Phase 3 exit. | Phase 3 sign-off | later |

Resolved:
- Product name is Praxis Sound (2026-11-10).
- Git identity, commit rules and private GitHub repo `shanmukhram/praxis-sound` (2026-11-10).
- Downloads of CC0/CC-BY packs and IRs under 100 MB each approved (2026-11-10).
