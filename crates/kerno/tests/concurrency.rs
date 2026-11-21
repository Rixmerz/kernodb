//! The concurrency model of `docs/concurrency.md`, through the public API on real directories:
//! many reader threads and one writer at a time inside a process, one writer process at a time on
//! a database directory, and read-only processes beside it.
//!
//! The process tests run this same test binary again as a child (`KERNO_CONCURRENCY_ROLE` says
//! which part it plays), so there is nothing else to build.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use kerno::units::{ChangeMeta, UnitUpdate};
use kerno::{AccessMode, Config, Db, UnitFilter, UnitId};

/// Long enough for a loaded machine to start a process and open a database.
const DEADLINE: Duration = Duration::from_secs(60);

/// The tests of this file run one at a time. A `fork` in one test copies the descriptors another
/// test has open, among them its `LOCK` file, and `flock` is released only when every copy is
/// gone: the other test would see `Locked` until the child reaches `exec`.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A fresh directory, removed when the test ends, pass or fail.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-concurrency-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir); // a leftover of an earlier run that died
        std::fs::create_dir_all(&dir).expect("create the test directory");
        TempDir(dir)
    }

    fn db(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn payload(v: u64) -> UnitUpdate {
    UnitUpdate { payload: v.to_string().into_bytes(), schema_version: None, metadata: None }
}

fn number(bytes: Option<&[u8]>) -> u64 {
    std::str::from_utf8(bytes.expect("the unit has a payload")).expect("UTF-8").parse().expect("a decimal number")
}

fn read_only() -> Config {
    Config { access: AccessMode::ReadOnly, allow_unverified_platform: true, ..Config::default() }
}

// ---------------------------------------------------------------- threads

/// A change set is atomic to readers: a snapshot sees all of its units at one version or none of
/// them, never a mix; it reads the same thing every time it is read; and a snapshot taken after
/// `commit` returned sees that commit.
#[test]
fn readers_see_whole_change_sets_only() {
    const CELLS: usize = 8;
    const ROUNDS: u64 = 100;
    const READERS: usize = 4;
    let _one = serial();
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let cells: Vec<UnitId> = (0..CELLS).map(|_| db.store("cell", "0").unwrap()).collect();
    let started = AtomicUsize::new(0);
    let done = AtomicBool::new(false);

    std::thread::scope(|s| {
        let readers: Vec<_> = (0..READERS)
            .map(|_| {
                s.spawn(|| {
                    started.fetch_add(1, Ordering::SeqCst);
                    let (mut last, mut snapshots) = (0u64, 0u64);
                    loop {
                        // Read the flag first: the snapshot below then starts after the writer
                        // finished when it is true, and must show the last version.
                        let finished = done.load(Ordering::SeqCst);
                        let snap = db.unit_db().snapshot().unwrap();
                        let read = |id| number(snap.get_unit(id).unwrap().expect("exists").payload.as_deref());
                        let versions: Vec<u64> = cells.iter().map(|&id| read(id)).collect();
                        assert!(versions.iter().all(|&v| v == versions[0]), "a partial change set: {versions:?}");
                        assert!(versions[0] >= last, "versions went back from {last} to {}", versions[0]);
                        let again: Vec<u64> = cells.iter().map(|&id| read(id)).collect();
                        assert_eq!(again, versions, "one snapshot read two different states");
                        (last, snapshots) = (versions[0], snapshots + 1);
                        if finished {
                            assert_eq!(last, ROUNDS, "a commit that returned is not visible to a new snapshot");
                            return snapshots;
                        }
                    }
                })
            })
            .collect();
        // The writer starts when every reader runs, so the commits happen under read load.
        while started.load(Ordering::SeqCst) < READERS {
            std::thread::yield_now();
        }
        for v in 1..=ROUNDS {
            let mut cs = db.unit_db().begin_changeset(ChangeMeta::default()).unwrap();
            for &id in &cells {
                cs.update(id, payload(v), None).unwrap();
            }
            cs.commit().unwrap();
        }
        done.store(true, Ordering::SeqCst);
        for r in readers {
            assert!(r.join().expect("a reader panicked") >= 1);
        }
    });
    db.close().unwrap();
}

/// A snapshot is a fixed view: commits made after it, by other threads, change neither what it
/// reads nor what it counts (no phantoms), while a snapshot taken afterwards sees them.
#[test]
fn a_snapshot_is_a_fixed_view() {
    let _one = serial();
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let kept = db.store("cell", "1").unwrap();
    let of_cell = UnitFilter { unit_type: Some("cell".into()), ..UnitFilter::default() };
    let before = db.unit_db().snapshot().unwrap();

    std::thread::scope(|s| {
        s.spawn(|| {
            db.unit_db().begin_changeset(ChangeMeta::default()).unwrap().commit().unwrap();
            let mut cs = db.unit_db().begin_changeset(ChangeMeta::default()).unwrap();
            cs.update(kept, payload(2), None).unwrap();
            cs.commit().unwrap();
            db.store("cell", "3").unwrap();
        });
    });

    let unit = before.get_unit(kept).unwrap().expect("exists");
    assert_eq!(number(unit.payload.as_deref()), 1, "the snapshot saw a later update");
    assert_eq!(before.query(&of_cell).unwrap().units.len(), 1, "the snapshot saw a later unit (a phantom)");
    assert_eq!(number(db.get(kept).unwrap().unwrap().payload.as_deref()), 2);
    assert_eq!(db.query(&of_cell).unwrap().len(), 2);
    drop(before);
    db.close().unwrap();
}

/// Writers are serial: change sets that read a counter and write it back never overlap, so no
/// update is lost, each read value is seen once, and the commit sequences are consecutive.
#[test]
fn writers_are_serial_and_lose_no_update() {
    const THREADS: usize = 4;
    const EACH: usize = 25;
    let _one = serial();
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let counter = db.store("counter", "0").unwrap();
    let first = db.unit_db().snapshot().unwrap().seq().0 + 1;
    let seen = Mutex::new(Vec::new());
    let seqs = Mutex::new(Vec::new());

    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| {
                for _ in 0..EACH {
                    let mut cs = db.unit_db().begin_changeset(ChangeMeta::default()).unwrap();
                    let now = number(cs.get_unit(counter).unwrap().expect("exists").payload.as_deref());
                    cs.update(counter, payload(now + 1), None).unwrap();
                    let info = cs.commit().unwrap();
                    seen.lock().unwrap().push(now);
                    seqs.lock().unwrap().push(info.commit_seq.0);
                }
            });
        }
    });

    let total = (THREADS * EACH) as u64;
    assert_eq!(number(db.get(counter).unwrap().unwrap().payload.as_deref()), total, "an update was lost");
    let mut seen = seen.into_inner().unwrap();
    seen.sort_unstable();
    assert_eq!(seen, (0..total).collect::<Vec<_>>(), "two change sets read the same state");
    let mut seqs = seqs.into_inner().unwrap();
    seqs.sort_unstable();
    assert_eq!(seqs, (first..first + total).collect::<Vec<_>>(), "the commit sequences are not consecutive");
    db.close().unwrap();
}

/// An open change set holds the write slot: another writer waits for it, while readers do not.
#[test]
fn a_writer_waits_for_the_open_change_set_and_readers_do_not() {
    let _one = serial();
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let id = db.store("cell", "1").unwrap();
    let (held_tx, held_rx) = mpsc::channel();
    let reader_done = AtomicBool::new(false);
    let committing = AtomicBool::new(false);

    std::thread::scope(|s| {
        s.spawn(|| {
            let mut cs = db.unit_db().begin_changeset(ChangeMeta::default()).unwrap();
            cs.update(id, payload(2), None).unwrap();
            held_tx.send(()).unwrap();
            // Commit only after a reader got through, and after the other writer had time to try.
            let end = Instant::now() + DEADLINE;
            while !reader_done.load(Ordering::SeqCst) {
                assert!(Instant::now() < end, "a reader is blocked by an open change set");
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(150));
            committing.store(true, Ordering::SeqCst);
            cs.commit().unwrap();
        });
        let other = {
            held_rx.recv_timeout(DEADLINE).expect("the first writer holds its change set");
            s.spawn(|| {
                db.store("cell", "other").unwrap();
                assert!(committing.load(Ordering::SeqCst), "a second writer got in while a change set was open");
            })
        };
        // A reader runs while the change set is open, and sees the committed state, not its writes.
        assert_eq!(number(db.get(id).unwrap().unwrap().payload.as_deref()), 1);
        reader_done.store(true, Ordering::SeqCst);
        other.join().expect("the second writer panicked");
    });
    assert_eq!(number(db.get(id).unwrap().unwrap().payload.as_deref()), 2);
    db.close().unwrap();
}

/// The lock is per open handle, not per process: a second read-write `Db` on the same directory is
/// refused while the first lives. A read-only one takes no lock (ADR-0024) and reads beside it.
#[test]
fn a_second_handle_in_the_same_process_gets_locked() {
    let _one = serial();
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.store("cell", "1").unwrap();
    assert!(Db::open(dir.db()).err().expect("a second read-write handle").is_locked());
    let beside = Db::open_with(dir.db(), read_only()).expect("a read-only handle beside the writer");
    assert_eq!(beside.query(&UnitFilter::default()).unwrap().len(), 1);
    beside.close().unwrap();
    db.close().unwrap();
    let again = Db::open_with(dir.db(), read_only()).expect("free after close");
    assert_eq!(again.query(&UnitFilter::default()).unwrap().len(), 1);
}

// -------------------------------------------------------------- processes

/// Environment variable that makes a re-run of this binary play a part (see [`child`]).
const ROLE: &str = "KERNO_CONCURRENCY_ROLE";
const DIR: &str = "KERNO_CONCURRENCY_DIR";
const INDEX: &str = "KERNO_CONCURRENCY_INDEX";
const REPORT: &str = "KERNO_CONCURRENCY> ";

/// In a child: sends one line to the parent.
fn report(msg: &str) {
    println!("{REPORT}{msg}");
}

/// Runs the test `test` again as a child playing `role`.
fn spawn(test: &str, role: &str, dir: &Path, index: usize) -> Child {
    Command::new(std::env::current_exe().expect("path of the running test binary"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .env(DIR, dir)
        .env(INDEX, index.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("re-execute the test binary as a child")
}

/// The lines a child reported, delivered as they come.
fn reports(child: &mut Child) -> mpsc::Receiver<String> {
    let mut out = BufReader::new(child.stdout.take().expect("piped stdout"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        // Read to the end: the child's stdout must never fill up and block it.
        while out.read_line(&mut line).unwrap_or(0) > 0 {
            if let Some(at) = line.find(REPORT) {
                let _ = tx.send(line[at + REPORT.len()..].trim_end().to_owned());
            }
            line.clear();
        }
    });
    rx
}

/// The child that holds the database: opens it, stores a unit, says `ready`, and closes when its
/// stdin does.
fn hold(dir: &Path) {
    let db = Db::open(dir).expect("the holder opens the database");
    db.store("held", "by the child").expect("the holder stores");
    report("ready");
    std::io::stdin().read_to_end(&mut Vec::new()).expect("stdin of the holder");
    db.close().expect("the holder closes cleanly");
}

/// Starts the holder and returns it once it is ready.
fn start_holder(test: &str, dir: &Path) -> Child {
    let mut child = spawn(test, "hold", dir, 0);
    let lines = reports(&mut child);
    match lines.recv_timeout(DEADLINE) {
        Ok(msg) if msg == "ready" => child,
        other => {
            let _ = child.kill();
            panic!("the holder did not become ready: {other:?}");
        }
    }
}

fn assert_locked(dir: &Path) {
    let rw = Db::open(dir).err().expect("a read-write open must fail while another process holds the directory");
    assert!(rw.is_locked(), "read-write open: {rw:?}");
    // A read-only open takes no lock (ADR-0024): it succeeds and sees what the holder committed.
    let ro = Db::open_with(dir, read_only()).expect("a read-only open beside the holder");
    let held = ro.query(&UnitFilter { unit_type: Some("held".into()), ..UnitFilter::default() }).unwrap();
    assert_eq!(held.len(), 1, "the reader sees the holder's commit");
    ro.close().unwrap();
}

/// What the holder stored is there once the directory is free, and both kinds of open work.
fn assert_free_and_intact(dir: &Path) {
    let db = Db::open_with(dir, read_only()).expect("a read-only open after the holder is gone");
    let held = db.query(&UnitFilter { unit_type: Some("held".into()), ..UnitFilter::default() }).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].payload.as_deref(), Some(&b"by the child"[..]));
    db.close().unwrap();
    Db::open(dir).expect("a read-write open after the holder is gone").close().unwrap();
}

/// While another process has the database, every read-write open says `Locked` at once and a
/// read-only one reads beside it; when it closes, the next process opens it and finds what it stored.
#[test]
fn a_second_process_gets_locked_until_the_first_closes() {
    const TEST: &str = "a_second_process_gets_locked_until_the_first_closes";
    if std::env::var(ROLE).is_ok() {
        return hold(Path::new(&std::env::var(DIR).expect("the directory"))); // the child
    }
    let _one = serial();
    let dir = TempDir::new();
    let mut child = start_holder(TEST, dir.db());
    let started = Instant::now();
    assert_locked(dir.db());
    assert!(started.elapsed() < Duration::from_secs(10), "an open waited for the lock instead of failing");
    std::thread::sleep(Duration::from_millis(100));
    assert_locked(dir.db()); // still locked a moment later: the first answers were not luck
    drop(child.stdin.take());
    assert!(child.wait().expect("wait for the holder").success(), "the holder failed");
    assert_free_and_intact(dir.db());
}

/// The lock belongs to the process: when it dies without closing, the kernel releases it and the
/// next process recovers the database with what the dead one had committed.
#[test]
fn a_killed_process_releases_the_lock() {
    const TEST: &str = "a_killed_process_releases_the_lock";
    if std::env::var(ROLE).is_ok() {
        return hold(Path::new(&std::env::var(DIR).expect("the directory"))); // the child
    }
    let _one = serial();
    let dir = TempDir::new();
    let mut child = start_holder(TEST, dir.db());
    assert_locked(dir.db());
    child.kill().expect("kill the holder");
    child.wait().expect("wait for the holder");
    assert_free_and_intact(dir.db());
}

/// The child that takes a turn: opens the database, retrying while another process has it, holds
/// it a while so the others have to wait, stores one unit and closes. Reports how often it was
/// refused.
fn take_a_turn(dir: &Path, index: usize) {
    let end = Instant::now() + DEADLINE;
    let mut refused = 0u32;
    let db = loop {
        match Db::open(dir) {
            Ok(db) => break db,
            Err(e) if e.is_locked() && Instant::now() < end => {
                refused += 1;
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => panic!("process {index} could not open the database: {e:?}"),
        }
    };
    std::thread::sleep(Duration::from_millis(200));
    db.store("turn", format!("process {index}")).expect("store");
    db.close().expect("close");
    report(&format!("refused {refused}"));
}

/// Processes that open the same directory at the same time take turns through the lock file:
/// each one gets in alone, none loses a commit, and the database is whole afterwards.
#[test]
fn processes_take_turns_through_the_lock_file() {
    const TEST: &str = "processes_take_turns_through_the_lock_file";
    const PROCESSES: usize = 4;
    if std::env::var(ROLE).is_ok() {
        let index = std::env::var(INDEX).expect("the index").parse().expect("a number");
        return take_a_turn(Path::new(&std::env::var(DIR).expect("the directory")), index);
    }
    let _one = serial();
    let dir = TempDir::new();
    // Create the database first, so the children contend for the lock rather than for creation.
    Db::open(dir.db()).unwrap().close().unwrap();
    let mut children: Vec<_> = (0..PROCESSES).map(|i| spawn(TEST, "turn", dir.db(), i)).collect();
    let lines: Vec<_> = children.iter_mut().map(reports).collect();
    let mut refused = 0u32;
    for (child, lines) in children.iter_mut().zip(&lines) {
        let msg = lines.recv_timeout(DEADLINE * 2).expect("a child finished its turn");
        refused += msg.strip_prefix("refused ").expect("a refusal count").parse::<u32>().expect("a number");
        assert!(child.wait().expect("wait for a child").success(), "a child failed");
    }
    assert!(refused >= 1, "no process was ever refused: the processes did not overlap, so nothing was shown");
    let db = Db::open(dir.db()).unwrap();
    let units = db.query(&UnitFilter { unit_type: Some("turn".into()), ..UnitFilter::default() }).unwrap();
    let mut got: Vec<String> =
        units.iter().map(|u| String::from_utf8(u.payload.clone().expect("payload")).expect("UTF-8")).collect();
    got.sort();
    assert_eq!(got, (0..PROCESSES).map(|i| format!("process {i}")).collect::<Vec<_>>());
    db.close().unwrap();
}
