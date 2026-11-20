#!/usr/bin/env bash
# Runs one Phase C measurement of docs/testing/phase-c-performance.md and keeps its raw output.
#
# Usage: scripts/phase-c-measure.sh STEP RUN
#
#   STEP  u6-disk    U6 with --medians (rows OPS-08, §3.8), both budgets, on DISK_DIR
#         u6-tmpfs   the same on TMPFS_DIR, as a reference only
#         u2-disk    U2 on DISK_DIR (row OPS-06)
#         r1         R1 crash-recovery time on DISK_DIR (row OPS-10)
#         sqlite     the Kerno vs SQLite comparison with SCALES=small REPS=5 (row OPS-10)
#   RUN   the number of the repetition; it names the output file
#
# Each run writes OUT/STEP-runRUN.txt, which starts and ends with /proc/loadavg, so a reader can
# tell how busy the machine was. A run is never repeated to get a better one; keep every file.
#
# Environment overrides (defaults in brackets):
#   OUT        [docs/testing/raw/<date>-phase-c]   where the raw files go
#   DISK_DIR   [target/bench-scratch]              scratch directory on the disk to measure
#   TMPFS_DIR  [/tmp]                              scratch directory on tmpfs (reference)
#
# Run it on a quiet machine. The sqlite step takes its output directory from OUT too.
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit "${1:-2}"
}
[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage 0
[[ $# -eq 2 ]] || usage 2

step="$1"
run="$2"
[[ "$run" =~ ^[0-9]+$ ]] || usage 2
case "$step" in u6-disk | u6-tmpfs | u2-disk | r1 | sqlite) ;; *) usage 2 ;; esac

cd "$(dirname "$0")/.."
command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }

OUT="${OUT:-docs/testing/raw/$(date +%F)-phase-c}"
DISK_DIR="${DISK_DIR:-target/bench-scratch}"
TMPFS_DIR="${TMPFS_DIR:-/tmp}"
mkdir -p -- "$OUT" "$DISK_DIR"
file="$OUT/$step-run$run.txt"

bench() {
  cargo run --release -p kerno-units --features test-support --example unit_bench -- "$@"
}

{
  cat /proc/loadavg
  case "$step" in
    u6-disk) bench --only U6 --medians --dir "$DISK_DIR" ;;
    u6-tmpfs) bench --only U6 --medians --dir "$TMPFS_DIR" ;;
    u2-disk) bench --only U2 --dir "$DISK_DIR" ;;
    r1) bench --only R1 --dir "$DISK_DIR" ;;
    sqlite) SCALES=small REPS=5 OUT="$OUT/sqlite" DATA="$DISK_DIR/sqlite" spikes/storage/run-sqlite-comparison.sh ;;
  esac
  cat /proc/loadavg
} 2>&1 | tee -- "$file"
