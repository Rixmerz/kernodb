//! Recovery (`docs/file-format/v1.md` §9): what a directory holds and what
//! to do about it.
//!
//! [`classify`] is a pure function: given the listing of the directory, the
//! replay of its MANIFEST and the scan of the WALs that matter, it applies
//! REC-1..REC-13 and returns a [`Plan`] or the error the table prescribes. It
//! reads no file and writes none, so the read-write open, the read-only open
//! and `verify()` share it. Only [`apply`] writes (S-2..S-5), so a directory
//! that the plan rejects is left as it was (I-18).
//!
//! The scan of the WALs is separate from the classification because the
//! rules need to know which WALs hold frames and how each one ends, not the
//! commits: [`scan_wals`] reads them without keeping any, and the commits are
//! read again by the replay that follows a plan, in chunks the size of a
//! write buffer (S-2), so recovery memory does not grow with the logs.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::Arc;

use kerno_core::vfs::Vfs;
use kerno_core::{Error, Result};

use crate::compaction::NUM_LEVELS;
use crate::config::ReadOnlyReason;
use crate::flush;
use crate::format::End;
use crate::format::limits::MAX_FILE_NUMBER;
use crate::format::manifest::{Edit, ManifestReplay, ManifestState};
use crate::format::props::REASON_RECOVERY;
use crate::format::wal::{WalHeaderState, WalScan};
use crate::manifest::{self, FileMeta, MANIFEST, MANIFEST_TMP, ManifestWriter, Snapshot};
use crate::memtable::Memtable;
use crate::sst::{Table, TableCtx, sst_name};
use crate::stats::bump;
use crate::version::{Levels, level_metas};
use crate::wal::{self, SPARE_WAL, WalWriter, wal_name};

/// What a directory entry is (DIR-1, §2).
pub enum FileKind {
    Manifest,
    ManifestTmp,
    Lock,
    /// The spare WAL (`wal::SPARE_WAL`): never replayed; it may be half
    /// written, so recovery deletes it.
    SpareWal,
    Wal(u64),
    Sst(u64),
    Other,
}

/// Classifies a directory entry. Only canonical names (`{n:06}.wal` /
/// `{n:06}.sst` with `1 <= n <= 2^48-1`, as produced by `wal_name` /
/// `sst_name`) are engine files: a stray `1.wal` would otherwise be opened
/// and deleted as `000001.wal`. Everything else is unknown: ignored and never
/// deleted.
pub fn parse_name(name: &str) -> FileKind {
    let number = |s: &str| {
        let n: u64 = s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok()).flatten()?;
        ((1..=MAX_FILE_NUMBER).contains(&n) && format!("{n:06}") == s).then_some(n)
    };
    if name == MANIFEST {
        FileKind::Manifest
    } else if name == MANIFEST_TMP {
        FileKind::ManifestTmp
    } else if name == "LOCK" {
        FileKind::Lock
    } else if name == SPARE_WAL {
        FileKind::SpareWal
    } else if let Some(n) = name.strip_suffix(".wal").and_then(number) {
        FileKind::Wal(n)
    } else if let Some(n) = name.strip_suffix(".sst").and_then(number) {
        FileKind::Sst(n)
    } else {
        FileKind::Other
    }
}

/// What a directory holds, by kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub has_manifest: bool,
    /// A `MANIFEST.tmp` (REC-12).
    pub has_manifest_tmp: bool,
    /// Canonical WAL numbers, ascending.
    pub wals: Vec<u64>,
    /// Canonical SST numbers, ascending.
    pub ssts: Vec<u64>,
    /// A spare WAL was left behind (it may be half written: never reused).
    pub spare_wal: bool,
    /// Entries that are not engine files (`LOCK` and `MANIFEST.tmp` aside),
    /// by name (REC-13).
    pub unknown: Vec<String>,
}

impl Listing {
    /// Builds the listing of a directory from its entry names.
    pub fn from_names<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Listing {
        let mut l = Listing::default();
        for name in names {
            match parse_name(name.as_ref()) {
                FileKind::Manifest => l.has_manifest = true,
                FileKind::ManifestTmp => l.has_manifest_tmp = true,
                FileKind::Wal(n) => l.wals.push(n),
                FileKind::Sst(n) => l.ssts.push(n),
                FileKind::SpareWal => l.spare_wal = true,
                FileKind::Lock => {}
                FileKind::Other => l.unknown.push(name.as_ref().to_string()),
            }
        }
        l.wals.sort_unstable();
        l.ssts.sort_unstable();
        l.unknown.sort_unstable();
        l
    }

    /// The largest file number present, 0 if there is none.
    fn max_number(&self) -> u64 {
        self.wals.iter().chain(&self.ssts).copied().max().unwrap_or(0)
    }
}

/// Lists `dir`; a directory that does not exist is empty.
pub fn list_dir(vfs: &dyn Vfs, dir: &Path) -> Result<Listing> {
    match vfs.list(dir) {
        Ok(names) => Ok(Listing::from_names(names)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Listing::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Why a directory without a MANIFEST cannot become a database (REC-2,
/// REC-3, DIR-2); `None` if it is empty or holds only `LOCK` and
/// `MANIFEST.tmp`, which a crash during creation leaves (REC-1). WAL or SST
/// files without a MANIFEST are a damaged database. Anything else is not
/// ours.
pub fn refuse_without_manifest(l: &Listing) -> Option<Error> {
    if !l.wals.is_empty() || !l.ssts.is_empty() {
        return Some(Error::Corruption("data files exist but the MANIFEST is missing".into()));
    }
    (!l.unknown.is_empty()).then(|| Error::InvalidArgument("not a Kerno database".into()))
}

/// A rule of the decision table (REC-5..REC-10) that a directory violates.
/// It converts to `Corruption` for the opens; `verify()` reports it as an
/// issue with the rule id as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleViolation {
    /// The file the rule names; empty for an internal inconsistency.
    pub file: String,
    pub rule: &'static str,
    pub detail: String,
}

impl RuleViolation {
    fn new(file: impl Into<String>, rule: &'static str, detail: impl Into<String>) -> Self {
        RuleViolation { file: file.into(), rule, detail: detail.into() }
    }
}

impl From<RuleViolation> for Error {
    fn from(v: RuleViolation) -> Error {
        if v.file.is_empty() {
            Error::Corruption(format!("{}: {}", v.rule, v.detail))
        } else {
            Error::Corruption(format!("{}: {}: {}", v.file, v.rule, v.detail))
        }
    }
}

/// What the scan of one WAL found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalFacts {
    pub number: u64,
    pub scan: WalScan,
}

/// The WALs whose scan [`classify`] needs, oldest first: after a clean close
/// the ones that cannot belong to it (numbered `next_file` or more, REC-7);
/// otherwise the live ones, from `log_number` (REC-10). A missing required
/// WAL is REC-9.
pub fn wals_to_scan(
    listing: &Listing,
    state: &ManifestState,
    clean: bool,
) -> std::result::Result<Vec<u64>, RuleViolation> {
    let from = if clean { state.next_file } else { state.log_number };
    if !clean && !listing.wals.contains(&state.log_number) {
        return Err(RuleViolation::new(
            wal_name(state.log_number),
            "REC-9",
            "required WAL missing (the MANIFEST says it is the oldest live log)",
        ));
    }
    Ok(listing.wals.iter().copied().filter(|&n| n >= from).collect())
}

/// What to do with a directory that has a MANIFEST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The last valid MANIFEST record is a `CLEAN` snapshot (MAN-R4).
    pub clean: bool,
    /// The WALs to replay, oldest first (none after a clean close).
    pub live_wals: Vec<u64>,
    /// The newest live WAL that holds at least one frame: the only one whose
    /// tail may not be durable (see [`apply`], S-2b).
    pub newest_wal_with_frames: Option<u64>,
    /// Canonical files that nothing needs: unreferenced SSTs and WALs that
    /// belong to no live chain (REC-7, REC-8, REC-11). Read-write opens
    /// delete them (S-5); the others report them.
    pub orphans: Vec<String>,
    /// A `MANIFEST.tmp` exists (REC-12).
    pub manifest_tmp: bool,
    /// Entries that are not engine files (REC-13).
    pub unknown: Vec<String>,
    /// The first file of the MANIFEST and the live WALs with `ro_compat`
    /// bits this engine does not know (HDR-7); table headers are added once
    /// the tables are open.
    pub unknown_ro_compat: Option<(String, u64)>,
    /// First number nothing in the directory or the MANIFEST uses.
    pub next_file: u64,
}

fn orphan_wal(n: u64) -> String {
    wal_name(n)
}

/// REC-4..REC-13 over a MANIFEST that was read without error. `scans` hold
/// the WALs [`wals_to_scan`] named, in order, each scanned against the
/// sequence its predecessor ended at.
pub fn classify(
    listing: &Listing,
    manifest: &ManifestReplay,
    scans: &[WalFacts],
) -> std::result::Result<Plan, RuleViolation> {
    let state = &manifest.state;
    let clean = manifest.clean;
    let referenced: HashSet<u64> = state.levels.iter().flatten().map(|f| f.number).collect();
    let present: HashSet<u64> = listing.ssts.iter().copied().collect();
    // REC-5, the part that needs only the listing.
    if let Some(n) = referenced.iter().filter(|n| !present.contains(n)).min() {
        return Err(RuleViolation::new(sst_name(*n), "REC-5", "referenced by the MANIFEST but missing"));
    }
    let expected_scans = wals_to_scan(listing, state, clean)?;
    if scans.iter().map(|s| s.number).ne(expected_scans.iter().copied()) {
        return Err(RuleViolation::new("", "internal", "the scanned WALs are not the ones to classify"));
    }
    let mut unknown_ro_compat = None;
    let mut note_ro_compat = |file: &str, bits: u64| {
        if bits != 0 && unknown_ro_compat.is_none() {
            unknown_ro_compat = Some((file.to_string(), bits));
        }
    };
    note_ro_compat(MANIFEST, manifest.header.unknown_ro_compat());

    let mut orphans: Vec<String> = Vec::new();
    let mut live_wals = Vec::new();
    let mut newest_wal_with_frames = None;
    if clean {
        // REC-6: an SST numbered `next_file` or more was written by a session
        // whose SESSION_OPEN is lost (W-8).
        if let Some(n) = listing.ssts.iter().find(|&&n| n >= state.next_file) {
            return Err(RuleViolation::new(
                sst_name(*n),
                "REC-6",
                format!("after a clean close, a table numbered {} or more exists (lost SESSION_OPEN)", state.next_file),
            ));
        }
        // REC-7: such a WAL is an orphan unless it holds a frame.
        for w in scans {
            if w.scan.frames > 0 {
                return Err(RuleViolation::new(
                    wal_name(w.number),
                    "REC-7",
                    format!("after a clean close, a log numbered {} or more holds commits", state.next_file),
                ));
            }
        }
        // REC-8: everything else that is canonical and unreferenced.
        orphans.extend(listing.wals.iter().map(|&n| orphan_wal(n)));
    } else {
        // REC-10: the chain of live WALs.
        live_wals = listing.wals.iter().copied().filter(|&n| n >= state.log_number).collect();
        let newest_with_frames = scans.iter().rposition(|w| w.scan.frames > 0);
        newest_wal_with_frames = newest_with_frames.map(|k| scans[k].number);
        for (i, w) in scans.iter().enumerate() {
            let name = wal_name(w.number);
            let intact_required = newest_with_frames.is_some_and(|k| i < k);
            if intact_required && (!matches!(w.scan.header, WalHeaderState::Valid(_)) || w.scan.end == End::Torn) {
                return Err(RuleViolation::new(
                    name,
                    "REC-10a",
                    "broken header or torn tail in a log that is followed by one with commits",
                ));
            }
            if let WalHeaderState::Valid(h) = &w.scan.header {
                note_ro_compat(&name, h.unknown_ro_compat());
            }
        }
        // REC-11.
        orphans.extend(listing.wals.iter().filter(|&&n| n < state.log_number).map(|&n| orphan_wal(n)));
    }
    orphans.extend(listing.ssts.iter().filter(|n| !referenced.contains(n)).map(|&n| sst_name(n)));
    // A spare WAL may be half written; the next one is prepared from scratch.
    if listing.spare_wal {
        orphans.push(SPARE_WAL.to_string());
    }
    orphans.sort_unstable();
    Ok(Plan {
        clean,
        live_wals,
        newest_wal_with_frames,
        orphans,
        manifest_tmp: listing.has_manifest_tmp,
        unknown: listing.unknown.clone(),
        unknown_ro_compat,
        next_file: state.next_file.max(1).max(listing.max_number() + 1),
    })
}

/// Scans the WALs `numbers` in order without keeping their commits: each
/// starts at the sequence the previous one ended at, the first at
/// `first_seq`.
pub fn scan_wals(vfs: &dyn Vfs, dir: &Path, db_id: [u8; 16], numbers: &[u64], first_seq: u64) -> Result<Vec<WalFacts>> {
    let mut expected = first_seq;
    let mut out = Vec::with_capacity(numbers.len());
    for &number in numbers {
        let scan = wal::replay(vfs, dir, number, db_id, expected, |_, _| Ok(()))?;
        expected = scan.next_seq;
        out.push(WalFacts { number, scan });
    }
    Ok(out)
}

/// S-1: reads the MANIFEST, scans the WALs that matter and classifies the
/// directory, without writing anything.
pub fn plan(vfs: &dyn Vfs, dir: &Path, listing: &Listing) -> Result<(manifest::Loaded, Plan)> {
    let loaded = manifest::read(vfs, dir)?;
    let state = &loaded.replay.state;
    let numbers = wals_to_scan(listing, state, loaded.replay.clean)?;
    let scans = scan_wals(vfs, dir, loaded.replay.header.db_id, &numbers, state.last_seq + 1)?;
    let plan = classify(listing, &loaded.replay, &scans)?;
    Ok((loaded, plan))
}

/// The SSTs the MANIFEST references, open and checked against it (REC-5,
/// SST-O1..O7), and the first of them with `ro_compat` bits this engine does
/// not know.
pub fn open_tables(ctx: &Arc<TableCtx>, state: &ManifestState) -> Result<(Levels, Option<(String, u64)>)> {
    let mut levels: Levels = vec![Vec::new(); NUM_LEVELS];
    let mut unknown = None;
    for (l, files) in state.levels.iter().enumerate() {
        for f in files {
            let table = Table::open(ctx, f, state.last_seq)?;
            if table.unknown_ro_compat != 0 && unknown.is_none() {
                unknown = Some((sst_name(f.number), table.unknown_ro_compat));
            }
            levels[l].push(Arc::new(table));
        }
    }
    Ok((levels, unknown))
}

/// Why a database opens read-only despite a read-write request: the first
/// file with unknown `ro_compat` bits (HDR-7).
pub fn unknown_ro_compat_reason(
    from_plan: &Option<(String, u64)>,
    from_tables: Option<(String, u64)>,
) -> Option<ReadOnlyReason> {
    from_plan.clone().or(from_tables).map(|(file, bits)| ReadOnlyReason::UnknownRoCompat { file, bits })
}

/// Replays every live WAL of the plan into one memtable, in memory, for a
/// read-only handle (RO-2). Returns it with the last sequence replayed.
pub fn replay_in_memory(
    ctx: &TableCtx,
    plan: &Plan,
    db_id: [u8; 16],
    state: &ManifestState,
) -> Result<(Memtable, u64)> {
    let mem = Memtable::new(0);
    let mut expected = state.last_seq + 1;
    for &n in &plan.live_wals {
        let scan = wal::replay(&*ctx.vfs, &ctx.dir, n, db_id, expected, |seq, commit| {
            mem.apply_commit(seq, commit);
            Ok(())
        })?;
        record_replay(ctx, &scan);
        expected = scan.next_seq;
    }
    Ok((mem, expected - 1))
}

fn record_replay(ctx: &TableCtx, scan: &WalScan) {
    bump(&ctx.stats.last_open_replayed_records, scan.commits);
    bump(&ctx.stats.last_open_replayed_bytes, scan.valid_len.saturating_sub(crate::format::header::HEADER_LEN as u64));
}

/// What the read-write open hands to the engine.
pub struct Recovered {
    pub levels: Levels,
    pub last_seq: u64,
    pub log_number: u64,
    pub next_file: u64,
    pub wal: WalWriter,
    pub manifest: ManifestWriter,
}

/// Writes the replayed commits as L0 tables, one set each time the memtable
/// reaches `write_buffer` bytes (S-2: recovery memory is bounded).
struct ChunkWriter<'a> {
    ctx: &'a Arc<TableCtx>,
    next_file: u64,
    write_buffer: usize,
    /// Tables written so far, newest first.
    tables: Vec<Arc<Table>>,
}

impl ChunkWriter<'_> {
    fn flush(&mut self, mem: &mut Memtable) -> Result<()> {
        if mem.is_empty() {
            return Ok(());
        }
        let mut number = || {
            self.next_file += 1;
            self.next_file - 1
        };
        let written = flush::write_memtable(self.ctx, mem, REASON_RECOVERY, &[], &mut number)?;
        bump(&self.ctx.stats.flushes, 1);
        bump(&self.ctx.stats.flush_bytes_written, written.iter().map(|t| t.size).sum());
        self.tables.splice(0..0, written.into_iter().map(Arc::new));
        *mem = Memtable::new(0);
        Ok(())
    }
}

/// S-2..S-5 on a directory that [`plan`] accepted, read-write: the replayed
/// commits go to L0 tables, a fresh WAL is created *before* its record
/// exists (W-1; REC-10c makes that window recoverable), `SESSION_OPEN` (or a
/// checkpoint if the MANIFEST ended in a torn tail or is over its size
/// trigger) makes the new log live and durable (W-8), and then the files
/// nothing needs are deleted.
pub fn apply(
    ctx: &Arc<TableCtx>,
    plan: &Plan,
    loaded: &manifest::Loaded,
    mut levels: Levels,
    write_buffer: usize,
    checkpoint_floor: u64,
    wal_prealloc: u64,
) -> Result<Recovered> {
    let (vfs, dir) = (&*ctx.vfs, ctx.dir.as_path());
    let state = &loaded.replay.state;
    let db_id = loaded.replay.header.db_id;
    let mut writer = loaded.writer(checkpoint_floor);

    // S-2.
    let mut chunks = ChunkWriter { ctx, next_file: plan.next_file, write_buffer, tables: Vec::new() };
    let mut mem = Memtable::new(0);
    let mut expected = state.last_seq + 1;
    for &n in &plan.live_wals {
        let scan = wal::replay(vfs, dir, n, db_id, expected, |seq, commit| {
            mem.apply_commit(seq, commit);
            if mem.is_full(chunks.write_buffer) { chunks.flush(&mut mem) } else { Ok(()) }
        })?;
        record_replay(ctx, &scan);
        expected = scan.next_seq;
    }
    chunks.flush(&mut mem)?;
    let last_seq = expected - 1;
    if !chunks.tables.is_empty() {
        vfs.sync_dir(dir)?;
    }
    let mut next_file = chunks.next_file;
    let new_l0: Vec<FileMeta> = chunks.tables.iter().map(|t| t.meta(0)).collect();
    levels[0].splice(0..0, chunks.tables);

    // S-2b: the frames just replayed may be unsynced (`Buffered`, or a
    // process that died with its page cache intact), and the new log is about
    // to claim they exist through its `base_seq`. Until `SESSION_OPEN` is
    // durable the old chain is what recovery would read, so a power loss in
    // that window must find these frames, or the new log would look like a
    // gap (REC-10c). Only the newest log with frames can have such a tail:
    // every rotation syncs the log it leaves (W-2).
    if let Some(n) = plan.newest_wal_with_frames {
        vfs.open(&dir.join(wal_name(n)), kerno_core::vfs::OpenMode::Existing)?.sync_data()?;
    }

    // S-3.
    if next_file >= MAX_FILE_NUMBER {
        return Err(Error::InvalidArgument("file number space exhausted".into()));
    }
    let wal_number = next_file;
    next_file += 1;
    let wal = WalWriter::create(vfs, dir, wal_number, db_id, last_seq + 1, wal_prealloc)?;

    // S-4.
    if writer.checkpoint_due() {
        let metas = level_metas(&levels);
        writer.checkpoint(
            vfs,
            dir,
            &Snapshot { next_file, last_seq, log_number: wal_number, clean: false, levels: &metas },
        )?;
        bump(&ctx.stats.manifest_checkpoints, 1);
    } else {
        let edit = Edit::SessionOpen { next_file, last_seq, log_number: wal_number, files: new_l0 };
        writer.append(edit)?;
        bump(&ctx.stats.manifest_records, 1);
    }
    bump(&ctx.stats.manifest_writes, 1);
    ctx.stats.manifest_bytes.store(writer.len(), std::sync::atomic::Ordering::Relaxed);

    // S-5: nothing in these files is needed any more. A snapshot that
    // outlived the previous handle may have deleted an obsolete one already.
    let mut deleted = false;
    let names = plan
        .orphans
        .iter()
        .cloned()
        .chain(plan.live_wals.iter().map(|&n| wal_name(n)))
        .chain(plan.manifest_tmp.then(|| MANIFEST_TMP.to_string()));
    for name in names {
        match vfs.remove_file(&dir.join(&name)) {
            Ok(()) => deleted = true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if deleted {
        vfs.sync_dir(dir)?;
    }
    Ok(Recovered { levels, last_seq, log_number: wal_number, next_file, wal, manifest: writer })
}
