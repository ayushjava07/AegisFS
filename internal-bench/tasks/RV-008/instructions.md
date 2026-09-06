# Task RV-008: Worker Pool Panic Boundary & Orphaned Lease Recovery

## Subsystem
`scheduler` / `plugins` (Panic Resilience & Fault Tolerance)

## Summary
When an external task handler plugin or user script encounters an unexpected runtime panic (e.g. division by zero, explicit panic, unwrapped None), the panic unwinds through `RunExecutor::attempt_with_token` into the worker OS thread. Without an explicit unwind boundary, the thread terminates immediately. As a result:
1. The worker thread is lost permanently, shrinking the pool capacity.
2. The claim lease for the run is never released or acknowledged, keeping the run orphaned until maximum lease timeout.
3. Subsequent jobs dispatched to the pool can starve.

## Expected Behavior
1. In `WorkerPool::spawn`, wrap the job execution attempt with `std::panic::catch_unwind(std::panic::AssertUnwindSafe(...))`.
2. When a panic is caught, release the claim lease with `store.release(&job.run_id, &job.token, clock.now_ms())`.
3. Keep the worker thread alive in its loop so it can continue processing subsequent queued jobs.

## Files Affected
- `src/scheduler/pool.rs`

## Verification
Run:
```bash
cargo test --lib scheduler::tests::worker_pool_panic_boundary_recovers_and_serves_subsequent_jobs
```
Assert that panicking handlers do not terminate the worker and subsequent jobs complete successfully.
