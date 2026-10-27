//! `LogEngine`: the simplest engine that satisfies the contract.
//!
//! The whole state lives in memory; the disk holds an append-only log of
//! checksummed batches that is rewritten when it gets too large. It exists to
//! validate the test harness itself (if this engine fails a shared test, the
//! test is wrong) and as a "no indexing on disk" baseline. It is not a
//! candidate: memory use and recovery time grow with the whole data set.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::engine::{Durability, Engine, Op, Options, Snapshot, WriteBatch};
use crate::error::{Error, Result};
use crate::vfs::{File, OpenMode, Vfs};

type Map = BTreeMap<Vec<u8>, Vec<u8>>;

const LOG: &str = "log";
const LOG_TMP: &str = "log.tmp";
const HEADER: usize = 16; // len u32 | crc u32 | seq u64
/// Written at the end of the log on clean close. If present, every record
/// before it must be valid: a bad record then is corruption, not a torn
/// write from a crash, and is reported instead of truncated.
const CLEAN_MAGIC: &[u8; 8] = b"KRNOLOGC";
const CLEAN_LEN: usize = 24; // magic | end offset u64 | crc u32 | pad u32

fn clean_marker(end: u64) -> [u8; CLEAN_LEN] {
    let mut m = [0u8; CLEAN_LEN];
    m[..8].copy_from_slice(CLEAN_MAGIC);
    m[8..16].copy_from_slice(&end.to_le_bytes());
    let crc = crc32c::crc32c(&m[..16]);
    m[16..20].copy_from_slice(&crc.to_le_bytes());
    m
}

struct Writer {
    file: Arc<dyn File>,
    end: u64,
    seq: u64,
    map: Arc<Map>,
    live_bytes: u64,
    poisoned: bool,
}

pub struct LogEngine {
    vfs: Arc<dyn Vfs>,
    dir: PathBuf,
    durability: Durability,
    writer: Mutex<Writer>,
    current: Mutex<(u64, Arc<Map>)>,
    _lock: Box<dyn std::any::Any + Send + Sync>,
}

fn encode_batch(ops: &[Op], out: &mut Vec<u8>) {
    for op in ops {
        match op {
            Op::Put { key, value } => {
                out.push(1);
                out.extend_from_slice(&(key.len() as u32).to_le_bytes());
                out.extend_from_slice(&(value.len() as u32).to_le_bytes());
                out.extend_from_slice(key);
                out.extend_from_slice(value);
            }
            Op::Delete { key } => {
                out.push(2);
                out.extend_from_slice(&(key.len() as u32).to_le_bytes());
                out.extend_from_slice(key);
            }
        }
    }
}

fn apply_encoded(map: &mut Map, mut body: &[u8]) -> Option<()> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        if b.len() < n {
            return None;
        }
        let (h, t) = b.split_at(n);
        *b = t;
        Some(h)
    }
    fn u32_(b: &mut &[u8]) -> Option<usize> {
        Some(u32::from_le_bytes(take(b, 4)?.try_into().ok()?) as usize)
    }
    while !body.is_empty() {
        match take(&mut body, 1)?[0] {
            1 => {
                let kl = u32_(&mut body)?;
                let vl = u32_(&mut body)?;
                let k = take(&mut body, kl)?.to_vec();
                let v = take(&mut body, vl)?.to_vec();
                map.insert(k, v);
            }
            2 => {
                let kl = u32_(&mut body)?;
                map.remove(take(&mut body, kl)?);
            }
            _ => return None,
        }
    }
    Some(())
}

fn record(seq: u64, body: &[u8]) -> Vec<u8> {
    let mut rec = Vec::with_capacity(HEADER + body.len());
    rec.extend_from_slice(&(body.len() as u32).to_le_bytes());
    let mut crc = crc32c::crc32c(&seq.to_le_bytes());
    crc = crc32c::crc32c_append(crc, body);
    rec.extend_from_slice(&crc.to_le_bytes());
    rec.extend_from_slice(&seq.to_le_bytes());
    rec.extend_from_slice(body);
    rec
}

fn map_bytes(map: &Map) -> u64 {
    map.iter().map(|(k, v)| (k.len() + v.len() + 9) as u64).sum()
}

impl LogEngine {
    fn rewrite(&self, w: &mut Writer) -> Result<()> {
        let tmp = self.dir.join(LOG_TMP);
        if self.vfs.exists(&tmp)? {
            self.vfs.remove_file(&tmp)?;
        }
        let f = self.vfs.open(&tmp, OpenMode::CreateNew)?;
        let mut body = Vec::new();
        let ops: Vec<Op> = w.map.iter().map(|(k, v)| Op::Put { key: k.clone(), value: v.clone() }).collect();
        encode_batch(&ops, &mut body);
        // A "checkpoint" record: marker byte 0xFF, then the full state.
        let mut cp = vec![0xFF];
        cp.extend_from_slice(&body);
        let rec = record(w.seq, &cp);
        f.write_at(&rec, 0)?;
        f.sync_data()?;
        self.vfs.rename(&tmp, &self.dir.join(LOG))?;
        self.vfs.sync_dir(&self.dir)?;
        w.file = f;
        w.end = rec.len() as u64;
        w.live_bytes = map_bytes(&w.map);
        Ok(())
    }
}

impl Engine for LogEngine {
    type Snapshot = LogSnapshot;

    fn name() -> &'static str {
        "log"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        vfs.create_dir_all(dir)?;
        let lock = vfs.lock(&dir.join("LOCK")).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock => Error::Locked,
            _ => Error::Io(e),
        })?;
        let path = dir.join(LOG);
        let file = vfs.open(&path, OpenMode::CreateOrOpen)?;
        // Always, not only when the file was just created: a previous open
        // may have created it and then failed before syncing the directory.
        vfs.sync_dir(dir)?;
        let len = file.len()?;
        let mut buf = vec![0u8; len as usize];
        file.read_at(&mut buf, 0)?;
        let clean = buf.len() >= CLEAN_LEN && {
            let data_end = (buf.len() - CLEAN_LEN) as u64;
            buf[buf.len() - CLEAN_LEN..] == clean_marker(data_end)
        };
        if clean {
            buf.truncate(buf.len() - CLEAN_LEN);
        }
        let mut map = Map::new();
        let mut pos = 0usize;
        let mut seq = 0u64;
        while pos + HEADER <= buf.len() {
            let blen = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
            let crc = u32::from_le_bytes(buf[pos + 4..pos + 8].try_into().unwrap());
            let rseq = u64::from_le_bytes(buf[pos + 8..pos + 16].try_into().unwrap());
            let Some(body) = buf.get(pos + HEADER..pos + HEADER + blen) else { break };
            let mut c = crc32c::crc32c(&rseq.to_le_bytes());
            c = crc32c::crc32c_append(c, body);
            if c != crc || (seq != 0 && rseq != seq + 1) {
                break;
            }
            if body.first() == Some(&0xFF) {
                map.clear();
                apply_encoded(&mut map, &body[1..]).ok_or_else(|| Error::corruption("bad checkpoint"))?;
            } else {
                apply_encoded(&mut map, body).ok_or_else(|| Error::corruption("bad batch"))?;
            }
            seq = rseq;
            pos += HEADER + blen;
        }
        if pos < buf.len() && clean {
            return Err(Error::corruption(format!("invalid log record at offset {pos} in a cleanly closed log")));
        }
        if (pos as u64) < len {
            // Torn tail from a crash (or the clean marker): cut it off so new
            // records follow the last valid one.
            file.set_len(pos as u64)?;
            file.sync_data()?;
        }
        let map = Arc::new(map);
        let live_bytes = map_bytes(&map);
        Ok(LogEngine {
            vfs,
            dir: dir.to_path_buf(),
            durability: opts.durability,
            writer: Mutex::new(Writer { file, end: pos as u64, seq, map: map.clone(), live_bytes, poisoned: false }),
            current: Mutex::new((seq, map)),
            _lock: lock,
        })
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        batch.validate()?;
        let mut w = self.writer.lock();
        if w.poisoned {
            return Err(Error::Poisoned("earlier I/O failure".into()));
        }
        if batch.is_empty() {
            return Ok(w.seq);
        }
        let seq = w.seq + 1;
        let mut body = Vec::new();
        encode_batch(batch.ops(), &mut body);
        let rec = record(seq, &body);
        let io = (|| -> Result<()> {
            w.file.write_at(&rec, w.end)?;
            if self.durability == Durability::Sync {
                w.file.sync_data()?;
            }
            Ok(())
        })();
        if let Err(e) = io {
            w.poisoned = true;
            return Err(e);
        }
        w.end += rec.len() as u64;
        w.seq = seq;
        let map = Arc::make_mut(&mut w.map);
        apply_encoded(map, &body).expect("own encoding");
        w.live_bytes = map_bytes(&w.map);
        *self.current.lock() = (seq, w.map.clone());
        if w.end > 4 * w.live_bytes + (64 << 10)
            && let Err(e) = self.rewrite(&mut w)
        {
            w.poisoned = true;
            // The commit itself is durable (or buffered) already.
            let _ = e;
        }
        Ok(seq)
    }

    fn snapshot(&self) -> Result<LogSnapshot> {
        let (seq, map) = self.current.lock().clone();
        Ok(LogSnapshot { seq, map })
    }

    fn close(self) -> Result<()> {
        let w = self.writer.lock();
        if w.poisoned {
            return Err(Error::Poisoned("earlier I/O failure".into()));
        }
        w.file.write_at(&clean_marker(w.end), w.end)?;
        w.file.sync_data()?;
        Ok(())
    }
}

pub struct LogSnapshot {
    seq: u64,
    map: Arc<Map>,
}

impl Snapshot for LogSnapshot {
    fn seq(&self) -> u64 {
        self.seq
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.map.get(key).cloned())
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if let (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) = (start, end)
            && (a > b || (a == b && !matches!((start, end), (Bound::Included(_), Bound::Included(_)))))
        {
            return Ok(Vec::new());
        }
        Ok(self.map.range::<[u8], _>((start, end)).take(limit).map(|(k, v)| (k.clone(), v.clone())).collect())
    }
}
