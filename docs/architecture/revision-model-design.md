# Design-level versioning prototype (M0)

Status: draft for review · Related to: [SAD §8](SAD.md#8-revision-engine-history-differences-and-provenance), [SAD §17.2](SAD.md#172-scope), [ADR-0004](../adr/0004-revision-history.md), [ADR-0001](../adr/0001-storage-engine.md)

M0 does not build the Revision Engine. This document fixes **how it would be represented** over the ordered key-value core, so that (a) the `history_sim` benchmark measures the cost of that representation on each storage candidate and (b) ADR-0001 takes into account the future compatibility with versioning (criterion 5 of SAD §17.5).

## 1. Principles that the design respects

1. **Revisions = data, not journal (D-06).** Revisions are written as normal keys in the same commit as the current state. The engine's WAL/journal only serves recovery and can be truncated without affecting the history (I-20).
2. **One atomic boundary (I-21).** Current state, revision entry and `ChangeSet` record go in the same `WriteBatch`. The engine already guarantees atomicity of the batch (I-01); the Revision Engine does not need a second commit protocol.
3. **Order by commit sequence (I-26).** The historical order is given by `commit_sequence`, not by the clock nor by UUIDv7.
4. **Restoring creates a new revision (I-22).**
5. **No invented context.** Actor, source and reason are optional.

## 2. Key layout

Collections are encoded as key prefixes (the core does not know about collections). Integers are encoded big-endian with a fixed width so that lexicographic order matches numeric order.

| Key | Value | What for |
|---|---|---|
| `c/{col}/{record_id}` | envelope of the current state: `rev_no`, `commit_seq`, `content_hash`, `schema_version`, flags, payload | reading the current state in a single search |
| `r/{col}/{record_id}/{rev_no:u64be}` | revision entry: `kind` (`snapshot` \| `delta` \| `tombstone` \| `restore`), `parent_rev`, `commit_seq`, `content_hash`, `changeset_id?`, full payload or delta | history of a record = **a single contiguous range** |
| `x/{changeset_id}` | `ChangeSet`: `commit_seq`, optional reason/actor/source/task, list of `(col, record_id, rev_no)` | answering "which operation produced this change" |
| `xs/{commit_seq:u64be}` | `changeset_id` | walking changes in commit order (auditing, derived-data synchronisation) |

`record_id` is the internal UUIDv7 (D-05). `rev_no` is a counter per record (1, 2, …); the identity of a revision is `(record_id, rev_no)` and its global position is `commit_seq`.

Choosing `record_id` before `rev_no` means that:

- *history of a record* is a prefix `scan`;
- *reconstructing revision r* is a `scan` from the last snapshot ≤ r to r (bounded by the delta policy);
- *diff between r1 and r2* reconstructs both with, at most, two ranges.

The alternative (`commit_seq` first) favours "what changed in commit N", which `xs/` already covers.

## 3. Snapshots and deltas

- The first revision of a record is always a `snapshot`.
- The following ones are structural `delta` relative to the previous one, unless (a) the maximum depth `K` is reached since the last snapshot, or (b) the accumulated size of the deltas exceeds a fraction `F` of the last snapshot. In both cases a `snapshot` is written.
- `K` and `F` are parameters to be fixed with benchmarks (Phase 3). `history_sim` uses `K = 8` and deltas of ~60 B over documents of 500 B as the starting point.
- Logical deletion: `tombstone` revision (the current state disappears or is marked according to the policy).
- Restoration: `restore` revision with the full historical content (snapshot) and `restored_from = rev_no`.
- The format of the delta (e.g. JSON Patch RFC 6902 or an own structural diff) and the canonical hash (RFC 8785) are decided in ADR-0004/ADR-0008. The design only requires that the delta be deterministic and applicable without the WAL.

## 4. History policies (SAD §8.4)

| Policy | What each change writes |
|---|---|
| `none` | only `c/…` (and `x/` if the application declares ChangeSets) |
| `audit` | `c/…` + `r/…` entry without payload (metadata: kind, hash, commit_seq, changeset) |
| `versioned` | `c/…` + `r/…` entry with snapshot/delta according to §3 |

## 5. Retention, suppression and what depends on the physical storage

- **Retention by age or number of revisions**: delete a range `r/{col}/{id}/[0, n)`. If the base snapshot of a chain is deleted, the first retained revision must be rewritten as a `snapshot` in the same commit.
- **Suppression of personal data** (ADR-0009): delete `c/`, all the `r/` of the record, references in `x/` and derived data (indexes, embeddings, queues), in a single logical transaction.
- **Physical deletion**: logical deletion does not delete the bytes. What remains depends on the candidate:
  - *CoW B+ tree*: the freed pages keep the old content until they are reused. To have suppression guarantees it is necessary to overwrite (scrub) the pages when freeing them, or to use encryption per record with key destruction.
  - *LSM*: the old versions remain in SST and in the WAL until compaction rewrites them and the WAL is recycled; the tombstones must reach the last level. A forced compaction of the affected range is needed or, equally, cryptographic erasure.
  - In both cases, backups and external copies follow their own deadlines (SAD §11.3, NIST SP 800-88).
- **Range deletion**: retention and suppression delete contiguous ranges. An LSM benefits from *range tombstones* (not implemented in the prototype); a B+ tree frees whole leaves.

## 6. How M0 measures it

The `history_sim` workload (see [`workloads.rs`](../../spikes/storage/spike-bench/src/workloads.rs)) applies this layout with `K = 8`: each update writes `cur/{id}` and `rev/{id}/{rev}` in the same batch, with revisions of different records interleaved in time (as real edits). It measures:

- cost of writing the history (revisions/s, commit latency p99);
- disk space against logical bytes (state + history);
- latency of reading the whole history of a record (`history_scan`);
- latency of reconstructing an arbitrary revision (`reconstruct`: base snapshot + deltas in a range).

The results and their reading are in the [M0 report](../testing/m0-report.md) and in ADR-0001.

## 7. Open questions (for ADR-0004)

1. `rev_no` per record or an own UUIDv7 `revision_id`? (The design uses a counter per record: shorter keys and trivial order.)
2. Structural deltas over the logical document or binary deltas over the serialisation? (Structural: readable and deterministic, but dependent on ADR-0008.)
3. Are the metadata of the record (classification, retention) versioned too?
4. How is a `ChangeSet` spanning thousands of records represented (inline list or `x/{id}/{n}` keys)?
5. Interaction with long-lived read snapshots: a reader walking the history while retention deletes old revisions must see a coherent state (guaranteed by I-05 of the engine).
