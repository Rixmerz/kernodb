#!/usr/bin/env bash
# Runs the long fuzzing campaign (OPS-13) in Docker, one fuzz process per dedicated CPU core.
# See docs/testing/fuzz-long-run.md, section "Running in Docker".
#
# Usage: docker/fuzz/run.sh [TARGET...]    (no TARGET: every target of `cargo fuzz list`)
#
# Environment overrides (defaults in brackets):
#   CPUS           [13-15]   host cores for the container, e.g. 13-15 or 2,4,6-7. One fuzz
#                            process per core; --cpus is set to the core count.
#   RESULTS        [./fuzz-results/<date>]   host directory for corpus, crashes and logs
#   IMAGE          [kerno-fuzz]   image tag; built from the repository root when missing
#   MONITOR_SECS   [600]     seconds between two lines of monitor.log
#   LONG_TARGETS, LONG_SECS, SHORT_SECS   as scripts/fuzz-long-run.sh (defaults: wal manifest
#                            open for 28800 s, every other target 1800 s)
#
# Results: monitor.log (host load, per-core usage, temperature, exec counts), and per core
# core-<N>/ with load.log, counts.tsv and <target>.log; corpus/<target>/ (working corpus) and
# artifacts/<target>/ (crashing inputs) hold one directory per target.
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit "${1:-2}"
}
[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage 0

command -v docker >/dev/null || { echo "docker is required" >&2; exit 1; }

repo="$(cd -- "$(dirname -- "$0")/../.." && pwd)"
CPUS="${CPUS:-13-15}"
IMAGE="${IMAGE:-kerno-fuzz}"
MONITOR_SECS="${MONITOR_SECS:-600}"
LONG_TARGETS="${LONG_TARGETS:-wal manifest open}"
LONG_SECS="${LONG_SECS:-28800}"
SHORT_SECS="${SHORT_SECS:-1800}"
RESULTS="${RESULTS:-$PWD/fuzz-results/$(date +%Y-%m-%d)}"
for n in "$MONITOR_SECS" "$LONG_SECS" "$SHORT_SECS"; do
  [[ "$n" =~ ^[1-9][0-9]*$ ]] || { echo "not a positive number of seconds: $n" >&2; exit 2; }
done
[[ "$CPUS" =~ ^[0-9]+([-,][0-9]+)*$ ]] || { echo "CPUS is not a core list: $CPUS" >&2; exit 2; }

# Expand "13-15" or "2,4,6-7" into one core per array element.
cores=()
IFS=',' read -r -a parts <<<"$CPUS"
for p in "${parts[@]}"; do
  if [[ "$p" == *-* ]]; then
    lo="${p%-*}" hi="${p#*-}"
    (( lo <= hi )) || { echo "bad core range: $p" >&2; exit 2; }
    for ((c = lo; c <= hi; c++)); do cores+=("$c"); done
  else
    cores+=("$p")
  fi
done
(( ${#cores[@]} == $(printf '%s\n' "${cores[@]}" | sort -un | wc -l) )) \
  || { echo "CPUS lists a core twice: $CPUS" >&2; exit 2; }
for c in "${cores[@]}"; do
  (( c < $(nproc --all) )) || { echo "core $c does not exist (nproc --all = $(nproc --all))" >&2; exit 2; }
done

docker image inspect "$IMAGE" >/dev/null 2>&1 \
  || docker build -f "$repo/docker/fuzz/Dockerfile" -t "$IMAGE" "$repo"

if [[ $# -gt 0 ]]; then
  targets=("$@")
else
  mapfile -t targets < <(docker run --rm "$IMAGE" cargo fuzz list)
fi

# Give each target to the core with the least work so far, long targets first.
# ponytail: greedy by planned seconds; a core that finishes early does not take another's targets.
order=()
for t in "${targets[@]}"; do [[ " $LONG_TARGETS " == *" $t "* ]] && order+=("$t"); done
for t in "${targets[@]}"; do [[ " $LONG_TARGETS " == *" $t "* ]] || order+=("$t"); done
load=() assigned=()
for i in "${!cores[@]}"; do load[i]=0 assigned[i]=""; done
for t in "${order[@]}"; do
  best=0
  for i in "${!cores[@]}"; do (( load[i] < load[best] )) && best="$i"; done
  secs="$SHORT_SECS"
  [[ " $LONG_TARGETS " == *" $t "* ]] && secs="$LONG_SECS"
  load[best]=$(( load[best] + secs ))
  assigned[best]+="$t "
done

mkdir -p -- "$RESULTS/corpus" "$RESULTS/artifacts"
RESULTS="$(cd -- "$RESULTS" && pwd)"
name="kerno-fuzz-$$"
for i in "${!cores[@]}"; do echo "core ${cores[i]}: ${assigned[i]:-(nothing)}(${load[i]} s planned)"; done | tee "$RESULTS/plan.txt"

# The container only sleeps; the fuzz processes are `docker exec`s, each pinned with taskset
# to its own core of the cpuset. The host user owns the results; HOME is a scratch directory.
docker run -d --rm --init --name "$name" \
  --cpuset-cpus="$CPUS" --cpus="${#cores[@]}" \
  --user "$(id -u):$(id -g)" \
  -v "$RESULTS:/results" \
  -v "$RESULTS/artifacts:/work/crates/kerno-storage/fuzz/artifacts" \
  --entrypoint sleep "$IMAGE" infinity >/dev/null

monitor_pid=""
cleanup() {
  [[ -n "$monitor_pid" ]] && kill "$monitor_pid" 2>/dev/null
  docker rm -f "$name" >/dev/null 2>&1 || true # already gone after a clean exit
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# --- monitor: runs on the host, because /proc/stat and /proc/loadavg there are the host's ---
cpu_line() { grep "^cpu$1 " /proc/stat; }
monitor() {
  local mlog="$RESULTS/monitor.log" c a b t temp hot z ta tb ia ib line
  while :; do
    declare -A before=()
    for c in "${cores[@]}"; do before[$c]="$(cpu_line "$c")"; done
    sleep 5
    line="$(date -u +%Y-%m-%dT%H:%M:%SZ) load=$(cut -d' ' -f1-3 /proc/loadavg)"
    for c in "${cores[@]}"; do
      a="${before[$c]}" b="$(cpu_line "$c")"
      # fields: cpuN user nice system idle iowait irq softirq steal; idle = idle + iowait
      read -r ta ia <<<"$(awk '{s=0; for(i=2;i<=9;i++) s+=$i; print s, $5+$6}' <<<"$a")"
      read -r tb ib <<<"$(awk '{s=0; for(i=2;i<=9;i++) s+=$i; print s, $5+$6}' <<<"$b")"
      if (( tb > ta )); then
        line+=" cpu$c=$(( 100 * ((tb - ta) - (ib - ia)) / (tb - ta) ))%"
      else
        line+=" cpu$c=?"
      fi
    done
    hot=""
    for z in /sys/class/thermal/thermal_zone*/temp; do
      temp="$(cat "$z" 2>/dev/null)" || continue
      [[ "$temp" =~ ^[0-9]+$ ]] && (( temp > ${hot:-0} )) && hot="$temp"
    done
    if [[ -n "$hot" ]]; then line+=" temp_max=$(( hot / 1000 ))C"; else line+=" temp_max=n/a"; fi
    # The libFuzzer status lines start with "#<executions>".
    for f in "$RESULTS"/core-*/*.log; do
      [[ -e "$f" && "$f" != */load.log && "$f" != */build.log ]] || continue
      # A finished target has libFuzzer's final stats; a running one only its status lines.
      t="$(sed -n 's/^stat::number_of_executed_units: *//p' "$f" | tail -n 1)"
      [[ -n "$t" ]] || t="$(tail -n 200 -- "$f" | grep -o '^#[0-9]*' | tail -n 1 | tr -d '#')" || true # no status line yet
      line+=" $(basename -- "$f" .log)=${t:-0}"
    done
    echo "$line" >>"$mlog"
    sleep "$MONITOR_SECS"
  done
}
monitor &
monitor_pid=$!

pids=()
for i in "${!cores[@]}"; do
  [[ -n "${assigned[i]}" ]] || continue
  read -r -a mine <<<"${assigned[i]}"
  docker exec \
    -e "OUT=/results/core-${cores[i]}" -e "WORK=/results/corpus" \
    -e "LONG_TARGETS=$LONG_TARGETS" -e "LONG_SECS=$LONG_SECS" -e "SHORT_SECS=$SHORT_SECS" \
    -w /work "$name" taskset -c "${cores[i]}" scripts/fuzz-long-run.sh "${mine[@]}" &
  pids+=("$!")
done

status=0
for p in "${pids[@]}"; do wait "$p" || status=1; done
echo "results in $RESULTS; exit $status"
exit "$status"
