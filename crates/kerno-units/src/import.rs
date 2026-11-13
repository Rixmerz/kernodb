//! Import of the draft logical export into a new database (E9-05, I-30, D10).
//!
//! Reads what [`crate::export`] writes (`docs/file-format/export-v0-borrador.md`)
//! and rebuilds the same logical state in an empty database of any engine: the
//! export of the result is, byte for byte, the file that was imported. The
//! format is a **draft**: nothing here is a promise (R-E9-2, D12).
//!
//! The file is decoded and checked in full before the first write, then the
//! commit sequences it names are replayed one change set each, so that the
//! revisions keep their `commit_seq` and the next ordinary write lands after
//! them. A sequence no revision or relation names is filled with an empty
//! `Index` change set. The change sets are rebuilt from the revisions
//! (purpose, actor and source only): the draft does not hold their reason or
//! correlation, and a relation-only sequence is recorded as `Index`, not
//! `Relation`. Last, the result is checked with [`verify_snapshot`].
//!
//! Public id tokens are restored with the unit, in the change set of its first
//! retained revision, so a token is the same after export and import. A file
//! that gives one token to two units is refused before the first write with
//! [`UnitError::PublicIdTaken`].
//!
//! Format errors are `Storage(Corruption)`; a `format_version` other than the
//! one of [`EXPORT_VERSION`] is `Storage(Incompatible)`; a target that is not
//! empty, or whose default policy is not the exported one, is
//! `InvalidArgument`.

use std::collections::{BTreeMap, BTreeSet};

use kerno_core::contract::TxnEngine;

use crate::codec::{Adjacency, Key, RelationRecord, RevisionMeta, UnitHead, valid_relation_type};
use crate::db::{ChangeMeta, ChangeSet, UnitDb, Write, corruption, key};
use crate::error::UnitError;
use crate::export::{DIGEST_LEN, EXPORT_MAGIC, EXPORT_VERSION, FLAG_DRAFT, NO_DEFAULT_POLICY};
use crate::ids::PublicId;
use crate::payload::{ContentHash, HashAlg, verify_payload};
use crate::types::{
    CommitSeq, HistoryPolicy, ProvenanceKind, RelationId, RelationProvenance, RevNo, RevisionKind, Timestamp, UnitId,
};
use crate::verify::{lifecycle_after, verify_snapshot};

/// Largest commit sequence an import replays (one commit per sequence).
// ponytail: a draft replays every sequence one by one; a sparse or hostile
// sequence number is refused instead. Lift it if the format ever freezes.
pub const MAX_REPLAY_SEQ: u64 = 1_000_000;

/// What an import wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub units: usize,
    pub revisions: usize,
    pub relations: usize,
    pub policies: usize,
    /// Public id tokens restored with their units.
    pub tokens: usize,
}

struct Rev {
    no: RevNo,
    meta: RevisionMeta,
    payload: Option<Vec<u8>>,
}

struct UnitData {
    id: UnitId,
    unit_type: String,
    created_seq: CommitSeq,
    created_at: Timestamp,
    oldest: RevNo,
    head: RevNo,
    revs: Vec<Rev>,
    token: Option<PublicId>,
}

struct Rel {
    id: RelationId,
    from: UnitId,
    to: UnitId,
    record: RelationRecord,
}

struct Parsed {
    default: Option<HistoryPolicy>,
    policies: Vec<(String, HistoryPolicy)>,
    units: Vec<UnitData>,
    relations: Vec<Rel>,
}

/// Imports `bytes` into `db`, which must be new: no commit yet, and a default
/// policy equal to the exported one. On an error after the first write the
/// directory is not a usable copy: discard it.
pub fn import<E: TxnEngine>(db: &UnitDb<E>, bytes: &[u8]) -> Result<ImportReport, UnitError> {
    let parsed = parse(bytes)?;
    if db.snapshot()?.seq().0 != 0 {
        return Err(UnitError::InvalidArgument("import needs a new database: it already has commits".into()));
    }
    if let Some(p) = parsed.default
        && p != db.config().default_policy
    {
        return Err(UnitError::InvalidArgument(format!(
            "the export has default policy {p:?}, the database {:?}",
            db.config().default_policy
        )));
    }
    let report = ImportReport {
        units: parsed.units.len(),
        revisions: parsed.units.iter().map(|u| u.revs.len()).sum(),
        relations: parsed.relations.len(),
        policies: parsed.policies.len(),
        tokens: parsed.units.iter().filter(|u| u.token.is_some()).count(),
    };
    replay(db, &parsed)?;
    for (unit_type, p) in &parsed.policies {
        db.set_policy(unit_type, *p, ChangeMeta::default())?;
    }
    verify_snapshot(&db.snapshot()?.snap)?.into_result()?;
    Ok(report)
}

/// What one sequence holds: `(unit, revision)` positions and relation positions.
type AtSeq = (Vec<(usize, usize)>, Vec<usize>);

/// One change set per sequence from 1 to the highest one named.
fn replay<E: TxnEngine>(db: &UnitDb<E>, p: &Parsed) -> Result<(), UnitError> {
    let mut by_seq: BTreeMap<u64, AtSeq> = BTreeMap::new();
    for (ui, u) in p.units.iter().enumerate() {
        for (ri, r) in u.revs.iter().enumerate() {
            by_seq.entry(r.meta.commit_seq.0).or_default().0.push((ui, ri));
        }
    }
    for (i, r) in p.relations.iter().enumerate() {
        by_seq.entry(r.record.created_seq.0).or_default().1.push(i);
    }
    let last = by_seq.keys().next_back().copied().unwrap_or(0);
    if last > MAX_REPLAY_SEQ {
        return Err(UnitError::InvalidArgument(format!("commit sequence {last} is above {MAX_REPLAY_SEQ}")));
    }
    for seq in 1..=last {
        let (revs, rels) = by_seq.get(&seq).map_or((&[][..], &[][..]), |(a, b)| (a, b));
        let mut cs = match revs.first() {
            Some(&(ui, ri)) => {
                let m = &p.units[ui].revs[ri].meta;
                let nonempty = |v: &[u8]| (!v.is_empty()).then(|| v.to_vec());
                let meta = ChangeMeta {
                    actor: nonempty(&m.actor_ref),
                    source: nonempty(&m.source_ref),
                    ..ChangeMeta::default()
                };
                db.begin_changeset(meta)?
            }
            None => ChangeSet::begin_index(db)?,
        };
        if cs.commit_seq().0 != seq {
            return Err(UnitError::InvalidArgument(format!(
                "the change set would commit as {}, the import needs {seq}",
                cs.commit_seq()
            )));
        }
        for &(ui, ri) in revs {
            let u = &p.units[ui];
            if ri == 0
                && let Some(token) = u.token
            {
                cs.restore_public_id(u.id, token)?;
            }
            write_revision(&mut cs, u, ri)?;
        }
        for &i in rels {
            write_relation(&mut cs, &p.relations[i])?;
        }
        cs.commit()?;
    }
    Ok(())
}

fn write_revision<E: TxnEngine>(cs: &mut ChangeSet<'_, E>, u: &UnitData, ri: usize) -> Result<(), UnitError> {
    let r = &u.revs[ri];
    let (id, m) = (u.id, &r.meta);
    let mut writes = vec![Write::Put(key(Key::RevMeta { unit: id, rev: r.no })?, m.encode(r.no)?)];
    if let Some(p) = &r.payload {
        writes.push(Write::Put(key(Key::RevPayload { unit: id, rev: r.no })?, p.clone()));
    }
    if r.no == u.head {
        let head = UnitHead {
            unit_type: u.unit_type.clone(),
            schema_version: m.schema_version,
            lifecycle: lifecycle_after(m.kind),
            head_rev: u.head,
            oldest_rev: u.oldest,
            created_seq: u.created_seq,
            updated_seq: m.commit_seq,
            created_at: u.created_at,
            updated_at: m.created_at,
            content_hash: m.content_hash,
            payload_len: m.payload_len,
            metadata: m.metadata.clone(),
        };
        writes.push(Write::Put(key(Key::UnitHead { unit: id })?, head.encode()?));
    }
    cs.retain(id, r.no, m.kind, writes)
}

fn write_relation<E: TxnEngine>(cs: &mut ChangeSet<'_, E>, r: &Rel) -> Result<(), UnitError> {
    let (edge, ty) = (r.id, &r.record.relation_type);
    cs.adjust_count(r.from, crate::types::Direction::Out, ty, 1)?;
    cs.adjust_count(r.to, crate::types::Direction::In, ty, 1)?;
    cs.write(&Write::Put(key(Key::Relation { edge })?, r.record.encode()?))?;
    let out = Key::RelationOut { unit: r.from, relation_type: ty.clone(), edge };
    cs.write(&Write::Put(key(out)?, Adjacency { other: r.to }.encode()))?;
    let inn = Key::RelationIn { unit: r.to, relation_type: ty.clone(), edge };
    cs.write(&Write::Put(key(inn)?, Adjacency { other: r.from }.encode()))
}

// ------------------------------------------------------------------ decoding

fn bad(msg: impl Into<String>) -> UnitError {
    corruption(format!("export: {}", msg.into()))
}

/// Decodes and checks the whole file; nothing is written from here.
fn parse(bytes: &[u8]) -> Result<Parsed, UnitError> {
    if bytes.len() < 16 + DIGEST_LEN {
        return Err(bad(format!("{} bytes are fewer than a header and a digest", bytes.len())));
    }
    if bytes[..8] != EXPORT_MAGIC {
        return Err(bad("the magic is not KUNITEXD"));
    }
    let version = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version != EXPORT_VERSION {
        return Err(UnitError::Storage(kerno_core::Error::Incompatible(format!(
            "export format_version {version}, this reader has {EXPORT_VERSION}"
        ))));
    }
    if bytes[12] != FLAG_DRAFT || bytes[13..16] != [0; 3] {
        return Err(bad("flags or reserved bytes are not those of a draft"));
    }
    let (body, digest) = bytes.split_at(bytes.len() - DIGEST_LEN);
    if blake3::hash(body).as_bytes() != digest {
        return Err(bad("the closing digest does not match"));
    }
    let mut r = In { b: &body[16..] };
    let default = match r.u8()? {
        NO_DEFAULT_POLICY => None,
        b => Some(HistoryPolicy::from_byte(b).ok_or_else(|| bad(format!("default policy byte {b}")))?),
    };
    let policies = policies(&mut r)?;
    let token_records = token_records(&mut r)?;
    let mut units = units(&mut r)?;
    attach_tokens(token_records, &mut units)?;
    let relations = relations(&mut r)?;
    if !r.b.is_empty() {
        return Err(bad(format!("{} bytes after the last section", r.b.len())));
    }
    // A mark exists only after a first write, and every first write leaves
    // something to export.
    if default.is_none() != (policies.is_empty() && units.is_empty() && relations.is_empty()) {
        return Err(bad("content without a layer mark, or a layer mark without content"));
    }
    check_relation_ends(&units, &relations)?;
    Ok(Parsed { default, policies, units, relations })
}

fn policies(r: &mut In<'_>) -> Result<Vec<(String, HistoryPolicy)>, UnitError> {
    let n = r.u32()?;
    let mut out: Vec<(String, HistoryPolicy)> = Vec::new();
    for _ in 0..n {
        let t = r.string()?;
        let b = r.u8()?;
        let p = HistoryPolicy::from_byte(b).ok_or_else(|| bad(format!("policy byte {b}")))?;
        if t.is_empty() || t.len() > crate::db::MAX_POLICY_UNIT_TYPE_LEN {
            return Err(bad("a policy names a unit type outside 1..=57 bytes"));
        }
        if out.last().is_some_and(|(prev, _)| *prev >= t) {
            return Err(bad("policies are not in strictly increasing unit type order"));
        }
        out.push((t, p));
    }
    Ok(out)
}

fn units(r: &mut In<'_>) -> Result<Vec<UnitData>, UnitError> {
    let n = r.u32()?;
    let mut out: Vec<UnitData> = Vec::new();
    for _ in 0..n {
        let u = unit(r)?;
        if out.last().is_some_and(|prev| prev.id >= u.id) {
            return Err(bad("units are not in strictly increasing id order"));
        }
        out.push(u);
    }
    Ok(out)
}

fn unit(r: &mut In<'_>) -> Result<UnitData, UnitError> {
    let id = UnitId(r.array()?);
    let unit_type = r.string()?;
    let created_seq = CommitSeq(r.u64()?);
    let created_at = Timestamp(r.i64()?);
    let (oldest, head) = (RevNo(r.u64()?), RevNo(r.u64()?));
    let count = r.u32()?;
    if oldest.0 == 0 || head < oldest || u64::from(count) != head.0 - oldest.0 + 1 {
        return Err(bad(format!(
            "unit {id}: {count} revisions do not cover [{oldest}, {head}] (a revision whose content the policy \
             does not keep is left out of the draft export, so such a history cannot be imported)"
        )));
    }
    let mut revs: Vec<Rev> = Vec::new();
    for i in 0..u64::from(count) {
        let rev = revision(r, id, RevNo(oldest.0 + i))?;
        if let Some(prev) = revs.last()
            && rev.meta.commit_seq <= prev.meta.commit_seq
        {
            return Err(bad(format!("unit {id} rev {}: commit_seq does not grow", rev.no)));
        }
        revs.push(rev);
    }
    let (first, last) = (&revs[0].meta, &revs[revs.len() - 1].meta);
    let first_ok = if oldest.0 == 1 { first.commit_seq == created_seq } else { first.commit_seq > created_seq };
    if !first_ok || created_seq.0 == 0 {
        return Err(bad(format!("unit {id}: created_seq {created_seq} disagrees with its first revision")));
    }
    if last.kind != RevisionKind::Delete && last.payload_len > 0 && revs[revs.len() - 1].payload.is_none() {
        return Err(bad(format!("unit {id}: the head revision has no payload")));
    }
    Ok(UnitData { id, unit_type, created_seq, created_at, oldest, head, revs, token: None })
}

fn revision(r: &mut In<'_>, unit: UnitId, no: RevNo) -> Result<Rev, UnitError> {
    let at = |what: &str| bad(format!("unit {unit} rev {no}: {what}"));
    let wire = r.u64()?;
    if wire != no.0 {
        return Err(at(&format!("numbered {wire} in a file that expects {no}")));
    }
    let kind_byte = r.u8()?;
    let kind = RevisionKind::from_byte(kind_byte).ok_or_else(|| at(&format!("kind byte {kind_byte}")))?;
    let parent_rev = r.opt_u64()?.map(RevNo);
    let commit_seq = CommitSeq(r.u64()?);
    let created_at = Timestamp(r.i64()?);
    let schema_version = r.u32()?;
    let alg_byte = r.u8()?;
    let alg = HashAlg::from_byte(alg_byte).ok_or_else(|| at(&format!("hash algorithm byte {alg_byte}")))?;
    let content_hash = ContentHash { alg, digest: r.array()? };
    let payload_len = r.u32()?;
    let restored_from = r.opt_u64()?.map(RevNo);
    let metadata = r.bytes()?.to_vec();
    let actor_ref = r.opt_bytes()?.map(<[u8]>::to_vec).unwrap_or_default();
    let source_ref = r.opt_bytes()?.map(<[u8]>::to_vec).unwrap_or_default();
    let payload = r.opt_bytes()?.map(<[u8]>::to_vec);
    if commit_seq.0 == 0 {
        return Err(at("commit_seq 0"));
    }
    if (kind == RevisionKind::Create) != (no.0 == 1) {
        return Err(at(&format!("kind {kind:?} at this revision")));
    }
    if no.0 > 1 && parent_rev != Some(RevNo(no.0 - 1)) {
        return Err(at("parent is not the previous revision"));
    }
    if no.0 == 1 && parent_rev.is_some() {
        return Err(at("the first revision has a parent"));
    }
    if (kind == RevisionKind::Restore) != restored_from.is_some() {
        return Err(at("restored_from and kind disagree"));
    }
    let meta = RevisionMeta {
        kind,
        parent_rev,
        commit_seq,
        created_at,
        schema_version,
        content_hash,
        payload_len,
        payload_present: payload.is_some(),
        restored_from,
        metadata,
        actor_ref,
        source_ref,
    };
    // The payload against what the revision declares: length and hash.
    verify_payload(&meta.payload_decl(), payload.as_deref())?;
    Ok(Rev { no, meta, payload })
}

fn relations(r: &mut In<'_>) -> Result<Vec<Rel>, UnitError> {
    let n = r.u32()?;
    let mut out: Vec<Rel> = Vec::new();
    for _ in 0..n {
        let id = RelationId(r.array()?);
        let (from, to) = (UnitId(r.array()?), UnitId(r.array()?));
        let relation_type = r.string()?;
        let created_seq = CommitSeq(r.u64()?);
        let kind_byte = r.u8()?;
        let origin = r.bytes()?.to_vec();
        let confidence = u16::from_be_bytes(r.array()?);
        let kind = ProvenanceKind::from_byte(kind_byte).ok_or_else(|| bad(format!("provenance byte {kind_byte}")))?;
        // The export writes a zero confidence and an empty origin where the
        // kind has none; anything else would not survive a re-export.
        let echoed = match kind {
            ProvenanceKind::Declared => origin.is_empty() && confidence == 0,
            ProvenanceKind::Inferred => true,
            _ => confidence == 0,
        };
        let provenance = match kind {
            ProvenanceKind::Declared => RelationProvenance::Declared,
            ProvenanceKind::Imported => RelationProvenance::Imported { source: origin },
            ProvenanceKind::Algorithmic => RelationProvenance::Algorithmic { origin },
            ProvenanceKind::Inferred => RelationProvenance::Inferred { origin, confidence_permille: confidence },
        };
        if !echoed || !valid_relation_type(&relation_type) || created_seq.0 == 0 {
            return Err(bad(format!("relation {id}: a field is outside what a writer produces")));
        }
        if out.last().is_some_and(|prev| prev.id >= id) {
            return Err(bad("relations are not in strictly increasing id order"));
        }
        out.push(Rel { id, from, to, record: RelationRecord { from, to, relation_type, created_seq, provenance } });
    }
    Ok(out)
}

/// The token section: strictly increasing unit ids, each a unit of the file
/// that is not deleted (deleting removes the mapping), and no token twice.
fn token_records(r: &mut In<'_>) -> Result<Vec<(UnitId, [u8; 16])>, UnitError> {
    let n = r.u32()?;
    let mut seen: BTreeSet<[u8; 16]> = BTreeSet::new();
    let mut out: Vec<(UnitId, [u8; 16])> = Vec::new();
    for _ in 0..n {
        let unit = UnitId(r.array()?);
        let bytes: [u8; 16] = r.array()?;
        if out.last().is_some_and(|(p, _)| *p >= unit) {
            return Err(bad("token records are not in strictly increasing unit id order"));
        }
        if bytes.iter().all(|b| *b == bytes[0]) {
            return Err(bad("a token record holds a constant token"));
        }
        if !seen.insert(bytes) {
            return Err(UnitError::PublicIdTaken);
        }
        out.push((unit, bytes));
    }
    Ok(out)
}

/// Each token record names a unit of the file that is not deleted.
fn attach_tokens(records: Vec<(UnitId, [u8; 16])>, units: &mut [UnitData]) -> Result<(), UnitError> {
    for (unit, bytes) in records {
        let Ok(at) = units.binary_search_by(|u| u.id.cmp(&unit)) else {
            return Err(bad("a token record names a unit that is not in the file"));
        };
        if units[at].revs.last().is_some_and(|rev| rev.meta.kind == RevisionKind::Delete) {
            return Err(bad("a token record names a deleted unit"));
        }
        units[at].token = Some(PublicId::from_bytes(bytes));
    }
    Ok(())
}

/// Both ends of a relation are units of the file.
fn check_relation_ends(units: &[UnitData], rels: &[Rel]) -> Result<(), UnitError> {
    for rel in rels {
        for end in [rel.from, rel.to] {
            if units.binary_search_by(|u| u.id.cmp(&end)).is_err() {
                return Err(bad(format!("relation {}: unit {end} is not in the file", rel.id)));
            }
        }
    }
    Ok(())
}

/// Big-endian reader over the body; every read checks what is left.
struct In<'a> {
    b: &'a [u8],
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], UnitError> {
        if self.b.len() < n {
            return Err(bad(format!("truncated: {n} bytes wanted, {} left", self.b.len())));
        }
        let (head, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], UnitError> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, UnitError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, UnitError> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, UnitError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64, UnitError> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], UnitError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn string(&mut self) -> Result<String, UnitError> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| bad("a text field is not UTF-8"))
    }
    fn flag(&mut self) -> Result<bool, UnitError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(bad(format!("presence byte {b}"))),
        }
    }
    fn opt_u64(&mut self) -> Result<Option<u64>, UnitError> {
        if self.flag()? { self.u64().map(Some) } else { Ok(None) }
    }
    fn opt_bytes(&mut self) -> Result<Option<&'a [u8]>, UnitError> {
        if self.flag()? { self.bytes().map(Some) } else { Ok(None) }
    }
}
