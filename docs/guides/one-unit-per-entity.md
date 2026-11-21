# Recipe: one unit per entity, keyed by a logical key

Use this when each real-world entity (a person, a product, a document) should be exactly one unit,
found by a key that means something outside Kerno, such as an email address or a SKU. Kerno's unit
ids are random, so the key must be a field of the unit.

## The three parts

1. **A schema with a unique field.** `unique` also makes the field indexed, so `find` works on it
   without a separate `create_index`. The unique check refuses a second active unit with the same
   value, so the key cannot be duplicated.
2. **`find` by the key.** `db.find(type, field, value)` returns the active units whose field equals
   the value. A deleted unit does not match, and its key is free again.
3. **An upsert in two steps.** Kerno has no upsert operation in the bindings. Find the unit, then
   update it or store a new one, under one lock in the process.

## Why a lock, and not a batch

A raw `commit` op cannot write a unit of a typed type: the schema refuses the payload
(`SchemaError: not a record`). The bindings have no way to put an encoded document into a batch, so
the upsert is one `find` followed by one `update_document` or `store_document`. The unique check is
the safety net: a second store of the same key is refused. Because one process holds a writable
database at a time, the only race is between threads that share the handle, and a `threading.Lock`
around find-and-write closes it.

## Example

The full script (it creates its own scratch directory, so it runs as is):

```python
import os, tempfile, threading
import kerno

db = kerno.open(os.path.join(tempfile.mkdtemp(), "db"))
# unique implies indexed: the field is both checked and findable by value
db.define_schema(
    "person",
    [
        {"name": "email", "type": "text", "required": True, "unique": True},
        {"name": "name", "type": "text"},
    ],
)
lock = threading.Lock()  # find and write are two steps; one thread at a time


def upsert_person(email, name):
    """Insert the person with this email, or update the one that has it."""
    doc = {"email": email, "name": name}
    with lock:
        hits = db.find("person", "email", email)
        if not hits:
            return db.store_document("person", doc)
        if db.read_document(hits[0].id) != doc:  # an identical document writes no revision
            db.update_document(hits[0].id, doc)
        return hits[0].id


ada = upsert_person("ada@example.com", "Ada")
print("insert:", ada == upsert_person("ada@example.com", "Ada"))
upsert_person("ada@example.com", "Ada Lovelace")
print("found:", [db.read_document(u.id)["name"] for u in db.find("person", "email", "ada@example.com")])
print("revisions:", [r.kind for r in db.history(ada)])

try:
    db.store_document("person", {"email": "ada@example.com", "name": "Someone else"})
except kerno.SchemaError as e:
    print("duplicate key refused:", e)

try:
    db.commit([("store", "person", "raw payload")])
except kerno.SchemaError as e:
    print("batch refused:", e)

db.delete(ada)
print("after delete, find:", db.find("person", "email", "ada@example.com"))
db.close()
```

Its output, as run against this tree:

```text
insert: True
found: ['Ada Lovelace']
revisions: ['update', 'create']
duplicate key refused: refused by the schema: unit 01a20048dae97005a20a3081484eb557: another unit already holds the unique value of "email"
batch refused: refused by the schema: unit 01a20048dae97006a22879b135458b5b: not a record (unknown version byte)
after delete, find: []
```

The unit ids in the two refusal lines differ on every run; the messages are otherwise the same.

## Upsert operation

There is no upsert operation on `main` (no `upsert` in the crates or the bindings). When one is added,
replace the two steps above with it and keep the same schema.

## Notes

- `find` returns active units only. It does not tell you whether the key was ever used by a deleted
  unit; read the history for that.
- Two processes cannot both hold a writable database, so a multi-process writer needs one writer
  process and the others reading with `read_only=True`.
