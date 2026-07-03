# AegisFS

[![CI](https://github.com/aegisfs/aegisfs/actions/workflows/ci.yml/badge.svg)](https://github.com/aegisfs/aegisfs/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.82%2B-blue)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](https://github.com/aegisfs/aegisfs#license)

A high-performance, deduplicating, encrypted virtual filesystem and archive storage engine built in Rust. Designed for robust data backups, versioned snapshots, and compressed cold-storage archives.

## Features

- **Content-Defined Deduplication** — Rabin fingerprinting + CDC to maximize storage efficiency.
- **Pluggable Compression & Encryption** — AES-256-GCM, ChaCha20-Poly1305 encryption; Zstd, LZ4 compression.
- **Hierarchical VFS** — Directory tree with parent-child metadata, permissions, path resolution.
- **Snapshot Management** — Versioned, read-only incremental snapshots with differential comparison.
- **Integrity Verification** — Full-scan chunk verification, manifest and tree consistency checks.
- **Async I/O** — Tokio-based async read/write streams, buffered and piped I/O.
- **Recovery & WAL** — Write-ahead journal for crash recovery and state reconstruction.
- **Background Tasks** — Scheduler, garbage collection, replication, throttled operations.
- **Metrics & Telemetry** — Prometheus metrics, structured tracing, event bus.
- **5 fuzz targets, 5 Criterion benchmarks, 556+ tests** — CI with clippy + fmt.

## Architecture

```
                   ┌──────────────────────┐
                   │   Archive Manager     │
                   └───┬────┬──────┬───────┘
                       │    │      │
              ┌────────┘    │      └────────┐
              ▼             ▼                ▼
     ┌──────────────┐ ┌──────────┐ ┌────────────────┐
     │ Virtual FS   │ │ Snapshot │ │ Integrity      │
     │ (filesystem/)│ │ Manager  │ │ Verifier       │
     └──────┬───────┘ │(snapshot/)│ │(verification/) │
            │         └──────────┘ └────────────────┘
            ▼
     ┌──────────────┐ ┌──────────┐
     │ Dedup Engine │ │ Metadata │
     │ (dedup/      │ │ Index    │
     │  chunking/)  │ │(metadata/)│
     └──────┬───────┘ └──────────┘
            ▼
     ┌──────────────────────────────────┐
     │  Decorated Chunk Storage         │
     │  (Encryption → Compression →     │
     │   Disk/Memory Backend)           │
     └──────────────────────────────────┘
```

### Component Layers

| Layer | Crate Module | Responsibility |
|---|---|---|
| **Public API** | `api/` | `AegisFs` facade, builder pattern |
| **Archive** | `archive/` | Archive lifecycle, manifest management |
| **Filesystem** | `filesystem/` | File/dir CRUD, path resolution, iterators |
| **Snapshot** | `snapshot/` | Incremental snapshots, differentials |
| **Deduplication** | `dedup/`, `chunking/` | CDC + fixed chunking, hash index |
| **Metadata** | `metadata/` | In-memory tree index, query, search |
| **Compression** | `compression/` | Zstd, LZ4, no-op providers |
| **Encryption** | `crypto/` | AES-256-GCM, ChaCha20-Poly1305, key derivation |
| **Verification** | `verification/` | Full-scan integrity, manifest/tree verification |
| **Serialization** | `serialization/` | Binary (bincode), JSON, format detection |
| **Network** | `network/` | TCP transport, connection pool, framing |
| **RPC** | `rpc/` | Request/response protocol, handler registry |
| **Auth** | `auth/` | Token-based authentication, role access control |
| **Events** | `events/` | Async event bus, pub/sub |
| **Scheduler** | `scheduler/` | Background task orchestration, GC |
| **Journal** | `journal/` | WAL write-ahead logging, replay |
| **Recovery** | `recovery/` | Crash recovery, state reconstruction |
| **Replication** | `replication/` | Cross-endpoint archive replication |
| **Config** | `config/` | Builder-based configuration, sub-configs |
| **Allocator** | `allocator/` | Memory pool, slab allocator |
| **Cache** | `cache/` | LRU, two-tier (async+sync) caching |
| **Throttle** | `throttle/` | Token-bucket rate limiting |
| **Policy** | `policy/` | Retention, GC eligibility policies |
| **Metrics** | `metrics/` | Prometheus counters, histograms |
| **Telemetry** | `telemetry/` | OpenTelemetry-style tracing spans |
| **Watch** | `watch/` | Filesystem watcher (poll-based) |
| **Plugin** | `plugin/` | Dynamic plugin registry, lifecycle |
| **Sync** | `sync/` | Sync engine, conflict detection |
| **Async I/O** | `async_io/` | Duplex streams, buffered reader/writer |
| **Health** | `health/` | Health check endpoints |
| **GC** | `gc/` | Garbage collection sweeps |
| **Lease** | `lease/` | Distributed lease management |
| **Limits** | `limits/` | Resource quotas |
| **Scope** | `scope/` | Scoped operations |
| **Trace** | `trace/` | Distributed trace propagation |
| **Progress** | `progress/` | Operation progress reporting |
| **Diagnostics** | `diagnostics/` | System diagnostics |
| **Migration** | `migration/` | Data format migration |
| **Version** | `version/` | Version info |
| **Checksum** | `checksum/` | SHA-256, BLAKE3, xxHash3, Combined hashers |
| **Core** | `core/` | Types, traits, error types, IDs |

## Build

```bash
# Default features (zstd + AES)
cargo build --release

# All features (LZ4 + ChaCha20)
cargo build --release --all-features
```

### Prerequisites
- Rust 1.82+ (see `rust-toolchain.toml`)
- Clang/LLVM (optional, for fuzzing)

## Test

```bash
# Unit + integration + property tests
cargo test --all-features

# Clippy (zero warnings)
cargo clippy --all-features -- -D warnings

# Formatting check
cargo fmt --check
```

**556 tests** — 537 unit, 1 integration, 18 proptest.

## Benchmarks

5 Criterion benchmarks under `benches/`:

| Benchmark | What it measures |
|---|---|
| `chunking` | Fixed-size & CDC throughput |
| `dedup` | Index insert/lookup performance |
| `checksum` | SHA-256, BLAKE3, xxHash3, Combined hasher throughput |
| `serialization` | Binary & JSON serialize/deserialize |
| `cache` | LRU insert, lookup, eviction |

```bash
cargo bench
```

## Fuzzing

5 `cargo-fuzz` targets under `fuzz/`:

| Target | Description |
|---|---|
| `fuzz_chunking` | CDC + fixed chunking with randomized bounds |
| `fuzz_checksum` | Checksum computation invariants |
| `fuzz_serialization` | Deserialization robustness |
| `fuzz_dedup` | Ingest + duplicate detection |
| `fuzz_compression` | Compress/decompress roundtrips |

```bash
cargo fuzz run fuzz_chunking
```

## License

Dual-licensed under **MIT** or **Apache-2.0** (your choice).
