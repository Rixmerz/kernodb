#!/usr/bin/env bash
# Reproduces the Kerno vs SQLite comparison of docs/testing/sqlite-benchmark.md with one command.
#
# Both engines run the same workloads through the same harness (spike-bench), in fresh processes
# and fresh directories, interleaved repetition by repetition. Durability is paired:
#
#   sync      kerno `Durability::Sync`      SQLite journal_mode=WAL, synchronous=FULL
#   buffered  kerno `Durability::Buffered`  SQLite journal_mode=WAL, synchronous=NORMAL
#
# Only the `sync` pair has the same guarantee (a commit that returned survives a power cut). The
# `buffered` pair measures the cost of execution without a per-commit fsync, not equal guarantees.
#
# Environment overrides (defaults in brackets):
#   SCALES     [small]   dataset size: tiny (5k records) | small (20k) | medium (200k) | <n>
#   REPS       [5]       repetitions per engine and workload
#   WORKLOADS  [bulk_load,point_reads,range_scans,small_commits,point_deletes,append_seq,mixed,
#               history_sim,livespec,cauce,agent_memory,concurrent_commits]
#   OUT        [spikes/storage/results/<date>-sqlite]   raw JSONL, env.json and tables.md
#   DATA       [bench-data]   scratch directory; it must be on the disk you want to measure
#   ALLOW_TMPFS [0]   a RAM-backed DATA makes fsync free, so the sync pair compares nothing: refused
#
# Run it on a quiet machine and look at `loadavg` in $OUT/env.json afterwards.
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit "${1:-2}"
}
[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage 0
[[ $# -eq 0 ]] || usage 2

cd "$(dirname "$0")/../.."
command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }

SCALES="${SCALES:-small}"
REPS="${REPS:-5}"
WORKLOADS="${WORKLOADS:-bulk_load,point_reads,range_scans,small_commits,point_deletes,append_seq,mixed,history_sim,livespec,cauce,agent_memory,concurrent_commits}"
OUT="${OUT:-spikes/storage/results/$(date +%Y-%m-%d)-sqlite}"
DATA="${DATA:-bench-data}"
ALLOW_TMPFS="${ALLOW_TMPFS:-0}"

mkdir -p -- "$DATA" "$OUT"
fs="$(stat -f -c %T -- "$DATA")"
if [[ "$fs" == "tmpfs" || "$fs" == "ramfs" ]] && [[ "$ALLOW_TMPFS" != 1 ]]; then
  echo "$DATA is on $fs: fsync is free there, so the sync pair would compare nothing." >&2
  echo "Point DATA at a real disk (or set ALLOW_TMPFS=1 to measure anyway)." >&2
  exit 1
fi

# Scratch space for the harness, removed on exit; nothing else under $DATA is touched.
scratch="$(mktemp -d -- "$DATA/run.XXXXXX")"
trap 'rm -rf -- "${scratch:?}"' EXIT

cargo build --release -p spike-bench --features candidates,kerno
bin=target/release/spike-bench

rm -f -- "$OUT/results-sync.jsonl" "$OUT/results-buffered.jsonl"
"$bin" env --dir "$scratch" > "$OUT/env.json"

for durability in sync buffered; do
  "$bin" suite --engines kerno,sqlite --workloads "$WORKLOADS" --scales "$SCALES" --reps "$REPS" \
    --durability "$durability" --dir "$scratch" --out "$OUT/results-$durability.jsonl" \
    2> >(tee -- "$OUT/progress-$durability.log" >&2)
done

python3 -I spikes/storage/aggregate-sqlite.py "$OUT" > "$OUT/tables.md"
echo "Tables: $OUT/tables.md"
