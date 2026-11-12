# Friction found while using the public API from outside

Everything below was met while writing this example with only what an outside
user can reach: the re-exports of `kerno-units`, its public modules, and the
`kerno-core` / `kerno-storage` types needed to open a database. Nothing was
fixed in the library. Line numbers refer to `src/lib.rs` of this example unless
another file is named. Items are ordered by how much they cost.

## A. Things an agent memory needs that the public API does not give

1. **`DerivedDb` cannot list, query or read history.** It exposes `get_unit`
   by id and `begin_changeset`, and nothing else for reading the store: no
   `snapshot()`, no `query(&UnitFilter)`, no `history`, no `compare`. Those
   exist on `UnitSnapshot`, which is reachable only from the `UnitDb` that
   `DerivedDb::open_persisted(db)` consumes. So "recall by time" cannot be
   asked of the database, and the previous revision of a corrected memory
   cannot be shown. Workaround: a *catalog* unit (`src/catalog.rs`) that
   lists every memory, rewritten in the same change set as each create and
   delete (`remember`, `forget`), plus a `get_unit` per id on every listing
   (`AgentMemory::all`):
   ```rust
   let db = UnitDb::<KernoDb>::open(StdVfs::shared(), dir, &cfg)?;      // line 221
   let (db, reopen) = DerivedDb::open_persisted(db)?;                   // db is gone from here on
   ```
   The catalog is a second source of truth next to the store and holds at most
   about 13 000 memories (one unit payload, 1 MiB).

2. **No way to find the derived data of a unit.** `DerivedRegistry` has
   `dependents_of_unit`, but `DerivedDb` offers only `state(&id)`,
   `descriptor(&id)`, `vector(&id)` and `value(&id)`, all by `DerivedId`. After
   a reopen the registry knows every vector, yet the caller cannot ask "which
   one is memory 7's". The catalog therefore also stores the hex `DerivedId` of
   each vector (`mem <n> <hex>`).

3. **A derived id changes at every rebuild, so the caller's record of it goes
   out of date, and the two cannot be updated atomically.** `rebuild_vector`
   returns a new `DerivedId` (the source revision is part of the identity) and
   commits by itself:
   ```rust
   let new = self.db.rebuild_vector(&old, meta("rebuild a stale vector"), &|_, inputs| { .. })?; // line 405
   cat.vectors.insert(id, Some(new));
   self.save_catalog(&cat, "record the rebuilt vector")                                          // line 409
   ```
   A crash between the two lines leaves a catalog that names a vector which no
   longer exists, and nothing in the API finds the new one (see 2). The only
   atomic route is `DerivedChangeSet::supersede_vector`, which also leaves the
   old descriptor and its `0x62` entry behind as `Superseded` clutter.
   `replace_vector` (the in-change-set form of rebuild) is `pub(crate)`.

4. **Physical erasure is unreachable, so "forget" is only logical.** `forget`
   deletes the unit, its vector and its summaries in one commit, and
   `prune_history` (`DerivedDb::run_retention` with `KeepLast(1)`) prunes the
   old revisions. The text is still in the files after both and after `close`:
   `forgotten_text_stays_in_the_files_after_retention` finds it in
   `000003.sst`. `UnitDb::compact` needs `E: TestHooks` and `KernoDb::compact_range`
   is an inherent method, but both sit behind `DerivedDb`, which hides the
   engine. `suppression.rs` says as much ("No promise of physical erasure")
   but offers no public way to get it; an agent that must honour a deletion
   request has none.

5. **Vector + summary cannot be opened together.** Three constructors exist and
   each knows one world:
   - `DerivedDb::open_persisted(db)` loads persisted vectors, no summaries;
   - `open_summaries(db, known, &summarizer)` / `DerivedDb::open(db, known, compute)`
     rebuild in-memory data from `known` descriptors, and do not load the
     persisted vectors.

   Workaround: open with `open_persisted`, then call `build_summary` again from
   a list of source ids kept in the catalog (line 256). Consequence: a summary
   that was `Stale` when the process stopped comes back `Ready` and recomputed
   on the current revisions, which hides that its sources moved; and summaries
   are memory-only (documented in `consolidation.rs`), so every app has to
   persist its own definition of them (`sum <ids>` in the catalog) in a commit
   that is not the one that built them (line 516).

## B. Awkward or confusing

6. **Unit ids are opaque 16 bytes with no friendly handle, and the id source
   must be chosen before the database is open.** The CLI needs short numbers.
   `CounterIds::new(start)` cannot be moved after the fact, and the next free
   number lives in the database (catalog), which cannot be read before
   `UnitDb::open(.., &cfg)` receives the source. So the example has its own
   `UnitIdSource` (`Ids`, line 154) whose counter is set after reading the
   catalog (line 244). The default `UuidV7Source` would avoid it, at the cost
   of ids no one can type.

7. **`create` returns a `UnitId`, but `put_vector` wants a `RevisionId`.** To
   persist the vector in the same change set as the create, the first revision
   number is written as a literal:
   ```rust
   let unit = cs.create(NewUnit { .. })?;
   cs.put_vector(&self.vector_spec(RevisionId { unit, rev: RevNo(1) }, v))?     // line 313
   ```
   It works because a new unit starts at revision 1, but the API does not say
   so; `create` could return the `RevisionId`.

8. **Replacing a stale vector with one the caller computed goes through the
   "recompute" API.** `rebuild_vector` takes a `compute` closure over the
   sources; to supply a vector made elsewhere the closure just ignores its
   arguments (line 360):
   ```rust
   self.db.rebuild_vector(&old, meta("replace a stale vector"), &|_, _| Some(vector.clone()))?
   ```
   `put_vector` at the new head would succeed but leave the old `Stale` datum
   in the registry and the store for good (nothing public deletes one), and
   `replace_vector` is not public, so this is the only clean route. The
   same closure also receives a `&DerivedDescriptor` the caller already knows.

9. **Two vector worlds that do not meet.** Persisted vectors live in
   `DerivedDb` (`vector(&id)`, one at a time). Nearest-neighbour search lives
   in `kerno_units::assoc_rank::{VectorSet, VectorQuery}`, in memory, not
   re-exported at the crate root and described as "never written anywhere".
   There is no search over the persisted vectors: `recall_similar` copies every
   `Ready` vector into a fresh `VectorSet` on each call (lines 449-455). Also
   `VectorSet::insert` fails for a vector of another dimension or a zero vector
   under cosine, and the example ignores that failure (`let _ =`) because a
   single odd vector must not block recall; an API that returned the skipped
   ones would be better.

10. **Recipe ceremony.** A `VectorSpec` needs `method`, `method_version`,
    `profile` (1..=64 bytes, "opaque"), `params` (at most 16, key <= 64,
    value <= 256) and the source; `VectorRecipe` repeats four of them. Nothing
    ties `params` (`dim`, `seed`) to the vector actually produced, so a caller
    that attaches a vector of another dimension under the same recipe is not
    stopped. The identity then has to be rediscovered by hashing, which is why
    the catalog keeps ids instead of recomputing them (an outside user cannot
    easily: `input_hash` needs a `SourceInput`, whose payload is irrelevant to
    the hash).

11. **`DerivedDb::close` returns `()`**, throwing away what `KernoDb::close`
    knows (`CloseReport`). The example cannot tell whether the close compacted,
    flushed or found trouble.

12. **Three error types, no common one.** `UnitError`, `DerivedStaleError` and
    `ConsolidationError` all come out of the same object, so the example
    wraps all three (`Error::{Unit, Derived, Consolidation}`) plus
    `VectorError` from `assoc_rank`. `Display` of `DerivedStaleError` and
    `ConsolidationError::NotASummary` names no unit or id ("a source unit does
    not exist or is deleted"), which is correct for I-37 but unhelpful for a
    caller with several sources.

13. **Opening needs three crates and a trait object.** `kerno-units` alone
    cannot open a database: the engine type (`KernoDb`, from `kerno-storage`)
    and the file system (`StdVfs::shared()`, from `kerno-core`) come from the
    others, and the engine is a type parameter that every signature then
    carries (`DerivedDb<KernoDb>`). A `UnitDb::open_dir(path)` for the
    default engine would remove two dependencies from the example's manifest.

14. **A second open of the same directory fails with a storage error** ("database
    is locked by another handle") wrapped as `UnitError::Storage`. That is the
    right behaviour, but the variant gives the caller no way to tell "locked"
    from "corrupt" except by matching the text.

15. **The unit type namespace is shared with the app's bookkeeping.** The
    catalog is a unit of type `"catalog"` next to the `"memory"` units; with no
    snapshot/query on `DerivedDb` (item 1) nothing separates them. A future
    `UnitFilter` query would return the catalog too, and the history policy
    (`Versioned` by default) keeps every revision of it, so the catalog grows
    with every write until `prune_history` is run.

## What worked without friction

- The guarded change set did what it promises: a `correct` marks the vector
  `Stale` in the same commit (`full_scenario_with_close_and_reopen` in
  `tests/scenario.rs`), and a `forget` removes the vector and any summary over
  the unit without the example doing anything.
- `open_persisted` served exactly the vectors that were `Ready` and verified
  (`ReopenReport::served`) after a close; a vector that was `Stale` at close
  came back `Stale`.
- `put_vector` in the same change set as `create` and the catalog update made
  "store a memory with its vector" a single atomic commit.
