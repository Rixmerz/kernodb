# 1.0.0-alpha.4 pre-release validation

- Commit: 4322a40 (main)
- Date: 2026-11-25
- Gate item: **item 3: outside integration by a locks worker** (RELEASING.md item 3)
- Who: a locks worker (security-auditor role), working as an outside user

## How it was run

Build, as the task asked:
- Python: `maturin build --release` (bindings/python) -> `kernodb-1.0.0a3-cp39-abi3-manylinux_2_39_x86_64.whl`
- Node: `pnpm install --frozen-lockfile && pnpm build && pnpm pack` (bindings/node) -> `rixmerz-kernodb-1.0.0-alpha.3.tgz`

Each was installed into a fresh project outside the repository: a new venv in /tmp with the wheel, and a new npm project in /tmp with the tarball. From then on only README.md, bindings/python/README.md, bindings/node/README.md and the docs they link were read (docs/bindings-coverage.md). Linux x86_64, CPython 3.14 in the venv, Node 22.

The task was a personal notes/bookmarks store:
- `note` units with str and bytes payloads
- a `bookmark` type with a schema (unique `url`, indexed `stars`, `title` indexed later)
- `tag` units with relations
- keyword, substring, hybrid and vector search
- batches, upserts, retention, a read-only reader, verify and compact

Each README claim was checked as an assertion, run, closed, reopened and checked again.

| Run | Checks | Failed |
|---|---|---|
| Python, write phase | 138 | 1 (my own test bug, see below) |
| Python, reopen phase | 18 | 0 |
| Node, write phase | 137 | 0 |
| Node, reopen phase | 14 (+2 probes) | 0 |
| Both README quickstarts, verbatim | 2 | 0 |

The one Python failure was my test, not the product. `read_as_of(n2, 1)` returned `None`, which is correct: commit 1 predates the unit. Rerun with commit 2, it raises `PrunedError` as documented.

Data survived close and reopen in both bindings:
- last commit sequence, unit counts, restored payload, upserted document
- unique and later-created indexes, relations
- fulltext and substring flags, search results
- deleted lifecycle, pruned history, `verify` clean

Every documented error class was raised at least once where the README says, except those listed under "Not checked". That includes CorruptionError/CORRUPTION, from a byte flip in an SST of a copy.

## Where I had to leave the READMEs or guess

1. **To get Node to load at all** I copied `bindings/node/kernodb.linux-x64-gnu.node` into the installed `node_modules/@rixmerz/kernodb/`. This was a guess from the napi-rs convention; no README says it (finding F1).
2. To explain F1 I read the installed `package.json`, `.github/workflows/npm.yml`, `git log`/`git show` of `bindings/node/package.json`, and RELEASING.md.
3. Nothing else needed the source. Every other behaviour was found from the READMEs and docs/bindings-coverage.md.

## Findings

### F1. `pnpm pack` produces a Node package without its native addon. Bug (packaging) + doc fix
- Repro: `cd bindings/node && pnpm install --frozen-lockfile && pnpm build && pnpm pack --pack-destination /tmp/t`. Then, in an empty npm project, `npm install /tmp/t/rixmerz-kernodb-1.0.0-alpha.3.tgz` and `node -e "import('@rixmerz/kernodb')"`.
- Result: `Cannot find native binding. npm has a bug related to optional dependencies (https://github.com/npm/cli/issues/4828). Please try npm i again after removing both package-lock.json and node_modules directory.`
- Why:
  - The tarball lists `binding.js, index.d.ts, index.js, LICENSE-*, package.json, README.md` and no `*.node`.
  - `files` excludes it, and there are no `optionalDependencies`.
  - The packed `package.json` has no `prepublishOnly`: pnpm pack drops it, though the repo file has `napi prepublish -t npm --no-gh-release`.
  - The addon only reaches users through the per-platform packages that `npm.yml` builds (`napi create-npm-dirs`, `napi artifacts`, `npm publish`).
- The loader's message blames an npm bug that does not apply here.
- "Build from source" in bindings/node/README.md never says how to use a local build from another project.
- Fix:
  - say in the Build from source section that `pnpm pack` alone is not installable, and give a working local path;
  - or include the host `.node` in a local pack;
  - and change the RELEASING.md item 2/3 instructions so they do not rely on `pnpm pack`.
- **Needs validation after publishing:** in an empty directory, `npm install @rixmerz/kernodb@<1.0.0-alpha.4>` and import it. That path was not exercised.

### F2. Node silently ignores unknown options. Bug
- Repro, on a db with deleted and active notes:
  - `db.query({ unitType: 'note', lifecycle: 'deleted' })` returns 7 active units.
  - `db.history(id, { order: 'asc' })` returns `[2,1]`.
  - `db.history(id, { toRevision: 2 })` returns `[2,1]`.
- The Node README says the API "follows the Python binding", where all three options exist.
- A Python user porting code gets wrong data with no error.
- docs/bindings-coverage.md lists none of these as gaps. Its `query` row only notes that Node returns active units.
- Fix: reject unknown option keys with `INVALID_ARGUMENT`, or implement them. Add `order`, `toRevision` and `lifecycle` to the coverage gaps.

### F3. Unit fields differ between bindings, undocumented. Doc fix (parity gap)
- Node `Unit` keys: `id, unitType, schemaVersion, lifecycle, revision, createdAtUs, updatedAtUs, payload`. There is **no `oldestRevision`**.
- Python `Unit`: `id, lifecycle, oldest_revision, payload, revision, schema_version, unit_type`. There is **no `created_at`/`updated_at`**.
- The Python README tells users to compare with `db.read(id).oldest_revision` before an old read. A Node user has no equivalent and must call `history` or catch `PRUNED`.
- The coverage page marks `get` -> `read` as Full in both.
- Fix: document the difference in the coverage page and the Node README, or add the missing fields.

### F4. Hybrid `score` is about 1e9 larger in Node than in Python. Doc fix
- Repro, one keyword leg on "beta" over two notes:
  - Python `hybrid_search("beta")` gives scores `0.016393442, 0.016129032`.
  - Node `hybridSearch({ text: 'beta' })` gives `16393442, 16129032`.
- The Python README defines the score as the sum of `weight / (rrf_k + rank)`. The Node README only says it is for comparing hits of the same query.
- Fix: state Node's scale, or return the same float.

### F5. After `compact()`, an as-of read of a deleted unit raises a generic error with an internal term. Bug
- Repro (both bindings):
  1. `d = store('note','doomed')`, then `s = last_commit_seq()`
  2. `update(d,'x')`, then `delete(d)`, then `compact()`
  3. `read_as_of(d, s)` / `readAsOf(d, s)`
- Python raises `KernoError: history unavailable under policy Audit`. Node throws `code: 'KERNO'` with the same text.
- The Python Limits section says this read "is then no longer possible" but names no error. Neither errors table lists the case, and "policy Audit" appears in no doc.
- Fix: map it to `PrunedError`/`PRUNED` or another named class, word the message for users, and document it.

### F6. Numeric lower bounds contradict the messages and the docs. Bug (messages) + doc fix
- Python `history(id, limit=-1)`: "limit must be a whole number from 0 to 2**64 - 1, not -1". But `limit=0`: "limit must be at least 1".
- Node `history(id, {limit: 1.5})`: "limit must be a whole number, 0 or more", while 0 is refused.
- `max_batch_ops=-1`: "from 0 to 2**64 - 1". But `max_batch_ops=0` and `2**20+1` give "max_changeset_ops 0 is outside 1..=1048576". Node is the same with `maxBatchOps`.
- The Python Limits section says "a counter, limit or revision argument is a whole number from 0 to 2**64 - 1".
- The Node README never states the 1..2^20 range of `maxBatchOps`.
- Fix: messages that state the real range and name the argument the user passed (`max_batch_ops`/`maxBatchOps`, not `max_changeset_ops`), plus the ranges in both READMEs.

### F7. Error messages leak internal or other-language names. Doc fix / message fix
All of these carry the right class or code, so matching on code works. Only the text is confusing:
- `refresh` on a read-write handle (both): "refresh needs a handle opened with AccessMode::ReadOnly" (a Rust type).
- Node: `store(undefined, 'x')` gives "Failed to convert JavaScript value `Undefined` into rust type `String`".
- Node: `store('note', null)` gives "Value is none of these types `String`, `Vec<u8>`, " (it ends in a dangling comma).
- Node: `commit(null)` gives "Failed to get Array length".
- Node errors name Rust/Python identifiers:
  - "turn it on for a type with set_fulltext" / "set_substring"
  - "search_text: limit must be at least 1"
  - "text_types is set but the keyword leg does not run"
  - "graph leg: max_depth 5 is outside 1..=4"
  - The Node PRUNED message already says `readAsOf`, so binding-specific text is possible.
- `relate(a, b, 'a\0b')`: "relation_type of 3 bytes is outside 1..=64 or has a NUL". It does not say which.
- `updateDocument` on an unknown id says "no unit dddd...". Every other NOT_FOUND says "unit dddd... not found".

### F8. `compact()` can report `after` above `before`. Doc fix
- Repro: a small store with one deleted unit.
  - Python `compact()` gives `{'before': 1375, 'after': 1380}`.
  - Node gives `{"before":943,"after":944}`.
- Both READMEs describe `after` as the size once space is given back, which suggests it never grows.
- Fix: say that `after` can exceed `before` when there is little to reclaim.

### F9. Python errors table wording. Doc fix
- The `ChangeSetTooLargeError` row ends in "...; a `PayloadTooLargeError`". This is a garbled way of saying it is a subclass: the MRO is `ChangeSetTooLargeError -> PayloadTooLargeError -> KernoError`.
- The upsert section says a non-unique field "raises `InvalidArgumentError`". The errors table says `NotUniqueError`. Both are true, since NotUniqueError subclasses InvalidArgumentError, but say it once and the same way.

### F10. Versions and links not yet at alpha.4. Release observation (doc fix at the bump)
- The wheel is `1.0.0a3`, the tarball `1.0.0-alpha.3`, and all three READMEs and the root "What is new" say alpha.3.
- `kerno.__version__` does not exist, so an outside user cannot check what they installed from Python.
- Root README: the Install section links the `1.0.0-alpha.2` checklist, and the Documentation table has no row for the Node README (it has one for Python).
- docs/bindings-coverage.md:
  - calls the Node package `kernodb` (it is `@rixmerz/kernodb`) and says its Node column was not checked against a built addon;
  - has no rows for `set_substring`/`substring`/`search_substring`, which both bindings have;
  - omits the F2/F3 gaps.
- The Node README H1 is `# kernodb`, but the package name is `@rixmerz/kernodb`.

### F11. Item 3 ran on local builds, not the published packages. Needs validation
RELEASING.md item 3 says the integration is built on the **published** packages (`pip install --pre kernodb`, `npm install @rixmerz/kernodb@alpha`). alpha.4 is not published, so this run used local builds of 4322a40 labelled alpha.3. Repeat the two installs and the quickstarts in empty directories once alpha.4 is published. The npm path matters most, because of F1.

## README claims that held

Everything else checked held in both bindings:
- open/close/reopen, close twice, CLOSED/ValueError after close, `with`
- read-only beside a writer with `refresh`, READ_ONLY, DATABASE_NOT_FOUND (and the directory is not created), LOCKED on a second read-write open in the same process
- store/read/update(ifChanged)/delete(cascade), HAS_RELATIONS, DELETED, NOT_FOUND
- relate(unique)/unrelate/related in and out/relations
- every batch op, including archive/unarchive/restore, upsert as a relation end, reason/actor/source limits, BAD_ENDPOINT, the one-op-per-unit rule, the 10,000-op limit and `maxBatchOps`
- schemas (same fields in another order accepted, SCHEMA_EXISTS), required/type/254-byte refusals (SCHEMA), int/float/bigint rules in Node
- find, find_range (Python), create_index, execute, explain (Python)
- upsert created/unchanged/updated and NOT_UNIQUE
- search_text and every documented refusal, substring search (case-insensitive, bytes needle, 1024-byte cap)
- hybrid search: three legs, weights, graph_gated, dot metric, every range refusal, dimension mismatch
- 1 MiB payload boundary, 64-byte type boundary, wrong-type arguments giving the binding's invalid-argument error (never TypeError)
- lone surrogate: InvalidArgumentError in Python, U+FFFD in Node
- retention: keep_last, keep_since, keep_for; PRUNED through history-from, read_as_of and compare; bad rule refused, null rule accepted
- verify (clean, and storage_error on corruption), CORRUPTION on a read of a damaged block, query(schema_version=)

## Not checked

- The published packages (F11).
- LockedError and refresh across two processes (only same-process handles).
- Thread sharing and GIL release; kill -9 durability; macOS, Windows and aarch64 wheels.
- The alpha.1 upgrade path (no alpha.1 database at hand).
- The query language beyond one `FIND ... WHERE`.
- Python hybrid `rrf_k`/`candidates`/`max_depth`, and Node `visitBudget`/`candidates`/`rrfK` bounds.
- Type stubs (`py.typed`) and `index.d.ts` against a type checker.
- DELETED for "not in the lifecycle the call needs" (e.g. unarchive of an active unit).

No security finding: nothing here crosses a trust boundary. These are integration and doc defects.
