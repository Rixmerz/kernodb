//! Write-ahead log: one file per memtable (`NNNNNN.wal`), never reused or
//! appended to after a restart.
//!
//! ```text
//! record: len u32 | crc u32 | seq u64 | batch
//!   len   = body length (< 2^31) | SYNCED (bit 31)
//!   crc   = crc32c(len | seq | batch)
//!   batch = count varint, then per op: tag u8 (1 put, 2 delete) | key bytes | [value bytes]
//! ```
//!
//! Each record is written with a single `write_at`. Replay stops at the
//! first record that is incomplete or fails its checksum; a record that
//! verifies but carries an unexpected sequence number or an undecodable
//! batch is corruption.
//!
//! **Torn tail or media damage?** A log is written by a single handle in a
//! single durability mode. `SYNCED` marks records written in `Sync` mode,
//! where a record is only written after the previous one is durable. So if
//! an invalid record is followed anywhere in the file by a valid `SYNCED`
//! record, the invalid one was durable before and has been damaged since:
//! that is reported as corruption instead of being dropped as a torn tail
//! (which would silently lose acknowledged commits). In `Buffered` mode,
//! writes are unordered and a valid record after an invalid one is a normal
//! power-loss outcome, so it is still treated as a torn tail.

use std::io;
use std::path::Path;
use std::sync::Arc;

use spike_core::vfs::{File, OpenMode, Vfs};
use spike_core::{Error, Op, Result};

use crate::coding::{Reader, put_bytes, put_varint, u32_at, u64_at};

pub const HEADER_LEN: usize = 16;
/// Bit of the length field marking a record written in `Sync` mode.
const SYNCED: u32 = 1 << 31;
/// Largest encodable batch.
pub const MAX_BODY_LEN: usize = (SYNCED - 1) as usize;
const TAG_PUT: u8 = 1;
const TAG_DELETE: u8 = 2;

pub fn wal_name(number: u64) -> String {
    format!("{number:06}.wal")
}

fn record_crc(header: &[u8], body: &[u8]) -> u32 {
    let c = crc32c::crc32c(&header[0..4]);
    let c = crc32c::crc32c_append(c, &header[8..16]);
    crc32c::crc32c_append(c, body)
}

/// Encodes one commit. Fails (before anything is written) if the encoded
/// batch does not fit the length field.
pub fn encode_record(seq: u64, ops: &[Op], synced: bool) -> Result<Vec<u8>> {
    let mut rec = vec![0u8; HEADER_LEN];
    put_varint(&mut rec, ops.len() as u64);
    for op in ops {
        match op {
            Op::Put { key, value } => {
                rec.push(TAG_PUT);
                put_bytes(&mut rec, key);
                put_bytes(&mut rec, value);
            }
            Op::Delete { key } => {
                rec.push(TAG_DELETE);
                put_bytes(&mut rec, key);
            }
        }
    }
    let body_len = rec.len() - HEADER_LEN;
    if body_len > MAX_BODY_LEN {
        return Err(Error::InvalidArgument(format!(
            "batch encodes to {body_len} bytes, more than the WAL limit of {MAX_BODY_LEN}"
        )));
    }
    let len = body_len as u32 | if synced { SYNCED } else { 0 };
    rec[0..4].copy_from_slice(&len.to_le_bytes());
    rec[8..16].copy_from_slice(&seq.to_le_bytes());
    let crc = record_crc(&rec[..HEADER_LEN], &rec[HEADER_LEN..]);
    rec[4..8].copy_from_slice(&crc.to_le_bytes());
    Ok(rec)
}

/// Looks for a valid `SYNCED` record starting after `bad` (the offset of an
/// invalid record) whose sequence number is plausible for this log. Every
/// byte offset is a candidate (the damage may be in a length field); the
/// cheap sequence-range test runs before the crc.
fn synced_record_after(buf: &[u8], bad: usize, expected: u64) -> Option<usize> {
    let max_records = (buf.len() - bad) as u64 / HEADER_LEN as u64 + 1;
    (bad + 1..buf.len().saturating_sub(HEADER_LEN - 1)).find(|&q| {
        let header = &buf[q..q + HEADER_LEN];
        let seq = u64_at(header, 8).unwrap_or(0);
        if seq < expected || seq > expected + max_records {
            return false;
        }
        let len = u32_at(header, 0).unwrap_or(0);
        let body_len = (len & !SYNCED) as usize;
        if len & SYNCED == 0 || body_len == 0 {
            return false;
        }
        match buf.get(q + HEADER_LEN..q + HEADER_LEN + body_len) {
            Some(body) => record_crc(header, body) == u32_at(header, 4).unwrap_or(0),
            None => false,
        }
    })
}

fn decode_batch(body: &[u8]) -> Option<Vec<Op>> {
    let mut r = Reader::new(body);
    let n = r.varint_usize()?;
    let mut ops = Vec::with_capacity(n.min(body.len()));
    for _ in 0..n {
        let op = match r.u8()? {
            TAG_PUT => Op::Put { key: r.bytes()?.to_vec(), value: r.bytes()?.to_vec() },
            TAG_DELETE => Op::Delete { key: r.bytes()?.to_vec() },
            _ => return None,
        };
        ops.push(op);
    }
    (r.is_empty() && n > 0).then_some(ops)
}

pub struct WalWriter {
    file: Arc<dyn File>,
    number: u64,
    offset: u64,
}

impl WalWriter {
    /// Creates a new, empty log file. The caller makes the directory entry
    /// durable (`sync_dir`) before acknowledging commits written to it.
    pub fn create(vfs: &dyn Vfs, dir: &Path, number: u64) -> Result<Self> {
        let file = vfs.open(&dir.join(wal_name(number)), OpenMode::CreateNew)?;
        Ok(WalWriter { file, number, offset: 0 })
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn append(&mut self, record: &[u8]) -> io::Result<()> {
        self.file.write_at(record, self.offset)?;
        self.offset += record.len() as u64;
        Ok(())
    }

    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_data()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Replay {
    /// Sequence of the last applied record (`first_seq - 1` if none).
    pub last_seq: u64,
    pub records: u64,
    /// Length of the valid prefix.
    pub valid_len: u64,
    /// Bytes follow the valid prefix (torn or garbage tail).
    pub torn: bool,
}

/// Replays `file`, expecting the first record to carry `first_seq` and each
/// following one the next number, calling `apply(seq, ops)` per record.
pub fn replay(file: &dyn File, name: &str, first_seq: u64, mut apply: impl FnMut(u64, &[Op])) -> Result<Replay> {
    let len = file.len()?;
    let mut buf = vec![0u8; len as usize];
    file.read_at(&mut buf, 0)?;
    let mut pos = 0usize;
    let mut expected = first_seq;
    let mut records = 0;
    while let Some(header) = buf.get(pos..pos + HEADER_LEN) {
        let body_len = (u32_at(header, 0).unwrap_or(0) & !SYNCED) as usize;
        let crc = u32_at(header, 4).unwrap_or(0);
        let seq = u64_at(header, 8).unwrap_or(0);
        let Some(body) = buf.get(pos + HEADER_LEN..pos + HEADER_LEN + body_len) else { break };
        if body_len == 0 || record_crc(header, body) != crc {
            break;
        }
        if seq != expected {
            return Err(Error::Corruption(format!(
                "{name}: record at offset {pos} has sequence {seq}, expected {expected}"
            )));
        }
        let ops =
            decode_batch(body).ok_or_else(|| Error::Corruption(format!("{name}: malformed batch at offset {pos}")))?;
        apply(seq, &ops);
        records += 1;
        expected += 1;
        pos += HEADER_LEN + body_len;
    }
    if pos < buf.len()
        && let Some(q) = synced_record_after(&buf, pos, expected)
    {
        return Err(Error::Corruption(format!(
            "{name}: invalid record at offset {pos} is followed by a valid record written in Sync mode \
             (offset {q}); the log was damaged after it was synced"
        )));
    }
    Ok(Replay { last_seq: expected - 1, records, valid_len: pos as u64, torn: pos < buf.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spike_core::vfs::{CrashKind, SimVfs};
    use spike_core::{Vfs, WriteBatch};

    fn batch(i: u64) -> WriteBatch {
        let mut b = WriteBatch::new();
        b.put(format!("k{i}"), vec![i as u8; (i as usize * 97) % 1500]);
        if i.is_multiple_of(3) {
            b.delete(format!("k{}", i - 1));
        }
        b
    }

    type Records = Vec<(u64, Vec<Op>)>;

    fn collect(vfs: &dyn Vfs, first: u64) -> Result<(Replay, Records)> {
        let f = vfs.open(Path::new("/w/000001.wal"), OpenMode::Existing)?;
        let mut got = Vec::new();
        let r = replay(&*f, "wal", first, |s, ops| got.push((s, ops.to_vec())))?;
        Ok((r, got))
    }

    #[test]
    fn roundtrip() {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1).unwrap();
        for i in 1..=20 {
            w.append(&encode_record(10 + i, batch(i).ops(), false).unwrap()).unwrap();
        }
        let (r, got) = collect(&*vfs, 11).unwrap();
        assert_eq!(r.records, 20);
        assert_eq!(r.last_seq, 30);
        assert!(!r.torn);
        for (i, (s, ops)) in got.iter().enumerate() {
            assert_eq!(*s, 11 + i as u64);
            assert_eq!(ops.as_slice(), batch(i as u64 + 1).ops());
        }
        // Wrong starting sequence is corruption, not a torn tail.
        assert!(matches!(collect(&*vfs, 12), Err(Error::Corruption(_))));
    }

    #[test]
    fn zeroed_header_is_not_a_record() {
        let header = [0u8; HEADER_LEN];
        assert_ne!(record_crc(&header, &[]), 0);
    }

    /// Unsynced appends + power loss: replay yields a prefix of the
    /// records and reports the rest as a torn tail.
    #[test]
    fn torn_tail_yields_a_prefix() {
        let mut torn_seen = 0;
        for seed in 0..200 {
            let sim = SimVfs::new(seed);
            let vfs = sim.as_vfs();
            vfs.create_dir_all(Path::new("/w")).unwrap();
            let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1).unwrap();
            for i in 1..=5 {
                w.append(&encode_record(i, batch(i).ops(), false).unwrap()).unwrap();
            }
            w.sync().unwrap();
            vfs.sync_dir(Path::new("/w")).unwrap();
            for i in 6..=12 {
                w.append(&encode_record(i, batch(i).ops(), false).unwrap()).unwrap();
            }
            let after = sim.crash_now(CrashKind::PowerLoss);
            let (r, got) = collect(&*after.as_vfs(), 1).unwrap();
            assert!(r.last_seq >= 5, "synced records lost");
            assert_eq!(got.len() as u64, r.last_seq);
            for (s, ops) in &got {
                assert_eq!(ops.as_slice(), batch(*s).ops());
            }
            if r.torn {
                torn_seen += 1;
            }
        }
        assert!(torn_seen > 0);
    }

    #[test]
    fn garbage_tail_after_valid_records() {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1).unwrap();
        w.append(&encode_record(1, batch(1).ops(), false).unwrap()).unwrap();
        let good = w.offset;
        let mut rec = encode_record(2, batch(2).ops(), false).unwrap();
        let n = rec.len();
        rec[n - 1] ^= 0x40;
        w.append(&rec).unwrap();
        w.append(&encode_record(3, batch(3).ops(), false).unwrap()).unwrap();
        let (r, got) = collect(&*vfs, 1).unwrap();
        assert_eq!((r.last_seq, r.valid_len, r.torn, got.len()), (1, good, true, 1));
        // Truncated header.
        let f = vfs.open(Path::new("/w/000001.wal"), OpenMode::Existing).unwrap();
        f.set_len(good + 7).unwrap();
        let (r, _) = collect(&*vfs, 1).unwrap();
        assert_eq!((r.last_seq, r.torn), (1, true));
    }

    /// In a Sync-mode log, an invalid record followed by a valid one cannot
    /// be a torn write: every record was durable before the next was
    /// written. Damage anywhere (body or length field) is corruption.
    #[test]
    fn damage_before_a_synced_record_is_corruption() {
        for damaged_byte in [0usize, 2, 9, 20, 40] {
            let sim = SimVfs::new(1);
            let vfs = sim.as_vfs();
            vfs.create_dir_all(Path::new("/w")).unwrap();
            let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1).unwrap();
            let mut offsets = Vec::new();
            for i in 1..=5 {
                offsets.push(w.offset);
                w.append(&encode_record(i, batch(i).ops(), true).unwrap()).unwrap();
                w.sync().unwrap();
            }
            vfs.sync_dir(Path::new("/w")).unwrap();
            sim.corrupt_byte(Path::new("/w/000001.wal"), offsets[2] + damaged_byte as u64, 0x10);
            assert!(
                matches!(collect(&*vfs, 1), Err(Error::Corruption(_))),
                "damage at byte {damaged_byte} of record 3 was taken for a torn tail"
            );
        }
        // Damage to the *last* synced record is indistinguishable from a
        // torn write of the commit in flight: accepted as a torn tail.
        let sim = SimVfs::new(2);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/w")).unwrap();
        let mut w = WalWriter::create(&*vfs, Path::new("/w"), 1).unwrap();
        for i in 1..=3 {
            w.append(&encode_record(i, batch(i).ops(), true).unwrap()).unwrap();
            w.sync().unwrap();
        }
        let len = w.offset;
        sim.corrupt_byte(Path::new("/w/000001.wal"), len - 1, 0x10);
        let (r, _) = collect(&*vfs, 1).unwrap();
        assert_eq!((r.last_seq, r.torn), (2, true));
    }
}
