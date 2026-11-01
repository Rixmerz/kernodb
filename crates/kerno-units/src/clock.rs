//! Injected time and identifiers (`etapa3-design.md` §6.7).
//!
//! The layer reads time only through [`Clock`] and mints unit ids only
//! through [`UnitIdSource`]. The only code in the crate that touches
//! `std::time` or a source of randomness is [`SystemClock`] and
//! [`UuidV7Source::new`]; tests use [`FixedClock`], [`CounterIds`] or a
//! seeded [`UuidV7Source::with_seed`], which are fully reproducible.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use crate::types::{Timestamp, UnitId};

/// Source of descriptive timestamps (`created_at`, `updated_at`). Ordering
/// never depends on it (I-26), so it may go backwards.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Source of new unit ids.
pub trait UnitIdSource: Send + Sync {
    fn next_id(&self) -> UnitId;
}

/// Wall-clock time from the operating system.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let micros = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_micros()).unwrap_or(i64::MAX),
            Err(e) => i64::try_from(e.duration().as_micros()).map_or(i64::MIN, |m| -m),
        };
        Timestamp(micros)
    }
}

/// A clock that returns a set time, then advances by a fixed step after each
/// read (step 0: always the same time). Tests may set it to any value,
/// including backwards.
#[derive(Debug)]
pub struct FixedClock {
    now: AtomicI64,
    step: i64,
}

impl FixedClock {
    pub fn new(start: Timestamp) -> Self {
        Self::stepping(start, 0)
    }

    pub fn stepping(start: Timestamp, step_micros: i64) -> Self {
        FixedClock { now: AtomicI64::new(start.0), step: step_micros }
    }

    pub fn set(&self, t: Timestamp) {
        self.now.store(t.0, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        let step = self.step;
        // `fetch_add` wraps on overflow, like `wrapping_add`.
        let prev = self.now.fetch_add(step, Ordering::SeqCst);
        Timestamp(prev)
    }
}

/// Ids `00…00 ‖ n` (big-endian `n` in the last 8 bytes) for `n = start, start+1, …`.
/// They sort in the order they were minted.
#[derive(Debug)]
pub struct CounterIds {
    next: AtomicU64,
}

impl CounterIds {
    pub fn new(start: u64) -> Self {
        CounterIds { next: AtomicU64::new(start) }
    }
}

impl UnitIdSource for CounterIds {
    fn next_id(&self) -> UnitId {
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let mut id = [0u8; 16];
        id[8..].copy_from_slice(&n.to_be_bytes());
        UnitId(id)
    }
}

/// UUIDv7 (RFC 9562 §5.7) built from a [`Clock`] and a 64-bit entropy
/// stream (SplitMix64 over a seed).
///
/// Layout: 48-bit Unix milliseconds ‖ version `7` ‖ 12-bit `rand_a` ‖
/// variant `10` ‖ 62-bit `rand_b`. `rand_a` is a counter (RFC 9562 §6.2,
/// method 1) reseeded with random bits in the lower half whenever the
/// millisecond advances, so ids from one source are strictly increasing
/// even when the clock stalls or goes backwards: in that case the last
/// millisecond is reused and the counter incremented; on counter overflow
/// the millisecond is advanced by one.
pub struct UuidV7Source {
    clock: Arc<dyn Clock>,
    state: Mutex<V7State>,
}

struct V7State {
    last_ms: u64,
    counter: u16,
    rng: u64,
}

const MS_MASK: u64 = (1 << 48) - 1;
const COUNTER_MAX: u16 = 0x0FFF;

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl UuidV7Source {
    /// The default: [`SystemClock`] and a seed from the standard library's
    /// per-process random hasher keys (no extra dependency).
    pub fn new() -> Self {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(SystemClock.now().0 as u64);
        Self::with_seed(Arc::new(SystemClock), h.finish())
    }

    /// Deterministic: the same clock readings and seed give the same ids.
    pub fn with_seed(clock: Arc<dyn Clock>, seed: u64) -> Self {
        UuidV7Source { clock, state: Mutex::new(V7State { last_ms: 0, counter: 0, rng: seed }) }
    }
}

impl Default for UuidV7Source {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for UuidV7Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UuidV7Source")
    }
}

impl UnitIdSource for UuidV7Source {
    fn next_id(&self) -> UnitId {
        // Pre-epoch times clamp to 0; times past 2^48 ms wrap per RFC 9562.
        let now_ms = (self.clock.now().0.max(0) / 1000) as u64 & MS_MASK;
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if now_ms > s.last_ms {
            s.last_ms = now_ms;
            // Start low in the counter space so that many ids fit in one ms.
            s.counter = (splitmix64(&mut s.rng) & 0x07FF) as u16;
        } else if s.counter < COUNTER_MAX {
            s.counter += 1;
        } else {
            s.last_ms = (s.last_ms + 1) & MS_MASK;
            s.counter = 0;
        }
        let rand_b = splitmix64(&mut s.rng);
        let mut b = [0u8; 16];
        b[..6].copy_from_slice(&s.last_ms.to_be_bytes()[2..]);
        b[6] = 0x70 | (s.counter >> 8) as u8;
        b[7] = s.counter as u8;
        b[8..].copy_from_slice(&rand_b.to_be_bytes());
        b[8] = (b[8] & 0x3F) | 0x80;
        UnitId(b)
    }
}
