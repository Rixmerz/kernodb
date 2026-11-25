# Close and recovery open, second before/after pass (2026-11-25)

Report: [`../../perf-gaps-2026-11.md`](../../perf-gaps-2026-11.md), "Rank 5 and the recovery open", subsection "Second pass". Method: ADR-0012. The first pass is in [`../2026-11-24-close-recovery/`](../2026-11-24-close-recovery/) and is not changed.

- `base-N.*` / `head-N.*`: `spike-bench suite --durability sync` (kerno and SQLite) at base `9f8c60a` (git archive, own target directory) and at head `ee2afee`, run alternately, base first. Rounds 1 to 3: `bulk_load` and `cauce`, 10k and 100k. Rounds 4 to 8: kerno `bulk_load` 100k. `load-*.txt` is the load average (1, 5, 15 min) right before each run.
- `python/`: `bindings/python/bench/vs_sqlite.py --n 100000 --reps 1`, three alternating rounds, one release wheel per side (`maturin build --release`, own venv and target directory). Both sides ran the same script (the one at head). `python-tables.md` is the summary.
- `tables.md`: summary of the Rust runs (`python3 -I aggregate.py`, `python3 -I aggregate-python.py` from this directory).
- Machine: same as the first pass (AMD Ryzen 7 4800H, btrfs, `fdatasync` p50 0.74 ms in `../2026-11-24-close-recovery/env.json`); not re-measured. The machine was busier than in the first pass: 1-minute load 2.05 to 13.81 at run starts (see `load-*.txt`).

**Label caveat.** `python/base-N.json` carries `env.commit: "ee2afee"`. That field is read from the git checkout the script runs in (the head worktree, for both sides), not from the wheel. The `base-N` runs used the wheel built from base `9f8c60a`; the raw files are kept as produced, and this note is the correction. The Rust `base-N.jsonl` rows carry no commit field; they came from the binary built from a `git archive` of `9f8c60a`.
