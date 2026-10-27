#!/usr/bin/env bash
# Reproduces the Kerno M0 storage benchmarks with a single command
# (SAD §17.6). Methodology: docs/adr/0012-metodologia-de-benchmarks.md
#
# Environment overrides (defaults in brackets):
#   SCALES  [small,medium]   dataset sizes: tiny|small|medium|large|<n>
#   REPS    [3]              repetitions per combination
#   ENGINES [btree,lsm,redb,fjall,sqlite]
#   COLD    [1]              also run read workloads with the OS cache dropped (needs root)
#   OUT     [spikes/storage/results/<date>]
#   DATA    [bench-data]     scratch directory (must be on the disk you want to measure)
set -euo pipefail
cd "$(dirname "$0")/../.."

SCALES="${SCALES:-small,medium}"
REPS="${REPS:-3}"
ENGINES="${ENGINES:-btree,lsm,redb,fjall,sqlite}"
COLD="${COLD:-1}"
OUT="${OUT:-spikes/storage/results/$(date +%Y-%m-%d)}"
DATA="${DATA:-bench-data}"

cargo build --release -p spike-bench --features candidates
BIN=target/release/spike-bench

mkdir -p "$OUT"
rm -f "$OUT/results.jsonl"
"$BIN" env --dir "$DATA" > "$OUT/env.json"

# 1. Every workload, durable commits (the comparison that decides ADR-0001).
"$BIN" suite --engines "$ENGINES" --workloads all --scales "$SCALES" --reps "$REPS" \
  --durability sync --dir "$DATA" --out "$OUT/results.jsonl"

# 2. Commit-heavy workloads without fsync per commit (cost of durability).
"$BIN" suite --engines "$ENGINES" --workloads small_commits,cauce --scales small --reps "$REPS" \
  --durability buffered --dir "$DATA" --out "$OUT/results.jsonl"

# 3. Reads served from the device instead of the OS page cache.
if [ "$COLD" = 1 ]; then
  LAST_SCALE="${SCALES##*,}"
  "$BIN" suite --engines "$ENGINES" --workloads point_reads,range_scans --scales "$LAST_SCALE" \
    --reps "$REPS" --durability sync --cold true --dir "$DATA" --out "$OUT/results.jsonl"
fi

"$BIN" report --input "$OUT/results.jsonl" > "$OUT/report.md"
rm -rf "$DATA"
echo "Results: $OUT/report.md"
