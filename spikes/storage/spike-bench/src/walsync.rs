//! `fdatasync` micro-benchmark behind the WAL preallocation lever
//! (`docs/testing/f1-decision-cond1.md` §3, lever 1): what does the sync of a
//! small append cost on a file that grows, on a sparse file whose length was
//! only set, and on a file whose blocks were already written?
//!
//! Each variant appends `record` bytes per iteration at increasing offsets and
//! times the `sync_data` that follows, like the WAL of a serial writer.
//! Optionally a second thread writes bursts to another file in the same
//! directory (a flush or compaction in the background of the engine).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use spike_core::vfs::{OpenMode, StdVfs};

use crate::metrics::{Lat, Report, load_average};

/// How the file is prepared before the first append.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Empty file: every append extends it, so every sync commits the size
    /// (and, with delayed allocation, the new extent).
    Grow,
    /// `set_len`: the size is final but the blocks are a hole.
    SetLen,
    /// Zeros written and synced up to the final size: an append overwrites
    /// blocks that are already allocated and written.
    Prewritten,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Grow, Variant::SetLen, Variant::Prewritten];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Grow => "grow",
            Variant::SetLen => "setlen",
            Variant::Prewritten => "prewritten",
        }
    }
}

/// How the background writer keeps its dirty data bounded while it writes a
/// burst (the engine's SST writers: `bytes_per_sync`, `bytes_per_writeback`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundSync {
    /// `sync_data` every MiB (`bytes_per_sync = 1 MiB`, the engine default).
    EveryMiB,
    /// Nothing until the end of the burst (M0).
    AtEnd,
    /// `File::start_writeback` every MiB, then one `sync_data` at the end.
    WritebackEveryMiB,
    /// `File::start_writeback` of every MiB and `File::wait_writeback` for
    /// the MiB before it (what the engine does with `bytes_per_writeback`),
    /// then one `sync_data` at the end.
    PacedWritebackEveryMiB,
    /// `File::start_writeback` of every MiB and `File::wait_writeback` for
    /// the same MiB right after: what `sync_data` does for the writer, minus
    /// the journal commit.
    BlockingWritebackEveryMiB,
}

impl BackgroundSync {
    pub const ALL: [BackgroundSync; 5] = [
        BackgroundSync::EveryMiB,
        BackgroundSync::AtEnd,
        BackgroundSync::WritebackEveryMiB,
        BackgroundSync::PacedWritebackEveryMiB,
        BackgroundSync::BlockingWritebackEveryMiB,
    ];

    pub fn from_name(name: &str) -> Option<BackgroundSync> {
        BackgroundSync::ALL.into_iter().find(|m| m.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            BackgroundSync::EveryMiB => "sync_every_mib",
            BackgroundSync::AtEnd => "sync_at_end",
            BackgroundSync::WritebackEveryMiB => "writeback_every_mib",
            BackgroundSync::PacedWritebackEveryMiB => "paced_writeback_every_mib",
            BackgroundSync::BlockingWritebackEveryMiB => "blocking_writeback_every_mib",
        }
    }
}

pub struct Params {
    pub dir: PathBuf,
    /// Appends (and syncs) per run.
    pub iterations: usize,
    /// Bytes per append.
    pub record: usize,
    /// Background write bursts while the appends run.
    pub background: bool,
    /// What the background writer does while it writes a burst.
    pub background_sync: BackgroundSync,
    /// The ways of [`suite_writeback`] (all of [`BackgroundSync::ALL`] by
    /// default).
    pub modes: Vec<BackgroundSync>,
    /// Bytes of one background burst (written, then synced).
    pub burst: usize,
    /// Pause between background bursts.
    pub pause: Duration,
}

/// One run of one variant: the latency of each `sync_data`.
pub fn run(variant: Variant, p: &Params) -> std::io::Result<Lat> {
    std::fs::create_dir_all(&p.dir)?;
    let path = p.dir.join(format!("walsync-{}.dat", variant.name()));
    let _ = std::fs::remove_file(&path);
    let size = (p.iterations * p.record) as u64;
    let file = OpenOptions::new().create_new(true).read(true).write(true).open(&path)?;
    match variant {
        Variant::Grow => {}
        Variant::SetLen => file.set_len(size)?,
        Variant::Prewritten => {
            let zeros = vec![0u8; 1 << 20];
            let mut at = 0;
            while at < size {
                let n = zeros.len().min((size - at) as usize);
                file.write_all_at(&zeros[..n], at)?;
                at += n as u64;
            }
        }
    }
    // The preparation is not part of the measurement; sync it so that the
    // timed syncs do not pay for it.
    file.sync_all()?;
    File::open(&p.dir)?.sync_all()?;

    let stop = Arc::new(AtomicBool::new(false));
    let bg = p.background.then(|| {
        let (stop, dir, burst, pause, how) = (stop.clone(), p.dir.clone(), p.burst, p.pause, p.background_sync);
        std::thread::spawn(move || background_writer(&dir, &stop, burst, pause, how))
    });
    // Let the first burst start.
    if p.background {
        std::thread::sleep(Duration::from_millis(20));
    }

    let buf = vec![0xA5u8; p.record];
    let mut lat = Lat::new();
    for i in 0..p.iterations {
        file.write_all_at(&buf, (i * p.record) as u64)?;
        let t = Instant::now();
        file.sync_data()?;
        lat.record_nanos(t.elapsed().as_nanos() as u64);
    }
    stop.store(true, Ordering::SeqCst);
    if let Some(bg) = bg {
        bg.join().expect("background writer panicked")?;
    }
    drop(file);
    let _ = std::fs::remove_file(&path);
    Ok(lat)
}

/// Writes `burst` bytes in 64 KiB chunks to a file of its own, bounding its
/// dirty data as `how` says (by default `sync_data` every MiB, the engine's
/// `bytes_per_sync`), then syncs, truncates and sleeps `pause`. Goes through
/// the engine's `Vfs`, so `WritebackEveryMiB` calls the same
/// `File::start_writeback` the SST writers do.
fn background_writer(
    dir: &Path,
    stop: &AtomicBool,
    burst: usize,
    pause: Duration,
    how: BackgroundSync,
) -> std::io::Result<()> {
    let vfs = StdVfs::shared();
    let path = dir.join("walsync-background.dat");
    let chunk = vec![0x5Au8; 64 << 10];
    let file = vfs.open(&path, OpenMode::CreateOrOpen)?;
    file.set_len(0)?;
    while !stop.load(Ordering::SeqCst) {
        let (mut written, mut since_hint) = (0usize, 0usize);
        while written < burst && !stop.load(Ordering::SeqCst) {
            file.write_at(&chunk, written as u64)?;
            written += chunk.len();
            since_hint += chunk.len();
            if since_hint >= 1 << 20 {
                match how {
                    BackgroundSync::EveryMiB => file.sync_data()?,
                    BackgroundSync::AtEnd => {}
                    BackgroundSync::WritebackEveryMiB => {
                        file.start_writeback((written - since_hint) as u64, since_hint as u64)?
                    }
                    BackgroundSync::BlockingWritebackEveryMiB => {
                        let (start, len) = ((written - since_hint) as u64, since_hint as u64);
                        file.start_writeback(start, len)?;
                        file.wait_writeback(start, len)?;
                    }
                    BackgroundSync::PacedWritebackEveryMiB => {
                        let (start, len) = ((written - since_hint) as u64, since_hint as u64);
                        file.start_writeback(start, len)?;
                        if start >= len {
                            file.wait_writeback(start - len, len)?;
                        }
                    }
                }
                since_hint = 0;
            }
        }
        file.sync_data()?;
        // Truncate so that the file does not grow for the whole run.
        file.set_len(0)?;
        std::thread::sleep(pause);
    }
    drop(file);
    vfs.remove_file(&path)?;
    Ok(())
}

/// The row of one run, in the shape of the other benchmark rows.
pub fn row(variant: Variant, rep: usize, p: &Params, lat: &Lat) -> Report {
    let mut rep_row = Report::default();
    rep_row.str("bench", "walsync");
    rep_row.str("variant", variant.name());
    rep_row.str("background", if p.background { "true" } else { "false" });
    if p.background {
        rep_row.str("background_sync", p.background_sync.name());
    }
    rep_row.num("rep", rep as f64);
    rep_row.num("iterations", p.iterations as f64);
    rep_row.num("record_bytes", p.record as f64);
    let load = load_average();
    rep_row.num("load1", load[0]);
    rep_row.num("load5", load[1]);
    rep_row.num("load15", load[2]);
    lat.report(&mut rep_row, "fdatasync");
    rep_row
}

/// Lever 5 of `docs/testing/f1-decision-cond1.md` §3: the sync of a growing
/// file (the default WAL) and of a pre-written one, with no background
/// writes and with each way of bounding the dirty data of a background burst
/// ([`BackgroundSync`]), interleaved within each repetition.
pub fn suite_writeback(p: &mut Params, reps: usize, out: &Path) -> std::io::Result<()> {
    let mut sink = OpenOptions::new().create(true).append(true).open(out)?;
    for rep in 0..reps {
        for variant in [Variant::Grow, Variant::Prewritten] {
            p.background = false;
            let lat = run(variant, p)?;
            let r = row(variant, rep, p, &lat);
            writeln!(sink, "{}", r.to_json())?;
            eprintln!("{}", r.to_json());
            p.background = true;
            for how in p.modes.clone() {
                p.background_sync = how;
                let lat = run(variant, p)?;
                let r = row(variant, rep, p, &lat);
                writeln!(sink, "{}", r.to_json())?;
                eprintln!("{}", r.to_json());
            }
        }
    }
    Ok(())
}

/// Runs every variant `reps` times, interleaved within each repetition, with
/// and without background writes, appending the rows to `out`.
pub fn suite(p: &mut Params, reps: usize, out: &Path) -> std::io::Result<()> {
    let mut sink = OpenOptions::new().create(true).append(true).open(out)?;
    for rep in 0..reps {
        for background in [false, true] {
            p.background = background;
            for variant in Variant::ALL {
                let lat = run(variant, p)?;
                let r = row(variant, rep, p, &lat);
                writeln!(sink, "{}", r.to_json())?;
                eprintln!("{}", r.to_json());
            }
        }
    }
    Ok(())
}
