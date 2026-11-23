#!/usr/bin/env bash
PY=/home/rixmerz/kernodb-perf/venv/bin/python3
PS=/home/rixmerz/kernodb-perf/venv/bin/py-spy
D=/home/rixmerz/kernodb-perf/prof
DRV=/home/rixmerz/kernodb-perf/scripts/prof_driver.py
run() { wl=$1; reps=$2; rm -rf "$D/$wl-100k"; "$PS" record --native -r 250 -f raw -o "$D/$wl-100k.raw" -- "$PY" "$DRV" "$wl" 100000 "$D/$wl-100k" "$reps" > "$D/$wl-100k.stderr" 2>&1; echo "$wl exit=$?"; }
run sym-store 2
run doc-store 2
run update 2
run relate 2
run traverse 3
run scan 20
run point-read 20
run close 3
run reopen 20
echo PROF-DONE
