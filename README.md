# Runvane

[![CI](https://github.com/aegisfs/aegisfs/actions/workflows/ci.yml/badge.svg)](https://github.com/aegisfs/aegisfs/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.96%2B-blue)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](https://github.com/aegisfs/aegisfs#license)

Runvane is a durable, self-hosted **distributed workflow-orchestration
platform** for internal platform teams. Declare versioned workflow definitions,
submit runs over HTTP or gRPC, and watch execution unfold through the
scheduler, the webhook/event dispatch layer, the `/metrics` and `/debug`
endpoints, and a minimal read-only status dashboard.

This repository also carries the name *aegisfs* for historical reasons — the
directory predates the product and the crate/binary are `runvane`.

## Status

Early development. Phase 0 (design + scaffold) is done; this README is a
skeleton and is expanded as the platform comes together. See `DESIGN.md` for
the architecture and `PLAN.md` for the phase-by-phase build plan.

## Features (target)

- Versioned **workflow definitions** and task specs with a verified
  state machine for runs and task runs.
- Durable run state on a pluggable `Store` (in-memory for tests, embedded
  SQL for single-node production) with schema migrations.
- **Scheduler + worker pool** with exponential backoff, decoupled jitter,
  per-task timeouts, and graceful draining.
- Pluggable **task handlers** with two first-party plugins.
- **HTTP (JSON) and gRPC (Protobuf)** API surfaces.
- Operator **CLI**, **configuration layer** (flags > env > file > defaults),
  **auth/RBAC**, **metrics**, **webhooks**, **maintenance workers**, and a
  server-rendered **status dashboard**.

> This is a synthetic benchmark repository. Its git history and contents are
> intentionally constructed to exercise realistic software-engineering
> defects for evaluation purposes.

## Prerequisites

- Rust 1.96.x (`rust-toolchain.toml` pins the channel)
- `protoc` only when regenerating the checked-in gRPC codegen (not needed to
  build; regeneration is documented in `CONTRIBUTING.md`)

## Quick start

```sh
cargo build
cargo test
cargo run -- --help
```

See `DESIGN.md` for the module layout and `PLAN.md` for the build plan.