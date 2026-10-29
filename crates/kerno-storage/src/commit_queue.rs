//! The commit queue: the synchronization protocol of group commit
//! (phase1-design.md §5.6), with no I/O.
//!
//! Every committing thread builds a [`Request`], [`enqueue`](CommitQueue::enqueue)s
//! it and either becomes the **leader** (the queue was empty) or waits on the
//! condition variable of its own request. The front request is always the
//! leader or is about to be woken as one; the leader takes a FIFO group
//! ([`take_group`](CommitQueue::take_group)), does the work for all of it
//! outside this module, and hands each member its result with
//! [`complete`](CommitQueue::complete), which also promotes the next leader.
//! Exactly one group is in flight at a time and a leader leads exactly one
//! group, itself included.
//!
//! * **Failure.** `complete` with a poison message fails the group with the
//!   errors the leader built, fails every queued request that was not in the
//!   group with [`Error::Poisoned`] and rejects every later `enqueue`: nobody
//!   waits for a leader that will never come.
//! * **Backpressure.** `enqueue` waits while more than `max_queued_bytes` are
//!   queued or in flight, so the memory behind the queue is bounded by that
//!   limit plus one request per blocked thread.
//! * **Drain.** [`drain`](CommitQueue::drain) waits until nothing is queued
//!   or in flight. A holder of the write slot (`txn_gate`, which committers
//!   take only around `enqueue`) that drained the queue has the pipeline to
//!   itself.
//!
//! The protocol is written against [`crate::sync`], so the models at the end
//! of this file run it under `loom` (`RUSTFLAGS="--cfg kerno_loom" cargo test
//! -p kerno-storage --release --lib commit_queue`) and the same scenarios run
//! as ordinary threaded tests otherwise.
//!
//! Lock order: `txn_gate` → queue state → request slot. No lock is held
//! across the I/O of a group.

use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use kerno_core::{Error, Result};

use crate::sync::{Arc, Condvar, Mutex};

/// Sizes of a group and of the whole queue (`GroupCommitConfig`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QueueLimits {
    /// A group stops growing when the next request would take it past this
    /// many bytes (the first request is always taken, whatever its size).
    pub max_group_bytes: usize,
    /// A group holds at most this many requests (at least 1).
    pub max_group_commits: usize,
    /// `enqueue` waits while more than this many bytes are queued.
    pub max_queued_bytes: usize,
}

enum Slot {
    /// Queued behind another group.
    Waiting,
    /// Promoted: this request leads the next group.
    Leader,
    Done(Result<u64>),
    /// The result was taken by [`Request::wait`].
    Taken,
}

/// One queued commit or `sync()`. The payload is immutable once queued; the
/// leader reads it while the owner waits.
pub(crate) struct Request<P> {
    payload: P,
    bytes: usize,
    enqueued: Instant,
    slot: Mutex<Slot>,
    cv: Condvar,
}

/// Why [`Request::wait`] returned.
pub(crate) enum Wake {
    /// This request is now the front: lead the next group.
    Lead,
    /// The request completed with this result.
    Done(Result<u64>),
}

impl<P> Request<P> {
    /// A request that counts `bytes` against the queue limits.
    pub fn new(payload: P, bytes: usize) -> Arc<Self> {
        Arc::new(Request {
            payload,
            bytes,
            enqueued: Instant::now(),
            slot: Mutex::new(Slot::Waiting),
            cv: Condvar::new(),
        })
    }

    pub fn payload(&self) -> &P {
        &self.payload
    }

    pub fn enqueued(&self) -> Instant {
        self.enqueued
    }

    /// Blocks until the request is promoted to leader or completed. A leader
    /// calls it again after its group completed to collect its own result.
    pub fn wait(&self) -> Wake {
        let mut slot = self.slot.lock();
        loop {
            match &*slot {
                Slot::Waiting => self.cv.wait(&mut slot),
                Slot::Leader => return Wake::Lead,
                Slot::Done(_) => match std::mem::replace(&mut *slot, Slot::Taken) {
                    Slot::Done(r) => return Wake::Done(r),
                    _ => unreachable!("matched Done above"),
                },
                Slot::Taken => unreachable!("the result of a request is collected once"),
            }
        }
    }

    fn promote(&self) {
        let mut slot = self.slot.lock();
        if matches!(*slot, Slot::Waiting) {
            *slot = Slot::Leader;
        }
        drop(slot);
        self.cv.notify_one();
    }

    fn finish(&self, result: Result<u64>) {
        *self.slot.lock() = Slot::Done(result);
        self.cv.notify_one();
    }
}

struct State<P> {
    /// FIFO. The first `in_flight` requests are the group being processed.
    requests: VecDeque<Arc<Request<P>>>,
    in_flight: usize,
    queued_bytes: usize,
    poisoned: Option<String>,
}

/// What [`CommitQueue::enqueue`] decided.
pub(crate) struct Enqueued {
    /// Whether the caller leads the next group or waits.
    pub leader: bool,
    /// Bytes queued or in flight after this request joined.
    pub queued_bytes: usize,
}

pub(crate) struct CommitQueue<P> {
    state: Mutex<State<P>>,
    /// Signalled when bytes leave the queue, when it empties and on poison:
    /// what backpressure and drain wait for.
    room: Condvar,
    limits: QueueLimits,
}

impl<P> CommitQueue<P> {
    pub fn new(limits: QueueLimits) -> Self {
        CommitQueue {
            state: Mutex::new(State { requests: VecDeque::new(), in_flight: 0, queued_bytes: 0, poisoned: None }),
            room: Condvar::new(),
            limits,
        }
    }

    /// Queues `req` after waiting for room. `Err(Poisoned)` if the queue was
    /// poisoned before or while waiting (the request is then not queued).
    pub fn enqueue(&self, req: &Arc<Request<P>>) -> Result<Enqueued> {
        let mut st = self.state.lock();
        loop {
            if let Some(msg) = &st.poisoned {
                return Err(Error::Poisoned(msg.clone()));
            }
            if st.queued_bytes <= self.limits.max_queued_bytes {
                break;
            }
            self.room.wait(&mut st);
        }
        st.queued_bytes += req.bytes;
        st.requests.push_back(req.clone());
        Ok(Enqueued { leader: st.requests.len() == 1, queued_bytes: st.queued_bytes })
    }

    /// The group the calling leader processes: the front of the queue, in
    /// order, as far as the limits allow (the leader itself first). Call it
    /// once per leadership.
    pub fn take_group(&self) -> Vec<Arc<Request<P>>> {
        let mut st = self.state.lock();
        debug_assert_eq!(st.in_flight, 0, "one group in flight");
        let (mut n, mut bytes) = (0, 0usize);
        for r in &st.requests {
            if n > 0
                && (n >= self.limits.max_group_commits.max(1)
                    || bytes.saturating_add(r.bytes) > self.limits.max_group_bytes)
            {
                break;
            }
            bytes = bytes.saturating_add(r.bytes);
            n += 1;
        }
        assert!(n > 0, "take_group on an empty queue: only a leader calls it");
        st.in_flight = n;
        st.requests.iter().take(n).cloned().collect()
    }

    /// Hands every member of `group` its result (`results[i]` for
    /// `group[i]`) and passes leadership on to the next request.
    ///
    /// With `poison`, the queue fails for good instead: the requests queued
    /// behind the group get [`Error::Poisoned`], the waiting `enqueue`s
    /// return it and so do all later ones.
    pub fn complete(&self, group: &[Arc<Request<P>>], results: Vec<Result<u64>>, poison: Option<String>) {
        assert_eq!(group.len(), results.len(), "one result per member");
        let mut st = self.state.lock();
        debug_assert_eq!(st.in_flight, group.len(), "the group in flight");
        for (member, result) in group.iter().zip(results) {
            let front = st.requests.pop_front().expect("a member is still queued");
            debug_assert!(Arc::ptr_eq(&front, member), "the group is the front of the queue");
            st.queued_bytes -= front.bytes;
            front.finish(result);
        }
        st.in_flight = 0;
        if let Some(msg) = poison {
            for r in st.requests.drain(..) {
                r.finish(Err(Error::Poisoned(msg.clone())));
            }
            st.queued_bytes = 0;
            st.poisoned = Some(msg);
        } else if let Some(next) = st.requests.front() {
            next.promote();
        }
        drop(st);
        self.room.notify_all();
    }

    /// Waits until nothing is queued or in flight (`begin_write`).
    pub fn drain(&self) {
        let mut st = self.state.lock();
        while !st.requests.is_empty() {
            self.room.wait(&mut st);
        }
    }
}

/// A new error equal to `e` (`Error` is not `Clone`): one per member of a
/// failed group.
pub(crate) fn replicate(e: &Error) -> Error {
    match e {
        Error::Io(e) => Error::Io(io::Error::new(e.kind(), e.to_string())),
        Error::Corruption(m) => Error::Corruption(m.clone()),
        Error::Locked => Error::Locked,
        Error::InvalidArgument(m) => Error::InvalidArgument(m.clone()),
        Error::Poisoned(m) => Error::Poisoned(m.clone()),
        Error::Incompatible(m) => Error::Incompatible(m.clone()),
        Error::ReadOnly(m) => Error::ReadOnly(m.clone()),
        // `Error` is non-exhaustive: keep the text of a variant added later.
        other => Error::Poisoned(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(kerno_loom)]
    use loom::thread;
    #[cfg(not(kerno_loom))]
    use std::thread;

    const LIMITS: QueueLimits =
        QueueLimits { max_group_bytes: 1 << 20, max_group_commits: 1024, max_queued_bytes: 1 << 20 };

    /// What the leader body of the scenarios shares: the "writer" (the last
    /// sequence assigned), the order payloads were processed in, and the
    /// counters the drain scenario checks.
    struct Shared {
        queue: CommitQueue<u32>,
        writer: Mutex<u64>,
        order: Mutex<Vec<u32>>,
        gate: Mutex<()>,
        submitted: Mutex<u32>,
        published: Mutex<u32>,
        /// Payload of the request whose group fails, if any.
        fail_on: Option<u32>,
    }

    impl Shared {
        fn new(limits: QueueLimits, fail_on: Option<u32>) -> Arc<Self> {
            Arc::new(Shared {
                queue: CommitQueue::new(limits),
                writer: Mutex::new(0),
                order: Mutex::new(Vec::new()),
                gate: Mutex::new(()),
                submitted: Mutex::new(0),
                published: Mutex::new(0),
                fail_on,
            })
        }

        /// The commit protocol of `KernoDb` with the I/O replaced by a
        /// counter: gate around `enqueue`, leader or follower, one group per
        /// leadership.
        fn commit(&self, payload: u32, bytes: usize) -> Result<u64> {
            let req = Request::new(payload, bytes);
            let enq = {
                let _gate = self.gate.lock();
                let enq = self.queue.enqueue(&req)?;
                *self.submitted.lock() += 1;
                enq
            };
            if !enq.leader {
                match req.wait() {
                    Wake::Done(r) => return r,
                    Wake::Lead => {}
                }
            }
            self.lead();
            match req.wait() {
                Wake::Done(r) => r,
                Wake::Lead => unreachable!("a completed leader is not promoted again"),
            }
        }

        fn lead(&self) {
            let mut writer = self.writer.lock();
            let group = self.queue.take_group();
            let failing = self.fail_on.is_some_and(|p| group.iter().any(|r| *r.payload() == p));
            if failing {
                drop(writer);
                let results = group.iter().map(|_| Err(Error::Io(io::Error::other("injected")))).collect();
                self.queue.complete(&group, results, Some("injected".into()));
                return;
            }
            let mut results = Vec::new();
            for r in &group {
                *writer += 1;
                self.order.lock().push(*r.payload());
                results.push(Ok(*writer));
            }
            *self.published.lock() += group.len() as u32;
            drop(writer);
            self.queue.complete(&group, results, None);
        }
    }

    /// Three committers: every commit gets a distinct sequence, sequences
    /// follow the order the payloads were processed in, and nobody hangs.
    fn scenario_leader_handoff() {
        let shared = Shared::new(LIMITS, None);
        let handles: Vec<_> = (1..=3u32)
            .map(|p| {
                let s = shared.clone();
                thread::spawn(move || (p, s.commit(p, 1)))
            })
            .collect();
        let mut seqs: Vec<(u32, u64)> = handles
            .into_iter()
            .map(|h| {
                let (p, r) = h.join().unwrap();
                (p, r.expect("no failure injected"))
            })
            .collect();
        seqs.sort_by_key(|&(_, s)| s);
        let by_seq: Vec<u32> = seqs.iter().map(|&(p, _)| p).collect();
        assert_eq!(by_seq, *shared.order.lock(), "sequences follow the processing order");
        let only: Vec<u64> = seqs.iter().map(|&(_, s)| s).collect();
        assert_eq!(only, [1, 2, 3]);
        assert!(shared.queue.state.lock().requests.is_empty());
    }

    /// A failing group poisons: its members get `Io`, everything queued
    /// behind it and everything later gets `Poisoned`, and nobody hangs.
    fn scenario_poison_fanout() {
        // Payload 1 fails whatever group it is in; the other two are either
        // in its group (Io) or behind it (Poisoned), or arrive after it
        // (Poisoned from `enqueue`), or lead a group of their own before it
        // (Ok, with a sequence below the failing group's).
        let shared = Shared::new(LIMITS, Some(1));
        let handles: Vec<_> = (1..=3u32)
            .map(|p| {
                let s = shared.clone();
                thread::spawn(move || (p, s.commit(p, 1)))
            })
            .collect();
        let mut failed_seen = false;
        for h in handles {
            let (p, r) = h.join().unwrap();
            match r {
                Ok(_) => assert_ne!(p, 1, "the failing commit cannot succeed"),
                Err(Error::Io(_)) | Err(Error::Poisoned(_)) => failed_seen = true,
                Err(e) => panic!("unexpected error {e:?}"),
            }
            if p == 1 {
                assert!(matches!(r, Err(Error::Io(_))), "the failing request is in the failing group");
            }
        }
        assert!(failed_seen);
        // The queue stays closed.
        assert!(matches!(shared.commit(9, 1), Err(Error::Poisoned(_))));
        assert!(shared.queue.state.lock().requests.is_empty());
    }

    /// A holder of the gate that drained the queue sees every submitted
    /// commit published, and no commit is submitted while it holds the gate.
    fn scenario_txn_gate_drain() {
        let shared = Shared::new(LIMITS, None);
        let committers: Vec<_> = (1..=2u32)
            .map(|p| {
                let s = shared.clone();
                thread::spawn(move || s.commit(p, 1).expect("no failure injected"))
            })
            .collect();
        let txn = {
            let s = shared.clone();
            thread::spawn(move || {
                let _gate = s.gate.lock();
                s.queue.drain();
                let (submitted, published) = (*s.submitted.lock(), *s.published.lock());
                assert_eq!(submitted, published, "drained under the gate: everything submitted is published");
                assert!(s.queue.state.lock().requests.is_empty());
            })
        };
        txn.join().unwrap();
        for c in committers {
            c.join().unwrap();
        }
        assert_eq!(*shared.published.lock(), 2);
    }

    /// With a byte limit smaller than a request, a second committer waits for
    /// the first group to leave the queue; nobody hangs, and with `fail` the
    /// waiter is released by the poison instead.
    fn scenario_backpressure(fail: bool) {
        let limits = QueueLimits { max_group_bytes: 1, max_group_commits: 1, max_queued_bytes: 1 };
        let shared = Shared::new(limits, fail.then_some(1));
        let handles: Vec<_> = (1..=3u32)
            .map(|p| {
                let s = shared.clone();
                thread::spawn(move || (p, s.commit(p, 2)))
            })
            .collect();
        let mut ok = 0;
        for h in handles {
            let (p, r) = h.join().unwrap();
            match r {
                Ok(_) => ok += 1,
                Err(Error::Io(_)) => assert!(fail && p == 1),
                Err(Error::Poisoned(_)) => assert!(fail),
                Err(e) => panic!("unexpected error {e:?}"),
            }
        }
        if !fail {
            assert_eq!(ok, 3);
            // One request per group: the limit of 1 commit held.
            assert_eq!(shared.order.lock().len(), 3);
        }
        assert!(shared.queue.state.lock().requests.is_empty());
        assert_eq!(shared.queue.state.lock().queued_bytes, 0);
    }

    #[cfg(kerno_loom)]
    fn model(f: impl Fn() + Sync + Send + 'static) {
        // Three threads with a mutex and two condvars per request explode
        // without a bound; 3 preemptions still cover every hand-off order.
        let mut b = loom::model::Builder::new();
        b.preemption_bound = Some(3);
        b.check(f);
    }

    #[cfg(kerno_loom)]
    #[test]
    fn loom_leader_handoff() {
        model(scenario_leader_handoff);
    }

    #[cfg(kerno_loom)]
    #[test]
    fn loom_poison_fanout() {
        model(scenario_poison_fanout);
    }

    #[cfg(kerno_loom)]
    #[test]
    fn loom_txn_gate_drain() {
        model(scenario_txn_gate_drain);
    }

    #[cfg(kerno_loom)]
    #[test]
    fn loom_backpressure() {
        model(|| scenario_backpressure(false));
        model(|| scenario_backpressure(true));
    }

    /// The same scenarios as ordinary threads, many times: a smoke test of
    /// the protocol on every build (the loom models are the exhaustive one).
    #[cfg(not(kerno_loom))]
    #[test]
    fn scenarios_hold_under_real_threads() {
        for _ in 0..300 {
            scenario_leader_handoff();
            scenario_poison_fanout();
            scenario_txn_gate_drain();
            scenario_backpressure(false);
            scenario_backpressure(true);
        }
    }

    #[cfg(not(kerno_loom))]
    mod single_threaded {
        use super::*;

        fn req(bytes: usize) -> Arc<Request<u32>> {
            Request::new(0, bytes)
        }

        #[test]
        fn group_is_the_fifo_prefix_within_the_limits() {
            let q =
                CommitQueue::new(QueueLimits { max_group_bytes: 10, max_group_commits: 3, max_queued_bytes: 1 << 20 });
            let rs: Vec<_> = [4, 4, 4, 1, 1].iter().map(|&b| req(b)).collect();
            for (i, r) in rs.iter().enumerate() {
                assert_eq!(q.enqueue(r).unwrap().leader, i == 0);
            }
            // 4 + 4 fits, the third 4 would make 12 > 10.
            let g = q.take_group();
            assert_eq!(g.len(), 2);
            assert!(Arc::ptr_eq(&g[0], &rs[0]) && Arc::ptr_eq(&g[1], &rs[1]));
            q.complete(&g, vec![Ok(1), Ok(2)], None);
            // The next leader is promoted; 4 + 1 + 1 hits the 3-request cap exactly.
            assert!(matches!(rs[2].wait(), Wake::Lead));
            let g = q.take_group();
            assert_eq!(g.len(), 3);
            q.complete(&g, vec![Ok(3), Ok(4), Ok(5)], None);
            assert!(matches!(rs[0].wait(), Wake::Done(Ok(1))));
            assert!(matches!(rs[4].wait(), Wake::Done(Ok(5))));
        }

        #[test]
        fn an_oversized_first_request_still_forms_a_group() {
            let q =
                CommitQueue::new(QueueLimits { max_group_bytes: 10, max_group_commits: 8, max_queued_bytes: 1 << 20 });
            let (big, small) = (req(100), req(1));
            q.enqueue(&big).unwrap();
            q.enqueue(&small).unwrap();
            let g = q.take_group();
            assert_eq!(g.len(), 1, "the big request goes alone");
            q.complete(&g, vec![Ok(1)], None);
            assert!(matches!(small.wait(), Wake::Lead));
        }

        #[test]
        fn poison_fails_the_group_and_everything_behind_it() {
            let q = CommitQueue::new(QueueLimits { max_group_commits: 1, ..LIMITS });
            let rs: Vec<_> = [7, 5, 3].iter().map(|&b| req(b)).collect();
            assert_eq!(q.enqueue(&rs[0]).unwrap().queued_bytes, 7);
            assert_eq!(q.enqueue(&rs[1]).unwrap().queued_bytes, 12);
            assert_eq!(q.enqueue(&rs[2]).unwrap().queued_bytes, 15);
            let g = q.take_group();
            assert_eq!(g.len(), 1);
            q.complete(&g, vec![Err(Error::Io(io::Error::other("boom")))], Some("boom".into()));
            assert!(matches!(rs[0].wait(), Wake::Done(Err(Error::Io(_)))));
            for r in &rs[1..] {
                assert!(matches!(r.wait(), Wake::Done(Err(Error::Poisoned(m))) if m == "boom"));
            }
            assert_eq!(q.state.lock().queued_bytes, 0);
            assert!(matches!(q.enqueue(&req(1)), Err(Error::Poisoned(m)) if m == "boom"));
        }

        #[test]
        fn replicate_keeps_the_variant_and_the_message() {
            let e = replicate(&Error::Io(io::Error::new(io::ErrorKind::StorageFull, "disk")));
            assert!(matches!(&e, Error::Io(io) if io.kind() == io::ErrorKind::StorageFull && io.to_string() == "disk"));
            assert!(matches!(replicate(&Error::Poisoned("p".into())), Error::Poisoned(m) if m == "p"));
            assert!(matches!(replicate(&Error::InvalidArgument("i".into())), Error::InvalidArgument(m) if m == "i"));
            assert!(matches!(replicate(&Error::Locked), Error::Locked));
        }
    }
}
