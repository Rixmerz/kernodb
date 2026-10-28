//! Deferred deletion of obsolete files (`docs/file-format/v1.md` W-13).
//!
//! A table that a durable MANIFEST record no longer lists is deleted when the
//! last version or snapshot that holds it goes away (`Table::drop`). That drop
//! can happen in a reader's thread, so it only queues the path here: readers
//! never delete files. While the engine is alive a dedicated thread
//! (`kerno-delete`) deletes the queued paths and retries the ones that fail,
//! counting each failure; in [`Background::Inline`](crate::Background) there
//! is no thread and the thread that queues a path deletes it right away. Once
//! the handle has closed ([`Deleter::shutdown`]) every path is deleted by
//! whoever queues it, as before.
//!
//! A file whose deletion keeps failing stays on disk, listed as an orphan by
//! the next recovery, which deletes it.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kerno_core::probe;
use kerno_core::vfs::Vfs;
use parking_lot::{Condvar, Mutex};

use crate::stats::{Stats, bump};

/// First delay before retrying a failed deletion; doubled up to
/// [`MAX_BACKOFF`].
const FIRST_BACKOFF: Duration = Duration::from_millis(10);
const MAX_BACKOFF: Duration = Duration::from_secs(1);

struct Item {
    path: PathBuf,
    size: u64,
    /// Failed attempts so far.
    attempts: u32,
    /// Not before.
    due: Instant,
}

#[derive(Default)]
struct Queue {
    items: Vec<Item>,
    /// Items taken by the deleting thread and not yet finished.
    in_flight: usize,
    /// A thread consumes the queue; otherwise whoever queues also deletes.
    threaded: bool,
    stop: bool,
}

pub struct Deleter {
    vfs: Arc<dyn Vfs>,
    stats: Arc<Stats>,
    queue: Mutex<Queue>,
    cv: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Deleter {
    /// A deleter without a thread: deletes in the thread that queues.
    pub fn inline(vfs: Arc<dyn Vfs>, stats: Arc<Stats>) -> Arc<Self> {
        Arc::new(Deleter {
            vfs,
            stats,
            queue: Mutex::new(Queue::default()),
            cv: Condvar::new(),
            thread: Mutex::new(None),
        })
    }

    /// Starts the `kerno-delete` thread.
    pub fn spawn(self: &Arc<Self>) -> io::Result<()> {
        self.queue.lock().threaded = true;
        let this = self.clone();
        let handle = std::thread::Builder::new().name("kerno-delete".into()).spawn(move || this.run())?;
        *self.thread.lock() = Some(handle);
        Ok(())
    }

    /// Queues `path`, a file of `size` bytes that nothing references any more.
    pub fn submit(&self, path: PathBuf, size: u64) {
        bump(&self.stats.obsolete_bytes_pending, size);
        let mut q = self.queue.lock();
        q.items.push(Item { path, size, attempts: 0, due: Instant::now() });
        if q.threaded {
            drop(q);
            self.cv.notify_all();
        } else {
            drop(q);
            self.delete_due();
        }
    }

    /// Deletes what is due, in the calling thread; used when no thread
    /// consumes the queue.
    pub fn delete_due(&self) {
        let _phase = probe::phase("delete");
        let batch = self.take_due();
        self.process(batch);
    }

    fn take_due(&self) -> Vec<Item> {
        let mut q = self.queue.lock();
        let now = Instant::now();
        let (due, later): (Vec<Item>, Vec<Item>) = std::mem::take(&mut q.items).into_iter().partition(|i| i.due <= now);
        q.items = later;
        q.in_flight += due.len();
        due
    }

    /// One attempt per item; failures go back to the queue with a delay.
    fn process(&self, batch: Vec<Item>) {
        let n = batch.len();
        let mut failed = Vec::new();
        for mut item in batch {
            match self.vfs.remove_file(&item.path) {
                Ok(()) => {
                    bump(&self.stats.obsolete_files_deleted, 1);
                    self.stats.obsolete_bytes_pending.fetch_sub(item.size, std::sync::atomic::Ordering::Relaxed);
                }
                // Already gone (a recovery or another handle deleted it).
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.stats.obsolete_bytes_pending.fetch_sub(item.size, std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => {
                    bump(&self.stats.delete_failures, 1);
                    item.attempts += 1;
                    let backoff = FIRST_BACKOFF.saturating_mul(1 << item.attempts.min(10)).min(MAX_BACKOFF);
                    item.due = Instant::now() + backoff;
                    failed.push(item);
                }
            }
        }
        let mut q = self.queue.lock();
        q.items.extend(failed);
        q.in_flight -= n;
        drop(q);
        self.cv.notify_all();
    }

    /// The body of the deleting thread.
    fn run(&self) {
        let _phase = probe::phase("delete");
        loop {
            {
                let mut q = self.queue.lock();
                loop {
                    if q.stop {
                        return;
                    }
                    let now = Instant::now();
                    match q.items.iter().map(|i| i.due).min() {
                        Some(due) if due <= now => break,
                        Some(due) => {
                            self.cv.wait_until(&mut q, due);
                        }
                        None => self.cv.wait(&mut q),
                    }
                }
            }
            let batch = self.take_due();
            self.process(batch);
        }
    }

    /// Waits until every queued path has been tried at least once (one that
    /// keeps failing does not hold this up: its failures are counted).
    /// Without a thread, deletes what is due first.
    pub fn settle(&self) {
        if !self.queue.lock().threaded {
            self.delete_due();
            return;
        }
        let mut q = self.queue.lock();
        while !q.stop && (q.in_flight > 0 || q.items.iter().any(|i| i.attempts == 0)) {
            self.cv.wait(&mut q);
        }
    }

    /// Stops the thread (if any) and deletes what is queued, once, in the
    /// calling thread; from now on every path is deleted by whoever queues
    /// it. Safe to call again.
    pub fn shutdown(&self) {
        {
            let mut q = self.queue.lock();
            q.stop = true;
        }
        self.cv.notify_all();
        if let Some(t) = self.thread.lock().take() {
            let _ = t.join();
        }
        {
            let mut q = self.queue.lock();
            q.threaded = false;
            q.stop = false;
            // One attempt for everything, whatever its backoff said.
            for i in &mut q.items {
                i.due = Instant::now();
            }
        }
        self.delete_due();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_core::vfs::OpenMode;
    use kerno_testkit::vfs::SimVfs;
    use std::path::Path;
    use std::sync::atomic::Ordering::Relaxed;

    fn setup() -> (Arc<dyn Vfs>, Arc<Stats>, Arc<Deleter>) {
        let vfs = SimVfs::new(1).as_vfs();
        vfs.create_dir_all(Path::new("/d")).unwrap();
        let stats = Arc::new(Stats::default());
        let deleter = Deleter::inline(vfs.clone(), stats.clone());
        (vfs, stats, deleter)
    }

    fn file(vfs: &Arc<dyn Vfs>, name: &str) -> PathBuf {
        let p = Path::new("/d").join(name);
        vfs.open(&p, OpenMode::CreateNew).unwrap().write_at(&[7; 100], 0).unwrap();
        p
    }

    #[test]
    fn inline_deleter_deletes_in_the_calling_thread() {
        let (vfs, stats, d) = setup();
        let p = file(&vfs, "a");
        d.submit(p.clone(), 100);
        assert!(!vfs.exists(&p).unwrap());
        assert_eq!((stats.obsolete_files_deleted.load(Relaxed), stats.obsolete_bytes_pending.load(Relaxed)), (1, 0));
    }

    #[test]
    fn a_thread_deletes_queued_paths_and_shutdown_goes_back_to_inline() {
        let (vfs, stats, d) = setup();
        d.spawn().unwrap();
        let p = file(&vfs, "a");
        d.submit(p.clone(), 100);
        d.settle();
        assert!(!vfs.exists(&p).unwrap());
        assert_eq!(stats.obsolete_files_deleted.load(Relaxed), 1);
        d.shutdown();
        let q = file(&vfs, "b");
        d.submit(q.clone(), 100);
        assert!(!vfs.exists(&q).unwrap(), "after shutdown the caller deletes");
        assert_eq!(stats.obsolete_files_deleted.load(Relaxed), 2);
    }

    #[test]
    fn a_missing_file_is_not_a_failure() {
        let (_, stats, d) = setup();
        d.submit(Path::new("/d/never-existed").to_path_buf(), 50);
        assert_eq!((stats.delete_failures.load(Relaxed), stats.obsolete_bytes_pending.load(Relaxed)), (0, 0));
    }
}
