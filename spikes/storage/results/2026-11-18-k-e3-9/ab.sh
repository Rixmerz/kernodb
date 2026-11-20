#!/bin/bash
# Alternates the new replan (after) and the old full replan (before, KERNO_AB_FULL_REPLAN) of one binary.
cd /tmp/k2512
for n in 1 2 3; do
  for v in after before; do
    f=ab-$v-U6-run$n
    { echo "loadavg: $(cat /proc/loadavg)"; echo "dir: /dev/shm (tmpfs)"; echo "variant: $v"; } > $f.txt
    if [ $v = before ]; then export KERNO_AB_FULL_REPLAN=1; else unset KERNO_AB_FULL_REPLAN; fi
    KERNO_RETENTION_TRACE=1 ./unit_bench_ab --only U6 --diag --budget 1000,10000 --dir /dev/shm/k2512 >> $f.txt 2> $f.err
    echo "loadavg at end: $(cat /proc/loadavg)" >> $f.txt
  done
done
echo done > ab.done
