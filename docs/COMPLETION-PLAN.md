# Kerno completion plan

This plan covers what is left between the code and the feature list, and the order to do it in. Every row of [FEATURES.md](FEATURES.md) whose status is not IMPLEMENTED has one entry here. Each entry gives what is missing, the work, how it is verified, its size, what it depends on and who acts. Milestone 1 covers two parts of FEATURES.md: the list "Ticked in ROADMAP.md but PARTIAL" and the section "Contradictions between ROADMAP.md and the code".

| Field | Value |
|---|---|
| Date | 2026-11-19 |
| Base | `docs/FEATURES.md` at commit `0974361`, with this sweep's status changes (OPS-05 IMPLEMENTED, OPS-07 PARTIAL): 220 rows, of which 101 are not IMPLEMENTED (PARTIAL 48, EXPERIMENTAL 18, PROPOSED 29, DEFERRED 5, REJECTED 1) |
| Sources read | The files each row cites, plus `ROADMAP.md`, `docs/compatibility.md`, `docs/concurrency.md`, `docs/release.md`, `docs/testing/phase-c-performance.md`, `docs/testing/sqlite-benchmark.md`, `docs/architecture/requirements-1.0.md`, `docs/architecture/stage8-closure.md`, `docs/adr/README.md`, `crates/kerno-storage/fuzz/README.md`, `experiments/narrative-threads/README.md` and `experiments/narrative-threads/results/REPORT.md` |
| What this is | A plan. It changes no code. The edits it names in `ROADMAP.md` and in other files belong to the steps that carry them out |

**How to read the row tables.** Each FEATURES.md row id that is not IMPLEMENTED is the first cell of exactly one row in this plan. Wherever else a row id appears (a "Depends on" cell, a paragraph, the summary), it is a cross-reference, not a second entry. The final section checks this mechanically.

| Column | Meaning |
|---|---|
| Missing | What the code or the evidence lacks today |
| Work | What to change, and the files it most likely touches |
| Verified by | The test, benchmark or document that shows the row is done. Any new branch of CLI, MCP, verifier or read-path code needs a test that a one-line hand mutant turns red; the cell names that mutant |
| Size | S: up to 1 engineer-day. M: 2 to 5 days. L: more than a week. "—" means no engineering work |
| Actor | `engineering`, `owner-only`, or `both` (engineering prepares, the owner decides or acts) |
| 1.0 | `yes` if the row must be done before 1.0 is declared. `no` if it stays as it is in 1.0 (the cell gives the reason) |

As this plan uses the term, **owner-only acts** are:

- publishing to PyPI, npm or crates.io;
- prebuilt binaries (pushing the release tag);
- merging to `main`;
- approving the CLA text;
- accepting or rejecting ADRs;
- approving a change to a criterion;
- declaring 1.0;
- finding real external users;
- providing an idle machine for long fuzzing and quiet benchmarks;
- funding or reopening an experiment.

**Implemented, verified and certified for 1.0.** These are three levels of evidence, not new statuses; the FEATURES.md schema is unchanged.

- **Implemented:** the code exists and a test or measurement in the repository exercises it. This is what IMPLEMENTED means in FEATURES.md.
- **Verified:** the "Verified by" cell of this plan has been run on the current commit and passed: the CI check, the named test, or the named benchmark with its recorded result.
- **Certified for 1.0:** verified, and covered by an owner act: the row's ADR is accepted, any criterion it depends on is approved, and the owner lists it in the 1.0 declaration (for API rows, under the compatibility promise of `docs/compatibility.md`). Only the owner certifies.

A row reaches IMPLEMENTED in FEATURES.md when it is implemented. Its "1.0: yes" cell here is satisfied only when it is certified.

## Summary

| Milestone | Rows | Engineering size | Owner-only items |
|---|---|---|---|
| M1 Truth fixes | 4 (ENG-07, ENG-10, IDX-16, REV-18), plus item T5 (ADR-0021, whose rows are IMPLEMENTED) | Text route (recommended): 5 S. Implement route: ENG-07 M; ENG-10, IDX-16 and REV-18 L each | Choose text or implement for each item; approve `docs/compatibility.md`; accept or reject ADR-0021 |
| M2 1.0 blockers | 9 (ENG-02, ENG-03, INT-05, INT-06, OPS-06, OPS-08, OPS-10, OPS-13, INT-13) | 4 M (crates, binaries, wheels, npm); 1 M (recovery time and SQLite conclusion); 1 S (U2 on disk; the fuzz script is done). U6: S to M for option (a), L for (b), S for (c) | Package names, version, registry accounts and tokens; push the tag; Windows in or out; idle machine; U6 option and its pre-registered criterion; real external users; CLA approval; declare 1.0; merge to `main` |
| M3 Remaining PARTIAL rows and the experimental cache | 38 (35 PARTIAL; 3 EXPERIMENTAL: ADA-07, ADA-12, ADA-14) | 1.0 subset (10 rows): TXN-01 S to M, IDX-06 M (also closes TXN-09), OPS-01 S, OPS-07 S, 4 S documentation edits (IDX-05, IDX-08, SEC-15, INT-03), IDX-12 closes with T5. Post-1.0: 4 S, 9 M, 6 L, and 9 rows with no work | Accept ADR-0007; set a backup retention deadline; confirm the declared limits |
| M4 Narrative search | 24 (15 EXPERIMENTAL, 9 PROPOSED) | None unless the owner reopens it. A second experiment round needs S of engineering. Engine integration after a GO: L | Close or reopen; budget; independent human reviewers and evaluators; accept the ADR that would follow a GO |
| M5 PROPOSED rows | 20 | 1.0 subset: 1 S (SEC-14). Post-1.0: 6 M, 9 L, 4 with no work | Close IDX-15 as rejected; say whether LiveSpec is an external user |
| Out of 1.0 | 6 (5 DEFERRED, 1 REJECTED) | None | Revisit only after 1.0, on the triggers named |
| **Total** | **101** | | |

**Critical path to 1.0:**

1. M1, text route.
2. M2 Phase A publishing and M2 Phase C measurements, run in parallel.
3. The 1.0 subsets of M3 and M5.
4. Phase D (owner).

M4 is off the critical path.

## Decisions the owner must make

1. **T1 to T4: change the text or implement** (rows ENG-10, ENG-07, IDX-16, REV-18). Recommendation: change the text for all four in 1.0. Implementing ENG-10 or REV-18 means locking or format changes late before 1.0.
2. **ADR-0021: accept or reject** (item T5). Recommendation: accept, because the code and its crash tests exist. Rejecting it means removing record families `0x78` and `0x79` before 1.0 freezes them.
3. **Approve `docs/compatibility.md`** and its covered-API list (row ENG-07), once the API-listing check of ENG-07 exists.
4. **U6 (row OPS-08): option (a), (b) or (c).** For (a), also pre-register the statistic, the number of runs, the medium (disk or tmpfs) and the tolerance that separates noise from a real gap. Recommendation: (a) first, then (b) only if (a) still shows a real gap.
5. **Phase A names and version:**
   - names: crate `kerno` on crates.io, distribution `kerno` on PyPI, package `kernodb` on npm. The npm name in `bindings/node/package.json` differs from the other two; confirm it is intended;
   - first published version: recommendation `0.1.0`, so that 1.0 stays the owner's declaration;
   - registry accounts, trusted publishing and tokens.
6. **Windows: in the 1.0 binary set or not** (row ENG-03).
7. **An idle machine** for long fuzzing (row OPS-13) and the quiet benchmark runs (rows OPS-01, OPS-06, OPS-08, OPS-10), with the hours it can be held. **Taken 2026-11-19:** the machine is the owner's own computer; no run counts until its load average is below 1 for the whole measurement, and a run that breaks that condition is inconclusive. See [owner-decisions](owner-decisions.md) and [testing/test-machine](testing/test-machine.md).
8. **ADR-0007: accept** the built-in type index as derived data rebuilt by `rebuild_index` (rows IDX-05, IDX-08). Recommendation: accept, since queries already fall back to the reference path when the index is stale.
9. **Backup retention deadline** (row SEC-15): how long shipped WAL segments and base backups keep suppressed data.
10. **CLA text** (`docs/legal/cla-draft.md`, decision D15): approve, change or replace it. **Taken 2026-11-19:** direction (b) is confirmed (a broad, non-exclusive licence with the right to relicense; no DCO). The text is not approved and stays a draft, not in force. See [owner-decisions](owner-decisions.md).
11. **Real external use** (row INT-13): who the users are, and what "feedback addressed" means. Recommendation: every issue opened through `docs/feedback.md` gets a disposition (fixed, declared limit, or refused with a reason). **Taken 2026-11-19:** INT-13 is BLOCKED, external validation pending. LiveSpec and the owner's own projects do not count, and neither do tests run by agents; the owner will look for three external evaluators. No names are invented, and the requirement is not declared met. See [owner-decisions](owner-decisions.md).
12. **Narrative search** (M4): close it (recommended for 1.0) or fund a second round. If it is reopened, name the independent human reviewers.
13. **Result cache** (rows ADA-07, ADA-12, ADA-14): keep it EXPERIMENTAL and off by default in 1.0 (recommended), or certify it.
14. **Close or keep the rows that are decisions rather than work:** IDX-15 (FTS5) as rejected; ENG-12 (configurable public ids, where ADR-0005 chose option 1); INT-07 (LiveSpec). **Taken 2026-11-19 for INT-07:** LiveSpec does not count as an external user, and INT-07 is BLOCKED. IDX-15 and ENG-12 are not decided here. See [owner-decisions](owner-decisions.md).
15. **Declare 1.0 and merge** `feature/roadmap-phases-a-b` and its successors to `main`.

## M1. Truth fixes

These are the places where `ROADMAP.md` (at the repository root) claims more than the code does. Each item has two options: change the text, or implement. The text options give exact replacement lines for `ROADMAP.md`.

| Item | ROADMAP.md line | What the code does | Row |
|---|---|---|---|
| T1 Cross-process concurrency (also the only entry of "Contradictions") | Line 51, Phase C, ticked: "many readers, one writer, across threads and processes" | One process owns a directory. A second one, even read-only, gets `Locked` (`docs/concurrency.md` §Processes; `crates/kerno-storage/tests/process_lock.rs`) | ENG-10 |
| T2 Compatibility promise status | Line 57, Phase D, ticked: "File format and API compatibility promise" | `docs/compatibility.md` says "Status: proposed"; the version is 0.0.0; its §4 lists the gaps | ENG-07 |
| T3 Local embedding model | Line 39, Phase B, ticked: "a local model or a remote API" | Ships a hashing provider (not semantic) and, behind the `embed-openai` feature, an OpenAI-compatible HTTP client. That client reaches local servers (Ollama, llama.cpp, vLLM, LM Studio), but no model runs in the process (`crates/kerno-units/src/embed/mod.rs`, invariant I-31) | IDX-16 |
| T4 On-disk memory summaries | Line 42, Phase B, ticked: "summarizing old memory" | Summaries are in-memory derived data (decision D8; `docs/architecture/stage8-closure.md`, F-05) | REV-18 |
| T5 ADR-0021 pending | Lines 49 and 50, Phase C, ticked: typed schemas and secondary indexes | Implemented and tested. ADR-0021 is still "Proposed, pending the owner's acceptance", and it adds record families `0x78`/`0x79` to the format | IDX-02 and DM-05 (both IMPLEMENTED, so no row entry here) |

**T5, the two options:**

- **Text:** append to lines 49 and 50: "(implemented; `docs/adr/0021-typed-schemas-and-secondary-indexes.md` is pending the owner's acceptance, and its record families may change until then)".
- **Implement:** the owner accepts ADR-0021, and engineering sets its Status to "Accepted (date)" in the ADR and in its row of `docs/adr/README.md`.
- Size S either way. The acceptance is owner-only.
- If the owner rejects it, then before 1.0 the schema and index code in `crates/kerno-units/src/schema/` and the facade methods `define_schema`, `create_index`, `drop_index`, `find` and `find_range` must change or go (L).

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| ENG-10 | PARTIAL | Item T1. No sharing across processes: a second process, even read-only, gets `Locked` until the process that owns the directory closes the database and releases the lock. Nothing queues or retries the second open; `kerno-server` is the multi-process path | **Text (recommended):** replace line 51 with "- [x] Documented concurrency: many readers and one writer across the threads of one process, with a stated isolation level; one process at a time owns a directory through an exclusive lock: a second process gets `Locked` until the owner closes the database and releases the lock (processes do not take turns automatically); `kerno-server` is the path for concurrent access from several processes (`docs/concurrency.md`)". **Implement:** add a shared lock for read-only opens in `crates/kerno-core/src/vfs/std_fs.rs` (`lock`); add a reader that follows another process's manifest in `crates/kerno-storage/src/manifest.rs` and `version.rs`; make file deletion in `crates/kerno-storage/src/deleter.rs` wait for readers in other processes. This reverses row FUT-01 | Text: `grep -n "threads and processes" ROADMAP.md` prints nothing. Implement: new cases in `crates/kerno-storage/tests/process_lock.rs`: two read-only processes open together; a reader process sees a writer process's commit after it returns; `kill -9` of the writer leaves the reader consistent | S text; L implement | FUT-01 | both | yes (text) |
| ENG-07 | PARTIAL | Item T2. The promise is not in force. Nothing checks that the covered API does not break. The CLI, MCP and bindings name lists are not frozen | **Text:** replace line 57 with "- [ ] File format and API compatibility promise: drafted in `docs/compatibility.md` (status: proposed); in force when the owner approves it and declares 1.0". **Implement (needed before the tick comes back):** (1) Commit the public API listing of the `kerno` crate as `crates/kerno/public-api.txt`, produced by `cargo public-api -p kerno` (a CI tool installed in the job, not a Cargo dependency). Add a CI job `public-api` in `.github/workflows/ci.yml` that regenerates the listing and fails on a diff. Once the crate is on crates.io, add `cargo semver-checks -p kerno` to the same job. (2) Freeze the surface lists in `docs/compatibility.md` §4: `kerno` CLI commands and flags, the seven `kerno-mcp` tools and their arguments, and the Python and Node function names. (3) The owner approves, and the status line becomes "Status: accepted (date)" | The CI job goes red on a hand mutant that deletes `pub fn explain` from `crates/kerno/src/lib.rs`, and green when it is restored. The four lists exist in §4, and each item is cross-checked against `crates/kerno-cli/tests/cli.rs`, `crates/kerno-mcp/tests/e2e.rs`, `bindings/python/tests/test_kerno.py` and `bindings/node/__test__/index.test.mts` | S text; M implement | ENG-02 (version), decision 3 | both | yes |
| IDX-16 | PARTIAL | Item T3. One remote shape; no in-process local model | **Text (recommended):** replace line 39 with "- [x] Pluggable embedding providers: any OpenAI-compatible endpoint, remote (OpenAI, Azure OpenAI) or local (Ollama, the llama.cpp server, vLLM, LM Studio), chosen by the user; no model runs inside the process". **Implement:** a separate crate `crates/kerno-embed-local` that implements `EmbeddingProvider` with an in-process model runtime. Three constraints apply. It cannot live in `kerno-units`, because the core runs no model (I-31). It adds a model runtime and a model download. It must pass the licence policy that `cargo deny` enforces in CI, the same policy that stopped the rustls attempts in this repository | Text: the replacement line names only what `crates/kerno-units/src/embed/tests.rs` exercises. Implement: a test embeds two paraphrases and one unrelated sentence and asserts the paraphrases are closer to each other; `cargo deny check` is green | S text; L implement | none | both | yes (text) |
| REV-18 | PARTIAL | Item T4. Summaries have no on-disk format (decision D8) and do not survive a reopen unless the caller keeps the descriptor | **Text (recommended for 1.0):** replace line 42 with "- [x] Memory hygiene for agents: importance, expiry (TTL) and deduplication; summaries of old memory as in-memory derived data rebuilt from their sources, not persisted". **Implement (post-1.0):** persist summaries through the derived-data registry of ADR-0018 (`crates/kerno-units/src/consolidation.rs`, `derived_stale.rs`). This needs a new layer version and its file-format document (`docs/file-format/units-v7.md`; layer 6 is taken by schemas). Two obligations follow. The sentinel survival measurement for summaries becomes mandatory (finding F-05 of the Stage 8 closure; `crates/kerno-units/tests/derived_suppression.rs`). And `crates/kerno-cli/tests/cli.rs` line 412, which asserts that an ordinary database's `layer_version` equals `MAX_LAYER_VERSION`, must be updated | Text: the line is replaced. Implement: a summary survives close and reopen with no descriptor passed; the sentinel survival test of summaries passes; `cargo test -p kerno-cli --release --test cli` is green | S text; L implement | none | both | yes (text) |

## M2. 1.0 blockers from ROADMAP.md

### Phase A (publishing) and Phase C (performance, fuzzing)

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| ENG-02 | PARTIAL | Not on crates.io. crates.io refuses a crate whose dependencies are not on it. `kerno` depends on `kerno-core`, `kerno-storage` and `kerno-units`: `kerno-storage` and `kerno-units` set `publish = false`, and `kerno-core` inherits the workspace's `publish = false` | (1) Set `publish = true` in `crates/kerno-core/Cargo.toml`, `crates/kerno-storage/Cargo.toml`, `crates/kerno-units/Cargo.toml` and `crates/kerno/Cargo.toml`. The workspace default stays `false`, so no other crate becomes publishable. (2) Give each of the four a `description`, `readme` and `keywords`. (3) Set `[workspace.package] version`, and the `version` of the four path entries in `[workspace.dependencies]`, to the owner's version. (4) Add a CI job `publish-dry-run` to `.github/workflows/ci.yml` that runs `cargo publish --dry-run -p kerno-core -p kerno-storage -p kerno-units -p kerno` (tested with cargo 1.99.1 on 2026-11-19: `cargo publish --help` lists `-p, --package [<SPEC>]  Package(s) to publish`, and `cargo publish --dry-run --no-verify -p kerno-core -p kerno-storage -p kerno-units -p kerno` parsed the four `-p` arguments and stopped only on `` `kerno-core`, `kerno-storage`, `kerno-units` cannot be published `` because `publish = false` is still set; re-run it after step (1) to confirm the dry run passes). The owner then runs the same command without `--dry-run`. Publishing `kerno-storage` and `kerno-units` does not put them under the promise: `docs/compatibility.md` §4 leaves them uncovered | The CI job is green. After publishing, a scratch project depending on `kerno = "<version>"` builds the code of `crates/kerno/examples/document_store.rs` | M | decision 5 | both | yes |
| ENG-03 | PARTIAL | `.github/workflows/release.yml` builds five targets on a `v*` tag, but no release is confirmed. Windows is marked `experimental: true` with `continue-on-error`, and `docs/release.md` says `StdVfs` is Unix-only. Yet commit `e42e3bd` made `crates/kerno-core/src/vfs/std_fs.rs` build on Windows (`seek_read`/`seek_write` loops), and no CI job builds on Windows | (1) Add a `windows` job (`windows-latest`) to `.github/workflows/ci.yml` that runs `cargo build --locked -p kerno-cli -p kerno-mcp` and `cargo test --locked -p kerno-core -p kerno-storage`. (2) If decision 6 keeps Windows and the job is green: remove `experimental: true` and the `continue-on-error` line from `release.yml`, and the section "Known limitation: Windows" from `docs/release.md`. If Windows is out: say so in `docs/release.md` and drop the matrix entry. (3) The owner pushes `v0.1.0-rc1`, then the final tag | The Release run is green, with one archive per kept target plus `SHA256SUMS`. `sha256sum -c` passes on a downloaded archive. `kerno --help` and `kerno-mcp --help` run from the archive on each OS | M | ENG-02 (version), decision 6 | both | yes |
| INT-05 | PARTIAL | No wheel build in CI; not on PyPI | Add `.github/workflows/wheels.yml`, run on `v*` tags and on `workflow_dispatch`. `PyO3/maturin-action` builds `abi3-py39` wheels (the feature `bindings/python/Cargo.toml` already sets) for manylinux x86_64 and aarch64, macOS x86_64 and arm64, and Windows x86_64 if decision 6 keeps it, plus an sdist. Each runner installs its wheel and runs `pytest bindings/python/tests`. A `publish` job uploads through PyPI trusted publishing, from a GitHub environment `pypi` that requires the owner's approval | The wheels workflow is green on `workflow_dispatch`. After publishing, `pip install kerno==<version>` in a clean venv works and the README example runs | M | ENG-03 (same tag), decision 5 | both | yes |
| INT-06 | PARTIAL | No prebuilt native modules; not on npm | Add `.github/workflows/npm.yml`: the napi-rs build matrix for the same targets, `__test__/index.test.mts` on each, then `napi prepublish -t npm` (the script in `bindings/node/package.json`) and `npm publish --provenance` from a GitHub environment `npm` that requires the owner's approval | The workflow is green on `workflow_dispatch`. After publishing, `npm install kernodb@<version>` in a clean directory works and the README example runs | M | ENG-03, decision 5 | both | yes |
| OPS-06 | PARTIAL | U2 (history cost) is closed on tmpfs only | Run `unit_bench --only U2` three times on the release-candidate code, on the idle machine's disk (`--dir` on that disk). Add §2.5 "On a disk" to `docs/testing/phase-c-performance.md`, with the raw outputs under `spikes/storage/results/<date>-u2-disk/` | U2's verdict (p99 within its threshold of the scan reference) holds in 3 of 3 runs; the raw outputs are committed | S | decision 7 | both | yes |
| OPS-08 | PARTIAL | U6 is not met. On tmpfs, the final runs fail 4 of 4 (throughput 0.79–0.90×, write p99 1.04–1.93×). On disk, 2 of 12 verdicts pass, and §3.7.1 shows that about half of that gap comes from comparing a long window with short "before" windows | See "U6: the open decision" below | See below | (a) S to M; (b) L; (c) S | decision 4 | both | yes (one of the three options) |
| OPS-10 | PARTIAL | No measurement of crash-recovery time. The SQLite benchmark (`docs/testing/sqlite-benchmark.md`, run on a heavily shared machine) states no conclusion | (1) Add workload `R1 recovery` to `crates/kerno-units/examples/unit_bench.rs`: write until the WAL holds 1, 64 and 256 MiB unflushed, kill the child with `kill -9` through `crates/kerno-testkit/src/process.rs`, then time `Db::open` up to the first read. Run 5 repetitions per size and print p50 and max. (2) On the idle machine, rerun `spikes/storage/run-sqlite-comparison.sh` with `SCALES=small REPS=5`. Add §6 "Conclusion" to `docs/testing/sqlite-benchmark.md` giving each workload one verdict (better, worse or no difference), using the disjoint-range rule its §4.4 already applies | The R1 output and its table are committed in `docs/testing/phase-c-performance.md` §5; §6 of the SQLite report exists, with one verdict for every workload | M | decision 7 | both | yes |
| OPS-13 | PARTIAL | CI runs each fuzz target for 60 s weekly. The Stage 9 runs lasted 30 min each on a loaded machine (load about 4 on 4 CPUs). `wal`, `manifest` and `open` reached only 4.5 to 6.7 million executions. **The long run has NOT been executed yet** | Engineering, done: `scripts/fuzz-long-run.sh` runs the targets one at a time with the README's command (`-max_len=65536 -timeout=1`), logs `uptime` every 10 minutes, and writes the count per target. `docker/fuzz/` (`run.sh`, `Dockerfile`) runs the same script in Docker, one fuzz process per dedicated core; see [docs/testing/fuzz-long-run.md](testing/fuzz-long-run.md), section "Running in Docker". Owner: run it, on an idle machine (load average below 1 for the whole run) or in Docker on dedicated cores (the owner's 2026-11-19 decision covers fuzzing only), with `wal`, `manifest` and `open` for 8 h each and the other 12 targets for 30 min each: about 30 h in total | A section "Long run (date)" in `crates/kerno-storage/fuzz/README.md` shows every target at or above its E9-14 execution count (the ROADMAP's "Stage 9 execution counts or more"), no crash, and the load log attached. Any crash is fixed and its input added to the corpus | Engineering done; owner machine time only | decision 7 | both (the run is owner-only) | yes |
| INT-13 | PARTIAL | `examples/agent-memory/` is an example application, and its only use is internal. Phase D asks for real use outside the team | Owner: find at least one user outside the core team (decision 11). Engineering: triage each report opened with the template in `docs/feedback.md`, give it a disposition, and keep a table in `docs/feedback.md` (issue, disposition, commit). Turning the example into a library module is post-1.0 (M) | The table in `docs/feedback.md` has a disposition for every external report, and the owner states that the use is real | — until reports arrive | decision 11 | owner-only, then both | yes |

**Phase D items with no FEATURES row:**

- **CLA:** the owner approves the text (decision 10, `docs/legal/cla-draft.md`). Engineering then points `CONTRIBUTING.md` at the approved text (S).
- **Declare 1.0:** owner-only, after the D13 list in `docs/architecture/requirements-1.0.md`, M1 and M2.
- **Merge to `main`:** owner-only.

### U6: the open decision (row OPS-08)

**What the measurements say** (`docs/testing/phase-c-performance.md` §3.5 to §3.7.1):

- The units-layer cost was already removed (the replan change).
- On disk, a control window of the same length with no retention already runs at 0.96× of the lower "before" window.
- During retention, throughput against that control is 0.95–0.96×.
- Retention holds the write slot for 0.9 % of the time.
- Each retention pass adds about 7 MB of flush and 65–70 MB of compaction on the same device the writers `fsync` to.

So about half of the disk gap is the criterion (a long window compared with short ones). The other half is real work: the engine compacting range deletions.

**Option (a): fix the measurement.**

- **Work.** In the U6 workload of `crates/kerno-units/examples/unit_bench.rs`:
  - make the reference the existing `--control` window: same writers, same length as the retention window, no retention;
  - run 7 rounds per budget;
  - compute the medians of during/control for write p99, throughput and read p99;
  - print them beside the current verdict (during against the lower "before" window), which stays printed and unchanged, and the threshold values stay unchanged too.
- **Pre-register before any run,** in a new §3.8 of the report: the statistic (median of 7), the medium (the idle machine's disk; tmpfs is CPU-bound and showed 0.76–0.90×, so the owner chooses), seed `0xE309`, both budgets, and the tolerance.
- **Cost:** S to M (1 to 3 days), plus about 3 h of machine time per run set.
- **Risk:** it changes the criterion, which can read as moving the goalposts. Mitigation: pre-registration, the owner's approval, and printing both verdicts.
- **Expected result:** a gap of about 4–5 % throughput on disk is likely to remain, because §3.7.1 measured during/control at 0.95–0.96×.

**Rule for a "real gap" after (a)** (proposed, owner approves): the gap is real if the median throughput during/control is below 0.98 (the spread between "before" windows named in §3.4), or the median write p99 during/control is above 1.10.

**Option (b): compaction by tombstone density** in `kerno-storage`.

- **Work.** In `crates/kerno-storage/src/compaction.rs`:
  - pick files by range-tombstone density;
  - drop data covered by a range tombstone at the first level that sees it.
- **No format change is allowed or needed.** Format v1 is frozen (ADR-0013), and the density comes from counts the SST properties already hold (`range_fragments`, `point_deletes` in `crates/kerno-storage/src/format/props.rs`).
- **Tests:**
  - unit tests for the picker;
  - `crates/kerno-storage/tests/compaction.rs`, `compact_range.rs` and `write_amp_levers.rs`;
  - a model test, built on `tests/model.rs`, that no key covered by a tombstone resurrects after compaction;
  - re-baselining the deterministic crash sweeps that number file operations, including `retention_every_point` in `crates/kerno-units/tests/retention.rs`. That test is fragile: its payload must stay under 100 bytes, or a WAL rotation moves into `run_retention` and breaks the deferred-sync assertion.
- **Cost:** L (2 to 3 weeks).
- **Risk:**
  - an engine change late before 1.0;
  - a wrong drop resurrects or loses data;
  - renumbered crash sweeps can hide a real change.
- **After it,** re-run (a).

**Option (c): leave U6 out of 1.0.**

- **Work:** in ROADMAP Phase C, move U6 out of "Closed performance gaps" to a line "Known limit in 1.0: writers lose 5 to 20 % of their throughput while `run_retention` runs". Name the mitigation from §3.6: raise `UnitConfig::retention_pacing.pause_ratio`, or run `run_retention` in quiet hours.
- **Cost:** S.
- **Risk:** the 1.0 claim of "parity with what people expect from SQLite" then carries a stated slowdown.

**Recommendation.** Do (a) first. Do (b) only if (a) shows a real gap by the rule above. Choose (c) if (b) is not done when the owner wants to declare 1.0.

## M3. Remaining PARTIAL rows, and the experimental result cache

### Transactions, indexes and integrity

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| TXN-01 | PARTIAL | Only a write error breaks a ChangeSet (`ChangeSet::write`, `crates/kerno-units/src/db.rs` near line 1121). A re-read of the code at `1b82464` shows no known leak today: the reads FEATURES cites (`create` at line 1320, `create_relation` at line 1532) run before that operation's first write; `apply` plans every read before writing; and `adjust_counts` plans both counts before applying either (`crates/kerno-units/src/adj_count.rs`, lines 300–303). But nothing enforces this, so a later change can bring back the partial effect described in note #960 | Add `fn guarded<T>(&mut self, op: impl FnOnce(&mut Self) -> Result<T, UnitError>) -> Result<T, UnitError>` to `ChangeSet` in `db.rs`. It records `self.writes`, runs `op`, and on `Err` with `self.writes` changed sets `self.broken = true`. Wrap the body of every public mutating `ChangeSet` method with it: `create`, `update`, `archive`, `unarchive`, `delete`, `restore`, `create_relation`, `delete_relation`, and the writing `ChangeSet` methods in `schema/write.rs`, `vector_store.rs`, `fulltext/mod.rs`, `ids.rs` and `derived_stale.rs` | Unit tests in `db.rs`: an operation that writes and then returns `Err` leaves the change set closed (`commit` returns `ChangeSetClosed`); an operation that fails before writing leaves it committable. The hand mutant `if false && ...` on the broken branch turns the first test red. `crates/kerno-units/tests/unit_db.rs` stays green | S to M | none | engineering | yes |
| TXN-06 | PARTIAL | No retry policy. The optimistic check (`expected: Option<RevNo>` on update, archive and delete) exists | Add nothing to the core. Document the retry loop on a revision conflict in the `kerno` crate docs (`crates/kerno/src/lib.rs`), with a doctest | The doctest passes in `cargo test -p kerno --doc` | S | none | engineering | no: documentation only; it can land in any 1.x |
| TXN-09 | PARTIAL | Integrity is checked only when `verify_units` runs by hand | Closes with the IDX-06 work | As IDX-06 | — | IDX-06 | engineering | yes |
| IDX-06 | PARTIAL | A lost index or adjacency entry makes reads return fewer results with no error (K-E3-7, accepted by design; note #948). Only `verify_units` finds it, and the `kerno` facade has no `verify` method | Add `Db::verify(&self) -> Result<VerifyReport, Error>` to `crates/kerno/src/lib.rs`: it runs `verify_units` on a snapshot, and the report type is re-exported. In the crate docs and `crates/kerno/README.md`, document when to run it (after a crash that leaves the index status `Failed`, before an export, on a schedule) and that `rebuild_index` repairs what it reports. The read path keeps its behaviour (K-E3-7 stays accepted) | In `crates/kerno/tests/db.rs`: a clean database verifies clean, and a database with a removed adjacency entry reports that entry. The removal reuses the mechanism `crates/kerno-units/tests/verify.rs` uses to create a missing entry, reached through `Db::unit_db` and the `test-support` feature. A hand mutant that returns the clean report unconditionally turns the second test red | M | none | engineering | yes |
| IDX-05 | PARTIAL | Only `rebuild_index` writes the built-in type index; ordinary writes do not (ADR-0007, status "Pendiente"). A commit makes the index `Stale`, and queries fall back to the reference path | The owner accepts ADR-0007 as implemented (decision 8). Engineering writes its Decision section and status in `docs/adr/0007-index-consistency-and-rebuilding.md` and its row in `docs/adr/README.md`. Maintaining the index on the write path (planned in `apply` like `plan_fulltext`) is post-1.0 (M to L); it changes write amplification and the crash sweeps | The ADR status line; `crates/kerno-units/tests/index.rs` (fallback when stale) stays green | S | decision 8 | both | yes (acceptance) |
| IDX-08 | PARTIAL | No automatic recovery of the built-in index after a crash | Covered by the ADR-0007 acceptance: the index status survives a crash, a query falls back, and the user rebuilds. Post-1.0 option: rebuild on open when the status is `Failed`, behind a `UnitConfig` flag (M) | As IDX-05 | S | IDX-05 | both | yes (acceptance) |
| IDX-12 | PARTIAL | A field exists only when a typed schema defines it | This is the declared design of ADR-0021; no work. The row closes when ADR-0021 is accepted (item T5) and FEATURES names the limit | ADR-0021 accepted | — | T5 | owner-only | yes (acceptance) |
| INT-03 | PARTIAL | Only one embedding adapter shape exists | Closes with the IDX-16 text (one adapter covers OpenAI-compatible endpoints). More shapes are post-1.0, M each, behind cargo features | The IDX-16 text is replaced | S | IDX-16 | engineering | yes (text) |

### Data model, lifecycle and security

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| DM-10 | PARTIAL | The `none` and `audit` policies keep less history, and retention prunes it | By design: the default is `Versioned`, and the reduced policies are opt-in. No work | — | — | none | — | no: declared limit |
| DM-16 | PARTIAL | No API rebuilds unit heads from the revision log | Post-1.0: `UnitDb::rebuild_heads()` in a new `crates/kerno-units/src/repair.rs` scans `REV_META` per unit and rewrites the `UnitHead` records in one maintenance change set. Exposed in the CLI as `kerno repair heads` (`crates/kerno-cli/src/main.rs`) | A test removes heads (`test-support`), runs the repair, and `verify_units` comes back clean. A CLI e2e test in `crates/kerno-cli/tests/cli.rs` goes red on a hand mutant that skips the rewrite | M | none | engineering | no: an additive repair tool |
| ENG-09 | PARTIAL | Older layers are read in place; there is no migration step | Within 1.x, `docs/compatibility.md` promises no migration step: layers are read in place, kept by `crates/kerno-units/tests/layer_fixtures.rs`. Migration tooling belongs to the next major (row STO-20) | — | — | STO-20 | — | no: the 1.x promise needs none |
| ENG-12 | PARTIAL | The public id is a random 128-bit token only | ADR-0005 chose option 1. No work unless the owner reopens ADR-0005 (decision 14); then a `PublicIdScheme` in `UnitConfig` (M) | — | — | decision 14 | owner-only | no: decided by ADR-0005 |
| SEC-05 | PARTIAL | No secret store; tokens and data share a host | A declared limit for an embedded library. `docs/deployment.md` states where the server's tokens file lives and what permissions it needs; add that if it is missing | The section exists in `docs/deployment.md` | S | none | engineering | no: documentation |
| SEC-08 | PARTIAL | Erasure is logical; physical deletion is not guaranteed | Option 3 of ADR-0009 is accepted: a declared limit. A real guarantee needs crypto-erasure, which depends on SEC-01 (out of 1.0) | — | — | SEC-01 | — | no: ADR-0009 |
| SEC-11 | PARTIAL | The fixtures prove reading of old layers, not migrating them | Same as ENG-09 | — | — | ENG-09 | — | no: the 1.x promise needs none |
| SEC-12 | PARTIAL | No operation-level audit log; decision D8 rules out an on-disk audit format in 1.0 | Post-1.0: a new ADR, then an audit record family written per change set (who, what, when) | ADR accepted; a test that an audit record survives a reopen | L | none | both | no: D8 |
| SEC-15 | PARTIAL | No retention deadline for backups (ADR-0009). `kerno-backup` now ships WAL segments and base backups (`crates/kerno-backup/src/ship.rs`), which ADR-0009 predates | 1.0: the owner sets the deadline (decision 9), and `docs/backup.md` states it, together with the fact that a suppressed unit stays in backups until they expire. Post-1.0: `kerno-backup prune --older-than <duration>` in `crates/kerno-backup/src/`, deleting expired base backups and WAL segments (M) | 1.0: the text in `docs/backup.md`. Prune: a test in `crates/kerno-backup/tests/roundtrip.rs` that a restore to a point before the cut-off is refused after a prune | S doc; M prune | decision 9 | both | yes (doc) |

### Relations and revisions

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| REL-04 | PARTIAL | A relation records the commit that created it, but has no history and no binding to a unit revision | Post-1.0: an ADR and a new layer version for relation history | ADR accepted; layer fixture tests | L | REL-03 | both | no: a format change, which a 1.x minor may add as a layer version (`docs/compatibility.md` §2.2) |
| REL-09 | PARTIAL | One path per hit | Post-1.0: an `AssocQuery` option `paths: usize` (maximum paths per hit) in `crates/kerno-units/src/assoc.rs`, enumerated within the visit budget, with the reference route in `crates/kerno-units/src/reference.rs` updated to match | The differential test `crates/kerno-units/tests/assoc_reference.rs` covers the new option and goes red on a hand mutant that stops after the first path | M | none | engineering | no: additive |
| REL-14 | PARTIAL | No derived-relation type; the cache is off by default | Post-1.0: derived relation records in the ADR-0018 registry, made stale when their sources change | Tests on the model of `crates/kerno-units/tests/derived_stale.rs` | L | ADA-07 | both | no: depends on the cache decision |
| REV-14 | PARTIAL | No named snapshot object; restore is per revision or per backup | Post-1.0: a persisted named snapshot, i.e. a pinned `commit_seq` that retention and compaction must keep. Touches `retention.rs` and `crates/kerno-storage/src/compaction.rs` | A retention pass and a compaction keep everything a named snapshot sees | L | none | both | no: additive |

### Performance and operations

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| OPS-01 | PARTIAL | No target-hardware runs (C-09); the recent runs were on a shared machine or on tmpfs | On the idle machine: run `spikes/storage/run-benchmarks.sh` and `unit_bench` in full mode, put the raw logs under `docs/testing/raw/<date>-idle/`, and add one table to `docs/testing/` that describes the machine | The raw logs are committed, and the load log shows load below 1 | S | decision 7 | both | yes: the Phase C benchmark claims rest on it |
| OPS-04 | PARTIAL | No write-amplification figure per workload | Add to the `unit_bench` output, per workload: (bytes flushed + bytes compacted) / user bytes written, from the counters of `crates/kerno-storage/src/metrics.rs`. Report it in the OPS-01 run | The output field exists and is above 1 for every workload | S | OPS-01 | engineering | no: no ROADMAP item asks for it; cheap to fold into the OPS-01 run |
| OPS-07 | PARTIAL | `unit_as_of` cost is measured (workload T1, `docs/testing/phase-c-performance.md` §5.2); `state_as_of` is not measured | Add `state_as_of` to workload T1 in `crates/kerno-units/examples/unit_bench.rs`, with its rows in the §5.2 table at depths 10, 100 and 1000 | The §5.2 table has a `state_as_of` column at those depths, from a run on the idle machine with OPS-01 | S | OPS-01 | both | yes: users size the cost of temporal queries |
| OPS-11 | PARTIAL | No dedicated load-test suite | Post-1.0: a load test of `kerno-server` over HTTP with concurrent tenants, under `crates/kerno-server/tests/` behind `#[ignore]`, run weekly | The weekly job reports p99 per route | M | none | engineering | no: the server is Phase E, outside the 1.0 core |
| OPS-12 | PARTIAL | Long soak runs are not in the release gate | Post-1.0: add a duration env var `KERNO_SOAK_SECS` to `crates/kerno-storage/tests/lifecycle_soak.rs`, and a weekly `soak` job in `ci.yml` that runs it for 2 h | The weekly job is green | S | none | engineering | no: not a ROADMAP item |
| OPS-16 | PARTIAL | The embedded library traces nothing per operation | Post-1.0: `tracing` spans in `kerno-units` behind a feature `trace` that is off by default (`kerno-core` stays dependency-free). The privacy rules of ADR-0015 apply: no payload in a span | A test that a span carries no payload byte (sentinel) | M | none | engineering | no: additive |
| OPS-17 | PARTIAL | Observations stay in memory and are not exported | Post-1.0: an export sink for the `Observer` (a callback), and observation counters on the server's `/metrics` | A test that the sink receives each observation once | M | OPS-16 | engineering | no: additive |
| INT-12 | PARTIAL | Query and operation tracing is partial | Closes with OPS-16 and OPS-17 | As those rows | — | OPS-16, OPS-17 | engineering | no |

### Adaptive optimisation (ADR-0016) and the experimental cache

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| ADA-07 | EXPERIMENTAL | The cache is off by default. E6 p99 on W1 is not certified (`docs/architecture/stage6-closure.md`) | Keep it off in 1.0 (decision 13). Path to IMPLEMENTED, post-1.0: re-run the Stage 6 measurement (the procedure of `docs/architecture/stage6-measurement.md`) on the idle machine. If W1 p99 meets its threshold, the owner decides whether to turn it on by default, and FEATURES then moves the three cache rows | A measurement document with the raw outputs | M | decisions 7 and 13 | both | no: off by default is safe |
| ADA-12 | EXPERIMENTAL | The guarantee holds for the off-by-default cache only | Follows ADA-07 | `crates/kerno-units/tests/cache_sentinels.rs` | — | ADA-07 | — | no |
| ADA-14 | EXPERIMENTAL | Proven for the cache only | Follows ADA-07 | `cache_sentinels.rs`, `cache_reversibility.rs` | — | ADA-07 | — | no |
| ADA-03 | PARTIAL | A query fingerprint exists, but nothing counts recurrence | Post-1.0: a bounded recurrence counter keyed on the fingerprint, in `crates/kerno-units/src/observation.rs` | A test that the counter stays within its bound | M | ADA-07 | engineering | no: ADR-0016 work after the cache |
| ADA-04 | PARTIAL | The route set is fixed in code | Post-1.0: learned routes. Needs ADA-06 and the G1 to G7 guarantees | Differential tests per new route (`crates/kerno-units/tests/query_differential.rs`) | L | ADA-06 | engineering | no |
| ADA-08 | PARTIAL | Preconditions are checked for the cache and planner paths only | The check grows with each new adaptive path; no work while no new path exists | — | — | ADA-04 | — | no |

### Server and replication (section 13 of FEATURES)

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| FUT-03 | PARTIAL | WAL shipping and point-in-time restore only; no live replica, no failover | Post-1.0: a read replica fed by the shipped WAL (`crates/kerno-backup/src/ship.rs`) | A replica serves a read of the latest shipped commit | L | none | both | no: FEATURES section 13 |
| FUT-04 | PARTIAL | Tenants, not users | Post-1.0: users inside a tenant, building on FUT-05 | Server e2e tests per route | M | FUT-05 | both | no |
| FUT-05 | PARTIAL | No roles, no per-operation rights | Post-1.0: roles per token in `crates/kerno-server/src/auth.rs` | An e2e test per route in `crates/kerno-server/tests/http.rs` that goes red on a hand mutant granting every role | M | none | engineering | no |

## M4. Narrative search (section 7): experiment closed with NO-GO

### M4.1 The approved experiment has already run

The experiment approved with about 180 questions and a US$25 budget ran and **closed with NO-GO**. The report is `experiments/narrative-threads/results/REPORT.md`, and `docs/architecture/stage7-record.md` records the closure ("cerrado con NO-GO; no se integra").

| Item | Approved | Result |
|---|---|---|
| Corpus | 5 to 6 documents | 6 documents (`corpus/MANIFEST.json`, with hashes and licences) |
| Questions | about 180 | 180 drafted, 172 frozen (8 dropped, 100 edited) |
| Budget | US$25 (15 extraction, 7 answers, 3 repetitions and evaluation) | US$6.49 spent: extraction 3.55 of 15; questions and evaluation 2.94 of 3; answers 0 of 7. **US$18.51 of the approval is unspent** |
| Review | Independent human | An LLM reviewed it (a declared deviation from the protocol) |
| Verdict | | NO-GO: coverage −14 % (0.437 against 0.507), justifiable relations 72 % |

The GO thresholds, verbatim from the owner's parameters in `experiments/narrative-threads/README.md`:

- evidence coverage at least 10% better than the best baseline (measured: −14 %, fails);
- segment precision no more than 5 percentage points worse (measured: −2.4 pp, passes);
- thread coherence at least 80% positive in human evaluations (not measured);
- justifiable relations at least 85% (measured: 72 %, fails);
- answers without invented evidence at least 95% (not measured);
- reproducibility: same version and configuration, with traceable results (passes).

**What the NO-GO rejects.** It rejects the implementation that was evaluated: coarse segments of about 8 kB from one extraction pass, and threads used to *replace* vector candidates. It does not reject the general hypothesis that narrative structure can improve retrieval. The report leaves one variant untested (threads as a *rerank* on long documents), and nothing in this plan treats that variant as approved.

### M4.2 The path (only on an explicit owner decision)

Nothing below is authorised. A second round, its budget, an ADR and any engine work each need an explicit owner decision, in this order: the experiment must reach GO before an ADR is written, and the ADR must be accepted before any implementation starts.


1. **Gate G1 (owner, decision 12): close or reopen.** Recommendation for 1.0: close it. No ROADMAP item depends on it, and the only positive signal was a small one on one long document (Moby Dick: 0.26 against 0.24, n = 28). Closing means FEATURES relabels the 24 rows as REJECTED with this report as the reason, and nothing else.
2. **If reopened: round 2,** testing the hypothesis REPORT.md names: threads used to **rerank** vector candidates on long documents, not to replace them.
   - **Corpus:** the same six documents (frozen hashes).
   - **Questions:** about 180 **new** questions (30 per document), because the 172 frozen ones cannot be used for tuning. Haiku drafts them; an independent human, provided by the owner, reviews them; they are frozen with SHA-256 by `freeze.py` before any retrieval run (`evaluate.py` refuses otherwise).
   - **Extraction:** re-extract with Sonnet. Round 1's coarse segments (about 8 kB) and its 72 % justifiable relations are properties of the extraction, so reusing it would fail the same criterion.
   - **Baselines:** vector, vector with context, threads as substitution (round 1), and threads as rerank (round 2).
   - **Blind human evaluation:** required this time. Run `answer.py`, then `eval_sheets.py --answers`, then human evaluators who did not write the questions or the threads. This is how thread coherence and invented evidence are measured.
   - **Pass criteria:** the six GO thresholds above, unchanged, on the full set, also reported per document.
   - **Budget:** none is assumed. The unspent US$18.51 can fund it only if the owner says so explicitly; otherwise it needs a new approval: extraction up to 8, questions up to 3, answers up to 7. The hard caps of `ledger.py` apply, and the rule stays "stop at the limit and present partial results".
   - **Engineering:** S (configuration and the rerank strategy in `retrieve.py`). Owner-only: the reviewers, the evaluators and the money.
3. **Gate G2 (owner): only after a round 2 GO, an ADR.** No ADR is drafted for acceptance before the GO. A new ADR, "Narrative segments and threads as derived data", built on the registry of ADR-0018 and the persistence of ADR-0020. Segments, relations and threads are derived data bound to the source revision, rebuilt when it changes. Extraction goes behind a provider trait, like `EmbeddingProvider`, because the core runs no model (I-31).
4. **Engine build, only after the G2 ADR is accepted:** L (4 to 8 weeks). A new module `crates/kerno-units/src/narrative/`, a thread leg in `crates/kerno-units/src/hybrid.rs`, and a frozen-set quality test on the model of `crates/kerno-units/tests/derived_measure.rs`.

Every row below is `1.0: no`. No ROADMAP item asks for narrative search, and the experiment closed NO-GO. The "Work" cell is the work after G2. If G1 closes the topic, no row has any work.

| ID | Status | Missing | Work after a GO (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| NAR-01 | EXPERIMENTAL | Segments are paragraph blocks, not narrative segments, and exist in Python only | `narrative/segment.rs`: a deterministic segmenter with byte ranges | Property test: the ranges cover the payload with no overlap; a golden test against `segment.py` output on the corpus | M | G2 | engineering | no: NO-GO |
| NAR-02 | EXPERIMENTAL | Position and order exist in Python only | A byte range and an order key in the derived segment record | Covered by the NAR-01 tests | S | NAR-01 | engineering | no: NO-GO |
| NAR-03 | EXPERIMENTAL | Phases come from a fixed list, not the spec's weighted sectors | Weights in [0, 1] per segment in the derived record, classified through the extraction provider | Validator tests ported from `validate.py` | M | NAR-01, NAR-04 | engineering | no: NO-GO |
| NAR-04 | PROPOSED | The sector list is fixed | A sector list per unit type in a configuration record | Write and read-back test | M | NAR-03 | engineering | no: NO-GO |
| NAR-05 | PROPOSED | No thematic-depth field | Define the metric in the G2 ADR first; build it only if round 2 shows a need | ADR text | M | G2 | both | no: NO-GO |
| NAR-06 | PROPOSED | Blocks are embedded, not segments | Segment vectors as derived vectors bound to the source revision (ADR-0020) | Tests on the model of `crates/kerno-units/tests/derived_vectors.rs` | M | NAR-01, IDX-16 | engineering | no: NO-GO |
| NAR-07 | EXPERIMENTAL | Threads break at extraction windows | A thread record as an ordered list of segment ids across a document; windows merged on overlap | A thread spans two windows in a test document | L | NAR-10 | engineering | no: NO-GO |
| NAR-08 | PROPOSED | No branches or convergence | The thread as a DAG instead of a chain | A DAG test with a fork and a merge | L | NAR-07 | engineering | no: NO-GO |
| NAR-09 | EXPERIMENTAL | Segments in several threads exist in Python only | A many-to-many segment-to-thread index | Index consistency checked by the verifier, with a hand mutant | S | NAR-07 | engineering | no: NO-GO |
| NAR-10 | EXPERIMENTAL | The LLM assigns relation types; 72 % are justifiable | Relation records with the seven types and verbatim quote ranges validated (port of `validate.py`); kept only if round 2 reaches 85 % | Validator tests; the round 2 figure | M | round 2 GO | engineering | no: NO-GO |
| NAR-11 | EXPERIMENTAL | The `contradicts` label comes from the LLM; there is no detector | The label comes through the extraction provider; no rule-based detector is planned | Covered by the NAR-10 tests | S | NAR-10 | engineering | no: NO-GO |
| NAR-12 | EXPERIMENTAL | The `develops` and `resumes` labels come from the LLM | As NAR-11 | Covered by the NAR-10 tests | S | NAR-10 | engineering | no: NO-GO |
| NAR-13 | EXPERIMENTAL | The `anticipates` and `resolves` labels come from the LLM | As NAR-11 | Covered by the NAR-10 tests | S | NAR-10 | engineering | no: NO-GO |
| NAR-14 | EXPERIMENTAL | The vector baseline is Python; the engine already has vector search and HNSW | Nothing narrative-specific: once segment vectors exist, the engine's `search_vectors` and hybrid search serve them | Covered by the NAR-06 tests | S | NAR-06 | engineering | no: NO-GO |
| NAR-15 | PROPOSED | No intent input | An intent parameter that selects which relation types to follow; needs its own arm in a later experiment | Experiment arm result | M | round 2 | both | no: NO-GO |
| NAR-16 | PROPOSED | No combined score | The round 2 rerank as a fused leg (RRF) in `hybrid.rs` | Frozen-set quality test | M | round 2 GO, NAR-17 | engineering | no: NO-GO |
| NAR-17 | EXPERIMENTAL | Thread expansion as substitution lost coverage (0.437) | Thread expansion as a rerank leg | Frozen-set quality test | M | NAR-07 | engineering | no: NO-GO |
| NAR-18 | EXPERIMENTAL | Neighbour context exists in Python only | A `context_radius` option on narrative results | A unit test on the radius bounds | S | NAR-01 | engineering | no: NO-GO |
| NAR-19 | EXPERIMENTAL | Trajectories are not a user-facing output | A result type `Trajectory` (segments, quote byte ranges, revision) | e2e test through the facade | M | NAR-07, NAR-20 | engineering | no: NO-GO |
| NAR-20 | PROPOSED | No revision binding in the engine | Derived records whose sources are `RevisionId`s (ADR-0018), as embeddings already do | Tests on the model of `derived_stale.rs` | S | G2 | engineering | no: NO-GO |
| NAR-21 | PROPOSED | No invalidation or rebuild in the engine | Reuse the derived states `ready`, `building`, `stale` and `failed` (`derived_stale.rs`) | A stale-on-edit test | S | NAR-20 | engineering | no: NO-GO |
| NAR-22 | PROPOSED | No fallback logic | When no thread is ready, `hybrid_search` behaves as today, with a fallback cause recorded as the planner does (`Fallback`) | A test with no threads returns exactly the plain hybrid result | S | NAR-16 | engineering | no: NO-GO |
| NAR-23 | EXPERIMENTAL | The comparison with chunks ran in Python and closed NO-GO | Round 2 runs it again. In the engine: a frozen-set test like `derived_measure.rs` | Round 2 report; the engine test | M | round 2 | both | no: NO-GO |
| NAR-24 | EXPERIMENTAL | Coherence and invented-evidence rates were not measured | Measured by the blind human evaluation of round 2 | Round 2 report | S | round 2 | both (the evaluators are owner-only) | no: NO-GO |

## M5. PROPOSED rows

Rule used for the 1.0 column. A row is `1.0: yes` if it is cheap, and users need it to judge or size the 1.0 promise. Additive work that changes no frozen surface is `no`, because a 1.x minor release can bring it under `docs/compatibility.md` (a minor release may also add a layer version, §2.2).

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| SEC-14 | PROPOSED | No compliance document in final form. A draft exists (`docs/security/compliance.md`, status: draft); the owner's review is missing | Drafted as `docs/security/compliance.md`: facts only, each naming the file or test that shows it, covering erasure (logical, ADR-0009), retention, export (data portability), encryption (none in 1.0, D3) and audit (per change set only, D8). No legal claims. Left: the owner's review | The owner reviews it | S | none | both | yes: users in regulated domains need the limits before they adopt, and every fact already exists |
| STO-20 | PROPOSED | No migration function or tool | For the next major: `kerno migrate`, which wraps the export with the old build and the import with the new one (the path `docs/compatibility.md` §3 already describes). A converter only if a major changes export v1 | A CLI e2e test of the migration | L | ENG-09 | engineering | no: 1.x needs no migration |
| IDX-03 | PROPOSED | One field per index | `create_index(unit_type, &[fields])` with a composite key in the `FIELD_INDEX` family; needs an ADR-0021 amendment | Tests on the model of `crates/kerno-units/tests/schema.rs` and `schema_crash.rs` | L | T5 | both | no: additive |
| IDX-07 | PROPOSED | No index consolidation | Not defined. Write a problem statement before any work | — | — | none | — | no: no use case stated |
| IDX-14 | PROPOSED | No highlighting or snippets; the facade does not expose `search_text` either | `search_text` returns the byte ranges of the matched terms (`crates/kerno-units/src/fulltext/mod.rs`), and the facade gets `Db::search_text` | A test on known ranges, red on a hand mutant that shifts the range by one | M | none | engineering | no: additive |
| IDX-15 | PROPOSED | FTS5 is not used | No work: Kerno's own inverted index replaces it. The owner closes the row as REJECTED (decision 14) | — | — | decision 14 | owner-only | no: not in the stack |
| IDX-20 | PROPOSED | No metadata filter on the vector leg | `HybridQuery::filter: Option<UnitFilter>` applied to the vector candidates, fetching 4 times k before filtering, in `crates/kerno-units/src/hybrid.rs` | A test in `crates/kerno-units/tests/hybrid.rs`, red on a hand mutant that drops the filter | M | none | engineering | no: additive |
| REL-03 | PROPOSED | No user metadata on a relation | `RelationRecord` gains a `metadata` byte field under a new layer version. `crates/kerno-cli/tests/cli.rs` line 412 must be updated | Layer fixture tests; the CLI test | L | none | both | no: a layer version can come in a 1.x minor |
| REL-12 | PROPOSED | No per-relation weight | A weight read from the REL-03 metadata by the ranking in `crates/kerno-units/src/assoc_rank.rs` | A ranking test | M | REL-03 | engineering | no: additive |
| REV-16 | PROPOSED | No branches or merges | Post-1.0: an ADR first | ADR accepted | L | none | both | no: section 13 |
| FUT-12 | PROPOSED | Branches and merges (same feature as REV-16) | Follows REV-16 | — | — | REV-16 | — | no |
| FUT-06 | PROPOSED | No publish and subscribe | A server change feed built on `change_sets_since` | Server e2e test | L | none | engineering | no: section 13 |
| FUT-07 | PROPOSED | No stream RPC | A server-streaming `Watch` RPC in `crates/kerno-server/proto/kerno/v1/kerno.proto`, on top of FUT-06 | gRPC e2e test | M | FUT-06 | engineering | no |
| FUT-10 | PROPOSED | No columnar storage | No plan and no use case | — | — | none | — | no |
| FUT-13 | PROPOSED | No general graph subsystem | Post-1.0: an ADR first | ADR accepted | L | none | both | no |
| FUT-14 | PROPOSED | No tree-sitter integration | An integration crate outside the core that turns code into units with relations | Integration tests | L | none | engineering | no |
| FUT-15 | PROPOSED | No advanced adaptive optimisation | Follows ADA-06 | — | L | ADA-06 | engineering | no |
| ADA-06 | PROPOSED | Nothing is learned from history | Post-1.0, under the G1 to G7 guarantees of ADR-0016 | Differential and reversibility tests | L | ADA-07 | engineering | no |
| ADA-13 | PROPOSED | Observations are not consolidated | Post-1.0: consolidate the bounded observations into per-fingerprint aggregates | A test that the aggregates stay within their bound | M | ADA-03 | engineering | no |
| INT-07 | PROPOSED | No LiveSpec integration (only a workload name and ADR text) | The owner says whether LiveSpec is an external user (decision 14). If it is, its use counts toward INT-13 and the integration is M | — | M | decision 14 | both | no |

## Out of 1.0: DEFERRED and REJECTED rows

| ID | Status | Missing | Work (files) | Verified by | Size | Depends on | Actor | 1.0 |
|---|---|---|---|---|---|---|---|---|
| SEC-01 | DEFERRED | No encryption at rest | None. Decision D3 (2026-11-13) and the ROADMAP's "Not planned" list keep it out of 1.0; the AEAD feature bit is reserved, so a later build can add it without breaking v1 readers. Revisit when a user has an at-rest encryption requirement, or when physical erasure (SEC-08) needs crypto-shredding | — | — | none | owner-only (to revisit) | no: D3 |
| SEC-02 | DEFERRED | No key management | None; follows SEC-01 | — | — | SEC-01 | owner-only | no: D3 |
| SEC-03 | DEFERRED | No key rotation | None; follows SEC-01 | — | — | SEC-01 | owner-only | no: D3 |
| SEC-04 | DEFERRED | No key lifecycle | None; follows SEC-01 | — | — | SEC-01 | owner-only | no: D3 |
| FUT-01 | DEFERRED | Two processes cannot open one directory | None. The exclusive lock is by design, and `kerno-server` is the multi-process path. Revisit if Phase D feedback asks for several reader processes on one host; that is the implement option of item T1 | — | — | T1 | owner-only (to revisit) | no: by design |
| FUT-09 | REJECTED | No SQL | None. ROADMAP "Not planned": no SQL dialect compatibility as a goal. The `FIND` query language exists | — | — | none | — | no |

## Check

Each row id below is the first cell of exactly one row in this plan. Checked on the documentation sweep on top of commit `0974361`, the HEAD it started from, after its two status changes (OPS-05 to IMPLEMENTED, OPS-07 to PARTIAL). The plan's (id, status) keys were compared with the FEATURES.md rows whose status is not IMPLEMENTED:

    diff <(grep -E '^\| [A-Z]{2,3}-[0-9]{2} \|' docs/FEATURES.md | awk -F'|' '{gsub(/ /,"",$2); gsub(/ /,"",$4); if ($4!="IMPLEMENTED") print $2, $4}' | sort) <(grep -E '^\| [A-Z]{2,3}-[0-9]{2} \|' docs/COMPLETION-PLAN.md | awk -F'|' '{gsub(/ /,"",$2); gsub(/ /,"",$3); print $2, $3}' | sort) && echo IDENTICAL

**Check line: the counts match.** The plan has 101 rows (PARTIAL 48, EXPERIMENTAL 18, PROPOSED 29, DEFERRED 5, REJECTED 1), the same as the rows of docs/FEATURES.md whose status is not IMPLEMENTED.
