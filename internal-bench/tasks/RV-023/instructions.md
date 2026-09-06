# Task RV-023: Propagate gRPC Bind Failure in Server Lifecycle

## Subsystem
`cli` / `lifecycle` (Server Startup & Lifecycle)

## Summary
When starting `runvane serve`, both HTTP and gRPC listeners are initialized. If the gRPC listener fails to bind (e.g. port already bound by another process), the failure occurs inside a detached spawned task while the HTTP server and background scheduler remain running. This leaves the server in a broken half-alive state where clients connecting over gRPC fail while the process appears alive.

## Expected Behavior
1. gRPC server startup and bind errors must not be swallowed; any bind or fatal runtime failure must terminate the serve process.
2. In-flight failure of either primary transport must trigger server shutdown with an appropriate `RunvaneError::Server` error.

## Files Affected
- `src/cli/serve.rs`

## Verification
Run:
```bash
cargo test --lib cli::serve::tests
```
Assert that gRPC bind conflicts promptly return an error rather than hanging.
