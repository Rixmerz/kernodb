# Write path (Rank 4) before/after against SQLite (2026-11-25)

Report: [`../../perf-gaps-2026-11.md`](../../perf-gaps-2026-11.md), "Rank 4, write path: before/after". Method: ADR-0012.

- `python/{base,head}-N-R.{json,md,log}`: `bindings/python/bench/vs_sqlite.py --n N --reps 1` (both 10k and 100k; each round is a separate process), round R in 1..6; rounds 1-3 ran base first, rounds 4-6 head first (the first three rounds ran under higher load, base first, which favoured head; rounds 4-6 balance the order). One release wheel per side (`maturin build --release`, own target directory and venv, Python 3.14.9). Base is `ea8b610` (parent of the write-path commit `7c27bc6`), head is `baf4085`. Both sides ran the same script (identical at both commits). `env.commit` in the JSON is read from the checkout the script runs in (head for both sides), not from the wheel.
- `load-{base,head}-N-R.txt`: 1, 5 and 15 minute load average right before each run.
- `tables.md`: every row, from `python3 -I aggregate.py`. `per-run.md`: the raw number of each run for the write rows.
- No benchmark row or script line was changed for this run.
- Machine: the owner's laptop (AMD Ryzen 7 4800H, btrfs). It was NOT idle: 1-minute load 2.63 to 19.14 at run starts (rounds 1-3 at 10k: 10.67 to 19.14; rounds 1-3 at 100k: 3.35 to 9.98; rounds 4-6: 2.63 to 3.98). `env.json`: machine and `fdatasync` latency of the disk (`spike-bench env`), measured on the same machine after the runs, not during them (its `loadavg` is the load at that moment).
