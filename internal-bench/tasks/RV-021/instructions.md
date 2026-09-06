# Task RV-021: Semantic Map Equality for Cross-Transport Tags

## Subsystem
`api` / `serialization` (Transport Interoperability & Property Tests)

## Summary
Workflow definitions and runs contain user-provided `tags` key-value pairs. In HTTP endpoints, tags serialize into JSON with deterministic ordering (via `BTreeMap`), whereas proto map representations and deserialized internal representations use hash structures or variable key ordering. Tests that assert byte-for-byte equality across transports fail intermittently despite semantic equivalence.

## Expected Behavior
1. Cross-transport round-trip assertions must compare tags semantically (key-by-key `BTreeMap` equivalence) rather than raw serialized byte-for-byte string equality.
2. Deserializing tags from JSON bytes into `BTreeMap<String, String>` preserves all keys and values regardless of wire key order.

## Files Affected
- `src/api/grpc.rs`

## Verification
Run:
```bash
cargo test --lib api::grpc::tests
```
Assert that semantic tag comparisons pass reliably across differing serialization key orders.
