# Changelog

All notable changes to Runvane are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] - 2026-09-06

### Added
- **Core Engine & Workflow State Machine**:
  - Validated DAG representation with Kahn's topological sort and cycle rejection.
  - Finite state machines for workflow runs and individual tasks with transition table verification.
  - Template expression evaluation engine supporting variable interpolation, numeric comparisons, and boolean logic.
- **Persistence Architecture**:
  - Transactional SQLite backend with WAL mode, index migrations, and cascading deletions.
  - Thread-safe in-memory store for isolated testing and high-speed execution.
  - Transparent `LruStore` caching layer with compound tenant keys and write-invalidation.
- **Scheduler & Execution**:
  - Deterministic worker pool with monotonic lease tokens and heartbeat renewal.
  - Exponential backoff algorithm with full and equal jitter support.
  - Worker thread panic boundary isolation via `catch_unwind` with automated lease release.
  - Fair-share tenant rate throttling and leaky-bucket concurrency limiter.
  - 5-field cron parser and monotonic schedule calculation.
- **Dual Transport Surface**:
  - Axum HTTP/JSON REST API with rich query filters, pagination boundaries, and server-rendered status dashboard.
  - Tonic gRPC API with protobuf specifications, streaming run monitors, and strict validation.
  - Unified error translation mapping platform errors to HTTP 4xx/5xx and tonic status codes.
- **Outbox & Event Bus**:
  - Outbox event table recording run state transitions.
  - Watcher polling daemon delivering webhook events with HMAC-SHA256 signatures and dedup delivery IDs.
- **Content-Addressable Artifact Store**:
  - SHA-256 CAS engine with prefix-sharded filesystem layouts and in-memory test mocks.
- **Verification Suites**:
  - Property-based testing via `proptest` for cron parsing, backoff monotonicity, DAG structures, and expression logic.
  - Multi-backend integration tests for concurrency contention, cascade deletion, and multi-tenant isolation.
  - Standalone fuzzing harness (`fuzz/`) with targets for CLI parsing, query filters, and tags deserialization.
  - Formal Loom concurrency models for lock-free lease acquisition and cache races.
- **Static Simulation & Diagnostics**:
  - `DryRunEngine` with topological stage decomposition, critical path analysis, and template reference reachability checking.
  - `runvane dry-run` CLI command rendering ASCII stage diagrams or structured JSON simulation reports.
  - `runvane stats` CLI command querying live control plane health, queue latency, and run status distributions.
- **Content-Addressable Artifact Retention Daemon**:
  - `ArtifactGc` worker for automated retention cleanup of expired blobs with active reference preservation and dry-run support.
- **Enriched HTML/SVG Dashboard**:
  - Dynamic SVG run distribution bar, responsive telemetry cards, and status badge styling.
- **End-to-End Orchestration Suite**:
  - Multi-tier diamond DAG execution, CAS artifact deduplication, tenant concurrency throttling, and fan-out/fan-in verification.
- **Benchmark Suite**:
  - Tagged `golden-baseline` clean release commit.
  - 33 packaged benchmark tasks (`internal-bench/tasks/RV-001` through `RV-033`) with `task.yaml`, `instructions.md`, `defect.patch`, `solution.patch`, and `test_patch.diff`.
  - Comprehensive defect taxonomy documented in `internal-bench/defects.yaml`.
