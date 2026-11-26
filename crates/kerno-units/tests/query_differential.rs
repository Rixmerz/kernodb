//! Phase C: the query language (`kerno_units::lang`, docs/query-language.md).
//!
//! The planned path (`execute_query`, with its field indexes, pushed-down terms
//! and early stop) and the reference evaluator (`reference_execute_query`) are
//! held to **a model that shares no code with the crate**: generated data is
//! kept in plain Rust structs, generated queries are evaluated over them by
//! the definition in the document, and all three must give the same ids in the
//! same order. Each test runs on `emulated_log` and on `kerno_db`.

mod harness;

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open<E: Target>() -> UnitDb<E> {
    open_checking::<E>(false)
}

/// `verify_payload_on_read`: whether a read checks each payload's hash.
fn open_checking<E: Target>(verify_payload_on_read: bool) -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c.verify_payload_on_read = verify_payload_on_read;
    open_units::<E>(SimVfs::new(5).as_vfs(), &c).expect("open")
}

// -------------------------------------------------------------------- data

use FieldType::{Bool, Bytes, Float, Int, Text};

/// `(unit type, field, type, indexed)`: the schemas of the generated data.
/// `ghost` has no schema: it can be joined through and never read.
const FIELDS: &[(&str, &str, FieldType, bool)] = &[
    ("person", "name", Text, false),
    ("person", "age", Int, true),
    ("person", "score", Float, true),
    ("person", "email", Text, true),
    ("person", "tag", Text, false),
    ("person", "flag", Bool, true),
    ("person", "blob", Bytes, true),
    ("team", "name", Text, true),
    ("team", "size", Int, true),
    ("team", "label", Text, false),
    ("repo", "title", Text, false),
    ("repo", "stars", Int, true),
];

fn schema_of(unit_type: &str) -> Vec<FieldDef> {
    FIELDS
        .iter()
        .filter(|f| f.0 == unit_type)
        .map(|&(_, name, ty, indexed)| {
            let def = FieldDef::new(name, ty);
            match (name, indexed) {
                ("email", _) | ("name", true) => def.unique(),
                (_, true) => def.indexed(),
                _ if name == "name" || name == "title" => def.required(),
                _ => def,
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
struct MUnit {
    id: UnitId,
    unit_type: &'static str,
    fields: BTreeMap<&'static str, FieldValue>,
    active: bool,
}

#[derive(Default)]
struct Model {
    units: Vec<MUnit>,
    /// `(relation type, from, to)`, once per relation (parallel ones repeat).
    relations: Vec<(&'static str, UnitId, UnitId)>,
}

/// A value of `field`, from a small pool so that equal values repeat.
fn pooled(rng: &mut Rng, field: &str) -> FieldValue {
    let pick = |rng: &mut Rng, n: usize| rng.below(n as u64) as usize;
    match field {
        "name" | "title" => FieldValue::Text(["ada", "bob", "cy", "dee", "eve", "fay", "gus"][pick(rng, 7)].into()),
        "age" => FieldValue::Int(18 + rng.below(9) as i64),
        "score" => FieldValue::Float([-1.5, -0.0, 0.0, 0.5, 2.0, 3.25][pick(rng, 6)]),
        "tag" | "label" => FieldValue::Text(["a", "b", "c"][pick(rng, 3)].into()),
        "flag" => FieldValue::Bool(rng.chance(1, 2)),
        "blob" => FieldValue::Bytes([vec![], vec![0], vec![0, 1], vec![255]][pick(rng, 4)].clone()),
        "size" => FieldValue::Int(1 + rng.below(6) as i64),
        "stars" => FieldValue::Int(rng.below(6) as i64),
        other => panic!("no pool for {other}"),
    }
}

fn commit<E: Target, T>(
    db: &UnitDb<E>,
    f: impl FnOnce(&mut ChangeSet<'_, E>) -> Result<T, UnitError>,
) -> Result<T, UnitError> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let out = f(&mut cs)?;
    cs.commit()?;
    Ok(out)
}

fn build<E: Target>(db: &UnitDb<E>, seed: u64) -> Model {
    let mut rng = Rng::new(seed);
    for t in ["person", "team", "repo"] {
        db.define_schema(t, schema_of(t), ChangeMeta::default()).expect("schema");
    }
    let mut m = Model::default();
    let mut serial = 0u32;
    let mut make = |db: &UnitDb<E>, rng: &mut Rng, m: &mut Model, unit_type: &'static str| {
        let mut fields = BTreeMap::new();
        for &(_, name, _, _) in FIELDS.iter().filter(|f| f.0 == unit_type) {
            serial += 1;
            let value = match (name, unit_type) {
                // About a third of the persons have no email: `IS NULL` has something to find.
                ("email", _) => (!rng.chance(1, 3)).then(|| FieldValue::Text(format!("p{serial}@x"))),
                ("name", "team") => Some(FieldValue::Text(format!("t{serial}"))),
                ("name" | "title", _) => Some(pooled(rng, name)),
                _ => rng.chance(4, 5).then(|| pooled(rng, name)),
            };
            if let Some(v) = value {
                fields.insert(name, v);
            }
        }
        let payload = if unit_type == "ghost" {
            vec![7, 7]
        } else {
            fields.iter().fold(Document::new(), |d, (k, v)| d.with(*k, v.clone())).encode().expect("encode")
        };
        let id = commit(db, |cs| {
            cs.create(NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata: vec![], payload })
        })
        .expect("create");
        m.units.push(MUnit { id, unit_type, fields, active: true });
    };
    for (unit_type, count) in [("person", 26), ("team", 7), ("repo", 9), ("ghost", 5)] {
        for _ in 0..count {
            make(db, &mut rng, &mut m, unit_type);
        }
    }
    let of = |m: &Model, t: &str| m.units.iter().filter(|u| u.unit_type == t).map(|u| u.id).collect::<Vec<_>>();
    let (persons, teams, repos, ghosts) = (of(&m, "person"), of(&m, "team"), of(&m, "repo"), of(&m, "ghost"));
    let mut edges = Vec::new();
    for (rel, from, to, n) in [
        ("member_of", &persons, &teams, 40),
        ("knows", &persons, &persons, 45),
        ("owns", &teams, &repos, 14),
        ("follows", &persons, &ghosts, 12),
    ] {
        for _ in 0..n {
            let (a, b) = (from[rng.below(from.len() as u64) as usize], to[rng.below(to.len() as u64) as usize]);
            let id = commit(db, |cs| cs.create_relation(a, b, rel, RelationProvenance::Declared)).expect("relate");
            edges.push((rel, a, b, id));
            // A parallel relation of the same type: one pair, two relations.
            if rng.chance(1, 8) {
                let id = commit(db, |cs| cs.create_relation(a, b, rel, RelationProvenance::Declared)).expect("relate");
                edges.push((rel, a, b, id));
            }
        }
    }
    // Some relations go away again; the pair stays joined while one is left.
    for (_, _, _, id) in edges.iter().filter(|_| rng.chance(1, 10)).cloned().collect::<Vec<_>>() {
        commit(db, |cs| cs.delete_relation(id)).expect("unrelate");
        edges.retain(|e| e.3 != id);
    }
    m.relations = edges.into_iter().map(|(rel, a, b, _)| (rel, a, b)).collect();
    // A unit with a relation cannot be archived: units that never had one can, and half of those do.
    for unit_type in ["person", "team", "repo"] {
        for _ in 0..3 {
            make(db, &mut rng, &mut m, unit_type);
            let Some(orphan) = m.units.last_mut() else { continue };
            if rng.chance(1, 2) {
                commit(db, |cs| cs.archive(orphan.id, None)).expect("archive");
                orphan.active = false;
            }
        }
    }
    m
}

// ------------------------------------------------------------------ queries

/// A generated condition; `usize` is the position of an alias in the shape.
#[derive(Clone, Debug)]
enum Gen {
    Cmp(usize, &'static str, &'static str, FieldValue),
    Null(usize, &'static str, bool),
    And(Vec<Gen>),
    Or(Vec<Gen>),
    Not(Box<Gen>),
}

/// `(alias, unit type)` per binding and `(relation, "out"/"in", from position)` per join.
struct Shape {
    bindings: &'static [(&'static str, &'static str)],
    joins: &'static [(&'static str, &'static str, usize)],
}

const SHAPES: &[Shape] = &[
    Shape { bindings: &[("p", "person")], joins: &[] },
    Shape { bindings: &[("t", "team")], joins: &[] },
    Shape { bindings: &[("p", "person"), ("t", "team")], joins: &[("member_of", "out", 0)] },
    Shape { bindings: &[("a", "person"), ("b", "person")], joins: &[("knows", "out", 0)] },
    Shape { bindings: &[("t", "team"), ("p", "person")], joins: &[("member_of", "in", 0)] },
    Shape {
        bindings: &[("p", "person"), ("t", "team"), ("r", "repo")],
        joins: &[("member_of", "out", 0), ("owns", "out", 1)],
    },
    Shape {
        bindings: &[("p", "person"), ("t", "team"), ("q", "person")],
        joins: &[("member_of", "out", 0), ("member_of", "in", 1)],
    },
    Shape {
        bindings: &[("p", "person"), ("g", "ghost"), ("t", "team")],
        joins: &[("follows", "out", 0), ("member_of", "out", 0)],
    },
    Shape { bindings: &[("t", "team"), ("r", "repo")], joins: &[("owns", "out", 0)] },
];

fn literal_text(v: &FieldValue) -> String {
    match v {
        FieldValue::Bool(b) => b.to_string(),
        FieldValue::Int(i) => i.to_string(),
        FieldValue::Float(f) => format!("{f:?}"),
        FieldValue::Text(s) => format!("{s:?}"),
        FieldValue::Bytes(b) => format!("x\"{}\"", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
    }
}

impl Gen {
    fn text(&self, shape: &Shape) -> String {
        match self {
            Gen::Cmp(a, f, op, v) => format!("{}.{f} {op} {}", shape.bindings[*a].0, literal_text(v)),
            Gen::Null(a, f, negated) => {
                format!("{}.{f} IS {}NULL", shape.bindings[*a].0, if *negated { "NOT " } else { "" })
            }
            Gen::And(v) => v.iter().map(|g| g.group(shape)).collect::<Vec<_>>().join(" AND "),
            Gen::Or(v) => v.iter().map(|g| g.group(shape)).collect::<Vec<_>>().join(" OR "),
            Gen::Not(g) => format!("NOT {}", g.group(shape)),
        }
    }

    fn group(&self, shape: &Shape) -> String {
        match self {
            Gen::Cmp(..) | Gen::Null(..) => self.text(shape),
            _ => format!("({})", self.text(shape)),
        }
    }

    /// By the definition: a comparison with a missing field is false.
    fn holds(&self, row: &[&MUnit]) -> bool {
        match self {
            Gen::Cmp(a, f, op, lit) => row[*a].fields.get(f).is_some_and(|v| {
                let ord = order(v, lit);
                match *op {
                    "=" => ord == Ordering::Equal,
                    "!=" => ord != Ordering::Equal,
                    "<" => ord == Ordering::Less,
                    "<=" => ord != Ordering::Greater,
                    ">" => ord == Ordering::Greater,
                    ">=" => ord != Ordering::Less,
                    other => panic!("operator {other}"),
                }
            }),
            Gen::Null(a, f, negated) => row[*a].fields.contains_key(f) == *negated,
            Gen::And(v) => v.iter().all(|g| g.holds(row)),
            Gen::Or(v) => v.iter().any(|g| g.holds(row)),
            Gen::Not(g) => !g.holds(row),
        }
    }
}

/// The order of two values of one type.
fn order(a: &FieldValue, b: &FieldValue) -> Ordering {
    match (a, b) {
        (FieldValue::Bool(a), FieldValue::Bool(b)) => a.cmp(b),
        (FieldValue::Int(a), FieldValue::Int(b)) => a.cmp(b),
        (FieldValue::Float(a), FieldValue::Float(b)) => a.partial_cmp(b).expect("no NaN"),
        (FieldValue::Text(a), FieldValue::Text(b)) => a.as_bytes().cmp(b.as_bytes()),
        (FieldValue::Bytes(a), FieldValue::Bytes(b)) => a.cmp(b),
        _ => panic!("one type per field"),
    }
}

struct Generated {
    text: String,
    expected: Vec<Vec<UnitId>>,
}

fn typed_fields(unit_type: &str) -> Vec<(&'static str, FieldType)> {
    FIELDS.iter().filter(|f| f.0 == unit_type).map(|f| (f.1, f.2)).collect()
}

fn gen_cond(rng: &mut Rng, m: &Model, shape: &Shape, depth: u32) -> Gen {
    let readable: Vec<usize> = (0..shape.bindings.len()).filter(|&i| shape.bindings[i].1 != "ghost").collect();
    if depth < 2 && rng.chance(2, 5) {
        let n = 2 + rng.below(2) as usize;
        let terms = (0..n).map(|_| gen_cond(rng, m, shape, depth + 1)).collect();
        return match rng.below(3) {
            0 => Gen::And(terms),
            1 => Gen::Or(terms),
            _ => Gen::Not(Box::new(gen_cond(rng, m, shape, depth + 1))),
        };
    }
    let a = readable[rng.below(readable.len() as u64) as usize];
    let fields = typed_fields(shape.bindings[a].1);
    let (field, ty) = fields[rng.below(fields.len() as u64) as usize];
    if rng.chance(1, 8) {
        return Gen::Null(a, field, rng.chance(1, 2));
    }
    let op = ["=", "=", "!=", "<", "<=", ">", ">="][rng.below(7) as usize];
    // Mostly a value some unit holds, so that equality finds rows; else any value of the pool.
    let held: Vec<&FieldValue> =
        m.units.iter().filter(|u| u.unit_type == shape.bindings[a].1).filter_map(|u| u.fields.get(field)).collect();
    let value = if !held.is_empty() && rng.chance(7, 10) {
        held[rng.below(held.len() as u64) as usize].clone()
    } else if field == "email" {
        FieldValue::Text(format!("p{}@x", rng.below(200)))
    } else if field == "name" && shape.bindings[a].1 == "team" {
        FieldValue::Text(format!("t{}", rng.below(200)))
    } else {
        pooled(rng, field)
    };
    assert_eq!(value.field_type(), ty);
    Gen::Cmp(a, field, op, value)
}

fn generate(rng: &mut Rng, m: &Model) -> Generated {
    let shape = &SHAPES[rng.below(SHAPES.len() as u64) as usize];
    let filter = rng.chance(4, 5).then(|| {
        let n = 1 + rng.below(3) as usize;
        let terms: Vec<Gen> = (0..n).map(|_| gen_cond(rng, m, shape, 0)).collect();
        if n == 1 { terms.into_iter().next().expect("one term") } else { Gen::And(terms) }
    });
    let readable: Vec<usize> = (0..shape.bindings.len()).filter(|&i| shape.bindings[i].1 != "ghost").collect();
    let keys: Vec<(usize, &'static str, bool)> = (0..rng.below(3))
        .map(|_| {
            let a = readable[rng.below(readable.len() as u64) as usize];
            let fields = typed_fields(shape.bindings[a].1);
            (a, fields[rng.below(fields.len() as u64) as usize].0, rng.chance(1, 2))
        })
        .collect();
    let limit = rng.chance(2, 5).then(|| rng.below(9) as usize);
    let offset = if rng.chance(3, 10) { 1 + rng.below(4) as usize } else { 0 };

    // The text.
    let mut text = format!("FIND {} AS {}", shape.bindings[0].1, shape.bindings[0].0);
    for (i, (rel, dir, from)) in shape.joins.iter().enumerate() {
        let (alias, unit_type) = shape.bindings[i + 1];
        text.push_str(&format!(
            " JOIN {unit_type} AS {alias} VIA {rel} {} FROM {}",
            dir.to_uppercase(),
            shape.bindings[*from].0
        ));
    }
    if let Some(f) = &filter {
        text.push_str(&format!(" WHERE {}", f.text(shape)));
    }
    for (i, (a, field, desc)) in keys.iter().enumerate() {
        text.push_str(&format!(
            "{} {}.{field}{}",
            if i == 0 { " ORDER BY" } else { "," },
            shape.bindings[*a].0,
            if *desc { " DESC" } else { "" }
        ));
    }
    if let Some(n) = limit {
        text.push_str(&format!(" LIMIT {n}"));
    }
    if offset > 0 {
        text.push_str(&format!(" OFFSET {offset}"));
    }

    // The answer, by the definition.
    let mut rows: Vec<Vec<&MUnit>> = vec![Vec::new()];
    for (i, (_, unit_type)) in shape.bindings.iter().enumerate() {
        let mut next = Vec::new();
        for row in &rows {
            for u in m.units.iter().filter(|u| u.active && u.unit_type == *unit_type) {
                let joined = i == 0 || {
                    let (rel, dir, from) = shape.joins[i - 1];
                    let (l, r) = if dir == "out" { (row[from].id, u.id) } else { (u.id, row[from].id) };
                    m.relations.iter().any(|&(t, a, b)| t == rel && a == l && b == r)
                };
                if joined {
                    let mut r = row.clone();
                    r.push(u);
                    next.push(r);
                }
            }
        }
        rows = next;
    }
    rows.retain(|r| filter.as_ref().is_none_or(|f| f.holds(r)));
    rows.sort_by(|x, y| {
        for &(a, field, desc) in &keys {
            let ord = match (x[a].fields.get(field), y[a].fields.get(field)) {
                (None, None) => Ordering::Equal,
                (None, _) => Ordering::Less,
                (_, None) => Ordering::Greater,
                (Some(p), Some(q)) => order(p, q),
            };
            let ord = if desc { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        x.iter().map(|u| u.id).cmp(y.iter().map(|u| u.id))
    });
    let expected = rows
        .into_iter()
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .map(|r| r.into_iter().map(|u| u.id).collect())
        .collect();
    Generated { text, expected }
}

fn ids(rows: &[Vec<Unit>]) -> Vec<Vec<UnitId>> {
    rows.iter().map(|r| r.iter().map(|u| u.id).collect()).collect()
}

/// Runs `queries` generated queries against `db` and counts the access paths used.
fn differential<E: Target>(db: &UnitDb<E>, m: &Model, rng: &mut Rng, queries: usize) -> BTreeMap<String, usize> {
    let key = FingerprintKey::generate().expect("key");
    let snap = db.snapshot().expect("snapshot");
    let mut used = BTreeMap::new();
    let mut non_empty = 0;
    for _ in 0..queries {
        let g = generate(rng, m);
        let q = Query::parse(&g.text).unwrap_or_else(|e| panic!("{}: {e}", g.text));
        assert_eq!(Query::parse(&q.to_string()).expect("the printed query parses"), q, "{}", g.text);
        let planned = snap.execute_query(&q).unwrap_or_else(|e| panic!("{}: {e}", g.text));
        assert_eq!(ids(&planned.rows), g.expected, "planned path: {}", g.text);
        assert_eq!(ids(&snap.reference_execute_query(&q).expect("reference")), g.expected, "reference: {}", g.text);
        // The plan explained is the plan that ran.
        let plan = snap.explain_query(&key, &q).expect("explain");
        assert_eq!(plan.chosen().access, planned.access, "{}", g.text);
        assert_eq!(planned.aliases.len(), q.joins.len() + 1);
        non_empty += usize::from(!g.expected.is_empty());
        *used.entry(format!("{:?}", planned.access).split(' ').next().unwrap_or("").to_owned()).or_default() += 1;
    }
    assert!(non_empty * 4 > queries, "the generated queries must find rows: {non_empty} of {queries}");
    used
}

fn generated_queries_match_the_model<E: Target>() {
    for seed in [1u64, 2, 3] {
        let db = open::<E>();
        let m = build(&db, seed);
        let mut rng = Rng::new(seed.wrapping_mul(1000));
        // The type index is not built: the scan reads every unit and field indexes are cheaper.
        let unbuilt = differential(&db, &m, &mut rng, 250);
        // Built: the planner's own cost decides the scan.
        db.rebuild_index().expect("rebuild");
        let built = differential(&db, &m, &mut rng, 250);
        for used in [&unbuilt, &built] {
            assert!(used.get("TypeScan").copied().unwrap_or(0) > 0, "{used:?}");
        }
        assert!(unbuilt.get("FieldIndex").copied().unwrap_or(0) > 20, "{unbuilt:?}");
    }
}

// ---------------------------------------------------------- hand-written

fn account_db<E: Target>() -> (UnitDb<E>, Vec<UnitId>) {
    account_db_checking::<E>(false)
}

fn account_db_checking<E: Target>(verify_payload_on_read: bool) -> (UnitDb<E>, Vec<UnitId>) {
    let db = open_checking::<E>(verify_payload_on_read);
    db.define_schema(
        "acct",
        vec![
            FieldDef::new("name", Text).required(),
            FieldDef::new("n", Int).indexed(),
            FieldDef::new("opt", Int),
            FieldDef::new("email", Text).unique(),
        ],
        ChangeMeta::default(),
    )
    .expect("schema");
    let mut out = Vec::new();
    for (name, n, opt, email) in
        [("a", 1, Some(5), Some("a@x")), ("b", 2, None, Some("b@x")), ("c", 3, Some(5), None), ("d", 3, Some(9), None)]
    {
        let mut d = Document::new().with("name", FieldValue::Text(name.into())).with("n", FieldValue::Int(n));
        if let Some(o) = opt {
            d = d.with("opt", FieldValue::Int(o));
        }
        if let Some(e) = email {
            d = d.with("email", FieldValue::Text(e.into()));
        }
        let payload = d.encode().expect("encode");
        out.push(
            commit(&db, |cs| {
                cs.create(NewUnit { unit_type: "acct".into(), schema_version: 1, metadata: vec![], payload })
            })
            .expect("create"),
        );
    }
    (db, out)
}

fn run<E: Target>(db: &UnitDb<E>, text: &str) -> Vec<Vec<UnitId>> {
    let snap = db.snapshot().expect("snapshot");
    let q = Query::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
    let planned = ids(&snap.execute_query(&q).unwrap_or_else(|e| panic!("{text}: {e}")).rows);
    assert_eq!(ids(&snap.reference_execute_query(&q).expect("reference")), planned, "{text}");
    planned
}

fn missing_fields_compare_false_and_null_finds_them<E: Target>() {
    let (db, id) = account_db::<E>();
    let one = |v: &[usize]| v.iter().map(|&i| vec![id[i]]).collect::<Vec<_>>();
    assert_eq!(run(&db, "FIND acct WHERE opt = 5"), one(&[0, 2]));
    // `!=` is false for the unit with no `opt`; `NOT` turns it true again.
    assert_eq!(run(&db, "FIND acct WHERE opt != 5"), one(&[3]));
    assert_eq!(run(&db, "FIND acct WHERE NOT opt = 5"), one(&[1, 3]));
    assert_eq!(run(&db, "FIND acct WHERE opt IS NULL"), one(&[1]));
    assert_eq!(run(&db, "FIND acct WHERE opt IS NOT NULL AND email IS NULL"), one(&[2, 3]));
    assert_eq!(run(&db, "FIND acct WHERE opt < 100 OR opt IS NULL"), one(&[0, 1, 2, 3]));
}

fn ordering_puts_missing_first_and_breaks_ties_by_id<E: Target>() {
    let (db, id) = account_db::<E>();
    let one = |v: &[usize]| v.iter().map(|&i| vec![id[i]]).collect::<Vec<_>>();
    assert_eq!(run(&db, "FIND acct ORDER BY opt"), one(&[1, 0, 2, 3]));
    assert_eq!(run(&db, "FIND acct ORDER BY opt DESC"), one(&[3, 0, 2, 1]));
    assert_eq!(run(&db, "FIND acct ORDER BY n DESC, name DESC"), one(&[3, 2, 1, 0]));
    assert_eq!(run(&db, "FIND acct ORDER BY n DESC"), one(&[2, 3, 1, 0]));
    assert_eq!(run(&db, "FIND acct ORDER BY name DESC LIMIT 2 OFFSET 1"), one(&[2, 1]));
    assert_eq!(run(&db, "FIND acct LIMIT 0"), Vec::<Vec<UnitId>>::new());
    assert_eq!(run(&db, "FIND acct OFFSET 9"), Vec::<Vec<UnitId>>::new());
    assert_eq!(run(&db, "FIND acct LIMIT 2 OFFSET 3"), one(&[3]));
}

fn the_plan_chooses_a_field_index_only_when_it_is_cheaper_and_valid<E: Target>() {
    let (db, id) = account_db::<E>();
    let key = FingerprintKey::generate().expect("key");
    let snap = db.snapshot().expect("snapshot");
    let explain = |text: &str| snap.explain_query(&key, &Query::parse(text).expect("parse")).expect("explain");
    let access = |text: &str| explain(text).chosen().access.clone();
    let index = |f: &str| Access::FieldIndex { field: f.into() };

    // One entry of four: the unique index wins; the plan lists both candidates in order.
    let plan = explain("FIND acct WHERE email = \"a@x\"");
    assert_eq!(plan.chosen().access, index("email"));
    assert_eq!(
        plan.candidates.iter().map(|c| c.access.clone()).collect::<Vec<_>>(),
        [Access::TypeScan, index("email")]
    );
    assert!(plan.candidates[1].cost < plan.candidates[0].cost);
    assert_eq!((plan.root_alias.as_str(), plan.root_type.as_str()), ("acct", "acct"));
    assert_eq!(plan.scan.fingerprint().kind(), QueryKind::Units);
    // The condition has to hold for every row: not under OR or NOT, not `!=`, not unindexed.
    for text in [
        "FIND acct WHERE email = \"a@x\" OR n = 1",
        "FIND acct WHERE NOT n = 1",
        "FIND acct WHERE n != 1",
        "FIND acct WHERE opt = 5",
        "FIND acct WHERE email IS NULL",
    ] {
        assert_eq!(access(text), Access::TypeScan, "{text}");
        assert_eq!(explain(text).candidates.len(), 1, "{text}");
    }
    // Every usable term is a candidate; the cheapest range wins (n = 3 holds two units, email one).
    let plan = explain("FIND acct WHERE n = 3 AND email = \"b@x\"");
    assert_eq!(plan.candidates.len(), 3);
    assert_eq!(plan.chosen().access, index("email"));
    // The cost of a field index is the entries its range holds (n is 1, 2, 3, 3), each opened.
    let per_entry = 1 + kerno_units::planner::OPEN_READS;
    for (cond, entries) in
        [("n = 3", 2), ("n < 3", 2), ("n <= 3", 4), ("n > 1", 3), ("n >= 1", 4), ("n > 3", 0), ("n < 1", 0)]
    {
        let plan = explain(&format!("FIND acct WHERE {cond}"));
        assert_eq!(plan.candidates[1].cost, Cost(entries * per_entry), "{cond}");
    }
    // An index key holds at most MAX_INDEXED_VALUE_LEN bytes: a text of that many less its two
    // terminator bytes still looks up, one byte more is left to the scan.
    let longest = kerno_units::schema::MAX_INDEXED_VALUE_LEN - 2;
    assert_eq!(explain(&format!("FIND acct WHERE email = \"{}\"", "z".repeat(longest))).candidates.len(), 2);
    assert_eq!(explain(&format!("FIND acct WHERE email = \"{}\"", "z".repeat(longest + 1))).candidates.len(), 1);
    // The ends of the integers: an empty range of the index, which is no reason to fail.
    for cond in ["n > 9223372036854775807", "n < -9223372036854775808", "n >= 9223372036854775807"] {
        assert_eq!(explain(&format!("FIND acct WHERE {cond}")).candidates.len(), 2, "{cond}");
        assert!(run(&db, &format!("FIND acct WHERE {cond}")).is_empty(), "{cond}");
    }
    assert_eq!(run(&db, "FIND acct WHERE n <= 9223372036854775807").len(), 4);
    assert_eq!(run(&db, "FIND acct WHERE n >= -9223372036854775808").len(), 4);
    // A value too long to be an index key cannot be looked up: the scan answers.
    let long = format!("FIND acct WHERE email = \"{}\"", "z".repeat(kerno_units::schema::MAX_INDEXED_VALUE_LEN + 1));
    assert_eq!(access(&long), Access::TypeScan);
    // The index answers what the scan answers, ranges included.
    let some = |v: &[usize]| v.iter().map(|&i| vec![id[i]]).collect::<Vec<_>>();
    for (text, expect) in [
        ("FIND acct WHERE n >= 2", some(&[1, 2, 3])),
        ("FIND acct WHERE n > 2", some(&[2, 3])),
        ("FIND acct WHERE n < 3 AND n > 1", some(&[1])),
        ("FIND acct WHERE n <= 1 OR n = 3", some(&[0, 2, 3])),
        ("FIND acct WHERE email >= \"b\" AND opt IS NULL", some(&[1])),
    ] {
        assert_eq!(run(&db, text), expect, "{text}");
    }
}

fn joins_pair_units_once_and_skip_units_that_are_not_active<E: Target>() {
    let (db, id) = account_db::<E>();
    let (a, b, c, d) = (id[0], id[1], id[2], id[3]);
    // A unit of another type with the same relation type, on both ends of `a`: it never joins an `acct`.
    commit(&db, |cs| {
        let other =
            cs.create(NewUnit { unit_type: "other".into(), schema_version: 1, metadata: vec![], payload: vec![1] })?;
        cs.create_relation(a, other, "to", RelationProvenance::Declared)?;
        cs.create_relation(other, a, "to", RelationProvenance::Declared)
    })
    .expect("relate another type");
    commit(&db, |cs| {
        cs.create_relation(a, b, "to", RelationProvenance::Declared)?;
        cs.create_relation(a, b, "to", RelationProvenance::Declared)?;
        cs.create_relation(a, c, "to", RelationProvenance::Declared)?;
        cs.create_relation(a, d, "other", RelationProvenance::Declared)?;
        cs.create_relation(c, a, "to", RelationProvenance::Declared)
    })
    .expect("relate");
    let pair = |x: UnitId, y: UnitId| vec![x, y];
    // Two parallel relations are one pair; the type of the relation matters.
    assert_eq!(run(&db, "FIND acct AS x JOIN acct AS y VIA to WHERE x.n = 1"), [pair(a, b), pair(a, c)]);
    assert_eq!(run(&db, "FIND acct AS x JOIN acct AS y VIA other"), [pair(a, d)]);
    // IN reverses the direction; FROM picks the binding the join starts from.
    assert_eq!(run(&db, "FIND acct AS x JOIN acct AS y VIA to IN"), [pair(a, c), pair(b, a), pair(c, a)]);
    assert_eq!(
        run(&db, "FIND acct AS x JOIN acct AS y VIA to JOIN acct AS z VIA other FROM x"),
        [vec![a, b, d], vec![a, c, d]]
    );
    // A condition on a joined binding, and an ordering on it.
    assert_eq!(run(&db, "FIND acct AS x JOIN acct AS y VIA to WHERE y.n >= 3"), [pair(a, c)]);
    assert_eq!(
        run(&db, "FIND acct AS x JOIN acct AS y VIA to ORDER BY y.name DESC"),
        [pair(a, c), pair(a, b), pair(c, a)]
    );
    // A relation that is deleted no longer joins.
    let gone = snap_relations(&db, c);
    commit(&db, |cs| gone.iter().try_for_each(|r| cs.delete_relation(*r))).expect("unrelate");
    assert_eq!(run(&db, "FIND acct AS x JOIN acct AS y VIA to"), [pair(a, b)]);
    assert_eq!(run(&db, "FIND acct AS x JOIN acct AS y VIA to IN"), [pair(b, a)]);
    // A type no unit has, and a relation type nothing uses, are empty and not an error.
    assert!(run(&db, "FIND acct JOIN acct AS y VIA nothing").is_empty());
    assert!(run(&db, "FIND ghost").is_empty());
}

fn snap_relations<E: Target>(db: &UnitDb<E>, unit: UnitId) -> Vec<RelationId> {
    let snap = db.snapshot().expect("snapshot");
    let mut out = snap.relations_of(unit, Direction::Out, None).expect("out");
    out.extend(snap.relations_of(unit, Direction::In, None).expect("in"));
    out.into_iter().map(|r| r.id).collect()
}

fn a_query_that_does_not_fit_the_schemas_is_refused<E: Target>() {
    let (db, _) = account_db::<E>();
    let snap = db.snapshot().expect("snapshot");
    let refused = |text: &str, why: &str| {
        let q = Query::parse(text).expect("parse");
        for e in [snap.execute_query(&q).expect_err(text), snap.reference_execute_query(&q).expect_err(text)] {
            assert!(matches!(&e, UnitError::InvalidArgument(m) if m.contains(why)), "{text}: {e}");
        }
        let key = FingerprintKey::generate().expect("key");
        assert!(matches!(snap.explain_query(&key, &q), Err(UnitError::InvalidArgument(_))), "{text}");
    };
    refused("FIND acct WHERE nope = 1", "declares no such field");
    refused("FIND acct WHERE n = \"1\"", "compared with");
    refused("FIND acct WHERE n = 1.0", "compared with");
    refused("FIND acct WHERE email = x\"00\"", "compared with");
    refused("FIND acct ORDER BY nope", "declares no such field");
    refused("FIND acct JOIN missing AS m VIA r WHERE m.x = 1", "has no schema");
    refused("FIND missing WHERE x = 1", "has no schema");
    refused("FIND acct JOIN acct AS y VIA r ORDER BY y.zzz", "declares no such field");

    // What `parse` cannot see in a hand-built query is refused too.
    let mut q = Query::parse("FIND acct AS x JOIN acct AS y VIA r").expect("parse");
    q.joins[0].from = "nobody".into();
    assert!(matches!(snap.execute_query(&q), Err(UnitError::InvalidArgument(m)) if m.contains("earlier alias")));
    let mut q = Query::parse("FIND acct AS x JOIN acct AS y VIA r").expect("parse");
    q.joins[0].binding.alias = "x".into();
    assert!(matches!(snap.execute_query(&q), Err(UnitError::InvalidArgument(m)) if m.contains("used twice")));
    let mut q = Query::parse("FIND acct AS x WHERE n = 1").expect("parse");
    q.order.push(OrderKey { field: FieldRef { alias: "nobody".into(), field: "n".into() }, descending: false });
    assert!(matches!(snap.execute_query(&q), Err(UnitError::InvalidArgument(m)) if m.contains("not an alias")));
    // And the text goes through the same error type.
    let e: UnitError = Query::parse("FIND").expect_err("no type").into();
    assert!(matches!(e, UnitError::InvalidArgument(m) if m.contains("at byte 4")));
}

fn the_first_binding_is_read_through_the_result_of_the_planner<E: Target>() {
    let (db, _) = account_db::<E>();
    let q = Query::parse("FIND acct").expect("parse");
    let snap = db.snapshot().expect("snapshot");
    // No index built: the planner answers with the reference scan, and says so.
    assert_eq!(snap.execute_query(&q).expect("run").scan_path, Some(PlanPath::UnitsReference));
    db.rebuild_index().expect("rebuild");
    let snap = db.snapshot().expect("snapshot");
    let r = snap.execute_query(&q).expect("run");
    let filter = UnitFilter { unit_type: Some("acct".into()), schema_version: None };
    assert_eq!((r.access, r.scan_path), (Access::TypeScan, Some(snap.query(&filter).expect("planner").path)));
    // A field index read is not a planner scan.
    let r = snap.execute_query(&Query::parse("FIND acct WHERE n = 3").expect("parse")).expect("run");
    assert_eq!((r.access, r.scan_path), (Access::FieldIndex { field: "n".into() }, None));
    assert_eq!(r.aliases, ["acct"]);
}

fn find_range_and_a_query_agree<E: Target>() {
    let (db, _) = account_db::<E>();
    let snap = db.snapshot().expect("snapshot");
    let by_index = snap
        .find_range("acct", "n", Bound::Included(&FieldValue::Int(2)), Bound::Unbounded, usize::MAX)
        .expect("range");
    let mut by_query =
        ids(&snap.execute_query(&Query::parse("FIND acct WHERE n >= 2").expect("parse")).expect("run").rows);
    by_query.sort();
    let mut expect: Vec<Vec<UnitId>> = by_index.iter().map(|u| vec![u.id]).collect();
    expect.sort();
    assert_eq!(by_query, expect);
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            macro_rules! each {
                ($f:ident) => {
                    #[test]
                    fn $f() {
                        super::$f::<$e>();
                    }
                };
            }
            each!(generated_queries_match_the_model);
            each!(missing_fields_compare_false_and_null_finds_them);
            each!(ordering_puts_missing_first_and_breaks_ties_by_id);
            each!(the_plan_chooses_a_field_index_only_when_it_is_cheaper_and_valid);
            each!(joins_pair_units_once_and_skip_units_that_are_not_active);
            each!(a_query_that_does_not_fit_the_schemas_is_refused);
            each!(the_first_binding_is_read_through_the_result_of_the_planner);
            each!(find_range_and_a_query_agree);
        }
    )*};
}

engines!(emulated_log: kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>, kerno_db: kerno_storage::KernoDb);

// A query without `ORDER BY` produces its rows in final order and stops at `OFFSET + LIMIT`: the
// proof is a unit it would have to read to go on, damaged, which a limited query never reaches.
#[cfg(feature = "test-support")]
mod early_stop {
    use super::*;
    use kerno_units::codec::Key;

    fn limited_queries_do_not_read_past_their_last_row<E: Target>() {
        // The damage is a payload hash mismatch: only a read that checks hashes sees it.
        let (db, id) = account_db_checking::<E>(true);
        let mut ghosts = Vec::new();
        for owner in [id[0], id[2]] {
            let g = commit(&db, |cs| {
                cs.create(NewUnit { unit_type: "ghost".into(), schema_version: 1, metadata: vec![], payload: vec![1] })
            })
            .expect("create");
            commit(&db, |cs| cs.create_relation(owner, g, "follows", RelationProvenance::Declared)).expect("relate");
            ghosts.push(g);
        }
        // The payload, not the fields of the head: a scan of heads does not check it, opening the
        // unit does. The head of a unit this small carries its payload (`units-v10.md`), so that
        // is the copy an open reads, and the one to damage: its last byte.
        let head = Key::UnitHead { unit: ghosts[1] }.encode().expect("key");
        let mut damaged = db.raw_get(&head).expect("read").expect("the head exists");
        *damaged.last_mut().expect("not empty") ^= 0xFF;
        db.raw_commit(vec![(head, Some(damaged))]).expect("damage");
        // With the type index ready the scan of `acct` opens only its own units, not every unit.
        db.rebuild_index().expect("rebuild reads heads only");

        let snap = db.snapshot().expect("snapshot");
        let run = |text: &str| snap.execute_query(&Query::parse(text).expect("parse"));
        let first = run("FIND acct JOIN ghost AS g VIA follows LIMIT 1").expect("stops before the damage");
        assert_eq!(ids(&first.rows), [vec![id[0], ghosts[0]]]);
        // OFFSET counts: the second row is the damaged one.
        assert!(run("FIND acct JOIN ghost AS g VIA follows LIMIT 1 OFFSET 1").is_err());
        // Without a limit, or with an ordering that has to see every row, the damage is read.
        for text in [
            "FIND acct JOIN ghost AS g VIA follows",
            "FIND acct JOIN ghost AS g VIA follows ORDER BY n LIMIT 1",
            "FIND acct JOIN ghost AS g VIA follows LIMIT 2",
        ] {
            let e = run(text).expect_err(text);
            assert!(matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{text}: {e}");
        }
    }

    #[test]
    fn emulated_log() {
        limited_queries_do_not_read_past_their_last_row::<
            kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>,
        >();
    }

    #[test]
    fn kerno_db() {
        limited_queries_do_not_read_past_their_last_row::<kerno_storage::KernoDb>();
    }
}
