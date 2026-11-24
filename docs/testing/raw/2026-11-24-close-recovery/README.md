# Close and recovery open, before and after (2026-11-24)

Report: [`../../perf-gaps-2026-11.md`](../../perf-gaps-2026-11.md), "Rank 5 and the recovery open". Method: ADR-0012.

- `base-N.*` / `head-N.*`: `spike-bench suite` (kerno and SQLite) at base `9f8c60a` and at the head, run alternately. Rounds 1 to 3 are `bulk_load` and `cauce` at 10k and 100k; rounds 4 to 8 are kerno `bulk_load` at 100k. `load-*.txt` is the load average before and after each run.
- `python/`: `vs_sqlite.py --n 100000 --reps 1`, three alternating rounds; `python-tables.md` is its summary.
- `tables.md`: summary of the Rust runs (`python3 -I aggregate.py`, `python3 -I aggregate-python.py` from this directory).
- `env.json`: machine and the `fdatasync` latency of the disk.
- `experiment-1-lz4/`, `experiment-2-lz4-and-syncs/`: one Python wheel with temporary environment switches (`KERNO_X_*`, not in the code) for the LZ4 pass, the intermediate syncs and a close trace (`XTRACE` lines in the `.log` files). Config `none`: no LZ4; `F`: LZ4 (the base behaviour); `FC`: LZ4 in flush and compaction; `noneNS`: no LZ4 and no intermediate syncs (the shipped behaviour); `FNS`: LZ4 and no intermediate syncs.
- `first-pass-flush-only/`, `second-pass-skip-lz4-keep-syncs/`: earlier full passes with the head before the syncs were turned off. They are kept because they show the LZ4-only loss on the same benchmark.
