# Stage 3 design: units, revisions and provenance

Status: draft for owner review · Related to: [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §6.1–6.3 and §16, [SAD §8](SAD.md#8-revision-engine-history-differences-and-provenance), [ADR-0001](../adr/0001-storage-engine.md), [ADR-0002](../adr/0002-transaction-model-and-durability.md), [ADR-0003](../adr/0003-physical-format-and-versioning.md), [ADR-0004](../adr/0004-revision-history.md) (accepted), [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) (accepted), [M0 revision design](revision-model-design.md), [Phase 1 design](phase1-design.md), [invariants](invariants.md), [roadmap](../roadmap/README.md#stage-3--units-revisions-and-provenance)

This document is **design only**: there is no code, no ADR is accepted, and not a single byte of the v1 format changes. Everything that depends on a not-yet-accepted ADR (0005, 0008, 0017) is presented as a **proposal** and is listed in §12 for the owner to decide.

## 0. Summary

1. **New layer, format intact.** `Unit`, `Revision` and `ChangeSet` are represented as **ordinary keys and values** of the LSM in `kerno-storage` (which already provides atomic batches, snapshots, `delete_range` and read-modify-write transactions). Stage 3 does not touch the v1 format (ADR-0013 «immutable while v1 exists»): no header, no WAL, no SST, no MANIFEST. The layer has its **own** version number, stored as a metadata record inside the key space (§2.1). If the owner also wants an older engine to refuse to open a database with units, that is done with a **feature bit** (`incompat`, reserved range 1..62) and never with a change to `format_version` (§12, decision 3).
2. **One `ChangeSet` = one engine commit.** Every logical change (content, revision, `ChangeSet` member record) goes in **one** `WriteTxn` and therefore in a single WAL frame: I-01 and I-21 are inherited from the engine instead of being reimplemented (§4).
3. **Immutable revisions.** The key of a revision is written once and never rewritten; the current state (`head`) is the only mutable thing. History lives in its own keys, not in the WAL (I-20, §3, §5).
4. **«As-of» reads** of two kinds: against an engine snapshot (consistent with I-05) and against a historical `commit_sequence` (binary search over the unit's revisions, §3.5).
5. **First typed public API** (`UnitDb`, `ChangeSet`, `UnitSnapshot`, `UnitError`; §6), generic over `TxnEngine` from `kerno-core` so that the same tests can run on `LogEngine`/`Emulated<E>` and on `KernoDb`.
6. **Targets T1–T10 unchanged.** The layer lives in another crate and does not modify `kerno-storage`; its costs are measured with **new** metrics (the `U` series, §9.4) that the owner approves before measuring (ADR-0012), without reopening T1–T10 (§9.5).

## 1. What is there and what is not underneath

Facts from the code at the base commit (checked, not assumed):

| Capability | Where | What Stage 3 uses |
|---|---|---|
| `Engine::commit(&WriteBatch) -> seq`, atomic (I-01), sequence `+1` per non-empty commit (I-04) | `kerno-core::contract` | the `commit_sequence` of a `ChangeSet` is the one returned by the commit |
| `TxnEngine::begin_write` → `Txn` with `get`, `scan`, `put`, `delete`, `delete_range`, `commit`, `abort` | `kerno-core::contract`, `kerno-storage::txn::WriteTxn` | the `ChangeSet` **is** a `Txn`: it takes the single write slot, reads what it writes, and leaves no trace if it is aborted or dropped |
| `Snapshot::{seq, get, scan}` | `KernoSnapshot` | consistent reads of head, revision and history (I-05) |
| `delete_range` with range tombstones and `compact_range` | `Config`/WP10 | history retention and GC (§5) |
| `Durability::{Sync, Buffered}`, `sync()` | `Config::durability`, `SyncEngine` | the layer inherits durability; it does not define another one |
| Limits: key 512 B, value 1 MiB, batch 64 MiB / 2^20 operations (adjustable up to 16 KiB / 64 MiB) | `Config::limits` | cap on the payload of a unit and of a `ChangeSet` (§2.4, §6.5) |
| `Error::{Io, Corruption, Locked, InvalidArgument, Poisoned, Incompatible, ReadOnly}` | `kerno-core::error` | wrapped without losing the class (§6.6) |
| `SimVfs` with power loss, torn write, `fsync` failure, full disk, traces | `kerno-testkit::vfs` | crash sweeps of the layer (§8) |

What there is **not**, and this stage does not add: reverse reads (`scan`s go forward; it is designed around that, §3.5), first-class composite keys (they are encoded by hand, §2), index types (Stage 4), encryption (AEAD stays `Incompatible`, ADR-0006).

Design consequence: the layer needs nothing from the engine that does not already exist, except exposing the read sequence on the `Txn` trait (`read_seq`, already present in `WriteTxn`). If a real need appears later, it enters through the ADR-0013 route: feature bit + vectors + ADR, never by rewriting v1 bytes.

## 2. Data model and key encoding

### 2.1 Principles

- **One tag byte per record type**, followed by fixed-width **big-endian** fields so that lexicographic order is numeric order (as in the M0 design, §2). No key exceeds 512 B (the largest is 25 B).
- **The records of a unit are contiguous**: `unit_id` comes before `rev_no`, so «the whole history of a unit» is a single prefix `scan` and retention is a single `delete_range`.
- **No collections in the key.** Spec §6.1 identifies the unit by `unit_id` and `unit_type` (a field of the head). Filtering by type is an index (Stage 4, ADR-0007/0014); Stage 3 walks the heads.
- **Historical order is given by `commit_sequence`**, not by the clock or UUIDv7 (I-26). `created_at` values are descriptive data.

### 2.2 Key space

| Tag | Key | Value | Mutable | What for |
|---|---|---|---|---|
| `0x01` `LAYER_META` | `01 ‖ name` (UTF-8, ≤ 64 B) | `layer_version` and, per name, the history policy per type | yes (rare) | marks that the database has a unit layer; see §2.6 |
| `0x10` `UNIT_HEAD` | `10 ‖ unit_id[16]` (17 B) | `UnitHead` (§2.3) | **yes** | current state in **one** lookup |
| `0x20` `REV_META` | `20 ‖ unit_id[16] ‖ rev_no:u64be` (25 B) | `RevisionMeta` (§2.4) | **no** | a unit's history = one contiguous range, without reading payloads |
| `0x21` `REV_PAYLOAD` | `21 ‖ unit_id[16] ‖ rev_no:u64be` (25 B) | revision payload (opaque bytes) | **no** | separates large bytes from metadata |
| `0x30` `CHANGESET` | `30 ‖ commit_seq:u64be` (9 B) | `ChangeSetRecord` (§2.5) | **no** | «which operation produced this change» and traversal in commit order |
| `0x31` `CS_MEMBER` | `31 ‖ commit_seq:u64be ‖ ordinal:u32be` (13 B) | `(unit_id, rev_no, kind)` | **no** | members of a `ChangeSet`, with no practical size limit |

Tags `0x02..0x0F`, `0x11..0x1F`, `0x22..0x2F` and `0x32..0xFF` are reserved for later stages (relations, derived data). A record with an unknown tag **inside the range of a known tag** is `Corruption`; an unknown top-level tag is `IncompatibleLayer` (a newer layer wrote it).

Why `REV_META` and `REV_PAYLOAD` are separate: listing the history (`history`, `compare` on metadata, GC) reads only `20…`, so a 800 KiB unit with 1 000 revisions does not force reading 800 MiB just to draw a list. The LSM stores the two ranges in different blocks. The cost is one more small key per revision in the same batch.

### 2.3 `UnitHead` (current state)

Fields (encoding: `envelope_version` byte, fixed-width big-endian integers, strings and bytes with `u32be` length prefix; the byte-by-byte specification will go to `docs/file-format/units-v1.md`, WP-E3-01):

| Field | Type | Note |
|---|---|---|
| `unit_type` | string ≤ 64 B | spec §6.1 |
| `schema_version` | `u32` | of the content schema, not of the layer |
| `lifecycle` | `u8` | `Active=0`, `Archived=1`, `Deleted=2` |
| `head_rev` | `u64` | current revision (`current_revision_id`); ≥ 1 |
| `oldest_rev` | `u64` | first **retained** revision (1 if there was no GC); written by GC, §5 |
| `created_seq`, `updated_seq` | `u64` | `commit_sequence` of the first and of the last revision |
| `created_at`, `updated_at` | `i64` µs Unix | from the injected clock (§6.7); descriptive |
| `content_hash` | `alg:u8 ‖ [u8;32]` | hash of the payload of `head_rev`; see §2.7 |
| `payload_len` | `u32` | of the payload of `head_rev` |
| `payload_codec` | `u8` | `0` opaque bytes; the rest is assigned by ADR-0008 |
| `metadata` | bytes ≤ 4 KiB | unit metadata (spec §6.1) |

The **current payload is not in the head**: it is in `REV_PAYLOAD(unit, head_rev)`. `get_unit` makes two point `get`s (head and payload) and verifies the hash. It was chosen this way, rather than duplicating the bytes in the head as the M0 design did, because (a) writing an update goes from «2× payload» to «1× payload», (b) there is no state in which head and current revision diverge, and (c) T1 is dominated by the WAL `fdatasync` (project note #88), so extra bytes in the WAL cost, but an extra `get` of a key in cache does not. It is decision 5 of §12 (alternative: duplicate in order to read in a single lookup).

### 2.4 `RevisionMeta` (immutable)

| Field | Type | Note |
|---|---|---|
| `kind` | `u8` | `Create=1`, `Update=2`, `Archive=3`, `Unarchive=4`, `Delete=5` (tombstone), `Restore=6`; `7..=15` reserved (e.g. `Delta`, §3.3) |
| `parent_rev` | `u64` | `0` only in `Create` |
| `commit_seq` | `u64` | `commit_sequence` (spec §6.2); also the key of its `ChangeSet` |
| `created_at` | `i64` µs | |
| `schema_version` | `u32` | |
| `content_hash`, `payload_len`, `payload_codec` | as in §2.3 | `payload_len = 0` and the hash of the empty payload in `Delete`; under the `audit` policy hash and length are kept with `payload_present = 0` (see «Note of E3-05») |
| `payload_present` | `u8` | `1` if a `REV_PAYLOAD` exists for this revision (becomes absent by policy or GC, §3.4/§5) |
| `restored_from` | `u64` | `0` except in `Restore` |
| `metadata` | bytes ≤ 4 KiB | the metadata **versioned** with the revision (decision 6) |
| `actor_ref`, `source_ref` | bytes ≤ 256 B each, optional | «no invented context» (revision-model §1.5) |

The public `revision_id` is `(unit_id, rev_no)`: a per-record counter (a decision already leaning that way in revision-model §7.1 for shorter keys and trivial order). There is no UUID `revision_id` of its own in Stage 3.

`Archive`/`Unarchive`/`Restore` **copy** the complete payload (their own `REV_PAYLOAD`): no revision depends on another one still existing, which makes retention by ranges safe (§5). Revisions by reference or by delta are a later optimisation behind the `kind` byte (§3.3).

### 2.5 `ChangeSetRecord`

| Field | Type | Note |
|---|---|---|
| `change_set_id` | implicit | **is** the `commit_sequence` (key `30 ‖ commit_seq`) |
| `created_at` | `i64` µs | |
| `member_count` | `u32` | number of `CS_MEMBER` keys |
| `purpose` | `u8` | `Application=0`, `Retention=1` (GC, §5), `Maintenance=2` |
| `reason`, `actor_ref`, `source_ref`, `correlation` | bytes ≤ 1 KiB each, optional | spec §6.3: reason, actor/source, correlation |

Making `change_set_id == commit_sequence` avoids a separate counter and makes «walking changes in commit order» (audit, synchronisation of Stage 8 derived data) a `scan` of `0x30`. The gap between sequences (commits that are not from the layer, or `Retention` not written) is normal: `ChangeSet`s are not contiguous.

**How the sequence is known before the commit.** The `Txn` holds the write slot; the contract guarantees that nothing else commits between `begin_write` and `commit`, and that a non-empty commit receives `seq + 1` (I-04). Therefore `commit_seq = txn.read_seq() + 1` is computed **before** writing `REV_META` and `CHANGESET`, and the commit returns that same value. The layer checks it after the commit; a discrepancy cannot be undone and is treated as an invariant violation (`UnitError::Storage(Error::Poisoned)` and the handle is invalidated), with a test that makes it practically impossible (§8, `seq_prediction`). **Required contract change:** `read_seq()` today is an inherent method of `kerno_storage::WriteTxn`; Stage 3 adds `fn read_seq(&self) -> u64` to the `kerno_core::Txn` trait (implemented by `WriteTxn` and by `EmulatedTxn`), the only modification outside `kerno-units`, made in E3-01. Discarded alternative: write first and note the sequence in a second commit (breaks I-21).

### 2.6 Layer marker and compatibility

`LAYER_META["layer"]` stores `layer_version` (`1` in Stage 3). The first `ChangeSet` of a new database creates it in the same commit. On `open`:

- empty database and `create_if_missing` → the marker is created with the first commit;
- database with keys but without the marker → `UnitError::NotAUnitDatabase` (nothing is guessed);
- `layer_version` greater than the supported one → `UnitError::IncompatibleLayer` without modifying anything (same philosophy as I-15/HDR-4);
- read-only mode: never creates anything (I-18); an empty database opened read-only is opened as empty.

This does not replace a feature bit: an older engine (or direct access to `KernoDb`) can write keys without going through the layer. While `kerno-storage` is the unstable API of Phase 1 and the layer is its only consumer, this is accepted; decision 3 of §12 asks the owner to settle whether the `incompat` bit «UNITS» is needed.

### 2.7 Content hash and serialisation

`content_hash` = hash of `payload_codec ‖ payload` (the payload as it is stored). Stage 3 treats the payload as **opaque bytes**: the canonical serialisation (RFC 8785 or another) and the algorithm are ADR-0008 (pending). The field carries an `alg` byte precisely so that the algorithm is not encoded in the layer's format. Proposal: BLAKE3-256 if a new dependency is accepted (justification pending: it is not in the standard library or in the current manifest), or SHA-256 otherwise; in either case the dependency enters with ADR-0008 and not before (decision 7). **Resolved (2026-10-31):** ADR-0008 accepted with BLAKE3-256 (`alg = 1`) and the dependency `blake3 = "=1.8.7"` pinned; the verification is in `units-v1.md` §5.1 (E3-02). In the meantime the layer may use `alg = 0` with CRC-32C, already available in the project, marked as «no cryptographic guarantee», only to detect end-to-end mismatch. Length and presence are checked on every read; the content hash is checked when `verify_payload_on_read` is on, and always by `verify()`, history, export/import and the reads a write builds on; a damaged block is always caught by the block CRC. A hash mismatch = `Corruption` (I-08), never data different from what was committed.

### 2.8 Layout example

Unit `U`, three changes (create, update, archive), `versioned` policies:

```
10 U                    -> head{head_rev=3, oldest_rev=1, lifecycle=Archived, ...}
20 U 0000000000000001   -> meta{Create,  commit_seq=41}
20 U 0000000000000002   -> meta{Update,  commit_seq=57}
20 U 0000000000000003   -> meta{Archive, commit_seq=90}
21 U 0000000000000001   -> payload v1
21 U 0000000000000002   -> payload v2
21 U 0000000000000003   -> payload v2 (copia)
30 0000000000000029     -> changeset{41}      31 0000000000000029 00000000 -> (U, 1, Create)
30 0000000000000039     -> changeset{57}      ...
```

## 3. Revisions, history and as-of reads

### 3.1 Immutability

A `REV_META`/`REV_PAYLOAD` key is **created** once. Inside the `Txn` the `ChangeSet` computes `rev_no = head_rev + 1` by reading the head, so the key cannot exist; even so, the layer does a check `get` in test mode (`debug_assert`) and `verify_units` (§8.5) checks it offline. Only two operations delete revision keys: retention (§5) and the suppression of ADR-0009 (outside this stage, §10); **none** of them rewrites them. This turns «immutable revision» into a property that can be checked by sweep: any two snapshots must agree on every `0x20/0x21` key that both contain.

### 3.2 Writes per operation

All of them happen in the `Txn` of the `ChangeSet`; `s = txn.read_seq() + 1`.

| Operation | Reads | Writes |
|---|---|---|
| `create(type, payload, meta)` | nothing (checks that `unit_id` does not exist) | `UNIT_HEAD`, `REV_META(1)`, `REV_PAYLOAD(1)`, `CS_MEMBER` |
| `update(id, payload, expected)` | `UNIT_HEAD` | `UNIT_HEAD` (new), `REV_META(n+1)`, `REV_PAYLOAD(n+1)`, `CS_MEMBER` |
| `archive` / `unarchive` | `UNIT_HEAD` + current payload | same as `update` with its own `kind` and a copy of the payload |
| `delete(id)` | `UNIT_HEAD` | `UNIT_HEAD{lifecycle=Deleted}`, `REV_META(n+1){Delete}`, `CS_MEMBER` (no payload) |
| `restore(id, rev)` | `UNIT_HEAD`, `REV_META(rev)`, `REV_PAYLOAD(rev)` | same as `update` with `kind=Restore`, `restored_from=rev` and a full copy |
| (once per `ChangeSet`) | | `CHANGESET(s)` with `member_count` |

`Delete` is logical: the head remains (with `lifecycle=Deleted`) so that history and as-of reads keep working. Recreating with the same `unit_id` is forbidden (`AlreadyExists`); another `unit_id` is created. Physical deletion is ADR-0009 (§10).

`expected` is an optional `RevNo` (optimistic check): if the head is no longer at that revision, `Conflict { expected, found }`. With a serialisable `Txn` there is no race inside the `ChangeSet`, but there is one between the application's read and `begin_changeset`.

**Restore** (I-22): creates a new revision whose content is that of `rev`; the revisions after `rev` **remain**. Restoring a pruned revision is `Pruned`.

### 3.3 Snapshots and deltas

Stage 3 stores **one complete payload per revision** (each `REV_PAYLOAD` is a snapshot). Reasons: (a) the delta format is ADR-0008 (pending) and must not encode something hard to reverse before it is accepted (ADR-0004 requires this); (b) without deltas, reconstruction is O(1) and retention by ranges does not need to rewrite the first retained revision; (c) the cost is space, measurable with M0's `history_sim`. The `kind` byte and the reserve `7..=15` leave room for `Delta` with maximum depth `K` and fraction `F` (revision-model §3) once there are benchmarks that set them; adding it is a change of `layer_version`, not of the storage format. Decision 4 of §12.

### 3.4 History policies (SAD §8.4)

Per unit type, stored in `LAYER_META` (`policy/<unit_type>`; by default the one in `UnitConfig`). Changing the policy writes the record inside a `Maintenance` `ChangeSet`; it does not rewrite existing revisions.

| Policy | `REV_META` | `REV_PAYLOAD` | `CHANGESET` + members | Possible reads |
|---|---|---|---|---|
| `none` | no | only the current one: each update deletes the previous payload in the same batch | yes (minimal provenance) | only current state (and as-of against an engine snapshot) |
| `audit` | yes, `payload_present=0` except for the current one | only the current one | yes | metadata and hash history; no past content |
| `versioned` | yes | all (until retention) | yes | everything |

A read that requires past content under `none`/`audit` returns `UnitError::HistoryUnavailable { policy }`, not an empty result.

### 3.5 Reads

All reads go through an engine snapshot (the one of the `UnitDb` in each standalone call, or the one pinned by `UnitSnapshot`), so they inherit I-05, and a head read together with its revisions is coherent even if a `ChangeSet` or retention commits in between.

- **`get_unit(id)`**: `get(10‖id)` + `get(21‖id‖head_rev)` + hash verification. `Deleted` returns the head without payload.
- **`get_revision(id, rev_no)`**: `get(20…)` + `get(21…)`; outside `[oldest_rev, head_rev]` → `Pruned` (below) or `RevisionNotFound` (above).
- **`history(id, range, order, limit)`**: `scan` of `20‖id‖[from, to)` of metadata only. `Order::Desc` is resolved with a forward page from `max(oldest, head - limit + 1)` and reversed in memory (the engine has no reverse scan; the limit bounds the read).
- **`unit_as_of(id, AsOf::Seq(s))`**: since `commit_seq` grows with `rev_no` within a unit, it finds the largest revision with `commit_seq ≤ s` by **binary search with `get` of `REV_META`** over `[oldest_rev, head_rev]` (O(log n) reads of ~100 B). Result: `Absent` (`s` before creation), `Present(Revision)`, `DeletedAt(rev)`, or `Pruned` if `s` falls before `oldest_rev`. An `s` greater than the snapshot's sequence is `InvalidArgument`. *Alternative discarded for now:* secondary index `22‖id‖(!commit_seq)` with the sequence inverted, which resolves «the last ≤ s» with a single forward `scan`; it adds one key per revision and is reconsidered if the binary search does not meet the `U` targets (§9.4).
- **`state_as_of(AsOf::Seq(s))`** (state of **all** units at `s`): walks `10…` and applies `unit_as_of` to each one whose `updated_seq > s`; if `updated_seq ≤ s` the head is already the answer. It is linear in units; it is documented as such. The walk over `CHANGESET` from `s+1` allows the incremental variant («what changed since `s`»): a `scan` of `30‖(s+1)…` and its `31`s.
- **`compare(id, a, b)`**: returns both `Revision`s and `same_content` (comparison of hash and bytes). The **structural diff** (JSON Patch or another) is defined by ADR-0008; Stage 3 does not invent it.
- **`AsOf::Time`** (clock instant) **is left out**: the clock is not monotonic and I-26 forbids ordering by it. If wanted, it is defined as «last `ChangeSet` whose `created_at ≤ t`, scanning by sequence» and it is documented that it depends on the clock (decision 8).

### 3.6 Order and visibility

Transactional visibility (spec §6.2): a `ChangeSet` is visible in full or not at all, and only after `commit` (the engine publishes `last_seq` after the `fsync` in `Sync`, I-16). A reader with a `UnitSnapshot` at `s` sees exactly the `ChangeSet`s with `commit_seq ≤ s`.

## 4. `ChangeSet` as an atomic commit

- `begin_changeset(meta)` calls `begin_write()`: it waits for the write slot (`ChangeSet`s and blind commits are serialised; note #2: a second `begin_*` on the same thread blocks; the `ChangeSet` is `!Send` only if the `Txn` is, and the documentation warns about it).
- Each operation validates (sizes, lifecycle state, `expected`) and writes into the `Txn`; **none touches shared state** until `commit` (engine contract): aborting or dropping leaves no trace and consumes no sequence (I-01).
- `commit()`: writes `CHANGESET(s)` with `member_count` and returns `CommitInfo { change_set_id, commit_seq, revisions }`. A `ChangeSet` without operations writes nothing and returns `CommitInfo::empty(seq_actual)` (the engine does not consume a sequence).
- **I-21** holds by construction: content (`10`, `21`), revision (`20`) and record (`30`, `31`) are in the same `Txn`, that is, in the same WAL frame. There is no own commit protocol; if in a later stage the indexes are transactional (Stage 4), they enter the same `Txn`.
- Limits: the engine rejects a batch that exceeds `max_batch_ops`/`max_batch_bytes` with `InvalidArgument` **when writing**, before any effect; the layer translates it to `ChangeSetTooLarge { ops, bytes }`. A unit with a payload > `max_value_len - 0` is `PayloadTooLarge` (no chunking at this stage; raising `Config::limits.max_value_len` up to 64 MiB is an adjustment, not a format change).
- A long `ChangeSet` holds the write slot: this is documented and `UnitConfig::max_changeset_ops` (10 000 operations by default) is offered as protection. Retention jobs use small batches (§5).
- Durability: that of the engine's `Config::durability`. `CommitInfo` indicates `durable: bool` (`true` in `Sync`; in `Buffered`, until `sync()`). The layer exposes `sync()` as it is (I-17).
- Poisoning: a failed `fsync` returns `UnitError::Storage(Poisoned)`; the `UnitDb` is unusable until reopened, and recovery (I-10, I-11) decides the state. The commit is never retried.

## 5. History storage, retention and GC

### 5.1 What takes up space and when it is freed

Revisions are normal keys: they take up space until they are deleted **and** the LSM rewrites them. There are three layers, each with its own responsibility:

1. **Logical retention** (the layer): decides which revisions stop being reachable and deletes them with `delete_range`.
2. **LSM compaction** (the engine): discards the keys covered by tombstones when no snapshot sees them any more (I-05). The layer does not control this, except by requesting `compact_range` over the range (`CompactRangeReport`).
3. **WAL**: it is recycled with the flush; it holds no history that anyone needs (I-20: the WAL serves only for recovery).

Consequence that must be visible in the API: **a long snapshot keeps pruned history alive** (I-05). `UnitDb::metrics()` re-exports the engine's snapshot metrics (how many there are, what they retain) and `RetentionReport` reports `bytes_logically_freed` apart from `bytes_reclaimed` (the latter only after `compact`).

### 5.2 Retention policies

`RetentionPolicy` per unit type: `KeepAll` (default), `KeepLast(n)`, `KeepFor(duration)` (uses the revision's `created_at`; descriptive, it does not order), or `KeepSince(commit_seq)`. Rules:

- The current revision and the last revision before a `Delete` are **always kept**; retention never leaves a unit without a valid head.
- Pruning = in **one** `Txn` per unit: `delete_range(20‖id‖[oldest, k))`, `delete_range(21‖id‖[oldest, k))`, `UNIT_HEAD{oldest_rev=k}` and a `CHANGESET{purpose=Retention}` with one member per unit. Head and ranges change together (I-21); a crash leaves the unit pruned entirely or not pruned at all.
- The first retained revision is not rewritten (unlike M0's design with deltas) because each revision is a complete snapshot (§3.3).
- `CHANGESET` and `CS_MEMBER` are **not pruned** in Stage 3: they are the provenance (small: ~60 B per revision). A member whose revision was pruned is reported as `pruned` (the unit's head says `oldest_rev`). Its deletion belongs to ADR-0009 suppression (decision 9).
- Retention runs **on request** (`UnitDb::run_retention(budget)`), not on its own thread, in batches bounded by `budget { max_units, max_ops }` and resumable: since each batch is atomic and self-contained, there is no intermediate state to recover and repeating it is idempotent. An automatic scheduler comes later.

### 5.3 What it does not promise

Logical deletion is **not** physical deletion: pruned versions remain in SST and WAL until compaction and recycling, and in backups (revision-model §5). The suppression guarantees (ADR-0009, ADR-0006) are not declared in this stage; the stage report says so explicitly.

## 6. First public Rust API

Subject to ADR-0010 (crates and public API, pending): **proposal** of a new crate `kerno-units` (provisional name) that depends on `kerno-core` and, for the concrete type, on `kerno-storage`. Unstable until ADR-0010 is accepted. The signatures are indicative; the type names are the ones that get tested.

### 6.1 Opening

```rust
pub struct UnitConfig {
    pub storage: kerno_storage::Config,      // durabilidad, límites, perfil
    pub create_if_missing: bool,
    pub default_policy: HistoryPolicy,
    pub max_changeset_ops: usize,            // 10_000 por defecto
    pub clock: Arc<dyn Clock>,               // inyectable (determinismo en SimVfs)
    pub ids: Arc<dyn UnitIdSource>,          // inyectable
}

pub struct UnitDb<E: TxnEngine = KernoDb> { /* engine, config, cache de políticas */ }
pub type KernoUnits = UnitDb<KernoDb>;

impl<E: TxnEngine> UnitDb<E> {
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &UnitConfig) -> Result<Self, UnitError>;
    pub fn close(self) -> Result<(), UnitError>;
    pub fn sync(&self) -> Result<u64, UnitError>;        // I-17 (requiere E: SyncEngine)
}
```

`open` opens the engine, validates the layer marker (§2.6) and writes nothing in read-only mode. A default type parameter allows normal use without generics and tests with `Emulated<LogEngine>`.

### 6.2 Value types

```rust
pub struct UnitId([u8; 16]);                    // interno (D-05); el id público es ADR-0005
pub struct RevNo(u64);                          // 1-based
pub struct RevisionId { pub unit: UnitId, pub rev: RevNo }
pub struct ChangeSetId(u64);                    // == commit_sequence
pub struct CommitSeq(u64);

pub enum Lifecycle { Active, Archived, Deleted }
pub enum ChangeKind { Create, Update, Archive, Unarchive, Delete, Restore }
pub enum HistoryPolicy { None, Audit, Versioned }

pub struct Unit { pub id: UnitId, pub unit_type: String, pub schema_version: u32,
                  pub lifecycle: Lifecycle, pub head: RevNo, pub oldest: RevNo,
                  pub created_at: Timestamp, pub updated_at: Timestamp,
                  pub metadata: Vec<u8>, pub payload: Option<Vec<u8>>, pub content_hash: ContentHash }
pub struct Revision { pub id: RevisionId, pub kind: ChangeKind, pub parent: Option<RevNo>,
                      pub commit_seq: CommitSeq, pub change_set: ChangeSetId, pub created_at: Timestamp,
                      pub schema_version: u32, pub content_hash: ContentHash,
                      pub restored_from: Option<RevNo>, pub provenance: Provenance,
                      pub payload: Option<Vec<u8>> }       // None en historial de metadatos
pub struct Provenance { pub actor: Option<Vec<u8>>, pub source: Option<Vec<u8>> }
```

`Timestamp` is an `i64` of Unix microseconds with its own name. Payloads are `Vec<u8>` (opaque bytes, §2.7); ADR-0008 will add typed codecs on top, without changing the layer.

### 6.3 Writing

```rust
pub struct ChangeMeta { pub reason: Option<String>, pub actor: Option<Vec<u8>>,
                        pub source: Option<Vec<u8>>, pub correlation: Option<Vec<u8>>,
                        pub purpose: ChangePurpose }

impl<E: TxnEngine> UnitDb<E> {
    pub fn begin_changeset(&self, meta: ChangeMeta) -> Result<ChangeSet<'_, E>, UnitError>;
    /// Atajos de una sola operación (un ChangeSet de un miembro).
    pub fn create_unit(&self, new: NewUnit, meta: ChangeMeta) -> Result<(UnitId, CommitInfo), UnitError>;
}

impl<'db, E: TxnEngine> ChangeSet<'db, E> {
    pub fn create(&mut self, new: NewUnit) -> Result<UnitId, UnitError>;
    pub fn update(&mut self, id: UnitId, upd: UnitUpdate, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn archive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn unarchive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn delete(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn restore(&mut self, id: UnitId, from: RevNo, expected: Option<RevNo>) -> Result<RevNo, UnitError>;
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError>;   // lee sus propias escrituras
    pub fn commit(self) -> Result<CommitInfo, UnitError>;
    pub fn abort(self);                                                       // y Drop
}

pub struct CommitInfo { pub change_set: ChangeSetId, pub commit_seq: CommitSeq,
                        pub revisions: Vec<RevisionId>, pub durable: bool }
```

`create` and the others are **not** `async`; they block like `begin_write` (engine contract). A `ChangeSet` that is empty at `commit` writes nothing.

### 6.4 Reading and history

```rust
impl<E: TxnEngine> UnitDb<E> {
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError>;
    pub fn snapshot(&self) -> Result<UnitSnapshot<E>, UnitError>;
}

impl<E: TxnEngine> UnitSnapshot<E> {
    pub fn seq(&self) -> CommitSeq;
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError>;
    pub fn get_revision(&self, id: UnitId, rev: RevNo) -> Result<Revision, UnitError>;
    pub fn history(&self, id: UnitId, q: HistoryQuery) -> Result<Vec<Revision>, UnitError>;
    pub fn unit_as_of(&self, id: UnitId, at: AsOf) -> Result<UnitAt, UnitError>;
    pub fn compare(&self, id: UnitId, a: RevNo, b: RevNo) -> Result<Comparison, UnitError>;
    pub fn change_set(&self, id: ChangeSetId) -> Result<ChangeSetInfo, UnitError>;
    pub fn change_sets_since(&self, after: CommitSeq, limit: usize) -> Result<Vec<ChangeSetInfo>, UnitError>;
    pub fn scan_units(&self, after: Option<UnitId>, limit: usize) -> Result<Vec<Unit>, UnitError>;
}

pub struct HistoryQuery { pub from: Option<RevNo>, pub to: Option<RevNo>,
                          pub order: Order, pub limit: usize, pub with_payload: bool }
pub enum AsOf { Seq(CommitSeq) }                        // Time queda fuera (decisión 8)
pub enum UnitAt { Absent, Present(Revision), DeletedAt(Revision), Pruned { oldest: RevNo } }
```

Paginated results carry an explicit `limit` and return the last `RevNo`/`UnitId` to continue; no query loads an entire history without a cap. Composite return types are named with a `type` alias (project rule: `clippy -D warnings` rejects `clippy::type_complexity`, constraint #17).

### 6.5 Maintenance

```rust
impl<E: TxnEngine> UnitDb<E> {
    pub fn set_policy(&self, unit_type: &str, p: HistoryPolicy, meta: ChangeMeta) -> Result<CommitInfo, UnitError>;
    pub fn run_retention(&self, policy: &RetentionPolicy, budget: RetentionBudget) -> Result<RetentionReport, UnitError>;
    pub fn compact(&self) -> Result<CompactReport, UnitError>;        // delega en compact_range
    pub fn verify_units(&self) -> Result<UnitVerifyReport, UnitError>; // §8.5, solo lectura
    pub fn metrics(&self) -> UnitMetrics;                              // incluye las del motor
}
```

### 6.6 Errors

```rust
#[non_exhaustive]
pub enum UnitError {
    Storage(kerno_core::Error),                 // Io, Corruption, Locked, Poisoned, Incompatible, ReadOnly, InvalidArgument
    NotFound { unit: UnitId },
    AlreadyExists { unit: UnitId },
    Deleted { unit: UnitId },                   // operación inválida sobre una unidad en lápida
    Conflict { unit: UnitId, expected: RevNo, found: RevNo },
    RevisionNotFound { unit: UnitId, rev: RevNo },
    Pruned { unit: UnitId, oldest: RevNo },
    HistoryUnavailable { policy: HistoryPolicy },
    PayloadTooLarge { len: usize, max: usize },
    ChangeSetTooLarge { ops: usize, bytes: usize },
    InvalidArgument(String),
    NotAUnitDatabase,
    IncompatibleLayer { found: u32, supported: u32 },
    ChangeSetClosed,
}
```

Rules: (a) every engine error is preserved **without changing class** inside `Storage` (in particular `Corruption` is never degraded to a layer error, and `Poisoned` still means «reopen»); (b) a layer record that does not decode or whose hash does not match is `Storage(Corruption)` with context (which key, which `UNIT-n` rule); (c) no message contains unit content or metadata, only ids and numbers (I-29, I-37).

### 6.7 Determinism

`Clock` and `UnitIdSource` are injectable traits (`SystemClock`/`UuidV7Source` by default; `FixedClock`/`CounterIds` in tests). Without them, `SimVfs` sweeps would not be reproducible by seed. The layer does not use `std::time` or randomness directly, just as `kerno-storage` does not use `std::fs` (everything goes through `Vfs`).

## 7. Invariants

Reuses I-01…I-19 from the engine and makes concrete I-20…I-22, I-26 and I-31. The new ones are proposals (`IU-n`) to be promoted to `invariants.md` once they have a test, under that document's rule: an invariant without an automated test is marked *pending*.

| ID | Invariant | Verification (§8) |
|---|---|---|
| I-20 | History does not depend on the WAL: after flush and WAL recycling, `history`/`get_revision` return the same thing; truncating or recycling the WAL never deletes revisions. | `wal_independence`, sweeps with the `Inline` profile (WAL recycled, note #27) |
| I-21 | Head, `REV_META`, `REV_PAYLOAD`, `CHANGESET` and `CS_MEMBER` of a `ChangeSet` appear all or none after any crash. | `changeset_every_point`, `model_crash` |
| I-22 | Restoring creates a new revision; no revision after the restored one changes or is deleted. | `restore_keeps_later`, comparison of snapshots before/after |
| I-26 | Historical order is `commit_sequence`; `rev_no` and `commit_seq` grow together per unit; neither UUIDv7 nor the clock participates. | `ordering_ignores_clock` (clock going backwards) |
| IU-1 | **Immutable revision**: an existing `0x20/0x21` key never changes value. | `immutability_sweep` between snapshots |
| IU-2 | **Coherent head**: `head_rev` exists, its `REV_META` matches hash/length/schema with the head, and `oldest_rev ≤ head_rev`; `REV_META` covers `[oldest_rev, head_rev]` without gaps (under `versioned`). | `verify_units` |
| IU-3 | **Chain and order**: `parent_rev = rev-1`, `commit_seq` strictly increasing within the unit. | `verify_units` |
| IU-4 | **Total provenance**: every `REV_META` with `commit_seq = s` has `CHANGESET(s)` and a `CS_MEMBER` that names it; `member_count` = number of members. | `verify_units` |
| IU-5 | **End-to-end hash**: the hash of each payload read matches; otherwise `Corruption`. | `payload_hash_checked`, damage sweep (§8.4) |
| IU-6 | **Exact as-of**: `unit_as_of(id, s)` matches the model at every `s` ≤ the snapshot's `seq`, including before creation and after deletion. | `model_as_of` |
| IU-7 | **Atomic and safe retention**: after `run_retention` (and after a crash at any point) each unit has an `oldest_rev` consistent with the existing keys and keeps its current revision. | `retention_every_point` |
| IU-8 | **History isolation**: a `UnitSnapshot` sees the history of its `seq` even if retention or new `ChangeSet`s commit afterwards (revision-model §7.5). | `snapshot_vs_retention` |
| IU-9 | **Policy respected**: under `none` no old `REV_META`/`REV_PAYLOAD` remains; under `audit`, no old payload. | `policy_matrix` |
| IU-10 | **No content in errors or metrics** (I-29/I-37 applied to the layer). | `sentinel_no_content` |

## 8. Test plan

Framework: the existing one (`kerno-testkit`, `SimVfs`, seeds via `SPIKE_SEEDS`, shared suites). Each new suite is validated first against the trivially correct engine (`LogEngine` with `Emulated<E>`) and then against `KernoDb`, as phase1-design §1.5 requires. A test never seen to fail counts for nothing: each suite includes its mutation (inverting an assertion or skipping a layer write) and it is checked that it turns red.

### 8.1 Model and functional tests

- **Reference model** (`UnitModel`): `BTreeMap<UnitId, Vec<ModelRevision>>` plus a list of `ChangeSet`s; it applies the same operations and defines the correct answer for `get_unit`, `history`, `unit_as_of`, `state_as_of`, `change_sets_since` and `compare`.
- `model_ops`: random operations (create, update, archive, delete, restore, `ChangeSet` of 1..50 members, aborts) with a full comparison after each `ChangeSet` and saved old `UnitSnapshot`s that are re-read at the end (IU-6, IU-8).
- `model_as_of`: for each `s` in `[0, seq]` and each unit, `unit_as_of` = model.
- `restore_keeps_later`, `delete_then_history`, `conflict_expected_rev`, `payload_limits`, `changeset_limits` (exactly at the limit and one above: no effects), `policy_matrix` (3 policies × operations), `ordering_ignores_clock` (injected clock that goes backwards).
- `seq_prediction`: with 1..N threads attempting blind commits and `ChangeSet`s, the `commit_seq` written in each `REV_META` is the one returned by `commit` (§2.5); an aborted `ChangeSet` consumes no sequence.

### 8.2 Crashes and recovery with `SimVfs`

The layer is tested with the same sweeps as the engine, but with **`ChangeSet` scenarios** instead of batches:

| Suite | What it does | Assertion after recovery |
|---|---|---|
| `changeset_every_point` | runs a fixed scenario per seed; `crash_after(n, kind)` for **each** mutating I/O operation `n` (like `crash::every_point`), with `CrashKind::{PowerLoss, ProcessKill}` and `Durability::{Sync, Buffered}` | `last_seq` = prefix of complete `ChangeSet`s; in `Sync`, `s ≥` last confirmed; the state = model after exactly `k` `ChangeSet`s; `verify_units` without findings (I-21, IU-2…IU-4) |
| `model_crash` | `model_ops` with a random crash per round (`crash_now`, `take_crash_image`, `restore_seeded`) and continuation on the recovered image | as above, and the database **keeps working** (`keeps_working`) |
| `retention_every_point` | `run_retention` over units with long history, crash at each point | each unit pruned entirely or not pruned (IU-7); current revision intact |
| `recovery_reentrant` | crash during the recovery of a database with a `ChangeSet` in the WAL (I-11) | second recovery identical |
| `fsync_failure` | `set_fsync_failure` on the commit of a `ChangeSet` | `Poisoned`; after reopening, the `ChangeSet` is either entire or absent; never halfway (I-10) |
| `enospc` | `set_space_limit` / `fill_disk_after` during a `ChangeSet` | clean error without partial effects; the layer does not leave a head without a revision. Reserve 96 B of WAL header space (note #11) |
| `torn_write` | `set_tear_unit` on the frame of a large `ChangeSet` | the torn frame is discarded entirely (WAL-R) |
| `wal_independence` | WAL flush, rotation and recycling (`Inline` profile, note #27), reopening | identical history (I-20) |

The four crash types (`PowerLoss`, `ProcessKill`, torn write, `fsync` failure) and both durability modes are swept as in the failure matrix ([failure-matrix](failure-matrix.md)); one row per layer-specific point is added (e.g. «crash between the head and the `CHANGESET`» is not an observable point because they go in the same frame, and the test proves it: no crash image contains one without the other).

The `TxnCommits<E>` wrapper (testkit) is not reused: here the unit of commit is the `ChangeSet`, not a `WriteBatch`; the sweep wraps `UnitDb` in its own `Step` (`Step::ChangeSet`) in a `harness::units` module.

### 8.3 Real processes

A real `kill -9` on a child process (`kerno-testkit::process`) that runs `ChangeSet`s in a loop, with prefix verification after each death (like `process_kill.rs`), 200 iterations in nightly CI. Without LazyFS (still pending from Phase 1).

### 8.4 Corruption

`bit_flips` and `truncations` on a database with units: every error must be `Corruption`, `Io` or `Poisoned` (I-08); in addition, changing one byte of a `REV_PAYLOAD` **with a valid block CRC** (SST patch with recalculation) must be detected by the content hash (IU-5, `Corruption`). Altered layer marker → `NotAUnitDatabase`/`IncompatibleLayer`, without writing (I-18).

### 8.5 `verify_units`

Read-only function over a snapshot: it walks `10`, `20`, `21`, `30`, `31` and checks IU-2, IU-3, IU-4 and the immutability between two snapshots (IU-1) without going through the read API. It runs after **each** crash of the sweeps in §8.2 and is designed to be exposed as a CLI in Phase 2/Stage 9. Each finding carries the id of the `UNIT-n` rule, like the engine's `verify()`.

### 8.6 Performance

Bench `unit_bench` (in `spikes/storage/spike-bench` or a new benches crate, depending on ADR-0010): loads `unit_updates` (1 unit, N updates), `history_scan`, `as_of_point`, `retention`. It reports p50/p95/p99 and write amplification compared with writing the same payload as a simple key in `KernoDb`.

### 8.7 CI budget

Default `SPIKE_SEEDS` (fast) in PRs; weekly with 1 000 seeds for `changeset_every_point`, `model_crash` and `retention_every_point`, split into a matrix so as not to exceed the 6 h limit per job (phase1-design §6.7).

## 9. Compatibility, performance and what is not touched

### 9.1 Format v1

| Element | Stage 3 |
|---|---|
| Common header, WAL, SST, MANIFEST, recovery | **unchanged** (immutable while v1 exists, ADR-0013) |
| Feature bits | **none new by default**; optional `incompat` «UNITS» only if the owner decides it (§12.3), with vectors and a note in ADR-0013 |
| `layer_version` byte | inside a user value; versioned on its own |
| Multiversion SST / compression / separate values | not used; they remain reserved bits |
| Phase 1 directory vectors | still valid byte for byte |

Guard test: the existing `dir_vectors` and `struct_vectors` pass without regenerating (`cargo test -p kerno-storage --test dir_vectors --test struct_vectors`). If any WP needed to regenerate them, the WP is wrong.

### 9.2 How format changes are avoided in the future

Three routes, in order: (1) new keys under reserved tags and `layer_version`; (2) engine `compat`/`ro_compat` bit if an old reader can ignore it; (3) `incompat` bit (1..62) for what an old reader cannot treat as opaque, with an ADR. The first two do not touch `kerno-storage`.

### 9.3 Expected cost of the layer (to be measured, not promised)

An isolated `update` writes 5 keys in one commit: `10`, `20`, `21`, `31`, `30` (the last one shared by all members). With T1 dominated by `fdatasync` (≈ 92–98 % of the commit latency, note #88), the expected additional cost is in WAL bytes and encoding CPU, not in `fsync`. The hypothesis is checked in WP-E3-08, not taken as given.

### 9.4 `U` metrics series (proposal; the owner approves them **before** measuring, ADR-0012)

| Id | Metric | Reference | Proposed target |
|---|---|---|---|
| U1 | p99 commit of a 1 KiB `update`, `Sync` | simple 1 KiB key in `KernoDb`, same machine and session | reported; suggested bound ≤ 1.5× |
| U2 | `history(id, 100 rev)` p99, metadata only | `scan` of the same number of 100 B keys | ≤ 1.5× |
| U3 | `unit_as_of` p99 with 1 000 revisions | — | reported; if > 5× a `get`, consider the inverted index (§3.5) |
| U4 | write amplification of 1 KiB and 64 KiB `update` | logical bytes written | reported |
| U5 | space per revision after N updates (`history_sim`) | M0 | reported |
| U6 | `run_retention` throughput and p99 of concurrent writes during pruning | — | without widening T1–T3 by more than measured at low load |

### 9.5 T1–T10 do not change

T1–T10 (phase1-design §7.2, [f1-performance-goals](../testing/f1-performance-goals.md)) measure `kerno-storage` and are not edited, reinterpreted or relaxed by this stage. This stage **does not depend** on T1 being met: the state of T1 and the contingency review of ADR-0001 are separate open decisions of the owner. The only thing the layer must guarantee is not to make them worse: there is no change in `kerno-storage` by default, and if a WP required one (e.g. a new metric) it must be isolated in its own WP with the T1–T10 comparison attached (CONTRIBUTING).

## 10. Out of scope for Stage 3

Relations and indexes (Stage 4: no index by type or by field; `scan_units` walks the heads); filter queries and the planner (5); suppression/physical deletion and legal retention (ADR-0009; the stage only leaves the hook from §5.3); encryption (ADR-0006: `Incompatible`); public IDs and tokens (ADR-0005); structural diff and canonicalisation (ADR-0008); deltas; chunking of large payloads; `AsOf::Time`; CLI; export/import (I-30, Stage 9); synchronisation or replication; multiple writers (the single write slot is an engine property, I-06).

## 11. Work packages and acceptance criteria

All are sequential unless indicated. «Done» includes: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the WP's suite and the complete workspace suite green, the test count observed before and after (the new suite is within the runner's globs), and each new test seen to fail by mutation.

| WP | Content | Depends on | Acceptance |
|---|---|---|---|
| **E3-00** Entry gates | Confirm the roadmap's start condition: WP14 integrated, v1 format frozen (`DRAFT` bit withdrawn) and ADR-0013 accepted; decisions 1–3 of §12 closed | — | Owner's minutes; `docs/file-format/v1.md` without `DRAFT`; no later WP starts before |
| **E3-01** ADR and layer specification | Accept ADR-0004 (revision model), ADR-0010 (crate/API), and propose minimal ADR-0008 (hash and opaque bytes); add `Txn::read_seq` to the `kerno-core` trait (with `impl` in `WriteTxn` and `EmulatedTxn`); `docs/file-format/units-v1.md` with byte-by-byte encodings, `UNIT-n` rules and vectors | E3-00 | ADR moves from Proposed to Accepted by the owner; structure vectors generated and an independent decoder (like `spec_decoder`) reproduces them |
| **E3-02** Codecs and keys (on 2026-10-31 the owner limited the first delivery to the integrity of `REV_PAYLOAD`, `units-v1.md` §5.1; the remaining codecs are still pending) | Crate `kerno-units`: key/value encoding, `UnitModel`, `Clock`, `UnitIdSource`, errors | E3-01 | Round-trip and lexicographic order = numeric order (property); decoding rejects each `UNIT-n` rule with the expected error class; 0 changes in `kerno-storage` except the `impl` of `Txn::read_seq` added in E3-01 (`git diff --stat` shows it) |
| **E3-03** `ChangeSet` and CRUD | `open`, `begin_changeset`, `create/update/archive/unarchive/delete`, `get_unit`, limits, sequence prediction | E3-02 | `model_ops` and `seq_prediction` green on `Emulated<LogEngine>` **and** `KernoDb`; I-21 by construction shown in `changeset_every_point` (fast version) |
| **E3-04** History and as-of | `history`, `get_revision`, `unit_as_of`, `state_as_of`, `change_sets_since`, `compare`, `restore` | E3-03 | `model_as_of`, `restore_keeps_later` (I-22), `ordering_ignores_clock` (I-26), `snapshot_vs_retention` partial |
| **E3-05** History policies | `none`/`audit`/`versioned` per type, `set_policy`, `HistoryUnavailable` | E3-03 | `policy_matrix` (IU-9); policy errors are not empty results |
| **E3-06** Retention and GC | `RetentionPolicy`, `run_retention`, `compact`, `RetentionReport` | E3-04, E3-05 | `retention_every_point`, `snapshot_vs_retention` (IU-8), measurable space recovered after `compact` in `history_sim` |
| **E3-07** `verify_units` and corruption | offline verifier, content hash on read, damage | E3-03 | `verify_units` without findings after each sweep; the damage of §8.4 gives `Corruption`/`IncompatibleLayer`; I-18 |
| **E3-08** Crash sweeps | `harness::units`, `changeset_every_point`, `model_crash`, `recovery_reentrant`, `fsync_failure`, `enospc`, `torn_write`, `wal_independence`, real `kill -9` | E3-03…E3-07 | all green with default `SPIKE_SEEDS` and with 1 000 seeds; new rows in `failure-matrix.md`; I-20 and I-21 marked verified in `invariants.md` |
| **E3-09** Benches and `U` metrics | `unit_bench` and targets U1–U6 (approved beforehand) | E3-03…E3-06 | Report `docs/testing/stage3-report.md` with the comparison against the reference; T1–T10 untouched (`git diff` of `docs/testing/f1-performance-goals.md` empty) |
| **E3-10** Closure | Adversarial review of the stage, promotion of IU-1…IU-10, update of the roadmap and of SAD §8 | all | Review report without open blocking items; roadmap updated with the real status; roadmap acceptance criteria met (reconstruct the evolution, restore without destroying later revisions, history without WAL, same atomic boundary in the failure matrix) |

Critical path: E3-00 → 01 → 02 → 03 → 04; 05, 06, 07 can be parallelised after 03/04; 08 after all of them; 09 can start after 06.

**Note of E3-03 (ambiguities resolved with the most conservative option; `crates/kerno-units/src/db.rs`):**

- `UnitConfig` does not carry `kerno_storage::Config`: `kerno-units` does not depend on `kerno-storage` (only as a development dependency, for the tests over `KernoDb`). It carries `engine: Options` from `kerno-core`, and `UnitDb::from_engine` accepts an already open engine with its own configuration. For the same reason there is no `KernoUnits` alias nor the default type parameter `E = KernoDb`.
- A `ChangeSet` creates at most **one revision per unit** (I-26, IU-3); a second operation on the same unit is `InvalidArgument` and leaves the `ChangeSet` usable.
- Lifecycle: `update` requires `Active` (on `Archived` it is `InvalidArgument`); `archive` requires `Active`; `unarchive`, `Archived`; `delete` accepts `Active` and `Archived`. Check order: `NotFound` → `Conflict` (`expected`) → `Deleted` → lifecycle.
- Each operation validates and encodes everything before its first write, so a rejection has no effect. If the engine rejects a write in the middle of an operation (batch limit, I/O), the `ChangeSet` is broken: it returns `ChangeSetClosed` and `commit` aborts it. An engine `InvalidArgument` is translated to `ChangeSetTooLarge`.
- `ChangeMeta`: `actor` and `source` are also copied into every `REV_META`, so their limit is 256 B; an empty `Some` is rejected (the format does not distinguish empty from absent); `purpose` only accepts `Application` (the others belong to E3-05/E3-06).
- `default_policy` only accepts `Versioned` until E3-05 (lifted in E3-05, see its note). With `create_if_missing = false`, an empty database is opened and read as empty, and `begin_changeset` is `InvalidArgument`.
- One clock reading per `ChangeSet`, used by the record and all its revisions. `get_unit` compares the head with the `REV_META` of `head_rev` (IU-2) in addition to `verify_payload`; a mismatch is `Storage(Corruption)`.
- After a failed sequence prediction the handle also rejects reads with `Storage(Poisoned)` until reopened.

**Note of E3-04 (ambiguities resolved with the most conservative option; `crates/kerno-units/src/history.rs` and `ChangeSet::restore` in `db.rs`):**

- `restore(id, from, expected)` requires an `Active` unit (on `Archived` it is `InvalidArgument`, on `Deleted` it is `Deleted`; it does not «undelete»), with the same check order as E3-03. `from` must be in `[oldest_rev, head_rev]` (`RevisionNotFound` above or with `0`, `Pruned` below) and have a payload: restoring a tombstone is `InvalidArgument`. Restoring the head itself or a `Restore` revision is allowed. The new revision copies the payload, `schema_version` and `metadata` of `from` (decision 6) and leaves the unit `Active`; the source payload is verified before being copied.
- `Revision` additionally carries `lifecycle` (deduced from `kind`: `Archive` → `Archived`, `Delete` → `Deleted`, the rest → `Active`), `payload_len` and `metadata`.
- `history`: `from` inclusive, `to` exclusive (§3.5); a `to` beyond the head is trimmed, but a `from` below `oldest_rev` is `Pruned` and is not trimmed silently. `limit = 0` is `InvalidArgument` in `history`, `state_as_of` and `change_sets_since`. Each page checks that the `REV_META`s are contiguous and with strictly increasing `commit_seq` (IU-3); otherwise, `Storage(Corruption)`.
- `unit_as_of`: binary search by `commit_seq`; each probe checks that its `commit_seq` lies strictly between those of its known neighbours and that the head matches `REV_META(head_rev)` (and with revision 1 if there was no pruning). A `REV_META` missing within the retained range is `Corruption`, never `Absent`. `UnitAt::Pruned` is a result, not an error.
- `state_as_of(at, after, limit)` is paginated by `unit_id` and **omits** the units not yet created at `at` (it does not return them as `Absent`).
- `change_set(id)` returns `Option` (`None` if there is no record with that sequence) instead of a new error. `change_set`/`change_sets_since` check that there are exactly `member_count` members with ordinals `0..n`; a non-UTF-8 `reason` is `Corruption`.
- `compare` returns both revisions with verified payload and `same_content` (equal hash and bytes); no structural diff (ADR-0008).
- `snapshot_vs_retention` is partial: it checks that an old `UnitSnapshot` still reads its history while other `ChangeSet`s commit (including restorations and deletions); the retention half arrives with E3-06.

**Note of E3-05 (ambiguities resolved with the most conservative option; `crates/kerno-units/src/db.rs`, `src/history.rs`, `src/model.rs`, `tests/policies.rs`):**

- The restriction of E3-03 is lifted: `default_policy` accepts all three policies. It is the policy written in the layer marker by the first `ChangeSet`; afterwards the one in the marker counts (reopening with another `default_policy` does not change it). The policy of a type is its `LAYER_META` record `policy/<unit_type>` (value `LayerMeta`, same format as the marker) and, if it has none, the one in the marker.
- The name of `LAYER_META` measures at most 64 B, so only types of 1..=57 B (`MAX_POLICY_UNIT_TYPE_LEN`) accept `set_policy`; a longer one is `InvalidArgument` and keeps the default policy.
- `set_policy` is a `Maintenance` `ChangeSet` without members (`member_count = 0`) that only writes that record; it accepts `purpose` `Application` or `Maintenance` (it records `Maintenance`), rejects `Retention`, and setting the current policy also commits. It does not rewrite revisions.
- The policy in force when a revision is written decides what happens to the earlier ones, in the same batch: `audit` rewrites the `REV_META` of the replaced head with `payload_present = 0` **keeping hash and length** (§3.4, «metadata and hash history», is followed and not the §2.4 note that asked for the hash of the empty payload) and deletes its `REV_PAYLOAD`; `none` deletes with two `delete_range`s all the `REV_META`/`REV_PAYLOAD` of the unit below the new one and sets `oldest_rev` to the new revision. Payloads from a `versioned` period prior to a change to `audit` are not deleted (only the replaced head); they become inaccessible by the read rule and retention (E3-06) will collect them. In this way IU-9 holds for every replaced revision under `none`/`audit`.
- Reads consult the policy in force **now** for the type. `HistoryUnavailable { policy }`: `get_revision`/`compare`/`restore` of a revision below the head under `none`/`audit` (after `RevisionNotFound` and before `Pruned`); `history` always under `none`, and with `with_payload` always under `audit` (without metadata it does work); `unit_as_of`/`state_as_of` between creation and the last revision under `none`/`audit` (before creation it is still `Absent`; from the last revision on, the head answers). `state_as_of` fails entirely if one unit of its page fails. Under any policy, reading the content of a non-tombstone revision with `payload_present = 0` is `HistoryUnavailable { policy: Audit }`.
- After going back from `none` to `versioned`, the deleted revisions remain below `oldest_rev` and are reported as `Pruned` (it is not distinguished from retention). The head can always be restored (it copies the current one) under any policy.
- `UnitModel` models the policies (`with_default_policy`, `set_policy`, `oldest`, `ModelRevision::content_kept`) and answers reads with policy (`read_revision`, `read_history`, `read_as_of`, `read_state_as_of`, `read_restore_source`); the earlier queries keep answering from the complete logical history.

**Note of E3-06 (ambiguities resolved with the most conservative option; `crates/kerno-units/src/retention.rs`, `ChangeSet::restore` in `src/db.rs`, `src/history.rs`, `src/model.rs`, `tests/retention.rs`, `tests/history.rs`, `tests/policies.rs`):**

- API: `RetentionPolicy { default, by_type }` assigns each type a `RetentionRule` (`KeepAll` by default, `KeepLast(n ≥ 1)`, `KeepFor(Duration)`, `KeepSince(CommitSeq)`); `KeepLast(0)` is `InvalidArgument`. `run_retention(&policy, RetentionBudget { max_units ≥ 1, max_ops ≥ 4 })` does not receive `ChangeMeta`: the `CHANGESET` carries `purpose = Retention` with no reason and no actor, and no application `ChangeSet` or `set_policy` can declare that purpose.
- The batch also respects `UnitConfig::max_changeset_ops` (§4): effective `max_units` = `min(max_units, max_changeset_ops)`; if units remain, `complete = false` and the next call continues.
- **One call = one `ChangeSet` = one `Txn`** with all the units of the batch (not one `Txn` per unit): each unit is still pruned entirely or not at all (IU-7) and the whole batch is atomic. The budget counts writes per unit (4 if it loses revisions: two `delete_range`s, head and member; 1 if it only loses payloads, plus 2 per payload), not the marker or the `CHANGESET`. No cursor: each call walks the heads from the beginning, and units with no work only cost reads; `complete = false` asks to repeat. An empty database or one with nothing to prune writes nothing (`change_set = None`).
- What is kept: always the head; if the head is a tombstone, also the previous revision (except under `none`, which only keeps the head anyway). `KeepSince(s)` and `KeepFor(d)` additionally keep the revision **in force at the cut point** (the last with `commit_seq ≤ s`, or the one before the first with `created_at ≥ now − d`), so that as-of reads from the cut keep answering; only a prefix is pruned, even though the clock is not monotonic (`created_at` is descriptive, I-26). `now` is the clock reading of the retention `ChangeSet` itself.
- Members of a `Retention` `CHANGESET`: one per unit touched, `(unit, k, kind of k)`, where `k` is the new `oldest_rev` (the first retained revision). Its `commit_seq` is not that of the `ChangeSet`: IU-4 is checked in the direction `REV_META → member`, and `verify_units` (E3-07) must treat `Retention` members that way. The head only changes in `oldest_rev` (`updated_seq` does not move: retention creates no revisions).
- Policy of history applied to what is stored (pending E3-05): under `audit`, every retained revision below the head that still has a `REV_PAYLOAD` (from an earlier `versioned` period) loses it and its `REV_META` is rewritten with `payload_present = 0` keeping hash and length, just as `audit` does for the replaced head (it is, like that one, the only rewrite of `REV_META`; IU-1 is bounded to «only the policy changes `payload_present`»). Under `none`, everything below the head is pruned (`oldest_rev = head_rev`). Both clean-ups happen even when the rule is `KeepAll`; those under `audit` may be spread across calls (each payload is atomic with its `REV_META`).
- `history` with `from = None` and `to ≤ oldest_rev` when there was pruning (`oldest_rev > 1`) returns `Pruned` instead of an empty page (also when paging `Desc` to the end of what is retained). With `oldest_rev = 1` it is still empty.
- `restore` applies `check_payload` against the **current** `max_payload_len`: the restored revision is new content and obeys the current limit, like `update` (`PayloadTooLarge` without effects). `archive`/`unarchive` do not apply it: they copy the already accepted head and must not block the lifecycle.
- Errors: a missing `REV_META` or one with out-of-order `commit_seq` within `[oldest_rev, head_rev]` is `Storage(Corruption)` and the call writes nothing; never «nothing to prune».
- `compact()` requires `E: TestHooks` (the only generic access to `compact_range` in `kerno-core`; implemented by `KernoDb`, not by `Emulated<LogEngine>`) and compacts `[0x20, 0x22)`. `CompactReport` gives the bytes of live tables before and after (sum of the engine's `l<N>_bytes` statistics) and `bytes_reclaimed()`; that is why `bytes_reclaimed` is not in `RetentionReport`, which gives `bytes_logically_freed` (keys and values of `REV_META`/`REV_PAYLOAD` removed, measured without reading payloads). What a snapshot still sees is not reclaimed (I-05). `UnitDb::metrics()` is not part of this package.
- The «space recovered measurable in `history_sim`» is checked with an equivalent load inside `kerno-units` (`history_sim_reclaims_space`, `KernoDb` only) instead of touching `spike-bench`. `retention_every_point` sweeps the commit of the retention and the closure that flushes it to tables.
- `UnitModel::retain`/`retain_within` decides retention independently of the database (same rules and same budget) and `policy_model` compares it in each round; `snapshot_vs_retention` is complete (IU-8).

**Note of E3-07 (ambiguities resolved with the most conservative option; `crates/kerno-units/src/verify.rs`, `docs/file-format/units-v1.md` §5.2, `tests/verify.rs` and the checks added to `tests/unit_db.rs`, `tests/history.rs`, `tests/policies.rs` and `tests/retention.rs`):**

- API: `UnitDb::verify_units()` and `UnitSnapshot::verify_units()` over the layer's snapshot; `verify_engine(&E)` and `verify_snapshot(&S)` over any engine, including one whose marker is damaged (which `open` rejects) or opened read-only; `verify_immutable(older, newer)` for IU-1. They return `UnitVerifyReport { seq, counts, findings, findings_omitted }`; each `VerifyFinding` carries the rule (`VerifyRule::{Structure, Payload, Cross}`, with id `UNIT-n`), its class, the record key and a detail without content (IU-10). `into_result()` converts the report into the error the read API would give: `Storage(Corruption)`, `Storage(Incompatible)` or `IncompatibleLayer` (the first `IncompatibleLayer` finding is preferred: nothing from a newer layer is interpreted). Damage is findings, not errors; only engine errors (`Io`, the engine's own `Corruption`, `Poisoned`) come out as `Err`. At most 1 024 findings are stored and the rest are counted.
- §8.5 asks that each finding carry a `UNIT-n` rule, but the inter-record invariants (IU-1…IU-4, IU-9) had no number: UNIT-29…UNIT-35 are added to `units-v1.md` §5.2. They change no byte and no decoder behaviour; all are `Corruption`.
- A record that does not decode is reported once and excluded from the inter-record checks (it is not reported again as «absent»). Keys with an unknown tag are reported (UNIT-1 `Corruption`, UNIT-2 `IncompatibleLayer`) and counted; the walk covers the whole key space, including the gaps between tags.
- Non-tombstone revision with `payload_present = 0`: it is one that `audit` emptied keeping hash and length (E3-05 note), so for it the part «`payload_present = 0` with `payload_len ≠ 0`» of UNIT-26 does not apply; only the absence of a `REV_PAYLOAD` is required. The current non-tombstone revision must always have a payload (UNIT-31).
- Policies (IU-9): the `LAYER_META` records do not store when the policy changed, and a change rewrites nothing (E3-05), so what an earlier period left is legitimate until retention. It is only checked where the policy in force at write time is safe: the revisions committed **after the last `Maintenance` `ChangeSet`** (the only ones that change policies) were written with the policy in force now. There, `none` requires `oldest_rev = head_rev` and `audit` requires `payload_present = 0` in every replaced revision. Before that point nothing is asserted (it could be a false positive).
- Provenance (IU-4): members of a `Retention` `ChangeSet` are only checked in the direction `REV_META → member`, as the E3-06 note requires (they name the first retained revision); of them only the shape is checked (ordinals `0..member_count`, no repeated unit, at least one). A `REV_META` whose `commit_seq` is a `ChangeSet` that is not `Application` is a finding. `Maintenance` must have 0 members; `Application` and `Retention`, at least one (an empty `ChangeSet` writes nothing). A member of `Application` whose revision is below `oldest_rev` is provenance of a pruned revision (retention or `none`) and is not a finding.
- Chains (IU-3): besides `parent_rev = rev − 1` and increasing `commit_seq`, the lifecycle transitions of the E3-03/E3-04 notes are checked, and that `Archive`/`Unarchive`/`Restore` copy hash, length, schema and metadata from their source when it is retained. `created_at`/`updated_at` are descriptive (I-26) and are not checked.
- Immutability (IU-1): `verify_immutable` only admits the `payload_present` step from 1 to 0 (E3-06 note); any other change to a `0x20/0x21` key present in both snapshots is UNIT-35. Payloads are compared by BLAKE3 so as not to store their bytes.
- Hash on read: all payload reads already did it (`get_unit`, `get_revision`, `history` with payload, `unit_as_of`, `compare`, `restore`, `archive`/`unarchive`); E3-07 does not change read code and tests it with each damage.
- §8.4: the «SST patch with CRC recalculation» is done at the logical plane: the damage is written with an engine batch, so all the engine's checksums are valid and only the layer can detect it; it is equivalent for the layer and does not require touching `kerno-storage`. The file-level `bit_flips`/`truncations` are done over a database with units (`media_damage`): every error is `Corruption`, `Io` or `Poisoned`. `Emulated<LogEngine>` may lose the tail of its truncated record (it is not a durable engine); it is admitted only there, and what remains must verify without findings. In `KernoDb` no rollback is admitted.
- I-18: the verifier only reads a snapshot. It is tested on both engines (keys and sequence unchanged after verifying and after a rejected open) and, on `KernoDb` opened with `AccessMode::ReadOnly`, over the directory byte for byte (only `LOCK` may appear). `kerno-storage` and `kerno-core` were not touched; in `kerno-units` only `Hash` is added to `PayloadRule`.
- The walk reads page by page (32 keys per `scan`) and never keeps payloads, but it does keep decoded and complete all the `UNIT_HEAD`, `REV_META`, `CHANGESET` and `CS_MEMBER` records of the store (metadata included, up to 4096 B each) and the key of each `REV_PAYLOAD`: in the worst case, about 4.6 KB per revision and 2.5 KB per change set. `verify_immutable` also keeps each `REV_META` of the old snapshot (encoded) and a 32 B BLAKE3 digest per `REV_PAYLOAD`. The work depends only on the records that exist, never on the numbers a record declares: a damaged `head_rev` (e.g. 2^63) is reported as UNIT-31 without walking its range. The Phase 2/Stage 9 CLI is out of scope.

**Note of E3-08 (ambiguities resolved with the most conservative option; `crates/kerno-units/tests/harness/units.rs`, `tests/crash.rs`, `tests/process_kill.rs`, [failure matrix §7](failure-matrix.md), [invariants](invariants.md) I-20/I-21, `.github/workflows/ci.yml`):**

- `harness::units` lives in the tests of `kerno-units` (`tests/harness/units.rs`, included as a module by each test file) and not in `kerno-testkit`: the testkit cannot depend on `kerno-units` without a cycle (the layer already depends on the testkit in development), and this way `kerno-storage`, `kerno-core` and `kerno-testkit` are not touched. It offers `Step::{ChangeSet(n), Close, Drop}`, a `Target` per engine and the common checks.
- Oracle, in two parts, because each tests something different. (1) **Byte by byte**: each recovery is compared with the whole store it leaves, after exactly `s` `ChangeSet`s, with a run without failures of **the same layer code** over `Emulated<LogEngine>` (all families: head, `REV_META`, `REV_PAYLOAD`, `CHANGESET`, `CS_MEMBER` and `LAYER_META`). This proves I-21 at the engine boundary (no image contains part of a `ChangeSet`) and that `KernoDb` and the reference engine are interchangeable, but **not** that the layer writes the right thing: a bug in `UnitDb` is on both sides and the comparison does not see it. (2) **Model**: the oracle also applies the same confirmed `ChangeSet`s to the independent `UnitModel` (the revisions are built in the test from the operation and the model's own state, never by reading layer records), and after each recovery the database's public reads must match it: the unit ids, each head (type, schema, lifecycle, revision, `oldest`, metadata, payload), each retained revision with `get_revision` (type, sequence, lifecycle, restore origin, metadata, hash and payload, or the same rejection), each `ChangeSet` with its purpose and members, and the policy of each type. This detects errors in the layer itself: omitting the `CHANGESET` write at sequence 3 in `db.rs` is detected by the model check (as well as by `verify_units`). The model does not check the descriptive fields (`created_at`, `reason`, provenance), which only the byte-by-byte comparison covers. For that reference store to exist, the load is a deterministic function of `(seed, sequence, stored heads)`: each step confirms **exactly one** `ChangeSet` (step `k` = sequence `k`; if `run_retention` has nothing to prune, a filler `ChangeSet` with one creation), the clock is fixed and the ids restart per step (`StepIds`: `k·2^16 + i`). A `ChangeSet` lost in a crash is redone identically after recovery. The load mixes `create/update/archive/unarchive/delete/restore` (from the head, which always has a payload), `set_policy` (the three policies), `run_retention` with `KeepLast(1..3)` and, every 7 steps, a large `ChangeSet` (3–5 payloads of 8–24 KB) that occupies a frame of many sectors and forces a flush within the commit.
- After **each** recovery of each sweep: sequence within the limits, store = reference, `verify_units` without findings, reads = `UnitModel` (includes `get_unit` of all heads, which verifies the hash) and `keeps_working`: one more `ChangeSet`, clean close, power cut and the same checks again.
- Engine: `KernoDb` is opened in the sweeps with `Background::Inline` (same I/O operations in each run, reproducible points, like the engine's sweeps); the real `kill -9` uses the default background threads. `changeset_every_point`, `model_crash` and `recovery_reentrant` run over `Emulated<LogEngine>` and `KernoDb`; `fsync_failure`, `enospc`, `torn_write` and `wal_independence` only over `KernoDb`, because they test the failure semantics of the real engine (`Poisoned`, WAL-R, recycling), which the reference engine does not model.
- Limits of the recovered sequence: `acked ≤ s ≤ attempted` (with `attempted = acked + 1` if the step failed), except for a power cut in `Buffered`, where only a prefix not earlier than the last **clean closure** is required (the testkit's `every_point` sweep does not admit that combination; here it is included with that weaker bound, which still requires complete `ChangeSet`s).
- `changeset_every_point` (complete; the fast version of E3-03 remains in `unit_db.rs`): per seed, a scenario of 12 `ChangeSet`s with clean and dirty intermediate closures, a crash before each mutating operation (+5 margin), 4 combinations of crash type and durability, a tear at 1 B on odd points. `model_crash`: 5 rounds of up to 20 `ChangeSet`s with random crash (1 in 5 rounds, also during recovery) and continuation over the image, as in `crash::run`.
- `recovery_reentrant`: database with 6 `ChangeSet`s only in the WAL, a crash (each type) and, for each mutating operation of its recovery (+2), a crash at that point; the second recovery must give the same store as the uninterrupted recovery. In `KernoDb` it is required that the recovery writes something (otherwise the sweep would test nothing).
- `fsync_failure`: fails, in turn, **each** `sync_data` of a scenario (opening, 10 `ChangeSet`s, closure), of the WAL, of SST or of the MANIFEST, with `KeepPending` and `LinuxDropPages`. `Storage(Io)` is required in the operation that suffers it and `Storage(Poisoned)` in the next `ChangeSet` and in `begin_changeset`; then a crash (alternating power cut and process death) and `N` or `N+1`.
- `enospc`: (a) `fill_disk_after` before each mutating operation of the same scenario; (b) «reserving 96 B of WAL header space (note #11)» is interpreted as a `set_space_limit` equal to the space used plus the 96 B of the header of a new WAL plus a margin of 0, 64, 512, 4 096 or 20 000 B, set after 0, 3, 6 or 9 `ChangeSet`s: the header of a WAL fits but the `ChangeSet` does not. In both, the first error must be `Storage(Io(StorageFull))` and the next `ChangeSet` `Poisoned`; when space is freed the same device is reopened (the process stays alive) and `N` or `N+1` is required. An operation that does not need space may let the scenario finish; that is not a failure. At least one case of (b) must be required to fill the disk.
- `torn_write`: power cut just after the largest WAL write of the large `ChangeSet` 7 and before its `fsync`, with a tear at 512 B and at 1 B; the result must be 6 or 7 complete, and the seed set must discard the frame at least once (with the default seeds: 22 discards and 10 whole frames out of 32).
- `wal_independence`: small history of 4 units (updates, archive/unarchive, delete, restore) which, when written, exists only in the WAL; then only **other** units are written until none of the WALs existing at that moment exists (flush and deletion; with `wal_preallocate`, recycling), and 30 more `ChangeSet`s. `history` and `get_revision` (with verified payload) of the 4 units must be identical after each `ChangeSet`, after `compact`, after reopening and after a power cut, with `verify_units` clean.
- Real `kill -9` (`tests/process_kill.rs`, Linux only): like the engine's, 20 iterations by default (`KERNO_KILL_ITERS`, 200 in the weekly run), `Sync` and `Buffered`, the engine's `verify()` before reopening and `N` or `N+1` with the reference store, reads equal to the `UnitModel` and `verify_units` clean. The CI job `test-spikes` already ran `kerno-units` (the whole workspace except `kerno-storage`) with `SPIKE_SEEDS=1000` in the weekly run; only `KERNO_KILL_ITERS` is added to it.
- Seeds: default values 4 (`changeset_every_point`), 16 (`model_crash`, `torn_write`), 4 (`recovery_reentrant`) and 2 (`fsync_failure`, `enospc`); `SPIKE_SEEDS`, `SPIKE_SEED` and `SPIKE_POINT` work as in the engine's sweeps. Check of 1 000 seeds done locally: `SPIKE_SEEDS=1000 cargo test --release -p kerno-units --test crash` (27 tests green in 7 min 21 s of wall clock locally, 441 s of execution; `torn_write` discarded the frame 1 274 times and found it whole 726 times out of 2 000) and `KERNO_KILL_ITERS=200 cargo test --release -p kerno-units --test process_kill` (2 tests green in 2 min 20 s; recovered sequences 818 in `Sync` and 942 in `Buffered`). With the default seeds, `crash.rs` takes about 2–5 s and `process_kill.rs` about 3–9 s. `retention_every_point` does not depend on seeds (fixed scenario). The time fits comfortably in a 6 h job, so there is no need to split into a matrix (§8.7).
- The byte cost of WAL of an `update` is not measured here (§9, «checked in WP-E3-08»): it is a performance measure and is left for the E3-09 benches. `retention_every_point` (E3-06) already ran `verify_units` after each crash and does not change.

**Note of E3-09 (`crates/kerno-units/examples/unit_bench.rs`, [report](../testing/stage3-report.md), raw output in `spikes/storage/results/2026-11-01-e3/`):**

- Thresholds of decision 12, set before measuring and not adjusted. Command: `cargo run --release -p kerno-units --example unit_bench` (full, ~7.5 min) or `-- --quick` (~3 s, added to the CI `bench-smoke` job; it reports, it does not fail the job over a threshold).
- Results (VM with 4 vCPU, `KernoDb` default, `Sync`, `StdVfs`): **U1 met** (p99 0.557 ms against 0.428 ms, 1.30×); **U2 not met** (0.1061 ms against 0.0580 ms, 1.83×); **U3 warning** (0.0326 ms against 0.0063 ms, 5.13×; 4.37× in another pass; no inverted index added); U4: +339 B of WAL per `update` (1 416 B against 1 077 B with 1 KiB; 1.01× with 64 KiB), which is the measure deferred from E3-08; U5: 1 257.7 B per 1 KiB revision against 1 068.0 B per key (1.178×); **U6 not met**: concurrent write p99 of ~2.4 ms before rising to 5 693 ms during pruning (217.7 ms with a budget of 25 units), because `run_retention` walks all the heads inside the write slot on each call.
- Fix within E3-09 (owner's decision: fix, without lowering thresholds): **U2** with a fast decoding path for `REV_META` (same result as the checked decoder, which still names the rule in errors) and byte comparison of keys in `history`; p99 0.0541 against 0.0367 ms = **1.47×** (1.53× in a repetition: at the limit). **U6** with incremental `run_retention`: planning in a snapshot outside the slot, check of heads and policies under the slot (re-planning of those that changed), resumable cursor in memory, walk bounded to 4 096 heads without work per call, and range deletions before the points. Write p99 during pruning 2.373 ms against 2.069 ms before (1.15×; before the fix 5 693 ms), throughput 1.14×, read p99 1.87×: **still not met**; with 25 units 1.59× / 0.76× / 6.07×. What remains depends on `kerno-storage` (memtable range tombstones without an index), which according to §9.5 requires its own WP with a T1–T10 comparison; the experiment and its numbers are in the report §6.
- `docs/testing/f1-performance-goals.md` unchanged; `kerno-storage` unchanged.

**Note of E3-09b (approved by the owner in PR #13; [report](../testing/stage3-report.md) §8):**

- `kerno-storage` (own WP, §9.5): incremental index of range deletions in the memtable (`covering_seq` `O(log n)`, fragments shared by readers without rebuilding per `scan`); T1–T10 measured before and after in the same session with interleaved binaries, no regressions beyond noise (report §8.2). No format change.
- **Decision 12b (contract of `run_retention`):** a call is no longer exactly one change set. It writes its budget in change sets of at most `RetentionPacing::units_per_change_set` units (16 by default) and after each one pauses `pause_ratio` (9) times the work since the previous pause. Reason: the slot time of a set of 1 000 units (~50 ms) and continuous work in a closed loop were what worsened the writers' p99 (U6). It is kept: each unit is pruned entirely in a single engine commit (I-21, IU-7); a crash leaves a prefix of the call's change sets (`retention_every_point_paced_*` sweeps); repeating is idempotent; `RetentionReport::change_sets` lists all of them and `change_set` is the last one. With `units_per_change_set = usize::MAX` and `pause_ratio = 0` the earlier behaviour is recovered (used by the `model_crash` harness, whose model is one commit per step).
- `history`: policy cache per policy epoch shared by the snapshots (the epoch goes up before and after each commit that may change a policy) and page built in the requested order. Same format on disk.
- Result: U2 met in 2 of 3 full passes (1.67× / 1.44× / 1.32×); **U6 still not met** (write p99 0.98–1.26×, throughput 0.91–0.98×, read p99 0.69–2.04×). No threshold has been changed. What is missing and why is in the report §8.4.


**Note of E3-09c (approved by the owner in PR #13; [report](../testing/stage3-report.md) §9):**

- **Decision 12c (deferred durability of retention and lightweight history view).**
  - *Retention without its own `fsync`.* `Retention` change sets are committed with `Txn::commit_deferred` (new in the contract, with a default implementation equal to `commit`, so that `LogEngine` and other engines do not change). In `KernoDb` a deferred commit is atomic and visible at once but does not ask for `fsync`, not even in `Sync`; it becomes durable with the next WAL `fsync`: that of any later non-deferred commit (which waits for that `fsync` as always), `sync()`, the memtable rotation (which now syncs the previous WAL if it has unsynced frames, also in `Sync`) or `close`. `CommitInfo::durable` is `false` for those sets. Guarantees: the WAL is written in order and recovered as a prefix, so a crash can only remove a tail of retention sets, never one earlier than a durable commit, nor a commit the caller saw as durable, nor durable user data (retention only prunes history already authorised to be pruned). Repeating the retention redoes what was lost (idempotent). In `Buffered` nothing changes.
  - *Change in `kerno-storage` (own WP per §9.5):* the group leader skips the `fsync` when all the accepted commits of the group are deferred; `durable_seq` only advances with an `fsync`. No format change. New sweeps: `kerno-storage/tests/deferred.rs` (crash before each I/O with deferred and durable commits mixed and rotations with unsynced frames, in `Sync` and `Buffered`, power loss and process death) and in `kerno-units` `retention_every_point` (user set after retention: durable together with all the earlier retention), `changeset_every_point`, `model_crash`, `recovery_reentrant`, `fsync_failure` and `torn_write`, which now distinguish the last step recognised as durable. T1–T10 before and after in report §9.
  - *Lightweight history view.* `UnitSnapshot::history_page` / `UnitDb::history_page(id, q)` returns a `HistoryPage` of borrowed `RevisionView`s: same query, same errors and same checks as `history` (key of each entry, each `REV_META` validated, `commit_seq` strictly increasing up to that of the head), without building a `Revision` per entry or copying `metadata`/provenance; metadata only (`with_payload` is `InvalidArgument`). It reads with `Snapshot::scan_visit` (new, default `scan`; `KernoDb` serves it without copying each entry). `history` and its return type do not change; `HistoryPage::to_revisions` gives exactly what `history` returns.
  - *U2 does not change definition:* the verdict still measures `history(id, latest(100))` as decision 12 sets; `history_page` is measured in the same pass and reported separately, without a verdict. If the owner wants U2 to judge the lightweight view, that is a separate decision of theirs.
- CI fix (report §8.5): a valid retention budget with `max_ops` 4 or 5 never advanced over a unit that needed to prune and drop contents; now pruning alone counts as progress.

## 12. Owner decisions

Decisions 1–8 were **decided** on 2026-10-31 ([E3-00 minutes](stage3-record-e3-00.md)), with the conditions recorded there; 12, on 2026-11-01 (12b and 12c, in PR #13); 9–11 and 13 remain open.

| # | Decision | Recommendation of this design | If something else is chosen |
|---|---|---|---|
| 1 | **Start condition**: Stage 3 requires Phase 1 closed (v1 format frozen, ADR-0013 accepted and WP14 completed). Does one start with the `DRAFT` draft or wait for the freeze?<br>**Decided (2026-10-31): satisfied (freeze done).** | Wait: the design does not depend on the format, but test databases created with `DRAFT` are rejected after the freeze | Start in parallel only with E3-01/E3-02 (pure code and documentation), without creating persistent databases |
| 2 | **Crate and API** (ADR-0010): a new `kerno-units` crate generic over `TxnEngine`, or integrate it into `kerno-storage`?<br>**Decided (2026-10-31): new `kerno-units` crate, strict separation from `kerno-storage`, no regressions in T1–T10.** | New crate: isolates the layer from T1–T10 and allows testing it over `LogEngine` | Integrate: simpler, but the whole layer falls under the Phase 1 targets |
| 3 | **Feature bit**: is the layer marker enough, or is an `incompat` «UNITS» reserved so that an old engine does not open the database?<br>**Decided (2026-10-31): on-disk layer marker, no `incompat` bit for UNITS while `kerno-storage` is internal.** | Only the layer marker while `kerno-storage` is internal; a bit if direct KV access is offered to the user | Bit in 1..62 with vectors and a note in ADR-0013 (does not change `format_version`) |
| 4 | **Snapshots versus deltas** (ADR-0004/0008): complete payload per revision until there is a benchmark?<br>**Decided (2026-10-31): complete copy per revision, no deltas until benchmarks justify them.** | Yes; `Delta` reserved in `kind` | Delta from the start: requires fixing the delta format and `K`/`F` before measuring |
| 5 | **Duplicate or not the current payload in the head** (§2.3)<br>**Decided (2026-10-31): a single copy, the cost of 2 `get`s is accepted.** | Do not duplicate (one copy; 2 `get`s) | Duplicate: 1 `get`, ~2× write bytes in `versioned` |
| 6 | **Versioned metadata**: are `metadata` and classification part of the revision?<br>**Decided (2026-10-31): metadata and classification versioned and audited in the revision.** | Yes, in `REV_META` | No: they would mutate only in the head and would not be audited |
| 7 | **Hash**: algorithm and dependency (ADR-0008)<br>**Decided (2026-10-31): BLAKE3-256 with its ADR (ADR-0008); CRC-32C provisional until that decision is complete.** | BLAKE3-256 with ADR; `alg=0` CRC-32C provisional | SHA-256; or postpone the cryptographic hash to Stage 9 |
| 8 | **`AsOf::Time`**<br>**Decided (2026-10-31): `AsOf::Time` out of Stage 3.** | Out of Stage 3 (I-26) | Add it with a documented definition that depends on the clock |
| 9 | **Pruning of `CHANGESET`**: are they always kept (provenance) until ADR-0009? | Yes, ~60 B per revision | Prune them with the revision: loses provenance |
| 10 | **`unit_id`**: internal injectable UUIDv7 (D-05) and mapping to public IDs in ADR-0005 | UUIDv7 | UUIDv4 or a counter per database: loses temporal locality of keys (write effect on the LSM to be measured) |
| 11 | **Payload limit**: 1 MiB by default, adjustable to 64 MiB; chunking? | No chunking in this stage | Chunking: new key tag and GC rules |
| 12 | **Thresholds U1–U6** and whether they are compared with SQLite in addition to the internal reference<br>**Decided (2026-11-01):** U1 (p99 commit of a 1 KiB `update`, `Sync`, against a simple 1 KiB put in `KernoDb`) ≤ 1.5×; U2 (`history(id, 100 rev)` p99 metadata only against a `scan` of the same number of 100 B keys) ≤ 1.5×; both with the same hardware, load, warm-up and configuration, reporting absolute latencies in ms besides ratios. U3 (warning if > 5× a `get`; the inverted index is not added merely for exceeding it, its cost must be justified beforehand), U4 and U5 informative but complete and reproducible (commands, configuration, seeds, raw output). U6: `run_retention` throughput and concurrent write p99 measured before and during pruning with the same load, comparing p99 and throughput; T1–T3 must not get worse beyond what is measured at low load. Internal reference only; SQLite is left for Stage 9. T1–T10 untouched; thresholds are never lowered to declare success. | Approve before measuring (ADR-0012) | — |
| 13 | **Security order against AI** (decision already pending in the roadmap): does not block Stage 3, but fixes whether suppression (ADR-0009) enters before real history accumulates | Bring ADR-0009 forward to before Stage 8 | — |

## 13. Risks

| ID | Risk | Mitigation |
|---|---|---|
| K-E3-1 | The prediction `commit_seq = read_seq + 1` fails (e.g. a future internal commit of the engine that consumes a sequence) | Check after commit with poisoning; `seq_prediction` test; document it in the `TxnEngine` contract; if it breaks, change the engine with a feature bit and not the layer |
| K-E3-2 | Single write slot: a long `ChangeSet` blocks everyone | `max_changeset_ops`, retention in batches, documentation; multiwriter out of scope |
| K-E3-3 | Double write per revision (REV_META + REV_PAYLOAD + member) raises amplification | Metrics U4/U5; the index and the delta are later optimisations behind `layer_version` |
| K-E3-4 | Long snapshots cancel retention | Snapshot metrics visible; report separates «logically freed» from «reclaimed» |
| K-E3-5 | A user layer built on pending ADRs becomes hard to revert | Opaque bytes, `alg` and `kind` reserved, `layer_version`; nothing of ADR-0008 encoded |
| K-E3-6 | Logical deletion is confused with suppression | Explicit documentation (§5.3) and security `docs`; ADR-0009 before promising anything |
