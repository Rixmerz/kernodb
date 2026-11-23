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

**Rank 7. Keyword index: posting blocks for BM25 terms.** The same move as Rank 6 for keyword postings. Expected: keyword store 7.3x toward about 2x to 3x (estimate, by the trigram precedent). Risk: high; format change as in Rank 6. Effort: high. **Applied: see §5.**

**Rank 8. Point reads.** The storage lookups dominate, but the ratio is 1.01x at 10k and 1.49x at 100k, which is near noise. Gain small. Effort: medium to high. Not recommended now.

**Rank 9. Storage-level recovery (Rust harness, 16x to 73x).** Recovery replays the WAL (`recovery_ms`). Faster recovery needs more frequent checkpoints, which costs on the write path (Rank 5 trade-off). Not profiled in this run.

## 4. Benchmark fairness (not changed; these notes only)

- **Reopen is not like-for-like (unfair to both sides).** `vs_sqlite.py:247` times `kerno.open(kdir).close()`, which fsyncs the manifest. `vs_sqlite.py:248` times `sqlite3.connect(sfile).execute("select count(*) ...")` and never closes that connection. The kerno side includes a close and the SQLite side includes a count. Fix: Rank 1.
- **Two-hop is not one query on both sides (unfair to kerno).** `vs_sqlite.py:177-178` does `[[db.related(r.to_id) for r in db.related(ks[i])] ...]`, which is 1+N Python-level calls. SQLite does one join (`vs_sqlite.py:181`). Kerno's number therefore includes Python loop and FFI overhead. A server-side or single-call traversal would be a fair comparison, and it would be a feature.
- **Relate does more work on kerno (a real product difference, not a bench bug).** `create_relation_unguarded` (`db.rs:1706`) checks both endpoints (`require_active`, `db.rs:1666`), checks id collision, and writes three records plus counts. SQLite inserts one row and one index entry. The gap is the price of the validation and the adjacency design. It is not an artifact of the script.
- **Scan and point read return different objects (unfair to kerno).** Kerno builds `Unit` objects and verifies each payload's `content_hash` (in the scan profile). SQLite returns tuples without a checksum. The scan ratio includes that work.
- **Close compares different work (both ways).** Kerno's close persists the memtable, which is why its reopen and recovery are cheap. SQLite's close after per-commit fsync is nearly free. Report both, as the Rust table does.
- **Rust harness is a key-value contract (not the units layer).** `sqlite-benchmark.md` §1 says so. The SQLite side is one `WITHOUT ROWID` table. It is equal for the `sync` pair only.
- **Interpreter change.** 3.14 here, 3.12 in the 2026-11-22 run. Both sides in one run use the same interpreter, so the ratios hold; the absolute times do not compare with earlier documents.

## 5. Ranks 6 and 7 applied: posting blocks for adjacency and keyword postings

Change: [ADR-0026](../adr/0026-posting-blocks-for-adjacency-and-keyword-postings.md), layer version 9 ([units-v9](../file-format/units-v9.md)). Method: [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), the same script as §1 (`bindings/python/bench/vs_sqlite.py`, 3 repetitions, a fresh process per repetition, data on `/home`, Python 3.14.9, SQLite 3.54.0). "Before" is a wheel built from `f7a5004`, "after" a wheel of this change, both run in the same sitting. Raw data and the load average at each start: [`raw/2026-11-23-blocks/`](raw/2026-11-23-blocks/). Kerno / SQLite ratio (median of 3), lower is better; seconds are Kerno's.

| Row | 10k before | 10k after | 10k seconds, before to after | 100k before | 100k after |
|---|---|---|---|---|---|
| store keyword-indexed, batches (Rank 7) | 7.04 | **3.66** | 0.607 to 0.310 | 6.51 | 5.32 |
| keyword search x100, top 10 (Rank 7) | 0.36 | **0.15** | 0.00851 to 0.00355 | 0.73 | 0.19 |
| relate, batches (Rank 6) | 7.19 | **4.94** | 0.148 to 0.101 | 22.97 | 10.08 |
| `related()` x1000 (Rank 6) | 2.56 | 2.41 | 0.0102 to 0.00969 | 6.19 | 5.98 |
| two-hop traversal x200 (Rank 6) | 4.94 | **3.83** | 0.00511 to 0.00403 | 14.74 | 11.01 |
| all relations of a type | 1.13 | 1.28 | 0.00607 to 0.0071 | 1.04 | 1.53 |
| close | 103 | 63 | 0.141 to 0.0884 | 46 | 68 |
| reopen | 15.0 | 12.8 | 0.00653 to 0.00662 | 6.6 | 0.78 |
| disk size | 1.83 | 1.72 | 31 to 29.2 MiB | 1.85 | 1.79 |

- **Rank 7 reached about what the trigram precedent promised at 10k** (keyword store 7.04x to 3.66x; expected 2x to 3x), and keyword search got faster again (0.36x to 0.15x of SQLite).
- **Rank 6 helped the writes and traversals only part way.** Relate went 7.19x to 4.94x and two-hop 4.94x to 3.83x, but `related()` is flat (2.56x to 2.41x). The expected 2x to 3x was not reached: each neighbour still costs a `REL` read, a head read of the other end and a mirror-entry seek (ADR-0026 option 4 would remove those and was not taken).
- **`all relations of a type` is worse at 10k** (0.00607 s to 0.0071 s, ranges 0.00602 to 0.00624 against 0.00666 to 0.00758, so a real ~17%). Not profiled.
- **The 100k columns are indicative only.** The load average at the start of the 100k "after" run was 28.9 (14.0 for the 100k "before" run, 4.5 to 5.6 for both 10k runs) and both engines, SQLite included, ran several times slower than at 10k. The 100k reopen and close values are within that noise (SQLite's reopen range spans 0.034 to 0.266 s). Read the 10k columns as the result and the 100k ones as the direction.
- **Rows this change does not touch** (plain batch store, point read, scan, update, history, substring) moved by at most 0.1x between the two 10k runs; that is the noise of the method.
- **Other gains in the same rows:** `Db::related` reads the head of the unit instead of loading and hashing its payload (`UnitSnapshot::unit_exists`); the keyword statistics record is written once per change set instead of once per unit.

## NOT ESTABLISHED

- **Profiles with few samples:** point read (97), reopen (23), and the first traversal run (32, superseded by the 2363-sample run). Point read and reopen are indicative only.
- **Line-level causes:** the `stage_substring` self-cost (22.3%) and the update's read-before-write are attributed by symbol and code reading (inferred). Line-level confirmation would need a debug-info build, which the disk did not allow.
- **Expected gains (all ranks):** estimates from profile shares; none is measured.
- **Rust storage numbers on a quiet machine:** the Rust run overlapped other load (load 5.14 in the first run's `env.json`). Ratios are within-repetition; absolute times are not comparable across the two scales.
- **The 100k rows of §5:** the "after" run overlapped heavy machine load (28.9 at its start), so only the 10k columns are a fair before/after.
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
