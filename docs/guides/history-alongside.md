# Kerno as a revisioned history next to another database

Keep your current database as the source of truth, and mirror every change into Kerno. Kerno then
holds every revision of every unit, so you can read any unit as it was at a commit, list its history,
and prune old revisions. Nothing in your application reads Kerno to decide what is true now.

The example below is runnable Python: it uses `sqlite3` from the standard library as the main database.

## The model in five rules

1. **The main database is the source of truth.** Write it first, in its own transaction. Kerno is
   the record of what it held, not a second copy you query for the current state.
2. **Mirror each main transaction as one Kerno commit.** `db.commit(ops, reason=..., detailed=True)`
   applies all the ops atomically and returns `commit_seq`. Keep that sequence with the change, so you
   can ask later for the state at that point.
3. **Keep the Kerno unit id in the main database.** Each row records the id of its unit, so a later
   update or delete knows which unit to change. (If you need to find a unit from a business key
   instead, see [one unit per entity](one-unit-per-entity.md).)
4. **A delete leaves a tombstone.** `delete` (or `delete_cascade`) does not erase the unit: it becomes
   `lifecycle == "deleted"` with no payload, and its earlier revisions stay readable.
5. **Relations are edges between units, not revisioned.** You can add, remove and list them, but
   `read_as_of` covers units only. Relations have no history of their own.

## Mirroring the writes

| Change in the main database | Kerno ops in one `commit` |
|---|---|
| insert a row | `("store", type, payload)` |
| update a row | `("update", id, payload)` |
| delete a row | `("delete", id)`, or `("delete_cascade", id)` to remove its relations too |
| a foreign key or edge | `("relate", from, to, relation_type)`; an end is a unit id, or the position in the ops list of an earlier `store` or `upsert` op of the same commit (a later op, an update or a relation there raises `BadEndpointError`) |
| remove an edge | `("unrelate", relation_id)` |

A unit with live relations refuses a plain `delete` with `HasRelationsError`. Either remove the
relations first with `unrelate`, or use `delete_cascade`, which removes them in the same commit.

Payloads are bytes or text. A unit of a type that has a schema cannot take raw payloads: write it with
`store_document` and `update_document` instead, one commit each. Those calls take no `reason`, so a
typed mirror has no provenance on its revisions.

## As-of reads and history

- `db.last_commit_seq()` is the sequence of the last commit.
- `db.read_as_of(id, seq)` returns the revision in force just after commit `seq`, or `None` before the
  unit existed. A sequence past the last commit reads the current state.
- `db.history(id)` lists the revisions of one unit, newest first. Each has its `revision`, `kind`
  (`create`, `update`, `delete`, ...) and `commit_seq`.

## Retention

Kerno keeps every revision until you prune. `db.run_retention(default=..., by_type=...)` drops old
revisions by rule, per unit type. A rule is one of:

- `{"keep_last": n}`: the newest `n` revisions
- `{"keep_for": seconds}`: revisions younger than that many seconds
- `{"keep_since": commit_seq}`: what answers as-of reads from that commit on

Two things to know:

- The policy is not stored. Pass it each time you prune.
- A pruned revision is gone: `read_as_of`, `history(from_revision=...)` and `compare` raise
  `PrunedError` for it. `verify()` stays clean.

## Readers in other processes, and `refresh`

One process at a time can hold a database open for writing; a second read-write open raises
`LockedError`. A read-only open takes no lock, so any number of readers can run beside the writer, in
other processes or in the same one. A reader sees the commits made up to when it opened. Call
`reader.refresh()` to move it to the commits made since; it returns the sequence it now reads. A
read-write handle is always current, and `refresh()` on one raises.

## Runnable example

The full script (it creates its own scratch directories, so it runs as is):

```python
import json, os, sqlite3, tempfile
import kerno

work = tempfile.mkdtemp()
main = sqlite3.connect(os.path.join(work, "app.db"))  # the source of truth
main.execute("create table customer (id integer primary key, name text)")
history = kerno.open(os.path.join(work, "history"))  # keeps every revision


def mirror(ops, reason):
    """Write one main-database transaction's changes to Kerno as one commit."""
    return history.commit(ops, reason=reason, detailed=True)


# create: the main database first, then the mirror
with main:
    main.execute("insert into customer (id, name) values (1, 'Ada')")
created = mirror([("store", "customer", json.dumps({"name": "Ada"}))], "create customer 1")
uid = created["units"][0]

# rename
with main:
    main.execute("update customer set name = 'Ada Lovelace' where id = 1")
renamed = mirror([("update", uid, json.dumps({"name": "Ada Lovelace"}))], "rename customer 1")

# an order related to the customer, in one commit
with main:
    main.execute("create table orders (id integer primary key, customer_id integer)")
    main.execute("insert into orders values (7, 1)")
placed = mirror(
    [("store", "order", "order 7"), ("relate", 0, uid, "placed_by")], "place order 7"
)

# delete: the tombstone stays in Kerno; the cascade removes the relation in the same commit
with main:
    main.execute("delete from customer where id = 1")
deleted = mirror([("delete_cascade", uid)], "delete customer 1")

print("as of create:", history.read_as_of(uid, created["commit_seq"]).payload)
print("as of rename:", history.read_as_of(uid, renamed["commit_seq"]).payload)
print("now:", history.read(uid).lifecycle, history.read(uid).payload)
print("revisions, newest first:", [(r.revision, r.kind) for r in history.history(uid)])
print("commit seqs:", created["commit_seq"], renamed["commit_seq"], placed["commit_seq"], deleted["commit_seq"])
print("relations left:", history.relations("placed_by"))

# a reader process: no lock, so the writer stays open
reader = kerno.open(os.path.join(work, "history"), read_only=True)
history.store("note", "written after the reader opened")
print("reader before refresh:", reader.last_commit_seq())
print("reader after refresh:", reader.refresh())

# retention: keep the newest two revisions of customers
report = history.run_retention(by_type={"customer": {"keep_last": 2}})
print("pruned revisions:", report["revisions_pruned"])
try:
    history.read_as_of(uid, created["commit_seq"])
except kerno.PrunedError as e:
    print("PrunedError:", e)

reader.close()
history.close()
```

Its output, as run against this tree:

```text
as of create: b'{"name": "Ada"}'
as of rename: b'{"name": "Ada Lovelace"}'
now: deleted None
revisions, newest first: [(3, 'delete'), (2, 'update'), (1, 'create')]
commit seqs: 1 2 3 4
relations left: []
reader before refresh: 4
reader after refresh: 5
pruned revisions: 1
PrunedError: revision pruned by retention; oldest kept is 2
```

Read the output this way: the first two lines are the as-of reads of commits 1 and 2; the unit is a
tombstone at the end; the cascade removed the `placed_by` relation; the reader saw commit 4 until it
refreshed to commit 5; the retention pass pruned the create revision.

## What Kerno cannot do here

- **Atomicity across the two databases.** Kerno cannot join the main database's transaction. If the
  process stops between the main commit and the mirror commit, the main database has the change and
  Kerno does not. Recover by re-mirroring from the main database's own log, if you keep one.
- **Edge history.** `read_as_of` answers for units. An edge shows only as live or removed, not as it
  was at a sequence.
- **Provenance on typed units.** `store_document` and `update_document` take no `reason`.
