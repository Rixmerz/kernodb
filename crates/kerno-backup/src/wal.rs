//! Reading the commits of a live database out of its write-ahead logs, without opening it.
//!
//! The engine holds an exclusive lock on its directory, so nothing here opens a database: the
//! `NNNNNN.wal` files are plain files, checksummed frame by frame, and the engine only ever appends
//! to the newest. A frame that is still being written reads as a torn tail and is left for the next
//! call, which is how the engine's own recovery treats it.

use std::io::Read;
use std::path::{Path, PathBuf};

use kerno_storage::format::FormatError;
use kerno_storage::format::header::{FileKind, HEADER_LEN, HeaderExpect, decode_header};
use kerno_storage::format::limits::MAX_HEADER_LEN;
use kerno_storage::format::wal::{CanonicalCommit, WalScanParams, scan_wal};

use crate::error::{Error, Result};

/// A key no layer of Kerno writes (its tags are all below 0x80). A commit that changes nothing
/// still spends a sequence number, and a restore has to spend it too: it deletes this key, which is
/// never there, to do so.
pub const PAD_KEY: &[u8] = b"\xffkerno-backup-pad";

/// One commit of the engine and the sequence number it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub seq: u64,
    pub body: CanonicalCommit<Vec<u8>>,
}

pub(crate) fn format_error(file: &str, e: &FormatError) -> Error {
    Error::Corrupt(format!("{file}: {} at offset {}: {}", e.rule, e.offset, e.detail))
}

/// Windows answers `Access is denied` (os error 5) to an open of a file the engine is deleting or
/// renaming at that moment (the delete-pending window, until its last handle is gone), where
/// Linux answers `NotFound`. Nothing else is wrong with such a file, so on Windows the open is
/// tried again for a moment; elsewhere a denied open is a real permission problem.
const DENIED_IS_TRANSIENT: bool = cfg!(windows);
const DENIED_ATTEMPTS: u32 = 50;
const DENIED_PAUSE: std::time::Duration = std::time::Duration::from_millis(20);

/// Runs `open`, trying again (at most [`DENIED_ATTEMPTS`] times, [`DENIED_PAUSE`] apart) while it
/// is denied and `transient` says a denial may pass.
fn retry_denied<T>(transient: bool, mut open: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut attempt = 1;
    loop {
        match open() {
            Err(e) if transient && e.kind() == std::io::ErrorKind::PermissionDenied && attempt < DENIED_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(DENIED_PAUSE);
            }
            other => return other,
        }
    }
}

/// Opens `path` to read; `None` when the file is gone.
fn open_wal(path: &Path) -> Result<Option<std::fs::File>> {
    match retry_denied(DENIED_IS_TRANSIENT, || std::fs::File::open(path)) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Store(format!("cannot read {}: {e}", path.display()))),
    }
}

/// The first bytes of `path` (up to what a header can take) and the length of the file;
/// `None` when the file is gone.
fn prefix_of(path: &Path) -> Result<Option<(Vec<u8>, u64)>> {
    let Some(file) = open_wal(path)? else { return Ok(None) };
    let len = file.metadata()?.len();
    let mut prefix = Vec::new();
    file.take(u64::from(MAX_HEADER_LEN)).read_to_end(&mut prefix)?;
    Ok(Some((prefix, len)))
}

/// The `db_id` the engine put in the header of the `MANIFEST` of `dir`: it names a database for
/// good, so a backup can tell its own database from another one.
pub fn read_db_id(dir: &Path) -> Result<[u8; 16]> {
    let path = dir.join("MANIFEST");
    let (prefix, len) = prefix_of(&path)?
        .ok_or_else(|| Error::Store(format!("{} is not a Kerno database: it has no MANIFEST", dir.display())))?;
    let header = decode_header(&prefix, len, FileKind::Manifest, &HeaderExpect { db_id: None, file_number: None })
        .map_err(|e| format_error("MANIFEST", &e))?;
    Ok(header.db_id)
}

struct Head {
    number: u64,
    path: PathBuf,
    db_id: [u8; 16],
    base_seq: u64,
}

/// The canonical WAL names of `dir` (`{n:06}.wal`), by number.
fn wal_files(dir: &Path) -> Result<Vec<(u64, PathBuf)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| Error::Store(format!("cannot list {}: {e}", dir.display())))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".wal") else { continue };
        if let Ok(n) = stem.parse::<u64>()
            && (1..1 << 48).contains(&n)
            && format!("{n:06}") == stem
        {
            files.push((n, dir.join(&name)));
        }
    }
    files.sort();
    Ok(files)
}

/// The header of a WAL; `None` for one that is gone, or whose header is not (yet) a valid one,
/// which the engine treats as a log without commits.
fn head_of(number: u64, path: PathBuf) -> Result<Option<Head>> {
    let Some((prefix, len)) = prefix_of(&path)? else { return Ok(None) };
    if prefix.len() < HEADER_LEN {
        return Ok(None);
    }
    let expect = HeaderExpect { db_id: None, file_number: Some(number) };
    Ok(decode_header(&prefix, len, FileKind::Wal, &expect).ok().map(|h| Head {
        number,
        path,
        db_id: h.db_id,
        base_seq: h.per_type,
    }))
}

/// Every commit of the logs of `dir` with a sequence above `after`, in order, and the `db_id` of the
/// logs (`None` when there is no log to read).
///
/// # Errors
///
/// [`Error::Gap`] when the commit `after + 1` is no longer in any log: the engine flushed and
/// deleted a log whose commits were not read yet. [`Error::Corrupt`] for a log that fails its
/// checksums before its tail.
pub fn read_commits(dir: &Path, after: u64) -> Result<(Vec<Commit>, Option<[u8; 16]>)> {
    let mut heads = Vec::new();
    for (number, path) in wal_files(dir)? {
        heads.extend(head_of(number, path)?);
    }
    let Some(oldest) = heads.iter().map(|h| h.base_seq).min() else { return Ok((Vec::new(), None)) };
    let db_id = heads.first().map(|h| h.db_id);
    let mut commits: Vec<Commit> = Vec::new();
    for (i, head) in heads.iter().enumerate() {
        // The next log starts where this one ends: when that is already behind `after`, nothing in
        // this one is needed, and it need not be read (nor trusted).
        if heads.get(i + 1).is_some_and(|next| next.base_seq <= after + 1) {
            continue;
        }
        // Deleted between the listing and now: the gap check below says whether it mattered.
        let Some(mut file) = open_wal(&head.path)? else { continue };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| Error::Store(format!("cannot read {}: {e}", head.path.display())))?;
        let params = WalScanParams { file_number: head.number, db_id: head.db_id, expected_seq: head.base_seq };
        let name = format!("{:06}.wal", head.number);
        scan_wal(&mut &bytes[..], &params, |_, frame| {
            for (i, body) in frame.commits.into_iter().enumerate() {
                let seq = frame.first_seq + i as u64;
                if seq > after {
                    commits.push(Commit { seq, body });
                }
            }
            Ok(())
        })
        .map_err(|e| format_error(&name, &e.into_format()))?;
    }
    match commits.first() {
        Some(first) if first.seq != after + 1 => return Err(Error::Gap { expected: after + 1, found: first.seq }),
        None if oldest > after + 1 => return Err(Error::Gap { expected: after + 1, found: oldest }),
        _ => {}
    }
    if let Some(w) = commits.windows(2).find(|w| w[1].seq != w[0].seq + 1) {
        return Err(Error::Gap { expected: w[0].seq + 1, found: w[1].seq });
    }
    Ok((commits, db_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_storage::format::features::frame_has_slot;
    use kerno_storage::format::header::Header;
    use kerno_storage::format::wal::{Frame, encode_frame};

    const DB: [u8; 16] = [7; 16];

    fn commit(i: u8) -> CanonicalCommit<Vec<u8>> {
        CanonicalCommit { ranges: vec![], points: vec![(vec![i], Some(vec![i, i]))] }
    }

    /// A WAL file `number` that starts at `base_seq` and holds one frame per entry of `frames`, each
    /// the number of commits it carries.
    fn wal(dir: &Path, number: u64, base_seq: u64, frames: &[u8]) {
        let header = Header::new(FileKind::Wal, DB, number, base_seq);
        let slot = frame_has_slot(header.incompat);
        let mut bytes = header.encode().to_vec();
        let mut seq = base_seq;
        for &n in frames {
            let commits = (0..n).map(|i| commit(i + 1)).collect();
            // Like the engine's: a frame says how much of the file was durable before it.
            let synced_len = bytes.len() as u64;
            bytes.extend(encode_frame(number, &Frame { first_seq: seq, synced_len, commits }, slot));
            seq += u64::from(n);
        }
        std::fs::write(dir.join(format!("{number:06}.wal")), bytes).unwrap();
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kb-wal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seqs(dir: &Path, after: u64) -> Result<Vec<u64>> {
        read_commits(dir, after).map(|(commits, _)| commits.iter().map(|c| c.seq).collect())
    }

    #[test]
    fn the_commits_of_every_log_after_a_sequence_are_read_in_order() {
        let d = dir("order");
        wal(&d, 1, 1, &[2, 1]); // commits 1 to 3
        wal(&d, 2, 4, &[2]); // commits 4 and 5
        std::fs::write(d.join("notes.txt"), b"not a log").unwrap();
        std::fs::write(d.join("1.wal"), b"not a canonical name").unwrap();
        assert_eq!(seqs(&d, 0).unwrap(), [1, 2, 3, 4, 5]);
        // The first log still has unshipped commits: it is read although the second exists.
        assert_eq!(seqs(&d, 2).unwrap(), [3, 4, 5]);
        // Everything in the first is shipped: only the second matters.
        assert_eq!(seqs(&d, 3).unwrap(), [4, 5]);
        assert_eq!(seqs(&d, 5).unwrap(), Vec::<u64>::new());
        assert_eq!(read_commits(&d, 0).unwrap().1, Some(DB));
        let (commits, _) = read_commits(&d, 4).unwrap();
        assert_eq!(commits[0].body, commit(2));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_log_that_was_deleted_before_it_was_read_is_a_gap_even_without_new_commits() {
        let d = dir("gap");
        // Only the log the engine opened after the others were flushed and deleted: it starts at 4.
        wal(&d, 2, 4, &[]);
        assert_eq!(seqs(&d, 3).unwrap(), Vec::<u64>::new());
        assert!(matches!(seqs(&d, 1), Err(Error::Gap { expected: 2, found: 4 })));
        wal(&d, 2, 4, &[1]);
        assert!(matches!(seqs(&d, 1), Err(Error::Gap { expected: 2, found: 4 })));
        assert_eq!(seqs(&d, 3).unwrap(), [4]);
        // No log at all: nothing to read, and nothing to say about a gap.
        std::fs::remove_file(d.join("000002.wal")).unwrap();
        assert_eq!(read_commits(&d, 9).unwrap(), (Vec::new(), None));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_frame_still_being_written_is_left_for_the_next_read() {
        let d = dir("torn");
        wal(&d, 1, 1, &[1, 1]);
        let path = d.join("000001.wal");
        let whole = std::fs::read(&path).unwrap();
        let mut next = whole.clone();
        let header = Header::new(FileKind::Wal, DB, 1, 1);
        let frame = encode_frame(
            1,
            &Frame { first_seq: 3, synced_len: whole.len() as u64, commits: vec![commit(9)] },
            frame_has_slot(header.incompat),
        );
        next.extend_from_slice(&frame[..frame.len() - 3]);
        std::fs::write(&path, &next).unwrap();
        assert_eq!(seqs(&d, 0).unwrap(), [1, 2]);
        next.extend_from_slice(&frame[frame.len() - 3..]);
        std::fs::write(&path, &next).unwrap();
        assert_eq!(seqs(&d, 0).unwrap(), [1, 2, 3]);
        // A damaged byte before the tail is not a torn tail: it is an error, named by file.
        let mut bad = next.clone();
        bad[HEADER_LEN + 40] ^= 0xff;
        std::fs::write(&path, &bad).unwrap();
        let err = seqs(&d, 0).unwrap_err().to_string();
        assert!(err.starts_with("000001.wal: "), "{err}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The Windows race of CI run 45717319100: a log the engine deletes right then is denied, not
    /// missing, for a moment. It cannot be staged on Linux, so the rule is tested with the platform
    /// as a parameter.
    #[test]
    fn a_denied_open_is_tried_again_only_where_a_denial_can_pass() {
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let mut calls = 0;
        let r: std::io::Result<()> = retry_denied(true, || {
            calls += 1;
            if calls < 3 { Err(denied()) } else { Err(std::io::ErrorKind::NotFound.into()) }
        });
        assert_eq!((calls, r.unwrap_err().kind()), (3, std::io::ErrorKind::NotFound));

        // A denial that never passes ends after the attempts, with its own error.
        calls = 0;
        let r: std::io::Result<()> = retry_denied(true, || {
            calls += 1;
            Err(denied())
        });
        assert_eq!((calls, r.unwrap_err().kind()), (DENIED_ATTEMPTS, std::io::ErrorKind::PermissionDenied));

        // Elsewhere a denial is a real permission problem: one attempt, no waiting.
        calls = 0;
        let r: std::io::Result<()> = retry_denied(false, || {
            calls += 1;
            Err(denied())
        });
        assert_eq!((calls, r.unwrap_err().kind()), (1, std::io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn the_db_id_comes_from_the_manifest_header() {
        let d = dir("id");
        assert!(read_db_id(&d).unwrap_err().to_string().contains("is not a Kerno database"));
        let header = Header::new(FileKind::Manifest, DB, 1, 1);
        std::fs::write(d.join("MANIFEST"), header.encode()).unwrap();
        assert_eq!(read_db_id(&d).unwrap(), DB);
        std::fs::write(d.join("MANIFEST"), b"short").unwrap();
        assert!(matches!(read_db_id(&d), Err(Error::Corrupt(_))));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
