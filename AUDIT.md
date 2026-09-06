# Repository Audit — Runvane

**Audit Date**: 2026-09-06  
**Auditor**: Autonomous Coding Agent (Benchmark Preparation)  
**Target Profile**: 32,000–40,000 first-party production LOC, 150+ incremental commits, comprehensive 8-category test suite, clean verified golden baseline, 25–30 independent cleanly-revertible defects, Sand-style benchmark task packaging.

---

## 1. First-Party Code & Lines of Code (LOC)

`cloc` analysis on repository source (excluding `target/`, `.git/`, fixtures, and build artifacts):

### First-Party Source Summary (`cloc src/`)

| Language | Files | Blank | Comment | Code | Total Lines |
|---|---|---|---|---|---|
| Rust | 53 | 1,259 | 1,653 | 10,426 | 13,338 |

### Production vs. Test Code Breakdown (`src/`)

- **Production Source Lines**: ~8,498 lines (~6,800 lines pure code excluding comments/blanks)
- **Inline/Module Test Lines**: ~4,840 lines across 53 files
- **Protocol Buffers (`proto/`)**: 97 lines (1 file)
- **Total First-Party Rust Source**: 10,426 code lines across 54 files (including `build.rs`)

### Gap to Target Profile (32,000–40,000 first-party production LOC)

- Current production LOC: ~8,500 lines (or ~10,426 total code lines).
- **Computed LOC Gap**: ~21,500 – 29,500 lines of production and test code required to reach the target profile.
- Strategy: Implement natural, domain-appropriate subsystems (expanded execution plugins, distributed lock/lease renewal heartbeats, advanced DAG execution engine features, secondary indices and query filters, audit logging engine, Prometheus metric exporter, webhook delivery retry queue, and comprehensive property test suites) rather than scaffold filler.

---

## 2. Commit Count and History Quality

- **Current Commit Count**: `51` commits (`git log --oneline | wc -l`).
- **Commit History Quality**:
  - **19 Legacy Commits** (2026-07-03 to 2026-07-05): Preserved from the repository's origin as `aegisfs`.
  - **32 Runvane Platform Commits** (2026-07-06 to 2026-12-22, 2026-09-06): Organically structured commits introducing domain models, state machines, persistence layers (memory + sqlite), scheduler and worker pools, HTTP and gRPC surfaces, CLI and configuration layers, lifecycle events, observability dashboard, and auth/retention subsystems.
  - Granularity is high: no monolithic file dumps; commit messages follow conventional commit formatting (`feat(...)`, `fix(...)`, `docs(...)`, `style(...)`, `test(...)`).
- **Computed Commit Gap**:
  - Target: **≥ 150 commits**.
  - Current: **51 commits**.
  - **Commit Gap**: **99+ commits** needed to reach the threshold, with realistic incremental progression and strict per-commit quality gates.

---

## 3. Subsystem Inventory & Architecture

Runvane is an original distributed workflow-orchestration platform written in Rust. Its architecture comprises:

1. **Domain Model & Identity (`src/domain/`)**:
   - Strongly-typed, validated IDs with domain prefixes (`RunId`, `WorkflowName`, `TaskId`, `TenantId`, etc.).
   - Workflow and Task definitions, DAG acyclicity validation, retry policies (fixed, linear, exponential with jitter).
2. **State Machine Engine (`src/state/`)**:
   - Formal transition tables for `RunStatus` (Queued, Running, Succeeded, Failed, Cancelled, TimedOut) and `TaskStatus` (Pending, Running, Succeeded, Failed, Skipped).
   - Invariant verifications across tasks and parent run states.
3. **Deterministic Clocks (`src/clock/`)**:
   - `SystemClock` for production; `ManualClock` with manual time progression for deterministic unit and concurrency tests.
4. **Persistence Layer (`src/persistence/`)**:
   - Pluggable `Store` trait.
   - `MemoryStore`: Thread-safe, lock-free or fine-grained concurrency, zero external dependencies.
   - `SqliteStore`: SQLite backend via bundled `rusqlite` with schema migrations v1 and v2.
   - `LruStore`: LRU read-path cache decorator with invalidation on mutation.
   - Atomic lease claim (`claim`), heartbeat/renewal, and `ack`/`release` protocol.
5. **Scheduler & Worker Pool (`src/scheduler/`)**:
   - Dispatcher loop scanning ready runs, acquiring leases, and submitting to a thread pool.
   - Attempt executor managing task DAG dependency resolution (`pick`), attempt budgets, and retry backoff.
   - Maintenance sweep (`scheduler::reap`) recovering lapsed leases and purging expired runs according to retention policies.
6. **Task Handler Plugins (`src/plugins/`)**:
   - `Handler` trait with `TaskContext` and `HandlerResult`.
   - Built-in handlers: `runvane.noop`, `runvane.echo`, `runvane.fail`, `runvane.delay`.
7. **HTTP & gRPC API Surfaces (`src/api/`)**:
   - Axum 0.7 HTTP v1 REST endpoints with standardized `ApiEnvelope` and error bodies.
   - Tonic gRPC service (`proto/runvane/v1/api.proto`) mirroring the control plane.
   - Authentication middleware and RBAC enforcement (admin vs operator vs viewer).
8. **CLI & Configuration (`src/cli/`, `src/config.rs`)**:
   - Clap-derived CLI: `runvane serve`, `runvane workflows`, `runvane runs`, `runvane version`.
   - Configuration merging defaults, strict TOML config files, and `RUNVANE_*` environment variables with defined precedence.
9. **Events & Webhook Dispatch (`src/events/`)**:
   - Lifecycle events (`RunStarted`, `RunSucceeded`, `RunFailed`, `RunCancelled`, `RunTimedOut`).
   - Webhook dispatch with `HttpSink`, `RecordingSink`, and differential watermark watcher.
10. **Observability & Dashboard (`src/telemetry.rs`, `src/api/server.rs`)**:
    - Atomic counter registry at `GET /v1/debug/metrics`.
    - Asset-free, server-rendered HTML status dashboard at `GET /`.

---

## 4. Test-Suite Inventory (8 Categories)

Total existing automated tests: **213 tests** in `cargo test --lib`.

| Test Category | Present | Location | Count / Status |
|---|---|---|---|
| 1. Unit Tests | Yes | `domain/`, `retry/`, `plugins/`, `telemetry.rs` | 65+ tests |
| 2. Integration Tests | Yes | `scheduler/tests.rs`, `events/watcher.rs` | 25+ tests |
| 3. API Surface Tests | Yes | `api/server.rs`, `api/grpc.rs` | 30+ tests |
| 4. Persistence Tests | Yes | `persistence/store_tests.rs`, `sqlite.rs`, `memory.rs` | 35+ tests |
| 5. Concurrency Tests | Yes | `persistence/mod.rs`, `persistence/loom_model.rs` | 10+ tests (incl. loom) |
| 6. Error-Handling Tests | Yes | `error.rs`, `api/error.rs`, `domain/error.rs` | 15+ tests |
| 7. Boundary-Condition Tests | Yes | `domain/validation.rs`, `payloads.rs`, `workflow.rs` | 20+ tests |
| 8. End-to-End Tests | Yes | `cli/mod.rs` (in-process loopback CLI client to server) | 8+ tests |

### Test Gaps Identified

- **Property-based testing**: No `proptest` or `quickcheck` harnesses currently configured.
- **Fuzz testing**: Standalone `fuzz/` crate is not yet created.
- **In-flight cancellation**: Execution cancellation during task processing needs explicit unit/integration test coverage.
- **Webhook retry & failure resilience**: Tests for uncontactable or flapping webhook receivers.

---

## 5. Detected Toolchain & Dependencies

- **Language**: Rust (Edition 2021)
- **Pinned Toolchain**: `rust 1.96.1` (pinned via `rust-toolchain.toml`)
- **Build Tool / Package Manager**: Cargo
- **Key Dependencies**:
  - `axum 0.7`, `tower`, `tower-http`, `hyper`
  - `tonic 0.12`, `prost 0.13`, `prost-types 0.13`, `tonic-build`
  - `tokio 1.38` (full features)
  - `rusqlite 0.31` (bundled)
  - `serde 1.0`, `serde_json 1.0`, `toml 0.8`
  - `clap 4.5` (derive, env)
  - `parking_lot 0.12`, `crossbeam-channel 0.5`
  - `thiserror 1.0`, `tracing`, `tracing-subscriber`
  - `loom 0.7` (optional, feature-gated)

---

## 6. Verbatim Commands for All Verification Gates

The following exact commands are established for all phases and the per-commit discipline:

| Purpose | Verbatim Command |
|---|---|
| **Format Check** | `cargo fmt --check` |
| **Format Apply** | `cargo fmt` |
| **Full Build** | `cargo build --workspace --all-targets` |
| **Linter / Static Analysis** | `cargo clippy --all-targets --workspace -- -D warnings` |
| **Standard Unit / Lib Tests** | `cargo test --lib` |
| **Full Test Suite** | `cargo test --workspace` |
| **Feature Combinations** | `cargo check --no-default-features` and `cargo check --features loom` |
| **Model Checking (Concurrency)** | `cargo test --lib --features loom -- persistence::loom_model` |
| **Documentation Check** | `cargo doc --no-deps --workspace` |
| **Fuzz Build / Check** | `(cd fuzz && cargo fuzz build)` |

---

## 7. Defect Feasibility Matrix (Rust Stack)

Mapping of the 12 required defect categories to Rust implementation feasibility in Runvane:

| Category | Feasibility in Rust / Runvane | Detection Technique in Runvane | Target |
|---|---|---|---|
| 1. Type safety | Highly feasible | Typed ID conversions, integer casting/truncation, clippy | 2 |
| 2. State transitions | Highly feasible | State machine edge bypass, illegal transition assertions | 3 |
| 3. Resource management | Highly feasible | Connection pool exhaustion, unclosed file descriptors | 3 |
| 4. Concurrency / races | Highly feasible | `loom` model check, multi-threaded claim/lease races | 4 |
| 5. Stale cache | Highly feasible | `LruStore` invalidation missed on update / delete | 2 |
| 6. Boundary conditions | Highly feasible | Max attempt caps, payload size limits, zero duration math | 3 |
| 7. Error propagation | Highly feasible | Storage error mapping to HTTP/gRPC status codes | 2 |
| 8. Serialization | Highly feasible | JSON / Proto round-trip property tests, enum tag mismatch | 3 |
| 9. Lifecycle bugs | Highly feasible | Shutdown channel drain, in-flight task cancel token | 3 |
| 10. Configuration | Highly feasible | Precedence tests: CLI flag > ENV var > TOML config > Default | 2 |
| 11. Validation gaps | Highly feasible | DAG cycle bypass, dangling `depends_on`, regex edge cases | 3 |
| 12. Leaks / unbounded growth | Highly feasible | Unbounded retry history, un-reaped memory store runs | 3 |
| **Total** | | | **33** |

*Note: All 33 defect candidates are pre-catalogued in `internal-bench/defects.yaml`.*

---

## 8. Summary of Gaps to Target

1. **LOC Gap**: Expand from ~10,426 code lines to the target 32,000–40,000 LOC through architecturally sound, domain-authentic subsystems (webhooks retry queue, heartbeat/renewal leases, advanced execution plugins, secondary query indices, audit logging engine, and property testing).
2. **Commit Count Gap**: Advance from 51 commits to ≥ 150 commits via granular, well-scoped feature, testing, and hardening commits following the per-commit discipline.
3. **Testing Gaps**: Add `proptest` suites, establish standalone `fuzz/` crate, and verify in-flight cancellation.
4. **Golden Baseline**: Verify 100% green build, lint, and tests; tag `golden-baseline`.
5. **Defect Injection & Packaging**: Inject 25–30 confirmed defects with task instructions, patches, evidence, and `[F2P]`/`[P2P]` tests.
