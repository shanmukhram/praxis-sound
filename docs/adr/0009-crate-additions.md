# ADR-0009: Workspace additions beyond the PRD crate table

Status: accepted

- `praxis-testkit`: golden-audio harness (hash per platform + feature tolerances) and sample-exact
  comparison helpers, used as a dev-dependency. The PRD lists the harness in Phase 0 but no crate.
- `praxis-app` (Phase 3) is the PRD's `sf-app`; the web front end lives in `app/`.
- Determinism is promised per platform (PRD wording). Golden hashes are stored per `os-arch`;
  cross-platform runs compare features within tolerance.
