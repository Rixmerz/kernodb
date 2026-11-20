# Contributing to Kerno

## Language

Everything written into this repository is in **English**: code, comments, commit messages, identifiers, API documentation (`///`), architecture documents, ADRs, plans and READMEs.

## Before writing code

1. Check whether the change touches an open decision in [SAD §2.2](docs/architecture/SAD.md#22-decisions-that-must-not-yet-be-assumed-as-resolved). If it does, the matching ADR must be accepted before you code an assumption that is hard to reverse.
2. Identify the affected [invariants](docs/architecture/invariants.md) and the test that verifies them.
3. Changes to the persistent format, the commit protocol or recovery require updating ADR-0002/ADR-0003 and the [failure matrix](docs/architecture/failure-matrix.md).

## Definition of Done (SAD §18.2)

A change is done when:

- the API and error contract is documented;
- the affected invariants are defined and tested;
- the relevant normal **and failure** tests pass;
- the effects on format, migration, history, indexes and backups were assessed;
- the security and privacy consequences were assessed;
- performance metrics exist where they apply;
- the related ADRs are up to date;
- the documentation describes the real behaviour, with no unproven guarantees.

## Local checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude spike-bench
```

To iterate on the product crates only (`kerno-core`, `kerno-testkit` and the `kerno-storage` engine, which runs the shared suite under the name `kerno`):

```bash
cargo test --release -p kerno-core -p kerno-testkit -p kerno-storage
```

Dependency and file-access guards (CI runs them in the `checks` job; each must exit with code 0). They fail closed: if `cargo tree` or `grep` errors (renamed crate, missing directory), the guard fails instead of passing.

```bash
# kerno-core has no dependencies
test "$(cargo tree -p kerno-core -e normal --prefix none | wc -l)" -eq 1
# the engine does not depend at runtime on spikes or test crates
tree=$(cargo tree -p kerno-storage -e normal --prefix none) && grep -q '^kerno-storage ' <<<"$tree" && ! grep -E '^(spike-|kerno-testkit|proptest|loom)' <<<"$tree"
# all file access in the engine goes through Vfs (grep exits with 1 only when there are no matches)
if grep -rn 'std::fs' crates/kerno-storage/src; then false; else test $? -eq 1; fi
```

The randomized suites accept `SPIKE_SEEDS=<n>` (more seeds) and `SPIKE_SEED=<n>` / `SPIKE_POINT=<n>` (reproduce a failure). See the [test plan](docs/testing/test-plan.md).

## Format vectors and byte changes

Everything the engine writes to disk in format v1 is versioned as deterministic vectors: structure vectors (`crates/kerno-storage/tests/vectors/v1/structs/`) and directory vectors (`.../v1/dirs/<case>/`, with an `EXPECTED` file). Two normal tests, run in CI, regenerate everything in memory and compare it byte by byte with what is versioned: `struct_vectors_encode_exact` and `dir_vectors_are_reproducible`.

- **A failing vector is not regenerated to make CI pass.** The failure means a byte of format v1 changed. First find out why: a dependency that changes an iteration order, a configuration default, a file name or a new field are all format changes, whether you meant them or not.
- **If the change is deliberate**, follow the process in [`docs/file-format/v1.md` §15](docs/file-format/v1.md#15-process-for-changes-to-the-draft-and-the-freeze), **in the same PR**:
  1. a row in the "Draft history" table and the update of `v1.md`;
  2. the regeneration of the affected vectors and the matching note in ADR-0013;
  3. a rerun of `spec_decoder_agrees` and of the corruption suites (`dir_vectors_exhaustive_bit_flips`, `dir_vectors_truncate_every_offset`);
  4. if the change touches checksums, the AEAD slot, the header or data erasure, the threat-model review is marked as pending.
- **Regenerating the directory vectors** (rewrites `tests/vectors/v1/dirs/`; only with the deliberate change above):

  ```bash
  cargo test --release -p kerno-storage --test dir_vectors -- --ignored regenerate_v1_vectors
  git diff --stat crates/kerno-storage/tests/vectors   # review which bytes changed
  ```

  Running it twice in a row must leave the same tree; if it does not, there is a source of non-determinism to fix before going on.
- **Adding a vector is always allowed**; add the case to the generator and regenerate. Changing the bytes of an existing one is only allowed through the §15 process.
- **After the freeze** (WP14) the process changes: any byte change requires raising `format_version` or adding a new feature bit, plus vectors and an ADR.

## Rules for engine code

- All file access goes through the `Vfs` abstraction; never use `std::fs` directly in engine code (this is what makes crash and corruption testing possible). A CI guard checks this in `crates/kerno-storage/src`.
- No data read from disk is used without validating lengths, bounds and checksum.
- After a failed `fsync` the operation is not retried on the assumption that the data arrived: the handle is poisoned.
- No home-made cryptography: only standard algorithms from established libraries (SAD §3.7).
- The core must not depend on a network, an AI provider or a model (I-31).

## Dependencies and licences

- Kerno is licensed as `MIT OR Apache-2.0` ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)); every contribution is accepted under those same two licences. New core dependencies must have a compatible licence (MIT, Apache-2.0, BSD, ISC, Zlib or an equivalent permissive one).
- Do not copy code from other projects without reviewing its licence and recording it. Reading the design of reference projects (redb, RocksDB, LevelDB, LMDB, SQLite) is fine; copying their implementation is not.
- Every new core dependency must be justified in the PR (size, maintenance, licence). Dependencies used only in benchmarks live in `spike-bench` or `benchmarks/`.
- `Cargo.lock` is versioned.

## Commits and PRs

- Small commits with an English imperative message (`Add …`, `Fix …`) and a body that explains why.
- A PR that changes performance attaches the comparison with the baseline (ADR-0012).
