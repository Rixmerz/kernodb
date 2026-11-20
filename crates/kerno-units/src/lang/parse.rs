//! The lexer and the recursive-descent parser of the query language.

use std::fmt;

use super::{Binding, CmpOp, Expr, FieldRef, Join, OrderKey, Query, is_keyword};
use crate::schema::FieldValue;
use crate::types::Direction;

/// Bytes of a query text.
pub const MAX_QUERY_LEN: usize = 16 * 1024;
/// Conditions (comparisons and `IS NULL` tests) in a `WHERE`.
pub const MAX_CONDITIONS: usize = 256;
/// Depth of parentheses and `NOT` in a `WHERE`.
pub const MAX_DEPTH: usize = 64;

/// A text that is not a query: the byte it stopped at and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.pos, self.message)
    }
}

impl std::error::Error for ParseError {}

impl From<ParseError> for crate::error::UnitError {
    fn from(e: ParseError) -> Self {
        crate::error::UnitError::InvalidArgument(format!("query {e}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Bytes(Vec<u8>),
    Int(i64),
    Float(f64),
    LParen,
    RParen,
    Comma,
    Dot,
    Op(CmpOp),
    End,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("{w:?}"),
            Tok::Quoted(_) => "a string".into(),
            Tok::Bytes(_) => "a bytes literal".into(),
            Tok::Int(_) | Tok::Float(_) => "a number".into(),
            Tok::LParen => "\"(\"".into(),
            Tok::RParen => "\")\"".into(),
            Tok::Comma => "\",\"".into(),
            Tok::Dot => "\".\"".into(),
            Tok::Op(op) => format!("{:?}", op.symbol()),
            Tok::End => "the end of the query".into(),
        }
    }
}

fn err<T>(pos: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError { pos, message: message.into() })
}

// -------------------------------------------------------------------- lexer

fn lex(src: &str) -> Result<Vec<(usize, Tok)>, ParseError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let tok = match c {
            b'"' => {
                let (s, next) = lex_string(src, i)?;
                i = next;
                Tok::Quoted(s)
            }
            b'x' | b'X' if bytes.get(i + 1) == Some(&b'"') => {
                let (s, next) = lex_string(src, i + 1)?;
                i = next;
                Tok::Bytes(unhex(&s).ok_or(ParseError {
                    pos: start,
                    message: "a bytes literal is an even number of hex digits".into(),
                })?)
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while bytes.get(i).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
                    i += 1;
                }
                Tok::Word(src[start..i].to_owned())
            }
            c if c.is_ascii_digit() || (c == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) => {
                let (t, next) = lex_number(src, i)?;
                i = next;
                t
            }
            b'(' | b')' | b',' | b'.' => {
                i += 1;
                match c {
                    b'(' => Tok::LParen,
                    b')' => Tok::RParen,
                    b',' => Tok::Comma,
                    _ => Tok::Dot,
                }
            }
            b'=' => {
                i += 1;
                Tok::Op(CmpOp::Eq)
            }
            b'!' if bytes.get(i + 1) == Some(&b'=') => {
                i += 2;
                Tok::Op(CmpOp::Ne)
            }
            b'<' | b'>' => {
                let or_equal = bytes.get(i + 1) == Some(&b'=');
                i += 1 + usize::from(or_equal);
                Tok::Op(match (c, or_equal) {
                    (b'<', false) => CmpOp::Lt,
                    (b'<', true) => CmpOp::Le,
                    (_, false) => CmpOp::Gt,
                    (_, true) => CmpOp::Ge,
                })
            }
            _ => {
                let ch = src[i..].chars().next().unwrap_or('?');
                return err(i, format!("unexpected character {ch:?}"));
            }
        };
        out.push((start, tok));
    }
    out.push((src.len(), Tok::End));
    Ok(out)
}

/// The string literal that opens at `at` (a `"`), and the byte after it.
fn lex_string(src: &str, at: usize) -> Result<(String, usize), ParseError> {
    let mut out = String::new();
    let mut chars = src[at + 1..].char_indices();
    while let Some((off, c)) = chars.next() {
        match c {
            '"' => return Ok((out, at + 1 + off + 1)),
            '\\' => {
                let Some((_, e)) = chars.next() else { break };
                out.push(match e {
                    '\\' => '\\',
                    '"' => '"',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    other => return err(at + 1 + off, format!("unknown escape \\{other}")),
                });
            }
            c => out.push(c),
        }
    }
    err(at, "a string is not closed")
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    s.as_bytes().chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok()).collect()
}

fn lex_number(src: &str, at: usize) -> Result<(Tok, usize), ParseError> {
    let bytes = src.as_bytes();
    let digits = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        i
    };
    let mut i = at + usize::from(bytes[at] == b'-');
    i = digits(i);
    let mut float = false;
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        float = true;
        i = digits(i + 1);
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(i + 1), Some(b'+' | b'-')));
        if bytes.get(i + 1 + sign).is_some_and(u8::is_ascii_digit) {
            float = true;
            i = digits(i + 1 + sign);
        }
    }
    if bytes.get(i).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
        return err(i, "a number is followed by letters");
    }
    let text = &src[at..i];
    if float {
        match text.parse::<f64>() {
            Ok(x) if x.is_finite() => Ok((Tok::Float(x), i)),
            _ => err(at, "a float must be finite"),
        }
    } else {
        text.parse::<i64>().map(|n| (Tok::Int(n), i)).or_else(|_| err(at, "an integer does not fit in 64 bits"))
    }
}

// ------------------------------------------------------------------- parser

struct Parser {
    toks: Vec<(usize, Tok)>,
    at: usize,
    aliases: Vec<String>,
    conditions: usize,
}

pub(super) fn parse(text: &str) -> Result<Query, ParseError> {
    if text.len() > MAX_QUERY_LEN {
        return err(MAX_QUERY_LEN, format!("a query is at most {MAX_QUERY_LEN} bytes"));
    }
    let mut p = Parser { toks: lex(text)?, at: 0, aliases: Vec::new(), conditions: 0 };
    p.query()
}

impl Parser {
    /// The token under the cursor; the end of the query once the cursor is past the last one.
    fn peek(&self) -> &Tok {
        self.toks.get(self.at).map_or(&Tok::End, |t| &t.1)
    }

    fn pos(&self) -> usize {
        self.toks.get(self.at).or(self.toks.last()).map_or(0, |t| t.0)
    }

    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        self.at += 1;
        t
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Word(w) if w.eq_ignore_ascii_case(kw))
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        let hit = self.is_kw(kw);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect_kw(&mut self, kw: &str) -> Result<(), ParseError> {
        if self.eat_kw(kw) { Ok(()) } else { self.unexpected(kw) }
    }

    fn unexpected<T>(&self, wanted: &str) -> Result<T, ParseError> {
        err(self.pos(), format!("expected {wanted}, found {}", self.peek().describe()))
    }

    /// A word that is not a keyword, or a string.
    fn name(&mut self, what: &str) -> Result<String, ParseError> {
        let at = self.pos();
        match self.peek() {
            Tok::Word(w) if is_keyword(w) => {
                err(at, format!("{w:?} is a keyword: write it as a string to use it as {what}"))
            }
            Tok::Word(_) | Tok::Quoted(_) => match self.bump() {
                Tok::Word(s) | Tok::Quoted(s) => Ok(s),
                _ => self.unexpected(what),
            },
            _ => self.unexpected(what),
        }
    }

    fn binding(&mut self) -> Result<Binding, ParseError> {
        let mut at = self.pos();
        let unit_type = self.name("a unit type")?;
        let alias = if self.eat_kw("AS") {
            at = self.pos();
            self.name("an alias")?
        } else {
            unit_type.clone()
        };
        if self.aliases.contains(&alias) {
            return err(at, format!("the alias {alias:?} is used twice"));
        }
        self.aliases.push(alias.clone());
        Ok(Binding { unit_type, alias })
    }

    fn count(&mut self) -> Result<usize, ParseError> {
        let at = self.pos();
        match self.bump() {
            Tok::Int(n) if n >= 0 => Ok(usize::try_from(n).unwrap_or(usize::MAX)),
            _ => err(at, "expected a number that is not negative"),
        }
    }

    fn query(&mut self) -> Result<Query, ParseError> {
        self.expect_kw("FIND")?;
        let root = self.binding()?;
        let mut joins = Vec::new();
        while self.eat_kw("JOIN") {
            let previous = self.aliases.last().cloned().unwrap_or_default();
            let binding = self.binding()?;
            self.expect_kw("VIA")?;
            let relation_type = self.name("a relation type")?;
            let direction = if self.eat_kw("IN") {
                Direction::In
            } else {
                self.eat_kw("OUT");
                Direction::Out
            };
            let from = if self.eat_kw("FROM") {
                let at = self.pos();
                let from = self.name("an alias")?;
                // The joined binding is already in `aliases`: it cannot be its own source.
                let earlier = &self.aliases[..self.aliases.len() - 1];
                if !earlier.contains(&from) {
                    return err(at, format!("FROM {from:?} is not an earlier alias"));
                }
                from
            } else {
                previous
            };
            joins.push(Join { binding, relation_type, direction, from });
        }
        let filter = if self.eat_kw("WHERE") { Some(self.expr(0)?) } else { None };
        let mut order = Vec::new();
        if self.eat_kw("ORDER") {
            self.expect_kw("BY")?;
            loop {
                let field = self.field_ref()?;
                let descending = if self.eat_kw("DESC") {
                    true
                } else {
                    self.eat_kw("ASC");
                    false
                };
                order.push(OrderKey { field, descending });
                if !matches!(self.peek(), Tok::Comma) {
                    break;
                }
                self.bump();
            }
        }
        let limit = if self.eat_kw("LIMIT") { Some(self.count()?) } else { None };
        let offset = if self.eat_kw("OFFSET") { self.count()? } else { 0 };
        if !matches!(self.peek(), Tok::End) {
            return self.unexpected("the end of the query");
        }
        Ok(Query { root, joins, filter, order, limit, offset })
    }

    /// `or`: the loosest level; `depth` counts parentheses and `NOT` around it.
    fn expr(&mut self, depth: usize) -> Result<Expr, ParseError> {
        let mut terms = vec![self.and(depth)?];
        while self.eat_kw("OR") {
            terms.push(self.and(depth)?);
        }
        Ok(collapse(terms, Expr::Or))
    }

    fn and(&mut self, depth: usize) -> Result<Expr, ParseError> {
        let mut terms = vec![self.not(depth)?];
        while self.eat_kw("AND") {
            terms.push(self.not(depth)?);
        }
        Ok(collapse(terms, Expr::And))
    }

    fn not(&mut self, depth: usize) -> Result<Expr, ParseError> {
        if self.is_kw("NOT") {
            let at = self.pos();
            self.bump();
            if depth + 1 > MAX_DEPTH {
                return err(at, format!("conditions nest at most {MAX_DEPTH} deep"));
            }
            return Ok(Expr::Not(Box::new(self.not(depth + 1)?)));
        }
        self.atom(depth)
    }

    fn atom(&mut self, depth: usize) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Tok::LParen) {
            let at = self.pos();
            self.bump();
            if depth + 1 > MAX_DEPTH {
                return err(at, format!("conditions nest at most {MAX_DEPTH} deep"));
            }
            let inner = self.expr(depth + 1)?;
            if !matches!(self.peek(), Tok::RParen) {
                return self.unexpected("\")\"");
            }
            self.bump();
            return Ok(inner);
        }
        let at = self.pos();
        self.conditions += 1;
        if self.conditions > MAX_CONDITIONS {
            return err(at, format!("a query has at most {MAX_CONDITIONS} conditions"));
        }
        let field = self.field_ref()?;
        if self.eat_kw("IS") {
            let negated = self.eat_kw("NOT");
            self.expect_kw("NULL")?;
            return Ok(Expr::IsNull { field, negated });
        }
        let Tok::Op(op) = *self.peek() else {
            return self.unexpected("a comparison operator or IS");
        };
        self.bump();
        Ok(Expr::Cmp { field, op, value: self.literal()? })
    }

    fn literal(&mut self) -> Result<FieldValue, ParseError> {
        let at = self.pos();
        Ok(match self.bump() {
            Tok::Int(n) => FieldValue::Int(n),
            Tok::Float(x) => FieldValue::Float(x),
            Tok::Quoted(s) => FieldValue::Text(s),
            Tok::Bytes(b) => FieldValue::Bytes(b),
            Tok::Word(w) if w.eq_ignore_ascii_case("true") => FieldValue::Bool(true),
            Tok::Word(w) if w.eq_ignore_ascii_case("false") => FieldValue::Bool(false),
            _ => return err(at, "expected a literal: a number, a string, x\"..\", true or false"),
        })
    }

    /// `name` (a field of the first binding) or `alias.name`.
    fn field_ref(&mut self) -> Result<FieldRef, ParseError> {
        let at = self.pos();
        let first = self.name("a field")?;
        if !matches!(self.peek(), Tok::Dot) {
            let root = self.aliases.first().cloned().unwrap_or_default();
            return Ok(FieldRef { alias: root, field: first });
        }
        self.bump();
        if !self.aliases.contains(&first) {
            return err(at, format!("{first:?} is not an alias of this query"));
        }
        Ok(FieldRef { alias: first, field: self.name("a field")? })
    }
}

fn collapse(mut terms: Vec<Expr>, make: fn(Vec<Expr>) -> Expr) -> Expr {
    if terms.len() == 1 { terms.remove(0) } else { make(terms) }
}
