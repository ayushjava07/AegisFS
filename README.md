# Runvane

[![Rust](https://img.shields.io/badge/rust-1.80%2B-blue)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Tests](https://img.shields.io/badge/tests-294%20passed-brightgreen)](#testing)
[![Commits](https://img.shields.io/badge/commits-160%2B-informational)](#history)

**Runvane** is a high-performance, durable, distributed workflow-orchestration platform engineered in Rust for resilient execution of multi-step task DAGs. It provides dual HTTP and gRPC transport APIs, robust persistence backends (in-memory and transactional SQLite with WAL mode), a multi-threaded worker pool with monotonic lease semantics, and an outbox event bus for reliable webhook delivery.

This repository serves both as an industrial-grade orchestration engine and a standardized software engineering benchmark platform with cleanly isolated, reproducible defect evaluations.

---

## Architectural Highlights

- **DAG Engine & State Machine**: Formal state transitions for workflow runs (`Queued -> Running -> Succeeded / Failed / TimedOut / Cancelled`) and tasks (`Pending -> Running -> Succeeded / Failed / Skipped`), with Kahn's topological sorting, cycle detection, and template interpolation.
- **Durable Persistence**: Pluggable storage architecture (`MemoryStore`, `SqliteStore`, and cached `LruStore`) supporting optimistic concurrency, leasing, multi-tenant compound isolation, and automatic retention reaping.
- **Fair Scheduling & Execution**: Deterministic worker thread pool with lease heartbeating, exponential backoff with decoupled jitter, panic isolation barriers, and fair-share tenant rate throttling.
- **Dual Transport Surface**:
  - **HTTP/JSON API**: Full RESTful interface powered by Axum with structured error mapping and embedded status dashboard (`GET /dashboard`).
  - **gRPC API**: High-throughput protobuf interface powered by Tonic with streamable run subscriptions and strict validation.
- **Event Outbox & Delivery**: Reliable at-least-once lifecycle event dispatch (`Watcher`, `Outbox`, `HttpSink`) supporting HMAC-SHA256 signature verification and dedup delivery IDs.
- **Content-Addressable Artifacts**: Disk-backed and memory-backed artifact store with SHA-256 content addressing, sharded filesystem layouts, and integrity validation.

---

## Quick Start

### Build and Test

```bash
# Compile in release mode
cargo build --release

# Run comprehensive test suite (294+ unit, integration, boundary, and property tests)
cargo test --workspace

# Check style and lint invariants
cargo fmt --check
cargo clippy --all-targets --workspace -- -D warnings
```

### Run Server

```bash
# Start Runvane server on default ports (HTTP: 8080, gRPC: 9090)
cargo run -- serve --host 127.0.0.1 --http-port 8080 --grpc-port 9090

# Or start with a custom SQLite database and worker pool size
cargo run -- serve --sqlite-path runvane.db --workers 8 --lease-ms 30000
```

---

## Command Line Interface

```
runvane 0.1.0
Distributed workflow orchestration control plane

USAGE:
    runvane [OPTIONS] <SUBCOMMAND>

SUBCOMMANDS:
    serve       Start HTTP server, gRPC server, and scheduler worker pool
    submit      Submit a workflow run from a JSON definition file
    status      Fetch current status and task progress of a workflow run
    cancel      Cancel a queued or running workflow run
    list        List workflows or historical runs with filtering
    dry-run     Statically simulate and analyze a workflow DAG without running it
    stats       Display real-time control plane health and execution telemetry
    completion  Generate shell autocompletion script (bash, zsh, fish)
    validate    Statically validate a workflow specification document
    migrate     Apply pending schema migrations to SQLite store
    export      Export run execution history to JSON or NDJSON
    replay      Replay an execution run against updated definitions
    help        Print help information
```

### Shell Autocompletion

Runvane can generate completion scripts for your shell:

```bash
# Bash
source <(runvane completion bash)

# Zsh
runvane completion zsh > ~/.zfunc/_runvane

# Fish
runvane completion fish > ~/.config/fish/completions/runvane.fish
```

---

## Deployment & Containerization

A multi-stage containerfile and compose setup are provided for containerized deployments:

```bash
# Build and run control plane with Docker Compose
docker compose up -d

# Check service health
curl -f http://localhost:8080/health
```

Configuration templates are available in [`config/runvane.example.toml`](config/runvane.example.toml).
Reference workflow definitions are provided under [`examples/workflows/`](examples/workflows/):
- `data_pipeline.json`: ETL diamond DAG with partition extraction and aggregation.
- `incident_response.json`: Alert triage, diagnostic gathering, and notification flow.
- `ml_training.json`: Distributed training pipeline with model evaluation and artifact publishing.

---

## Testing & Verification

Runvane includes an 8-category test suite:
- **Unit Tests**: Domain logic, state machines, parsers, and error bridges.
- **Integration Tests**: Concurrent lease races, cascade reaping, and SQLite persistence.
- **Property-Based Tests**: `proptest` suites verifying monotonic cron ticks, backoff curves, DAG cycles, and expression algebras.
- **Boundary Tests**: Payload size ceilings, JSON recursion depth, timeout boundaries, and pagination limits.
- **Concurrency Verification**: Loom model checking for lock-free claims and cache invalidations.
- **Fuzzing Harness**: Independent libFuzzer targets in `fuzz/` covering parsers, filters, and wire inputs.
- **Examples Validation**: Automated JSON schema and dependency graph validation (`tests/examples_validation.rs`).
- **Shell Completion Tests**: Syntax and parameter consistency checking (`tests/cli_completion.rs`).

Run all tests and lint checks:
```bash
cargo fmt --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace
./scripts/verify_benchmarks.sh
```

---

## Benchmark Structure

This repository includes benchmark evaluation tasks structured under `internal-bench/`:
- `internal-bench/defects.yaml`: Complete defect catalog with root causes, mechanisms, and verification commands.
- `internal-bench/tasks/RV-001` through `RV-033`: Sand-style benchmark task bundles containing:
  - `instructions.md`: Operator task description and expected behavior.
  - `task.yaml`: Metadata, category, subsystem, and target test command.
  - `defect.patch`: Cleanly reversible defect injection patch.
  - `solution.patch`: Golden baseline fix patch.
  - `test_patch.diff`: Fail-to-pass witness test patch.

To verify the integrity and schema of all 33 benchmark packages:
```bash
./scripts/verify_benchmarks.sh
```

---

## Repository Documentation

- [`COMMIT_PLAN.md`](COMMIT_PLAN.md): Development roadmap and milestone progression.
- [`COMMIT_PROGRESS.md`](COMMIT_PROGRESS.md): Batch progress tracking register.
- [`HISTORY_AUDIT.md`](HISTORY_AUDIT.md): Comprehensive repository development and history audit.
- [`ARCHITECTURE.md`](ARCHITECTURE.md): High-level system architecture and component design.
- [`CHANGELOG.md`](CHANGELOG.md): Version history and release notes.

---

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.