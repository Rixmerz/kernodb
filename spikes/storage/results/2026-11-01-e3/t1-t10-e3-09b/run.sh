#!/usr/bin/env bash
# Interleaved before/after T1-T10 run (E3-09b). Same machine, session, seeds.
set -u
S=${S:-$(mktemp -d)}  # scratch dir holding sb-before / sb-after binaries
OUT=$S/t
mkdir -p $OUT
WL="bulk_load point_reads range_scans small_commits point_deletes append_seq history_sim livespec cauce agent_memory mixed concurrent_commits rmw_counters"
one() { # bin label engine workload n rep threads file
  local line
  line=$($1 run --engine $3 --workload $4 --n $5 --durability sync --dir $S/bd --seed $((42+$6)) --cold false --threads $7 --trace false 2>>$OUT/err.log | tail -1)
  rm -rf $S/bd
  [ -n "$line" ] && python3 -c 'import json,sys; v=json.loads(sys.argv[1]); v["rep"]=int(sys.argv[2]); print(json.dumps(v))' "$line" $6 >> $8 || echo "FAIL $2 $3 $4 $5 $6" >> $OUT/err.log
}
for w in $WL; do for rep in 0 1 2; do
  echo "$(date +%T) $w rep $rep" >> $OUT/progress.log
  one $S/sb-before b kerno $w medium $rep 8 $OUT/before.jsonl
  one $S/sb-after a kerno $w medium $rep 8 $OUT/after.jsonl
  for e in lsm btree fjall; do one $S/sb-after r $e $w medium $rep 8 $OUT/after.jsonl; done
done; done
for rep in 0 1 2; do
  one $S/sb-before b kerno concurrent_commits medium $rep 1 $OUT/before-t1.jsonl
  one $S/sb-after a kerno concurrent_commits medium $rep 1 $OUT/after-t1.jsonl
done
one $S/sb-before b kerno bulk_load large 0 8 $OUT/before.jsonl
one $S/sb-after a kerno bulk_load large 0 8 $OUT/after.jsonl
echo "$(date +%T) done" >> $OUT/progress.log
