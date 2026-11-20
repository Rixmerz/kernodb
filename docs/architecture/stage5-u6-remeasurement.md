# Stage 5 · E5-00 — Re-measurement of U6 with active indexes and relations

Entry gate of Stage 5 ([minutes](stage5-record.md) §3, row E5-00; risk K-E3-9 / M-9). The thresholds do not change: U6 passes if the write p99 during `run_retention` is ≤ the worse of the two "before" windows, the throughput is ≥ the lower of them and the read p99 is ≤ the worse "before" ([ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md)).

## How it was measured

```
cargo run --release -p kerno-units --example unit_bench -- --only U6 --indexes --relations
```

- Full mode (not `--quick`), seed `0xE309`, 5 rounds, two writers of 1 KiB and one reader.
- `--relations`: each update by a writer creates a relation in the same change set (K-E3-9).
- `--indexes` (new option of the example, measurement only): `rebuild_index` after loading the data and at the start of each round, so that retention runs over a database with index entries (M-9). The header of the raw output records `relations=true indexes=true`.
- Both retention budgets that the example measures were run.

## Summary

| Retention budget | Write p99, during / before | Throughput, during / before | Read p99, during / before | Retention | U6 |
|---|---|---|---|---|---|
| 1000 units / 10000 ops | 1.804 ms / 1.704 ms (1.06×) | 3243.0 / 3473.2 ops/s (0.93×) | 0.0050 / 0.0044 ms (1.13×) | 34,736 rev/s | **FAIL** |
| 25 units / 100 ops | 1.889 ms / 1.602 ms (1.18×) | 3308.2 / 3287.4 ops/s (1.01×) | 0.0045 / 0.0202 ms (0.22×) | 23,774 rev/s | **FAIL** |

U6 still fails with active indexes and relations, as in Stages 3 and 4 (K-E3-9: write p99 1.05–1.18×, throughput 0.89–0.94×). The large budget fails all three conditions (write p99, throughput and read p99); the small one fails only the write p99. The observations of Stage 5 do not write to the engine, so they should not aggravate this. The failure stays documented as a risk and does not block (minutes §5, merge policy). No threshold was lowered.

Variation between passes: an earlier first pass, without the `indexes` mark in the header (same order), gave write p99 of 1.26× and 1.05× for the large and small budgets, both also FAIL. The machine is shared and the variation between passes is of the order of the margins measured, so one pass does not tell 1.05× from 1.18×. That pass is not committed; the raw output below is that of the pass that does record `indexes=true`.

## Raw output

```text
     Running `target/release/examples/unit_bench --only U6 --indexes --relations`
# unit_bench mode=full relations=true indexes=true seed=0xe309 dir=/tmp engine=KernoDb options=Options { durability: Sync, cache_bytes: 67108864, write_buffer_bytes: 8388608 } vfs=StdVfs rounds=5
## U6: run_retention (KeepLast(1), RetentionBudget { max_units: 1000, max_ops: 10000 }) with 2 writer(s) of 1 KiB updates and 1 reader
raw U6 low load (1 writer, no reader) write: n=12845 p50_ms=0.2050 p99_ms=0.5426 max_ms=6.4466 mean_ms=0.2305
raw U6 low load throughput_ops_s=4281.2
raw U6 round 0: before_a 3.00s 11204 writes, before_b 3.00s 10367 writes, during 6.88s 23606 writes
raw U6 round 1: before_a 3.00s 10373 writes, before_b 3.00s 10384 writes, during 7.38s 23836 writes
raw U6 round 2: before_a 3.00s 12218 writes, before_b 3.00s 9947 writes, during 6.97s 23034 writes
raw U6 round 3: before_a 3.00s 8524 writes, before_b 3.00s 10985 writes, during 6.70s 22136 writes
raw U6 round 4: before_a 3.00s 10427 writes, before_b 3.00s 10416 writes, during 7.54s 22429 writes
raw U6 before_a write: n=52746 p50_ms=0.5258 p99_ms=1.5165 max_ms=39.5652 mean_ms=0.5667
raw U6 before_a read: n=6251433 p50_ms=0.0021 p99_ms=0.0044 max_ms=1.8763 mean_ms=0.0024
raw U6 before_a write_throughput_ops_s=3516.2 secs=15.00
raw U6 before_b write: n=52099 p50_ms=0.5199 p99_ms=1.7035 max_ms=22.7392 mean_ms=0.5736
raw U6 before_b read: n=6499991 p50_ms=0.0021 p99_ms=0.0038 max_ms=2.8491 mean_ms=0.0023
raw U6 before_b write_throughput_ops_s=3473.2 secs=15.00
raw U6 during write: n=115041 p50_ms=0.5440 p99_ms=1.8041 max_ms=22.6643 mean_ms=0.6147
raw U6 during read: n=13317311 p50_ms=0.0023 p99_ms=0.0050 max_ms=6.2811 mean_ms=0.0026
raw U6 during write_throughput_ops_s=3243.0 secs=35.47
raw U6 retention: calls=55 revisions_pruned=1232224 units_retained=50500 secs=35.474 revisions_per_s=34736 units_per_s=1424
verdict U6: FAIL budget 1000 units/10000 ops: write p99 during 1.804 ms vs before 1.704 ms (1.06x; low load 0.543 ms); throughput during 3243.0 ops/s vs before 3473.2 ops/s (0.93x); read p99 during 0.0050 ms vs before 0.0044 ms (1.13x); retention 34736 revisions/s
## U6: run_retention (KeepLast(1), RetentionBudget { max_units: 25, max_ops: 100 }) with 2 writer(s) of 1 KiB updates and 1 reader
raw U6 low load (1 writer, no reader) write: n=13072 p50_ms=0.2053 p99_ms=0.5853 max_ms=15.3979 mean_ms=0.2268
raw U6 low load throughput_ops_s=4357.0
raw U6 round 0: before_a 3.00s 9315 writes, before_b 3.00s 8754 writes, during 9.43s 32463 writes
raw U6 round 1: before_a 3.00s 10362 writes, before_b 3.00s 10824 writes, during 10.99s 35984 writes
raw U6 round 2: before_a 3.00s 10690 writes, before_b 3.00s 9978 writes, during 10.76s 35930 writes
raw U6 round 3: before_a 3.00s 8595 writes, before_b 3.00s 11121 writes, during 12.35s 40200 writes
raw U6 round 4: before_a 3.00s 10350 writes, before_b 3.00s 10885 writes, during 10.87s 35383 writes
raw U6 before_a write: n=49312 p50_ms=0.5580 p99_ms=1.5563 max_ms=24.9822 mean_ms=0.6060
raw U6 before_a read: n=5892313 p50_ms=0.0021 p99_ms=0.0202 max_ms=11.3883 mean_ms=0.0025
raw U6 before_a write_throughput_ops_s=3287.4 secs=15.00
raw U6 before_b write: n=51562 p50_ms=0.5338 p99_ms=1.6015 max_ms=13.9102 mean_ms=0.5797
raw U6 before_b read: n=6234138 p50_ms=0.0021 p99_ms=0.0042 max_ms=3.7242 mean_ms=0.0024
raw U6 before_b write_throughput_ops_s=3437.4 secs=15.00
raw U6 during write: n=179960 p50_ms=0.5280 p99_ms=1.8890 max_ms=191.4949 mean_ms=0.6025
raw U6 during read: n=21124358 p50_ms=0.0023 p99_ms=0.0045 max_ms=6.0994 mean_ms=0.0025
raw U6 during write_throughput_ops_s=3308.2 secs=54.40
raw U6 retention: calls=2020 revisions_pruned=1293233 units_retained=50500 secs=54.398 revisions_per_s=23774 units_per_s=928
verdict U6: FAIL budget 25 units/100 ops: write p99 during 1.889 ms vs before 1.602 ms (1.18x; low load 0.585 ms); throughput during 3308.2 ops/s vs before 3287.4 ops/s (1.01x); read p99 during 0.0045 ms vs before 0.0202 ms (0.22x); retention 23774 revisions/s
time U6: 192.1 s
# summary
U6 FAIL budget 1000 units/10000 ops: write p99 during 1.804 ms vs before 1.704 ms (1.06x; low load 0.543 ms); throughput during 3243.0 ops/s vs before 3473.2 ops/s (0.93x); read p99 during 0.0050 ms vs before 0.0044 ms (1.13x); retention 34736 revisions/s
U6 FAIL budget 25 units/100 ops: write p99 during 1.889 ms vs before 1.602 ms (1.18x; low load 0.585 ms); throughput during 3308.2 ops/s vs before 3287.4 ops/s (1.01x); read p99 during 0.0045 ms vs before 0.0202 ms (0.22x); retention 23774 revisions/s
```
