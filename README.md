# Kerno

**Versioned Embedded Data Engine** — an embedded, local-first data engine that combines transactional persistence, versioned evolution of data and verifiable integrity, with an extension path toward hybrid retrieval for AI applications.

> Each piece of data is stored once, its derived representations stay consistent, its evolution can be reconstructed, and its access and retention policies are explicit.

## Status

Kerno is under development. **There is no 1.0 release, and no stability promise yet for the file format or the API.** The on-disk format v1 is frozen in the repository ([ADR-0013](docs/adr/0013-formato-v1-y-protocolo-del-motor.md)), and the test vectors fail if one of its bytes changes, but that freeze is not a compatibility promise to users. Whether and when to declare 1.0 is reserved to the project owner.

Stages follow the [roadmap](docs/roadmap/README.md):

- **Phase 0 / M0 — closed** (2026-10-28). The storage decision and ADR-0002 and ADR-0003 were accepted. See the [storage decision](#storage-decision-of-m0) below.
- **Stage 3, units, revisions and provenance — closed** (2026-11-04), with two performance risks accepted by the owner as open (K-E3-8 and K-E3-9).
- **Stage 4, relations and indexes — closed** (2026-11-05).
- **Stage 5, planner and observability — closed** (2026-11-06).
- **Stage 6, first operational adaptation — closed** (2026-11-11). Its one optimization, a result cache, is disabled by default, and no net performance improvement is claimed.
- **Stage 7, associative memory — closed** (2026-11-12). The ranking of associative results is off by default and marked experimental.
- **Stage 8, consolidation and embeddings — closed as engineering work** (2026-11-12). Criterion Q4, retrieval quality with and without the vector signal, is **not demonstrated** and is recorded as not met; it could be checked on only one of the four query classes. A second frozen evaluation set (E8-10) is required before 1.0.
- **Stage 9, hardening — all packages merged, closure written** (2026-11-13). Its entry acta is approved by delegation of the owner, which the owner can revoke, and the entry gate (E9-00) is done. The advance of Stage 9 work brought forward beside Stage 7 (E9A) closed on 2026-11-12. The decisions on public identifiers, encryption and key management, format freezing, the 1.0 declaration and the license text for contributors were reserved for the owner in person on 2026-11-12. On 2026-11-13 the owner, in person, asked the delegate to take the best decision on each pending point, and they were taken by express delegation, revocable by the owner: public identifiers, option 1 of ADR-0005 (the package E9-02 was merged on 2026-11-13, squash commit 2697980, and invariant I-27 is verified); no encryption and no authenticated integrity in 1.0; 1.0 is not declared now, and only after D13 is met and after real use outside the team; the license text for contributors still needs the owner's approval before any external contribution is accepted. The owner approved the Stage 9 closure in person on 2026-11-13.

### Storage decision of M0

**[ADR-0001](docs/adr/0001-motor-de-almacenamiento.md) (accepted 2026-10-28): leveled LSM tree.** Two prototypes, a copy-on-write B+ tree and a leveled LSM tree, passed the same crash, I/O-failure and corruption tests and were compared with the same benchmarks. The LSM tree was chosen. The [M0 report](docs/testing/m0-report.md) has the results.

## Documentation

| Document | Content |
|---|---|
| [SAD](docs/architecture/SAD.md) | Reference architecture and implementation plan (version 0.3.0) |
| [ADR index](docs/adr/README.md) | Architecture decisions and their status |
| [ADR-0001](docs/adr/0001-motor-de-almacenamiento.md) | Storage engine: decision (leveled LSM tree) and M0 evidence |
| [Invariants](docs/architecture/invariants.md) | What no feature may break, and how it is tested |
| [Failure matrix](docs/architecture/failure-matrix.md) | What happens if the process or the power fails at each point of a commit |
| [M0 report](docs/testing/m0-report.md) | Test and benchmark results of M0 |
| [Test plan](docs/testing/test-plan.md) | How the guarantees are verified |
| [File format v1](docs/file-format/v1.md) | On-disk format v1, frozen under ADR-0013 |
| [Threat model](docs/security/threat-model.md) | Assets, adversaries, scope and the limits stated so far |
| [Roadmap](docs/roadmap/README.md) | State by phase and stage |

The linked documents are written in Spanish and remain so; this README is in English.

## Structure

```text
crates/kerno-core/      contract shared by the crates: errors, write batches, durability modes and the Vfs file abstraction
crates/kerno-storage/   storage engine: leveled LSM tree on format v1, WAL, MANIFEST, transactions; all file access goes through Vfs
crates/kerno-testkit/   test kit: Vfs with crash and failure simulation, shared test suites and a reference engine
crates/kerno-units/     units layer over the engine: units, revisions, relations, indexes, planner and derived data
examples/               agent-memory: an example that uses only the public API; it is outside the workspace
spikes/                 M0 prototypes and benchmarks; not the engine, and their formats are not frozen, so do not create databases with them
docs/                   architecture, ADRs, file format, security, testing and roadmap
```

## Development

Requirements: Rust 1.89 or later (the workspace `rust-version`; tested with Rust 1.97) and Linux. The benchmark crates compile SQLite and need a C compiler.

The commands below are the ones the CI runs (`.github/workflows/ci.yml`). The CI also writes the test output to a log and copies its "Outcomes" lines into the job summary; these commands omit that.

```bash
# checks job: formatting and lints
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# checks job: dependency and file-access guards (each must exit with code 0)
test "$(cargo tree -p kerno-core -e normal --prefix none | wc -l)" -eq 1
tree=$(cargo tree -p kerno-storage -e normal --prefix none) && grep -q '^kerno-storage ' <<<"$tree" && ! grep -E '^(spike-|kerno-testkit|proptest|loom)' <<<"$tree"
if grep -rn 'std::fs' crates/kerno-storage/src; then false; else test $? -eq 1; fi

# test-spikes job: all workspace crates except kerno-storage
cargo test --workspace --exclude kerno-storage --all-features --release -- --nocapture

# test-kerno job: the kerno-storage suites in all profiles
cargo test -p kerno-storage --all-features --release -- --nocapture

# bench-smoke job: tiny-scale benchmark run from the repository root
cargo build --release -p spike-bench --features candidates,kerno
target/release/spike-bench suite --engines btree,lsm,kerno,redb,fjall,sqlite --workloads all --scales tiny --reps 1 --dir bench-data --out smoke.jsonl
target/release/spike-bench report --input smoke.jsonl
! grep -q '"error"' smoke.jsonl
cargo run --release -p kerno-units --features test-support --example unit_bench -- --quick
```

The weekly scheduled run sets `SPIKE_SEEDS=1000` and `KERNO_KILL_ITERS=200` for the randomized and `kill -9` tests. Two further jobs run only weekly or on demand: `loom` (the commit-queue models, built with `RUSTFLAGS=--cfg kerno_loom`) and the LazyFS scenario, which needs FUSE (`.github/workflows/lazyfs.yml`). The M0 benchmarks are described in [spikes/storage/README.md](spikes/storage/README.md).

Contribution conventions are in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Kerno is distributed under either of these two licenses, at your option:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in Kerno, as defined in the Apache-2.0 license, is licensed under these two licenses, without any additional terms or conditions.
