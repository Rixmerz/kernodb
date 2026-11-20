# Physical format

Status: the LSM engine format is specified in **[`v1.md`](v1.md)** and **frozen since 2026-10-31** ([ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) accepted). The `incompat` bit `DRAFT` is retired: `kerno-storage` no longer writes it, and a database created with the draft is rejected with `Incompatible("pre-freeze draft format")` (v1.md §4.3). Changing a byte requires a new `format_version` or a feature bit, vectors and an ADR (v1.md §15).

Contents of the directory:

| File | What it is |
|---|---|
| [`v1.md`](v1.md) | Normative specification of format v1 (directory, common header, WAL, SST, MANIFEST, recovery, semantics, vectors, compatibility, draft history) |
| [`units-v1.md`](units-v1.md) | Keys and values of the units layer v1 (`layer_version = 1`) |
| [`units-v2.md`](units-v2.md) | Units layer v2 (`layer_version = 2`, feature `DERIVED_VECTORS`): vectors persisted as derived data, [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md). Its vectors are checked byte by byte by `crates/kerno-units/tests/derived_vectors.rs` |
| [`units-v3.md`](units-v3.md) | Units layer v3 (`layer_version = 3`): public identifiers, records `0x70`/`0x71` ([ADR-0005](../adr/0005-public-identifiers.md)), with the vector table that `crates/kerno-units/tests/public_ids.rs` checks |
| [`units-v7.md`](units-v7.md) | Units layer v7 (`layer_version = 7`): per-type options, record `0x77`, with the keyword-index opt-out of [ADR-0022](../adr/0022-per-type-fulltext-opt-out.md) |
| [`export-v1.md`](export-v1.md) | Logical export, **frozen** at `format_version = 1` (golden files in `crates/kerno-units/tests/golden/export-v1/`). `export-v0-draft.md` is a pointer kept for old links |
| `crates/kerno-storage/tests/vectors/v1/` | Structure and directory vectors; a CI guard regenerates them in memory and compares them byte by byte ([CONTRIBUTING](../../CONTRIBUTING.md#format-vectors-and-byte-changes)) |

The principles below were extracted from SAD §13 and from what M0 taught before the format was written; `v1.md` complies with them, and they remain the requirements of any future Kerno format.

## Mandatory principles

1. **Magic and version in every persistent file** (and in every commit meta and header). An unknown version is rejected without modifying the file.
2. **Explicit lengths and maximum limits** for every variable structure; no length or pointer read from disk is used without validation against the file size and the format limits.
3. **Checksum over every byte that influences a result.** M0 uses CRC32C; cryptographic integrity (AEAD) is decided in ADR-0006 and the format must reserve space and version for it.
4. **Identifiable commit.** It must be possible to determine unambiguously which is the last valid commit, including after broken or reordered writes (ADR-0002).
5. **Atomic creation.** Files that define the state (meta, manifest) are created or replaced with *temporary file → fsync → rename → fsync of the directory*; an existing file that is empty or short is corruption, not a "new database".
6. **Distinction between broken tail and corruption.** An invalid record at the end of the journal after a crash is a broken tail; after a clean shutdown, or followed by valid records, it is corruption and is reported.
7. **Three independent version axes:** physical format, logical schema and derived data (SAD §13).
8. **Logical export independent** of the physical format, versioned (I-30).
9. **Migrations** with preconditions, resumable or abortable execution, validation afterwards and a restore path from backup; the migration state is persisted.
10. **Versioned test vectors** for each binary structure (example files + expected values), used by the compatibility tests.

## Non-normative references

- Format of the CoW B+ tree prototype: [`spikes/storage/spike-btree/DESIGN.md`](../../spikes/storage/spike-btree/DESIGN.md)
- Format of the LSM prototype: [`spikes/storage/spike-lsm/DESIGN.md`](../../spikes/storage/spike-lsm/DESIGN.md)
