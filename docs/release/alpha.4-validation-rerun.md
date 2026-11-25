# kernodb 1.0.0-alpha.4: RELEASING.md item 3, re-run after the fix round

- Commit: main at b0da880. Date: 2026-11-25. Linux x86_64, CPython venv (uv), Node 22.
- Python: `maturin build --release` (CARGO_TARGET_DIR on /home) gave `kernodb-1.0.0a3-cp39-abi3-manylinux_2_39_x86_64.whl`, installed into a fresh uv venv outside the repo.
- Node: `pnpm build` in bindings/node, then `npm install /abs/path/bindings/node` into a fresh `npm init -y` project outside the repo, as RELEASING.md item 2 now says. It loads with no manual copy.
- After installing I read only README.md, bindings/python/README.md, bindings/node/README.md and the docs they link (docs/bindings-coverage.md, the two guides).
  - **Exception 1:** S2–S5 and N8 are findings about the Rust rustdoc. For those I read only the `///` lines of crates/kerno/src/lib.rs.
  - **Exception 2:** for S7 and N4 I read the installed `kerno/__init__.pyi` stub and the runtime `__doc__` strings.
- **The repo was not modified:** `git status` shows only the `bindings/python/uv.lock` that was already untracked. The scratch work (cargo target, wheel, venv, node project, databases) is in `~/.cache/kgate4`, 350 MB on a /home that is 96% full, and can be deleted.

## Runs

| Run | Checks | Result |
|---|---|---|
| Python, write phase | 162 | 162 pass |
| Python, reopen phase | 26 (1 is a placeholder) | 26 pass |
| Node, write phase | 144 (1 is a non-asserting print) | 142 pass, 2 fail (see below) |
| Node, reopen phase | 21 | 21 pass |
| Both quickstarts, run verbatim | 2 | output as documented |

The two Node failures:
- N1 is real and still open.
- `rrfK: 0` was my own wrong expectation. 0 is valid in both bindings: Python's message says `rrf_k` is "from 0 to 2**32 - 1".

**Data survived close and reopen in both bindings:**
- last commit sequence, the active-unit set, str and bytes payloads, the restored payload, the upserted document (an upsert after reopen gives `unchanged`)
- the unique index and the index created later, `find_range` (Python), relations
- the fulltext and substring flags and their search results
- archived and deleted lifecycles, `oldestRevision` 4 with PRUNED, HISTORY_UNAVAILABLE after compact, an as-of read
- the schema still refuses different fields, and `verify` is clean with the same unit count

## Old findings

| Id | Status | Repro run on b0da880 |
|---|---|---|
| F1 | fixed (docs) | The Node README "Build from source" section and RELEASING.md now say to install the directory. `npm install <dir>` loads with no copy. The packaging itself is unchanged: `npm pack --dry-run` still lists 7 files and no `.node`. The npm install after publishing is still unchecked (F11). |
| F2 | fixed | `query({unitType:'note', lifecycle:'deleted'})` returns the 2 deleted units with no payload. `history(id,{order:'asc'})` gives `[1,2]`. Paging with `toRevision` gives `[[7,6,5],[4,3,2]]`. An unknown key throws `INVALID_ARGUMENT`: `history options: unknown option "sideways" (known: ...)`. |
| F3 | fixed | Node `Unit` keys include `oldestRevision`. Python `Unit` has `created_at_us` and `updated_at_us`. The coverage doc lists the same fields for both. |
| F4 | fixed | Node `hybridSearch({text:'beta'})` gives scores `0.016393442, 0.016129032`, the same as Python. |
| F5 | fixed | After `compact()`, `read_as_of(d, s)` raises `HistoryUnavailableError` in Python and `HISTORY_UNAVAILABLE` in Node, with a user-facing message. Both READMEs document it. |
| F6 | **still open (part)** | `limit` and `max_batch_ops`/`maxBatchOps` messages now state the real range ("from 1 to ..."). What is left is listed below. |
| F7 | **still open (part)** | Most messages are fixed: `setFulltext`, `textTypes`, `maxDepth`, `relationType`, "store: unitType is required", "commit: ops is required", the `refresh` message, the `updateDocument` NOT_FOUND text. Still open: `relate(a,b,'a\0b')` in both bindings gives "relation_type of 3 bytes is outside 1..=64 or has a NUL", which does not say which rule failed. See also NEW-3. |
| F8 | **still open** | One store and one delete, then `compact()`: Python gives `{'before': 809, 'after': 828}`, Node gives `{"before":809,"after":827}`. Neither README says `after` can be larger than `before`. |
| F9 | fixed | The Python errors table now says "A subclass of `PayloadTooLargeError`". The upsert section names `NotUniqueError`. |
| F10 | **still open (part)** | Fixed: `kerno.__version__` exists (`1.0.0a3`), the root README links the alpha.4 validation and has a Node row, the coverage doc says `@rixmerz/kernodb` and has substring rows, the Node README H1 is `@rixmerz/kernodb`. Still open: everything still says alpha.3 (wheel, `package.json`, all three banners; expected at the version bump). The coverage doc still says "Neither binding was built or run for this page" and has no `substring(type)` row. |
| F11 | **still open** | `npm view @rixmerz/kernodb dist-tags` shows `alpha: 1.0.0-alpha.3`. PyPI has `1.0.0a1`, `1.0.0a2`, `1.0.0a3`. alpha.4 is not published, so the installs from the registries cannot be run yet. |
| B1 | fixed | `query(unit_type="", lifecycle="archived")` gives `InvalidArgumentError: unit_type must be 1 to 64 bytes, not 0 bytes`. `"t"*65, lifecycle="all"` is refused the same way. |
| S1 | fixed | Same repro as F2: Node `toRevision` and `order` work, and unknown keys are refused. |
| S2 | fixed | lib.rs:987 now says "(1 to 64 bytes, no NUL)". Python `relate(..,"r"*65)` raises `InvalidArgumentError`. |
| S3 | fixed | lib.rs:631 now says "`ChangeSetTooLarge` when the batch has more operations than the limit". Python raises `ChangeSetTooLargeError` with `max_batch_ops=2`. |
| S4 | fixed | lib.rs:466 and :574 now say `NotUnique`. |
| S5 | fixed | lib.rs:846-849: `HistoryUnavailable` is for a compacted deleted unit, and a pruned revision answers `Ok(UnitAt::Pruned { oldest })`. |
| S6 | fixed | Python: `HistoryUnavailableError` from `read_as_of` and `compare`, and `history(with_payload=True)` lists the 3 revisions with `payload=None`. Node: the code `HISTORY_UNAVAILABLE`. Node `history({withPayload:true})` still throws, but that is now a documented difference in the Node README and the coverage doc. |
| S7 | fixed | `Revision.created_at_us` is about 1.7956e15, which is µs. Its docstring says "microseconds since the Unix epoch". `created_at` no longer exists. |
| S8 | fixed | Same repro as F3. |
| S9 | fixed | `read('zz')`, a bad direction, an unknown kwarg, a bad order, `store(None,'x')` and the other refusals I tried raise `InvalidArgumentError`, whose MRO includes both `KernoError` and `ValueError`. A call after close raises `ClosedError`, also a `KernoError` and a `ValueError`. |
| S10 | fixed | Scores are on the same float scale in both bindings (F4). The coverage doc's hybrid row is "Partial" and lists every difference. |
| N1 | **still open (Node)** | Python `read_document(note)` gives `SchemaError: the payload is not a document`. Node `readDocument(note)` gives `code=INVALID_ARGUMENT invalid argument: record: not a record (unknown version byte)`, but the Node README SCHEMA row promises `SCHEMA`. |
| N2 | fixed | Both bindings now refuse the field type `"str"` with `unknown field type "str"`. |
| N3 | fixed | The Python README `hybrid_search` row lists `text_types`. |
| N4 | fixed | The runtime `__doc__` of `upsert` and the installed stub (`__init__.pyi:267`) both say `NotUniqueError`. |
| N5 | fixed | Python `types="note"` gives "types must be a list of strings ...". Node gives "searchSubstring options.types must be an array, not string", and also "hybridSearch: query.relations.seeds must be an array" (same for `types`, `textTypes`, `items`). |
| N6 | **still open (not confirmable from outside)** | The Node README now promises the code `INTERNAL` for failures that are neither engine errors nor argument errors. My only outside probe, `read.call({}, id)`, gave `INVALID_ARGUMENT`, which does not reach that path. Needs validation by someone who can see the code. |
| N7 | fixed | The `updateDocument` note is now in the NOT_FOUND row. |
| N8 | fixed | The `find_range` rustdoc (lib.rs:965) now names `limit == 0` and low above high. |
| N9 | fixed (documented) | The coverage doc's "Numeric ranges" section states the difference. Node `limit: 2**32` gives "at most 4294967295". The alpha.3 strings are tracked under F10. |

What is left of F6:
- **keep_last messages:** `keep_last`/`keepLast` -1 says "a whole number, 0 or more", but 0 is refused, in both bindings. See NEW-3.
- **Python README Limits (line 343):** says "a counter, limit or revision argument is a whole number from 0 to 2**64 - 1". But `limit=0` gives "limit must be a whole number from 1 to 2**64 - 1".
- **Coverage doc line 150:** says the same "0 to 2**64 - 1".

## New findings

**NEW-1 (doc). The "Python equivalent" column of the Node README errors table is stale.**
- Where: bindings/node/README.md:270 and :272.
- `HISTORY_UNAVAILABLE` lists `KernoError` and `CLOSED` lists `ValueError`. Python now raises `HistoryUnavailableError` and `ClosedError` (seen in the Python run).
- The fix round added the two classes but did not update this table.

**NEW-2 (doc + message). The wrong lifecycle is not `DeletedError`/`DELETED` as the tables say.**
- Where: bindings/python/README.md:264 and bindings/node/README.md:255. Both say DELETED covers "not in the lifecycle the call needs".
- Python `commit([("unarchive", active_id)])` gives `InvalidArgumentError: invalid argument: unit … is Active, not Archived`.
- Node `{op:'unarchive'}` gives `INVALID_ARGUMENT` with the same text.
- The message also prints the Rust variant names `Active`/`Archived`.

**NEW-3 (bug, message). Retention messages contradict themselves and leak a Rust name.**
- Python `run_retention(by_type={"note":{"keep_last":-1}})` gives "keep_last must be a whole number, 0 or more, not -1".
- But `keep_last: 0` gives "invalid argument: KeepLast(0) would drop the head".
- Node is identical: "keepLast must be a whole number, 0 or more", then "KeepLast(0) would drop the head".
- This is the F6/F7 class, in a new place.

**NEW-4 (message nit). The Node PRUNED message names a call Node does not have.**
- `readAsOf` of a pruned revision says "`history` and `compare` take revision numbers".
- Node has no `compare` (the coverage doc lists it as a Gap).

**NEW-5 (doc nit). The `rrf_k`/`rrfK` range is in neither README.**
- 0 is accepted and gives scores 1.0 and 0.5.
- Python's message gives "0 to 2**32 - 1", but neither README states it.
- Where: bindings/python/README.md:241 and bindings/node/README.md:225.

**NEW-6 (doc nit). The coverage doc has no row for `substring(type)`.**
- It has rows for `set_substring` and `search_substring`, and one for `fulltext(type)`.
- Both READMEs document `db.substring(type)`.

## README claims that held

Everything else I checked held in both bindings:
- open, close twice, `with`, read-only beside a writer with `refresh`, LOCKED, DATABASE_NOT_FOUND (the directory is not created)
- every batch op (store, update, update_if_changed/ifChanged, archive, restore, upsert, relate/relate_unique by position, delete, delete_cascade, unrelate), BAD_ENDPOINT, one op per unit, reason/actor limits
- schema define/redefine, SCHEMA_EXISTS after `create_index`, the 254-byte index cap, SCHEMA type refusal
- find, find_range (Python), execute, explain (Python)
- upsert created/unchanged/updated, NOT_UNIQUE, SCHEMA
- search_text and search_substring with every documented refusal
- hybrid search: three legs, `graph_gated`, `score` equal to the sum of the leg scores, and the documented refusals
- retention: PRUNED through `read_as_of`, `history(from_revision)` and `compare` (Python)
- the 1 MiB boundary, the lone surrogate (Python: `InvalidArgumentError`; Node: U+FFFD), bigint round trip
- unknown-option refusal in every Node options object and nested hybrid object, and for an unknown Python kwarg
- the quickstarts' output matched. The Python quickstart's last line printed `[note]`, the note's id, as expected.

## Where I had to guess

**Nowhere in the integration.** The READMEs were enough to build the notes/bookmarks store in both languages. The only source I read is the exceptions named at the top.

## Not checked

- The published alpha.4 packages (F11).
- Two-process LockedError and `refresh` (I used same-process handles only).
- CorruptionError from a damaged file, kill -9, threads and the GIL.
- macOS, Windows and aarch64.
- The alpha.1 upgrade path.
- Type checkers against `py.typed` and `index.d.ts`.

No security finding: nothing here crosses a trust boundary. No SAST/SCA scanner was run: this item is an integration and doc gate, not a code audit.
