# Enhancement Plan — Runvane

**Based on Phase 0 Audit (`AUDIT.md`)**  
**Target Profile**: 32,000–40,000 LOC, ≥150 commits, comprehensive 8-category test suite, verified golden baseline, 25–30 catalogued & packaged benchmark defects.

---

## 1. Growth Gap Strategy (Domain-Authentic Subsystems)

To bridge the LOC gap (~10,426 current code lines to target 32,000–40,000 LOC) without artificial filler, the following 8 cohesive, domain-appropriate subsystems will be added to the Runvane platform:

### Subsystem 1: In-Flight Task Cooperative Cancellation & Token Propagation
- **Domain Need**: When an operator cancels a running workflow (`cancel_run`), worker threads executing tasks need prompt cooperative cancellation rather than spinning to completion.
- **Components**:
  - `CancellationToken` in `src/plugins/handler.rs` and `TaskContext`.
  - Worker poll and executor abort logic in `src/scheduler/executor.rs` and `src/scheduler/pool.rs`.
  - Cancellation state propagation in SQLite and In-Memory stores.

### Subsystem 2: Distributed Lease Heartbeating & Lock Renewal
- **Domain Need**: Long-running workflow tasks can exceed standard lease timeouts (`lease_ms`). A background lease-heartbeating mechanism ensures valid running tasks are not stolen by competitor workers.
- **Components**:
  - Store primitives: `renew_lease(run_id, token, extend_by_ms)` in `Store` trait, `MemoryStore`, and `SqliteStore`.
  - Periodic heartbeat driver in `WorkerPool` during attempt execution.
  - Metrics tracking lease renewal frequency and failures.

### Subsystem 3: Durable Webhook Retry Delivery Outbox & Dead-Letter Queue (DLQ)
- **Domain Need**: Webhooks currently fire best-effort. Network blips drop delivery without retries.
- **Components**:
  - `WebhookOutbox` model and persistence table in SQLite (`migrations/v3_webhooks.sql`) and MemoryStore.
  - Webhook dispatcher retry loop with exponential backoff and max attempt exhaustion.
  - Dead-letter event archive for terminal webhook delivery failures.

### Subsystem 4: Advanced Run Querying, Tag Faceting & Temporal Filters
- **Domain Need**: Operators need to search runs by arbitrary metadata tags, temporal execution windows (`started_before`, `finished_after`), and status unions.
- **Components**:
  - Enhanced `RunFilter` predicate engine with multi-tag matching (ALL/ANY), timestamp range filters, and definition name globbing.
  - SQL index optimization in SQLite and memory-efficient indexed lookups in `MemoryStore`.
  - HTTP and gRPC query parameter expansions.

### Subsystem 5: Production Execution Handlers (`runvane.http` & `runvane.script`)
- **Domain Need**: Workflows need realistic task execution capabilities beyond echo and noop.
- **Components**:
  - `runvane.http`: Outgoing HTTP request task handler with method, headers, payload, response status assertions, and timeout guards.
  - `runvane.script`: Safe subprocess / script execution handler with environment sandboxing, timeout enforcement, and stdout/stderr capture into `TaskRun` outputs.
  - Built-in registration in `plugins::handler::Registry`.

### Subsystem 6: Structured Audit Logging Engine
- **Domain Need**: Administrative actions (workflow creation, run cancellation, token authentication failures, schema changes) require durable audit trails.
- **Components**:
  - `AuditLogger` trait and implementations (`FileAuditLogger`, `MemoryAuditLogger`, `StoreAuditLogger`).
  - Plumbed into API middleware, CLI control plane, and scheduler maintenance workers.
  - Queryable `/v1/audit` endpoint and CLI `runvane audit list`.

### Subsystem 7: Prometheus Metrics Exposition & System Health Diagnostics
- **Domain Need**: Real-time production observability with Prometheus scraper compatibility.
- **Components**:
  - Standard text-based Prometheus exposition handler at `/metrics`.
  - Histograms for run duration, task duration, and lease hold duration.
  - Gauges for active worker thread utilization, queue depth by priority, and memory store footprint.

### Subsystem 8: Standalone Fuzzing Crate (`fuzz/`) & Property-Based Verification
- **Domain Need**: Robust verification of hand-written decoders, parsers, and state transitions.
- **Components**:
  - Standalone `fuzz/` package with 5 targets: `fuzz_priority_parse`, `fuzz_run_filter`, `fuzz_event_filter`, `fuzz_cli_spec`, `fuzz_tags_json`.
  - `proptest` suites for JSON round-tripping, DAG validation invariants, and retry policy math.

---

## 2. Commit Count Roadmap (≥ 150 Commits)

With 51 existing commits, ~100 incremental commits will be produced across Phases 2 through 8:

| Phase | Planned Scope | Commit Estimate | Target Cumulative Commits |
|---|---|---|---|
| **Phase 2: Growth Gap** | Subsystems 1 to 7 (cooperative cancel, lease renewal, webhook outbox, run search, plugins, audit, prometheus) | ~40 commits | ~91 commits |
| **Phase 3: Hardening** | Input validation bounds, resource cleanup audits, strict error mapping, clippy clean | ~15 commits | ~106 commits |
| **Phase 4: Test Completion** | `fuzz/` scaffolding, `proptest` suites, boundary tests, witness test tags (`[F2P]`/`[P2P]`) | ~20 commits | ~126 commits |
| **Phase 5: Golden Baseline** | Golden baseline verification, model checking (`loom`), `golden-baseline` tag | ~4 commits | ~130 commits |
| **Phase 6: Defect Injection** | 25–30 isolated, cleanly-revertible defect commits from catalogue | ~25 commits | ~155 commits |
| **Phase 7: Packaging** | Sand-style task packages (`instructions.md`, patches, evidence) | ~10 commits | ~165 commits |
| **Phase 8: Finalize** | Docs, changelog, audit sync, benchmark notes | ~5 commits | ~170 commits |

---

## 3. Defect Category Distribution (Rebalanced for Rust)

Target: 33 catalogued candidates; 25–30 confirmed independent injected defects.

| Category | Target | Grounded Rust Mechanism |
|---|---|---|
| 1. Type safety | 2 | Wire conversion integer truncation, signed/unsigned bounds |
| 2. State transitions | 3 | Run FSM retry edge violation, stale status re-queue |
| 3. Resource management | 3 | Worker pool channel disconnection, unclosed connection leaks |
| 4. Concurrency / races | 4 | Claim/ack token reuse race, TOCTOU status read, lease expiration race |
| 5. Stale cache | 2 | LRU eviction key misconstruction, missed invalidation on delete |
| 6. Boundary conditions | 3 | Zero/negative delay arithmetic, max payload boundary off-by-one |
| 7. Error propagation | 2 | Swallowed database errors mapped to 500 instead of 404/400 |
| 8. Serialization | 3 | Protobuf bytes JSON mismatch, enum discriminator parsing |
| 9. Lifecycle bugs | 3 | Worker pool shutdown hang, missing in-flight cancel check |
| 10. Configuration | 2 | Nested TOML typo acceptance, env variable integer overflow |
| 11. Validation gaps | 3 | Dangling `depends_on` references, unclamped description lengths |
| 12. Leaks / growth | 3 | Unbounded retry attempt history, HTTP sink socket exhaustion |
| **Total** | **33** | |

---

## 4. Per-Commit Discipline Protocol

Every commit authored from Phase 2 onward adheres strictly to:
1. `cargo fmt --check` must pass.
2. `cargo build --workspace --all-targets` must succeed.
3. `cargo clippy --all-targets --workspace -- -D warnings` must report 0 warnings.
4. `cargo test --lib` (and touched package tests) must pass with 0 regressions.
5. Diff size strictly within ~8% of total LOC.
6. Commit trailer format:
   ```
   Gate: build=pass lint=pass tests=pass race=n/a
   ```
7. Running entry recorded in `## Commit Log` below.

---

## Commit Log

f1a8f47 | phase 2 | gate: PASS | feat(scheduler): support cooperative in-flight task cancellation and token signalling
839c1f0 | phase 2 | gate: PASS | feat(persistence): implement lease renewal primitive across memory and sqlite backends
b766915 | phase 2 | gate: PASS | feat(scheduler): heartbeat and renew claim leases across multi-task execution
86da055 | phase 2 | gate: PASS | feat(events): add durable webhook retry outbox and dead-letter queue (DLQ)
ff8a926 | phase 2 | gate: PASS | feat(persistence): expand RunFilter with multi-status, tag keys, finished bounds, duration, and pagination offset
d86f450 | phase 2 | gate: PASS | feat(plugins): implement runvane.http and runvane.script execution handlers
9b0c257 | phase 2 | gate: PASS | feat(audit): introduce structured audit logging engine with memory and file backends
2d41573 | phase 2 | gate: PASS | feat(api): expose /v1/audit endpoint and record operational audit events
b29aa64 | phase 2 | gate: PASS | feat(telemetry): add Prometheus text exposition and diagnostic health probes
0067e04 | phase 2 | gate: PASS | feat(domain): add workflow versioning, canary routing, and compatibility analysis
3e2b792 | phase 2 | gate: PASS | feat(engine): implement task expression evaluation and variable templating
8d8939b | phase 2 | gate: PASS | feat(scheduler): implement cron schedule parser and periodic trigger engine
787dcfc | phase 2 | gate: PASS | feat(scheduler): implement multi-tenant concurrency throttling and leaky-bucket limiter
9b52ece | phase 2 | gate: PASS | feat(storage): implement content-addressable artifact store with disk and memory backends
eaa0658 | phase 3 | gate: PASS | feat(validation): enforce maximum JSON nesting depth to prevent recursion overflow
67f434e | phase 3 | gate: PASS | feat(api): expand RunQuery with rich filters and strict query boundary validation
13d334f | phase 3 | gate: PASS | feat(scheduler): install panic boundary on worker thread loop with lease release
1ea0f7e | phase 3 | gate: PASS | feat(retry): harden backoff calculation against floating-point and integer overflow
7110e89 | phase 4 | gate: PASS | test(retry): add property-based test suite for backoff curves and jitter invariants
baeeeef | phase 4 | gate: PASS | test(dag): add property-based test suite for topological sorting and cycle detection
7e4bf47 | phase 4 | gate: PASS | test(scheduler): add property-based test suite for cron parser and tick calculation
6eea3a4 | phase 4 | gate: PASS | test(engine): add property-based test suite for expression evaluation and interpolation
a2e6c5c | phase 4 | gate: PASS | test(boundary): add comprehensive boundary-condition test suite
