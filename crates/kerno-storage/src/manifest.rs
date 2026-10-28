//! The MANIFEST: a single checksummed snapshot of the durable LSM state,
//! replaced atomically (write `MANIFEST.tmp`, `sync_data`, `rename`,
//! `sync_dir`) on every flush and compaction.
//!
//! ```text
//! magic "KRNOMAN1" | body_len u32 | crc u32 (crc32c of body) | body
//! body: format u32 | last_seq u64 | log_number u64 | next_file u64 | levels u32
//!       per level: count u32, per file: number u64 | size u64 | entries u64 | smallest bytes | largest bytes
//! ```
//!
//! `last_seq` is the last commit contained in the SSTs; WAL files numbered
//! `>= log_number` hold the commits after it. A MANIFEST that exists but does
//! not verify is `Error::Corruption` (never "start empty").

use std::collections::HashSet;
use std::path::Path;

use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Error, Result};

use crate::coding::{Reader, put_bytes, put_u32, put_u64, u32_at};

pub const MANIFEST: &str = "MANIFEST";
pub const MANIFEST_TMP: &str = "MANIFEST.tmp";
const MAGIC: &[u8; 8] = b"KRNOMAN1";
const FORMAT: u32 = 1;
const HEADER: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMeta {
    pub number: u64,
    pub size: u64,
    pub entries: u64,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestData {
    pub last_seq: u64,
    pub log_number: u64,
    pub next_file: u64,
    /// L0 newest first; L1+ sorted by key and non-overlapping.
    pub levels: Vec<Vec<FileMeta>>,
}

impl ManifestData {
    pub fn empty(num_levels: usize) -> Self {
        ManifestData { last_seq: 0, log_number: 0, next_file: 1, levels: vec![Vec::new(); num_levels] }
    }
}

pub fn encode(m: &ManifestData) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, FORMAT);
    put_u64(&mut body, m.last_seq);
    put_u64(&mut body, m.log_number);
    put_u64(&mut body, m.next_file);
    put_u32(&mut body, m.levels.len() as u32);
    for level in &m.levels {
        put_u32(&mut body, level.len() as u32);
        for f in level {
            put_u64(&mut body, f.number);
            put_u64(&mut body, f.size);
            put_u64(&mut body, f.entries);
            put_bytes(&mut body, &f.smallest);
            put_bytes(&mut body, &f.largest);
        }
    }
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, body.len() as u32);
    put_u32(&mut out, crc32c::crc32c(&body));
    out.extend_from_slice(&body);
    out
}

pub fn decode(buf: &[u8], num_levels: usize) -> Result<ManifestData> {
    let corrupt = |m: &str| Error::Corruption(format!("{MANIFEST}: {m}"));
    if buf.len() < HEADER || &buf[..8] != MAGIC {
        return Err(corrupt("bad header"));
    }
    let body_len = u32_at(buf, 8).unwrap_or(0) as usize;
    let crc = u32_at(buf, 12).unwrap_or(0);
    let body = &buf[HEADER..];
    if body.len() != body_len {
        return Err(corrupt("length mismatch"));
    }
    if crc32c::crc32c(body) != crc {
        return Err(corrupt("checksum mismatch"));
    }
    let malformed = || corrupt("malformed body");
    let mut r = Reader::new(body);
    if r.u32().ok_or_else(malformed)? != FORMAT {
        return Err(corrupt("unknown format version"));
    }
    let last_seq = r.u64().ok_or_else(malformed)?;
    let log_number = r.u64().ok_or_else(malformed)?;
    let next_file = r.u64().ok_or_else(malformed)?;
    if r.u32().ok_or_else(malformed)? as usize != num_levels {
        return Err(corrupt("unexpected number of levels"));
    }
    let mut levels = Vec::with_capacity(num_levels);
    for _ in 0..num_levels {
        let n = r.u32().ok_or_else(malformed)? as usize;
        let mut files = Vec::with_capacity(n.min(body.len()));
        for _ in 0..n {
            files.push(FileMeta {
                number: r.u64().ok_or_else(malformed)?,
                size: r.u64().ok_or_else(malformed)?,
                entries: r.u64().ok_or_else(malformed)?,
                smallest: r.bytes().ok_or_else(malformed)?.to_vec(),
                largest: r.bytes().ok_or_else(malformed)?.to_vec(),
            });
        }
        levels.push(files);
    }
    if !r.is_empty() {
        return Err(malformed());
    }
    let m = ManifestData { last_seq, log_number, next_file, levels };
    validate(&m).map_err(corrupt)?;
    Ok(m)
}

/// Structural invariants (defence in depth behind the checksum).
fn validate(m: &ManifestData) -> std::result::Result<(), &'static str> {
    let mut numbers = HashSet::new();
    for (l, level) in m.levels.iter().enumerate() {
        for (i, f) in level.iter().enumerate() {
            if !numbers.insert(f.number) || f.number >= m.next_file {
                return Err("bad file number");
            }
            if f.smallest > f.largest || f.entries == 0 {
                return Err("bad file key range");
            }
            if l > 0 && i > 0 && level[i - 1].largest >= f.smallest {
                return Err("overlapping files in a sorted level");
            }
        }
    }
    if m.log_number > m.next_file {
        return Err("log number beyond next file number");
    }
    Ok(())
}

pub fn read(vfs: &dyn Vfs, dir: &Path, num_levels: usize) -> Result<ManifestData> {
    let f = vfs.open(&dir.join(MANIFEST), OpenMode::Existing)?;
    let len = f.len()?;
    let mut buf = vec![0u8; len as usize];
    f.read_at(&mut buf, 0)?;
    decode(&buf, num_levels)
}

/// Atomically replaces the MANIFEST; durable when this returns `Ok`.
pub fn write(vfs: &dyn Vfs, dir: &Path, m: &ManifestData) -> Result<()> {
    let tmp = dir.join(MANIFEST_TMP);
    if vfs.exists(&tmp)? {
        vfs.remove_file(&tmp)?;
    }
    let f = vfs.open(&tmp, OpenMode::CreateNew)?;
    f.write_at(&encode(m), 0)?;
    f.sync_data()?;
    vfs.rename(&tmp, &dir.join(MANIFEST))?;
    vfs.sync_dir(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_testkit::vfs::SimVfs;

    fn sample() -> ManifestData {
        let f = |n: u64, a: &str, b: &str| FileMeta {
            number: n,
            size: 100 * n,
            entries: n,
            smallest: a.as_bytes().to_vec(),
            largest: b.as_bytes().to_vec(),
        };
        let mut m = ManifestData::empty(4);
        m.last_seq = 42;
        m.log_number = 9;
        m.next_file = 10;
        m.levels[0] = vec![f(8, "a", "z"), f(5, "c", "d")];
        m.levels[1] = vec![f(3, "a", "b"), f(4, "c", "c"), f(6, "d", "k")];
        m.levels[3] = vec![f(1, "", "zzz")];
        m
    }

    #[test]
    fn roundtrip_through_the_vfs() {
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        let dir = Path::new("/m");
        vfs.create_dir_all(dir).unwrap();
        let m = sample();
        write(&*vfs, dir, &m).unwrap();
        assert_eq!(read(&*vfs, dir, 4).unwrap(), m);
        let mut m2 = m.clone();
        m2.last_seq = 43;
        write(&*vfs, dir, &m2).unwrap();
        assert_eq!(read(&*vfs, dir, 4).unwrap(), m2);
        assert!(!vfs.exists(&dir.join(MANIFEST_TMP)).unwrap());
    }

    #[test]
    fn every_bit_flip_and_truncation_is_detected() {
        let buf = encode(&sample());
        for byte in 0..buf.len() {
            for bit in 0..8 {
                let mut bad = buf.clone();
                bad[byte] ^= 1 << bit;
                assert!(matches!(decode(&bad, 4), Err(Error::Corruption(_))), "flip {byte}/{bit}");
            }
        }
        for len in 0..buf.len() {
            assert!(matches!(decode(&buf[..len], 4), Err(Error::Corruption(_))), "truncated to {len}");
        }
        assert!(decode(&buf, 5).is_err());
    }

    #[test]
    fn invariants_are_checked() {
        let mut m = sample();
        m.levels[1][1].smallest = b"b".to_vec(); // overlaps the previous file
        assert!(decode(&encode(&m), 4).is_err());
        let mut m = sample();
        m.levels[3][0].number = 8; // duplicate
        assert!(decode(&encode(&m), 4).is_err());
        let mut m = sample();
        m.next_file = 8;
        assert!(decode(&encode(&m), 4).is_err());
    }
}
