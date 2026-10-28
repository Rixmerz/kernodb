//! `verify()`: a full integrity check of a database that never writes
//! (`docs/file-format/v1.md` §7.9 SST-V1..V4 and §9.5 RO-1..RO-4).
//!
//! It takes the exclusive `LOCK`, reads the MANIFEST, scans the WALs the
//! recovery table needs and classifies the directory with the same
//! [`recovery::classify`] the opens use, then opens every referenced table and
//! reads every one of its blocks. Nothing is kept: tables are read block by
//! block straight from the file, bypassing the block cache (`fill_cache =
//! false`), and WALs are scanned in 1 MiB reads, so memory does not grow with
//! the database. The only write path of recovery, [`recovery::apply`], is not
//! called from here.
//!
//! Damage is data: every violated rule becomes an [`Issue`] carrying the rule
//! id, and the check goes on with the next file. The error is reserved for
//! what stops the check from saying anything: `Io`, `Locked`, `Incompatible`
//! (a version or feature this engine does not support is not damage) and
//! `InvalidArgument` for a directory that is not a database.
//!
//! The classification stops at the first rule it finds violated, as the
//! opens do: a report with an issue from `REC-5..REC-10` is not a list of
//! every such violation, though it does list every damaged table block.

use std::io;
use std::path::Path;
use std::sync::Arc;

use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Error, Result};

use crate::compaction::NUM_LEVELS;
use crate::files::{FileSource, format_error};
use crate::format::sst::{SstExpect, SstMeta, SstVerifier, open_sst_from, read_data_block};
use crate::format::wal::{WalHeaderState, WalScan, WalScanParams, scan_wal};
use crate::format::{End, ErrorClass, FormatError, ReadError};
use crate::inspect::{FileInfo, FileType, HeaderInfo, lock_and_list, refuse_directory_without_database};
use crate::manifest::{self, FileMeta, MANIFEST};
use crate::recovery::{self, Listing, Plan, RuleViolation, WalFacts, wals_to_scan};
use crate::sst::sst_name;
use crate::wal::wal_name;

/// What [`verify`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Read every block of every table (SST-R1..R3, SST-V1..V4). Without it
    /// only the open-time checks run (SST-O1..O7 against the MANIFEST).
    pub deep: bool,
    /// Scan the live WALs (WAL-R*, REC-7, REC-10). Without it they are not
    /// read and the rules that need their contents are not checked.
    pub check_wal: bool,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        VerifyOptions { deep: true, check_wal: true }
    }
}

/// One violated rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// The file it is in, by name.
    pub file: String,
    /// Where in the file, when the rule has a position.
    pub offset: Option<u64>,
    /// The rule id of `docs/file-format/v1.md` (`SST-R1`, `REC-5`, ...).
    pub rule: &'static str,
    /// What is wrong; never key or value bytes.
    pub detail: String,
}

/// What the MANIFEST says about one level, as opposed to what was read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelReport {
    pub files: u64,
    pub bytes: u64,
    /// Point entries.
    pub entries: u64,
    /// Range-delete fragments.
    pub fragments: u64,
}

/// What the scan of one live WAL found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalReport {
    pub number: u64,
    pub frames: u64,
    pub commits: u64,
    /// The header is valid (a log whose creation a crash cut short is not).
    pub header_valid: bool,
    /// The log ends in bytes that are not a valid frame (WAL-R5): the tail a
    /// crash tore, accepted when no frame follows it.
    pub torn_tail: bool,
}

/// A file nothing needs (REC-7, REC-8, REC-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Orphan {
    pub name: String,
    pub len: u64,
}

/// The result of [`verify`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Version and feature bits of every file read: the MANIFEST, the WALs
    /// that were scanned and the referenced tables.
    pub files: Vec<FileInfo>,
    /// The last valid MANIFEST record is a `CLEAN` snapshot (MAN-R4).
    pub clean: bool,
    /// The MANIFEST ends in a torn tail (MAN-R3): accepted, the next
    /// read-write open starts a new generation.
    pub manifest_torn_tail: bool,
    /// The MANIFEST's `last_seq`.
    pub last_seq: u64,
    /// Seven levels, as the MANIFEST records them.
    pub levels: Vec<LevelReport>,
    /// The live WALs, oldest first (none when `check_wal` is off).
    pub wals: Vec<WalReport>,
    /// Canonical files nothing needs; a read-write open deletes them.
    pub orphans: Vec<Orphan>,
    /// Orphans that disappeared while the check ran (RO-4): an outliving
    /// snapshot deleted them. Informational.
    pub vanished_orphans: Vec<String>,
    /// A `MANIFEST.tmp` exists (REC-12).
    pub manifest_tmp: bool,
    /// Entries that are not engine files (REC-13).
    pub unknown: Vec<String>,
    pub issues: Vec<Issue>,
}

impl VerifyReport {
    /// No rule is violated. Orphans, unknown files and a torn tail are not
    /// violations.
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Checks the database in `dir` without writing anything (the only file it
/// may create is `LOCK`).
///
/// Errors: `Locked` if a handle or another process holds the database,
/// `Incompatible` for a file this engine cannot read by design (a newer
/// version, an unknown `incompat` bit, the M0 format), `InvalidArgument` if
/// `dir` is not a database, `Io`. Everything else that is wrong with the
/// files is in [`VerifyReport::issues`].
pub fn verify(vfs: Arc<dyn Vfs>, dir: &Path, opts: &VerifyOptions) -> Result<VerifyReport> {
    let first = recovery::list_dir(&*vfs, dir)?;
    refuse_directory_without_database(&first, dir)?;
    let (_lock, listing) = lock_and_list(&*vfs, dir)?;
    refuse_directory_without_database(&listing, dir)?;
    let mut v = Verifier { vfs: &*vfs, dir, opts, report: VerifyReport::default() };
    v.run(&listing)?;
    Ok(v.report)
}

struct Verifier<'a> {
    vfs: &'a dyn Vfs,
    dir: &'a Path,
    opts: &'a VerifyOptions,
    report: VerifyReport,
}

impl Verifier<'_> {
    fn issue(&mut self, file: &str, offset: Option<u64>, rule: &'static str, detail: impl Into<String>) {
        if !self.report.issues.iter().any(|i| i.file == file && i.rule == rule) {
            self.report.issues.push(Issue { file: file.to_string(), offset, rule, detail: detail.into() });
        }
    }

    /// Damage becomes an issue; an unsupported version or feature is the error.
    fn format_issue(&mut self, file: &str, e: FormatError) -> Result<()> {
        match e.class {
            ErrorClass::Incompatible => Err(format_error(file, e)),
            ErrorClass::Corruption => {
                self.issue(file, Some(e.offset), e.rule, e.detail);
                Ok(())
            }
        }
    }

    fn violation(&mut self, v: RuleViolation) {
        self.issue(&v.file, None, v.rule, v.detail);
    }

    fn run(&mut self, listing: &Listing) -> Result<()> {
        self.report.unknown = listing.unknown.clone();
        self.report.manifest_tmp = listing.has_manifest_tmp;
        if !listing.has_manifest {
            // refuse_directory_without_database let this through: data files
            // and no MANIFEST (REC-2).
            self.issue(MANIFEST, None, "REC-2", "data files exist but the MANIFEST is missing");
            return Ok(());
        }
        let loaded = match manifest::read_structured(self.vfs, self.dir) {
            Ok(l) => l,
            Err(ReadError::Source(e)) => return Err(e.into()),
            Err(ReadError::Format(e)) => return self.format_issue(MANIFEST, e),
        };
        let replay = &loaded.replay;
        let state = &replay.state;
        self.report.clean = replay.clean;
        self.report.manifest_torn_tail = replay.end == End::Torn;
        self.report.last_seq = state.last_seq;
        let manifest_len = self.vfs.open(&self.dir.join(MANIFEST), OpenMode::Existing)?.len()?;
        self.report.files.push(FileInfo {
            name: MANIFEST.to_string(),
            file_type: FileType::Manifest,
            len: manifest_len,
            header: Some(HeaderInfo::of(&replay.header)),
            problem: None,
        });
        self.report.levels = state
            .levels
            .iter()
            .map(|files| LevelReport {
                files: files.len() as u64,
                bytes: files.iter().map(|f| f.size).sum(),
                entries: files.iter().map(|f| f.point_entries).sum(),
                fragments: files.iter().map(|f| f.range_fragments).sum(),
            })
            .collect();
        debug_assert_eq!(self.report.levels.len(), NUM_LEVELS);

        let plan = self.classify(listing, replay)?;
        for (level, files) in state.levels.iter().enumerate() {
            debug_assert!(files.iter().all(|f| usize::from(f.level) == level));
            for meta in files {
                self.table(meta, state.last_seq, replay.header.db_id)?;
            }
        }
        if let Some(plan) = plan {
            self.orphans(&plan)?;
        }
        Ok(())
    }

    /// REC-5..REC-13 over the listing, the MANIFEST and the WAL scans. `None`
    /// when a rule is violated (it is in the issues) or a WAL cannot be
    /// scanned.
    fn classify(
        &mut self,
        listing: &Listing,
        replay: &crate::format::manifest::ManifestReplay,
    ) -> Result<Option<Plan>> {
        let state = &replay.state;
        let numbers = match wals_to_scan(listing, state, replay.clean) {
            Ok(n) => n,
            Err(v) => {
                self.violation(v);
                return Ok(None);
            }
        };
        let scans = if self.opts.check_wal {
            match self.scan_wals(&numbers, replay.header.db_id, state.last_seq + 1)? {
                Some(s) => s,
                None => return Ok(None),
            }
        } else {
            numbers.iter().map(|&number| WalFacts { number, scan: unscanned() }).collect()
        };
        match recovery::classify(listing, replay, &scans) {
            Ok(plan) => Ok(Some(plan)),
            Err(v) => {
                self.violation(v);
                Ok(None)
            }
        }
    }

    /// The dry-run replay: every frame of every log is read and checked, none
    /// is applied. `None` after the first damaged log, whose end sequence the
    /// next one would be checked against.
    fn scan_wals(&mut self, numbers: &[u64], db_id: [u8; 16], first_seq: u64) -> Result<Option<Vec<WalFacts>>> {
        let mut expected = first_seq;
        let mut facts = Vec::with_capacity(numbers.len());
        for &number in numbers {
            let name = wal_name(number);
            let file = self.vfs.open(&self.dir.join(&name), OpenMode::Existing)?;
            let mut src = FileSource::new(&*file)?;
            let params = WalScanParams { file_number: number, db_id, expected_seq: expected };
            let scan = match scan_wal(&mut src, &params, |_, _| Ok(())) {
                Ok(scan) => scan,
                Err(ReadError::Source(e)) => return Err(e.into()),
                Err(ReadError::Format(e)) => {
                    self.format_issue(&name, e)?;
                    return Ok(None);
                }
            };
            expected = scan.next_seq;
            let (header, problem) = match scan.header {
                WalHeaderState::Valid(h) => (Some(HeaderInfo::of(&h)), None),
                WalHeaderState::Broken { rule } => (None, Some(format!("{name}: {rule}: header does not decode"))),
            };
            self.report.wals.push(WalReport {
                number,
                frames: scan.frames,
                commits: scan.commits,
                header_valid: header.is_some(),
                torn_tail: scan.end == End::Torn,
            });
            self.report.files.push(FileInfo { name, file_type: FileType::Wal, len: src_len(&*file)?, header, problem });
            facts.push(WalFacts { number, scan });
        }
        Ok(Some(facts))
    }

    /// Opens a referenced table (SST-O1..O7) and, with `deep`, reads every
    /// block of it (SST-R1..R3) and runs SST-V1..V4 over what it holds.
    fn table(&mut self, meta: &FileMeta, last_seq: u64, db_id: [u8; 16]) -> Result<()> {
        let name = sst_name(meta.number);
        let file = match self.vfs.open(&self.dir.join(&name), OpenMode::Existing) {
            Ok(f) => f,
            // A referenced table cannot disappear legitimately (RO-4).
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.issue(&name, None, "REC-5", "referenced by the MANIFEST but missing");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let mut src = FileSource::new(&*file)?;
        let ex = SstExpect { file_number: meta.number, db_id, meta: Some(meta), last_seq };
        let sst = match open_sst_from(&mut src, &ex) {
            Ok(sst) => sst,
            Err(ReadError::Source(e)) => return Err(e.into()),
            Err(ReadError::Format(e)) => return self.format_issue(&name, e),
        };
        self.report.files.push(FileInfo {
            name: name.clone(),
            file_type: FileType::Sst,
            len: src_len(&*file)?,
            header: Some(HeaderInfo::of(&sst.header)),
            problem: None,
        });
        if self.opts.deep {
            self.read_blocks(&name, &*file, &sst, meta.number)?;
        }
        Ok(())
    }

    /// Every data block, one buffer at a time: the index (SST-O5) bounds each
    /// block by the file, so no buffer is larger than the table.
    fn read_blocks(&mut self, name: &str, file: &dyn kerno_core::vfs::File, sst: &SstMeta, number: u64) -> Result<()> {
        let mut verifier = SstVerifier::new(sst);
        // After a damaged block the totals SST-V1 compares are incomplete.
        let mut intact = true;
        for i in 0..sst.index.len() {
            let handle = sst.index[i].handle;
            let mut buf = vec![0u8; handle.len as usize];
            file.read_at(&mut buf, handle.offset)?;
            match read_data_block(&buf, number, &sst.index, i, &sst.props) {
                Err(e) => {
                    intact = false;
                    self.format_issue(name, e)?;
                }
                Ok(entries) => {
                    for e in &entries {
                        if let Err(err) = verifier.observe(e, handle.offset) {
                            intact = false;
                            self.format_issue(name, err)?;
                            break;
                        }
                    }
                }
            }
        }
        if intact && let Err(e) = verifier.finish() {
            self.format_issue(name, e)?;
        }
        Ok(())
    }

    /// RO-3, RO-4: the files nothing needs are reported, never deleted. One
    /// that cannot be opened because an outliving snapshot deleted it is a
    /// vanished orphan, not an issue.
    fn orphans(&mut self, plan: &Plan) -> Result<()> {
        for name in &plan.orphans {
            match self.vfs.open(&self.dir.join(name), OpenMode::Existing) {
                Ok(file) => self.report.orphans.push(Orphan { name: name.clone(), len: file.len()? }),
                Err(e) if e.kind() == io::ErrorKind::NotFound => self.report.vanished_orphans.push(name.clone()),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

fn src_len(file: &dyn kerno_core::vfs::File) -> Result<u64> {
    file.len().map_err(Error::Io)
}

/// The scan of a WAL that `check_wal = false` did not read: no frames and no
/// valid header, so [`recovery::classify`] has no WAL rule to apply to it.
fn unscanned() -> WalScan {
    WalScan {
        header: WalHeaderState::Broken { rule: "unscanned" },
        frames: 0,
        commits: 0,
        valid_len: 0,
        end: End::Clean,
        next_seq: 0,
    }
}
