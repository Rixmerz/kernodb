# Kerno performance gaps against SQLite (2026-11-23)

Kerno still loses 1.8x to 127x on batch writes, relations and traversals, scans by type, close, and reopen, and it wins on substring and keyword search. Profiles point to one cause for most losses: each record or each unit costs a separate engine lookup or write, so per-record cost dominates. The cheapest honest gap to close is the benchmark's reopen row, which is not like-for-like.

Measured on `main` at commit `f7a5004`. Raw data: [`raw/2026-11-23-perf/`](raw/2026-11-23-perf/). Methodology: [ADR-0012](adr/0012-benchmark-methodology-and-exit-thresholds.md), [ADR-0025](adr/0025-substring-index-and-sqlite-parity.md), [`python-vs-sqlite.md`](python-vs-sqlite.md), [`sqlite-benchmark.md`](sqlite-benchmark.md).

## Setup and what changed from the 2026-11-22 run

- **Python binding (units level):** `bindings/python/bench/vs_sqlite.py`, unchanged. Built with `maturin build --release` from `f7a5004` and installed into a venv outside the repo. The import is `kerno/__init__.py` in that venv, and `search_substring` is present, so the substring rows are the index ("kerno index" in the table), not the Python-filter path.
- **Interpreter:** Python 3.14.9 and SQLite 3.54.0 (only Python 3.14 is installed here). The 2026-11-22 run used Python 3.12.14 and SQLite 3.53.1, so absolute times are not comparable to it. Each ratio compares both engines in the same process and the same repetition, so the ratios are.
- **Repetitions:** 3, each a fresh process and fresh directories, median with range, as in ADR-0012. Data on `/home` (ext4/btrfs NVMe), not tmpfs.
- **Load:** 1-minute load average at the start of each run: 4.71 (10k) and 4.66 (100k), see `raw/2026-11-23-perf/load-py-*.txt`. At the start of the Rust run it was recorded in `load-rust.txt`.
- **Rust side:** `spikes/storage/run-sqlite-comparison.sh` with `SCALES=10000,100000 REPS=5`, both durability pairs (`sync` is the equivalent pair, `buffered` is not). This compares the storage engines over a key-value contract (`sqlite-benchmark.md` §1). It is not a units-level Rust benchmark, so it is not a Rust twin of the Python run. The aggregator pools both scales into one table (10 repetitions per engine), so the per-scale tables were produced from the raw JSONL and are in `raw/2026-11-23-perf/rust-by-scale/{10k,100k}/tables.md`.

## 1. Results

### 1a. Python binding, units level (median of 3 repetitions; time in seconds)

| Workload | 10k Kerno | 10k SQLite | 10k ratio | 100k Kerno | 100k SQLite | 100k ratio |
|---|---|---|---|---|---|---|
| store x2000, one commit each | 2.03 | 2.2 | 0.92 | 2.24 | 2.12 | 1.05 |
| store x10000 / x100000, batches of 10000 | 0.0999 | 0.0636 | 1.57 | 1.09 | 0.606 | 1.81 |
| point read x2000 | 0.00939 | 0.00932 | 1.01 | 0.0156 | 0.0105 | 1.49 |
| scan all units of a type (12000 / 102000) | 0.023 | 0.0105 | 2.18 | 0.28 | 0.0896 | 3.13 |
| update x10000 / x100000 keeping history, batches | 0.142 | 0.0801 | 1.78 | 1.71 | 0.878 | 1.95 |
| relate x10000 / x100000, batches | 0.148 | 0.0269 | 5.51 | 3.03 | 0.388 | 7.81 |
| related() x1000 | 0.0162 | 0.0042 | 3.86 | 0.0256 | 0.00453 | 5.64 |
| two-hop traversal x200 | 0.00734 | 0.00117 | 6.29 | 0.0154 | 0.00137 | 11.21 |
| all relations of a type | 0.0077 | 0.0057 | 1.35 | 0.0725 | 0.073 | 0.99 |
| history x1000 | 0.00683 | 0.00509 | 1.34 | 0.00857 | 0.00508 | 1.69 |
| store substring-indexed, batches | 0.475 | 0.122 | 3.89 | 6.3 | 1.79 | 3.51 |
| substring search x20 vs `LIKE '%x%'` | 0.00489 | 0.0251 | **0.19** | 0.1 | 0.311 | **0.32** |
| substring search x20 vs FTS5 trigram | 0.00489 | 0.0169 | **0.29** | 0.1 | 0.167 | **0.60** |
| store keyword-indexed, batches | 0.62 | 0.104 | 5.93 | 7.37 | 1.0 | 7.33 |
| keyword search x100, top 10 | 0.00845 | 0.0235 | **0.36** | 0.0922 | 0.236 | **0.39** |
| close | 0.158 | 0.00124 | 127.41 | 0.174 | 0.00336 | 51.66 |
| reopen | 0.00881 | 0.00101 | 8.69 | 0.0329 | 0.0254 | 1.30 |
| disk MiB (size, not time) | 31.1 | 17 | 1.83 | 299 | 162 | 1.84 |

The substring hit counts match between the engines (18,426 at 10k, 186,413 at 100k), and the script asserts it. Bold ratios are the wins.

The 100k reopen row has a SQLite range of 0.000869 to 0.0283 s across repetitions, so its 1.30 median is noise-dominated. The 10k row is larger but the same method applies. See §4 for why the row itself is unfair.

### 1b. Rust storage harness (`spike-bench`, key-value contract), `sync` pair, kerno worse with disjoint ranges

Source: `raw/2026-11-23-perf/rust-by-scale/{10k,100k}/tables.md`. Ratio = kerno / sqlite, lower is better for kerno.

| Scale | Workload | Metric | Kerno | SQLite | Ratio |
|---|---|---|---|---|---|
| 100k | bulk_load | `close_ms` | 46.0 | 4.95 | 9.29 |
| 100k | bulk_load | `reopen_ms` | 3.61 | 0.44 | 8.23 |
| 100k | bulk_load | `recovery_ms` | 9.04 | 0.46 | 19.56 |
| 100k | append_seq | `space_amp` | 1.76 | 1.42 | 1.24 |
| 100k | cauce | `recovery_ms` | 28.6 | 0.39 | 72.93 |
| 10k | bulk_load | `close_ms` | 21.6 | 2.29 | 9.44 |
| 10k | bulk_load | `reopen_ms` | 2.39 | 0.38 | 6.37 |
| 10k | bulk_load | `recovery_ms` | 6.33 | 0.38 | 16.61 |
| 10k | append_seq | `peak_rss_mb` | 16.5 | 10.6 | 1.56 |
| 10k | livespec | `module_scan_p50_us` | 134 | 101 | 1.32 |
| 10k | livespec | `module_scan_p99_us` | 317 | 261 | 1.21 |
| 10k | cauce | `recovery_ms` | 16.4 | 0.39 | 41.93 |

Summary counts (`sync`): at 100k kerno is better in 42, not distinguishable in 20, worse in 5; at 10k better in 45, not distinguishable 15, worse 7. With `buffered` (not an equal guarantee): 100k better 54, worse 7; 10k better 58, worse 9. Kerno wins the storage-level point reads at 4 threads, deletes and the mixed workload, and it ties on small commits and most of the rest. Its losses are close, reopen, recovery, and a few space and scan metrics.

The Rust `sync` pair is an equal-guarantee comparison (`sqlite-benchmark.md` §1). Its losses are not the units-level losses above, so the two layers should not be merged in one table.

## 2. Profiles and root causes

Method: `py-spy record --native -r 250 -f raw` on one process per workload (`raw/2026-11-23-perf/profiles/prof_driver.py`), 100k units. The driver runs only the timed section, and `analyze.py` keeps only samples whose stack goes through `timed (prof_driver.py`, so data generation is excluded. Without that filter `make_data` alone was 37.6% of the batch-store samples. Rust frames are v0-mangled; `analyze.py` decodes the names. Shares below are inclusive unless marked self.

No `perf`, `valgrind` or `cargo flamegraph` is installed. Functions marked "inferred" are attributed by symbol and code reading, not by a line-level measurement.

| Profile | Samples (timed) | Top frames (share of timed samples) | Root cause |
|---|---|---|---|
| batch store, 100k (`profiles/bs100k.raw`) | 654 | memtable `apply_commit` 12.4%; skiplist `search_position` 10.2%; overlay `BTreeMap::insert` 9.0%; `commit_queue::Request` drop 6.1%; `Vec` grow 5.7%; `PyString` to Cow 4.1% | Per-record cost of the write path: one overlay entry, one memtable insert and one request free per record |
| substring store, 100k (`profiles/sym-store-100k.raw`) | 3738 | `ChangeSet::stage_substring` self **22.3%** (inclusive 35.6%); memtable `apply_commit` 8.1%; overlay `BTreeMap` drain 4.6% | Per-trigram bookkeeping in `stage_substring` (inferred: `list_key` allocation and a map entry per trigram, see `substring/mod.rs:278`) |
| keyword store, 100k (`profiles/doc-store-100k.raw`) | 4367 | overlay `BTreeMap::insert` 26.6%; memtable `apply_commit` 12.5%; commit `Request` drop 10.3%; skiplist search 10.3% | One posting record per term and unit, the same per-record cost as the first trigram design (ADR-0025) |
| update, 100k (`profiles/update-100k.raw`) | 1330 | memtable `apply_commit` 12.1%; skiplist search 10.0%; SST `Table::get` 7.6%; `memcpy` 5.7% | Read before write: the current head is looked up in memtable and SST for each update (inferred from the symbols) |
| relate, 100k (`profiles/relate-100k.raw`) | 1710 | skiplist `lower_bound` 14.5%; SST `Table::get` 9.2%; overlay/`BTreeSet` insert 9.2%; block cache 5.0%; varint decode 4.3% | Endpoint checks and the id-collision check are point reads (`require_active` at `db.rs:1666`, `create_relation_unguarded` at `db.rs:1706`), then three records and counts |
| two-hop / related, 100k (`profiles/traverse-100k-b.raw`, 2363 samples, 300 reps) | 2363 | skiplist `lower_bound` 20.2%; SST `Table::get` 17.4%; varint decode 7.4%; block cache 7.1%; `Version::get` 4.6% | Each neighbour is a point read (memtable, then SST, then block cache). SQLite does one index range and a join |
| scan by type, 100k (`profiles/scan-100k.raw`) | 1487 | SST `Table::get` 15.6%; skiplist `lower_bound` 14.5%; `reference_filter` 3.4%; `UnitHead::decode` 2.6%; `content_hash` 1.7% | One point lookup per unit (N gets) plus per-unit decode and checksum, not a range walk over the type's keys |
| point read, 100k (`profiles/point-read-100k.raw`) | 97 | SST `Table::get` 24.7%; skiplist `lower_bound` 21.7%; block cache 14.4%; varint decode 11.3% | Storage lookups. Python conversion is not among the top frames. Only 97 samples; treat as indicative |
| close, 100k (`profiles/close-100k.raw`) | 209 | LZ4 compress (`lz4_flex`) 24.4%; `Table::open_file` 6.7%; filter build 3.8%; `decode_index` 5.7%; `pread64` 4.8% | The close writes the memtable as an SST with LZ4 and a bloom filter, then reopens the tables (`close_inner`, `db.rs:2232`) |
| reopen, 100k (`profiles/reopen-100k.raw`) | 23 | `fdatasync`/`fsync` 21.7% (inside `Engine::close`, `ManifestWriter::append` 13.0%) | The benchmark's kerno side is `open().close()`, so each rep fsyncs the manifest. The SQLite side does not close its connection (see §4). Only 23 samples; treat as indicative |

Findings per gap:

- **Per-record cost (batch store, update, relate, keyword store).** The overlay `BTreeMap` insert, the memtable insert and the free of the commit request appear on every record. The keyword store has the largest share (overlay 26.6%). This confirms note #1736 for the keyword index. It does not confirm it for plain batch stores, where the same three costs are spread over more frames (overlay 9.0%, memtable 12.4%).
- **Per-trigram bookkeeping (substring store).** The substring store's largest self-cost is `stage_substring` (22.3%), not the engine. The engine part is 8.1% (memtable). So the trigram writer's own code is the biggest single cost, ahead of the engine.
- **Point lookups in traversal and scan.** Every neighbour, and every unit of a type, costs a memtable skiplist search plus an SST lookup plus a block-cache hit. That is the "one point read per record" pattern again, now on the read path.
- **FFI.** Only the batch store shows a binding-side cost worth naming: `PyString` to Cow at 4.1% (the Python str to Rust conversion for each op). The read profiles show no PyO3 frame in their top ten. `with_db` releases the GIL (`bindings/python/src/lib.rs:543-550`), so FFI is not a contention cost here.
- **Close and reopen.** Close is the LZ4 flush and table reopen (§2 table). Reopen is `fsync`, and the timed section includes a close, which the SQLite side does not do.

## 3. Proposed fixes, ranked by gain per effort

Gains are estimates from the profile shares above, not measurements. Each fix needs its own before/after run.

**Rank 1. Make the reopen row like-for-like (benchmark, not engine).** Gain: removes the largest unfair row, no kerno change. Effort: small. Risk: none to the product. Proposal: on both sides, open, read one unit, and close the handle inside the timed section (`sq.close()` on SQLite, which the script does not call). This strengthens the benchmark rather than weakening it. See §4.

**Rank 2. Substring store: build each unit's gram set once and append sorted runs.** `stage_substring` (`substring/mod.rs:278`) does one map entry per trigram, and its self-cost is 22.3% of the store. Building per-list vectors and inserting them once per list (already sorted by unit) should remove most of that. Expected: substring store 3.5x toward about 2.5x (estimate). Risk: low; in-memory, and the on-disk format does not change. Effort: small to medium.

**Rank 3. Scan by type: one ordered range over the type's keys, not N point gets.** The profile shows one lookup per unit. A range over the type's unit keys with a cursor, decoding each head once, removes the lookup share (about 35% of scan time). Expected: scan 3.1x toward about 1.5x to 2x (estimate). Risk: medium; it must keep the snapshot semantics and the `content_hash` check. Effort: medium.

**Rank 4. Write path: reduce per-record overlay and request costs for batches.** Replace the per-record overlay `BTreeMap` insert with a buffer sorted once at commit, and reuse the commit request. The three costs are about 9% (overlay), 12% (memtable) and 6% (request drop) of the batch store. Expected: batch store and update 1.8x and 2.0x toward about 1.4x (estimate, 15 to 25%). Risk: medium; it changes `WriteTxn` (`crates/kerno-storage/src/txn.rs`), so the crash and recovery tests must run. Effort: medium.

**Rank 5. Close: skip the LZ4 pass and filter build for the close-time flush, or defer table reopen.** LZ4 (24.4%), `open_file` (6.7%) and filter build (3.8%) are about 35% of close. Expected: close from about 170 ms toward about 110 ms (estimate). The cost moves to reopen and recovery, and disk size grows (already 1.8x). Risk: a format and durability trade-off; needs a decision. Effort: medium.

**Rank 6. Adjacency as posting blocks (relate, related, two-hop).** Store each unit's neighbours per type in one block, as the trigram index does for postings. Then a traversal reads one block instead of N head decodes. The trigram precedent (ADR-0025) went from 21x to 3.5x with the same move. Expected: related 5.6x and two-hop 11.2x toward about 2x to 3x (estimate). Risk: high; a format change needs a layer bump, a test at `crates/kerno-cli/tests/cli.rs:412` (constraint #1281), and a docs update (constraint #1532). Effort: high. **Applied: see §5.**
**Rank 5 and the recovery open: implemented (decision, method and numbers).**

*Decision, close.* The close-time flush skips the LZ4 pass **and** the intermediate syncs of its output (`flush::write_memtable` with `closing`). The table reopen is not deferred and the filter is still built:

- **LZ4 alone is a loss here.** Python close, 100k units, one wheel with environment switches (raw data in [`raw/2026-11-24-close-recovery/experiment-2-lz4-and-syncs/`](raw/2026-11-24-close-recovery/experiment-2-lz4-and-syncs/), median of 3, close flush 11.7 MB compressed or 20.3 MB plain): LZ4 and syncs as before 0.156 s (flush 138 ms); no LZ4, syncs on 0.195 s (flush 178 ms); LZ4, syncs off 0.148 s (flush 128 ms); no LZ4, syncs off 0.135 s (flush 119 ms). The 24.4% of the profile is CPU in the compressor, but the 8.5 MB it saves cost more to write and sync than the pass costs. The intermediate syncs are what makes the write slow (the compactions of a close already turn them off); with them off, skipping LZ4 is a further gain of about 9%.
- **No format change.** The table keeps `BLOCK_LZ4` and stores every data block with codec 0 (v1.md §4.4 allows that), so every table still carries the bit and readers need nothing new. Compaction and running flushes still compress. Disk after close grows 2.7% (298.5 to 306.7 MiB at 100k).
- **Reopen not deferred.** The open of the table written by a flush is also its read-back check (SST-O1..O7); a close that skipped it would delete the WAL (W-9) on a table nobody read back, and a lazy table would add a not-yet-open state to every read path. Its share of a close (`open_file`, `decode_index`, `pread`) stays.
- **Filter kept.** The format requires it (SST-V3, `get` trusts the filter), 3.8% of a close.

*Decision, recovery open.* The flush of the replayed commits is deferred (`recovery::apply`) when background threads run: the replayed memtables stay in memory, the flusher writes them after the open, and the old logs stay until then (`SESSION_OPEN` keeps the old `log_number` and `last_seq`; a crash at any point replays the same logs). It is not deferred when a replayed log has a torn tail or a broken header, or when the replayed data is more than twice the write buffer. `Background::Inline` (test configuration, no flusher) keeps the flush at open, so the crash sweeps run as before; with the thread-local test hook `KernoDb::set_defer_recovery_flush` on, an `Inline` open defers it without threads, for the deterministic tests in `crates/kerno-storage/tests/close_recovery.rs`.

*Method (ADR-0012).* Same machine, one process and one directory per run, `sync` durability, base `9f8c60a` and head built in `release` in separate target directories and run alternately, round by round; 1-minute load 1.7 to 3.2 at each start (`load-*.txt`); `btrfs`, `fdatasync` p50 0.74 ms (`env.json`). Rust: `spike-bench suite` (`bulk_load` and `cauce`, kerno and SQLite, 10k and 100k; 3 rounds, plus 5 more of kerno `bulk_load` at 100k). Python: `bindings/python/bench/vs_sqlite.py --n 100000 --reps 1`, 3 rounds, wheels from `maturin build --release`. Medians, with the range when it is over 15% of the median. Raw data and `aggregate*.py` next to the tables in [`raw/2026-11-24-close-recovery/`](raw/2026-11-24-close-recovery/); the first two passes (flush without LZ4 only, and with syncs left on) are kept in `first-pass-flush-only/` and `second-pass-skip-lz4-keep-syncs/`.

| Metric (100k unless noted) | Before | After | SQLite |
|---|---:|---:|---:|
| Python `close`, s (3 runs) | 0.173 | 0.148 (0.130 to 0.180) | 0.0068 |
| Python `reopen`, s | 0.0204 | 0.0209 | 0.0315 (0.0009 to 0.037) |
| Python disk after close, MiB | 298.5 | 306.7 | 162.2 |
| Rust `bulk_load` `close_ms` (8 runs) | 40.1 (27.6 to 203.6) | 34.3 (28.0 to 52.2) | 4.7 |
| Rust `bulk_load` `reopen_ms` | 3.6 | 3.8 | 0.4 |
| Rust `bulk_load` `recovery_ms` (8 runs) | 10.3 (9.2 to 19.4) | 5.6 (4.3 to 8.9) | 0.5 |
| Rust `cauce` `recovery_ms` (3 runs) | 33.3 (25.8 to 54.5) | 16.1 (13.3 to 17.1) | 0.4 |
| Rust `bulk_load` `recovery_ms`, 10k | 7.6 (6.1 to 13.6) | 6.7 (4.3 to 10.1) | 0.4 |
| Rust `cauce` `recovery_ms`, 10k | 18.8 | 8.5 (7.6 to 9.2) | 0.4 |

*Reading.* Recovery open at 100k `bulk_load` is now about 5.6 ms against 3.8 ms for a clean reopen (it was 10.3 ms); `cauce` halves, from 33 to 16 ms, and stays above SQLite's 0.4 ms. The Python close is 15% faster at the median (0.173 to 0.148 s, the single-wheel experiment gives 0.156 to 0.135 s, 13%), against SQLite's 0.007 s, so the gap stays about 20x; the ranges of the two sides touch (3 runs each), so read it as indicative. The Rust close at 100k is bimodal in both builds (about 30 to 50 ms, or about 165 to 205 ms when a compaction lands at close: 3 of 8 base runs, none of 8 head runs; with 8 runs that is not evidence of a change). The Rust data are close to incompressible, which is why the LZ4 pass matters little there. **Not reproduced:** the 0.17 to 0.29 s of recovery open quoted for this task is in no harness of this report (the Rust `recovery_ms` are 5 to 33 ms and `vs_sqlite.py` has no recovery row), so the goal of "close to the clean reopen" is shown on the Rust rows only. The numbers above are the ones to replace the baseline in section 2 when the table is regenerated; no benchmark row changed.

**Rank 6. Adjacency as posting blocks (relate, related, two-hop).** Store each unit's neighbours per type in one block, as the trigram index does for postings. Then a traversal reads one block instead of N head decodes. The trigram precedent (ADR-0025) went from 21x to 3.5x with the same move. Expected: related 5.6x and two-hop 11.2x toward about 2x to 3x (estimate). Risk: high; a format change needs a layer bump, a test at `crates/kerno-cli/tests/cli.rs:412` (constraint #1281), and a docs update (constraint #1532). Effort: high.

**Rank 7. Keyword index: posting blocks for BM25 terms.** The same move as Rank 6 for keyword postings. Expected: keyword store 7.3x toward about 2x to 3x (estimate, by the trigram precedent). Risk: high; format change as in Rank 6. Effort: high. **Applied: see §5.**

**Rank 8. Point reads.** The storage lookups dominate, but the ratio is 1.01x at 10k and 1.49x at 100k, which is near noise. Gain small. Effort: medium to high. Not recommended now.

**Rank 9. Storage-level recovery (Rust harness, 16x to 73x).** Recovery replays the WAL (`recovery_ms`). Faster recovery needs more frequent checkpoints, which costs on the write path (Rank 5 trade-off). Not profiled in this run.

### 3a. Rank 2 and Rank 3 applied (before and after)

Method as in ADR-0012: `bindings/python/bench/vs_sqlite.py`, 3 repetitions, fresh process and directories, data on `/home`, same interpreter (3.14.9) as the "before" run. Before = §1a (commit `f7a5004`); after = this change. Raw: [`raw/2026-11-23-perf-rank23/`](raw/2026-11-23-perf-rank23/). Times in seconds, median; ratio = kerno / SQLite in the same run.

| Workload | Scale | Kerno before | Kerno after | SQLite before / after | Ratio before | Ratio after |
|---|---|---|---|---|---|---|
| store, substring-indexed, batches | 10k | 0.475 | 0.244 | 0.122 / 0.118 | 3.89 | 2.08 |
| store, substring-indexed, batches | 100k | 6.3 | 3.05 | 1.79 / 1.73 | 3.51 | 1.76 |
| scan all units of a type | 10k | 0.023 | 0.022 | 0.0105 / 0.0103 | 2.18 | 2.13 |
| scan all units of a type | 100k | 0.28 | 0.205 | 0.0896 / 0.0912 | 3.13 | 2.25 |

Substring hit counts still match SQLite (18,426 and 186,413). Substring search is unchanged (0.17x / 0.31x of `LIKE`).

- **Rank 2** beat its estimate (about 2.5x): 2.1x and 1.8x. The change is in `substring/mod.rs`: a `HashMap` of per-list vectors with one reused key buffer, sorted once at flush.
- **Rank 3** changed the read path from three point reads per unit to forward scans (`PagedGets` in `db.rs`; the reference and `IDX_TYPE` paths both use it; a page ends at 256 rows or 1 MiB, so payloads up to the 64 MiB limit are not read ahead; the benchmark's payloads are far below 1 MiB per page, so the rows above are not affected). At 100k it moved 3.13x to 2.25x, short of the 1.5x to 2x estimate. At 10k there is no measurable gain: a post-change profile (py-spy, 12k units) shows the rest is the engine scan itself (`Version::scan`, merge iterator), `load_unit` and `UnitHead::decode`, and building the Python `Unit` objects. Not established: how much of the remaining gap is the Python object construction, which SQLite's tuples do not pay (§4).

## 4. Benchmark fairness (not changed; these notes only)

- **Reopen is not like-for-like (unfair to both sides).** `vs_sqlite.py:247` times `kerno.open(kdir).close()`, which fsyncs the manifest. `vs_sqlite.py:248` times `sqlite3.connect(sfile).execute("select count(*) ...")` and never closes that connection. The kerno side includes a close and the SQLite side includes a count. Fix: Rank 1.
- **Two-hop is not one query on both sides (unfair to kerno).** `vs_sqlite.py:177-178` does `[[db.related(r.to_id) for r in db.related(ks[i])] ...]`, which is 1+N Python-level calls. SQLite does one join (`vs_sqlite.py:181`). Kerno's number therefore includes Python loop and FFI overhead. A server-side or single-call traversal would be a fair comparison, and it would be a feature.
- **Relate does more work on kerno (a real product difference, not a bench bug).** `create_relation_unguarded` (`db.rs:1706`) checks both endpoints (`require_active`, `db.rs:1666`), checks id collision, and writes three records plus counts. SQLite inserts one row and one index entry. The gap is the price of the validation and the adjacency design. It is not an artifact of the script.
- **Scan and point read return different objects (unfair to kerno).** Kerno builds `Unit` objects and verifies each payload's `content_hash` (in the scan profile). SQLite returns tuples without a checksum. The scan ratio includes that work.
- **Close compares different work (both ways).** Kerno's close persists the memtable, which is why its reopen and recovery are cheap. SQLite's close after per-commit fsync is nearly free. Report both, as the Rust table does.
- **Rust harness is a key-value contract (not the units layer).** `sqlite-benchmark.md` §1 says so. The SQLite side is one `WITHOUT ROWID` table. It is equal for the `sync` pair only.
- **Interpreter change.** 3.14 here, 3.12 in the 2026-11-22 run. Both sides in one run use the same interpreter, so the ratios hold; the absolute times do not compare with earlier documents.

## 5. Ranks 6 and 7 applied: posting blocks for adjacency and keyword postings

Change: [ADR-0026](../adr/0026-posting-blocks-for-adjacency-and-keyword-postings.md), layer version 9 ([units-v9](../file-format/units-v9.md)). Method: [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), the same script as §1 (`bindings/python/bench/vs_sqlite.py`, 3 repetitions, a fresh process per repetition, data on `/home`, Python 3.14.9, SQLite 3.54.0). "Before" is a wheel built from `ea8b610` (the parent of this change; its code is that of `f7a5004`, the report was added after), "after" a wheel of this change, both run back to back in one sitting (before, then after, at 10k and then at 100k). Raw data and the load average at each start: [`raw/2026-11-24-blocks/`](raw/2026-11-24-blocks/); the `commit` field of those JSON files is the checkout the script was run from, the same for both sides, not the wheel. Kerno / SQLite ratio (median of 3), lower is better; seconds are Kerno's.

| Row | 10k before | 10k after | 10k seconds, before to after | 100k before | 100k after | 100k seconds, before to after |
|---|---|---|---|---|---|---|
| store keyword-indexed, batches (Rank 7) | 7.27 | **3.74** | 0.634 to 0.307 | 6.68 | **3.52** | 6.88 to 3.67 |
| keyword search x100, top 10 (Rank 7) | 0.37 | **0.15** | 0.00873 to 0.00358 | 0.41 | **0.11** | 0.099 to 0.0264 |
| relate, batches (Rank 6) | 7.08 | **4.98** | 0.149 to 0.103 | 10.07 | 9.64 | 2.98 to 2.85 |
| `related()` x1000 (Rank 6) | 2.85 | **2.17** | 0.0116 to 0.00873 | 5.83 | **5.00** | 0.0268 to 0.0229 |
| two-hop traversal x200 (Rank 6) | 4.80 | **3.49** | 0.00520 to 0.00363 | 11.41 | **8.69** | 0.0159 to 0.0121 |
| all relations of a type | 1.11 | 1.10 | 0.00620 to 0.00637 | 0.96 | 0.95 | 0.0726 to 0.0731 |
| close | 103 | 57 | 0.147 to 0.0881 | 42 | 67 | 0.172 to 0.124 |
| reopen | 14.7 | 11.2 | 0.00702 to 0.00624 | 16.7 | 0.99 | 0.0197 to 0.0155 |
| disk size | 1.83 | 1.72 | 31.1 to 29.2 MiB | 1.84 | 1.76 | 299 to 286 MiB |

Load average (1 minute) at the start of each run: 4.74 (10k before), 4.10 (10k after), 3.74 (100k before), 3.59 (100k after). Two ratios at 100k move for a reason on SQLite's side: its `close` took 0.0041 s before and 0.0019 s after (Kerno's went 0.172 s to 0.124 s, so the ratio rose while the time fell), and its `reopen` has a repetition at 0.0286 s (range 0.00095 to 0.0286, median 0.0157), which makes the 0.99 ratio an outlier; Kerno's own reopen went 0.0197 s to 0.0155 s. Two earlier measurements of this change, taken with builds that were not kept, are superseded.

- **Rank 7 reached about what the trigram precedent promised** (keyword store 7.27x to 3.74x at 10k and 6.68x to 3.52x at 100k; expected 2x to 3x), and keyword search got faster again (0.37x to 0.15x, and 0.41x to 0.11x).
- **Rank 6 helped the traversals and the writes, part way.** Two-hop went 4.80x to 3.49x (10k) and 11.41x to 8.69x (100k); `related()` 2.85x to 2.17x and 5.83x to 5.00x; relate 7.08x to 4.98x (0.149 s to 0.103 s, ranges 0.143 to 0.163 against 0.099 to 0.104) and 10.07x to 9.64x at 100k (2.98 s to 2.85 s, ranges 2.95 to 3.04 against 2.71 to 2.95). The expected 2x to 3x for the traversals was not reached: each neighbour still costs a `REL` read, a head read of the other end and a mirror-entry seek (ADR-0026 option 4 would remove those and was not taken). Relate at 100k is within the range of its repetitions, not a clear gain; the writer-only runs below show it at or below the old build's time.
- **What the relate row was spending, and what was changed.** An earlier build of this change was slower than the build before it at 100k: 100k random pairs, one type, 1000 per commit, base and head alternated, 2.43 s against 2.91 s (+19%; 2.90 s against 3.18 s in the Python run), and the first explanation in this report (a read and rewrite of a block per list that already has entries) was a guess. Temporary timers around the commit path (not kept) put the seek that counts the entries of a list with no count record, which the old writer paid too, at 0.67 s of about 2.9 s (126,607 of them, 5.3 µs each; a point read of a count record costs 1.9 µs when it is absent and 3.2 µs when it is there), and the staging and the flush of the blocks at 0.14 s and 0.29 s against 0.08 s for the three blind puts of the old writer. Three changes brought the writer to the old build's time, none of them a format change: (1) a list that already has entries is read with a point read of its tail block, not a seek (`blocks::flush`; sound because the tail holds every id above the last bound and ids are minted ascending); (2) a list's staged entries are a sorted `Vec`, not a `BTreeMap` that allocated a node per list; (3) the counts and the staged entries of a change set are one map by list instead of two hash/set maps plus a third structure, which removes a key built twice and two map inserts per list.
- **Writer-only result**, the method of the finding that reopened this work ([`relate-alternated.txt`](raw/2026-11-24-blocks/relate-alternated.txt): 100k random-pair relates through `UnitDb`, 5 alternations of the old build and this one, seconds): 1000 per commit, old 2.436 (2.429 to 2.513), new 2.387 (2.329 to 2.476), 2% less; 10,000 per commit, old 2.597 (2.484 to 2.733), new 2.295 (2.224 to 2.343), 12% less. The criterion "no worse than before" is met for relate at both sizes.
- **Why the seek per new list is kept.** A first build of this change took a missing count record for an empty list and wrote its blocks without reading, which made relate cheaper. A review of that build found that a count record lost with its entries left behind then made the next relation overwrite the tail block and lose the entries the list held, where the build before the change rebuilt the count from them. The seek came back (`adj_count::count_entries`, ADR-0026), as `rel_counts.rs` pins. A cheaper proof that a list is empty (a point read of its tail) was considered and not taken: a list can have bounded blocks and no tail (the tail was emptied and deleted), and the invariant that would rule that out costs a promotion of the last bounded block on every emptied tail.
- **`all relations of a type` did not move** (1.11x to 1.10x at 10k, 0.96x to 0.95x at 100k).
- **Rows this change does not touch** (plain batch store, point read, scan, update, history, substring): read the 10k and 100k `.md` files in the raw directory; they moved within the range of the repetitions. The substring-indexed store went 4.16x to 4.47x at 10k and 3.13x to 2.87x at 100k; Kerno's own time went 0.44 s to 0.431 s and 5.43 s to 5.10 s, and the 10k ratio moved with SQLite's time (0.106 s to 0.0964 s); the shared `blocks::flush` change (the tail point read) does not change its bytes.
- **Other gains in the same rows:** `Db::related` reads the head of the unit instead of loading and hashing its payload (`UnitSnapshot::unit_exists`); the keyword statistics record is written once per change set instead of once per unit.

## NOT ESTABLISHED

- **Profiles with few samples:** point read (97), reopen (23), and the first traversal run (32, superseded by the 2363-sample run). Point read and reopen are indicative only.
- **Line-level causes:** the `stage_substring` self-cost (22.3%) and the update's read-before-write are attributed by symbol and code reading (inferred). Line-level confirmation would need a debug-info build, which the disk did not allow.
- **Expected gains (all ranks):** estimates from profile shares; none is measured.
- **Rust storage numbers on a quiet machine:** the Rust run overlapped other load (load 5.14 in the first run's `env.json`). Ratios are within-repetition; absolute times are not comparable across the two scales.
- **The rows of §5:** one sitting, load 3.59 to 4.74 at the start of the four runs; the machine was shared with other work, and the 100k relate gain in the Python run (4%) is inside the range of its repetitions. The alternated writer runs (`relate-alternated.txt`) are the firmer evidence for relate. The timer figures that located the relate cost were taken on a build with temporary timers and are the sizes of the parts, not a profile.
- **Whether note #1736 holds for plain batch stores:** the profile supports the per-record cost for keyword and substring stores, but the batch-store profile spreads the cost over more frames. It is not proven for the plain store.

## Reproduce

```
maturin build --release --manifest-path bindings/python/Cargo.toml -o <dir on /home>
<venv>/bin/pip install <wheel>
<venv>/bin/python3 bindings/python/bench/vs_sqlite.py --n 10000  --reps 3 --data <dir on /home> --out py-10000.json
<venv>/bin/python3 bindings/python/bench/vs_sqlite.py --n 100000 --reps 3 --data <dir on /home> --out py-100000.json
SCALES=10000,100000 REPS=5 OUT=<dir> DATA=<dir on /home> bash spikes/storage/run-sqlite-comparison.sh
<venv>/bin/py-spy record --native -r 250 -f raw -o X.raw -- <venv>/bin/python3 profiles/prof_driver.py <workload> 100000 <dir> <reps>
<venv>/bin/python3 profiles/analyze.py X.raw 15 "timed (prof_driver.py"
```

The profile workloads are listed in `profiles/run-prof.sh`. `prof_driver.py` imports `vs_sqlite` from the repo path, so run it from the repo checkout.
