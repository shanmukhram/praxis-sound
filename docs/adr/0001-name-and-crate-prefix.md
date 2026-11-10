# ADR-0001: Product name and crate prefix

Status: accepted (owner, 2026-11-10)

The product is **Praxis Sound**. Earlier briefs used "Sonic Foundry"; the owner corrected this,
and the PRD notes that Sonic Foundry was the original maker of Sound Forge (trademark clash).

Decision: crates are named `praxis-*` instead of the PRD's `sf-*`; the CLI binary is `praxis`;
project files use the `.praxis` extension. The PRD's crate table maps one to one
(`sf-score` -> `praxis-score`, ...). A trademark check of "Praxis Sound" is still open
(docs/NEEDS_OWNER.md).
