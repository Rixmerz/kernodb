//! A declarative query language over units and their relations (Phase C).
//!
//! The grammar and the meaning of every construct are in
//! `docs/query-language.md`. In short: `FIND type [AS a] {JOIN type [AS b] VIA
//! relation [IN|OUT] [FROM x]} [WHERE ..] [ORDER BY ..] [LIMIT n] [OFFSET m]`
//! over the typed [`Document`] payloads of units whose type has a [`Schema`].
//!
//! * [`Query::parse`] reads a text into a [`Query`], which prints back to a
//!   canonical text ([`Display`](fmt::Display)) that parses to an equal query.
//! * [`UnitSnapshot::explain_query`] shows the [`QueryPlan`]: the access paths
//!   of the first binding with their costs, the one chosen, the plan the
//!   existing planner makes for a scan of its type, and the joins.
//!   [`UnitSnapshot::execute_query`] runs that very plan, on one snapshot.
//! * [`UnitSnapshot::reference_execute_query`] is the definition of the answer,
//!   with no index and no pruning; the differential tests hold the planned path
//!   to it.

mod exec;
mod parse;
mod reference;
#[cfg(test)]
mod tests;

use std::cmp::Ordering;
use std::fmt;

use crate::db::Unit;
use crate::error::UnitError;
use crate::plan::{Cost, Plan};
use crate::schema::{Document, FieldValue, Schema};
use crate::types::Direction;

pub use parse::{MAX_CONDITIONS, MAX_DEPTH, MAX_QUERY_LEN, ParseError};

/// Reads a field of one unit of a row; `None` when the unit does not have it.
type FieldOf<'a> = &'a dyn Fn(&FieldRef) -> Option<&'a FieldValue>;

// ---------------------------------------------------------------------- AST

/// A unit type and the name its units go by in the query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub unit_type: String,
    pub alias: String,
}

/// `JOIN binding VIA relation_type [IN|OUT] [FROM from]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub binding: Binding,
    pub relation_type: String,
    /// `Out`: a relation goes from the unit of `from` to the joined unit.
    /// `In`: from the joined unit to the unit of `from`.
    pub direction: Direction,
    /// The alias of an earlier binding.
    pub from: String,
}

/// A field of one binding: `alias.field`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRef {
    pub alias: String,
    pub field: String,
}

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    /// Whether `ord` (of the field against the literal) satisfies the operator.
    fn holds(self, ord: Ordering) -> bool {
        match self {
            CmpOp::Eq => ord == Ordering::Equal,
            CmpOp::Ne => ord != Ordering::Equal,
            CmpOp::Lt => ord == Ordering::Less,
            CmpOp::Le => ord != Ordering::Greater,
            CmpOp::Gt => ord == Ordering::Greater,
            CmpOp::Ge => ord != Ordering::Less,
        }
    }
}

/// A condition. `And` and `Or` hold at least two terms.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Cmp {
        field: FieldRef,
        op: CmpOp,
        value: FieldValue,
    },
    /// `field IS NULL` (`negated: false`) or `field IS NOT NULL`.
    IsNull {
        field: FieldRef,
        negated: bool,
    },
}

/// One `ORDER BY` key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderKey {
    pub field: FieldRef,
    pub descending: bool,
}

/// A parsed query. Aliases are always explicit and every reference is resolved
/// to an alias the query declares ([`Query::parse`] guarantees it; a hand-built
/// query that breaks it is refused by `explain_query` and `execute_query`).
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub root: Binding,
    pub joins: Vec<Join>,
    pub filter: Option<Expr>,
    pub order: Vec<OrderKey>,
    pub limit: Option<usize>,
    pub offset: usize,
}

impl Query {
    /// Reads `text`; see `docs/query-language.md` for the grammar.
    ///
    /// # Errors
    ///
    /// [`ParseError`] with the byte where the text stopped making sense.
    pub fn parse(text: &str) -> Result<Query, ParseError> {
        parse::parse(text)
    }

    /// The bindings in row order: the first, then one per join.
    pub fn bindings(&self) -> impl Iterator<Item = &Binding> {
        std::iter::once(&self.root).chain(self.joins.iter().map(|j| &j.binding))
    }

    fn position(&self, alias: &str) -> Option<usize> {
        self.bindings().position(|b| b.alias == alias)
    }

    /// Every field the query reads, with the literal it is compared with.
    fn field_uses(&self) -> Vec<(&FieldRef, Option<&FieldValue>)> {
        let mut out = Vec::new();
        if let Some(f) = &self.filter {
            f.collect(&mut out);
        }
        out.extend(self.order.iter().map(|k| (&k.field, None)));
        out
    }
}

impl Expr {
    fn collect<'a>(&'a self, out: &mut Vec<(&'a FieldRef, Option<&'a FieldValue>)>) {
        match self {
            Expr::And(v) | Expr::Or(v) => v.iter().for_each(|e| e.collect(out)),
            Expr::Not(e) => e.collect(out),
            Expr::Cmp { field, value, .. } => out.push((field, Some(value))),
            Expr::IsNull { field, .. } => out.push((field, None)),
        }
    }

    /// Whether the condition holds. A comparison with a missing field is false.
    fn eval(&self, get: FieldOf<'_>) -> bool {
        match self {
            Expr::And(v) => v.iter().all(|e| e.eval(get)),
            Expr::Or(v) => v.iter().any(|e| e.eval(get)),
            Expr::Not(e) => !e.eval(get),
            Expr::Cmp { field, op, value } => {
                get(field).and_then(|f| compare(f, value)).is_some_and(|ord| op.holds(ord))
            }
            Expr::IsNull { field, negated } => get(field).is_none() == !negated,
        }
    }
}

/// The order of two values of one type; `None` for values of different types.
fn compare(a: &FieldValue, b: &FieldValue) -> Option<Ordering> {
    match (a, b) {
        (FieldValue::Bool(a), FieldValue::Bool(b)) => Some(a.cmp(b)),
        (FieldValue::Int(a), FieldValue::Int(b)) => Some(a.cmp(b)),
        (FieldValue::Float(a), FieldValue::Float(b)) => a.partial_cmp(b),
        (FieldValue::Text(a), FieldValue::Text(b)) => Some(a.as_bytes().cmp(b.as_bytes())),
        (FieldValue::Bytes(a), FieldValue::Bytes(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// The order of two possibly missing values: missing first.
fn compare_opt(a: Option<&FieldValue>, b: Option<&FieldValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => compare(a, b).unwrap_or(Ordering::Equal),
    }
}

// ------------------------------------------------------------------ results

/// How the first binding is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// Every `Active` unit of the type, through [`UnitSnapshot::query`](crate::UnitSnapshot::query).
    TypeScan,
    /// The range of the user-defined index on `field`.
    FieldIndex { field: String },
}

/// An access path and what it is expected to cost, in reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessCandidate {
    pub access: Access,
    pub cost: Cost,
}

/// One `JOIN`, as it will run: a nested loop with a planned relation lookup
/// per row so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinStep {
    pub alias: String,
    pub unit_type: String,
    pub relation_type: String,
    pub direction: Direction,
    pub from: String,
}

/// How a query will run and why; see [`UnitSnapshot::explain_query`](crate::UnitSnapshot::explain_query).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryPlan {
    pub root_alias: String,
    pub root_type: String,
    /// `TypeScan` first, then one `FieldIndex` per usable condition in the order written.
    pub candidates: Vec<AccessCandidate>,
    /// The cheapest candidate, the first on a tie.
    pub chosen: usize,
    /// What the existing planner does for a scan of the root type: its own
    /// candidates, costs, freshness and cache view.
    pub scan: Plan,
    pub joins: Vec<JoinStep>,
    /// The planner version the costs were made under.
    pub planner_version: u32,
}

impl QueryPlan {
    pub fn chosen(&self) -> &AccessCandidate {
        &self.candidates[self.chosen]
    }
}

/// The rows of a query.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResult {
    /// The alias of each column of a row, in order.
    pub aliases: Vec<String>,
    /// One unit per alias in each row.
    pub rows: Vec<Vec<Unit>>,
    /// The access path that read the first binding.
    pub access: Access,
    /// For `TypeScan`, the path the existing planner answered through.
    pub scan_path: Option<crate::PlanPath>,
}

/// The payload of a unit as a record: empty when it has none or its type has
/// no schema (no field of it is read then).
fn document_of(unit: &Unit, schema: Option<&Schema>) -> Result<Document, UnitError> {
    match (schema, unit.payload.as_deref()) {
        (Some(_), Some(p)) => Document::parse(p).map_err(|m| {
            crate::db::corruption(format!("unit {} of a type with a schema does not hold a record: {m}", unit.id))
        }),
        _ => Ok(Document::new()),
    }
}

// ------------------------------------------------------------------ display

const KEYWORDS: &[&str] = &[
    "FIND", "AS", "JOIN", "VIA", "IN", "OUT", "FROM", "WHERE", "ORDER", "BY", "ASC", "DESC", "LIMIT", "OFFSET", "AND",
    "OR", "NOT", "IS", "NULL", "TRUE", "FALSE",
];

pub(crate) fn is_keyword(word: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word))
}

fn write_string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '\\' => f.write_str("\\\\")?,
            '"' => f.write_str("\\\"")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            '\0' => f.write_str("\\0")?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

fn write_name(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    let mut chars = name.chars();
    let word = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if word && !is_keyword(name) { f.write_str(name) } else { write_string(f, name) }
}

fn write_value(f: &mut fmt::Formatter<'_>, v: &FieldValue) -> fmt::Result {
    match v {
        FieldValue::Bool(b) => write!(f, "{b}"),
        FieldValue::Int(i) => write!(f, "{i}"),
        FieldValue::Float(x) => write!(f, "{x:?}"),
        FieldValue::Text(s) => write_string(f, s),
        FieldValue::Bytes(b) => {
            f.write_str("x\"")?;
            b.iter().try_for_each(|byte| write!(f, "{byte:02x}"))?;
            f.write_str("\"")
        }
    }
}

impl fmt::Display for FieldRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_name(f, &self.alias)?;
        f.write_str(".")?;
        write_name(f, &self.field)
    }
}

impl Expr {
    /// `parent`: 0 at the top, 1 under `OR`, 2 under `AND`, 3 under `NOT`. An `AND`
    /// or `OR` is parenthesised under a parent that binds as tightly or tighter,
    /// so a nested term written in parentheses is written in them again.
    fn write(&self, f: &mut fmt::Formatter<'_>, parent: u8) -> fmt::Result {
        let own = match self {
            Expr::Or(_) => 1,
            Expr::And(_) => 2,
            Expr::Not(_) => 3,
            _ => 4,
        };
        let paren = own <= 2 && parent >= own;
        if paren {
            f.write_str("(")?;
        }
        match self {
            Expr::Or(v) | Expr::And(v) => {
                let word = if own == 1 { " OR " } else { " AND " };
                for (i, e) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(word)?;
                    }
                    e.write(f, own)?;
                }
            }
            Expr::Not(e) => {
                f.write_str("NOT ")?;
                e.write(f, 3)?;
            }
            Expr::Cmp { field, op, value } => {
                write!(f, "{field} {} ", op.symbol())?;
                write_value(f, value)?;
            }
            Expr::IsNull { field, negated } => write!(f, "{field} IS {}NULL", if *negated { "NOT " } else { "" })?,
        }
        if paren { f.write_str(")") } else { Ok(()) }
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let binding = |f: &mut fmt::Formatter<'_>, b: &Binding| -> fmt::Result {
            write_name(f, &b.unit_type)?;
            f.write_str(" AS ")?;
            write_name(f, &b.alias)
        };
        f.write_str("FIND ")?;
        binding(f, &self.root)?;
        for j in &self.joins {
            f.write_str(" JOIN ")?;
            binding(f, &j.binding)?;
            f.write_str(" VIA ")?;
            write_name(f, &j.relation_type)?;
            f.write_str(if j.direction == Direction::Out { " OUT FROM " } else { " IN FROM " })?;
            write_name(f, &j.from)?;
        }
        if let Some(e) = &self.filter {
            f.write_str(" WHERE ")?;
            e.write(f, 0)?;
        }
        for (i, k) in self.order.iter().enumerate() {
            f.write_str(if i == 0 { " ORDER BY " } else { ", " })?;
            write!(f, "{}{}", k.field, if k.descending { " DESC" } else { " ASC" })?;
        }
        if let Some(n) = self.limit {
            write!(f, " LIMIT {n}")?;
        }
        if self.offset > 0 {
            write!(f, " OFFSET {}", self.offset)?;
        }
        Ok(())
    }
}
