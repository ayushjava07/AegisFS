# Runvane Benchmark & Evaluation Notes

This document describes the benchmark architecture, defect taxonomy, and task packaging format used to evaluate autonomous software engineering agents on Runvane.

---

## 1. Overview & Evaluation Goals

Runvane is designed to serve as a rigorous, industrial-scale benchmark for evaluating autonomous coding agents on realistic, complex software engineering tasks. The benchmark models real-world issues encountered in production distributed systems:
- Distributed race conditions and TOCTOU bugs.
- Boundary conditions (overflows, underflows, recursion limits).
- Cache invalidation and multi-tenant isolation leaks.
- Cross-transport protocol desynchronization (HTTP REST vs gRPC Protobuf).
- Resource leaks, panic handling, and graceful shutdown lifecycles.

The clean, verified platform reference is pinned at git tag `golden-baseline`.

---

## 2. Sand-Style Task Packaging Format

Each evaluation task lives in `internal-bench/tasks/RV-XXX/` and consists of five standardized artifacts:

```
internal-bench/tasks/RV-XXX/
├── task.yaml          # Machine-readable task metadata and evaluation command
├── instructions.md    # Problem description presented to the agent
├── defect.patch       # Reversible patch that injects the bug into golden-baseline
├── solution.patch     # Reference solution patch resolving the defect
└── test_patch.diff    # Fail-to-pass (F2P) test witness verifying resolution
```

### Execution Protocol

1. **Setup**: The evaluation harness checks out `golden-baseline` and applies `defect.patch` and `test_patch.diff`.
2. **Initial Verification**: Running the test command (`task.yaml:test_command`) must fail (F2P).
3. **Agent Invocation**: The agent is provided with `instructions.md` and workspace access.
4. **Resolution Verification**: The agent's proposed changes must cause the test command to pass without regressions across the workspace test suite (`cargo test --workspace`).

---

## 3. Defect Taxonomy & Task Catalog

A total of 28 independent defect tasks are packaged:

| Task ID | Category | Subsystem | Title |
|:---|:---|:---|:---|
| **RV-001** | `type_safety` | api (gRPC) | gRPC List Limit Upper Boundary Clamping |
| **RV-002** | `type_safety` | persistence | Tenant-Isolated Monotonic Run Sequences |
| **RV-003** | `state_transition` | scheduler | Re-queue Clamping for Timed-Out Runs |
| **RV-004** | `state_transition` | scheduler | Pre-Execution Cancellation Verification |
| **RV-005** | `state_transition` | scheduler | Terminal Status Preservation Across Release Races |
| **RV-006** | `resource_mgmt` | scheduler | Non-Zero Worker Thread Pool Validation |
| **RV-007** | `resource_mgmt` | scheduler | Status Persistence on Lease Release Races |
| **RV-008** | `resource_mgmt` | scheduler | Worker Thread Panic Boundary with Lease Cleanup |
| **RV-009** | `concurrency` | scheduler | Re-check State Inside Claim Critical Section (TOCTOU) |
| **RV-010** | `concurrency` | persistence | Atomic Slot Invalidation in LRU Cache |
| **RV-011** | `concurrency` | events | Deduplication of Terminal Event Deliveries |
| **RV-012** | `concurrency` | scheduler | Monotonic Clock Source Propagation in Leases |
| **RV-013** | `stale_cache` | persistence | Compound `(Tenant, Name)` Cache Keys |
| **RV-014** | `stale_cache` | persistence | Cache Eviction on Workflow Version Updates |
| **RV-015** | `boundary` | retry | Normalize `max_attempts: 0` Sentinel |
| **RV-016** | `boundary` | scheduler | Saturating Addition on Lease Deadlines (`now_ms + lease_ms`) |
| **RV-017** | `boundary` | api (HTTP) | Enforce RunQuery Limit Boundaries |
| **RV-018** | `error_propagation` | api | Cross-Transport Conflict Status Code Mapping Parity |
| **RV-019** | `error_propagation` | api | Translate Domain Validation Errors to 4xx Bad Request |
| **RV-020** | `serialization` | events | Pinned Stable Machine Codes for `EventKind` |
| **RV-021** | `serialization` | api | Semantic Map Equality in Tag Round-Trip Assertions |
| **RV-022** | `serialization` | api (gRPC) | Validate `uint64` Millisecond Bounds in Proto Retry |
| **RV-023** | `lifecycle` | cli | Propagate gRPC Bind Failure as Fatal Server Error |
| **RV-024** | `lifecycle` | cli | Thread Cancellation Tokens on Server Shutdown |
| **RV-025** | `lifecycle` | events | Decouple Event Watcher Polling Cadence from Reap Sweeps |
| **RV-026** | `config` | config | Reject Negative Worker Count in Environment Overrides |
| **RV-027** | `config` | config | Reject Unknown Fields and Subsections in TOML Config |
| **RV-028** | `validation` | api / domain | Enforce `MAX_RUN_INPUT_BYTES` Size Ceiling on Submits |

---

## 4. Verification Invariants

All tasks satisfy the following quality invariants:
- **Independence**: Fixing one defect does not resolve or break another.
- **Reversibility**: `defect.patch` applies cleanly to `golden-baseline` with 0 hunk offsets.
- **F2P Guarantee**: Every defect has an unambiguous failing test that turns green only upon applying the solution.
- **P2P Guarantee**: Golden baseline maintains 100% test pass rate across all 294+ workspace tests with 0 Clippy warnings.
