#!/usr/bin/env bash
# Long fuzzing campaign of the kerno-storage readers (row OPS-13; docs/testing/fuzz-long-run.md).
#
# Runs the cargo-fuzz targets of crates/kerno-storage/fuzz one at a time, with the command of
# its README (-max_len=65536 -timeout=1), logs `uptime` every LOAD_INTERVAL seconds and writes
# the execution count of each target. A crashing target is recorded and the run goes on; the
# script exits non-zero at the end if any target failed.
#
# Usage: scripts/fuzz-long-run.sh [TARGET...]    (no TARGET: every target of `cargo fuzz list`)
#
# Environment overrides (defaults in brackets):
#   LONG_TARGETS   [wal manifest open]   targets that run LONG_SECS; every other one runs SHORT_SECS
#   LONG_SECS      [28800]   8 h
#   SHORT_SECS     [1800]    30 min
#   LOAD_INTERVAL  [600]     seconds between two `uptime` lines in load.log
#   TOOLCHAIN      [nightly] runs `cargo +$TOOLCHAIN fuzz`; empty runs plain `cargo fuzz`
#   OUT            [fuzz-long-run/<date>]   load.log, counts.tsv and one <target>.log per target
#   WORK           [${TMPDIR:-/tmp}/kerno-fuzz-<date>]   working corpora; the committed corpus is
#                  only read
#
# Default campaign: 3 x 8 h + 12 x 30 min, about 30 h. Run it on an idle machine, or in Docker on
# dedicated cores with docker/fuzz/run.sh, which calls this script once per core with its own
# targets, OUT and WORK.
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit "${1:-2}"
}
[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage 0

command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }

stamp="$(date +%Y-%m-%d-%H%M%S)"
LONG_TARGETS="${LONG_TARGETS:-wal manifest open}"
LONG_SECS="${LONG_SECS:-28800}"
SHORT_SECS="${SHORT_SECS:-1800}"
LOAD_INTERVAL="${LOAD_INTERVAL:-600}"
TOOLCHAIN="${TOOLCHAIN-nightly}"
OUT="${OUT:-$PWD/fuzz-long-run/$stamp}"
WORK="${WORK:-${TMPDIR:-/tmp}/kerno-fuzz-$stamp}"
for n in "$LONG_SECS" "$SHORT_SECS" "$LOAD_INTERVAL"; do
  [[ "$n" =~ ^[1-9][0-9]*$ ]] || { echo "not a positive number of seconds: $n" >&2; exit 2; }
done

mkdir -p -- "$OUT" "$WORK"
OUT="$(cd -- "$OUT" && pwd)"
WORK="$(cd -- "$WORK" && pwd)"
cd -- "$(dirname -- "$0")/../crates/kerno-storage"

fuzz=(cargo)
[[ -n "$TOOLCHAIN" ]] && fuzz+=("+$TOOLCHAIN")
fuzz+=(fuzz)

if [[ $# -gt 0 ]]; then
  targets=("$@")
else
  mapfile -t targets < <("${fuzz[@]}" list)
fi

log_load() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(uptime)" >>"$OUT/load.log"; }
# The first load line shows the machine as the run found it, before the build loads it.
log_load
# Build every target first, so a build failure stops the run before the machine time starts.
"${fuzz[@]}" build >"$OUT/build.log" 2>&1 || {
  echo "cargo fuzz build failed; see $OUT/build.log" >&2
  exit 1
}

(
  while :; do
    log_load
    sleep "$LOAD_INTERVAL"
  done
) &
load_pid=$!
trap 'kill "$load_pid" 2>/dev/null; log_load' EXIT

printf 'target\tseconds\texecutions\tcov\tft\texit\n' >"$OUT/counts.tsv"
failed=()
for t in "${targets[@]}"; do
  secs="$SHORT_SECS"
  [[ " $LONG_TARGETS " == *" $t "* ]] && secs="$LONG_SECS"
  mkdir -p -- "$WORK/$t"
  echo "$(date -u +%H:%M:%SZ) $t for ${secs}s"
  status=0
  "${fuzz[@]}" run "$t" "$WORK/$t" "fuzz/corpus/$t" -- \
    -max_total_time="$secs" -max_len=65536 -timeout=1 -print_final_stats=1 \
    >"$OUT/$t.log" 2>&1 || status=$?
  # The count comes from the final stats; cov and ft from the last libFuzzer status line.
  # `|| true`: a target that dies before its first status line has none, and prints `?`.
  execs="$(sed -n 's/^stat::number_of_executed_units: *//p' "$OUT/$t.log" | tail -n 1)"
  cov="$(grep -o 'cov: [0-9]*' "$OUT/$t.log" | tail -n 1 | cut -d' ' -f2 || true)"
  ft="$(grep -o 'ft: [0-9]*' "$OUT/$t.log" | tail -n 1 | cut -d' ' -f2 || true)"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$t" "$secs" "${execs:-?}" "${cov:-?}" "${ft:-?}" "$status" \
    >>"$OUT/counts.tsv"
  [[ "$status" -eq 0 ]] || failed+=("$t")
done

column -t -s $'\t' "$OUT/counts.tsv" 2>/dev/null || cat "$OUT/counts.tsv"
echo "results in $OUT"
if [[ ${#failed[@]} -gt 0 ]]; then
  echo "failed: ${failed[*]}; inputs under crates/kerno-storage/fuzz/artifacts/<target>/" >&2
  exit 1
fi
