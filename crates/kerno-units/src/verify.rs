//! The offline verifier of the units layer (`etapa3-design.md` §8.5, E3-07).
//!
//! [`verify_snapshot`] walks every key of one engine snapshot, in key order
//! and in bounded pages, **without** going through the read API, and reports
//! every rule it finds broken as a [`VerifyFinding`] carrying the id of a
//! `UNIT-n` rule (`units-v1.md` §5, §5.1 and §5.2), like the engine's
//! `verify()`. [`UnitDb::verify_units`] and [`UnitSnapshot::verify_units`]
//! run it on the layer's own snapshot; [`verify_engine`] runs it on any
//! engine, including one whose layer mark is damaged (which
//! [`UnitDb::open`] refuses) or one opened read-only.
//!
//! # What is checked
//!
//! * **Every record decodes** (UNIT-1..=UNIT-25), unknown tags included:
//!   inside a known range (UNIT-1) they are `Corruption`, above it (UNIT-2)
//!   `IncompatibleLayer`.
//! * **The layer mark** (UNIT-29): a store with keys has the `LAYER_META`
//!   record `layer`; every other `LAYER_META` name is `policy/<unit_type>`.
//! * **Orphans** (UNIT-30): `REV_META`/`REV_PAYLOAD` without `UNIT_HEAD`,
//!   `REV_PAYLOAD` without its `REV_META`, `CS_MEMBER` without `CHANGESET`.
//! * **Payloads** (UNIT-26..=UNIT-28, §5.1): every `REV_META` against its
//!   `REV_PAYLOAD` — presence, length, then hash. A non-tombstone revision
//!   with `payload_present = 0` is one an `Audit` period emptied (hash and
//!   length are kept, "Nota de E3-05"): it must have no `REV_PAYLOAD`.
//! * **Heads** (UNIT-31, IU-2): `REV_META` covers exactly
//!   `[oldest_rev, head_rev]`; the head agrees with its `REV_META` (hash,
//!   length, schema, metadata, `updated_seq`, lifecycle) and keeps its
//!   payload; `created_seq` is revision 1's `commit_seq` while it is retained.
//! * **Chains** (UNIT-32, IU-3): `Create` exactly at revision 1,
//!   `parent_rev = rev - 1`, `commit_seq` strictly increasing and not above
//!   the snapshot, lifecycle transitions a writer can produce, and the copies
//!   `Archive`/`Unarchive`/`Restore` equal to their retained source.
//! * **Provenance** (UNIT-33, IU-4): every `REV_META` has an `Application`
//!   `CHANGESET` at its `commit_seq` with a member naming it; members are
//!   numbered `0..member_count`, name a unit at most once, and each member of
//!   an `Application` change set names a revision with that `commit_seq` and
//!   kind (or one already pruned). Members of a `Retention` change set are
//!   checked only in the `REV_META → member` direction ("Nota de E3-06"):
//!   they name the first retained revision, not one they created.
//! * **History policies** (UNIT-34, IU-9), where the policy in force when a
//!   revision was written is known: the policy in force now, for every
//!   revision written after the last `Maintenance` change set. `None` leaves
//!   `oldest_rev = head_rev`; `Audit` leaves every superseded revision
//!   without payload.
//! * **Immutability** (UNIT-35, IU-1) between two snapshots:
//!   [`verify_immutable`].
//!
//! # Contracts
//!
//! * **Read-only (I-18).** It only reads one snapshot; nothing is written,
//!   repaired or skipped.
//! * **Damage is a finding, not an error.** Engine errors (`Io`,
//!   `Corruption` found by the engine itself, `Poisoned`) are returned as
//!   `Err`; everything the layer finds is in the report.
//!   [`UnitVerifyReport::into_result`] turns a report into the error the read
//!   API would give.
//! * **No content (IU-10).** Details name record types, ids and numbers only.
//!   One broken record produces one finding: records that do not decode are
//!   left out of the checks between records, so the damage is not reported
//!   again as "missing".
//! * **Bounded memory, proportional to the records.** Keys are read in pages
//!   of [`VERIFY_PAGE`], but every `UNIT_HEAD`, `REV_META`, `CHANGESET` and
//!   `CS_MEMBER` of the store is kept decoded in full (metadata included, up
//!   to 4096 B each), plus the key of every `REV_PAYLOAD`; payloads never.
//!   Worst case, about 4.6 KB per revision and 2.5 KB per change set.
//!   [`verify_immutable`] also keeps every `REV_META` of the older snapshot
//!   (encoded) and a 32-byte digest per `REV_PAYLOAD`. Work never depends on
//!   the numbers a record claims (a damaged `head_rev`), only on the records
//!   that exist.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot, TxnEngine};

use crate::codec::{
    ChangeSetRecord, CsMember, Key, LAYER_MARK_NAME, LayerMeta, Record, RevisionMeta, TAG_REV_META, TAG_REV_PAYLOAD,
    UnitHead,
};
use crate::db::{MAX_POLICY_UNIT_TYPE_LEN, UnitDb, UnitSnapshot};
use crate::error::{DecodeError, ErrorClass, UnitError, UnitRule};
use crate::payload::{PayloadRule, verify_payload};
use crate::types::{CommitSeq, HistoryPolicy, Lifecycle, Purpose, RevNo, RevisionKind, UnitId};

/// Keys read per engine scan.
pub const VERIFY_PAGE: usize = 32;

/// Findings kept in a report; the rest are only counted
/// ([`UnitVerifyReport::findings_omitted`]).
pub const MAX_FINDINGS: usize = 1024;

/// The rules between records of `units-v1.md` §5.2 (E3-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CrossRule {
    /// Layer mark missing from a store with keys, or an unknown `LAYER_META` name.
    Unit29,
    /// A record whose owner is missing.
    Unit30,
    /// A head that disagrees with its revisions (IU-2).
    Unit31,
    /// A revision chain no writer produces (IU-3).
    Unit32,
    /// Provenance: change sets and members (IU-4).
    Unit33,
    /// A history policy not applied (IU-9).
    Unit34,
    /// A revision that changed between two snapshots (IU-1).
    Unit35,
}

impl CrossRule {
    pub fn number(self) -> u8 {
        29 + self as u8
    }
}

/// The rule a finding breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VerifyRule {
    /// A record that does not decode (`units-v1.md` §5).
    Structure(UnitRule),
    /// A payload against its `REV_META` (`units-v1.md` §5.1).
    Payload(PayloadRule),
    /// A rule between records (`units-v1.md` §5.2).
    Cross(CrossRule),
}

impl VerifyRule {
    /// `"UNIT-n"`.
    pub fn id(self) -> String {
        match self {
            VerifyRule::Structure(r) => r.id(),
            VerifyRule::Payload(r) => r.id().to_owned(),
            VerifyRule::Cross(r) => format!("UNIT-{}", r.number()),
        }
    }

    /// `IncompatibleLayer` only for UNIT-2, UNIT-7 and UNIT-16; every other
    /// rule is `Corruption`.
    pub fn class(self) -> ErrorClass {
        match self {
            VerifyRule::Structure(r) => r.class(),
            _ => ErrorClass::Corruption,
        }
    }
}

impl fmt::Display for VerifyRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id())
    }
}

/// One broken rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyFinding {
    pub rule: VerifyRule,
    /// The key of the record the finding is about (for a rule between
    /// records, the record that names the missing or disagreeing one).
    pub key: Vec<u8>,
    /// Record types, ids and numbers; never content (IU-10).
    pub detail: String,
    /// For UNIT-16, the `layer_version` found.
    found_layer_version: Option<u32>,
}

impl VerifyFinding {
    pub fn class(&self) -> ErrorClass {
        self.rule.class()
    }

    /// The error the read API gives for this damage: `Storage(Corruption)`,
    /// `Storage(Incompatible)`, or [`UnitError::IncompatibleLayer`] for a
    /// newer `layer_version`.
    pub fn to_error(&self) -> UnitError {
        if let (VerifyRule::Structure(UnitRule::Unit16), Some(found)) = (self.rule, self.found_layer_version) {
            return UnitError::IncompatibleLayer { found, supported: crate::codec::LAYER_VERSION };
        }
        let msg = self.to_string();
        UnitError::Storage(match self.class() {
            ErrorClass::Corruption => kerno_core::Error::Corruption(msg),
            ErrorClass::IncompatibleLayer => kerno_core::Error::Incompatible(msg),
        })
    }
}

impl fmt::Display for VerifyFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.rule, self.class(), self.detail)
    }
}

/// Records seen by the verifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerifyCounts {
    pub layer_records: u64,
    pub units: u64,
    pub revisions: u64,
    pub payloads: u64,
    pub change_sets: u64,
    pub members: u64,
    /// Keys with a tag this layer does not know.
    pub unknown: u64,
}

/// Result of [`verify_snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitVerifyReport {
    /// The snapshot verified.
    pub seq: CommitSeq,
    pub counts: VerifyCounts,
    /// At most [`MAX_FINDINGS`], in the order found.
    pub findings: Vec<VerifyFinding>,
    /// Findings beyond [`MAX_FINDINGS`], counted but not kept.
    pub findings_omitted: u64,
}

impl UnitVerifyReport {
    /// No finding at all.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.findings_omitted == 0
    }

    /// `Ok(self)` when clean; otherwise the error of the first
    /// `IncompatibleLayer` finding (nothing of a newer layer is
    /// interpreted), else of the first finding.
    pub fn into_result(self) -> Result<Self, UnitError> {
        let first = self.findings.iter().find(|f| f.class() == ErrorClass::IncompatibleLayer).or(self.findings.first());
        match first {
            Some(f) => Err(f.to_error()),
            None => Ok(self),
        }
    }

    /// The ids of the rules found, without repetition, in order.
    pub fn rules(&self) -> BTreeSet<String> {
        self.findings.iter().map(|f| f.rule.id()).collect()
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// Verifies the whole store at the latest commit (see the [module
    /// documentation](self)). Read-only.
    pub fn verify_units(&self) -> Result<UnitVerifyReport, UnitError> {
        self.snapshot()?.verify_units()
    }
}

impl<E: Engine> UnitSnapshot<E> {
    /// Verifies the whole store at this snapshot. Read-only.
    pub fn verify_units(&self) -> Result<UnitVerifyReport, UnitError> {
        verify_snapshot(&self.snap)
    }
}

/// Verifies an engine that may not open as a units database (a damaged or
/// missing mark is a finding here, not a refusal). Read-only: one snapshot.
pub fn verify_engine<E: Engine>(engine: &E) -> Result<UnitVerifyReport, UnitError> {
    verify_snapshot(&engine.snapshot()?)
}

/// Calls `f` on every pair with a key in `[start, end)` (`end = None`: to
/// the end), in key order, [`VERIFY_PAGE`] pairs per scan.
fn for_each_pair<S: Snapshot>(
    snap: &S,
    start: &[u8],
    end: Option<&[u8]>,
    mut f: impl FnMut(Vec<u8>, Vec<u8>),
) -> Result<(), UnitError> {
    let mut last: Option<Vec<u8>> = None;
    loop {
        let lo = match &last {
            Some(k) => Bound::Excluded(k.as_slice()),
            None => Bound::Included(start),
        };
        let hi = end.map_or(Bound::Unbounded, Bound::Excluded);
        let page = snap.scan(lo, hi, VERIFY_PAGE)?;
        let n = page.len();
        for (k, v) in page {
            last = Some(k.clone());
            f(k, v);
        }
        if n < VERIFY_PAGE {
            return Ok(());
        }
    }
}

/// What the walk collected.
#[derive(Default)]
struct State {
    seq: u64,
    counts: VerifyCounts,
    findings: Vec<VerifyFinding>,
    omitted: u64,
    any_key: bool,
    mark: Option<LayerMeta>,
    bad_mark: bool,
    policies: BTreeMap<String, HistoryPolicy>,
    heads: BTreeMap<UnitId, UnitHead>,
    bad_heads: BTreeSet<UnitId>,
    metas: BTreeMap<(UnitId, RevNo), RevisionMeta>,
    bad_metas: BTreeSet<(UnitId, RevNo)>,
    payloads: BTreeSet<(UnitId, RevNo)>,
    change_sets: BTreeMap<CommitSeq, ChangeSetRecord>,
    bad_change_sets: BTreeSet<CommitSeq>,
    members: BTreeMap<CommitSeq, Vec<(u32, CsMember)>>,
    bad_members: BTreeSet<CommitSeq>,
}

fn key_of(k: Key) -> Vec<u8> {
    // Every key built here comes from a decoded one, so it encodes.
    k.encode().unwrap_or_default()
}

/// The record a key names, by ids and numbers only (a `LAYER_META` name may
/// carry a unit type, so only its length is given).
fn describe(k: &Key) -> String {
    match k {
        Key::LayerMeta { name } if name == LAYER_MARK_NAME => "LAYER_META \"layer\"".into(),
        Key::LayerMeta { name } => format!("LAYER_META name of {} bytes", name.len()),
        Key::UnitHead { unit } => format!("UNIT_HEAD of unit {unit}"),
        Key::RevMeta { unit, rev } => format!("REV_META of unit {unit} rev {rev}"),
        Key::RevPayload { unit, rev } => format!("REV_PAYLOAD of unit {unit} rev {rev}"),
        Key::ChangeSet { seq } => format!("CHANGESET {seq}"),
        Key::CsMember { seq, ordinal } => format!("CS_MEMBER {seq}/{ordinal}"),
    }
}

/// The lifecycle a revision of `kind` leaves its unit in.
fn lifecycle_after(kind: RevisionKind) -> Lifecycle {
    match kind {
        RevisionKind::Archive => Lifecycle::Archived,
        RevisionKind::Delete => Lifecycle::Deleted,
        _ => Lifecycle::Active,
    }
}

/// Whether a writer can write a revision of `kind` over a unit in `state`
/// (design §3.2, notes of E3-03 and E3-04).
fn transition_ok(state: Lifecycle, kind: RevisionKind) -> bool {
    match kind {
        RevisionKind::Create => false,
        RevisionKind::Update | RevisionKind::Archive | RevisionKind::Restore => state == Lifecycle::Active,
        RevisionKind::Unarchive => state == Lifecycle::Archived,
        RevisionKind::Delete => state != Lifecycle::Deleted,
    }
}

/// Copies carry their source's content (design §2.4, §3.2).
fn same_content(a: &RevisionMeta, b: &RevisionMeta) -> bool {
    a.content_hash == b.content_hash
        && a.payload_len == b.payload_len
        && a.schema_version == b.schema_version
        && a.metadata == b.metadata
}

impl State {
    fn push(&mut self, rule: VerifyRule, key: Vec<u8>, detail: String) {
        self.push_finding(VerifyFinding { rule, key, detail, found_layer_version: None });
    }

    fn push_finding(&mut self, f: VerifyFinding) {
        if self.findings.len() < MAX_FINDINGS {
            self.findings.push(f);
        } else {
            self.omitted += 1;
        }
    }

    fn cross(&mut self, rule: CrossRule, key: Vec<u8>, detail: String) {
        self.push(VerifyRule::Cross(rule), key, detail);
    }

    fn decode_error(&mut self, key: Vec<u8>, e: DecodeError) {
        let found_layer_version = e.found_layer_version;
        self.push_finding(VerifyFinding {
            rule: VerifyRule::Structure(e.rule),
            key,
            detail: e.detail,
            found_layer_version,
        });
    }

    /// One pair of the walk: decode it and keep what later checks need.
    fn visit(&mut self, k: Vec<u8>, v: Vec<u8>) {
        self.any_key = true;
        let decoded_key = match Key::decode(&k) {
            Ok(key) => key,
            Err(e) => {
                self.counts.unknown += u64::from(matches!(e.rule, UnitRule::Unit1 | UnitRule::Unit2));
                self.decode_error(k, e);
                return;
            }
        };
        if let Key::RevPayload { unit, rev } = decoded_key {
            self.counts.payloads += 1;
            self.payloads.insert((unit, rev));
            self.check_payload(k, unit, rev, &v);
            return;
        }
        match Record::decode(&k, &v) {
            Ok(rec) => self.keep(rec),
            Err(e) => {
                match &decoded_key {
                    Key::LayerMeta { name } if name == LAYER_MARK_NAME => self.bad_mark = true,
                    Key::UnitHead { unit } => {
                        self.bad_heads.insert(*unit);
                    }
                    Key::RevMeta { unit, rev } => {
                        self.bad_metas.insert((*unit, *rev));
                    }
                    Key::ChangeSet { seq } => {
                        self.bad_change_sets.insert(*seq);
                    }
                    Key::CsMember { seq, .. } => {
                        self.bad_members.insert(*seq);
                    }
                    _ => {}
                }
                self.decode_error(k, crate::db::context(e, &describe(&decoded_key)));
            }
        }
    }

    fn keep(&mut self, rec: Record) {
        match rec {
            Record::LayerMeta { name, meta } => {
                self.counts.layer_records += 1;
                if name == LAYER_MARK_NAME {
                    self.mark = Some(meta);
                } else if let Some(t) = name.strip_prefix("policy/")
                    && !t.is_empty()
                    && t.len() <= MAX_POLICY_UNIT_TYPE_LEN
                {
                    self.policies.insert(t.to_owned(), meta.history_policy);
                } else {
                    let k = key_of(Key::LayerMeta { name: name.clone() });
                    self.cross(
                        CrossRule::Unit29,
                        k,
                        format!("LAYER_META name of {} bytes is neither the mark nor a policy", name.len()),
                    );
                }
            }
            Record::UnitHead { unit, head } => {
                self.counts.units += 1;
                self.heads.insert(unit, head);
            }
            Record::RevMeta { unit, rev, meta } => {
                self.counts.revisions += 1;
                self.metas.insert((unit, rev), meta);
            }
            Record::RevPayload { .. } => {}
            Record::ChangeSet { seq, record } => {
                self.counts.change_sets += 1;
                self.change_sets.insert(seq, record);
            }
            Record::CsMember { seq, ordinal, member } => {
                self.counts.members += 1;
                self.members.entry(seq).or_default().push((ordinal, member));
            }
        }
    }

    /// A stored `REV_PAYLOAD` against its `REV_META` (already read: `0x20`
    /// sorts before `0x21`).
    fn check_payload(&mut self, k: Vec<u8>, unit: UnitId, rev: RevNo, stored: &[u8]) {
        let Some(meta) = self.metas.get(&(unit, rev)) else {
            if !self.bad_metas.contains(&(unit, rev)) {
                self.cross(CrossRule::Unit30, k, format!("REV_PAYLOAD of unit {unit} rev {rev} without its REV_META"));
            }
            return;
        };
        if meta.kind != RevisionKind::Delete && !meta.payload_present {
            self.push(
                VerifyRule::Payload(PayloadRule::Unit26),
                k,
                format!("unit {unit} rev {rev}: REV_PAYLOAD exists but payload_present = 0"),
            );
            return;
        }
        if let Err(e) = verify_payload(&meta.payload_decl(), Some(stored)) {
            self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {}", e.detail));
        }
    }

    // ------------------------------------------------------------ after the walk

    fn finish(&mut self) {
        self.check_mark();
        self.check_missing_payloads();
        self.check_orphan_revisions();
        let heads = std::mem::take(&mut self.heads);
        let last_maintenance = self
            .change_sets
            .iter()
            .filter(|(_, r)| r.purpose == Purpose::Maintenance)
            .map(|(s, _)| *s)
            .max()
            .unwrap_or(CommitSeq(0));
        for (unit, head) in &heads {
            self.check_head(*unit, head);
            self.check_chain(*unit, head);
            self.check_policy(*unit, head, last_maintenance);
        }
        self.heads = heads;
        self.check_change_sets();
        self.check_revision_provenance();
    }

    fn check_mark(&mut self) {
        if self.any_key && self.mark.is_none() && !self.bad_mark {
            self.cross(
                CrossRule::Unit29,
                key_of(Key::LayerMeta { name: LAYER_MARK_NAME.into() }),
                "the store has keys but no layer mark".into(),
            );
        }
    }

    /// `REV_META` that declare a payload no `REV_PAYLOAD` holds (UNIT-26).
    fn check_missing_payloads(&mut self) {
        let missing: Vec<((UnitId, RevNo), String)> = self
            .metas
            .iter()
            .filter(|(id, m)| !self.payloads.contains(id) && (m.payload_present || m.kind == RevisionKind::Delete))
            .filter_map(|(id, m)| verify_payload(&m.payload_decl(), None).err().map(|e| (*id, e.detail)))
            .collect();
        for ((unit, rev), detail) in missing {
            self.push(
                VerifyRule::Payload(PayloadRule::Unit26),
                key_of(Key::RevMeta { unit, rev }),
                format!("unit {unit} rev {rev}: {detail}"),
            );
        }
    }

    /// `REV_META`/`REV_PAYLOAD` of a unit without head (UNIT-30), once per unit.
    fn check_orphan_revisions(&mut self) {
        let owners: BTreeSet<UnitId> = self.metas.keys().chain(self.payloads.iter()).map(|(u, _)| *u).collect();
        for unit in owners {
            if !self.heads.contains_key(&unit) && !self.bad_heads.contains(&unit) {
                self.cross(
                    CrossRule::Unit30,
                    key_of(Key::UnitHead { unit }),
                    format!("unit {unit} has revisions but no UNIT_HEAD"),
                );
            }
        }
    }

    fn unit_metas(&self, unit: UnitId) -> Vec<(RevNo, RevisionMeta)> {
        self.metas.range((unit, RevNo(0))..=(unit, RevNo(u64::MAX))).map(|((_, r), m)| (*r, m.clone())).collect()
    }

    /// IU-2 (UNIT-31).
    fn check_head(&mut self, unit: UnitId, head: &UnitHead) {
        let hk = key_of(Key::UnitHead { unit });
        let metas = self.unit_metas(unit);
        let (oldest, top) = (head.oldest_rev, head.head_rev);
        let outside: Vec<RevNo> = metas.iter().map(|(r, _)| *r).filter(|r| *r < oldest || *r > top).collect();
        if let Some(first) = outside.first() {
            self.cross(
                CrossRule::Unit31,
                key_of(Key::RevMeta { unit, rev: *first }),
                format!("unit {unit}: {} REV_META outside [{oldest}, {top}], first rev {first}", outside.len()),
            );
        }
        let payloads_outside = self
            .payloads
            .range((unit, RevNo(0))..=(unit, RevNo(u64::MAX)))
            .filter(|(_, r)| *r < oldest || *r > top)
            .count();
        if payloads_outside > 0 {
            self.cross(
                CrossRule::Unit31,
                hk.clone(),
                format!("unit {unit}: {payloads_outside} REV_PAYLOAD outside [{oldest}, {top}]"),
            );
        }
        // Only the REV_META that exist are walked: a damaged head may claim
        // a range of up to 2^64 revisions.
        let mut present: Vec<u64> = self
            .metas
            .range((unit, oldest)..=(unit, top))
            .map(|((_, r), _)| r.0)
            .chain(self.bad_metas.range((unit, oldest)..=(unit, top)).map(|(_, r)| r.0))
            .collect();
        present.sort_unstable();
        present.dedup();
        let span = u128::from(top.0) - u128::from(oldest.0) + 1;
        let missing = span - present.len() as u128;
        let first_missing = (missing > 0).then(|| {
            let mut expected = oldest.0;
            for r in &present {
                if *r != expected {
                    break;
                }
                expected += 1;
            }
            expected
        });
        if let Some(r) = first_missing {
            self.cross(
                CrossRule::Unit31,
                hk.clone(),
                format!("unit {unit}: {missing} REV_META missing in [{oldest}, {top}], first rev {r}"),
            );
        }
        let Some(m) = self.metas.get(&(unit, top)).cloned() else {
            return;
        };
        let agrees = m.content_hash == head.content_hash
            && m.payload_len == head.payload_len
            && m.schema_version == head.schema_version
            && m.metadata == head.metadata
            && m.commit_seq == head.updated_seq
            && lifecycle_after(m.kind) == head.lifecycle;
        if !agrees {
            self.cross(
                CrossRule::Unit31,
                hk.clone(),
                format!("unit {unit}: UNIT_HEAD disagrees with its REV_META {top}"),
            );
        }
        if m.kind != RevisionKind::Delete && !m.payload_present {
            self.cross(CrossRule::Unit31, hk.clone(), format!("unit {unit}: head revision {top} has no payload"));
        }
        if let Some(first) = self.metas.get(&(unit, oldest)) {
            let ok =
                if oldest.0 == 1 { first.commit_seq == head.created_seq } else { first.commit_seq > head.created_seq };
            if !ok {
                self.cross(
                    CrossRule::Unit31,
                    hk,
                    format!(
                        "unit {unit}: created_seq {} disagrees with revision {oldest} (commit_seq {})",
                        head.created_seq, first.commit_seq
                    ),
                );
            }
        }
    }

    /// IU-3 (UNIT-32).
    fn check_chain(&mut self, unit: UnitId, head: &UnitHead) {
        let metas = self.unit_metas(unit);
        let snap_seq = self.seq;
        if head.updated_seq.0 > snap_seq {
            self.cross(
                CrossRule::Unit32,
                key_of(Key::UnitHead { unit }),
                format!("unit {unit}: updated_seq {} after the snapshot {snap_seq}", head.updated_seq),
            );
        }
        let by_rev: BTreeMap<RevNo, &RevisionMeta> = metas.iter().map(|(r, m)| (*r, m)).collect();
        let mut prev: Option<(RevNo, &RevisionMeta)> = None;
        for (rev, m) in &metas {
            let k = || key_of(Key::RevMeta { unit, rev: *rev });
            if (m.kind == RevisionKind::Create) != (rev.0 == 1) {
                self.cross(
                    CrossRule::Unit32,
                    k(),
                    format!("unit {unit} rev {rev}: kind {:?} at this revision", m.kind),
                );
            }
            if rev.0 > 1 && m.parent_rev != Some(RevNo(rev.0 - 1)) {
                self.cross(CrossRule::Unit32, k(), format!("unit {unit} rev {rev}: parent_rev is not {}", rev.0 - 1));
            }
            if m.commit_seq.0 > snap_seq {
                self.cross(
                    CrossRule::Unit32,
                    k(),
                    format!("unit {unit} rev {rev}: commit_seq {} after the snapshot {snap_seq}", m.commit_seq),
                );
            }
            if let Some((p, pm)) = prev {
                if m.commit_seq <= pm.commit_seq {
                    self.cross(
                        CrossRule::Unit32,
                        k(),
                        format!(
                            "unit {unit} rev {rev}: commit_seq {} not after rev {p}'s {}",
                            m.commit_seq, pm.commit_seq
                        ),
                    );
                }
                if p.0 + 1 == rev.0 {
                    if !transition_ok(lifecycle_after(pm.kind), m.kind) {
                        self.cross(
                            CrossRule::Unit32,
                            k(),
                            format!("unit {unit} rev {rev}: {:?} after {:?}", m.kind, pm.kind),
                        );
                    }
                    let copies = matches!(m.kind, RevisionKind::Archive | RevisionKind::Unarchive);
                    if copies && !same_content(m, pm) {
                        self.cross(
                            CrossRule::Unit32,
                            k(),
                            format!("unit {unit} rev {rev}: {:?} does not copy rev {p}", m.kind),
                        );
                    }
                }
            }
            if let (RevisionKind::Restore, Some(from)) = (m.kind, m.restored_from)
                && let Some(src) = by_rev.get(&from)
                && !same_content(m, src)
            {
                self.cross(CrossRule::Unit32, k(), format!("unit {unit} rev {rev}: Restore does not copy rev {from}"));
            }
            prev = Some((*rev, m));
        }
    }

    /// IU-9 (UNIT-34), for the revisions written after the last policy change.
    fn check_policy(&mut self, unit: UnitId, head: &UnitHead, last_maintenance: CommitSeq) {
        let Some(mark) = &self.mark else {
            return;
        };
        let policy = self.policies.get(&head.unit_type).copied().unwrap_or(mark.history_policy);
        match policy {
            HistoryPolicy::Versioned => {}
            HistoryPolicy::None => {
                if head.updated_seq > last_maintenance && head.oldest_rev != head.head_rev {
                    self.cross(
                        CrossRule::Unit34,
                        key_of(Key::UnitHead { unit }),
                        format!(
                            "unit {unit}: policy None kept revisions [{}, {}) below the head",
                            head.oldest_rev, head.head_rev
                        ),
                    );
                }
            }
            HistoryPolicy::Audit => {
                let metas = self.unit_metas(unit);
                for pair in metas.windows(2) {
                    let ((r, m), (n, next)) = (&pair[0], &pair[1]);
                    if r.0 + 1 == n.0 && next.commit_seq > last_maintenance && m.payload_present {
                        self.cross(
                            CrossRule::Unit34,
                            key_of(Key::RevMeta { unit, rev: *r }),
                            format!("unit {unit} rev {r}: policy Audit kept the payload of a superseded revision"),
                        );
                    }
                }
            }
        }
    }

    /// IU-4 from the change sets' side (UNIT-33), and orphan members (UNIT-30).
    fn check_change_sets(&mut self) {
        let members = std::mem::take(&mut self.members);
        for (seq, list) in &members {
            if !self.change_sets.contains_key(seq) && !self.bad_change_sets.contains(seq) {
                self.cross(
                    CrossRule::Unit30,
                    key_of(Key::ChangeSet { seq: *seq }),
                    format!("{} CS_MEMBER of change set {seq} without its CHANGESET", list.len()),
                );
            }
        }
        let change_sets = std::mem::take(&mut self.change_sets);
        for (seq, rec) in &change_sets {
            let ck = || key_of(Key::ChangeSet { seq: *seq });
            if seq.0 > self.seq {
                self.cross(CrossRule::Unit33, ck(), format!("change set {seq} after the snapshot {}", self.seq));
            }
            let empty = Vec::new();
            let list = members.get(seq).unwrap_or(&empty);
            let shape_ok = match rec.purpose {
                Purpose::Maintenance => rec.member_count == 0,
                Purpose::Application | Purpose::Retention => rec.member_count >= 1,
            };
            if !shape_ok {
                self.cross(
                    CrossRule::Unit33,
                    ck(),
                    format!("change set {seq}: purpose {:?} with member_count {}", rec.purpose, rec.member_count),
                );
            }
            if self.bad_members.contains(seq) {
                continue;
            }
            let ordinals_ok =
                list.len() == rec.member_count as usize && list.iter().enumerate().all(|(i, (o, _))| *o as usize == i);
            if !ordinals_ok {
                self.cross(
                    CrossRule::Unit33,
                    ck(),
                    format!(
                        "change set {seq}: member_count {} but {} CS_MEMBER, not numbered 0..n",
                        rec.member_count,
                        list.len()
                    ),
                );
            }
            let mut units = BTreeSet::new();
            for (ordinal, m) in list {
                let mk = || key_of(Key::CsMember { seq: *seq, ordinal: *ordinal });
                if !units.insert(m.unit) {
                    self.cross(CrossRule::Unit33, mk(), format!("change set {seq}: unit {} named twice", m.unit));
                }
                // Members of Retention change sets name the first retained
                // revision, not one they wrote: only REV_META → member is
                // checked for them ("Nota de E3-06").
                if rec.purpose != Purpose::Application {
                    continue;
                }
                self.check_member(*seq, mk(), m);
            }
        }
        self.members = members;
        self.change_sets = change_sets;
    }

    /// An `Application` member names a revision of its change set.
    fn check_member(&mut self, seq: CommitSeq, mk: Vec<u8>, m: &CsMember) {
        let (unit, rev) = (m.unit, m.rev_no);
        let Some(head) = self.heads.get(&unit) else {
            if !self.bad_heads.contains(&unit) {
                self.cross(CrossRule::Unit33, mk, format!("change set {seq}: member names unknown unit {unit}"));
            }
            return;
        };
        if rev > head.head_rev {
            let top = head.head_rev;
            self.cross(
                CrossRule::Unit33,
                mk,
                format!("change set {seq}: member names unit {unit} rev {rev} above head {top}"),
            );
            return;
        }
        if rev < head.oldest_rev {
            return; // pruned by retention or by policy None
        }
        let Some(meta) = self.metas.get(&(unit, rev)) else {
            return; // reported as missing by the head check
        };
        if meta.commit_seq != seq || meta.kind != m.kind {
            let detail = format!(
                "change set {seq}: member ({unit}, {rev}, {:?}) disagrees with REV_META (commit_seq {}, {:?})",
                m.kind, meta.commit_seq, meta.kind
            );
            self.cross(CrossRule::Unit33, mk, detail);
        }
    }

    /// IU-4 from the revisions' side (UNIT-33).
    fn check_revision_provenance(&mut self) {
        let mut found = Vec::new();
        for ((unit, rev), meta) in &self.metas {
            let seq = meta.commit_seq;
            let problem = match self.change_sets.get(&seq) {
                None if self.bad_change_sets.contains(&seq) => None,
                None => Some(format!("unit {unit} rev {rev}: no CHANGESET {seq}")),
                Some(r) if r.purpose != Purpose::Application => {
                    Some(format!("unit {unit} rev {rev}: CHANGESET {seq} has purpose {:?}", r.purpose))
                }
                Some(_) if self.bad_members.contains(&seq) => None,
                Some(_) => {
                    let named = self.members.get(&seq).is_some_and(|l| {
                        l.iter().any(|(_, m)| m.unit == *unit && m.rev_no == *rev && m.kind == meta.kind)
                    });
                    (!named).then(|| format!("unit {unit} rev {rev}: no CS_MEMBER of change set {seq} names it"))
                }
            };
            if let Some(d) = problem {
                found.push((key_of(Key::RevMeta { unit: *unit, rev: *rev }), d));
            }
        }
        for (k, d) in found {
            self.cross(CrossRule::Unit33, k, d);
        }
    }
}

/// Verifies every record of the units layer in `snap` (see the [module
/// documentation](self)). Read-only.
pub fn verify_snapshot<S: Snapshot>(snap: &S) -> Result<UnitVerifyReport, UnitError> {
    let mut st = State { seq: snap.seq(), ..State::default() };
    for_each_pair(snap, &[], None, |k, v| st.visit(k, v))?;
    st.finish();
    Ok(UnitVerifyReport {
        seq: CommitSeq(st.seq),
        counts: st.counts,
        findings: st.findings,
        findings_omitted: st.omitted,
    })
}

/// IU-1 (UNIT-35): every `REV_META`/`REV_PAYLOAD` key present in both
/// snapshots has the same value in both, except that a history policy may
/// have turned a `REV_META`'s `payload_present` from 1 to 0 (and nothing
/// else; "Nota de E3-06"). `older` must not be after `newer`. Records that do
/// not decode are left to [`verify_snapshot`]. Read-only.
pub fn verify_immutable<A: Snapshot, B: Snapshot>(older: &A, newer: &B) -> Result<Vec<VerifyFinding>, UnitError> {
    if older.seq() > newer.seq() {
        return Err(UnitError::InvalidArgument(format!(
            "older snapshot {} is after newer snapshot {}",
            older.seq(),
            newer.seq()
        )));
    }
    let start = [TAG_REV_META];
    let end = [TAG_REV_PAYLOAD + 1];
    // Payloads are compared by digest so that memory stays small.
    let digest =
        |k: &[u8], v: &[u8]| if k[0] == TAG_REV_PAYLOAD { blake3::hash(v).as_bytes().to_vec() } else { v.to_vec() };
    let mut old: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for_each_pair(older, &start, Some(&end), |k, v| {
        let d = digest(&k, &v);
        old.insert(k, d);
    })?;
    let mut findings = Vec::new();
    for_each_pair(newer, &start, Some(&end), |k, v| {
        let Some(before) = old.get(&k) else {
            return;
        };
        let now = digest(&k, &v);
        if *before == now {
            return;
        }
        let (Ok(Key::RevMeta { unit, rev }) | Ok(Key::RevPayload { unit, rev })) = Key::decode(&k) else {
            return;
        };
        let allowed = k[0] == TAG_REV_META
            && match (RevisionMeta::decode(rev, before), RevisionMeta::decode(rev, &now)) {
                (Ok(a), Ok(b)) => a.payload_present && b == RevisionMeta { payload_present: false, ..a },
                _ => true, // left to verify_snapshot
            };
        if !allowed && findings.len() < MAX_FINDINGS {
            let what = if k[0] == TAG_REV_META { "REV_META" } else { "REV_PAYLOAD" };
            findings.push(VerifyFinding {
                rule: VerifyRule::Cross(CrossRule::Unit35),
                key: k.clone(),
                detail: format!(
                    "unit {unit} rev {rev}: {what} changed between snapshots {} and {}",
                    older.seq(),
                    newer.seq()
                ),
                found_layer_version: None,
            });
        }
    })?;
    Ok(findings)
}
