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
| 2 — scheduler/worker pool/retry/backoff | complete |
| 3 — HTTP + gRPC API | complete |
| 4 — CLI + config layer | complete |
| 5 — plugins + webhooks/events | complete |
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

## Benchmark-preparation engine (enhanced spec, folded in)

The updated build prompt tightened the brief: explicit numeric gates, a
defect-tracking manifest, and per-defect task packaging. This section is the
operational summary; the Rust tooling below is settled by Deviation 1 plus the
prompt's own "different language" swap table.

### Numeric gates

- Commits: `git log --oneline | wc -l` >= 150 at finish.
- Production LOC: 32,000-40,000 first-party `src` lines (cloc, excluding
  tests/benches/fuzz/generated), tracked per phase below.
- Golden tag at Phase 9: zero known defects, all runs green.

### Defect catalogue and packaging

- Manifest: `internal-bench/defects.yaml` — tracked in-repo but kept out of
  any task extraction by construction. Holds 30-36 candidates; 25-30 must
  survive independence review as distinct root causes.
- Each entry records: `id`, `subsystem`, `category`, `mechanism` (the precise
  realistic mistake), `detection` (specific test/fuzz/tool run), `fix`, and an
  `independence check` against every other entry.
- Injection is one isolated, normally-worded, **cleanly revertible** commit
  per defect on top of the golden tag; the golden tag is the all-fixed state.
- Packaging (Phase 11) per surviving defect: broken state = golden minus that
  one revert; solution patch = the fix diff; `instructions.md` describing the
  operator-visible symptom (never the fix); >= 1 `[F2P]` regression test that
  fails on broken/passes on fixed; >= 2 `[P2P]` tests passing in both states;
  evidence = dashboard screenshot or captured terminal/log transcript.
- Phase 8 adds the `[F2P]`/`[P2P]` markers to the intended tests now, while
  intent is fresh, even though defects are not yet injected.

### Category table (with Rust detection)

| Category | Target | Detection in Rust |
|---|---|---|
| Type-safety mistakes | 2 | clippy + typed-id discipline + targeted unit tests |
| Incorrect state transitions | 3 | state-machine table tests + property tests |
| Resource-management problems | 3 | explicit close-path tests, RAII/drop audits |
| Concurrency/race conditions | 4 | `cargo miri` + `loom` model checks + stress tests |
| Stale-cache behavior | 2 | integration tests on the invalidation paths |
| Boundary-condition errors | 3 | table-driven boundary tests + `cargo fuzz` |
| Incorrect error propagation | 2 | `errors.Is/As`-style tests over typed errors |
| Serialization inconsistencies | 3 | round-trip property tests (`proptest`) |
| Lifecycle bugs | 3 | startup/shutdown/cancel integration tests |
| Configuration mistakes | 2 | precedence tests (flag > env > file > default) |
| Validation gaps | 3 | negative-case API/boundary tests |
| Memory/resource leaks | 3 | explicit close-path + allocator/drop audits |

Tools: mutation = `cargo-mutants` scoped per package (state machine, retry,
validation) with a recorded mutation score; fuzz = `fuzz/` standalone crate
(deviation 4) targeting every hand-written parser/decoder; leak = ownership
model + explicit close-path harness.

### Definition of done (checked at Phase 12)

- [ ] `git log --oneline | wc -l` >= 150
- [ ] cloc first-party `src` in 32,000-40,000
- [ ] `cargo build` + `cargo test --lib` green on golden tag (both feature
      sets)
- [ ] `cargo clippy --all-targets` and `cargo fmt --check` clean
- [ ] `cargo miri`/`loom` model checks clean where applicable
- [ ] fuzz targets for every hand-written parser/decoder, local runs green
- [ ] scoped `cargo-mutants` pass recorded with score
- [ ] `internal-bench/defects.yaml` 30-36 candidates, 25-30 confirmed
- [ ] every surviving defect isolated + cleanly revertible; >=1 `[F2P]` and
      >=2 `[P2P]` tests; complete broken/fixed/patch/instructions/evidence
      bundle
- [ ] README/CONTRIBUTING/CHANGELOG/LICENSE consistent with commit history
- [ ] no build/test step needs the network; no timing-flaky assertions

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

## Phase 2 status: COMPLETE

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

## Phase 2 status: COMPLETE

- Retry/backoff, handler plugins, scheduler (pick/executor/pool), e2e
  determinism, deadline enforcement, and lease reaping all landed.
- Retry walk on the run machine: `Running -> Failed -> Queued`; failed tasks
  are re-selected by the picker on later attempts and only while their own
  budget remains; skip is sticky for already-skipped descendants.
- Commit count at phase end: 29 (19 pre-existing + 10 build commits).
- `cargo test --lib` = 130 passed (126 without `sqlite`); clippy clean.
- Production LOC (cloc `src`, 37 Rust files, incl. test blocks):
  5,480 code + 1,007 comment lines.
- Deviations: see table at top.

## Phase 3 status: COMPLETE

- HTTP v1 surface in `src/api`: uniform `ApiEnvelope` + stable error bodies
  (`ErrorBody` mapped through `RunvaneError::http_status()`); axum router
  (0.7 `:param` segments) with health, workflow create/list/get, and run
  submit/list/get/cancel handlers. Requests carry a versioned payload subset;
  responses return the canonical domain documents unchanged.
- Wire contract pinned by router tests (real MemoryStore + ManualClock,
  `tower::ServiceExt::oneshot`): status codes, envelope shape, error codes,
  list filters, cancel idempotency, and the 1 MiB run-input boundary.
- `Store::cancel_run` added to the trait and both backends: transitions
  `Queued | Running -> Cancelled`, removes the queue entry unconditionally
  (no lease race), returns `false` for terminal runs; pinned in the shared
  backend suite.
- gRPC mirror in `src/api/grpc.rs` from `proto/runvane/v1/api.proto`
  (`tonic-build` moved to `[build-dependencies]`, `build.rs` added): same
  operations over HTTP/2; elastic documents travel as serialized JSON in
  `bytes` fields. In-process loopback tests cover create/submit/get/cancel
  round trips and tonic error-code mapping (incl. `InvalidArgument`,
  `NotFound`, `FailedPrecondition`).
- Commit count at phase end: 34 (19 pre-existing + 15 build commits).
- `cargo test --lib` = 148 passed (144 without `sqlite`); `cargo clippy
  --all-targets` zero warnings; `--no-default-features` build green.
- Production LOC (cloc `src`, 42 files, incl. test blocks):
  6,978 code + 1,160 comment lines.
- Deviations: see table at top.

## Phase 4 status: COMPLETE

- Config layer in `src/config.rs`: defaults, a strict TOML file layer
  (unknown keys fail loudly), and `RUNVANE_*` environment overrides; the
  pure `apply_env_map` split keeps precedence tests free of process-global
  state. Explicit validation rejects port-0 and non-positive durations.
- `src/cli` with clap derive: `runvane serve`, `workflows
  {create,list,get}`, `runs {submit,list,get,cancel}`, `version`.
- `serve` wires config -> store (in-memory or SQLite) -> registry built-ins
  -> shared clock -> axum + tonic listeners, an optional scheduler thread
  (worker pool + dispatcher + lease reap) gated by `--workers 0`, and
  cooperative ctrl-c shutdown. Flags > env > file > defaults pinned by tests.
- Control-plane subcommands are thin shells over the **gRPC client**
  (`src/cli/client.rs`), dogfooding the exact protocol stubs the server
  serves; output is one pretty JSON document per invocation. Spec files map
  onto the proto payloads with strict subsections and duplicate-tag rejection.
- In-process loopback e2e proves create -> submit -> list through the CLI
  against a running control plane; the real binary was smoke-tested
  (`runvane --help`, `runvane version`).
- Commit count at phase end: 37 (19 pre-existing + 18 build commits).
- `cargo test --lib` = 176 passed (172 without `sqlite`); `cargo clippy
  --all-targets` zero warnings.
- Production LOC (cloc `src`, 46 Rust files, incl. test blocks):
  8,211 code + 1,334 comment lines.
- Deviations: see table at top.

## Phase 5 status: COMPLETE

- Event model in `src/events`: `EventKind` (started + the four terminal
  states) with stable wire codes, and a `RunEventDoc` that is the payload
  for every transport (webhook body, logs, tests).
- Hook dispatch in `src/events/dispatch.rs`: `match_hooks` selects the
  right lifecycle slice and applies `event_filter` (code or status-name);
  deliveries carry a sha256-derived stable id for consumer dedup. Sinks are
  a plain trait: `RecordingSink` (test/audit), `LoggingSink` (serve
  default), and a dependency-free `HttpSink` that POSTs to plain-http
  loopback receivers (pinned by a real one-shot loopback exchange).
- Differential watcher in `src/events/watcher.rs`: watermark cursors
  (started, finished) anchored at boot replay nothing; one poll observes,
  orders by `(timestamp, run_id)`, dispatches start-before-terminal, and
  advances watermarks only after dispatch. Wired into the `serve` scheduler
  thread.
- Hooks now travel the full wire contract: `HooksSpec`/`HookSpec` messages
  in the proto, converted + URL-validated in the gRPC handler (bad URLs ->
  `InvalidArgument`), and accepted in CLI spec files; HTTP payloads already
  carried `Hooks`. Round-trip and rejection tests on both transports.
- `serve` installs the plugin registry built-ins; definitions created via
  any transport can bind hooks that the scheduler fires.
- Commit count at phase end: 40 (19 pre-existing + 21 build commits).
- `cargo test --lib` = 193 passed (189 without `sqlite`); `cargo clippy
  --all-targets` zero warnings.
- Production LOC (cloc `src`, 49 Rust files, incl. test blocks):
  9,162 code + 1,478 comment lines.
- Deviations: see table at top.

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