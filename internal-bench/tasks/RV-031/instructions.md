# Task RV-031: Bound Connection Resources in HttpSink

## Subsystem
`events` (Webhook Delivery & Network Resource Management)

## Summary
When dispatching run status notifications to webhook subscribers, `HttpSink` initiates an unpooled raw `TcpStream::connect` per delivery attempt. Under high webhook volume or slow receiver latency, unpooled socket creation causes thousands of sockets to accumulate in `TIME_WAIT` state, eventually exhausting operating system file descriptors (`EMFILE`) and dropping critical events.

## Expected Behavior
1. `HttpSink` must manage network resources with reuse, backpressure, or connection bounds.
2. Webhook delivery loops must maintain bounded descriptor consumption across sustained deliveries.

## Files Affected
- `src/events/dispatch.rs`

## Verification
Run:
```bash
cargo test --lib events::dispatch::tests
```
Assert that `HttpSink` executes deliveries without leaking socket descriptors.
