# Commit Roadmap and Engineering Evolution Plan

This document establishes the coherent engineering progression of the **Runvane** distributed workflow orchestration platform across ~200 development commits leading to its production GitHub release.

---

## Evolution Phases & Milestones

### Phase 0: Repository Inception & Platform Architecture (Commits 001–008)
- Initial project scaffolding, workspace layout, Cargo dependencies.
- Architecture specification, domain boundary definitions, and coding standards.
- In-tree build plan and verification criteria.

### Phase 1: Core Domain Entities & Formal State Machines (Commits 009–026)
- Strongly typed Crockford Base32 and hex identifiers (`RunId`, `WorkflowId`, `TaskRunId`).
- Workflow definition specification, task specifications, and topological DAG sorting.
- Kahn's algorithm cycle detection and dependency validation.
- Formal finite state machines (`RunFsm`, `TaskFsm`) and invariant checkers.
- Input validation safeguards: recursion depth limits and payload byte bounds.

### Phase 2: Dual Persistence Backends & Caching Layer (Commits 027–048)
- Abstract `Store` trait for transactional workflow, run, and task lifecycle management.
- Concurrent in-memory storage engine backed by parking_lot mutexes for deterministic testing.
- Embedded SQLite persistence with WAL mode, index migrations, and cascade deletions.
- Transparent `LruStore` caching decorator with compound `(tenant, name)` keys and invalidation triggers.
- Reaping engine for terminal runs past retention deadlines.

### Phase 3: Scheduler, Worker Pool & Concurrency Throttling (Commits 049–072)
- Multi-threaded `WorkerPool` with thread panic barriers (`catch_unwind`) and clean shutdown.
- Lease-based queue dispatcher with monotonic tokens and periodic heartbeats.
- Exponential backoff with full and equal jitter support.
- Tenant-level concurrency limiters and leaky-bucket fair-share rate limiters.
- Standard 5-field cron parser and monotonic tick scheduler.

### Phase 4: Dynamic Expression Engine & Template Interpolation (Commits 073–086)
- Template parser resolving `${tasks.<task>.output.<field>}` and `${inputs.<field>}` variables.
- Dynamic boolean and numeric comparison expression evaluator.
- Type coercions, error propagation, and security recursion limits.

### Phase 5: Dual Transport Surface: REST API & gRPC (Commits 087–112)
- Axum HTTP/1.1 REST API with JSON schemas, query filters, and pagination boundaries.
- Tonic gRPC API with Protobuf definitions mirroring the HTTP control plane.
- Unified error translation mapping domain errors to HTTP 4xx/5xx and gRPC status codes.
- Operator CLI client with subcommands for workflow creation, run submission, and status inspection.

### Phase 6: Event Outbox & Webhook Dispatch Subsystem (Commits 113–128)
- Transactional event outbox logging all state transitions.
- Background watcher daemon matching events to workflow completion hooks.
- Asynchronous webhook HTTP client with HMAC-SHA256 signature verification and delivery deduplication.

### Phase 7: Observability, Metrics & Telemetry Dashboard (Commits 129–144)
- Lock-free atomic metric counters, latency histograms, and Prometheus `/v1/debug/metrics`.
- Server-rendered status dashboard with responsive layout and queue depth counters.
- Audit logging subsystem tracking operator actions.

### Phase 8: Hardening, Property Tests & Golden Baseline (Commits 145–164)
- `proptest` suites for cron parsing, backoff monotonicity, DAG topologies, and expression algebra.
- Boundary test suite for payload sizes, JSON nesting, timeout boundaries, and pagination limits.
- Formal Loom concurrency model testing for lock-free claims and cache races.
- Verified golden baseline tag (`golden-baseline`).

### Phase 9: Benchmark Defect Packaging (Commits 165–182)
- Packaging of all 33 Sand-style defect bundles (`RV-001` through `RV-033`).
- Complete defect catalog with root causes, mechanisms, and verification commands.

### Phase 10: Static Simulation, CLI Diagnostics & Storage GC (Commits 183–189)
- Compile-time static workflow simulation engine (`DryRunEngine`) with topological stage decomposition.
- `runvane dry-run` and `runvane stats` CLI commands.
- Content-addressable storage (CAS) automated garbage collection daemon (`ArtifactGc`).
- Rich SVG execution timeline and status distribution visualization on the dashboard.
- End-to-end multi-step orchestration test suite.

### Phase 11: Production Readiness & Release Stabilization (Commits 190–204)
- Reference production workflow examples (ETL pipelines, incident response, ML training).
- Automated validation test harness for example workflows.
- Operator configuration templates with inline field documentation.
- Shell completion subcommand for Bash, Zsh, and Fish.
- Multi-stage Docker containerization and Docker Compose setup.
- Automated benchmark task verification script.
- History and architecture audit reports.
- Final production stabilization.

---

## Target Metrics

- **Total Commits**: 204 commits (within target range 190–220).
- **Author Identity**: `Ayushjava07 <ayushjhasahab07@gmail.com>`.
- **Integrity**: 0 history rewrites, 100% existing functionality preserved, clean working directory.
