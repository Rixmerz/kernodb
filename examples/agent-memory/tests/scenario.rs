//! The agent-memory scenario end to end, through the public API only.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_memory::{AgentMemory, Embedder, Error, OpenReport, VectorStatus};
use kerno_units::{DerivedState, FixedClock, Timestamp};

const T0: i64 = 1_700_000_000_000_000;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("agent-memory-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn open(dir: &Path) -> (AgentMemory, OpenReport) {
    let clock = Arc::new(FixedClock::stepping(Timestamp(T0), 1_000_000));
    AgentMemory::open(dir, clock, Embedder::default()).expect("open")
}

fn store(mem: &AgentMemory, text: &str) -> u64 {
    mem.remember(text, &["t".to_owned()], Some(Embedder::default().embed(text.as_bytes()))).expect("remember")
}

fn recall(mem: &AgentMemory, query: &str, k: usize) -> Vec<u64> {
    let hits = mem.recall_similar(&Embedder::default().embed(query.as_bytes()), k).expect("recall");
    hits.iter().map(|h| h.memory.id).collect()
}

#[test]
fn full_scenario_with_close_and_reopen() {
    let dir = scratch("full");
    let embedder = Embedder::default();
    let (mem, first) = open(&dir);
    assert_eq!(first, OpenReport::default());

    // Store four memories, each with a caller-supplied vector.
    let a = store(&mem, "the deploy script needs the staging token");
    let b = store(&mem, "alice prefers short answers in the morning");
    let c = store(&mem, "the staging token rotates every friday");
    let d = store(&mem, "bob asked about the quarterly report");
    assert!([a, b, c, d].windows(2).all(|w| w[0] < w[1]));
    assert_eq!(mem.get(a).unwrap().tags, ["t"]);
    assert_eq!(mem.vector(a).unwrap().unwrap(), embedder.embed(b"the deploy script needs the staging token"));

    // Recall by vector neighbourhood: the two staging memories come first.
    let near: std::collections::BTreeSet<u64> = recall(&mem, "staging token", 2).into_iter().collect();
    assert_eq!(near, [a, c].into());

    // Recall by time: the window and the newest.
    let (ta, tc) = (mem.get(a).unwrap().created_at, mem.get(c).unwrap().created_at);
    let ids = |v: Vec<agent_memory::Memory>| v.iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(ids(mem.recall_between(ta, tc).unwrap()), [a, b, c]);
    assert_eq!(ids(mem.recent(2).unwrap()), [d, c]);

    // Correct a memory: a new revision, its vector reported stale and not served.
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Ready);
    assert_eq!(mem.correct(a, "the deploy script needs the production token").unwrap(), 2);
    assert_eq!(mem.get(a).unwrap().revision, 2);
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Stale);
    assert_eq!(mem.vector(a).unwrap(), None);
    assert!(!recall(&mem, "deploy script production token", 9).contains(&a));

    // Rebuild: the vector is the stand-in's vector of the new text.
    mem.rebuild_vector(a).unwrap();
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Ready);
    assert_eq!(mem.vector(a).unwrap().unwrap(), embedder.embed(b"the deploy script needs the production token"));
    assert_eq!(recall(&mem, "deploy script production token", 1), [a]);

    // Summarise two memories; the originals stay readable.
    let s = mem.summarize(&[a, c]).unwrap();
    let info = &mem.summaries()[s];
    assert_eq!((info.sources.clone(), info.state), (vec![a, c], DerivedState::Ready));
    assert_eq!(info.text.as_deref(), Some("the deploy script needs \nthe staging token rotate"));
    assert_eq!(mem.get(a).unwrap().text, "the deploy script needs the production token");
    assert_eq!(mem.get(c).unwrap().text, "the staging token rotates every friday");

    // Forget one source: its vector and the summary are gone, the other source is not.
    let vector_of_c = mem.vector_id(c).unwrap().unwrap();
    assert_eq!(mem.derived_state(&vector_of_c), Some(DerivedState::Ready));
    mem.forget(c).unwrap();
    assert_eq!(mem.derived_state(&vector_of_c), None);
    assert!(mem.summaries().is_empty());
    assert!(matches!(mem.get(c), Err(Error::NotFound(n)) if n == c));
    assert!(matches!(mem.vector_status(c), Err(Error::NotFound(_))));
    assert_eq!(mem.get(a).unwrap().revision, 2);
    assert!(!recall(&mem, "staging token", 9).contains(&c));
    mem.close().unwrap();

    // Reopen: everything is as it was left, nothing is demoted, nothing is found wrong.
    let (mem, report) = open(&dir);
    assert_eq!(report, OpenReport { vectors_served: 3, ..OpenReport::default() });
    assert_eq!(mem.all().unwrap().iter().map(|m| m.id).collect::<Vec<_>>(), [a, b, d]);
    assert_eq!(mem.vector(a).unwrap().unwrap(), embedder.embed(b"the deploy script needs the production token"));
    assert_eq!(recall(&mem, "deploy script production token", 1), [a]);
    assert!(matches!(mem.get(c), Err(Error::NotFound(_))));
    assert!(mem.summaries().is_empty());

    // New memories do not reuse a number, even one that was forgotten.
    let e = store(&mem, "carol joined the team on monday");
    assert!(e > d);
    mem.close().unwrap();
}

#[test]
fn stale_vector_survives_reopen_and_is_rebuilt_after_it() {
    let dir = scratch("stale");
    let (mem, _) = open(&dir);
    let a = store(&mem, "first draft of the plan");
    mem.correct(a, "second draft of the plan").unwrap();
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Stale);
    mem.close().unwrap();

    let (mem, report) = open(&dir);
    assert_eq!(report.vectors_served, 0);
    assert_eq!(report.findings, 0);
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Stale);
    assert!(recall(&mem, "plan", 5).is_empty());
    mem.rebuild_vector(a).unwrap();
    assert_eq!(mem.vector(a).unwrap().unwrap(), Embedder::default().embed(b"second draft of the plan"));
    mem.close().unwrap();

    // The rebuilt vector is what the catalog remembers.
    let (mem, report) = open(&dir);
    assert_eq!(report.vectors_served, 1);
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Ready);
    mem.close().unwrap();
}

#[test]
fn summaries_are_rebuilt_on_reopen_and_follow_their_sources() {
    let dir = scratch("summary");
    let (mem, _) = open(&dir);
    let a = store(&mem, "alpha one two three");
    let b = store(&mem, "beta four five six");
    mem.summarize(&[b, a]).unwrap();
    assert!(mem.summarize(&[a, 999]).is_err());
    mem.close().unwrap();

    let (mem, report) = open(&dir);
    assert_eq!(report.summaries_rebuilt, 1);
    assert_eq!(mem.summaries()[0].text.as_deref(), Some("alpha one two three\nbeta four five six"));

    // Correcting a source makes the summary stale; refreshing rebuilds it.
    mem.correct(b, "beta seven eight nine").unwrap();
    assert_eq!(mem.summaries()[0].state, DerivedState::Stale);
    assert_eq!(mem.summaries()[0].text, None);
    assert_eq!(mem.get(b).unwrap().text, "beta seven eight nine");
    assert_eq!(mem.refresh_summaries().unwrap(), 1);
    assert_eq!(mem.summaries()[0].text.as_deref(), Some("alpha one two three\nbeta seven eight nine"));

    // Forgetting a source after a reopen removes the summary for good.
    mem.forget(a).unwrap();
    assert!(mem.summaries().is_empty());
    mem.close().unwrap();
    let (mem, report) = open(&dir);
    assert_eq!(report.summaries_rebuilt, 0);
    assert!(mem.summaries().is_empty());
    mem.close().unwrap();
}

#[test]
fn attach_a_vector_later_and_replace_a_stale_one() {
    let dir = scratch("attach");
    let (mem, _) = open(&dir);
    let a = mem.remember("no vector yet", &[], None).unwrap();
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::None);
    assert!(recall(&mem, "vector", 5).is_empty());

    let mut v = vec![0.0; 64];
    v[3] = 1.0;
    mem.attach_vector(a, v.clone()).unwrap();
    assert_eq!(mem.vector(a).unwrap(), Some(v.clone()));
    assert!(matches!(mem.attach_vector(a, v.clone()), Err(Error::Invalid(_))));
    assert_eq!(recall(&mem, "anything", 1).len(), 1);

    // After a correction the caller may supply the new vector itself.
    mem.correct(a, "now it has one").unwrap();
    let mut w = vec![0.0; 64];
    w[5] = 2.0;
    mem.attach_vector(a, w.clone()).unwrap();
    assert_eq!(mem.vector(a).unwrap(), Some(w));
    assert_eq!(mem.vector_status(a).unwrap(), VectorStatus::Ready);
    mem.close().unwrap();
}

#[test]
fn bad_input_is_refused() {
    let dir = scratch("bad");
    let (mem, _) = open(&dir);
    assert!(matches!(mem.remember("x", &["a\nb".to_owned()], None), Err(Error::Invalid(_))));
    assert!(matches!(mem.remember("x", &[String::new()], None), Err(Error::Invalid(_))));
    // A vector with a NaN is refused by the store and nothing is written.
    assert!(mem.remember("x", &[], Some(vec![f32::NAN; 64])).is_err());
    assert!(mem.all().unwrap().is_empty());
    assert!(matches!(mem.correct(42, "x"), Err(Error::NotFound(42))));
    assert!(matches!(mem.forget(42), Err(Error::NotFound(42))));
    mem.close().unwrap();
}

fn files_containing(dir: &Path, needle: &[u8]) -> Vec<String> {
    let mut hits = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if fs::read(&path).unwrap().windows(needle.len()).any(|w| w == needle) {
            hits.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    hits
}

/// Characterisation of FRICTION.md item 4: forgetting is logical. If the
/// library ever offers physical erasure through `DerivedDb`, this test is the
/// one to turn around.
#[test]
fn forgotten_text_stays_in_the_files_after_retention() {
    let dir = scratch("erase");
    let (mem, _) = open(&dir);
    let a = mem.remember("zzmarkerzz must not survive", &[], None).unwrap();
    mem.forget(a).unwrap();
    assert!(mem.prune_history().unwrap() > 0);
    mem.close().unwrap();
    assert!(!files_containing(&dir, b"zzmarkerzz").is_empty());
}
