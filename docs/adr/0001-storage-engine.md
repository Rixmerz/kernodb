# ADR-0001 — Physical storage engine

| Field | Value |
|---|---|
| Status | **Accepted** (2026-10-28, by the project owner) — with the evidence of M0 (410 benchmark runs, failure suites with 400 and 1000 seeds) and the conditions of the decision as mandatory work for Phase 1 |
| Date | 2026-10-27 (proposal) · 2026-10-28 (acceptance) |
| Related to | SAD §7.1, §15, §17; ADR-0002, ADR-0003, ADR-0004, ADR-0009, ADR-0012; [M0 report](../testing/m0-report.md); [failure matrix](../architecture/failure-matrix.md) |

## Context

Kerno needs a transactional, embedded and local-first key-value core on which to build documents, revisions, indexes and recovery for AI. The physical architecture conditions the persistent format, the commit protocol, recovery, the cost of history and data suppression, and changing it later would be a migration, not a refactoring. That is why SAD §7.1 and D-11 require choosing between a **copy-on-write B+ tree** and an **LSM tree** with a reproducible test, not out of popularity or third-party figures.

## Options considered

1. **A — Copy-on-write B+ tree** (shadow paging, in the style of LMDB/redb): a file of pages, double meta, MVCC through page retention, no background threads.
2. **B — Leveled LSM** (in the style of LevelDB/RocksDB): WAL + memtable + immutable SSTs + background compaction, MVCC through file versions.
3. **C — Third way** (adopt an existing engine as the storage layer, or a hybrid with two primary engines): only if the evidence justified it. SAD §15 frames ADR-0001 as an own implementation of one of the two approaches and admits a third option "only if the evidence justifies it"; the principles of a small core and of measuring before specializing (SAD §3, principles 2 and 8) and Kerno's master plan rule out two primary engines in the first version.

## Decision criteria (set before measuring)

From SAD §15 and §17.5: correctness; p50/p95/p99 latencies; throughput; recovery time; maximum RAM; bytes written and disk size against data; operational complexity; behaviour in recovery, compaction and ranges; future compatibility with the Revision Engine. Methodology in ADR-0012. The reference cases (LiveSpec, Cauce, agent memory) weigh more than micro-benchmarks.

## Evidence

All the detail is in the [M0 report](../testing/m0-report.md); here, what decides.

### 1. Correctness: a requirement met by both

Both prototypes pass, with 400 seeds, the same shared battery (the B+ tree, without the `Buffered` + power-loss variant, whose guarantee it does not offer; §2): a random model, isolation, concurrency, crashes at random points and at **each** I/O operation (process death and power loss, with tearing at 512 B and at byte level), an isolated EIO on each operation, targeted bit flips and truncations, and a differential test against redb, fjall and SQLite. Neither ever returned wrong data or silently rolled back.

Getting there required fixing real defects in both, found by the harness and by an adversarial review with independent reproduction: in the B+ tree, one of **high** severity (opening trusted a non-durable meta and could destroy the only durable tree) and three medium; in the LSM, one **medium-high** (damage in the middle of the WAL taken as a broken tail, with silent loss of confirmed commits) and one medium. **Correctness does not break the tie**: both architectures have protocols with subtleties, although in different places (meta and free-page invariants in the B+ tree; file lifecycle and concurrency with background threads in the LSM).

### 2. Durability without `fsync` per commit

| | CoW B+ tree | LSM |
|---|---|---|
| Power loss in `Buffered` mode | exact state of some commit **or** detected corruption; ~87 % of the simulated cuts left the database **unable to open** (and another 12 %, with damage detected on read) | **consistent prefix** of commits, always |

Offering the same guarantee in a CoW requires synchronization points and deferred reuse of pages (sketched design, not implemented). In the LSM it comes from the WAL's own design. Kerno wants to offer a relaxed durability mode without compromising consistency (ADR-0002), so this weighs.

### 3. Performance (`medium` scale, 200 000 records, `Sync` unless noted, median of 3 repetitions; `large` figures are single runs)

| Workload | Metric | B+ tree | LSM | LSM / B+ | redb | fjall | SQLite |
|---|---|---:|---:|---:|---:|---:|---:|
| **Cauce** (durable commit + reads) | steps/s | 1 111 | **3 087** | 2.8× | 1 748 | 2 941 | 2 187 |
| | p99 commit · p99 read (µs) | 1 966 · **76** | **713** · 98 | | 1 213 · 59 | 658 · 118 | 1 078 · 151 |
| | recovery after dirty close (ms) | **0.9** | 9.7 | | 1.1 | 363 | 0.7 |
| **Cauce** `Buffered` (`small`) | steps/s | 16 250 | **57 110** | 3.5× | 15 680 † | 61 490 | 7 413 |
| **LiveSpec** | change sets/s | 408 | **667** | 1.6× | 524 | 522 | 468 |
| | p99 commit · p99 module listing (µs) | **3 762 · 440** | 4 764 · 657 | | 3 148 · 377 | 2 425 · 1 296 | 12 530 · 346 |
| **Agent memory** | steps/s (`medium` · `small`) | **1 073** · 1 377 | 1 037 · **1 642** | 0.97× · 1.19× | 1 098 · 1 595 | 1 279 · 2 166 | 704 · 832 |
| | p99 write (µs) | **1 573** | 5 837 | | 2 128 | 1 090 | 1 613 |
| **Simulated history** (write) | revisions/s (thousands) | 8.3 | **66.6** | 8.0× | 11.4 | 61.6 | 11.4 |
| | p99 commit of 100 revisions (µs) | 17 870 | **9 118** | | 13 480 | 3 168 | 26 800 |
| **Simulated history** (read) | p50 history of a record (µs) | **7.6** | 14.4 | | 5.9 | 20.3 | 9.8 |
| Commits of one record | commits/s | 1 542 | **4 044** | 2.6× | 2 499 | 4 054 | 4 113 |
| | p99 commit (µs) | 1 331 | **584** | | 951 | 616 | 630 |
| | `fsync` per commit | 2 | **1** | | — | — | — |
| | write amplification | 112× | **1.1×** | | 132× | 1.2× | 37× |
| Commits of one record `Buffered` (`small`) | commits/s | 31 080 | **200 800** | 6.5× | 18 770 † | 311 600 | 27 760 |
| Point deletes | commits/s | 1 365 | **4 343** | 3.2× | 2 480 | 3 991 | 3 202 |
| Bulk load | kops/s (`medium` · `large`) | 20.1 · 10.4 | **162 · 110** | 8.0× · 10.6× | 26.2 · 18.5 | 133 · 131 | 12.6 · 10.3 |
| | disk space / data | 1.65 | **1.03** | | 2.36 | 1.58 | 1.48 |
| | maximum RAM (MB) | 89 | **25** | | 72 | 136 | 40 |
| Mixed 90/10 (4 threads) | kops/s | 11.6 | **28.0** | 2.4× | 15.3 | 30.3 | 25.3 |
| | p99 read · p99 write (µs) | 84 · 7 274 | **62 · 3 817** | | 128 · 6 488 | 45 · 3 504 | 237 · 3 916 |
| Point reads, warm cache | kops/s 1 thread · p99 (µs) | 188 · 12.5 | **230** · 12.5 | 1.2× | 273 · 9.4 | 235 · 9.0 | 138 · 21.3 |
| Point reads, cold cache | kops/s | 57 | **117** | 2.1× | 65 | 143 | 52 |
| Scan of 100, warm / cold cache | p99 (µs) | 171 / 1 758 | **146 / 521** | | 182 / 1 652 | 140 / 310 | 167 / 1 745 |
| Sequential insert + updates | kops/s update | 93 | **115** | 1.2× | 111 | 86 | 121 |
| | p99 commit of 100 updates (µs) | **2 306** | 6 840 | | 1 732 | 2 634 | 7 999 |
| | write amplification · space / data | **2.3× · 1.62** | 3.4× · 2.08 | | 2.9× · 2.11 | 1.7× · 3.24 | 3.2× · 1.34 |
| Bulk load: clean reopen · clean close | ms | **1.0 · 12** | 3.4 · 759 | | 1.2 · 8.7 | 392 · 96 | 0.5 · 13 |

† redb `Durability::None` does not survive process death: figure not comparable (ADR-0012).

Environment: VM with 4 cores and `fdatasync` p50 of 113 µs (p99 568 µs), 410 runs without failures; detail, ranges and the `small` and `large` scales in the [M0 report §3](../testing/m0-report.md#3-performance).

Reading the data:

- **The LSM reproduces fjall's profile; the B+ tree stays below redb except in sequential insertion.** At `medium` and `Sync` mode, the LSM prototype performs between 0.8× and 1.3× fjall; the B+ tree, between 0.55× and 0.8× redb in Cauce, LiveSpec, commits of one record, deletes, mixed, history and bulk load (partly because of its two `fsync`s per commit versus one), level in agent memory and above in sequential insertion (1.3×). That is why the LSM / B+ ratios also include maturity differences (risk R-S1). **Among the mature engines (fjall against redb) the LSM architecture's advantage holds and is generally smaller**: a tie in LiveSpec (1.0×), 1.6× in commits of one record and in deletes, 1.7× in Cauce, 2.0× in the mixed workload and 5.1–5.4× in bulk load and history writes; only in agent memory is it larger than among the prototypes (1.2× against 0.97×); redb wins in sequential updates (fjall 0.78×) and in point reads with one thread (0.86×).
- **Write performance: the LSM wins almost everywhere, with very different margins.** In the reference cases it ranges from a tie (agent memory: 0.97× at `medium`, 1.19× at `small`, with values of 1–4 KiB and 3 KiB embeddings) to 1.6× (LiveSpec), 2.8× (Cauce) and 8.0× (history writes). In micro-benchmarks, from 1.2–1.3× (sequential insertion) to 2.6× (durable commits of one record), 6.5× (commits without `fsync`) and 8–10.6× (random bulk load). In a CoW every commit rewrites the root-to-leaf path of each leaf touched plus metadata; with two `fsync`s (prototype) or one (redb), the ceiling of durable commits stays below the LSM's.
- **Bytes written and space: in favour of the LSM except in sequential writing and at `small` scale.** It writes ~100× fewer bytes per small commit and occupies less after a load and in agent memory at `medium` (2.64 against 3.93); with sequential inserts and updates it occupies more (2.08 against 1.62) and, at `medium`, it also writes more (3.4× against 2.3×); at `small` scale it occupies more in history (2.98 against 2.32) and in agent memory (2.03 against 1.64).
- **Tail latency: depends on the workload.** The LSM has better p99 in small durable commits (Cauce 2.8×, commits of one record 2.3×, write in the mixed workload 1.9×, history 2.0×, deletes 3.2×). The B+ tree has it at `medium` when the LSM's compaction coincides with sustained writing (agent memory 3.7×, sequential updates 3.0×, sequential insertion 2.1×, LiveSpec commit 1.3×; at `small`, with little compaction, the LSM wins those four tails) and in short reads of small ranges (module listing 1.5×, memory recall 2.0×, Cauce read 1.3×).
- **Reads with warm cache: similar**; the B+ tree wins in the history of one record (1.9×, a contiguous range within a leaf) and in scans at `small` scale, and the LSM in point reads and in scans from `medium` onwards. **With cold cache the LSM wins** (2.1× in point reads, scan p99 3.4× lower): copy-on-write allocation scatters the pages and the SSTs are sequential.
- **Opening and closing: the B+ tree wins** (reopening of 1–2 ms against 3–18 ms at `medium` and `large`; closing of 12 ms against 0.76 s at `medium` and of 22 ms against 3.6 s at `large`, because the LSM compacts before closing).
- **Memory:** the LSM uses much less RAM when loading (25 against 89 MB), but somewhat more in agent memory and LiveSpec, because of its memtables.

### 4. Qualitative criteria

| Criterion | CoW B+ tree | LSM |
|---|---|---|
| Operational complexity | **Lower**: no background threads, no compaction or tuning | Higher: compaction, backpressure, file lifecycle, per-level tuning |
| Recovery | **Trivial**: choose meta + `fsync`; no replay | WAL replay + rewrite to L0 + MANIFEST; re-entrant and tested |
| Compaction | None; the file does not shrink without offline compaction | Continuous; background write amplification; tombstones |
| Ranges | Direct leaf walk | Merge of several sources; needs bloom + indexes |
| Compatibility with Revision Engine | Each revision is one more random write (expensive: 8.0× fewer revisions/s than the LSM in the prototype, 5.4× redb against fjall; 112× bytes in commits of one record) | Revisions are sequential writes in WAL/memtable (cheap); history reads somewhat slower |
| Retention and suppression (ADR-0009) | Freed pages keep their bytes until reused; deleting ranges frees leaves | Old versions in SST/WAL until compaction and recycling; needs *range tombstones* and forced compaction |
| Encryption (ADR-0006) | New nonce per rewritten page | Immutable files: encrypted once |
| Long-lived snapshots | Retain pages (the file grows) | Retain files and versions (the directory grows) |

## Decision

**Adopt a leveled LSM as the storage architecture of Kerno's core**, starting from the design and tests of the `spike-lsm` prototype, with these conditions before the format is frozen (ADR-0003):

1. **Bounded read and write queues**: tune compaction (subcompactions or limits per round, overlap limits with the grandparent level), a compaction budget in `close()` and stall metrics; acceptance criterion with the p99 of LiveSpec, Cauce and agent memory against the M0 baseline (the write p99 in agent memory, 3.7× that of the B+ tree, is the case to solve).
2. **Range tombstones and targeted compaction** of ranges, a requirement of the retention and suppression of revisions (ADR-0004, ADR-0009).
3. **MANIFEST as a log of edits** with a clean-shutdown mark, and **group commit** for concurrent writers.
4. Keep **the B+ tree prototype and the whole harness** as differential oracle and as a read-performance reference.
5. **Review this decision** if a real use case turns out to be dominated by low-tail reads with few writes (trigger: failing LiveSpec's read p99 targets after the optimizations of point 1).

### Why not the other options

- **CoW B+ tree**: simpler to operate, with immediate opening and closing and better tails when the LSM's compaction competes with sustained writes, but Kerno's reference cases are write-intensive (Cauce: small durable commits; agent memory: continuous ingestion; Revision Engine: each change also writes a revision and a `ChangeSet`). There the CoW prototype performs between the same (agent memory) and 8× less (history writes), with 2.8× less in Cauce and 1.6× less in LiveSpec; among mature engines (redb against fjall) the gap goes in the same direction and is generally smaller (from a tie in LiveSpec to 1.7× in Cauce and 5.4× in history; only in agent memory is it larger, 1.2×); it writes ~100× more bytes per small commit, has worse tails on small durable commits, reads worse with cold cache, and offers no consistent relaxed durability mode without substantial additional work.
- **Third way**: in development days, the prototypes came close to the mature engines of their family — the LSM, level with fjall (0.8–1.3× at `medium`); the B+ tree, within ADR-0012's 2× target against redb (0.55–0.8× in most workloads, level in agent memory and above in sequential insertion) — and they passed a failure battery more demanding than a functional test, so the evidence does **not** force giving up the own core. A hybrid with two primary engines contradicts the small-core principle (SAD §3, principle 2). Adopting an external engine remains a contingency plan if Phase 1 does not meet its exit criteria.

## Consequences

- **Decisions that now depend on storage**: commit protocol (WAL + MANIFEST, ADR-0002), format (WAL, SST, MANIFEST with versioned headers, ADR-0003), cost and design of retention/suppression (targeted compaction, ADR-0009), encryption unit (immutable file/block, ADR-0006), operational metrics (levels, compaction, stalls).
- **Decisions that remain decoupled**: transaction and error semantics (ADR-0002), the revision model over keys (ADR-0004; the design only requires atomic batches and scans), identifiers (ADR-0005), derived indexes with a sequence watermark (ADR-0007), serialization (ADR-0008), API (ADR-0010), vectors (ADR-0011).
- Phase 1 starts with an engine that has background threads: concurrency and failure tests during compaction are mandatory from day one (the M0 harness already includes them).
- Clean shutdown costs more (it compacts until stable); it will be bounded with a budget.

## Follow-up

- Accepted on 2026-10-28 together with ADR-0002 and ADR-0003; M0 is closed.
- Repeat the M0 benchmarks on the target hardware (risk R-S3) before closing Phase 1.
- Implement points 1–3 of the decision in Phase 1, with the exit criteria of ADR-0012.
- **Phase 1 follow-up (WP14, 2026-10-29; no change of decision).** The product engine (`kerno-storage`) implements the conditions of this decision over the format v1 ([`docs/file-format/v1.md`](../file-format/v1.md), interpretations in [ADR-0013](0013-format-v1-and-engine-protocol.md), still «Proposed»). The exit measurement on the M0 VM class **does not meet T1** (condition 1) and leaves other documented non-compliances in the [Phase 1 report](../testing/f1-report.md); the contingency review (R-10/R-11) is decided by the owner. The repetition on the target hardware (R-S3) remains open.
