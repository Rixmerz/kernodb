//! Binding, planning and execution of a [`Query`] on one snapshot.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use super::{
    Access, AccessCandidate, CmpOp, Expr, FieldOf, FieldRef, JoinStep, Query, QueryPlan, QueryResult, compare_opt,
    document_of,
};
use crate::db::{Unit, UnitSnapshot};
use crate::error::UnitError;
use crate::fingerprint::{FingerprintKey, RelationQuery};
use crate::index::{IndexState, UnitFilter};
use crate::plan::Cost;
use crate::planner::{OPEN_READS, PROBE};
use crate::schema::{Document, FieldValue, MAX_INDEXED_VALUE_LEN, Schema};
use crate::types::{Direction, Lifecycle, UnitId};

/// A unit of a row with its record decoded once.
#[derive(Clone)]
pub(super) struct Loaded {
    pub(super) unit: Unit,
    pub(super) doc: Document,
}

/// An access path with the key range it reads.
struct Candidate {
    access: Access,
    cost: Cost,
    lo: Bound<FieldValue>,
    hi: Bound<FieldValue>,
}

fn invalid(m: String) -> UnitError {
    UnitError::InvalidArgument(format!("query: {m}"))
}

/// The terms of the top-level `AND` of a condition.
pub(super) fn conjuncts(filter: Option<&Expr>) -> Vec<&Expr> {
    match filter {
        None => Vec::new(),
        Some(Expr::And(v)) => v.iter().collect(),
        Some(e) => vec![e],
    }
}

fn reads(units: u64) -> Cost {
    Cost(units.saturating_mul(1 + OPEN_READS))
}

impl<E: Engine> UnitSnapshot<E> {
    /// Checks that the query is well formed against the schemas of this
    /// snapshot and returns the schema of each binding, in row order.
    pub(super) fn bind_query(&self, q: &Query) -> Result<Vec<Option<Schema>>, UnitError> {
        let aliases: Vec<&str> = q.bindings().map(|b| b.alias.as_str()).collect();
        for (i, a) in aliases.iter().enumerate() {
            if aliases[..i].contains(a) {
                return Err(invalid(format!("the alias {a:?} is used twice")));
            }
        }
        for (i, j) in q.joins.iter().enumerate() {
            if !aliases[..=i].contains(&j.from.as_str()) {
                return Err(invalid(format!("a join starts from {:?}, which is not an earlier alias", j.from)));
            }
        }
        let schemas = q.bindings().map(|b| self.schema(&b.unit_type)).collect::<Result<Vec<_>, _>>()?;
        for (field, literal) in q.field_uses() {
            let Some(pos) = q.position(&field.alias) else {
                return Err(invalid(format!("{:?} is not an alias of the query", field.alias)));
            };
            let unit_type = &q.bindings().nth(pos).map_or("", |b| b.unit_type.as_str());
            let Some(schema) = &schemas[pos] else {
                return Err(invalid(format!(
                    "{}: the type {unit_type:?} has no schema, so it has no fields to read",
                    field
                )));
            };
            let Some(def) = schema.field(&field.field) else {
                return Err(invalid(format!("{field}: the type {unit_type:?} declares no such field")));
            };
            if let Some(v) = literal.filter(|v| v.field_type() != def.ty) {
                return Err(invalid(format!("{field} is a {:?} field, compared with a {:?}", def.ty, v.field_type())));
            }
        }
        Ok(schemas)
    }

    /// The access paths of the first binding, costed, and the cheapest.
    fn decide_query(&self, q: &Query, schemas: &[Option<Schema>]) -> Result<(Vec<Candidate>, usize), UnitError> {
        let filter = UnitFilter { unit_type: Some(q.root.unit_type.clone()), schema_version: None };
        let (status, entries) = self.index_stats()?;
        let (status, decision) = self.decide_units_with(&filter, status, entries)?;
        // The planner costs the reference path from the entries of an index
        // that may not exist; without a ready index the scan reads every unit.
        let scan = if status.state == IndexState::Ready {
            decision.chosen().cost
        } else {
            reads(crate::type_count::expected_all(&self.snap)?.values().sum())
        };
        let mut candidates =
            vec![Candidate { access: Access::TypeScan, cost: scan, lo: Bound::Unbounded, hi: Bound::Unbounded }];
        let schema = schemas[0].as_ref();
        for term in conjuncts(q.filter.as_ref()) {
            let Expr::Cmp { field, op, value } = term else { continue };
            let indexed = schema.and_then(|s| s.field(&field.field)).is_some_and(|d| d.indexed);
            let fits = value.index_bytes().is_some_and(|e| e.len() <= MAX_INDEXED_VALUE_LEN);
            if field.alias != q.root.alias || *op == CmpOp::Ne || !indexed || !fits {
                continue;
            }
            let at = |b: fn(FieldValue) -> Bound<FieldValue>| b(value.clone());
            let (lo, hi) = match op {
                CmpOp::Eq => (at(Bound::Included), at(Bound::Included)),
                CmpOp::Lt => (Bound::Unbounded, at(Bound::Excluded)),
                CmpOp::Le => (Bound::Unbounded, at(Bound::Included)),
                CmpOp::Gt => (at(Bound::Excluded), Bound::Unbounded),
                _ => (at(Bound::Included), Bound::Unbounded),
            };
            let (start, end) = self.field_bounds(&q.root.unit_type, &field.field, lo.as_ref(), hi.as_ref())?;
            // A range that ends where it starts (`n > i64::MAX`) is empty, and the engine says so.
            let found = self.snap.scan(Bound::Included(&start), Bound::Excluded(&end), PROBE)?.len();
            candidates.push(Candidate {
                access: Access::FieldIndex { field: field.field.clone() },
                cost: reads(found as u64),
                lo,
                hi,
            });
        }
        // The first of equal costs: `TypeScan`, then the order written.
        let chosen = (0..candidates.len()).min_by_key(|&i| (candidates[i].cost, i)).unwrap_or(0);
        Ok((candidates, chosen))
    }

    /// How `execute_query` would answer `q` on this snapshot: the access paths
    /// of the first binding with their costs and the one chosen, what the
    /// existing planner does for a scan of that type (`key` fingerprints its
    /// shape, see [`explain_filter`](Self::explain_filter)), and the joins.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a query that does not fit the schemas (a field a
    /// type does not declare, a literal of the wrong type, a type with no
    /// schema that a field is read from); the engine fails or a record is corrupt.
    pub fn explain_query(&self, key: &FingerprintKey, q: &Query) -> Result<QueryPlan, UnitError> {
        let schemas = self.bind_query(q)?;
        let (candidates, chosen) = self.decide_query(q, &schemas)?;
        let filter = UnitFilter { unit_type: Some(q.root.unit_type.clone()), schema_version: None };
        Ok(QueryPlan {
            root_alias: q.root.alias.clone(),
            root_type: q.root.unit_type.clone(),
            candidates: candidates.into_iter().map(|c| AccessCandidate { access: c.access, cost: c.cost }).collect(),
            chosen,
            scan: self.explain_filter(key, &filter)?,
            joins: q
                .joins
                .iter()
                .map(|j| JoinStep {
                    alias: j.binding.alias.clone(),
                    unit_type: j.binding.unit_type.clone(),
                    relation_type: j.relation_type.clone(),
                    direction: j.direction,
                    from: j.from.clone(),
                })
                .collect(),
            planner_version: crate::planner::PLANNER_VERSION,
        })
    }

    /// Runs `q` on this snapshot through the plan [`explain_query`](Self::explain_query)
    /// shows. The answer is that of
    /// [`reference_execute_query`](Self::reference_execute_query).
    ///
    /// # Errors
    ///
    /// As [`explain_query`](Self::explain_query).
    pub fn execute_query(&self, q: &Query) -> Result<QueryResult, UnitError> {
        let schemas = self.bind_query(q)?;
        let (candidates, chosen) = self.decide_query(q, &schemas)?;
        let pick = &candidates[chosen];
        let (mut roots, scan_path) = match &pick.access {
            Access::TypeScan => {
                let out =
                    self.query(&UnitFilter { unit_type: Some(q.root.unit_type.clone()), schema_version: None })?;
                (out.units, Some(out.path))
            }
            Access::FieldIndex { field } => {
                (self.find_range(&q.root.unit_type, field, pick.lo.as_ref(), pick.hi.as_ref(), usize::MAX)?, None)
            }
        };
        // The canonical order of rows is by unit id, whatever order the path reads in.
        roots.sort_by_key(|u| u.id);

        let mut terms: Vec<Vec<&Expr>> = vec![Vec::new(); 1 + q.joins.len()];
        for term in conjuncts(q.filter.as_ref()) {
            let mut uses = Vec::new();
            term.collect(&mut uses);
            let level = uses.iter().filter_map(|(f, _)| q.position(&f.alias)).max().unwrap_or(0);
            terms[level].push(term);
        }
        let run = Run {
            snap: self,
            q,
            schemas: &schemas,
            terms,
            // Rows come out in their final order unless `ORDER BY` reorders them.
            stop: if q.order.is_empty() { q.limit.map(|n| n.saturating_add(q.offset)) } else { None },
        };
        let mut rows = Vec::new();
        for unit in roots {
            if run.done(&rows) {
                break;
            }
            let doc = document_of(&unit, schemas[0].as_ref())?;
            let mut row = vec![Loaded { unit, doc }];
            if run.holds(&row, 0) {
                run.extend(&mut row, &mut rows)?;
            }
        }
        Ok(QueryResult {
            aliases: q.bindings().map(|b| b.alias.clone()).collect(),
            rows: finish(q, rows),
            access: pick.access.clone(),
            scan_path,
        })
    }
}

/// The nested loops of one execution.
struct Run<'a, E: Engine> {
    snap: &'a UnitSnapshot<E>,
    q: &'a Query,
    schemas: &'a [Option<Schema>],
    /// The `WHERE` terms to check once binding `i` is bound, by `i`.
    terms: Vec<Vec<&'a Expr>>,
    /// Rows after which nothing more is needed.
    stop: Option<usize>,
}

impl<E: Engine> Run<'_, E> {
    fn done(&self, rows: &[Vec<Loaded>]) -> bool {
        self.stop.is_some_and(|n| rows.len() >= n)
    }

    /// Whether the terms of `level` hold on `row`, which has `level + 1` units.
    fn holds(&self, row: &[Loaded], level: usize) -> bool {
        let get: FieldOf<'_> = &|r: &FieldRef| self.q.position(&r.alias).and_then(|p| row[p].doc.get(&r.field));
        self.terms[level].iter().all(|t| t.eval(get))
    }

    /// Binds the joins after `row.len()` bindings, pushing every full row.
    fn extend(&self, row: &mut Vec<Loaded>, out: &mut Vec<Vec<Loaded>>) -> Result<(), UnitError> {
        let level = row.len();
        let Some(join) = self.q.joins.get(level - 1) else {
            out.push(row.clone());
            return Ok(());
        };
        let from = self.q.position(&join.from).map_or(0, |p| p.min(level - 1));
        let relations = self.snap.query_relations(&RelationQuery {
            unit: row[from].unit.id,
            direction: join.direction,
            relation_type: Some(join.relation_type.clone()),
        })?;
        // One row per pair of units, however many parallel relations join them, by unit id.
        let others: BTreeSet<UnitId> =
            relations.relations.iter().map(|r| if join.direction == Direction::Out { r.to } else { r.from }).collect();
        for id in others {
            if self.done(out) {
                break;
            }
            let Some(unit) = self.snap.get_unit(id)? else { continue };
            if unit.lifecycle != Lifecycle::Active || unit.unit_type != join.binding.unit_type {
                continue;
            }
            let doc = document_of(&unit, self.schemas[level].as_ref())?;
            row.push(Loaded { unit, doc });
            if self.holds(row, level) {
                self.extend(row, out)?;
            }
            row.pop();
        }
        Ok(())
    }
}

/// Orders full rows (by the `ORDER BY` keys, then by unit ids) and pages them.
pub(super) fn finish(q: &Query, mut rows: Vec<Vec<Loaded>>) -> Vec<Vec<Unit>> {
    let ids = |r: &[Loaded]| r.iter().map(|l| l.unit.id).collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        let by_keys = q.order.iter().fold(Ordering::Equal, |acc, k| {
            acc.then_with(|| {
                let pos = q.position(&k.field.alias).unwrap_or(0);
                let ord = compare_opt(a[pos].doc.get(&k.field.field), b[pos].doc.get(&k.field.field));
                if k.descending { ord.reverse() } else { ord }
            })
        });
        by_keys.then_with(|| ids(a).cmp(&ids(b)))
    });
    rows.into_iter()
        .skip(q.offset)
        .take(q.limit.unwrap_or(usize::MAX))
        .map(|r| r.into_iter().map(|l| l.unit).collect())
        .collect()
}
