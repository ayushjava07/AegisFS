# Plan — runvane build

This file is the resumption point for the build. It is updated at the end of
every phase. Consulting it first is mandatory if the session is interrupted.

## Status legend

- `[x]` phase complete and checkpoint verified
- `[~]` phase in progress
- `[ ]` not started

## Global counters

- Running production LOC (source + comments, excluding tests/benches/fuzz/gen):
  see `cloc` run in Phase notes; tracked at each phase end.
- Commit target: >= 150 `git log --oneline | wc -l` (includes the 19 commits
  that predate the runvane build, which are preserved by design).

## Phases

| Phase | Status |
|---|---|
| 0 — design, scaffold, CI | complete |
| 1 — domain + state machine + persistence | in progress |
| 2 — scheduler/worker pool/retry/backoff | |
| 3 — HTTP + gRPC API | |
| 4 — CLI + config layer | |
| 5 — plugins + webhooks/events | |
| 6 — observability/cache/maintenance/dashboard | |
| 7 — hardening pass | |
| 8 — test-suite completion | |
| 9 — clean-baseline verification (golden tag) | |
| 10 — defect catalogue + injection | |
| 11 — per-defect packaging | |
| 12 — documentation + CI finalization | |

## Deviations (recorded as they happen)

1. **Language and stack**: the original plan targets Go. This repo is a Rust
   repository by prior decision ("lang not go, strict"), so the whole build is
   re-scoped to Rust. Tool mappings: `cargo clippy`/`cargo miri`+`loom` for
   race/concurrency analysis, `cargo fuzz` for fuzzing, `cargo-mutants` for
   mutation testing, Rust ownership for leak analysis supplemented by a
   dedicated resource-tracking test harness.
2. **Persistence**: Postgres is replaced by an embedded SQL backend
   (`rusqlite`, bundled) plus the in-memory backend over the same `Store`
   trait. Rationale: zero external-service dependency, reproducible fresh
   clone with no manual setup. Postgres layering remains possible via the
   trait without touching domain code.
3. **Product name**: the original spec required an original name. Both
   "AegisFlow" and "AegisOps" were found to collide with existing products.
   The name **Runvane** is chosen; repository directory keeps legacy name
   `aegisfs`.
4. **Single package**: the build is a single Cargo package (`runvane`, old
   `aegisfs`) with a `fuzz/` workspace member and `benches/`, rather than a
   multi-crate workspace. Keeps builds fast and LOC accounting simple without
   changing any subsystem boundary.

## Phase 0 notes

- Decision: reset the source tree from the legacy aegisfs filesystem library
  to the Runvane platform, preserving repository history (the original
  filesystem commits remain; documented in commit messages).
- Product name verified-ish via web search; no material collision found for
  "Runvane".
- Toolchain: rust 1.96.1 (from `rust-toolchain.toml`, matches installed
  toolchain after rustup init). protoc 36.1 installed via Homebrew for
  `tonic`/`prost-build`.
- Background tool installs: cargo-fuzz, cargo-mutants (running at Phase 0 time). Both installed.

## Phase 0 status: COMPLETE

- Checkpoint met: DESIGN.md exists with a package path for every required
  subsystem; scaffold builds (`cargo build`).
- Commit count at phase end: 21 (19 pre-existing + 2 new).
- Production LOC (Rust, excl. tests/benches/fuzz): ~35 (scaffold only).
- Deviations: see table at top.