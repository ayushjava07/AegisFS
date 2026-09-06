# Task RV-024: Thread Cancellation Tokens on Shutdown

## Subsystem
`cli` / `scheduler` (Graceful Shutdown & Task Cancellation)

## Summary
When `runvane serve` receives SIGINT or SIGTERM, it initiates shutdown by setting `stop.store(true, Ordering::SeqCst)`. However, workers executing in-flight task handlers (such as long sleeps or network calls) do not receive a cancellation signal. Consequently, the server process remains alive waiting for background handlers to complete naturally instead of aborting promptly.

## Expected Behavior
1. The shutdown trigger must notify the worker pool and active worker execution contexts.
2. Handlers respecting cancellation tokens or worker thread join operations must unblock and terminate within the graceful shutdown window.

## Files Affected
- `src/cli/serve.rs`
- `src/scheduler/pool.rs`

## Verification
Run:
```bash
cargo test --lib cli::serve::tests
```
Assert that shutdown signals propagate to active execution pools.
