#!/bin/bash -eu

cd "$SRC"

# Build all fuzz targets in release mode with debug assertions
cargo fuzz build -O --debug-assertions

# Copy fuzz binaries to $OUT
cp fuzz/target/x86_64-unknown-linux-gnu/release/chunking_fuzzer      "$OUT/"
cp fuzz/target/x86_64-unknown-linux-gnu/release/compression_fuzzer   "$OUT/"
cp fuzz/target/x86_64-unknown-linux-gnu/release/dedup_fuzzer         "$OUT/"
cp fuzz/target/x86_64-unknown-linux-gnu/release/serialization_fuzzer "$OUT/"
