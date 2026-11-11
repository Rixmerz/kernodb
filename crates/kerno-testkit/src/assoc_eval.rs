//! The frozen evaluation set of the associative memory (Stage 7, E7-00).
//!
//! The set is an agent's memory written as a story, generated from a fixed
//! seed. The story is rendered twice: as a corpus of units, relations and
//! change sets, and as queries whose expected answers are read off the story
//! itself, with a graded relevance from 0 to 3. The expected answers are never
//! produced by running a traversal over the generated graph: the query walks
//! below follow the story's own bookkeeping (a topic chain and the evidence of
//! each decision). This module does not depend on `kerno-units`; the four
//! provenance kinds are plain data here.
//!
//! Story, per session `s` (20 units, three change sets at steps `3s+1`,
//! `3s+2` and `3s+3`):
//!
//! * the session runs by one agent (the change actor) and writes one
//!   transcript (the change source);
//! * three tasks, each `in_session` of it; three decisions of one topic, each
//!   `produced_by` its task and each `revises` the previous decision of the
//!   topic, so that each topic is a chain across sessions;
//! * eight facts. `F0` and `F2` support `D0` and `D2`; `F1` supports `D1`, but
//!   it arrives by import in the second change set (`imported`). `F3`..`F7`
//!   are `about` tasks. Every fact cites one of five sources, in a random
//!   arrangement per session (three pairs of facts share a source);
//! * `F0`..`F2` each `mention` a decision of another topic: a distractor;
//! * the third change set (the consolidator) adds `related` links, inferred
//!   with a confidence: one true link to the decision two places back in the
//!   topic, one false link to a decision of another topic; the two confidence
//!   ranges overlap. A dedup job adds `same_source` links between facts
//!   (`algorithmic`): three true (a pair that cites one source), three false
//!   (a random pair that does not). Order and orientation are random.
//!
//! Relevance follows the topic chain. The decisions one and two revisions away
//! from the seed are relevant (grades 3 and 2), and so is the evidence of the
//! seed and of those decisions (grades 3, 2 and 1). Everything else a query
//! can reach is a distractor (grade 0), and the set names the distractors it
//! means. The expected-answer walks read the story's own records, so a unit is
//! relevant only if the constraints of its query admit a path to it.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::str::FromStr;

use crate::rng::Rng;
use crate::sha256;

/// Seed of the frozen set: the date of the entry record, 2026-11-11.
pub const SEED: u64 = 20_261_111;
/// Mixed into the seed for the queries, so they do not share a stream with the corpus.
const QUERY_STREAM: u64 = 0x0E70_0000_0000_0001;
/// Units in each session. Quality: 100 sessions (2 000 units); performance: 1 000 (20 000).
pub const UNITS_PER_SESSION: u32 = 20;
const TOPICS: u64 = 12;
const AGENTS: [&str; 3] = ["agent-alpha", "agent-beta", "agent-gamma"];
const IMPORT_SYSTEM: &str = "wiki-export";
const DEDUP_ORIGIN: &str = "dedup-v2";
const CONSOLIDATOR_ORIGIN: &str = "consolidator-v1";
/// Sessions before a seed, so that every seed has a cross-topic mention target.
const MIN_SEED_SESSION: u32 = 5;
/// Cut of every evaluation (acta §6.2).
pub const CUT: usize = 10;
/// Highest relevance grade. Gains are `2^grade - 1` (see `assoc_metrics`).
pub const MAX_GRADE: u8 = 3;

pub const IN_SESSION: &str = "in_session";
pub const PRODUCED_BY: &str = "produced_by";
pub const REVISES: &str = "revises";
pub const SUPPORTS: &str = "supports";
pub const MENTIONS: &str = "mentions";
pub const ABOUT: &str = "about";
pub const CITES: &str = "cites";
pub const RELATED: &str = "related";
pub const SAME_SOURCE: &str = "same_source";

/// The relation types of the chain queries.
const CHAIN_TYPES: [&str; 3] = [REVISES, SUPPORTS, MENTIONS];
/// `(evidence, task)` offsets of the `about` links written in the first change set.
const ABOUT_MAIN: [(usize, usize); 4] = [(3, 0), (4, 1), (5, 2), (7, 1)];
/// `(evidence, task)` offsets of the `about` link written by the import.
const ABOUT_IMPORTED: (usize, usize) = (6, 0);
/// The source class (0..5) each fact cites before the per-session shuffle: classes
/// 0, 1 and 2 are cited twice and 3 and 4 once, so a session has three pairs of
/// facts that share a source.
const SOURCE_CLASSES: [usize; 8] = [0, 1, 2, 3, 4, 0, 1, 2];
/// `same_source` links the dedup job gets wrong, per session.
const SAME_FALSE_PER_SESSION: usize = 3;
/// Confidence (per mille) of the true and of the false `related` links. The
/// ranges overlap on 450..=800, so confidence is informative but not sufficient.
const TRUE_CONFIDENCE: (u64, u64) = (450, 950);
const FALSE_CONFIDENCE: (u64, u64) = (300, 800);

const WRITE: &str = "a String accepts every write";

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
            Scale::Performance => 1000,
        }
    }

    fn from_name(name: &str) -> Option<Scale> {
        [Scale::Quality, Scale::Performance].into_iter().find(|s| s.name() == name)
    }
}

/// How a relation came to exist (the four kinds of ADR-0014).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Declared,
    Imported,
    Algorithmic,
    Inferred,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Declared, Kind::Imported, Kind::Algorithmic, Kind::Inferred];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Declared => "declared",
            Kind::Imported => "imported",
            Kind::Algorithmic => "algorithmic",
            Kind::Inferred => "inferred",
        }
    }

    fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provenance {
    Declared,
    Imported(String),
    Algorithmic(String),
    Inferred { origin: String, confidence_permille: u16 },
}

impl Provenance {
    pub fn kind(&self) -> Kind {
        match self {
            Provenance::Declared => Kind::Declared,
            Provenance::Imported(_) => Kind::Imported,
            Provenance::Algorithmic(_) => Kind::Algorithmic,
            Provenance::Inferred { .. } => Kind::Inferred,
        }
    }
}

/// A change set of the story: its logical step (1, 2, 3, ...), its actor and
/// its source. Steps are a commit order, not a clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub step: u64,
    pub actor: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// Stable local index: the position in [`Corpus::units`].
    pub index: u32,
    /// The step of the change set that created the unit.
    pub step: u64,
    pub unit_type: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    /// Stable local index: the position in [`Corpus::relations`].
    pub index: u32,
    /// The step of the change set that created the relation.
    pub step: u64,
    pub from: u32,
    pub to: u32,
    pub relation_type: String,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corpus {
    pub seed: u64,
    pub scale: Scale,
    /// `changes[i]` has step `i + 1`.
    pub changes: Vec<Change>,
    pub units: Vec<Unit>,
    pub relations: Vec<Relation>,
}

/// Which way a traversal may follow a relation, relative to the unit it is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Only relations that start at the current unit.
    Out,
    /// Only relations that end at the current unit.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    MultiHop,
    Temporal,
    Provenance,
    Combined,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::MultiHop => "multi_hop",
            Class::Temporal => "temporal",
            Class::Provenance => "provenance",
            Class::Combined => "combined",
        }
    }

    fn from_name(name: &str) -> Option<Class> {
        [Class::MultiHop, Class::Temporal, Class::Provenance, Class::Combined].into_iter().find(|c| c.name() == name)
    }
}

/// The bounds and filters of a query. Every relation a walk follows must
/// satisfy all of them; `window` is an inclusive range of creation steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraints {
    pub direction: Direction,
    /// Non-empty: the relation types a walk may follow.
    pub relation_types: Vec<String>,
    pub max_depth: u8,
    pub window: Option<(u64, u64)>,
    /// Empty: every provenance kind is admitted.
    pub kinds: Vec<Kind>,
    /// The actor of the change set that created the relation.
    pub actor: Option<String>,
    /// The source of the change set that created the relation.
    pub source: Option<String>,
}

/// One expected answer: a unit and its relevance grade. Grade 0 is a named
/// distractor: reachable within the bounds, not relevant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Judgment {
    pub index: u32,
    pub grade: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// For example `MH-01`; the prefix names the query family.
    pub id: String,
    pub class: Class,
    /// The seed unit.
    pub seed: u32,
    /// The type of the seed unit. The baseline B0 filters by this type.
    pub unit_type: String,
    pub constraints: Constraints,
    /// Sorted by unit index, without duplicates.
    pub expected: Vec<Judgment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuerySet {
    pub seed: u64,
    pub scale: Scale,
    pub queries: Vec<Query>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Story {
    pub corpus: Corpus,
    pub queries: QuerySet,
}

impl Story {
    /// Generates the corpus and the queries of `scale` from `seed`. The same
    /// arguments give the same story, byte for byte once rendered.
    pub fn generate(scale: Scale, seed: u64) -> Story {
        let mut builder = Builder {
            corpus: Corpus { seed, scale, changes: Vec::new(), units: Vec::new(), relations: Vec::new() },
            led: Ledger { chains: vec![Vec::new(); TOPICS as usize], ..Ledger::default() },
            rng: Rng::new(seed),
        };
        for s in 0..scale.sessions() {
            builder.session(s);
        }
        let Builder { corpus, led, .. } = builder;
        let queries = build_queries(&corpus, &led, seed);
        Story { corpus, queries: QuerySet { seed, scale, queries } }
    }
}

/// Story bookkeeping that the expected answers read. Kept beside the corpus,
/// never inside it.
#[derive(Default)]
struct Ledger {
    nodes: Vec<Node>,
    /// Per topic: its decisions in commit order.
    chains: Vec<Vec<u32>>,
    sessions: Vec<SessionInfo>,
    /// Per relation: whether the story says it holds. Only inferred and
    /// algorithmic relations can be false.
    truth: Vec<bool>,
}

#[derive(Clone, Default)]
struct Node {
    session: u32,
    /// For a decision: its place in its topic chain.
    chain_pos: u32,
    /// For a decision: its `revises` relation to the previous one of the topic.
    revises: Option<u32>,
    /// For a decision: its `supports` relation from the evidence fact.
    evidence: Option<u32>,
    /// For a fact: its `mentions` relation to a decision of another topic.
    mention: Option<u32>,
    /// `related` and `same_source` relations at this unit, either end.
    links: Vec<u32>,
}

struct SessionInfo {
    topic: usize,
    actor: &'static str,
    parts: Parts,
}

/// The units of a session that later change sets refer to.
#[derive(Clone, Copy)]
struct Parts {
    decisions: [u32; 3],
}

impl Ledger {
    fn session_of(&self, unit: u32) -> &SessionInfo {
        &self.sessions[self.nodes[unit as usize].session as usize]
    }

    fn chain_of(&self, decision: u32) -> &[u32] {
        &self.chains[self.session_of(decision).topic]
    }
}

struct Builder {
    corpus: Corpus,
    led: Ledger,
    rng: Rng,
}

impl Builder {
    fn change(&mut self, step: u64, actor: &str, source: String) {
        assert_eq!(step, self.corpus.changes.len() as u64 + 1, "steps are consecutive");
        self.corpus.changes.push(Change { step, actor: actor.to_owned(), source });
    }

    fn unit(&mut self, step: u64, session: u32, unit_type: &str, label: String) -> u32 {
        let index = self.corpus.units.len() as u32;
        self.corpus.units.push(Unit { index, step, unit_type: unit_type.to_owned(), label });
        self.led.nodes.push(Node { session, ..Node::default() });
        index
    }

    fn rel(&mut self, step: u64, from: u32, to: u32, relation_type: &str, provenance: Provenance, truth: bool) -> u32 {
        let index = self.corpus.relations.len() as u32;
        if relation_type == RELATED || relation_type == SAME_SOURCE {
            self.led.nodes[from as usize].links.push(index);
            self.led.nodes[to as usize].links.push(index);
        }
        self.corpus.relations.push(Relation {
            index,
            step,
            from,
            to,
            relation_type: relation_type.to_owned(),
            provenance,
        });
        self.led.truth.push(truth);
        index
    }

    /// A decision of another topic from an earlier session, if one is drawn.
    fn other_topic_decision(&mut self, s: u32, topic: usize) -> Option<u32> {
        if s == 0 {
            return None;
        }
        for _ in 0..8 {
            let earlier = self.rng.below(u64::from(s)) as usize;
            if self.led.sessions[earlier].topic != topic {
                let decisions = self.led.sessions[earlier].parts.decisions;
                return Some(decisions[self.rng.below(decisions.len() as u64) as usize]);
            }
        }
        None
    }

    fn session(&mut self, s: u32) {
        let topic = self.rng.below(TOPICS) as usize;
        let actor = AGENTS[self.rng.below(AGENTS.len() as u64) as usize];
        let main = main_step(s);
        let import = main + 1;
        let consolidate = main + 2;

        self.change(main, actor, transcript_source(s));
        let session = self.unit(main, s, "session", format!("s{s:04}"));
        let tasks: [u32; 3] = std::array::from_fn(|i| self.unit(main, s, "task", format!("s{s:04}.t{i}")));
        let decisions: [u32; 3] = std::array::from_fn(|i| self.unit(main, s, "decision", format!("s{s:04}.d{i}")));
        let facts: [u32; 8] = std::array::from_fn(|i| self.unit(main, s, "fact", format!("s{s:04}.f{i}")));
        let sources: [u32; 5] = std::array::from_fn(|i| self.unit(main, s, "source", format!("s{s:04}.src{i}")));
        self.led.sessions.push(SessionInfo { topic, actor, parts: Parts { decisions } });

        for &t in &tasks {
            self.rel(main, t, session, IN_SESSION, Provenance::Declared, true);
        }
        for (&d, &t) in decisions.iter().zip(&tasks) {
            self.rel(main, d, t, PRODUCED_BY, Provenance::Declared, true);
        }
        for (i, (&f, &d)) in facts.iter().zip(&decisions).enumerate() {
            // F1 supports D1 only through the import, in the second change set.
            if i != 1 {
                let r = self.rel(main, f, d, SUPPORTS, Provenance::Declared, true);
                self.led.nodes[d as usize].evidence = Some(r);
            }
        }
        for &f in facts.iter().take(3) {
            if let Some(target) = self.other_topic_decision(s, topic) {
                let r = self.rel(main, f, target, MENTIONS, Provenance::Declared, true);
                self.led.nodes[f as usize].mention = Some(r);
            }
        }
        for (f, t) in ABOUT_MAIN {
            self.rel(main, facts[f], tasks[t], ABOUT, Provenance::Declared, true);
        }
        let mut classes = SOURCE_CLASSES;
        self.shuffle(&mut classes);
        for (&f, &class) in facts.iter().zip(&classes) {
            self.rel(main, f, sources[class], CITES, Provenance::Declared, true);
        }
        for &d in &decisions {
            let previous = self.led.chains[topic].last().copied();
            if let Some(previous) = previous {
                let r = self.rel(main, d, previous, REVISES, Provenance::Declared, true);
                self.led.nodes[d as usize].revises = Some(r);
            }
            let chain = &mut self.led.chains[topic];
            self.led.nodes[d as usize].chain_pos = chain.len() as u32;
            chain.push(d);
        }

        self.change(import, "importer", format!("{IMPORT_SYSTEM}/{s:04}"));
        let imported = Provenance::Imported(IMPORT_SYSTEM.to_owned());
        let r = self.rel(import, facts[1], decisions[1], SUPPORTS, imported.clone(), true);
        self.led.nodes[decisions[1] as usize].evidence = Some(r);
        let (f, t) = ABOUT_IMPORTED;
        self.rel(import, facts[f], tasks[t], ABOUT, imported, true);

        self.change(consolidate, "consolidator", format!("job/consolidate/{s:04}"));
        for &d in &decisions {
            let pos = self.led.nodes[d as usize].chain_pos as usize;
            // Both links of a decision are drawn first and written in a random
            // order, so that the creation order does not tell them apart.
            let mut links = Vec::new();
            if pos >= 2 {
                let target = self.led.chains[topic][pos - 2];
                links.push((target, self.rng.range(TRUE_CONFIDENCE.0, TRUE_CONFIDENCE.1) as u16, true));
            }
            if let Some(target) = self.other_topic_decision(s, topic) {
                links.push((target, self.rng.range(FALSE_CONFIDENCE.0, FALSE_CONFIDENCE.1) as u16, false));
            }
            self.shuffle(&mut links);
            for (target, confidence, truth) in links {
                self.rel(consolidate, d, target, RELATED, inferred(confidence), truth);
            }
        }
        // `same_source`: the pairs of facts that cite one source are true; the
        // dedup job also links other pairs, which is false. Pairs, their order and
        // the end each fact sits at are random, so no index, offset or creation
        // order decides the truth.
        let mut pairs = Vec::new();
        for a in 0..facts.len() {
            for b in a + 1..facts.len() {
                pairs.push((a, b, classes[a] == classes[b]));
            }
        }
        self.shuffle(&mut pairs);
        let mut links: Vec<_> = pairs.iter().filter(|p| p.2).copied().collect();
        links.extend(pairs.iter().filter(|p| !p.2).take(SAME_FALSE_PER_SESSION));
        self.shuffle(&mut links);
        for (a, b, truth) in links {
            let (from, to) = if self.rng.chance(1, 2) { (a, b) } else { (b, a) };
            let dedup = Provenance::Algorithmic(DEDUP_ORIGIN.to_owned());
            self.rel(consolidate, facts[from], facts[to], SAME_SOURCE, dedup, truth);
        }
    }

    /// Fisher-Yates, from the seeded generator.
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.rng.below(i as u64 + 1) as usize);
        }
    }
}

fn main_step(s: u32) -> u64 {
    3 * u64::from(s) + 1
}

fn transcript_source(s: u32) -> String {
    format!("transcript/{s:04}")
}

fn inferred(confidence_permille: u16) -> Provenance {
    Provenance::Inferred { origin: CONSOLIDATOR_ORIGIN.to_owned(), confidence_permille }
}

fn pick(rng: &mut Rng, list: &[u32]) -> u32 {
    assert!(!list.is_empty(), "every query family has seeds at every scale");
    list[rng.below(list.len() as u64) as usize]
}

/// A window of creation steps around the session of a seed: it always holds
/// the seed's own change sets, and up to two sessions on either side.
fn window_around(rng: &mut Rng, session: u32) -> (u64, u64) {
    let main = main_step(session);
    let lo = main.saturating_sub(3 * rng.below(3)).max(1);
    let hi = main + 3 * rng.below(3) + 2;
    (lo, hi)
}

/// Decisions of a session with enough history and a mention on their evidence.
/// `declared_evidence` keeps only seeds whose evidence is declared, so that a
/// query restricted to declared relations still reaches a distractor.
fn eligible_chain_seeds(c: &Corpus, led: &Ledger, declared_evidence: bool) -> Vec<u32> {
    c.units
        .iter()
        .filter(|u| u.unit_type == "decision")
        .map(|u| u.index)
        .filter(|&d| {
            let node = &led.nodes[d as usize];
            let pos = node.chain_pos as usize;
            if node.session < MIN_SEED_SESSION || pos < 4 || pos + 4 >= led.chain_of(d).len() {
                return false;
            }
            let Some(r) = node.evidence else { return false };
            let evidence = &c.relations[r as usize];
            let mentions = led.nodes[evidence.from as usize].mention.is_some();
            mentions && (!declared_evidence || evidence.provenance.kind() == Kind::Declared)
        })
        .collect()
}

/// Decisions with a true and a false `related` link, and a history to walk.
fn eligible_related_seeds(c: &Corpus, led: &Ledger) -> Vec<u32> {
    c.units
        .iter()
        .filter(|u| u.unit_type == "decision")
        .map(|u| u.index)
        .filter(|&d| {
            let node = &led.nodes[d as usize];
            let related: Vec<bool> = node
                .links
                .iter()
                .filter(|&&r| c.relations[r as usize].relation_type == RELATED)
                .map(|&r| led.truth[r as usize])
                .collect();
            node.session >= MIN_SEED_SESSION
                && node.chain_pos >= 2
                && related.contains(&true)
                && related.contains(&false)
        })
        .collect()
}

/// Facts with a true and a false `same_source` link.
fn eligible_same_source_seeds(c: &Corpus, led: &Ledger) -> Vec<u32> {
    c.units
        .iter()
        .filter(|u| u.unit_type == "fact")
        .map(|u| u.index)
        .filter(|&f| {
            let truths: Vec<bool> = led.nodes[f as usize]
                .links
                .iter()
                .filter(|&&r| c.relations[r as usize].relation_type == SAME_SOURCE)
                .map(|&r| led.truth[r as usize])
                .collect();
            truths.contains(&true) && truths.contains(&false)
        })
        .collect()
}

fn build_queries(c: &Corpus, led: &Ledger, seed: u64) -> Vec<Query> {
    let mut rng = Rng::new(seed ^ QUERY_STREAM);
    let chain_seeds = eligible_chain_seeds(c, led, false);
    let declared_seeds = eligible_chain_seeds(c, led, true);
    let related_seeds = eligible_related_seeds(c, led);
    let same_seeds = eligible_same_source_seeds(c, led);
    let mut out = Vec::new();

    // Multi-hop: the topic chain and its evidence, in four shapes.
    for n in 0..16u8 {
        let seed_unit = pick(&mut rng, &chain_seeds);
        let (direction, types, max_depth) = match n % 4 {
            0 => (Direction::Both, CHAIN_TYPES.to_vec(), 3),
            1 => (Direction::Both, CHAIN_TYPES.to_vec(), 4),
            2 => (Direction::Out, vec![REVISES], 4),
            _ => (Direction::In, vec![REVISES, SUPPORTS], 4),
        };
        let constraints = Constraints {
            direction,
            relation_types: owned(&types),
            max_depth,
            window: None,
            kinds: Vec::new(),
            actor: None,
            source: None,
        };
        let expected = expect_chain(c, led, seed_unit, &constraints);
        out.push(make(c, format!("MH-{:02}", n + 1), Class::MultiHop, seed_unit, constraints, expected));
    }

    // Temporal: the same chain, read through a window over the commit order.
    for n in 0..16u8 {
        let seed_unit = pick(&mut rng, &chain_seeds);
        let window = window_around(&mut rng, led.nodes[seed_unit as usize].session);
        let constraints = Constraints {
            direction: Direction::Both,
            relation_types: owned(&CHAIN_TYPES),
            max_depth: 3 + n % 2,
            window: Some(window),
            kinds: Vec::new(),
            actor: None,
            source: None,
        };
        let expected = expect_chain(c, led, seed_unit, &constraints);
        out.push(make(c, format!("TM-{:02}", n + 1), Class::Temporal, seed_unit, constraints, expected));
    }

    // Combined: declared relations only, a window, and sometimes the seed's agent.
    for n in 0..16u8 {
        let seed_unit = pick(&mut rng, &declared_seeds);
        let window = window_around(&mut rng, led.nodes[seed_unit as usize].session);
        let actor = (n % 2 == 0).then(|| led.session_of(seed_unit).actor.to_owned());
        let constraints = Constraints {
            direction: Direction::Both,
            relation_types: owned(&CHAIN_TYPES),
            max_depth: 3 + n % 2,
            window: Some(window),
            kinds: vec![Kind::Declared],
            actor,
            source: None,
        };
        let expected = expect_chain(c, led, seed_unit, &constraints);
        out.push(make(c, format!("CB-{:02}", n + 1), Class::Combined, seed_unit, constraints, expected));
    }

    // Provenance: a kind (related inferences, same-source algorithms), an actor, a source.
    for n in 0..4u8 {
        let seed_unit = pick(&mut rng, &related_seeds);
        let constraints = Constraints {
            direction: Direction::Both,
            relation_types: owned(&[RELATED]),
            max_depth: 1,
            window: None,
            kinds: vec![Kind::Inferred],
            actor: None,
            source: None,
        };
        let expected = expect_links(c, led, seed_unit, RELATED, 2, &constraints);
        out.push(make(c, format!("PV-RE-{}", n + 1), Class::Provenance, seed_unit, constraints, expected));
    }
    for n in 0..4u8 {
        let seed_unit = pick(&mut rng, &same_seeds);
        let constraints = Constraints {
            direction: Direction::Both,
            relation_types: owned(&[SAME_SOURCE]),
            max_depth: 1,
            window: None,
            kinds: vec![Kind::Algorithmic],
            actor: None,
            source: None,
        };
        let expected = expect_links(c, led, seed_unit, SAME_SOURCE, 3, &constraints);
        out.push(make(c, format!("PV-SS-{}", n + 1), Class::Provenance, seed_unit, constraints, expected));
    }
    for n in 0..4u8 {
        let seed_unit = pick(&mut rng, &declared_seeds);
        let constraints = Constraints {
            direction: Direction::Both,
            relation_types: owned(&CHAIN_TYPES),
            max_depth: 3 + n % 2,
            window: None,
            kinds: Vec::new(),
            actor: Some(led.session_of(seed_unit).actor.to_owned()),
            source: None,
        };
        let expected = expect_chain(c, led, seed_unit, &constraints);
        out.push(make(c, format!("PV-AC-{}", n + 1), Class::Provenance, seed_unit, constraints, expected));
    }
    for n in 0..4u8 {
        let seed_unit = pick(&mut rng, &declared_seeds);
        let session = led.nodes[seed_unit as usize].session;
        let constraints = Constraints {
            direction: Direction::Both,
            relation_types: owned(&CHAIN_TYPES),
            max_depth: 3 + n % 2,
            window: None,
            kinds: Vec::new(),
            actor: None,
            source: Some(transcript_source(session)),
        };
        let expected = expect_chain(c, led, seed_unit, &constraints);
        out.push(make(c, format!("PV-SR-{}", n + 1), Class::Provenance, seed_unit, constraints, expected));
    }
    out
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

fn make(c: &Corpus, id: String, class: Class, seed: u32, constraints: Constraints, expected: Vec<Judgment>) -> Query {
    Query { id, class, seed, unit_type: c.units[seed as usize].unit_type.clone(), constraints, expected }
}

/// Whether a relation satisfies every filter of the constraints.
fn admits(c: &Corpus, cons: &Constraints, rel: &Relation) -> bool {
    let change = &c.changes[rel.step as usize - 1];
    cons.relation_types.contains(&rel.relation_type)
        && (cons.kinds.is_empty() || cons.kinds.contains(&rel.provenance.kind()))
        && cons.window.is_none_or(|(lo, hi)| (lo..=hi).contains(&rel.step))
        && cons.actor.as_ref().is_none_or(|a| *a == change.actor)
        && cons.source.as_ref().is_none_or(|s| *s == change.source)
}

/// Whether a walk at a unit may follow a relation, given the direction.
fn may_follow(direction: Direction, from_here: bool) -> bool {
    match direction {
        Direction::Both => true,
        Direction::Out => from_here,
        Direction::In => !from_here,
    }
}

/// Grade of a decision `depth` revisions away from the seed.
fn decision_grade(depth: u8) -> u8 {
    match depth {
        1 => 3,
        2 => 2,
        _ => 0,
    }
}

/// Grade of the evidence of a decision `depth` revisions away (seed: 0).
fn evidence_grade(depth: u8) -> u8 {
    match depth {
        0 => 3,
        1 => 2,
        2 => 1,
        _ => 0,
    }
}

fn raise(grades: &mut BTreeMap<u32, u8>, index: u32, grade: u8) {
    let slot = grades.entry(index).or_insert(0);
    *slot = (*slot).max(grade);
}

/// The decisions a walk along the topic chain reaches from `start`, one
/// revision per step in direction `step`, with their depth.
fn walk_chain(
    c: &Corpus,
    led: &Ledger,
    cons: &Constraints,
    chain: &[u32],
    start: usize,
    step: isize,
) -> Vec<(u32, u8)> {
    let mut found = Vec::new();
    let (mut cur, mut depth) = (start, 0u8);
    while let Some(next) = cur.checked_add_signed(step).filter(|&n| n < chain.len()) {
        // The link between two neighbours is the `revises` of the newer one.
        let Some(r) = led.nodes[chain[cur.max(next)] as usize].revises else { break };
        let rel = &c.relations[r as usize];
        let allowed = admits(c, cons, rel) && may_follow(cons.direction, rel.from == chain[cur]);
        if !allowed || depth >= cons.max_depth {
            break;
        }
        depth += 1;
        found.push((chain[next], depth));
        cur = next;
    }
    found
}

/// The expected answers of a chain query from `seed`: the decisions of its
/// topic that the walks reach, their evidence, and the cross-topic mentions
/// of that evidence (grade 0). Walks follow the story's own chain.
fn expect_chain(c: &Corpus, led: &Ledger, seed: u32, cons: &Constraints) -> Vec<Judgment> {
    let chain = led.chain_of(seed);
    let start = led.nodes[seed as usize].chain_pos as usize;
    let mut members = vec![(seed, 0u8)];
    members.extend(walk_chain(c, led, cons, chain, start, -1));
    members.extend(walk_chain(c, led, cons, chain, start, 1));

    let mut grades = BTreeMap::new();
    for &(decision, depth) in &members {
        if depth > 0 {
            raise(&mut grades, decision, decision_grade(depth));
        }
        let Some(r) = led.nodes[decision as usize].evidence else { continue };
        let support = &c.relations[r as usize];
        if !admits(c, cons, support) || !may_follow(cons.direction, support.from == decision) || depth >= cons.max_depth
        {
            continue;
        }
        let fact = support.from;
        raise(&mut grades, fact, evidence_grade(depth));
        let Some(m) = led.nodes[fact as usize].mention else { continue };
        let mention = &c.relations[m as usize];
        let reachable = admits(c, cons, mention) && may_follow(cons.direction, mention.from == fact);
        if reachable && depth + 1 < cons.max_depth {
            raise(&mut grades, mention.to, 0);
        }
    }
    grades.into_iter().map(|(index, grade)| Judgment { index, grade }).collect()
}

/// The expected answers of a one-hop query over `relation_type` links from
/// `seed`: a true link gets `true_grade`, a false one grade 0.
fn expect_links(
    c: &Corpus,
    led: &Ledger,
    seed: u32,
    relation_type: &str,
    true_grade: u8,
    cons: &Constraints,
) -> Vec<Judgment> {
    let mut out: Vec<Judgment> = led.nodes[seed as usize]
        .links
        .iter()
        .filter_map(|&r| {
            let rel = &c.relations[r as usize];
            if rel.relation_type != relation_type || !admits(c, cons, rel) {
                return None;
            }
            if !may_follow(cons.direction, rel.from == seed) {
                return None;
            }
            let other = if rel.from == seed { rel.to } else { rel.from };
            let grade = if led.truth[r as usize] { true_grade } else { 0 };
            Some(Judgment { index: other, grade })
        })
        .collect();
    out.sort_by_key(|j| j.index);
    out
}

/// A corpus, rendered as the text of `data/assoc-eval/corpus.txt`.
///
/// Records, one per line: `C step actor source` opens a change set;
/// `U index step type label` creates a unit; `R index step from to type
/// provenance` creates a relation, where provenance is `declared`,
/// `imported <source>`, `algorithmic <origin>` or `inferred <origin>
/// <confidence_permille>`.
pub fn render_corpus(c: &Corpus) -> String {
    let mut out = String::new();
    writeln!(out, "# kerno-testkit assoc-eval corpus v1").expect(WRITE);
    writeln!(out, "seed {} scale {}", c.seed, c.scale.name()).expect(WRITE);
    writeln!(out, "counts units {} relations {} changes {}", c.units.len(), c.relations.len(), c.changes.len())
        .expect(WRITE);
    let (mut next_unit, mut next_rel) = (0, 0);
    for change in &c.changes {
        writeln!(out, "C {} {} {}", change.step, change.actor, change.source).expect(WRITE);
        while let Some(u) = c.units.get(next_unit).filter(|u| u.step == change.step) {
            writeln!(out, "U {} {} {} {}", u.index, u.step, u.unit_type, u.label).expect(WRITE);
            next_unit += 1;
        }
        while let Some(r) = c.relations.get(next_rel).filter(|r| r.step == change.step) {
            let prov = provenance_text(&r.provenance);
            writeln!(out, "R {} {} {} {} {} {}", r.index, r.step, r.from, r.to, r.relation_type, prov).expect(WRITE);
            next_rel += 1;
        }
    }
    out
}

fn provenance_text(p: &Provenance) -> String {
    match p {
        Provenance::Declared => "declared".to_owned(),
        Provenance::Imported(source) => format!("imported {source}"),
        Provenance::Algorithmic(origin) => format!("algorithmic {origin}"),
        Provenance::Inferred { origin, confidence_permille } => format!("inferred {origin} {confidence_permille}"),
    }
}

/// The query set, rendered as the text of `data/assoc-eval/queries.txt`.
///
/// `Q id class seed type direction types depth window kinds actor source`
/// opens a query (`-` for an absent field, `lo-hi` for a window, commas
/// between list items), followed by one `E index grade` line per expected answer.
pub fn render_queries(set: &QuerySet) -> String {
    let mut out = String::new();
    writeln!(out, "# kerno-testkit assoc-eval queries v1").expect(WRITE);
    writeln!(out, "seed {} scale {} cut {}", set.seed, set.scale.name(), CUT).expect(WRITE);
    writeln!(out, "counts queries {}", set.queries.len()).expect(WRITE);
    for q in &set.queries {
        let k = &q.constraints;
        let window = k.window.map_or_else(|| "-".to_owned(), |(lo, hi)| format!("{lo}-{hi}"));
        let kinds = if k.kinds.is_empty() {
            "-".to_owned()
        } else {
            k.kinds.iter().map(|kind| kind.name()).collect::<Vec<_>>().join(",")
        };
        let actor = k.actor.as_deref().unwrap_or("-");
        let source = k.source.as_deref().unwrap_or("-");
        let types = k.relation_types.join(",");
        writeln!(
            out,
            "Q {} {} {} {} {} {} {} {} {} {} {}",
            q.id,
            q.class.name(),
            q.seed,
            q.unit_type,
            k.direction.name(),
            types,
            k.max_depth,
            window,
            kinds,
            actor,
            source
        )
        .expect(WRITE);
        for j in &q.expected {
            writeln!(out, "E {} {}", j.index, j.grade).expect(WRITE);
        }
    }
    out
}

/// A malformed line of a frozen file: where it is and what is wrong.
#[derive(Debug, PartialEq, Eq)]
pub struct FormatError {
    pub line: usize,
    pub reason: String,
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

fn number<T: FromStr>(line: usize, token: Option<&str>, what: &str) -> Result<T, FormatError> {
    let token = token.ok_or_else(|| bad(line, format!("missing {what}")))?;
    token.parse().map_err(|_| bad(line, format!("{what} {token:?} is not a number")))
}

fn expect_word(line: usize, token: Option<&str>, word: &str) -> Result<(), FormatError> {
    match token {
        Some(t) if t == word => Ok(()),
        _ => Err(bad(line, format!("expected {word:?}"))),
    }
}

/// The meaningful lines of a frozen file: `(line number, text)`, without
/// comments and blank lines.
fn records(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().map(|(i, l)| (i + 1, l)).filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
}

/// The manifest of frozen files: one `digest  name` line per file, in the
/// format `sha256sum` writes and checks (`sha256sum -c MANIFEST`).
pub fn manifest_text(files: &[(&str, &[u8])]) -> String {
    files.iter().map(|(name, bytes)| format!("{}  {name}\n", sha256::hex(&sha256::digest(bytes)))).collect()
}

/// Parses a manifest written by [`manifest_text`] into `(digest, name)` pairs.
pub fn parse_manifest(text: &str) -> Result<Vec<(String, String)>, FormatError> {
    records(text)
        .map(|(n, line)| {
            let (digest, name) = line.split_once("  ").ok_or_else(|| bad(n, "expected 'digest  name'"))?;
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad(n, "the digest is not 64 hex digits"));
            }
            Ok((digest.to_owned(), name.to_owned()))
        })
        .collect()
}

/// Parses the text written by [`render_corpus`].
pub fn parse_corpus(text: &str) -> Result<Corpus, FormatError> {
    let mut lines = records(text);
    let (n, header) = lines.next().ok_or_else(|| bad(0, "empty corpus"))?;
    let mut t = header.split_whitespace();
    expect_word(n, t.next(), "seed")?;
    let seed = number(n, t.next(), "seed")?;
    expect_word(n, t.next(), "scale")?;
    let scale_name = t.next().ok_or_else(|| bad(n, "missing scale"))?;
    let scale = Scale::from_name(scale_name).ok_or_else(|| bad(n, format!("unknown scale {scale_name:?}")))?;
    let (n, counts) = lines.next().ok_or_else(|| bad(n, "missing counts"))?;
    let mut t = counts.split_whitespace();
    expect_word(n, t.next(), "counts")?;
    expect_word(n, t.next(), "units")?;
    let units_expected: usize = number(n, t.next(), "unit count")?;
    expect_word(n, t.next(), "relations")?;
    let relations_expected: usize = number(n, t.next(), "relation count")?;
    expect_word(n, t.next(), "changes")?;
    let changes_expected: usize = number(n, t.next(), "change count")?;

    let mut c = Corpus { seed, scale, changes: Vec::new(), units: Vec::new(), relations: Vec::new() };
    for (n, line) in lines {
        let mut t = line.split_whitespace();
        match t.next() {
            Some("C") => {
                let step: u64 = number(n, t.next(), "step")?;
                if step != c.changes.len() as u64 + 1 {
                    return Err(bad(n, format!("change step {step} is not the next one")));
                }
                let actor = t.next().ok_or_else(|| bad(n, "missing actor"))?;
                let source = t.next().ok_or_else(|| bad(n, "missing source"))?;
                c.changes.push(Change { step, actor: actor.to_owned(), source: source.to_owned() });
            }
            Some("U") => {
                let index: u32 = number(n, t.next(), "unit index")?;
                let step: u64 = number(n, t.next(), "step")?;
                let unit_type = t.next().ok_or_else(|| bad(n, "missing unit type"))?;
                let label = t.next().ok_or_else(|| bad(n, "missing label"))?;
                if index as usize != c.units.len() || !step_is_current(&c, step) {
                    return Err(bad(n, format!("unit {index} is out of order")));
                }
                c.units.push(Unit { index, step, unit_type: unit_type.to_owned(), label: label.to_owned() });
            }
            Some("R") => {
                let index: u32 = number(n, t.next(), "relation index")?;
                let step: u64 = number(n, t.next(), "step")?;
                let from: u32 = number(n, t.next(), "from")?;
                let to: u32 = number(n, t.next(), "to")?;
                let relation_type = t.next().ok_or_else(|| bad(n, "missing relation type"))?;
                let provenance = parse_provenance(n, &mut t)?;
                if index as usize != c.relations.len() || !step_is_current(&c, step) {
                    return Err(bad(n, format!("relation {index} is out of order")));
                }
                let ends_exist = (from as usize) < c.units.len() && (to as usize) < c.units.len();
                if !ends_exist || c.units[from as usize].step > step || c.units[to as usize].step > step {
                    return Err(bad(n, format!("relation {index} has an end created after its step")));
                }
                c.relations.push(Relation {
                    index,
                    step,
                    from,
                    to,
                    relation_type: relation_type.to_owned(),
                    provenance,
                });
            }
            other => return Err(bad(n, format!("unknown record {other:?}"))),
        }
    }
    if c.units.len() != units_expected || c.relations.len() != relations_expected || c.changes.len() != changes_expected
    {
        return Err(bad(0, "the counts in the header do not match the records"));
    }
    Ok(c)
}

/// Whether `step` is the step of the change set currently being read.
fn step_is_current(c: &Corpus, step: u64) -> bool {
    c.changes.last().is_some_and(|change| change.step == step)
}

fn parse_provenance<'a>(n: usize, t: &mut impl Iterator<Item = &'a str>) -> Result<Provenance, FormatError> {
    match t.next() {
        Some("declared") => Ok(Provenance::Declared),
        Some("imported") => {
            let source = t.next().ok_or_else(|| bad(n, "missing import source"))?;
            Ok(Provenance::Imported(source.to_owned()))
        }
        Some("algorithmic") => {
            let origin = t.next().ok_or_else(|| bad(n, "missing algorithm origin"))?;
            Ok(Provenance::Algorithmic(origin.to_owned()))
        }
        Some("inferred") => {
            let origin = t.next().ok_or_else(|| bad(n, "missing inference origin"))?;
            let confidence_permille = number(n, t.next(), "confidence")?;
            if confidence_permille > 1000 {
                return Err(bad(n, "confidence above 1000 permille"));
            }
            Ok(Provenance::Inferred { origin: origin.to_owned(), confidence_permille })
        }
        other => Err(bad(n, format!("unknown provenance {other:?}"))),
    }
}

/// Parses the text written by [`render_queries`].
pub fn parse_queries(text: &str) -> Result<QuerySet, FormatError> {
    let mut lines = records(text);
    let (n, header) = lines.next().ok_or_else(|| bad(0, "empty query set"))?;
    let mut t = header.split_whitespace();
    expect_word(n, t.next(), "seed")?;
    let seed = number(n, t.next(), "seed")?;
    expect_word(n, t.next(), "scale")?;
    let scale_name = t.next().ok_or_else(|| bad(n, "missing scale"))?;
    let scale = Scale::from_name(scale_name).ok_or_else(|| bad(n, format!("unknown scale {scale_name:?}")))?;
    expect_word(n, t.next(), "cut")?;
    let cut: usize = number(n, t.next(), "cut")?;
    if cut != CUT {
        return Err(bad(n, format!("cut {cut} is not the agreed {CUT}")));
    }
    let (n, counts) = lines.next().ok_or_else(|| bad(n, "missing counts"))?;
    let mut t = counts.split_whitespace();
    expect_word(n, t.next(), "counts")?;
    expect_word(n, t.next(), "queries")?;
    let count: usize = number(n, t.next(), "query count")?;

    let mut queries: Vec<Query> = Vec::new();
    for (n, line) in lines {
        let mut t = line.split_whitespace();
        match t.next() {
            Some("Q") => queries.push(parse_query_line(n, &mut t)?),
            Some("E") => {
                let index: u32 = number(n, t.next(), "expected index")?;
                let grade: u8 = number(n, t.next(), "grade")?;
                let query = queries.last_mut().ok_or_else(|| bad(n, "E line before any Q"))?;
                if grade > MAX_GRADE {
                    return Err(bad(n, format!("grade {grade} is above {MAX_GRADE}")));
                }
                query.expected.push(Judgment { index, grade });
            }
            other => return Err(bad(n, format!("unknown record {other:?}"))),
        }
    }
    if queries.len() != count {
        return Err(bad(0, "the query count in the header does not match the records"));
    }
    Ok(QuerySet { seed, scale, queries })
}

fn parse_query_line<'a>(n: usize, t: &mut impl Iterator<Item = &'a str>) -> Result<Query, FormatError> {
    let mut field = || t.next().ok_or_else(|| bad(n, "missing field of a query"));
    let id = field()?.to_owned();
    let class_name = field()?;
    let class = Class::from_name(class_name).ok_or_else(|| bad(n, format!("unknown class {class_name:?}")))?;
    let seed: u32 = number(n, Some(field()?), "seed unit")?;
    let unit_type = field()?.to_owned();
    let dir_name = field()?;
    let direction = Direction::from_name(dir_name).ok_or_else(|| bad(n, format!("unknown direction {dir_name:?}")))?;
    let relation_types: Vec<String> = field()?.split(',').map(str::to_owned).collect();
    let max_depth: u8 = number(n, Some(field()?), "depth")?;
    let window = match field()? {
        "-" => None,
        w => {
            let (lo, hi) = w.split_once('-').ok_or_else(|| bad(n, "window is not lo-hi"))?;
            Some((number(n, Some(lo), "window start")?, number(n, Some(hi), "window end")?))
        }
    };
    let kinds: Vec<Kind> = match field()? {
        "-" => Vec::new(),
        list => list
            .split(',')
            .map(|k| Kind::from_name(k).ok_or_else(|| bad(n, format!("unknown kind {k:?}"))))
            .collect::<Result<_, _>>()?,
    };
    let actor = match field()? {
        "-" => None,
        a => Some(a.to_owned()),
    };
    let source = match field()? {
        "-" => None,
        s => Some(s.to_owned()),
    };
    let constraints = Constraints { direction, relation_types, max_depth, window, kinds, actor, source };
    Ok(Query { id, class, seed, unit_type, constraints, expected: Vec::new() })
}
