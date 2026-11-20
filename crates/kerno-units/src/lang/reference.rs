//! The reference evaluator: the definition of what a query returns.

use kerno_core::contract::Engine;

use super::exec::{Loaded, finish};
use super::{FieldOf, FieldRef, Query, document_of};
use crate::db::{Unit, UnitSnapshot};
use crate::error::UnitError;
use crate::index::UnitFilter;
use crate::types::{Direction, UnitId};

impl<E: Engine> UnitSnapshot<E> {
    /// The answer of [`execute_query`](Self::execute_query), by the definition
    /// of the language and nothing else: every `Active` unit of each binding's
    /// type and every relation of each join's type are read, every combination
    /// of one unit per binding is formed, and the ones whose joined pairs are
    /// related and whose `WHERE` holds are kept, then ordered and paged. No
    /// index, no pruning, no early stop: cost is the product of the bindings.
    ///
    /// # Errors
    ///
    /// As [`explain_query`](Self::explain_query).
    pub fn reference_execute_query(&self, q: &Query) -> Result<Vec<Vec<Unit>>, UnitError> {
        let schemas = self.bind_query(q)?;
        let active = self.reference_filter(&UnitFilter::default())?;
        let mut columns: Vec<Vec<Loaded>> = Vec::new();
        for (binding, schema) in q.bindings().zip(&schemas) {
            let mut column = Vec::new();
            for unit in active.iter().filter(|u| u.unit_type == binding.unit_type) {
                column.push(Loaded { unit: unit.clone(), doc: document_of(unit, schema.as_ref())? });
            }
            columns.push(column);
        }
        let mut pairs: Vec<Vec<(UnitId, UnitId)>> = Vec::new();
        for join in &q.joins {
            let related = self.reference_relations(|r| r.relation_type == join.relation_type)?;
            pairs.push(related.iter().map(|r| (r.from, r.to)).collect());
        }
        let mut rows = Vec::new();
        let mut row: Vec<&Loaded> = Vec::new();
        product(q, &columns, &pairs, &mut row, &mut rows);
        Ok(finish(q, rows))
    }
}

fn product<'a>(
    q: &Query,
    columns: &'a [Vec<Loaded>],
    pairs: &[Vec<(UnitId, UnitId)>],
    row: &mut Vec<&'a Loaded>,
    out: &mut Vec<Vec<Loaded>>,
) {
    let level = row.len();
    if level == columns.len() {
        let get: FieldOf<'_> = &|r: &FieldRef| q.position(&r.alias).and_then(|p| row[p].doc.get(&r.field));
        if q.filter.as_ref().is_none_or(|f| f.eval(get)) {
            out.push(row.iter().map(|l| (*l).clone()).collect());
        }
        return;
    }
    for candidate in &columns[level] {
        let joined = level == 0 || {
            let join = &q.joins[level - 1];
            let from = q.position(&join.from).map_or(0, |p| p.min(level - 1));
            let (left, right) = (row[from].unit.id, candidate.unit.id);
            let pair = if join.direction == Direction::Out { (left, right) } else { (right, left) };
            pairs[level - 1].contains(&pair)
        };
        if joined {
            row.push(candidate);
            product(q, columns, pairs, row, out);
            row.pop();
        }
    }
}
