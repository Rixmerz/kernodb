#!/bin/bash
# Final U6 measurements of the shipped code: three runs at the default pacing, one at pause_ratio 29.
# Usage: final.sh <results-dir> <unit_bench binary>
out=$1; bin=$2
for n in 1 2 3 p29; do
  f=$out/final-U6-run$n
  args=(--only U6 --diag --budget 1000,10000 --dir /dev/shm/k2512final)
  [ $n = p29 ] && args+=(--pacing 16,29)
  rm -rf /dev/shm/k2512final
  { echo "loadavg: $(cat /proc/loadavg)"; echo "dir: /dev/shm (tmpfs)"; echo "args: ${args[*]}"; } > $f.txt
  $bin "${args[@]}" >> $f.txt 2> $f.err
  echo "loadavg at end: $(cat /proc/loadavg)" >> $f.txt
done
echo done > $out/final.done
