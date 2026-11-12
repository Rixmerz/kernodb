//! The catalog: one unit that remembers what the public API cannot be asked.
//!
//! Text format, one record per line (the unit's payload):
//!
//! ```text
//! next <n>            the next free memory number
//! mem <n> <vec|->     a memory and the hex derived id of its vector, or `-`
//! sum <n>,<n>,…       the sources of a summary
//! ```
//!
//! ponytail: the whole catalog is rewritten on every write and a unit payload
//! holds at most 1 MiB, so it fits about 13 000 memories; split it by range or
//! keep the ids in a unit per memory if an agent needs more.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kerno_units::DerivedId;

use crate::{Error, Result};

#[derive(Debug, Default)]
pub(crate) struct Catalog {
    pub next: u64,
    pub vectors: BTreeMap<u64, Option<DerivedId>>,
    pub summaries: Vec<Vec<u64>>,
}

fn corrupt(line: &str) -> Error {
    Error::Corrupt(format!("cannot read the line starting {:?}", line.split(' ').next().unwrap_or("")))
}

fn num(s: &str, line: &str) -> Result<u64> {
    s.parse().map_err(|_| corrupt(line))
}

fn parse_hex(s: &str, line: &str) -> Result<DerivedId> {
    if s.len() != 64 || !s.is_ascii() {
        return Err(corrupt(line));
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| corrupt(line))?;
    }
    Ok(DerivedId(out))
}

impl Catalog {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = format!("next {}\n", self.next);
        for (n, v) in &self.vectors {
            match v {
                None => writeln!(out, "mem {n} -"),
                Some(id) => {
                    let hex: String = id.0.iter().map(|b| format!("{b:02x}")).collect();
                    writeln!(out, "mem {n} {hex}")
                }
            }
            .expect("writing to a String cannot fail");
        }
        for s in &self.summaries {
            let list: Vec<String> = s.iter().map(u64::to_string).collect();
            writeln!(out, "sum {}", list.join(",")).expect("writing to a String cannot fail");
        }
        out.into_bytes()
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Corrupt("not UTF-8".into()))?;
        let mut cat = Catalog::default();
        for line in text.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts.as_slice() {
                ["next", n] => cat.next = num(n, line)?,
                ["mem", n, "-"] => {
                    cat.vectors.insert(num(n, line)?, None);
                }
                ["mem", n, hex] => {
                    cat.vectors.insert(num(n, line)?, Some(parse_hex(hex, line)?));
                }
                ["sum", list] => cat.summaries.push(list.split(',').map(|n| num(n, line)).collect::<Result<_>>()?),
                _ => return Err(corrupt(line)),
            }
        }
        if cat.next == 0 {
            return Err(Error::Corrupt("no `next` line".into()));
        }
        Ok(cat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Catalog { next: 9, ..Catalog::default() };
        c.vectors.insert(2, None);
        c.vectors.insert(3, Some(DerivedId([0xAB; 32])));
        c.summaries.push(vec![2, 3]);
        let back = Catalog::parse(&c.encode()).unwrap();
        assert_eq!((back.next, back.vectors, back.summaries), (9, c.vectors, c.summaries));
        assert!(Catalog::parse(b"mem x -\nnext 1\n").is_err());
    }
}
