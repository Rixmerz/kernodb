# kernodb 1.0.0-alpha.4: whole-API review

- **Commit:** 4322a40 (main)
- **Date:** 2026-11-25
- **Gate:** RELEASING.md pre-release item 4: whole-API review by a locks worker (did not write the code)
- **Scope read:** `crates/kerno/src/lib.rs` (pub API and rustdoc); `bindings/python` (`src/lib.rs`, `python/kerno/__init__.py`, `__init__.pyi`, `README.md`); `bindings/node` (`src/lib.rs`, `index.js`, `index.d.ts`, `README.md`); `docs/bindings-coverage.md`.
- **Artifacts probed:** `bindings/python/python/kerno/_kerno.abi3.so` (built 13:39) and `bindings/node/kernodb.linux-x64-gnu.node` (built 13:40), both after the 4322a40 commit time (13:26) on a clean tree (only `bindings/python/uv.lock` untracked). Not rebuilt.
- **Result:** 1 blocker, 10 should-fix, 9 nits.

## Blocker

### B1. Python `query()` silently returns `[]` for an invalid type name when `lifecycle` is not `"active"`
- **Where:** `bindings/python/src/lib.rs:677-700`. The `"active"` path calls `db.query(&filter)`, which validates `unit_type`. The other paths only call `filter.matches(...)` (line 695), which validates nothing.
- **Contract broken:** `__init__.pyi:180` says "An invalid `unit_type` name (empty or over 64 bytes) raises `InvalidArgumentError`", with no lifecycle exception.
- **Repro:**
  - `db.query(unit_type="")` gives `InvalidArgumentError: invalid argument: unit_type is not a valid unit type name`
  - `db.query(unit_type="", lifecycle="archived")` gives `OK []`
  - `db.query(unit_type="t"*65, lifecycle="all")` gives `OK []`
- **Fix:** validate `unit_type` with `kerno::units::codec::valid_unit_type` before branching, and raise `InvalidArgumentError`.

## Should-fix

### S1. Node `history` silently ignores `toRevision` and `order`
- **Where:** `bindings/node/src/lib.rs:342-351` (`HistoryOptions` has only `limit`, `withPayload` and `fromRevision`); `index.d.ts:274-281`.
- **The problem:**
  - Python has `to_revision` and `order` (`python/src/lib.rs:1258`).
  - The Node README says the API "follows the Python binding".
  - `docs/bindings-coverage.md:109` does not list the gap.
  - napi object structs drop unknown keys, so plain JS gets no error. A paging loop ported from the Python README (`to_revision=page[-1].revision`) returns the same page every time.
- **Repro:** `db.history(a, {order: "asc", toRevision: 1})` gives `OK [1]`, with no error and the options not applied.
- **Fix:** add `toRevision` and `order`, or throw `INVALID_ARGUMENT` on unknown keys. List the gap in the coverage doc either way.

### S2. Rust doc: wrong relation-type limit
- **Where:** `crates/kerno/src/lib.rs:981` says "1 to 256 bytes, no NUL". The real limit is 64 (`crates/kerno-units/src/codec.rs:96`, `MAX_RELATION_TYPE_LEN: u32 = 64`). Both bindings document 64 correctly.
- **Repro:** `db.relate(a, b, "r"*65)` gives `InvalidArgumentError: relation_type of 65 bytes is outside 1..=64 or has a NUL`.

### S3. Rust doc: an oversized batch is `ChangeSetTooLarge`, not `InvalidArgument`
- **Where:** `crates/kerno/src/lib.rs:630-631` ("`InvalidArgument` when ... the batch is larger than the layer's change-set limit").
- **Repro:** `kerno.open(p, max_batch_ops=2)`, then `db.commit([3 stores])` gives `ChangeSetTooLargeError ... change set too large: 3 operations, 626 bytes`. That class is not an `InvalidArgumentError`.
- **Impact:** a Rust caller matching `Error::InvalidArgument` misses it.

### S4. Rust doc: upsert by a non-unique field (or a type with no schema) is `Error::NotUnique`, not `InvalidArgument`
- **Where:** `crates/kerno/src/lib.rs:466` (`Batch::upsert`) and `:574` (`Db::upsert_document`).
- **Repro:**
  - `db.upsert("user","age",{...})` gives `NotUniqueError: no unique key: field "age" is not declared unique`
  - `db.upsert("nosuch","f",{"f":1})` gives `NotUniqueError: ... has no schema`
- **Why it matters in Rust:** `NotUnique` is its own `UnitError` variant (`kerno-units/src/error.rs:260`). It only reads as "invalid argument" in Python, where `NotUniqueError` subclasses `InvalidArgumentError`.

### S5. Rust doc: `Db::read_as_of` lists retention under `HistoryUnavailable`
- **Where:** `crates/kerno/src/lib.rs:845-846`.
- **Actual:** a pruned revision comes back as `Ok(UnitAt::Pruned { oldest })`, not as an `Err`, and `lib.rs:877` says so. `HistoryUnavailable` comes only from a deleted unit after `compact`.
- **Repro (Python, which turns `UnitAt::Pruned` into an error):** `run_retention(keep_last=1)`, then `read_as_of(a, 1)` gives `PrunedError`.

### S6. No dedicated error for `HistoryUnavailable` in either binding; the message misleads
- **Where:** it falls through to the default arm: `python/src/lib.rs:136` (`KernoError`) and `node/src/lib.rs:138` (`KERNO`).
- **Repro:** store, update, delete, then `compact()`. After that:
  - `read_as_of(b, s)`, `history(b, with_payload=True)` and `compare(b, 1, 2)` all give `KernoError: history unavailable under policy Audit`
  - Node `readAsOf` gives `code=KERNO`
- **The problem:**
  - A documented outcome of `compact()` gets the catch-all class, and the message names a policy the user never set.
  - `history(with_payload=True)` of such a unit fails as a whole, rather than listing the revisions with `payload=None`.
  - Neither README names the error.

### S7. Python `Revision.created_at` is documented as milliseconds but is microseconds
- **Where:** `bindings/python/src/lib.rs:359` ("Descriptive wall-clock time (milliseconds)"). `Timestamp` is microseconds (`kerno-units/src/types.rs:70`). Node names it `createdAtUs`.
- **Repro:** `(r.created_at, time.time()*1e3, time.time()*1e6)` gives `(1795625588014647, 1795625588014, 1795625588014734)`.
- **Fix:** correct the doc. Consider `created_at_us`, to match Node.

### S8. `Unit` has different fields in each binding
- **Where:** `python/src/lib.rs:421-437` and `node/src/lib.rs:193-209`.
- **Python has `oldest_revision`; Node does not.**
  - The retention advice in both READMEs ("compare it with `oldest_revision`, or catch `PRUNED`") cannot be followed in Node.
  - Repro: Node `Object.keys(db.read(a))` gives `["id","unitType","schemaVersion","lifecycle","revision","createdAtUs","updatedAtUs","payload"]`.
- **Node has `createdAtUs` / `updatedAtUs`; Python has no unit timestamps.**
- Neither gap is in `docs/bindings-coverage.md`.

### S9. Python argument refusals split between `ValueError` and `InvalidArgumentError`
- **Where:** `python/src/lib.rs`:
  - 286 (id)
  - 560 (closed handle)
  - 620 (direction)
  - 672 (lifecycle)
  - 271 (field type)
  - 885-887 (field without name/type)
  - 1086 (metric)
  - 1098 (vector without vectors)
  - 1177 and 1231 (batch op)
  - 1274 (order)
  - 235-259 (retention rule)
- **The problem:**
  - `ValueError` is not a `KernoError`, so `except kerno.KernoError` misses a malformed id or a closed handle.
  - Node throws `INVALID_ARGUMENT` / `CLOSED` for all of them.
  - The behaviour is documented (Python README:281-285), but `__init__.pyi` never mentions `ValueError`.
  - Constraint #1578 accepts `ValueError` only for numeric options.
- **Repro:**
  - `db.read("zz")` gives `ValueError [KernoError=False]`
  - `db.related(a,"sideways")` gives `ValueError [KernoError=False]`
  - After close, `db.read(a)` gives `ValueError [KernoError=False] operation on a closed database`
  - Node `db.read("zz")` gives `code=INVALID_ARGUMENT`; after close, Node `read` gives `code=CLOSED`
- **Fix options:** make `InvalidArgumentError` also subclass `ValueError` (keeps both contracts), and add a `ClosedError`.

### S10. Hybrid search: same names, different meanings; the coverage doc says "Full"
- **Where:** `docs/bindings-coverage.md:80` marks `hybrid_search` "Full" for both bindings.
- **Python lacks** `fusion`, relation `direction`, relation-type filter and `visit_budget`. Its budget is fixed at `MAX_ASSOC_VISITS` (`python/src/lib.rs:1103`).
- **`score` has a different scale in each binding:**
  - Python: a float `score / 1e9` (`python/src/lib.rs:1128`), plus per-leg `*_score`
  - Node: an integer scaled by 1e9 (`node/src/lib.rs:825`, `1543`), with no per-leg scores
  - Repro: Node `hybridSearch({text:"v1"})` gives `"score":16393442`; Python gives `0.0163...` for the same fusion.
- **Rank names differ:** Python `relation_rank`, Node `ranks.relations`.
- **Fix:** mark the row partial and list the differences; align `score` units.

## Nits

- **N1. `readDocument` on a payload that is not a document gives the generic invalid-argument error, not the schema one.** Both READMEs (Node:266, Python:279) and `python/src/lib.rs:98` say SCHEMA / `SchemaError` covers "the payload is not a document". Repro: Python gives `InvalidArgumentError: record: not a record (unknown version byte)`; Node gives `code=INVALID_ARGUMENT`.
- **N2. Python accepts the field type `"str"` (`python/src/lib.rs:269`); Node refuses it** (`node/src/lib.rs:908`). Repro: Node `defineSchema("u",[{name:"a",type:"str"}])` gives `INVALID_ARGUMENT unknown field type "str"`. Neither README lists `str`.
- **N3. The `hybrid_search` row of the Python README API table (line 241) omits `text_types`.** The stub and docstring have it.
- **N4. Python `upsert` docstring vs stub.** The docstring (`python/src/lib.rs:919-920`) says `InvalidArgumentError` for a non-unique field; the stub (`__init__.pyi:241`) says `NotUniqueError`. Both are true (subclass); name the specific class in both.
- **N5. Raw pyo3/napi messages leak** in otherwise correctly classed errors:
  - Python `search_text("x", types="note")` gives "Can't extract `str` to `Vec`"
  - Node `searchSubstring("x",{types:"note"})` gives "Failed to get Array length on SearchSubstringOptions.types"
  - Node `searchText` has its own message for the same case; `SearchSubstringOptions.types` and `RelationsInput.types/seeds` are plain `Vec<String>`, not the `Names` wrapper (`node/src/lib.rs:803`, `733`)
- **N6. `node/index.js:21-24` turns every error without a Kerno code into `INVALID_ARGUMENT`.** That includes napi `GenericFailure` or any non-argument failure, so an internal fault would read as a caller error. Inferred from the code, not reproduced.
- **N7. Node README:262 puts the `updateDocument` NOT_FOUND note inside the `READ_ONLY` row of the error table.**
- **N8. Rust `Db::find_range` docs (`crates/kerno/src/lib.rs:958-960`, "As `Db::find`") do not say that `limit == 0` and `lo > hi` are `InvalidArgument`.** Python documents and enforces both. Repro: `find_range(..., limit=0)` and `find_range("user","age",50,10)` both give `InvalidArgumentError`.
- **N9. Numeric ranges differ by binding.** Node caps `limit`, `candidates` and `schemaVersion` at 2^32-1 and sequences at 2^53; Python accepts up to 2^64-1 (for example `read_as_of(a, 2**64-1)` gives OK). Both document their own range; this is a cross-language difference only. Also, the READMEs and manifests still say alpha.3 (`bindings/node/package.json:3`, root `Cargo.toml:26`, both README banners). That is presumably the release-bump step, not this review.

## Checked and consistent across Rust, Python and Node

- **Payload limit 1 MiB.** 1,048,576 bytes is accepted; 1,048,577 gives `PayloadTooLargeError` / `PAYLOAD_TOO_LARGE`.
- **Unit type and relation type 1-64 bytes in both bindings.** Empty or 65 bytes gives invalid argument.
- **Commit metadata.** `reason` over 1024 bytes and `actor` over 256 bytes are refused.
- **`max_batch_ops`.** 0 and 2^21 are refused (`outside 1..=1048576`).
- **Empty path.** `open("")` is refused in both bindings.
- **Wrong-typed arguments (constraint #1698).** `store(None,"x")`, `store("note",None)`, `read(None)`, Node `store(undefined,"x")`, `store("note",null)` and `read(undefined)` all give the binding's own invalid-argument error.
- **Numeric options (constraint #1578).** -1, 2**64, 1.5 and 2**32 (Node) give invalid argument and are never truncated.
- **Retention.** `keep_last 0` is refused in both bindings.
- **Pruned revisions.** PRUNED is raised by `read_as_of`, `history(from_revision)` and `compare` (Python); `oldest_revision` moves up to 4.
- **No-match searches raise instead of returning `[]`.**
  - `search_text`: `NotIndexed`, `UnknownType`, empty `types`, no word, `limit 0`
  - `search_substring`: empty needle, over 1024 bytes, `limit 0`
  - `find`: no schema or unindexed field gives `NotIndexed`; a value over 254 bytes is invalid argument
  - `find_range`: `limit 0`, `low > high`
- **Hybrid bounds.** `limit` 1001, `candidates` 0, weight 1001, `max_depth` 9, `visitBudget` 200000, NaN or an f32-overflowing vector, and `text_types` without text are all refused.
- **Batches.** The same unit twice gives invalid argument; a bad endpoint gives `BadEndpointError` / `BAD_ENDPOINT`.
- **Deleted units.** relate, update and delete on one give `DeletedError`.
- **Unknown ids.** update, delete and unrelate give `NotFoundError`.
- **Read-only handles.** A write gives `ReadOnlyError`; `commit([])` is OK; opening a missing database gives `DatabaseNotFoundError`; `refresh` on a read-write handle gives invalid argument.
- **A second read-write open** gives `LockedError`.
