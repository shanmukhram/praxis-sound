# ADR-0002: UI shell: Tauri 2 with a web front end

Status: accepted (PRD recommendation; owner may revisit)

Options: Tauri 2 + web front end, or a pure-Rust GUI (egui). Choose **Tauri 2**:
- The same front end runs in the browser build against `praxis-wasm`, which the PRD requires.
- Timeline, piano roll and spectrogram are canvas drawing either way; the web stack gives text,
  layout and accessibility for free.
- The UI can be developed and checked in a browser without the desktop shell, which also lets
  automated sessions screenshot and test it.
- Tauri 2.12 is maintained (Apache-2.0 OR MIT) and uses the system WebView, keeping the installer
  well under 30 MB.

Audio never runs in the web view: on desktop the real-time engine runs in Rust (cpal) and the UI
sends commands through Tauri IPC; in the browser it runs in an AudioWorklet backed by wasm.
The front end is TypeScript + Vite with no heavy framework (canvas components), in `app/`.
`sf-app` becomes `crates/praxis-app` (the Tauri crate), added to the workspace in Phase 3 but
kept out of `default-members` so core builds stay fast.
