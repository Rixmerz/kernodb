//! The catalog: one unit that remembers what the public API cannot be asked.
//!
//! Text format, one record per line (the unit's payload):
//!
//! ```text
//! next <n>            the next free memory number
//! sum <n>,<n>,…       the sources of a summary
//! ```
//!
//! The previous version of this example also wrote `mem <n> <vec|->` for every
//! memory. Memories are now listed with `DerivedDb::query` and their vectors
//! found with `DerivedDb::derived_of`, so those lines carry nothing. [`Catalog::parse`]
//! still accepts them and drops them, so a directory written by the previous
//! version opens; the next write of the catalog leaves them out.
//!
//! ponytail: the whole catalog is rewritten on every write; it now holds only
//! the counter and the summary definitions, so it stays small. Prune its old
//! revisions with `AgentMemory::prune_history`.

use std::fmt::Write as _;

use crate::{Error, Result};

#[derive(Debug, Default)]
pub(crate) struct Catalog {
    pub next: u64,
    pub summaries: Vec<Vec<u64>>,
}

fn corrupt(line: &str) -> Error {
    Error::Corrupt(format!("cannot read the line starting {:?}", line.split(' ').next().unwrap_or("")))
}

fn num(s: &str, line: &str) -> Result<u64> {
    s.parse().map_err(|_| corrupt(line))
}

impl Catalog {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = format!("next {}\n", self.next);
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
                // Legacy line of the previous version: validated, then dropped.
                ["mem", n, vector] => {
                    num(n, line)?;
                    if *vector != "-" && (vector.len() != 64 || !vector.bytes().all(|b| b.is_ascii_hexdigit())) {
                        return Err(corrupt(line));
                    }
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
        c.summaries.push(vec![2, 3]);
        let back = Catalog::parse(&c.encode()).unwrap();
        assert_eq!((back.next, back.summaries), (9, c.summaries));
        assert!(Catalog::parse(b"mem x -\nnext 1\n").is_err());
        assert!(Catalog::parse(b"next 1\nmem 2 abcd\n").is_err());
    }

    #[test]
    fn legacy_mem_lines_are_accepted_and_dropped() {
        let hex = "ab".repeat(32);
        let legacy = format!("next 7\nmem 2 -\nmem 3 {hex}\nsum 2,3\n");
        let cat = Catalog::parse(legacy.as_bytes()).unwrap();
        assert_eq!((cat.next, cat.summaries.clone()), (7, vec![vec![2, 3]]));
        // Writing it again leaves the legacy lines out.
        let text = String::from_utf8(cat.encode()).unwrap();
        assert_eq!(text, "next 7\nsum 2,3\n");
    }
}
