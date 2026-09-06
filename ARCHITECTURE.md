# Runvane Architecture

This document details the architectural design, subsystems, concurrency model, and data flow of the Runvane distributed workflow orchestration platform.

```
                     +---------------------------------------+
                     |           Clients / Operators         |
                     +---------------------------------------+
                                  |                 |
                         (REST / JSON)        (gRPC / Proto)
                                  v                 v
                     +---------------------------------------+
                     |               API Layer               |
                     |  Axum HTTP Server     Tonic gRPC Svc  |
                     +---------------------------------------+
                                        |
                                        v
+-------------------------------------------------------------------------------+
|                                Control Plane                                  |
|                                                                               |
|  +------------------+     +-------------------+     +----------------------+  |
|  |  Domain & DAG    |     |  Scheduler Engine |     |  State Machines      |  |
|  |  Validation      |     |  Dispatcher       |     |  RunFsm / TaskFsm    |  |
|  +------------------+     |  WorkerPool       |     +----------------------+  |
|                           |  Lease Heartbeat  |                               |
|                           +-------------------+                               |
|                                     |                                         |
|  +----------------------------------+--------------------------------------+  |
|  |                                                                         |  |
|  v                                                                         v  |
| +-------------------+    +--------------------+    +--------------------+  |  |
| | Storage Abstr.    |    | Event Outbox Bus   |    | Artifact Storage   |  |  |
| | MemoryStore       |    | Watcher Poller     |    | SHA-256 CAS        |  |  |
| | SqliteStore (WAL) |    | Webhook Sinks      |    | Sharded On-Disk    |  |  |
| | LruStore Cache    |    | Dedup Delivery IDs |    +--------------------+  |  |
| +-------------------+    +--------------------+                            |  |
+-------------------------------------------------------------------------------+
```

---

## 1. Subsystems Overview

### 1.1 Domain Layer (`src/domain/`)
- **Entities**: Strongly-typed domain identifiers (`RunId`, `TaskId`, `TenantId`, `HandlerId`) using Crockford Base32 and hex representations to prevent cross-domain key collisions.
- **Workflow & Run Definitions**: Declarative workflows with metadata, versioning, timeout budgets, execution limits, and structured input/output JSON schemas.
- **DAG & Cycle Detection**: Kahn's algorithm implementation for topological sorting, validating absence of cyclic dependencies and detecting unreachable nodes.
- **Input Validation**: Strict recursion depth limits (`MAX_JSON_DEPTH = 64`) and payload byte ceilings (`MAX_RUN_INPUT_BYTES`) to protect against stack and heap overflow.

### 1.2 State Machines (`src/state/`)
- **Run FSM**: Enforces valid transitions across lifecycle milestones:
  `Queued -> Running -> Succeeded / Failed / TimedOut / Cancelled`.
- **Task FSM**: Governs individual task nodes:
  `Pending -> Running -> Succeeded / Failed / Skipped`.
- **Invariant Checking**: Invariant checker ensuring terminal runs have no active uncompleted tasks, and skipped tasks only occur when parent dependencies fail.

### 1.3 Persistence Layer (`src/persistence/`)
- **`Store` Trait**: Unified transactional interface for workflow definitions, runs, and queue entries.
- **`MemoryStore`**: Concurrent, thread-safe in-memory implementation backed by `parking_lot::Mutex` and monotonic timestamps.
- **`SqliteStore`**: Embedded relational persistence with WAL (Write-Ahead Logging), multi-index schema migrations, foreign keys, and cascading cleanup.
- **`LruStore`**: Caching decorator providing transparent O(1) reads for warm workflow definitions, keyed by compound `(Tenant, Name)` tuples with write-invalidation.

### 1.4 Scheduler & Execution Engine (`src/scheduler/`)
- **Monotonic Leasing**: Queue claims acquire time-bounded leases with explicit claim tokens (`ClaimToken`). Workers periodically renew leases via heartbeats.
- **Worker Pool**: Thread pool with bound channels, panic barriers (`std::panic::catch_unwind`), and graceful termination drains.
- **Failure Recovery & Reaping**: Background lease reaper scans expired leases and returns abandoned runs to the ready queue.
- **Fair-Share & Throttling**: Leaky-bucket rate limiters and tenant interleaving preventing noisy-neighbor starvation.
- **Cron Scheduling**: Standard 5-field cron parser computing deterministic monotonic ticks for periodic workflows.

### 1.5 Event Delivery & Outbox (`src/events/`)
- **Outbox Bus**: Transactional event logging recording run state transitions.
- **Watcher Poller**: Scans run state changes and matches them against workflow hook specifications.
- **Webhook Sinks**: Asynchronous HTTP client sink with signature generation (HMAC-SHA256) and deduplicated delivery keys (`delivery_id`).

### 1.6 Artifact Storage (`src/storage/`)
- **Content-Addressable Storage (CAS)**: Deduplicated blob storage where objects are addressed by their SHA-256 digests.
- **Sharded Layout**: Hex-prefix sharded directory structures avoiding single-directory filesystem performance degradation.

---

## 2. Cross-Cutting Patterns

### Error Handling & Propagation
Runvane enforces strict error boundaries:
- Internal domain errors (`DomainError`) and storage errors (`StorageError`) unify into `RunvaneError`.
- `ApiError` translates domain errors to corresponding HTTP status codes (e.g., 400 Bad Request, 404 Not Found, 409 Conflict) and gRPC status codes (`tonic::Status`).
- Errors never leak low-level stack traces across external network boundaries.

### Monotonic Clocks
All scheduling, lease expiration, cron calculations, and event timestamps rely on abstract monotonic clock abstractions (`Clock` trait, `SystemClock`, and mock `ManualClock`), ensuring clock drift or leap seconds do not cause race conditions or premature lease drops.
