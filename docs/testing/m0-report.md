# M0 report — storage spike

Status: results of 2026-10-27 · Related to: [ADR-0001](../adr/0001-storage-engine.md), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), [test plan](test-plan.md), [failure matrix](../architecture/failure-matrix.md)

This report gathers the evidence on which ADR-0001 is decided: first the **correctness** (which is a requirement, not a tie-breaker) and then the **performance**. The benchmarks are reproducible from the commit indicated in `env.json` (`df45ddb`); the correctness tests, from `a7d40d7` (the 1000-seed pass ran on `c8b30f4`, with the same code).

## 1. What was compared

| Engine | Architecture | Role |
|---|---|---|
| `btree` | own copy-on-write B+ tree ([design](../../spikes/storage/spike-btree/DESIGN.md)) | candidate A |
| `lsm` | own leveled LSM ([design](../../spikes/storage/spike-lsm/DESIGN.md)) | candidate B |
| `redb` 4.1 | mature copy-on-write B+ tree | reference for family A |
| `fjall` 3.1 | mature LSM | reference for family B |
| SQLite 3 (rusqlite 0.41, WAL) | B-tree + WAL | general reference |

The two candidates implement the same contract (`spike_core::Engine`) and pass the same shared suite, with one declared exception: the B+ tree does not promise a consistent prefix in `Buffered` under a power cut, and does not run that variant (§2.3). The unit and regression tests belong to each engine. The reference engines take part only in the benchmarks and in the differential test.

## 2. Correctness

### 2.1 Final result

Run with `SPIKE_SEEDS=400 cargo test --workspace --all-features --release` on commit `a7d40d7` (full log in [`tests-seeds400.log`](../../spikes/storage/results/2026-10-27/tests-seeds400.log)):

| Suite (shared unless stated) | B+ tree | LSM |
|---|---|---|
| Functional (14 tests: random model, clean and dirty reopens, isolation under churn, 4 concurrent readers, limits, space, real FS) | ✅ | ✅ |
| Crashes at random points (`crash::run`: 400 seeds × 5 rounds, including crashes during recovery and byte-level tearing in half of the seeds) | ✅ 3 variants (without `Buffered` + power cut) | ✅ 4 variants |
| Crash before **each** I/O operation of the fixed scenario (power cut and process death in `Sync`, process death in `Buffered`; after recovering, one more commit, close and power cut) | ✅ | ✅ |
| One isolated EIO at **each** I/O operation (restart and immediate cut) | ✅ | ✅ |
| `Buffered` + power cut = consistent prefix | ⚠️ not guaranteed (see §2.3) | ✅ |
| Bit flips after a clean close (400; half aimed at headers/metas/footers) | ✅ 28 detected on open · 191 on read · 181 harmless · **0 wrong data · 0 rollbacks** | ✅ 36 on open · 364 on read · 0 harmless · **0 wrong data · 0 rollbacks** |
| Truncations after a clean close (400; half aimed at headers/metas/footers, ¼ to 0 bytes) | ✅ 400/400 detected on open | ✅ 400/400 detected on open |
| Own unit tests | ✅ 32 | ✅ 24 unit + 7 integration |
| Regressions from the adversarial review and from the audit before merge | ✅ 9 | ✅ 7 |
| Differential test (same history in btree, lsm, LogEngine, redb, fjall, SQLite) | ✅ | ✅ |

Additional stress pass with `SPIKE_SEEDS=1000` over the shared suites of both candidates, on commit `c8b30f4` (same engine code as `a7d40d7`; only the report and the log change) ([`tests-seeds1000.log`](../../spikes/storage/results/2026-10-27/tests-seeds1000.log)): all green. Bit flips: B+ tree 79 detected on open · 451 on read · 470 harmless; LSM 122 · 878 · 0. Truncations: 1000/1000 detected on open in both. No wrong data and no rollback.

The B+ tree's "harmless" bytes are bytes that the engine never reads (free pages, meta padding, the second copy of the meta when the first one is intact). The LSM has no uncovered bytes: all damage is detected.

### 2.2 Defects found and how they were found

M0 did not only measure: the verification process found and fixed real defects in both candidates. This is evidence in itself of where the complexity of each architecture lies.

**During implementation (by the harness):**

| Engine | Defect | Detected by |
|---|---|---|
| B+ tree | Meta slot chosen by parity of the sequence: after losing one slot, the next commit overwrote the only valid meta | own unit test |
| B+ tree | Directory synced only if the file had "just been created" | `single_io_failure_at_every_point` |
| LSM | If the MANIFEST `rename` succeeded but its directory `fsync` failed, outputs that the surviving MANIFEST could reference were deleted | the implementer's own review |
| reference | Same directory defect; corruption in the middle of the log treated as a broken tail | failure and corruption suites |

**Through the adversarial review** (5 reviewers per engine, one per failure class; each finding was verified by 2 independent skeptics with a reproduction test; all verified findings were **confirmed and reproduced**):

| Engine | Severity | Defect | Status |
|---|---|---|---|
| B+ tree | **high** | Opening treated as durable a meta that was only visible (process dead between writing it and syncing it, or failed `fsync`); its freed pages —the previous tree, the only durable one— were reused, and a restart followed by a power cut during the next commit could destroy the database (~30% of the simulated runs) | fixed (`fsync` on open) + regression |
| B+ tree | medium | A damaged bit in the most recent meta after a dirty close silently rolled back to the previous commit | fixed (two copies of the meta per slot) + regression |
| B+ tree | medium | A panic inside the commit did not poison the handle; later commits left the database unable to open | fixed |
| B+ tree | medium (performance) | Values larger than ~1 KiB were never cached: one `pread` per 4 KiB page on every read | fixed (unified cache) |
| B+ tree | low | Panic on a file crafted with valid CRCs; sequence overflow at `u64::MAX` | fixed + regressions |
| LSM | **medium-high** | Damage in the middle of the most recent WAL after a dirty close was treated as a broken tail: commits confirmed in `Sync` mode were silently lost and the WAL was deleted | fixed (`SYNCED` flag per record) + regression |
| LSM | low (performance: medium) | `close()` abandoned the compaction in progress and added another L0 file: L0 grew without limit with short sessions, and each reopen started with backlogged work | fixed (compacts on close) + regression |
| LSM | low | Batches > 4 GiB confirmed and then unreadable; race with a snapshot of an earlier handle that deletes an orphan SST; non-canonical names; unchecked arithmetic with crafted files; double block read in `seek` | fixed |
| both | medium | The database directory was never synced in its parent directory: a power cut could make a new database disappear with everything confirmed in it | fixed in both + regressions |

Low-severity performance findings that were not verified (agent quota exhausted) remain as known limitations: contention on global atomic counters in concurrent reads (LSM); `fdatasync` of a WAL that grows on every commit, without preallocation (LSM); extra copies of pages in very large atomic batches (B+ tree).

**Defects of the harness itself** found by the candidates and fixed: two races with background work (a crash trigger that survived recovery; `dir_size` against concurrent deletions) and two coverage gaps (uniform damage that almost never reached metas and headers; tearing only at 512 B, which made small writes atomic). Each gap was then validated by reintroducing the defect that got through ([test plan §2.1](test-plan.md#21-harness-validation-manual-mutation-testing)).

### 2.3 Architectural difference observed in durability

In `Buffered` mode (no `fsync` per commit) and under a power cut:

- **LSM**: always a consistent prefix of commits. It follows from the design itself: the old WAL is synced before a new one is created, and replay stops at the first gap.
- **CoW B+ tree**: never wrong data (the Merkle checksums prevent it), but **not** a prefix: in simulation, ~87% of the cuts (130 of 150) left the database unable to open, and another 12% opened with damage detected on read, because a new meta can reach the disk before its pages, and freed pages are reused before they are durable. Offering the guarantee requires periodic sync points and deferred reuse (design sketched in the prototype). <a id="3-rendimiento"></a>

## 3. Performance

### 3.1 Environment and method

| | |
|---|---|
| Machine | VM, Intel Xeon 2.80 GHz, 4 cores, 15.7 GiB, ext4, kernel 6.18 |
| `fdatasync` of 4 KiB | p50 113 µs · p95 400 µs · p99 568 µs (the floor of any durable commit here) |
| Build | `rustc 1.97.0`, `release` profile, commit `df45ddb` |
| Method | [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md): a new process and a new directory per run, 3 interleaved repetitions (1 in `large`), median (range in parentheses if it exceeds 15%), 64 MiB cache, no compression, equivalent durability; in the workloads that reopen the database before measuring, with a warm cache, a full scan of the database before measuring (warm-up outside the measurement) |
| Scales | `small` = 20 000 records, `medium` = 200 000, `large` = 1 000 000 (bulk load, point reads, scans and small commits); the cold-cache reads empty the OS page cache before reopening (applied in all those runs) |
| Data | 410 runs, 0 failures: [`results.jsonl`](../../spikes/storage/results/2026-10-27/results.jsonl), [full tables](../../spikes/storage/results/2026-10-27/report.md), [environment](../../spikes/storage/results/2026-10-27/env.json) |
| Reproduction | `spikes/storage/run-benchmarks.sh` |

† In the tables, redb in `Buffered` mode uses `Durability::None`, which does not survive process death: its `Buffered` figures are **not comparable** with those of the other engines (ADR-0012).

### 3.2 Reference cases (scale `medium` unless stated)

| Case | Metric | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| **Cauce** (`Sync`) | steps/s (durable commit + reads) | 1 111 | **3 087** | 2.8× | 1 748 | 2 941 | 2 187 |
| | p99 commit (µs) | 1 966 | **713** | | 1 213 | 658 | 1 078 |
| | p99 read (µs) | **76** | 98 | | 59 | 118 | 151 |
| | recovery after dirty close (ms) | **0.9** | 9.7 | | 1.1 | 363 | 0.7 |
| **Cauce** (`Buffered`, `small`) | steps/s | 16 250 | **57 110** | 3.5× | 15 680 † | 61 490 | 7 413 |
| **LiveSpec** | change sets/s | 408 | **667** | 1.6× | 524 | 522 | 468 |
| | p99 commit (µs) | **3 762** | 4 764 | | 3 148 | 2 425 | 12 530 |
| | p99 module listing (µs) | **440** | 657 | | 377 | 1 296 | 346 |
| **Agent memory** | steps/s | **1 073** | 1 037 | 0.97× | 1 098 | 1 279 | 704 |
| | steps/s (`small`) | 1 377 | **1 642** | 1.19× | 1 595 | 2 166 | 832 |
| | p99 memory write (µs) | **1 573** | 5 837 | | 2 128 | 1 090 | 1 613 |
| | p99 memory recall (µs) | **49** | 100 | | 46 | 73 | 121 |
| | space / live data | 3.93 | **2.64** | | 5.04 | 3.14 | 4.43 |
| **Simulated history** | revisions/s (thousands) | 8.3 | **66.6** | 8.0× | 11.4 | 61.6 | 11.4 |
| | p99 commit of 100 revisions (µs) | 17 870 | **9 118** | | 13 480 | 3 168 | 26 800 |
| | p50 full history of one record (µs) | **7.6** | 14.4 | | 5.9 | 20.3 | 9.8 |
| | p99 rebuild one revision (µs) | **6.4** | 8.7 | | 4.5 | 32.6 | 7.7 |
| | space / data (`medium` · `small`) | 2.04 · **2.32** | **1.88** · 2.98 | | 3.42 · 4.28 | 3.64 · 22.9 | 1.74 · 3.32 |

### 3.3 Micro-benchmarks (scale `medium` unless stated)

| Workload | Metric | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| Single-record commits (`Sync`) | commits/s | 1 542 | **4 044** | 2.6× | 2 499 | 4 054 | 4 113 |
| | p99 commit (µs) | 1 331 | **584** | | 951 | 616 | 630 |
| | `fsync` per commit | 2 | **1** | | — | — | — |
| | write amplification | 112× | **1.1×** | | 132× | 1.2× | 37× |
| | commits/s (`large`, 1 M) | 1 554 | **4 069** | 2.6× | 3 138 | 4 326 | 4 460 |
| Single-record commits (`Buffered`, `small`) | commits/s | 31 080 | **200 800** | 6.5× | 18 770 † | 311 600 | 27 760 |
| Point deletes (`Sync`) | commits/s | 1 365 | **4 343** | 3.2× | 2 480 | 3 991 | 3 202 |
| | p50 read of deleted key · p99 of live key (µs) | 3.4 · 21.5 | **0.7 · 15.5** | | 1.9 · 15.0 | 1.8 · 10.6 | 7.6 · 38.8 |
| Random bulk load | kops/s | 20.1 | **162** | 8.0× | 26.2 | 133 | 12.6 |
| | kops/s (`large`) | 10.4 | **110** | 10.6× | 18.5 | 131 | 10.3 |
| | write amplification | 10.1× | **3.4×** | | 10.6× | 1.6× | 36.6× |
| | space / data (closed) | 1.65 | **1.03** | | 2.36 | 1.58 | 1.48 |
| | peak RAM (MB) | 89 | **25** | | 72 | 136 | 40 |
| | clean close (ms; `large`) | **12; 22** | 759; 3 587 | | 8.7; 13 | 96; 919 | 13; 14 |
| | clean reopen (ms; `large`) | **1.0; 1.3** | 3.4; 10.1 | | 1.2; 0.9 | 392; 292 | 0.5; 0.6 |
| Point reads, warm cache | kops/s, 1 thread / 4 threads | 188 / 370 | **230 / 620** | 1.2× / 1.7× | 273 / 365 | 235 / 773 | 138 / 441 |
| | p50 · p99, 1 thread (µs) | 4.8 · 12.5 | **3.2** · 12.5 | | 3.7 · 9.4 | 3.5 · 9.0 | 6.3 · 21.3 |
| | kops/s, 1 thread (`large`) | 135 | **163** | 1.2× | 176 | 117 | 101 |
| Point reads, **cold cache** | kops/s, 1 thread · p99 (µs) | 56.7 · 117 | **117 · 86** | 2.1× | 64.9 · 113 | 143 · 82 | 52.0 · 182 |
| Scan of 100, warm cache | p50 · p99 (µs) | 82 · 171 | **58 · 146** | | 84 · 182 | 73 · 140 | 73 · 167 |
| | p50 · p99 (µs), `small` | **17 · 58** | 23 · 72 | | 25 · 84 | 65 · 132 | 50 · 128 |
| | p50 · p99 (µs), `large` | 91 · 189 | **68 · 128** | | 99 · 176 | 105 · 164 | 77 · 140 |
| Scan of 100, **cold cache** | p50 · p99 (µs) | 103 · 1 758 | **84 · 521** | | 90 · 1 652 | 75 · 310 | 74 · 1 745 |
| Mixed 90/10, 4 threads | kops/s | 11.6 | **28.0** | 2.4× | 15.3 | 30.3 | 25.3 |
| | p99 read · p99 write (µs) | 84 · 7 274 | **62 · 3 817** | | 128 · 6 488 | 45 · 3 504 | 237 · 3 916 |
| | p99 read (µs), `small` | **12** | 31 | | 28 | 24 | 177 |
| Sequential insert + updates (batches of 100) | kops/s insert · update | 102 · 93 | **135 · 115** | 1.3× · 1.2× | 77 · 111 | 140 · 86 | 118 · 121 |
| | p99 commit of insert · of update (µs) | **1 947 · 2 306** | 4 139 · 6 840 | | 3 103 · 1 732 | 1 504 · 2 634 | 7 999 · 7 999 |
| | write amplification · space / data | **2.3× · 1.62** | 3.4× · 2.08 | | 2.9× · 2.11 | 1.7× · 3.24 | 3.2× · 1.34 |

### 3.4 Reading the results

1. **The LSM reproduces the fjall profile; the B+ tree stays below redb except in sequential insert.** In `medium` and `Sync` mode, the LSM prototype performs between 0.8× and 1.3× fjall in all the reference cases and write micro-benchmarks (without `fsync`, at scale `small`, fjall is faster: 1.6× in single-record commits). The B+ tree prototype performs between 0.55× and 0.8× redb in Cauce, LiveSpec, single-record commits, deletes, mixed, history, bulk load and point reads with one thread (partly because of its two `fsync` per commit versus one), on a par in agent memory and in reads with 4 threads, and above it in sequential insert (1.3×: 102 versus 77 kops/s). That is why the LSM / B+ ratios also include maturity differences. **Among the mature engines (fjall versus redb) the advantage of the LSM architecture holds and is in general smaller**: a tie in LiveSpec (1.0×), 1.6× in single-record commits and in deletes, 1.7× in Cauce, 2.0× in the mixed workload and 5.1–5.4× in bulk load and history writes; only in agent memory is it larger than between the prototypes (1.2× versus 0.97×); redb wins in sequential updates (fjall 0.78×) and in point reads with one thread (0.86×).
2. **Write performance: the LSM wins almost everywhere, with very different margins.** In the reference cases, from a tie in agent memory (0.97× in `medium`, 1.19× in `small`) to 1.6× in LiveSpec, 2.8× in Cauce and 8.0× when writing history. In micro-benchmarks, from 1.2–1.3× in sequential insert to 2.6× in durable single-record commits, 3.2× in deletes, 6.5× in commits without `fsync` and 8–10.6× in random bulk load. The factor that weighs most: the B+ tree does two `fsync` per commit (redb, one) and rewrites the root-to-leaf path of each leaf it touches; the LSM does one `fsync` and one sequential write.
3. **Bytes written and space: almost always in favour of the LSM, with exceptions.** It writes ~100× less per small commit (1.1× versus 112×) and occupies less after a load (1.03× versus 1.65×) and in agent memory at `medium` (2.64× versus 3.93×). But in sequential insert with updates it occupies more (2.08× versus 1.62× space) and, at `medium`, it also writes more (3.4× versus 2.3× amplification); and at scale `small` it occupies more in history (2.98× versus 2.32×) and in agent memory (2.03× versus 1.64×), because of versions not yet compacted.
4. **Tail latency: it depends on the workload; neither wins everywhere.**
   - The **LSM** wins on small durable commits: Cauce p99 2.8× lower, single-record commits 2.3× lower, mixed-workload write 1.9× lower, history 2.0× lower and deletes 3.2× lower.
   - The **B+ tree** wins when LSM compaction coincides with sustained writes or reads: at `medium`, agent-memory write p99 3.7× lower, sequential insert 2.1× lower, sequential updates 3.0× lower and LiveSpec commit 1.3× lower (at `small`, with little compaction, the LSM wins those four tails: 1.1×, 3.5×, 1.4× and 1.25× lower); and in short reads of small ranges (memory recall 2.0×, module listing 1.5×, Cauce read 1.3×).
   - It is the main weak point of the LSM and the first condition of ADR-0001.
5. **Reads with a warm cache: similar.** The LSM is somewhat faster in point reads (1.2× with 1 thread, 1.7× with 4) and in scans from `medium` onward; the B+ tree wins in scans at `small` scale and in the history of one record (1.9×, a contiguous range inside one leaf).
6. **Reads with a cold cache: the LSM wins** (2.1× in point reads; scan p99 3.4× lower). Copy-on-write allocation scatters the pages across the file; the LSM's SSTs are sequential.
7. **Opening, recovery and close: the B+ tree wins** (1–2 ms versus 3–18 ms to open with a warm cache at `medium` and `large`, and 3–4 ms versus 8–12 ms with a cold cache; 12 ms versus 759 ms to close at `medium` and 22 ms versus 3.6 s at `large`, because the LSM compacts before closing). Opening in milliseconds is irrelevant for an embedded database; the LSM close will be bounded.
8. **Memory:** the LSM uses much less RAM when loading (25 MB versus 89 MB) and in sequential inserts, but somewhat more in agent memory (94 versus 77 MB) and in LiveSpec (86 versus 74 MB) because of its memtables. In point reads with a warm cache the peak RAM is similar across the four Rust engines (139–163 MB) and lower in SQLite (66 MB).

### 3.5 Measurement limitations

- A single virtual machine with virtualised storage: `fdatasync` is cheap (p50 ~0.1 ms, p99 ~0.6 ms). On a disk with a more expensive `fsync`, the LSM advantage in durable commits (1 versus 2 `fsync`) grows; on a cheaper one, it shrinks. Repeat on the target hardware (R-S3).
- `wchar` measures bytes handed to write calls, not bytes on the device.
- The `large` scale was run only once and only for four workloads; its figures indicate a trend, not a median.
- The VM's CPU shows as 2.80 GHz in this run and as 2.10 GHz in an earlier run on the same day: the VM may move between hosts across sessions, but within one run all combinations share the machine and interleave engines.
- redb in `Buffered` mode uses `Durability::None`, which does not survive process death: its `Buffered` figures are not equivalent (†).

## 4. Conclusion

Both architectures are implementable with the required guarantees; neither wins on correctness. In Kerno's reference cases —write-intensive, with history and with durability per commit— the **LSM** performs between a tie (agent memory) and 8× more (history writes), with 2.8× in Cauce and 1.6× in LiveSpec; part of that advantage comes from the maturity of the B+ tree prototype, but among the mature engines (fjall versus redb) it holds: from a tie in LiveSpec to 1.7× in Cauce and 5.4× in history; it writes ~100× fewer bytes per small commit, occupies less in most cases, reads better with a cold cache and offers a consistent `Buffered` mode. The **B+ tree** has better tails when LSM compaction competes with sustained writes or short reads, faster history reads, immediate opening and closing, and a simpler operation. The recommendation, its conditions and the decisions that depend on it are in [ADR-0001](../adr/0001-storage-engine.md).
