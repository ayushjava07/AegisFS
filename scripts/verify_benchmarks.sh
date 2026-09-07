#!/usr/bin/env bash
# Verifies benchmark tasks packaging integrity and schema conformance.
set -euo pipefail

BENCH_DIR="internal-bench/tasks"

if [ ! -d "$BENCH_DIR" ]; then
    echo "Error: Benchmark directory $BENCH_DIR not found." >&2
    exit 1
fi

echo "==> Auditing benchmark task packages in $BENCH_DIR"

TASK_COUNT=0
VALID_COUNT=0

for task_dir in "$BENCH_DIR"/RV-*; do
    if [ ! -d "$task_dir" ]; then
        continue
    fi

    TASK_COUNT=$((TASK_COUNT + 1))
    task_id="$(basename "$task_dir")"

    # Check required files
    for req in task.yaml instructions.md defect.patch solution.patch test_patch.diff; do
        if [ ! -f "$task_dir/$req" ]; then
            echo "  [FAIL] $task_id missing required file: $req" >&2
            exit 1
        fi
        if [ ! -s "$task_dir/$req" ]; then
            echo "  [FAIL] $task_id has empty file: $req" >&2
            exit 1
        fi
    done

    # Check task.yaml has expected id and subsystem
    if ! grep -q "^id: $task_id" "$task_dir/task.yaml"; then
        echo "  [FAIL] $task_id: task.yaml id mismatch" >&2
        exit 1
    fi

    if ! grep -q "^subsystem:" "$task_dir/task.yaml"; then
        echo "  [FAIL] $task_id: task.yaml missing subsystem" >&2
        exit 1
    fi

    VALID_COUNT=$((VALID_COUNT + 1))
done

echo "==> Verified $VALID_COUNT / $TASK_COUNT benchmark tasks successfully."
echo "==> All benchmark task packages conform to specification."
