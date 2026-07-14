# Contributing to Runvane

Thank you for considering contributing to Runvane. Please follow these
guidelines.

## Getting Started

1. Ensure you have Rust 1.96.x installed (`rust-toolchain.toml` pins the
   channel; `rustup` picks it up automatically).
2. Clone the repository.
3. Install optional dev tooling used by the quality gates:

   ```sh
   cargo install cargo-fuzz --locked    # fuzzing (see below)
   cargo install cargo-mutants --locked # mutation testing
   cargo install cargo-llvm-cov         # coverage
   ```

   `protoc` is **only** needed when regenerating the checked-in gRPC source
   (see "Regenerating gRPC codegen" below); it is not needed to build.
4. Run `scripts/ci.sh` to run every CI gate locally.

## Code Style

- Run `cargo fmt` before committing.
- `cargo clippy --all-targets --workspace -- -D warnings` must be clean.
- Prefer `thiserror` for library error types; reserve `anyhow` for the binary
  and tests.
- Follow the existing module structure and naming in `DESIGN.md`. Each
  subsystem owns its module; keep `Store`/`Clock`/handler traits in the
  low-level modules so higher layers stay testable in isolation.

## Testing

- All new behavior must have tests.
- Unit tests live next to the code; integration tests in `tests/`.
- Uses of time must flow through `clock::Clock` — tests use a `ManualClock`,
  never wall-clock sleeps. The suite must stay deterministic.
- Property-based tests use `proptest` and live in `tests/property_tests.rs`
  and per-module `#[cfg(test)]` modules.
- Concurrency-sensitive structures that can be model-checked use `loom`
  behind the `loom` feature (see `queue::loom_model`).

## Fuzzing

Fuzz targets live in `fuzz/` (a standalone crate so it never affects normal
builds). To run a target for ~30 seconds:

```sh
cd fuzz && cargo fuzz run <target> -- -max_total_time=30
```

See `fuzz/README.md` in that directory for the full list of targets.

## Regenerating gRPC codegen

The generated tonic/prost sources under `src/rpc/gen/` are committed so the
build is hermetic (no `protoc` requirement). To regenerate after editing
`proto/*.proto`:

```sh
protoc --version >/dev/null   # protoc must be on PATH
cargo run -p runvane --example genproto
git diff --stat               # review the regenerated files
```

## Mutation testing

Scoped mutation runs are part of the quality bar for the state-machine,
retry/backoff, and validation packages. Run per package:

```sh
cargo mutants --file src/state/ ...
```

## Pull Request Process

1. Create a feature branch from `main`.
2. Make your changes with clear, incremental commits.
3. Run the full local suite (`scripts/ci.sh`).
4. Ensure CI passes.
5. Submit a PR with a clear description.

## License

Contributions are licensed under the MIT OR Apache-2.0 license.