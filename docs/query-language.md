# The Kerno query language

A small declarative language over units and their relations. It reads: filters,
ordering, limits, and joins across unit types through typed relations. It never
writes. One query runs against one snapshot, so every row comes from the same
commit.

It is available as:

| Where | How |
|---|---|
| Rust, `kerno` | `Db::execute("FIND ...")`, `Db::explain("FIND ...")` |
| Rust, `kerno-units` | `Query::parse`, `UnitSnapshot::execute_query`, `explain_query`, `reference_execute_query` |
| Command line | `kerno <dir> query "FIND ..." [--explain] [--json]` |

```
FIND user AS u
JOIN team AS t VIA member_of
WHERE u.age >= 30 AND (t.name = "core" OR u.score > 4.5)
ORDER BY u.age DESC, u.email
LIMIT 10
```

## Grammar

Keywords are case-insensitive. Names are case-sensitive.

```
query     = "FIND" binding { join }
            [ "WHERE" expr ]
            [ "ORDER" "BY" key { "," key } ]
            [ "LIMIT" int ] [ "OFFSET" int ] ;

binding   = name [ "AS" name ] ;                 (* alias defaults to the type name *)
join      = "JOIN" binding "VIA" name [ "OUT" | "IN" ] [ "FROM" name ] ;
key       = ref [ "ASC" | "DESC" ] ;

expr      = and { "OR" and } ;
and       = not { "AND" not } ;
not       = "NOT" not | atom ;
atom      = "(" expr ")"
          | ref cmp literal
          | ref "IS" [ "NOT" ] "NULL" ;
cmp       = "=" | "!=" | "<" | "<=" | ">" | ">=" ;

ref       = name                                  (* a field of the first binding *)
          | name "." name ;                       (* alias "." field *)

literal   = int | float | string | bytes | "true" | "false" ;
int       = [ "-" ] digit { digit } ;             (* i64 *)
float     = [ "-" ] digit { digit } ( "." digit { digit } [ exp ] | exp ) ;
exp       = ( "e" | "E" ) [ "+" | "-" ] digit { digit } ;   (* finite *)
string    = '"' { char | escape } '"' ;           (* escapes: \\ \" \n \r \t \0 *)
bytes     = ( "x" | "X" ) '"' { hexdigit hexdigit } '"' ;
name      = word | string ;                       (* word: [A-Za-z_][A-Za-z0-9_]*, not a keyword *)
```

Keywords: `FIND AS JOIN VIA IN OUT FROM WHERE ORDER BY ASC DESC LIMIT OFFSET AND OR
NOT IS NULL TRUE FALSE`. A name that is a keyword, or has any other character
(a relation type such as `depends-on`), is written as a string: `VIA "depends-on"`.

Limits: a query text is at most 16 KiB, with at most 256 conditions and
parentheses or `NOT` nested at most 64 deep. A parse error names the byte where
it stopped.

## Meaning

**Bindings.** `FIND user AS u` ranges `u` over the `Active` units of type `user`.
Each `JOIN` adds one more binding. A row is one unit per binding, in the order
the bindings are written.

**Joins.** `JOIN team AS t VIA member_of` pairs a unit `a` of the binding it
starts from with every `Active` unit `b` of type `team` such that a relation of
type `member_of` goes from `a` to `b`. `IN` reverses it: from `b` to `a`.
`FROM x` starts the join from binding `x` instead of the previous one. The join
is over *pairs of units*: parallel relations of the same type give one row, not
several. A unit that is not `Active`, or of another type, never joins.

**Fields.** A field is read from the unit's payload, which is a typed record
(`kerno_units::Document`). So every binding a query reads a field of must be of
a type that has a schema declaring that field, and the literal it is compared
with must be of the field's type: `Int` with an integer, `Float` with a float
(`5.0`, not `5`), `Text` with a string, `Bytes` with `x"..."`, `Bool` with
`true`/`false`. A binding no field is read from (only joined through) needs no
schema. A violation is `InvalidArgument` and nothing runs.

**Missing fields.** An optional field a unit does not have is *missing*. A
comparison with a missing field is false, whatever the operator (`!=`
included); `NOT` then turns it true. `IS NULL` is true exactly for a missing
field, `IS NOT NULL` for a present one. Logic is two-valued; there is no
unknown.

**Order of values.** `Bool`: false before true. `Int` and `Float`: numerically
(`-0.0` equals `0.0`; a record never holds NaN). `Text` and `Bytes`: bytewise,
which for text is code point order.

**Ordering.** `ORDER BY` sorts by its keys left to right; a missing value sorts
before every present one under `ASC` and after under `DESC`. Rows equal on every
key are ordered by their unit ids, binding by binding, ascending, so the
result is always a total, reproducible order. Without `ORDER BY` the rows come
in that id order.

**Paging.** `OFFSET` skips that many rows, then `LIMIT` keeps at most that many.

## Plans and indexes

`explain` shows how a query would run, and `execute` runs that very plan. The
first binding is read through one of:

| Access | Reads | Cost |
|---|---|---|
| `TypeScan` | every `Active` unit of the type through the existing planner (`UnitSnapshot::query`: `IDX_TYPE`, `IDX_PRIMARY` or the reference scan, with its own `Plan`, freshness and fallback) | the planner's cost when its index is `Ready`, else `(1 + OPEN_READS)` per `Active` unit |
| `FieldIndex(f)` | the range of the user-defined index on field `f`, for a condition `f = v`, `f < v`, `f <= v`, `f > v` or `f >= v` that holds for every row (a top-level `AND` term, not under `OR` or `NOT`) | `(1 + OPEN_READS) × m`, with `m` the entries in the range, probed up to `PROBE` (a lower bound when the probe is full) |

The cheapest candidate is chosen; on a tie, `TypeScan`. The index only
*narrows*: every row is then checked against the whole `WHERE` again, so an
index can change how a query runs, never what it returns. `!=`, `IS NULL`, a
field without an index, a `Text` or `Bytes` value whose encoding exceeds
`MAX_INDEXED_VALUE_LEN`, and conditions on joined bindings are not indexed.

Each `JOIN` is a nested loop: one relation lookup per row so far, through
`UnitSnapshot::query_relations` (so each lookup is itself planned between the
adjacency index and the reference scan). A `WHERE` term is applied as soon as
every binding it mentions is bound, so a term on the first binding prunes before
any join runs. Without `ORDER BY`, rows are produced in final order, so the
query stops as soon as `OFFSET + LIMIT` rows exist.

`ponytail:` `ORDER BY` sorts the rows in memory (an index does not stream them in
order), and a join is a nested loop with no index on the joined side. Fine for an
embedded engine's working set; a hash or index join is the upgrade if a workload
shows it.

`ponytail:` an index read trusts the index the way `find_range` does: it checks
each unit it returns against its entry but cannot see an entry that is *missing*;
that is `verify_units` (UNIT-50). `reference_execute_query` does not use an index.

## Reference evaluator

`UnitSnapshot::reference_execute_query` is the definition of the answer: it
reads every `Active` unit and every relation, forms every combination of the
bindings, keeps those whose relations exist and whose `WHERE` holds, sorts and
pages. It uses no index and no early stop. `tests/query_differential.rs` compares
the planned path with it, and both with a model that shares no code with the
crate, on generated data and generated queries.

## Not in the language

Comparisons between two fields, arithmetic, functions, aggregates (`COUNT`,
`GROUP BY`), `DISTINCT`, subqueries, outer joins, parameters, and writes. Text
that comes from a user belongs in a string literal: escape `\` and `"` (or
build the query from `Query` values, which need no escaping).
