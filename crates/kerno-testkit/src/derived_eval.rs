//! The frozen evaluation set of the derived data and of the vector search
//! (Stage 8, E8-01; acta §6.1, decision D5).
//!
//! The set measures mechanics: the exact search over vectors, and the
//! registry of derived data that later packages add. It does not measure the
//! quality of any model. The vectors come from a function of [`SEED`]: each
//! topic has a centre, and its units are drawn around that centre, so the
//! search recovers the topic by construction. Nothing more is claimed.
//!
//! * [`Story::generate`] builds the corpus, the vectors and the queries from a
//!   seed. Identifiers are a seeded permutation of the creation order.
//! * Grades come from the story, not from any engine. A path of true links
//!   gives grade 3 at depth 1, 2 at depth 2 and 1 from depth 3. Named
//!   distractors are grade 0: the seed, the far end of a false link, and the
//!   vector neighbours that no link reaches.
//! * [`render_corpus`], [`render_vectors`] and [`render_queries`] write the
//!   three files, and [`parse_corpus`], [`parse_vectors`] and [`parse_queries`]
//!   read them back. The manifest is [`crate::assoc_eval::manifest_text`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::str::{FromStr, SplitWhitespace};

use crate::rng::Rng;

/// The seed of this set. It is not the seed of the Stage 7 set.
pub const SEED: u64 = 20_261_112;
const VECTOR_STREAM: u64 = 0xD8E1_0000_0000_0003;
const QUERY_STREAM: u64 = 0xD8E1_0000_0000_0002;
const PERMUTATION_STREAM: u64 = 0xD8E1_0000_0000_0004;

/// Dimension of every vector, units and queries alike.
pub const DIM: usize = 16;
/// Topics of the story. A topic is one chain of decisions that revise each other.
pub const TOPICS: usize = 10;
/// Cut of the measurements (acta §6.2).
pub const CUT: usize = 10;
/// Queries per class; acta §6.1 asks for at least 20.
pub const PER_CLASS: usize = 25;
/// Vector neighbours judged per vector query. Equal to the cut, so that the
/// top ten of the exact search is always judged (see [`judge_neighbours`]).
pub const VECTOR_NEIGHBOURS: usize = CUT;
const CENTRE_RANGE: i64 = 1_000;
/// Offset of a decision's walk from the previous one, per component.
const WALK_STEP: i64 = 40;
/// Noise around a decision's position, per component.
const DECISION_NOISE: i64 = 15;
/// Noise around the centre of a topic for the other units, per component.
const NOISE_RANGE: i64 = 400;
const UNITS_PER_SESSION: u32 = 20;
const MAX_DRAWS: usize = 10_000;
const WRITE: &str = "a String accepts every write";
const AGENTS: [&str; 3] = ["agent-alpha", "agent-beta", "agent-gamma"];
const IMPORTER: &str = "importer";
const CONSOLIDATOR: &str = "consolidator";
const IMPORT_SYSTEM: &str = "wiki-export";
const DEDUP_ORIGIN: &str = "dedup-v2";
const CONSOLIDATOR_ORIGIN: &str = "consolidator-v1";
/// The source each of a session's eight facts cites, before the per-session shuffle.
const SOURCE_SLOTS: [u32; 8] = [0, 1, 2, 3, 4, 0, 1, 2];
const TRUE_CONFIDENCE: (u64, u64) = (450, 950);
const FALSE_CONFIDENCE: (u64, u64) = (300, 800);

/// The two scales. Only [`Scale::Quality`] is frozen; [`Scale::Performance`]
/// is generated from the same seed for the 20 000-unit runs of E8-07.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Quality,
    Performance,
}

impl Scale {
    pub fn name(self) -> &'static str {
        match self {
            Scale::Quality => "quality",
            Scale::Performance => "performance",
        }
    }

    pub fn sessions(self) -> u32 {
        match self {
            Scale::Quality => 100,
            Scale::Performance => 1_000,
        }
    }

    fn from_name(name: &str) -> Option<Scale> {
        [Scale::Quality, Scale::Performance].into_iter().find(|s| s.name() == name)
    }
}

/// What a unit is in the story.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Session,
    Task,
    Decision,
    Fact,
    Source,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Session => "session",
            Kind::Task => "task",
            Kind::Decision => "decision",
            Kind::Fact => "fact",
            Kind::Source => "source",
        }
    }

    fn from_name(name: &str) -> Option<Kind> {
        [Kind::Session, Kind::Task, Kind::Decision, Kind::Fact, Kind::Source].into_iter().find(|k| k.name() == name)
    }
}

pub const IN_SESSION: &str = "in_session";
pub const REVISES: &str = "revises";
pub const SUPPORTS: &str = "supports";
pub const MENTIONS: &str = "mentions";
pub const CITES: &str = "cites";
pub const RELATED: &str = "related";
pub const SAME_SOURCE: &str = "same_source";
const RELATION_KINDS: [&str; 7] = [IN_SESSION, REVISES, SUPPORTS, MENTIONS, CITES, RELATED, SAME_SOURCE];
const PROVENANCE_CLASSES: [&str; 4] = ["declared", "imported", "algorithmic", "inferred"];

fn relation_kind(name: &str) -> Option<&'static str> {
    RELATION_KINDS.into_iter().find(|k| *k == name)
}

fn provenance_class(name: &str) -> Option<&'static str> {
    PROVENANCE_CLASSES.into_iter().find(|k| *k == name)
}

/// The provenance classes of ADR-0014. A `related` link is always `inferred`
/// and a `same_source` link is always `algorithmic`, as in the Stage 7 set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provenance {
    Declared,
    Imported(String),
    Algorithmic(String),
    /// The origin and the confidence in thousandths, from 0 to 1000.
    Inferred(String, u16),
}

impl Provenance {
    pub fn class(&self) -> &'static str {
        match self {
            Provenance::Declared => "declared",
            Provenance::Imported(_) => "imported",
            Provenance::Algorithmic(_) => "algorithmic",
            Provenance::Inferred(..) => "inferred",
        }
    }

    fn render(&self) -> String {
        match self {
            Provenance::Declared => "declared".to_owned(),
            Provenance::Imported(source) => format!("imported {source}"),
            Provenance::Algorithmic(origin) => format!("algorithmic {origin}"),
            Provenance::Inferred(origin, confidence) => format!("inferred {origin} {confidence}"),
        }
    }
}

/// A unit of the corpus. Its identifier is the permuted one, not the creation order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub id: u32,
    pub step: u64,
    pub kind: Kind,
    pub label: String,
}

/// A change set: the steps of the story, one per change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub step: u64,
    pub actor: String,
    pub source: String,
}

/// A link between two units, created at `step`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub step: u64,
    pub from: u32,
    pub to: u32,
    pub kind: &'static str,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corpus {
    pub seed: u64,
    pub scale: Scale,
    /// In creation order; the identifiers are permuted.
    pub units: Vec<Unit>,
    pub changes: Vec<Change>,
    pub relations: Vec<Relation>,
}

/// The four query classes of acta §6.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    VectorNeighbourhood,
    Temporal,
    Provenance,
    Combined,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::VectorNeighbourhood => "vector_neighbourhood",
            Class::Temporal => "temporal",
            Class::Provenance => "provenance",
            Class::Combined => "combined",
        }
    }

    fn from_name(name: &str) -> Option<Class> {
        [Class::VectorNeighbourhood, Class::Temporal, Class::Provenance, Class::Combined]
            .into_iter()
            .find(|c| c.name() == name)
    }
}

/// Which way a walk may follow a link, relative to the unit it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    Both,
}

impl Direction {
    pub fn name(self) -> &'static str {
        match self {
            Direction::Out => "out",
            Direction::In => "in",
            Direction::Both => "both",
        }
    }

    fn from_name(name: &str) -> Option<Direction> {
        [Direction::Out, Direction::In, Direction::Both].into_iter().find(|d| d.name() == name)
    }
}

/// A judged unit and its grade, 0 to 3. Grade 0 is a named distractor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Judgment {
    pub unit: u32,
    pub grade: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub id: String,
    pub class: Class,
    /// The unit the walk starts from.
    pub seed: u32,
    pub direction: Direction,
    /// The relation kinds the walk may follow. Never empty.
    pub kinds: Vec<&'static str>,
    pub depth: u8,
    /// Inclusive range of creation steps of the links, or none.
    pub window: Option<(u64, u64)>,
    /// The provenance classes the walk may follow; empty admits all.
    pub provenance: Vec<&'static str>,
    /// The query vector of a vector query.
    pub vector: Option<[i32; DIM]>,
    /// Sorted by unit, without repeats.
    pub expected: Vec<Judgment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuerySet {
    pub seed: u64,
    pub scale: Scale,
    pub queries: Vec<Query>,
}

/// The whole set: the corpus, one vector per unit (indexed by identifier), and the queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Story {
    pub corpus: Corpus,
    pub vectors: Vec<[i32; DIM]>,
    pub queries: QuerySet,
}

/// Draft units hold their creation index as the identifier until the permutation.
#[derive(Default)]
struct Draft {
    units: Vec<DraftUnit>,
    /// Each link with whether the story says it is true. Creation order.
    links: Vec<Link>,
    changes: Vec<Change>,
}

type Link = (Relation, bool);

struct DraftUnit {
    session: u32,
    topic: u8,
    step: u64,
    kind: Kind,
    label: String,
}

impl Draft {
    fn unit(&mut self, step: u64, session: u32, topic: u8, kind: Kind, label: String) {
        self.units.push(DraftUnit { session, topic, step, kind, label });
    }

    fn link(&mut self, step: u64, from: u32, to: u32, kind: &'static str, provenance: Provenance, truth: bool) {
        self.links.push((relation(step, from, to, kind, provenance), truth));
    }
}

fn relation(step: u64, from: u32, to: u32, kind: &'static str, provenance: Provenance) -> Relation {
    Relation { step, from, to, kind, provenance }
}

fn shuffle<T>(rng: &mut Rng, items: &mut [T]) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        items.swap(i, j);
    }
}

/// A decision of another topic, from an earlier session, chosen uniformly.
fn other_topic(decisions: &[(u32, u8)], topic: u8, rng: &mut Rng) -> Option<u32> {
    let others: Vec<u32> = decisions.iter().filter(|(_, t)| *t != topic).map(|(id, _)| *id).collect();
    if others.is_empty() {
        return None;
    }
    Some(others[rng.below(others.len() as u64) as usize])
}

/// Writes the story in creation order, one session after another. The three
/// change sets of a session are at steps `3s+1` (the agent), `3s+2` (the
/// importer) and `3s+3` (the consolidator), so the links of each step are
/// written after the units they join.
fn draft(sessions: u32, rng: &mut Rng) -> Draft {
    let mut d = Draft::default();
    let mut chains: Vec<Vec<u32>> = vec![Vec::new(); TOPICS];
    let mut decisions: Vec<(u32, u8)> = Vec::new();
    for s in 0..sessions {
        let topic = rng.below(TOPICS as u64) as u8;
        let a = 3 * u64::from(s) + 1;
        let b = a + 1;
        let c = a + 2;
        let agent = AGENTS[rng.below(AGENTS.len() as u64) as usize];
        d.changes.push(Change { step: a, actor: agent.to_owned(), source: format!("transcript/{s:04}") });
        d.changes.push(Change { step: b, actor: IMPORTER.to_owned(), source: format!("{IMPORT_SYSTEM}/{s:04}") });
        d.changes.push(Change { step: c, actor: CONSOLIDATOR.to_owned(), source: format!("job/consolidate/{s:04}") });

        // Creation index of this session's session unit; the rest follow it.
        let base = d.units.len() as u32;
        d.unit(a, s, topic, Kind::Session, format!("s{s:04}"));
        for k in 0..3 {
            d.unit(a, s, topic, Kind::Task, format!("s{s:04}.t{k}"));
        }
        for k in 0..3 {
            d.unit(a, s, topic, Kind::Decision, format!("s{s:04}.d{k}"));
        }
        for k in 0..8 {
            d.unit(a, s, topic, Kind::Fact, format!("s{s:04}.f{k}"));
        }
        for k in 0..5 {
            d.unit(a, s, topic, Kind::Source, format!("s{s:04}.src{k}"));
        }
        let session = base;
        let decision = |k: u32| base + 4 + k;
        let fact = |k: u32| base + 7 + k;
        let source = |k: u32| base + 15 + k;

        for u in base + 1..base + UNITS_PER_SESSION {
            d.link(a, u, session, IN_SESSION, Provenance::Declared, true);
        }

        // The chain of the topic: each decision revises the previous one.
        let chain = &mut chains[usize::from(topic)];
        let mut positions = [0usize; 3];
        for (k, position) in positions.iter_mut().enumerate() {
            let k = k as u32;
            if let Some(&previous) = chain.last() {
                d.link(a, decision(k), previous, REVISES, Provenance::Declared, true);
            }
            *position = chain.len();
            chain.push(decision(k));
        }

        for k in 0..3 {
            d.link(a, fact(k), decision(k), SUPPORTS, Provenance::Declared, true);
        }
        let mut slots = SOURCE_SLOTS;
        shuffle(rng, &mut slots);
        for (k, slot) in slots.iter().enumerate() {
            d.link(a, fact(k as u32), source(*slot), CITES, Provenance::Declared, true);
        }
        // Distractors of the chain: a fact mentions a decision of another topic.
        for k in 0..5 {
            if let Some(target) = other_topic(&decisions, topic, rng) {
                d.link(a, fact(k), target, MENTIONS, Provenance::Declared, false);
            }
        }

        // The importer adds one true support link from the last fact to the second decision.
        d.link(b, fact(7), decision(1), SUPPORTS, Provenance::Imported(IMPORT_SYSTEM.to_owned()), true);

        // The consolidator adds inferred `related` links and algorithmic `same_source` links.
        let mut pending: Vec<Link> = Vec::new();
        for (k, &position) in positions.iter().enumerate() {
            let k = k as u32;
            if position >= 2 {
                let confidence = rng.range(TRUE_CONFIDENCE.0, TRUE_CONFIDENCE.1) as u16;
                let origin = Provenance::Inferred(CONSOLIDATOR_ORIGIN.to_owned(), confidence);
                pending.push((relation(c, decision(k), chain[position - 2], RELATED, origin), true));
            }
            if let Some(target) = other_topic(&decisions, topic, rng) {
                let confidence = rng.range(FALSE_CONFIDENCE.0, FALSE_CONFIDENCE.1) as u16;
                let origin = Provenance::Inferred(CONSOLIDATOR_ORIGIN.to_owned(), confidence);
                pending.push((relation(c, decision(k), target, RELATED, origin), false));
            }
        }
        let pairs: Vec<(usize, usize)> = (0..8).flat_map(|i| (i + 1..8).map(move |j| (i, j))).collect();
        let same: Vec<(usize, usize)> = pairs.iter().copied().filter(|&(i, j)| slots[i] == slots[j]).collect();
        let mut differ: Vec<(usize, usize)> = pairs.iter().copied().filter(|&(i, j)| slots[i] != slots[j]).collect();
        shuffle(rng, &mut differ);
        let links = same.into_iter().map(|p| (p, true)).chain(differ.into_iter().take(3).map(|p| (p, false)));
        for ((i, j), truth) in links {
            let (x, y) = if rng.chance(1, 2) { (i, j) } else { (j, i) };
            let origin = Provenance::Algorithmic(DEDUP_ORIGIN.to_owned());
            pending.push((relation(c, fact(x as u32), fact(y as u32), SAME_SOURCE, origin), truth));
        }
        shuffle(rng, &mut pending);
        d.links.append(&mut pending);

        for k in 0..3 {
            decisions.push((decision(k), topic));
        }
    }
    d
}

/// Fills `out` with a vector around `centre`: each component is moved by a
/// uniform offset in `-range..=range`.
fn around(rng: &mut Rng, centre: &[i32; DIM], range: i64) -> [i32; DIM] {
    let mut v = [0i32; DIM];
    for (x, c) in v.iter_mut().zip(centre) {
        let offset = rng.range(0, (2 * range) as u64) as i64 - range;
        *x = (i64::from(*c) + offset) as i32;
    }
    v
}

fn norm(v: &[i32; DIM]) -> f64 {
    v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt()
}

/// The cosine similarity of two vectors. Vector queries rank by it.
pub fn cosine(a: &[i32; DIM], b: &[i32; DIM]) -> f64 {
    let dot: i64 = a.iter().zip(b).map(|(x, y)| i64::from(*x) * i64::from(*y)).sum();
    dot as f64 / (norm(a) * norm(b))
}

/// Who may follow which links: the adjacency of the corpus, both ways.
struct Edge {
    to: u32,
    kind: &'static str,
    class: &'static str,
    step: u64,
    truth: bool,
}

struct Adjacency {
    out: Vec<Vec<Edge>>,
    inbound: Vec<Vec<Edge>>,
}

impl Adjacency {
    fn new(units: usize, relations: &[Relation], truth: &[bool]) -> Adjacency {
        let mut out: Vec<Vec<Edge>> = (0..units).map(|_| Vec::new()).collect();
        let mut inbound: Vec<Vec<Edge>> = (0..units).map(|_| Vec::new()).collect();
        for (r, &truth) in relations.iter().zip(truth) {
            let class = r.provenance.class();
            out[r.from as usize].push(Edge { to: r.to, kind: r.kind, class, step: r.step, truth });
            inbound[r.to as usize].push(Edge { to: r.from, kind: r.kind, class, step: r.step, truth });
        }
        Adjacency { out, inbound }
    }

    fn edges(&self, unit: u32, direction: Direction) -> Vec<&Edge> {
        let u = unit as usize;
        let mut found = Vec::new();
        if direction != Direction::In {
            found.extend(self.out[u].iter());
        }
        if direction != Direction::Out {
            found.extend(self.inbound[u].iter());
        }
        found
    }
}

/// Everything the query builders need, indexed by identifier.
struct Frame<'a> {
    adjacency: Adjacency,
    vectors: &'a [[i32; DIM]],
    /// The noise-free position of each unit, by identifier.
    bases: Vec<[i32; DIM]>,
    kind: Vec<Kind>,
    session: Vec<u32>,
    sessions: u32,
}

fn admits(q: &Query, e: &Edge) -> bool {
    q.kinds.contains(&e.kind)
        && (q.provenance.is_empty() || q.provenance.contains(&e.class))
        && match q.window {
            Some((lo, hi)) => (lo..=hi).contains(&e.step),
            None => true,
        }
}

/// Grade of a unit reached through true links at this depth.
fn grade_at(depth: u8) -> u8 {
    4u8.saturating_sub(depth).max(1)
}

/// The judgments the story gives to a query before vector neighbours: the
/// seed (grade 0), the units reached through true links (grade by depth) and
/// the far ends of false links that the walk meets (grade 0).
///
/// The walk expands only through true links. A false link ends the walk, so
/// the distractors are leaves, never the path to a relevant unit.
fn walk(adj: &Adjacency, q: &Query) -> BTreeMap<u32, u8> {
    let mut reached: BTreeMap<u32, u8> = BTreeMap::new();
    let mut named: BTreeSet<u32> = BTreeSet::new();
    reached.insert(q.seed, 0);
    let mut frontier = vec![q.seed];
    for depth in 0..q.depth {
        let mut next = Vec::new();
        for &u in &frontier {
            for e in adj.edges(u, q.direction) {
                if !admits(q, e) {
                    continue;
                }
                if !e.truth {
                    named.insert(e.to);
                } else if let std::collections::btree_map::Entry::Vacant(slot) = reached.entry(e.to) {
                    slot.insert(depth + 1);
                    next.push(e.to);
                }
            }
        }
        frontier = next;
    }
    let mut judged: BTreeMap<u32, u8> = BTreeMap::new();
    judged.insert(q.seed, 0);
    for unit in named {
        judged.entry(unit).or_insert(0);
    }
    for (unit, depth) in reached {
        if unit != q.seed {
            judged.insert(unit, grade_at(depth));
        }
    }
    judged
}

/// Judges the ten nearest units by cosine that are not judged yet, as grade 0.
///
/// After this, every unit in the exact top ten of the query vector is judged:
/// a unit of that top ten that was not judged would be among the ten nearest
/// unjudged units, since at most nine units are nearer to the query. The list
/// is fixed here, before any engine runs, so no arm of the comparison chooses it.
fn judge_neighbours(vectors: &[[i32; DIM]], query: &[i32; DIM], judged: &mut BTreeMap<u32, u8>) {
    let mut ranked: Vec<(f64, u32)> = vectors.iter().enumerate().map(|(id, v)| (cosine(v, query), id as u32)).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut added = 0;
    for &(_, id) in &ranked {
        if added == VECTOR_NEIGHBOURS {
            break;
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = judged.entry(id) {
            slot.insert(0);
            added += 1;
        }
    }
}

/// Judges the first ten units of the simple order (hops, then identifier) as
/// grade 0 when they are not judged yet. The simple order is the reference of
/// acta §6.2 (Q6). Its walk follows every admitted link, false ones included,
/// so its first ten can lie beyond a false link; those units are judged here
/// too, and no unjudged unit can sit in the top ten of this order.
fn judge_simple_top(adj: &Adjacency, q: &Query, judged: &mut BTreeMap<u32, u8>) {
    let mut hop: BTreeMap<u32, u8> = BTreeMap::from([(q.seed, 0)]);
    let mut frontier = vec![q.seed];
    for depth in 0..q.depth {
        let mut next = Vec::new();
        for &u in &frontier {
            for e in adj.edges(u, q.direction) {
                if admits(q, e) && !hop.contains_key(&e.to) {
                    hop.insert(e.to, depth + 1);
                    next.push(e.to);
                }
            }
        }
        frontier = next;
    }
    let mut order: Vec<(u8, u32)> = hop.into_iter().filter(|(u, _)| *u != q.seed).map(|(u, d)| (d, u)).collect();
    order.sort_unstable();
    for (_, unit) in order.into_iter().take(CUT) {
        judged.entry(unit).or_insert(0);
    }
}

/// A query is usable when it has a relevant answer and a named distractor
/// other than its own seed. A seed that lacks either is drawn again.
fn draw(
    f: &Frame<'_>,
    rng: &mut Rng,
    candidates: &[u32],
    used: &mut BTreeSet<u32>,
    make: impl Fn(&mut Rng, u32) -> Query,
) -> Query {
    for _ in 0..MAX_DRAWS {
        let seed = candidates[rng.below(candidates.len() as u64) as usize];
        if used.contains(&seed) {
            continue;
        }
        let mut q = make(&mut *rng, seed);
        let mut judged = walk(&f.adjacency, &q);
        if let Some(vector) = q.vector {
            judge_neighbours(f.vectors, &vector, &mut judged);
        }
        judge_simple_top(&f.adjacency, &q, &mut judged);
        let relevant = judged.values().any(|&g| g >= 1);
        let distractor = judged.iter().any(|(&u, &g)| g == 0 && u != seed);
        if relevant && distractor {
            used.insert(seed);
            q.expected = judged.into_iter().map(|(unit, grade)| Judgment { unit, grade }).collect();
            return q;
        }
    }
    panic!("no usable seed in {MAX_DRAWS} draws: the story has no unit for this query");
}

fn draw_class(
    f: &Frame<'_>,
    rng: &mut Rng,
    prefix: &str,
    count: usize,
    candidates: &[u32],
    make: impl Fn(&mut Rng, u32) -> Query,
) -> Vec<Query> {
    let mut used = BTreeSet::new();
    (1..=count)
        .map(|n| {
            let mut q = draw(f, rng, candidates, &mut used, &make);
            q.id = format!("{prefix}{n:02}");
            q
        })
        .collect()
}

/// The window of creation steps of the sessions within `radius` of `session`,
/// both ends included.
fn window(f: &Frame<'_>, session: u32, radius: u32) -> (u64, u64) {
    let first = session.saturating_sub(radius);
    let last = (session + radius).min(f.sessions - 1);
    (3 * u64::from(first) + 1, 3 * u64::from(last) + 3)
}

fn ids_of(f: &Frame<'_>, kind: Kind) -> Vec<u32> {
    (0..f.kind.len() as u32).filter(|&id| f.kind[id as usize] == kind).collect()
}

fn blank(class: Class, direction: Direction, kinds: Vec<&'static str>, depth: u8) -> Query {
    Query {
        id: String::new(),
        class,
        seed: 0,
        direction,
        kinds,
        depth,
        window: None,
        provenance: Vec::new(),
        vector: None,
        expected: Vec::new(),
    }
}

/// The queries of the four classes, in this order: 25 vector neighbourhood,
/// 25 temporal, and 25 provenance (8 `related`, 8 `same_source`, 9 declared
/// chain), and 25 combined.
fn build_queries(f: &Frame<'_>, rng: &mut Rng) -> Vec<Query> {
    let decisions = ids_of(f, Kind::Decision);
    let facts = ids_of(f, Kind::Fact);
    let old: Vec<u32> = decisions.iter().copied().filter(|&d| f.session[d as usize] >= 2).collect();
    let mut queries = Vec::new();

    queries.extend(draw_class(f, rng, "VN-", PER_CLASS, &decisions, |rng, seed| {
        let base = f.bases[seed as usize];
        let mut q = blank(Class::VectorNeighbourhood, Direction::Both, vec![REVISES], 2);
        q.seed = seed;
        q.vector = Some(around(rng, &base, DECISION_NOISE));
        q
    }));

    queries.extend(draw_class(f, rng, "TE-", PER_CLASS, &old, |_, seed| {
        let mut q = blank(Class::Temporal, Direction::Both, vec![REVISES, SUPPORTS, MENTIONS], 3);
        q.seed = seed;
        q.window = Some(window(f, f.session[seed as usize], 2));
        q
    }));

    let related = draw_class(f, rng, "PV-RE-", 8, &decisions, |_, seed| {
        let mut q = blank(Class::Provenance, Direction::Out, vec![RELATED], 1);
        q.seed = seed;
        q.provenance = vec!["inferred"];
        q
    });
    let same_source = draw_class(f, rng, "PV-SS-", 8, &facts, |_, seed| {
        let mut q = blank(Class::Provenance, Direction::Both, vec![SAME_SOURCE], 1);
        q.seed = seed;
        q.provenance = vec!["algorithmic"];
        q
    });
    let declared: Vec<u32> = decisions.iter().copied().filter(|&d| f.session[d as usize] >= 3).collect();
    let declared_chain = draw_class(f, rng, "PV-DE-", 9, &declared, |_, seed| {
        let mut q = blank(Class::Provenance, Direction::Both, vec![REVISES, SUPPORTS, MENTIONS], 3);
        q.seed = seed;
        q.provenance = vec!["declared"];
        q
    });
    queries.extend(related);
    queries.extend(same_source);
    queries.extend(declared_chain);

    queries.extend(draw_class(f, rng, "CB-", PER_CLASS, &decisions, |_, seed| {
        let mut q = blank(Class::Combined, Direction::Both, vec![REVISES, SUPPORTS, RELATED], 3);
        q.seed = seed;
        q.window = Some(window(f, f.session[seed as usize], 1));
        q.provenance = vec!["declared", "inferred"];
        q
    }));
    queries
}

impl Story {
    /// Generates the whole set from `seed`. The same seed and scale give the
    /// same story, byte for byte once rendered.
    pub fn generate(scale: Scale, seed: u64) -> Story {
        let mut rng = Rng::new(seed);
        let draft = draft(scale.sessions(), &mut rng);
        let count = draft.units.len();

        // ids[creation index] is the identifier: a seeded permutation.
        let mut ids: Vec<u32> = (0..count as u32).collect();
        shuffle(&mut Rng::new(seed ^ PERMUTATION_STREAM), &mut ids);

        let units: Vec<Unit> = draft
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| Unit { id: ids[i], step: u.step, kind: u.kind, label: u.label.clone() })
            .collect();
        let (relations, truth): (Vec<Relation>, Vec<bool>) = draft
            .links
            .iter()
            .map(|(r, t)| {
                let from = ids[r.from as usize];
                let to = ids[r.to as usize];
                (Relation { from, to, ..r.clone() }, *t)
            })
            .unzip();

        // Vectors. A topic has a centre. Its decisions walk away from the centre
        // one step per decision of the chain, so decisions that are close in the
        // chain are close in the space. Other units stay near the centre.
        let mut vrng = Rng::new(seed ^ VECTOR_STREAM);
        let centres: Vec<[i32; DIM]> = (0..TOPICS).map(|_| around(&mut vrng, &[0; DIM], CENTRE_RANGE)).collect();
        let mut walks: Vec<[i32; DIM]> = vec![[0; DIM]; TOPICS];
        let mut vectors = vec![[0i32; DIM]; count];
        let mut bases = vec![[0i32; DIM]; count];
        let mut session = vec![0u32; count];
        let mut kind = vec![Kind::Session; count];
        for (i, u) in draft.units.iter().enumerate() {
            let id = ids[i] as usize;
            let t = usize::from(u.topic);
            let (base, noise) = if u.kind == Kind::Decision {
                let step = around(&mut vrng, &[0; DIM], WALK_STEP);
                walks[t] = std::array::from_fn(|d| walks[t][d] + step[d]);
                (std::array::from_fn(|d| centres[t][d] + walks[t][d]), DECISION_NOISE)
            } else {
                (centres[t], NOISE_RANGE)
            };
            bases[id] = base;
            vectors[id] = around(&mut vrng, &base, noise);
            session[id] = u.session;
            kind[id] = u.kind;
        }

        let queries = {
            let frame = Frame {
                adjacency: Adjacency::new(count, &relations, &truth),
                vectors: &vectors,
                bases,
                kind,
                session,
                sessions: scale.sessions(),
            };
            build_queries(&frame, &mut Rng::new(seed ^ QUERY_STREAM))
        };

        Story {
            corpus: Corpus { seed, scale, units, changes: draft.changes, relations },
            vectors,
            queries: QuerySet { seed, scale, queries },
        }
    }
}

// ---------------------------------------------------------------------------
// Text formats. One record per line; blank lines and lines starting with `#`
// are ignored. Fields are separated by spaces, and no field contains one.
// ---------------------------------------------------------------------------

/// Writes the corpus: a change line, then its units, then its links.
pub fn render_corpus(c: &Corpus) -> String {
    let mut s = String::from("# kerno-testkit derived-eval corpus v1\n");
    writeln!(s, "seed {} scale {}", c.seed, c.scale.name()).expect(WRITE);
    writeln!(s, "counts units {} relations {} changes {}", c.units.len(), c.relations.len(), c.changes.len())
        .expect(WRITE);
    for change in &c.changes {
        writeln!(s, "C {} {} {}", change.step, change.actor, change.source).expect(WRITE);
        for u in c.units.iter().filter(|u| u.step == change.step) {
            writeln!(s, "U {} {} {} {}", u.id, u.step, u.kind.name(), u.label).expect(WRITE);
        }
        for r in c.relations.iter().filter(|r| r.step == change.step) {
            writeln!(s, "R {} {} {} {} {}", r.step, r.from, r.to, r.kind, r.provenance.render()).expect(WRITE);
        }
    }
    s
}

/// Writes one vector per unit, in identifier order.
pub fn render_vectors(vectors: &[[i32; DIM]]) -> String {
    let mut s = String::from("# kerno-testkit derived-eval vectors v1\n");
    writeln!(s, "dim {DIM} counts vectors {}", vectors.len()).expect(WRITE);
    for (id, v) in vectors.iter().enumerate() {
        write!(s, "V {id}").expect(WRITE);
        for x in v {
            write!(s, " {x}").expect(WRITE);
        }
        s.push('\n');
    }
    s
}

fn list_or_dash<T: ToString>(items: &[T]) -> String {
    if items.is_empty() {
        return "-".to_owned();
    }
    items.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
}

/// Writes each query on a `Q` line followed by its `E` lines.
pub fn render_queries(set: &QuerySet) -> String {
    let mut s = String::from("# kerno-testkit derived-eval queries v1\n");
    writeln!(s, "seed {} scale {} cut {CUT} dim {DIM}", set.seed, set.scale.name()).expect(WRITE);
    writeln!(s, "counts queries {}", set.queries.len()).expect(WRITE);
    for q in &set.queries {
        let window = match q.window {
            Some((lo, hi)) => format!("{lo}-{hi}"),
            None => "-".to_owned(),
        };
        let vector = match &q.vector {
            Some(v) => list_or_dash(v),
            None => "-".to_owned(),
        };
        writeln!(
            s,
            "Q {} {} {} {} {} {} {} {} {}",
            q.id,
            q.class.name(),
            q.seed,
            q.direction.name(),
            list_or_dash(&q.kinds),
            q.depth,
            window,
            list_or_dash(&q.provenance),
            vector
        )
        .expect(WRITE);
        for j in &q.expected {
            writeln!(s, "E {} {}", j.unit, j.grade).expect(WRITE);
        }
    }
    s
}

/// A format error with the line it was found on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError {
    line: usize,
    reason: String,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for FormatError {}

fn bad(line: usize, reason: impl Into<String>) -> FormatError {
    FormatError { line, reason: reason.into() }
}

fn records(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().map(|(i, l)| (i + 1, l)).filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
}

fn token<'a>(line: usize, token: Option<&'a str>, what: &str) -> Result<&'a str, FormatError> {
    token.ok_or_else(|| bad(line, format!("missing {what}")))
}

fn num<T: FromStr>(line: usize, value: Option<&str>, what: &str) -> Result<T, FormatError> {
    let text = token(line, value, what)?;
    text.parse().map_err(|_| bad(line, format!("{what} is not a number: {text}")))
}

fn expect_word(line: usize, token: Option<&str>, word: &str) -> Result<(), FormatError> {
    match token {
        Some(t) if t == word => Ok(()),
        _ => Err(bad(line, format!("expected '{word}'"))),
    }
}

fn names(line: usize, text: &str, known: fn(&str) -> Option<&'static str>) -> Result<Vec<&'static str>, FormatError> {
    text.split(',').map(|name| known(name).ok_or_else(|| bad(line, format!("unknown name {name}")))).collect()
}

fn parse_provenance(line: usize, t: &mut SplitWhitespace<'_>) -> Result<Provenance, FormatError> {
    match token(line, t.next(), "provenance")? {
        "declared" => Ok(Provenance::Declared),
        "imported" => Ok(Provenance::Imported(token(line, t.next(), "source")?.to_owned())),
        "algorithmic" => Ok(Provenance::Algorithmic(token(line, t.next(), "origin")?.to_owned())),
        "inferred" => {
            let origin = token(line, t.next(), "origin")?.to_owned();
            let confidence: u16 = num(line, t.next(), "confidence")?;
            if confidence > 1000 {
                return Err(bad(line, "confidence above 1000"));
            }
            Ok(Provenance::Inferred(origin, confidence))
        }
        other => Err(bad(line, format!("unknown provenance {other}"))),
    }
}

/// Reads a corpus written by [`render_corpus`]. It checks the counts, the
/// order of the change steps, that identifiers are in range and not repeated,
/// and that every link joins units created at or before its step.
pub fn parse_corpus(text: &str) -> Result<Corpus, FormatError> {
    let mut lines = records(text);
    let (n, header) = lines.next().ok_or_else(|| bad(0, "empty corpus"))?;
    let mut t = header.split_whitespace();
    expect_word(n, t.next(), "seed")?;
    let seed: u64 = num(n, t.next(), "seed")?;
    expect_word(n, t.next(), "scale")?;
    let scale = Scale::from_name(token(n, t.next(), "scale")?).ok_or_else(|| bad(n, "unknown scale"))?;

    let (n, counts) = lines.next().ok_or_else(|| bad(0, "missing counts"))?;
    let mut t = counts.split_whitespace();
    expect_word(n, t.next(), "counts")?;
    expect_word(n, t.next(), "units")?;
    let unit_count: usize = num(n, t.next(), "units")?;
    expect_word(n, t.next(), "relations")?;
    let relation_count: usize = num(n, t.next(), "relations")?;
    expect_word(n, t.next(), "changes")?;
    let change_count: usize = num(n, t.next(), "changes")?;

    let mut changes = Vec::new();
    let mut units = Vec::new();
    let mut relations = Vec::new();
    let mut created_at: Vec<Option<u64>> = vec![None; unit_count];
    let mut current = 0u64;
    for (n, line) in lines {
        let mut t = line.split_whitespace();
        match token(n, t.next(), "record")? {
            "C" => {
                let step: u64 = num(n, t.next(), "step")?;
                if step != current + 1 {
                    return Err(bad(n, "the change step is not the next one"));
                }
                current = step;
                let actor = token(n, t.next(), "actor")?.to_owned();
                let source = token(n, t.next(), "source")?.to_owned();
                changes.push(Change { step, actor, source });
            }
            "U" => {
                let id: u32 = num(n, t.next(), "id")?;
                let step: u64 = num(n, t.next(), "step")?;
                let kind = Kind::from_name(token(n, t.next(), "kind")?).ok_or_else(|| bad(n, "unknown kind"))?;
                let label = token(n, t.next(), "label")?.to_owned();
                if step != current {
                    return Err(bad(n, "the unit does not belong to the change being read"));
                }
                match created_at.get_mut(id as usize) {
                    Some(slot) if slot.is_none() => *slot = Some(step),
                    _ => return Err(bad(n, "unit identifier out of range or repeated")),
                }
                units.push(Unit { id, step, kind, label });
            }
            "R" => {
                let step: u64 = num(n, t.next(), "step")?;
                if step != current {
                    return Err(bad(n, "the link does not belong to the change being read"));
                }
                let from: u32 = num(n, t.next(), "from")?;
                let to: u32 = num(n, t.next(), "to")?;
                let kind = relation_kind(token(n, t.next(), "kind")?).ok_or_else(|| bad(n, "unknown link kind"))?;
                for end in [from, to] {
                    match created_at.get(end as usize).copied().flatten() {
                        Some(created) if created <= step => {}
                        _ => return Err(bad(n, "a link endpoint is not created yet")),
                    }
                }
                let provenance = parse_provenance(n, &mut t)?;
                relations.push(relation(step, from, to, kind, provenance));
            }
            other => return Err(bad(n, format!("unknown record {other}"))),
        }
    }
    if units.len() != unit_count || relations.len() != relation_count || changes.len() != change_count {
        return Err(bad(0, "the records do not match the counts in the header"));
    }
    Ok(Corpus { seed, scale, units, changes, relations })
}

/// Reads the vectors written by [`render_vectors`], in identifier order.
pub fn parse_vectors(text: &str) -> Result<Vec<[i32; DIM]>, FormatError> {
    let mut lines = records(text);
    let (n, header) = lines.next().ok_or_else(|| bad(0, "empty vectors"))?;
    let mut t = header.split_whitespace();
    expect_word(n, t.next(), "dim")?;
    let dim: usize = num(n, t.next(), "dim")?;
    if dim != DIM {
        return Err(bad(n, format!("dimension {dim}, expected {DIM}")));
    }
    expect_word(n, t.next(), "counts")?;
    expect_word(n, t.next(), "vectors")?;
    let count: usize = num(n, t.next(), "vectors")?;
    let mut vectors = Vec::with_capacity(count);
    for (n, line) in lines {
        let mut t = line.split_whitespace();
        expect_word(n, t.next(), "V")?;
        let id: usize = num(n, t.next(), "id")?;
        if id != vectors.len() {
            return Err(bad(n, "vector out of order"));
        }
        let mut v = [0i32; DIM];
        for x in &mut v {
            *x = num(n, t.next(), "component")?;
        }
        if t.next().is_some() {
            return Err(bad(n, "more components than the dimension"));
        }
        vectors.push(v);
    }
    if vectors.len() != count {
        return Err(bad(0, "the vectors do not match the count in the header"));
    }
    Ok(vectors)
}

fn parse_query_line(line: usize, t: &mut SplitWhitespace<'_>) -> Result<Query, FormatError> {
    let id = token(line, t.next(), "id")?.to_owned();
    let class = Class::from_name(token(line, t.next(), "class")?).ok_or_else(|| bad(line, "unknown class"))?;
    let seed: u32 = num(line, t.next(), "seed")?;
    let direction =
        Direction::from_name(token(line, t.next(), "direction")?).ok_or_else(|| bad(line, "unknown direction"))?;
    let kinds = names(line, token(line, t.next(), "kinds")?, relation_kind)?;
    let depth: u8 = num(line, t.next(), "depth")?;
    let window = match token(line, t.next(), "window")? {
        "-" => None,
        text => {
            let (lo, hi) = text.split_once('-').ok_or_else(|| bad(line, "a window is lo-hi or -"))?;
            Some((num(line, Some(lo), "window start")?, num(line, Some(hi), "window end")?))
        }
    };
    let provenance = match token(line, t.next(), "provenance")? {
        "-" => Vec::new(),
        text => names(line, text, provenance_class)?,
    };
    let vector = match token(line, t.next(), "vector")? {
        "-" => None,
        text => {
            let parts: Vec<&str> = text.split(',').collect();
            if parts.len() != DIM {
                return Err(bad(line, "a query vector has the wrong dimension"));
            }
            let mut v = [0i32; DIM];
            for (x, part) in v.iter_mut().zip(parts) {
                *x = num(line, Some(part), "query component")?;
            }
            Some(v)
        }
    };
    if t.next().is_some() {
        return Err(bad(line, "more fields than a query has"));
    }
    Ok(Query { id, class, seed, direction, kinds, depth, window, provenance, vector, expected: Vec::new() })
}

/// Reads the queries written by [`render_queries`]. Expected answers must be
/// sorted by unit, without repeats, and have grades from 0 to 3.
pub fn parse_queries(text: &str) -> Result<QuerySet, FormatError> {
    let mut lines = records(text);
    let (n, header) = lines.next().ok_or_else(|| bad(0, "empty queries"))?;
    let mut t = header.split_whitespace();
    expect_word(n, t.next(), "seed")?;
    let seed: u64 = num(n, t.next(), "seed")?;
    expect_word(n, t.next(), "scale")?;
    let scale = Scale::from_name(token(n, t.next(), "scale")?).ok_or_else(|| bad(n, "unknown scale"))?;
    expect_word(n, t.next(), "cut")?;
    let cut: usize = num(n, t.next(), "cut")?;
    expect_word(n, t.next(), "dim")?;
    let dim: usize = num(n, t.next(), "dim")?;
    if cut != CUT || dim != DIM {
        return Err(bad(n, "the cut or the dimension is not the one of this set"));
    }

    let (n, counts) = lines.next().ok_or_else(|| bad(0, "missing counts"))?;
    let mut t = counts.split_whitespace();
    expect_word(n, t.next(), "counts")?;
    expect_word(n, t.next(), "queries")?;
    let count: usize = num(n, t.next(), "queries")?;

    let mut queries: Vec<Query> = Vec::new();
    for (n, line) in lines {
        let mut t = line.split_whitespace();
        match token(n, t.next(), "record")? {
            "Q" => queries.push(parse_query_line(n, &mut t)?),
            "E" => {
                let unit: u32 = num(n, t.next(), "unit")?;
                let grade: u8 = num(n, t.next(), "grade")?;
                if grade > 3 {
                    return Err(bad(n, "grade outside 0 to 3"));
                }
                let q = queries.last_mut().ok_or_else(|| bad(n, "an answer before any query"))?;
                if q.expected.last().is_some_and(|prev| prev.unit >= unit) {
                    return Err(bad(n, "answers are not sorted or repeat a unit"));
                }
                q.expected.push(Judgment { unit, grade });
            }
            other => return Err(bad(n, format!("unknown record {other}"))),
        }
    }
    if queries.len() != count {
        return Err(bad(0, "the queries do not match the count in the header"));
    }
    Ok(QuerySet { seed, scale, queries })
}
