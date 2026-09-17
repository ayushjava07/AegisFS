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
| 1 — domain + state machine + persistence | complete |
| 2 — scheduler/worker pool/retry/backoff | in progress |
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
   `aegisfs`). The `fuzz/` crate is created in a later phase as a *standalone*
   package (NOT a workspace member) so `cd fuzz && cargo fuzz` drives it
   exactly as the tooling prefers; `benches/` stay inside the main package.

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

## Phase 2 status: IN PROGRESS

Checkpoint framework for phase completion:

- [x] Retry/backoff: seeded ChaCha8 `Backoff` (fixed/linear/exponential +
      full/equal/none jitter), `RetryPlanner`, `FailureKind::retryable()`.
- [x] Handler plugins: `Handler` trait, `TaskContext`, `HandlerResult`,
      `HandlerError` (transient/permanent), `Registry` with built-ins
      (`runvane.{noop,echo,fail,delay}`).
- [x] Scheduler core: `pick` (ready set incl. failed-task retries, skipped
      chain collapse), `executor` (attempt engine, run fsm `Running ->
      Failed -> Queued` retry walk, run+task budgets, `RunAction`), `pool`
      (scan/claim dispatcher + threaded `WorkerPool`, `SyncSender` bound
      channel, ack/release driver applying claim tokens).
- [x] E2E determinism: `ManualClock` + `MemoryStore` + seeded backoff;
      linear success, parallel runs, retry-until-giveup, retry-then-succeed,
      dependency-skip chain. No wall-clock dependence beyond bounded polling.
- [x] Deadlines: `deadline_at_ms` enforced in the attempt engine (run walks
      `Running -> TimedOut` without touching tasks when past deadline).
- [x] Lease recovery: `scheduler::reap` maintenance pass over the store's
      `recover_expired_leases`, un-leasing only lapsed leases.

Remaining in phase 2:

- [ ] Events/notifications window brought forward only if needed for the API
      phase; otherwise parked until Phase 5.

Verified at this point: `cargo test --lib` = 130 passed (126 without the
`sqlite` feature); `cargo clippy --all-targets` zero warnings.
Commit count entering phase 2: 26.

## Phase 1 status: COMPLETE

- Domain model, state machine, and persistence all landed and verified.
- State machine: `state::machine` generic engine + `state::run_fsm` (8 legal
  run edges, exhaustively pinned) + `state::task_fsm` (6 legal edges) +
  `state::invariant` cross-entity checks + `state::snapshots`. Every legal
  edge and a table-driven set of illegal edges are asserted (>= 3 illegal per
  machine).
- Clock: `clock::{SystemClock, ManualClock}` deterministic time injected into
  all scheduling paths.
- Persistence: single `Store` trait, `MemoryStore` (thread-safe, no global
  lock) and `SqliteStore` (rusqlite, committed migrations v1+v2, optimistic
  version guard on workflows, token-guarded queue leases). Shared behavior
  suite runs against **both** backends; concurrency tests prove single-winner
  claims and unique run numbers under threads.
- Feature gates: `default = ["sqlite"]`; `--no-default-features` build green
  (no `rusqlite`). `loom` optional behind `loom` feature (Phase 7).
- `cargo build` zero warnings; `cargo clippy --all-targets` zero warnings;
  `cargo test --lib` = 101 passed (97 without the `sqlite` feature).
- Commit count at phase end: 26 (19 pre-existing + 7 build commits).
- Production LOC (cloc `src`, 27 Rust files, incl. test blocks):
  3,875 code + 752 comment lines. Test-block lines are counted separately at
  Phase 8 when the mix is re-audited against the 32-40k production target.
- Deviations: see table at top.