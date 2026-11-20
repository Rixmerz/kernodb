# kerno-client (Python)

A pure-Python HTTP client for [`kerno-server`](../../crates/kerno-server), the Kerno database
server. Standard library only (Python 3.9 or newer), no runtime dependencies. It is typed
(`py.typed`) and not published anywhere yet: use it from a checkout.

```python
from kerno_client import ApiError, FieldDef, GraphLeg, KernoClient

kerno = KernoClient("http://127.0.0.1:8080")

# Units: a payload is a str (text), bytes, or a dict of fields for a type that has a schema.
note = kerno.create_unit("note", "Rust has no GC")
topic = kerno.create_unit("topic", "memory")
kerno.relate(note.id, topic.id, "about")
kerno.update_unit(note.id, "Rust has no tracing GC", expected_revision=1)

# Typed records, and the query language.
kerno.define_schema("user", [FieldDef("email", "text", unique=True), FieldDef("age", "int", indexed=True)])
kerno.create_unit("user", {"email": "ada@example.com", "age": 36})
for row in kerno.query("FIND user AS u WHERE u.age >= 30 ORDER BY u.age").rows:
    print(row[0].payload)
print(kerno.explain("FIND user AS u WHERE u.age = 36").chosen)

# Hybrid retrieval: keywords, relations and (caller-supplied) vectors, fused by reciprocal rank.
for hit in kerno.search(text="rust", graph=GraphLeg([topic.id])):
    print(hit.id, hit.score, hit.ranks)

try:
    kerno.get_unit("00" * 16)
except ApiError as e:
    print(e.status, e.code, e.message)  # 404 not_found unit 0000... not found
```

Every method raises `ApiError` when the server refuses the call (`.status`, `.code`, `.message`;
the codes are in the server README) and `TransportError` when it cannot be reached; both are
`KernoError`s.

A few things to know:

* Bytes that are valid UTF-8 come back as `str`: the server cannot tell them apart.
* Passing an `int` for a `float` field is fine; it is stored and read back as a float.
* `search`'s vector leg ranks the vectors you pass in: the server keeps no vector index.
* The server has no authentication or TLS yet, so neither has this client.

## Tests

```sh
pip install pytest
pytest            # from clients/python
```

The tests start the real server: `$KERNO_SERVER_BIN` when set, else the binary
`cargo build -p kerno-server` makes. A missing binary fails the run, it does not skip it.
