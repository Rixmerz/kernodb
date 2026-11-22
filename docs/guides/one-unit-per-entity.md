# Recipe: one unit per entity, keyed by a logical key

Use this when each real-world entity (a person, a product, a document) should be exactly one unit,
found by a key that means something outside Kerno, such as an email address or a SKU. Kerno's unit
ids are random, so the key must be a field of the unit.

## The three parts

1. **A schema with a unique field.** `unique` also makes the field indexed, so `find` works on it
   without a separate `create_index`. The unique check refuses a second active unit with the same
   value, so the key cannot be duplicated.
2. **`upsert` by the key.** `db.upsert(type, field, document)` stores the document as a new unit, or
   revises the active unit that holds the same value of `field`. It is one commit, so there is no
   find-then-write race, and it returns `{"id", "outcome"}` with `outcome` `"created"`, `"updated"`
   or `"unchanged"` (an identical document writes no revision).
3. **`find` by the key** when you only want to read: `db.find(type, field, value)` returns the active
   units whose field equals the value. A deleted unit does not match, and its key is free again.

## In a batch

`("upsert", type, field, document)` is also a batch op, so many entities (and relations to them) go
in one commit. Its result is in `out["upserts"]` (with `detailed=True`), not in `out["units"]`.
A `relate` op can name an upsert by its position in the ops list: `("upsert", "file", "path", {...}),
("upsert", "sym", "qn", {...}), ("relate", 0, 1, "defines")` links the two units. Several upserts of one key
in a batch make one unit, and the last document wins; each reports that unit's id and one outcome
(`created` when the first made it). A raw
`("store", type, payload)` of a typed type is still refused by the schema
(`SchemaError: not a record`): use `upsert` or `store_document`.

## Example

The full script (it creates its own scratch directory, so it runs as is):

```python
import os, tempfile
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

ada = db.upsert("person", "email", {"email": "ada@example.com", "name": "Ada"})
print("created:", ada["outcome"])
print("same document:", db.upsert("person", "email", {"email": "ada@example.com", "name": "Ada"})["outcome"])
print("new name:", db.upsert("person", "email", {"email": "ada@example.com", "name": "Ada Lovelace"})["outcome"])
print("found:", [db.read_document(u.id)["name"] for u in db.find("person", "email", "ada@example.com")])
print("revisions:", [r.kind for r in db.history(ada["id"])])

try:
    db.store_document("person", {"email": "ada@example.com", "name": "Someone else"})
except kerno.SchemaError as e:
    print("duplicate key refused:", e)

try:
    db.commit([("store", "person", "raw payload")])
except kerno.SchemaError as e:
    print("batch refused:", e)

db.delete(ada["id"])
print("after delete, find:", db.find("person", "email", "ada@example.com"))
print("after delete, upsert:", db.upsert("person", "email", {"email": "ada@example.com", "name": "Ada"})["outcome"])
db.close()
```

Its output, as run against this tree:

```text
created: created
same document: unchanged
new name: updated
found: ['Ada Lovelace']
revisions: ['update', 'create']
duplicate key refused: refused by the schema: unit 01a200ee69727613a8c9c1cd6f30b7ee: another unit already holds the unique value of "email"
batch refused: refused by the schema: unit 01a200ee69727614be2b858de7f2a2b2: not a record (unknown version byte)
after delete, find: []
after delete, upsert: created
```

The unit ids in the two refusal lines differ on every run; the messages are otherwise the same.

## Errors

- A field that is not declared `unique`, or a type with no schema: `InvalidArgumentError`
  (Node `INVALID_ARGUMENT`).
- A document with no value in the field, or one the schema refuses: `SchemaError` (Node `SCHEMA`).
- Only active units hold a key: an archived or deleted unit does not, so upserting its key creates a
  new unit.

## Notes

- `find` returns active units only. It does not tell you whether the key was ever used by a deleted
  unit; read the history for that.
- Two processes cannot both hold a writable database, so a multi-process writer needs one writer
  process and the others reading with `read_only=True`.
- Node has the same calls: `db.upsert(type, field, document)` and `{ op: 'upsert', ... }` in `commit`.
