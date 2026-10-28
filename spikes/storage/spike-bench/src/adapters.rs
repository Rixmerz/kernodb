//! Production engines wrapped in the spike contract, as reference points:
//! redb (copy-on-write B+ tree), fjall (LSM tree) and SQLite (B-tree + WAL).
//!
//! They ignore the `Vfs` argument and use the real file system directly.
//! Commit sequence numbers are only tracked in memory (benchmarks do not
//! reopen and check them).

use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use redb::ReadableDatabase;
use spike_core::{Durability, Engine, Error, Op, Options, Result, Snapshot, Vfs, WriteBatch};

fn err(e: impl std::fmt::Display) -> Error {
    Error::Io(io::Error::other(e.to_string()))
}

// ---------------------------------------------------------------- redb

const REDB_TABLE: redb::TableDefinition<&[u8], &[u8]> = redb::TableDefinition::new("kv");

pub struct RedbEngine {
    db: redb::Database,
    durability: Durability,
    seq: AtomicU64,
    lock: Mutex<()>,
}

pub struct RedbSnapshot {
    table: redb::ReadOnlyTable<&'static [u8], &'static [u8]>,
    seq: u64,
}

impl Engine for RedbEngine {
    type Snapshot = RedbSnapshot;

    fn name() -> &'static str {
        "redb"
    }

    fn open(_vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut b = redb::Builder::new();
        b.set_cache_size(opts.cache_bytes);
        let db = b.create(dir.join("data.redb")).map_err(err)?;
        let tx = db.begin_write().map_err(err)?;
        tx.open_table(REDB_TABLE).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(RedbEngine { db, durability: opts.durability, seq: AtomicU64::new(0), lock: Mutex::new(()) })
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        let _g = self.lock.lock();
        let mut tx = self.db.begin_write().map_err(err)?;
        tx.set_durability(match self.durability {
            Durability::Sync => redb::Durability::Immediate,
            Durability::Buffered => redb::Durability::None,
        })
        .map_err(err)?;
        {
            let mut t = tx.open_table(REDB_TABLE).map_err(err)?;
            for op in batch.ops() {
                match op {
                    Op::Put { key, value } => {
                        t.insert(key.as_slice(), value.as_slice()).map_err(err)?;
                    }
                    Op::Delete { key } => {
                        t.remove(key.as_slice()).map_err(err)?;
                    }
                }
            }
        }
        tx.commit().map_err(err)?;
        Ok(self.seq.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn snapshot(&self) -> Result<RedbSnapshot> {
        let seq = self.seq.load(Ordering::SeqCst);
        let r = self.db.begin_read().map_err(err)?;
        Ok(RedbSnapshot { table: r.open_table(REDB_TABLE).map_err(err)?, seq })
    }

    fn close(self) -> Result<()> {
        if self.durability == Durability::Buffered {
            // Make buffered commits durable, as the contract requires.
            let mut tx = self.db.begin_write().map_err(err)?;
            tx.set_durability(redb::Durability::Immediate).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        Ok(())
    }
}

impl Snapshot for RedbSnapshot {
    fn seq(&self) -> u64 {
        self.seq
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.table.get(key).map_err(err)?.map(|v| v.value().to_vec()))
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut out = Vec::new();
        if limit == 0 {
            return Ok(out);
        }
        for item in self.table.range::<&[u8]>((start, end)).map_err(err)? {
            let (k, v) = item.map_err(err)?;
            out.push((k.value().to_vec(), v.value().to_vec()));
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- fjall

pub struct FjallEngine {
    db: fjall::Database,
    ks: fjall::Keyspace,
    mode: fjall::PersistMode,
    seq: AtomicU64,
    lock: Mutex<()>,
}

pub struct FjallSnapshot {
    snap: fjall::Snapshot,
    ks: fjall::Keyspace,
    seq: u64,
}

impl Engine for FjallEngine {
    type Snapshot = FjallSnapshot;

    fn name() -> &'static str {
        "fjall"
    }

    fn open(_vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        let db = fjall::Database::builder(dir).cache_size(opts.cache_bytes as u64).open().map_err(err)?;
        // No compression: none of the other engines compress, and the
        // benchmark values are random bytes anyway.
        let ks = db
            .keyspace("kv", || {
                fjall::KeyspaceCreateOptions::default()
                    .data_block_compression_policy(fjall::config::CompressionPolicy::disabled())
            })
            .map_err(err)?;
        let mode = match opts.durability {
            Durability::Sync => fjall::PersistMode::SyncData,
            Durability::Buffered => fjall::PersistMode::Buffer,
        };
        Ok(FjallEngine { db, ks, mode, seq: AtomicU64::new(0), lock: Mutex::new(()) })
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        let _g = self.lock.lock();
        let mut b = self.db.batch().durability(Some(self.mode));
        for op in batch.ops() {
            match op {
                Op::Put { key, value } => b.insert(&self.ks, key.as_slice(), value.as_slice()),
                Op::Delete { key } => b.remove(&self.ks, key.as_slice()),
            }
        }
        b.commit().map_err(err)?;
        Ok(self.seq.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn snapshot(&self) -> Result<FjallSnapshot> {
        let seq = self.seq.load(Ordering::SeqCst);
        Ok(FjallSnapshot { snap: self.db.snapshot(), ks: self.ks.clone(), seq })
    }

    fn close(self) -> Result<()> {
        self.db.persist(fjall::PersistMode::SyncAll).map_err(err)
    }
}

impl Snapshot for FjallSnapshot {
    fn seq(&self) -> u64 {
        self.seq
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        use fjall::Readable;
        Ok(self.snap.get(&self.ks, key).map_err(err)?.map(|v| v.to_vec()))
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        use fjall::Readable;
        let mut out = Vec::new();
        if limit == 0 {
            return Ok(out);
        }
        for guard in self.snap.range::<&[u8], _>(&self.ks, (start, end)) {
            let (k, v) = guard.into_inner().map_err(err)?;
            out.push((k.to_vec(), v.to_vec()));
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- SQLite

pub struct SqliteEngine {
    path: PathBuf,
    writer: Mutex<rusqlite::Connection>,
    readers: Arc<Mutex<Vec<rusqlite::Connection>>>,
    reader_cache_kib: i64,
    seq: AtomicU64,
}

pub struct SqliteSnapshot {
    conn: Option<rusqlite::Connection>,
    pool: Arc<Mutex<Vec<rusqlite::Connection>>>,
    seq: u64,
}

fn sqlite_conn(path: &Path, synchronous: &str, cache_kib: i64) -> Result<rusqlite::Connection> {
    let c = rusqlite::Connection::open(path).map_err(err)?;
    c.execute_batch(&format!(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous={synchronous}; PRAGMA cache_size=-{cache_kib};"
    ))
    .map_err(err)?;
    Ok(c)
}

impl Engine for SqliteEngine {
    type Snapshot = SqliteSnapshot;

    fn name() -> &'static str {
        "sqlite"
    }

    fn open(_vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("data.sqlite");
        // WAL + FULL fsyncs the WAL on every commit (power-loss durable);
        // WAL + NORMAL defers fsync to checkpoints (process-crash durable).
        let sync = match opts.durability {
            Durability::Sync => "FULL",
            Durability::Buffered => "NORMAL",
        };
        // Each SQLite connection has a private page cache. Keep the total
        // within the shared budget: half for the writer, an eighth for each
        // of up to four reader connections.
        let cache_kib = (opts.cache_bytes / 1024) as i64;
        let writer = sqlite_conn(&path, sync, (cache_kib / 2).max(1024))?;
        writer
            .execute_batch("CREATE TABLE IF NOT EXISTS kv (k BLOB PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;")
            .map_err(err)?;
        Ok(SqliteEngine {
            path,
            writer: Mutex::new(writer),
            readers: Arc::new(Mutex::new(Vec::new())),
            reader_cache_kib: (cache_kib / 8).max(1024),
            seq: AtomicU64::new(0),
        })
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        let mut c = self.writer.lock();
        let tx = c.transaction().map_err(err)?;
        {
            let mut put = tx.prepare_cached("INSERT OR REPLACE INTO kv (k, v) VALUES (?1, ?2)").map_err(err)?;
            let mut del = tx.prepare_cached("DELETE FROM kv WHERE k = ?1").map_err(err)?;
            for op in batch.ops() {
                match op {
                    Op::Put { key, value } => {
                        put.execute((key, value)).map_err(err)?;
                    }
                    Op::Delete { key } => {
                        del.execute([key]).map_err(err)?;
                    }
                }
            }
        }
        tx.commit().map_err(err)?;
        Ok(self.seq.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn snapshot(&self) -> Result<SqliteSnapshot> {
        let seq = self.seq.load(Ordering::SeqCst);
        let conn = match self.readers.lock().pop() {
            Some(c) => c,
            None => sqlite_conn(&self.path, "NORMAL", self.reader_cache_kib)?,
        };
        // A read transaction pins its WAL snapshot at the first read.
        conn.execute_batch("BEGIN; SELECT count(*) FROM (SELECT 1 FROM kv LIMIT 1);").map_err(err)?;
        Ok(SqliteSnapshot { conn: Some(conn), pool: self.readers.clone(), seq })
    }

    fn close(self) -> Result<()> {
        self.writer.lock().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").map_err(err)
    }
}

impl Drop for SqliteSnapshot {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take()
            && c.execute_batch("COMMIT;").is_ok()
        {
            self.pool.lock().push(c);
        }
    }
}

impl Snapshot for SqliteSnapshot {
    fn seq(&self) -> u64 {
        self.seq
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let c = self.conn.as_ref().unwrap();
        let mut st = c.prepare_cached("SELECT v FROM kv WHERE k = ?1").map_err(err)?;
        let mut rows = st.query([key]).map_err(err)?;
        match rows.next().map_err(err)? {
            Some(r) => Ok(Some(r.get(0).map_err(err)?)),
            None => Ok(None),
        }
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let c = self.conn.as_ref().unwrap();
        let mut sql = String::from("SELECT k, v FROM kv WHERE 1");
        let mut params: Vec<&[u8]> = Vec::new();
        match start {
            Bound::Included(k) => {
                sql.push_str(" AND k >= ?");
                params.push(k);
            }
            Bound::Excluded(k) => {
                sql.push_str(" AND k > ?");
                params.push(k);
            }
            Bound::Unbounded => {}
        }
        match end {
            Bound::Included(k) => {
                sql.push_str(" AND k <= ?");
                params.push(k);
            }
            Bound::Excluded(k) => {
                sql.push_str(" AND k < ?");
                params.push(k);
            }
            Bound::Unbounded => {}
        }
        sql.push_str(&format!(" ORDER BY k LIMIT {limit}"));
        let mut st = c.prepare_cached(&sql).map_err(err)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(params), |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
    }
}
