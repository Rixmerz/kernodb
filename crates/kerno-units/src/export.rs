//! Logical export of the layer, draft format (E9-04, I-30, D10).
//!
//! The format is specified in `docs/file-format/export-v0-borrador.md`. It is
//! a **draft**: versioned and not frozen, so no byte of it is a promise
//! (R-E9-2); freezing it is a decision of the project owner (D12). It has its
//! own header and does not use `KRNO_EXP`, which stays reserved and unused.
//!
//! An export holds units, their retained revisions, relations and history
//! policies, as the public reads of the layer answer them at one snapshot,
//! and the public identifier token of each unit that has one (ADR-0005: the
//! mapping is part of the logical export, so a token is stable across export
//! and import). It holds no derived data (ADR-0018: derived data is
//! rebuilt). It is a function of the logical
//! state only: no sequence of the snapshot, no file, level, offset or
//! compaction state, so the same state gives the same bytes (I-30).
//!
//! The entry points take a snapshot or a database and return bytes; neither
//! hands out the engine they read from.

use std::collections::BTreeMap;

use kerno_core::contract::{Engine, TxnEngine};

use crate::codec::{Key, LAYER_MARK_NAME, LayerMeta, MAX_LAYER_VERSION, TAG_LAYER_META, TAG_REL, TAG_UNIT_HEAD};
use crate::db::{UnitDb, UnitSnapshot, context, corruption, scan_all};
use crate::error::UnitError;
use crate::types::{HistoryPolicy, RelationId, RelationProvenance, RevNo, UnitId};

/// First bytes of an export: "Kerno UNIt EXport Draft". Not `KRNO_EXP`.
pub const EXPORT_MAGIC: [u8; 8] = *b"KUNITEXD";
/// The draft format version. 1 added the token section.
pub const EXPORT_VERSION: u32 = 1;
/// Header flag: the format is not frozen (R-E9-2).
pub const FLAG_DRAFT: u8 = 1;
/// Length of the closing BLAKE3 digest.
pub const DIGEST_LEN: usize = 32;
/// Byte that stands for "no layer-wide policy yet" in the header section.
pub const NO_DEFAULT_POLICY: u8 = 0xFF;

/// The bytes of the draft export of the database at its current state.
pub fn export<E: TxnEngine>(db: &UnitDb<E>) -> Result<Vec<u8>, UnitError> {
    export_snapshot(&db.snapshot()?)
}

/// The bytes of the draft export of `snap`: header, policies, units with
/// their retained revisions, relations, then the BLAKE3 digest of all that
/// precedes it. Units are in id order, relations in id order, policies in
/// unit type order.
///
/// A revision the public reads cannot give (its content is not kept by the
/// unit type's policy) is left out, as it is for every other reader.
// ponytail: one `get_revision` per revision; fine for a draft, batch the
// reads per unit if an export of a large database is ever hot.
pub fn export_snapshot<E: Engine>(snap: &UnitSnapshot<E>) -> Result<Vec<u8>, UnitError> {
    let mut w = Out(Vec::new());
    w.0.extend_from_slice(&EXPORT_MAGIC);
    w.u32(EXPORT_VERSION);
    w.u8(FLAG_DRAFT);
    w.0.extend_from_slice(&[0; 3]);

    policies(snap, &mut w)?;
    let ids = unit_ids(snap)?;
    tokens(snap, &ids, &mut w)?;
    units(snap, &ids, &mut w)?;
    relations(snap, &mut w)?;

    let digest = blake3::hash(&w.0);
    w.0.extend_from_slice(digest.as_bytes());
    Ok(w.0)
}

/// Layer-wide policy, then one record per unit type with its own policy.
fn policies<E: Engine>(snap: &UnitSnapshot<E>, w: &mut Out) -> Result<(), UnitError> {
    let mut default = NO_DEFAULT_POLICY;
    let mut per_type: BTreeMap<String, HistoryPolicy> = BTreeMap::new();
    let (start, end) = Key::tag_range(TAG_LAYER_META);
    scan_all(&snap.snap, start, end, |k, v| {
        let Key::LayerMeta { name } = Key::decode(k)? else {
            return Err(corruption("a key in the LAYER_META range is not a LAYER_META".into()));
        };
        let policy = |what: &str| -> Result<HistoryPolicy, UnitError> {
            Ok(LayerMeta::decode_upto(v, MAX_LAYER_VERSION).map_err(|e| context(e, what))?.history_policy)
        };
        if name == LAYER_MARK_NAME {
            default = policy("LAYER_META \"layer\"")?.to_byte();
        } else if let Some(unit_type) = name.strip_prefix("policy/") {
            per_type.insert(unit_type.to_owned(), policy(&format!("LAYER_META \"{name}\""))?);
        }
        Ok(())
    })?;
    w.u8(default);
    w.u32(per_type.len() as u32);
    for (unit_type, p) in &per_type {
        w.bytes(unit_type.as_bytes());
        w.u8(p.to_byte());
    }
    Ok(())
}

fn unit_ids<E: Engine>(snap: &UnitSnapshot<E>) -> Result<Vec<UnitId>, UnitError> {
    let mut ids = Vec::new();
    let (start, end) = Key::tag_range(TAG_UNIT_HEAD);
    scan_all(&snap.snap, start, end, |k, _| {
        let Key::UnitHead { unit } = Key::decode(k)? else {
            return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
        };
        ids.push(unit);
        Ok(())
    })?;
    Ok(ids)
}

fn units<E: Engine>(snap: &UnitSnapshot<E>, ids: &[UnitId], w: &mut Out) -> Result<(), UnitError> {
    w.u32(ids.len() as u32);
    for id in ids {
        unit(snap, *id, w)?;
    }
    Ok(())
}

/// The public id of each unit that has one, in unit id order: `unit` and
/// `token`, 16 bytes each. A pair that disagrees fails the export.
fn tokens<E: Engine>(snap: &UnitSnapshot<E>, ids: &[UnitId], w: &mut Out) -> Result<(), UnitError> {
    let mut found = Vec::new();
    for id in ids {
        if let Some(token) = snap.public_id(*id)? {
            found.push((*id, token));
        }
    }
    w.u32(found.len() as u32);
    for (id, token) in found {
        w.0.extend_from_slice(id.as_bytes());
        w.0.extend_from_slice(&token.to_bytes());
    }
    Ok(())
}

fn unit<E: Engine>(snap: &UnitSnapshot<E>, id: UnitId, w: &mut Out) -> Result<(), UnitError> {
    let u = snap.get_unit(id)?.ok_or_else(|| corruption(format!("unit {id}: UNIT_HEAD vanished during the export")))?;
    // Only the revisions the public reads give: a policy that keeps no
    // content for a revision makes `get_revision` refuse it.
    let mut revs = Vec::new();
    for n in u.oldest.0..=u.head.0 {
        match snap.get_revision(id, RevNo(n)) {
            Ok(r) => revs.push(r),
            Err(UnitError::HistoryUnavailable { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    w.0.extend_from_slice(id.as_bytes());
    w.bytes(u.unit_type.as_bytes());
    w.u64(u.created_seq.0);
    w.i64(u.created_at.0);
    w.u64(u.oldest.0);
    w.u64(u.head.0);
    w.u32(revs.len() as u32);
    for r in revs {
        w.u64(r.id.rev.0);
        w.u8(r.kind.to_byte());
        w.opt_u64(r.parent.map(|p| p.0));
        w.u64(r.commit_seq.0);
        w.i64(r.created_at.0);
        w.u32(r.schema_version);
        w.u8(r.content_hash.alg.to_byte());
        w.0.extend_from_slice(&r.content_hash.digest);
        w.u32(r.payload_len);
        w.opt_u64(r.restored_from.map(|p| p.0));
        w.bytes(&r.metadata);
        w.opt_bytes(r.provenance.actor.as_deref());
        w.opt_bytes(r.provenance.source.as_deref());
        w.opt_bytes(r.payload.as_deref());
    }
    Ok(())
}

fn relations<E: Engine>(snap: &UnitSnapshot<E>, w: &mut Out) -> Result<(), UnitError> {
    let mut ids: Vec<RelationId> = Vec::new();
    let (start, end) = Key::tag_range(TAG_REL);
    scan_all(&snap.snap, start, end, |k, _| {
        let Key::Relation { edge } = Key::decode(k)? else {
            return Err(corruption("a key in the REL range is not a REL".into()));
        };
        ids.push(edge);
        Ok(())
    })?;
    w.u32(ids.len() as u32);
    for id in ids {
        let r = snap
            .get_relation(id)?
            .ok_or_else(|| corruption(format!("relation {id}: REL vanished during the export")))?;
        w.0.extend_from_slice(id.as_bytes());
        w.0.extend_from_slice(r.from.as_bytes());
        w.0.extend_from_slice(r.to.as_bytes());
        w.bytes(r.relation_type.as_bytes());
        w.u64(r.created_seq.0);
        let (origin, confidence): (&[u8], u16) = match &r.provenance {
            RelationProvenance::Declared => (&[], 0),
            RelationProvenance::Imported { source } => (source, 0),
            RelationProvenance::Algorithmic { origin } => (origin, 0),
            RelationProvenance::Inferred { origin, confidence_permille } => (origin, *confidence_permille),
        };
        w.u8(r.provenance.kind().to_byte());
        w.bytes(origin);
        w.0.extend_from_slice(&confidence.to_be_bytes());
    }
    Ok(())
}

/// Big-endian writer; every variable field is length-prefixed.
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            Some(v) => {
                self.u8(1);
                self.u64(v);
            }
            None => self.u8(0),
        }
    }
    fn opt_bytes(&mut self, v: Option<&[u8]>) {
        match v {
            Some(b) => {
                self.u8(1);
                self.bytes(b);
            }
            None => self.u8(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use kerno_core::contract::Options;
    use kerno_core::vfs::Vfs;
    use kerno_storage::KernoDb;
    use kerno_testkit::emulated::Emulated;
    use kerno_testkit::reference::LogEngine;
    use kerno_testkit::vfs::SimVfs;

    use super::*;
    use crate::clock::{CounterIds, FixedClock};
    use crate::db::{ChangeMeta, NewUnit, UnitConfig, UnitUpdate};
    use crate::types::Timestamp;

    type EmuLog = Emulated<LogEngine>;

    const T0: Timestamp = Timestamp(1_700_000_000_000_000);

    fn cfg() -> UnitConfig {
        let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
        c.engine = Options::small_for_tests();
        c
    }

    fn new_unit(t: &str, payload: &[u8]) -> NewUnit {
        NewUnit { unit_type: t.into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
    }

    /// The same logical operations on any engine.
    fn populate<E: TxnEngine>(db: &UnitDb<E>) {
        db.set_policy("note", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
        let mut cs = db.begin_changeset(ChangeMeta { actor: Some(b"ann".to_vec()), ..ChangeMeta::default() }).unwrap();
        let a = cs.create(new_unit("doc", b"doc-v1")).unwrap();
        let b = cs.create(new_unit("note", b"old-note")).unwrap();
        cs.create_relation(a, b, "cites", RelationProvenance::Declared).unwrap();
        cs.commit().unwrap();
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(a, UnitUpdate { schema_version: None, metadata: None, payload: b"doc-v2".to_vec() }, None).unwrap();
        cs.update(b, UnitUpdate { schema_version: None, metadata: None, payload: b"new-note".to_vec() }, None).unwrap();
        cs.commit().unwrap();
    }

    fn open<E: TxnEngine>(dir: &str) -> UnitDb<E> {
        let vfs: Arc<dyn Vfs> = Arc::new(SimVfs::new(1));
        UnitDb::<E>::open(vfs, Path::new(dir), &cfg()).unwrap()
    }

    fn rules<E: TxnEngine>() {
        let db = open::<E>("/db");
        // An empty database exports header, no policy, no units, no relations.
        let empty = export(&db).unwrap();
        assert_eq!(&empty[..8], &EXPORT_MAGIC);
        assert_ne!(&empty[..8], b"KRNO_EXP");
        assert_eq!(u32::from_be_bytes(empty[8..12].try_into().unwrap()), EXPORT_VERSION);
        assert_eq!(empty[12], FLAG_DRAFT);
        // header, default policy, then the counts of policies, units, relations and tokens.
        assert_eq!(empty.len(), 16 + 1 + 4 + 4 + 4 + 4 + DIGEST_LEN);
        assert_eq!(empty[16], NO_DEFAULT_POLICY);

        populate(&db);
        let a = export(&db).unwrap();
        // Deterministic: the same state gives the same bytes.
        assert_eq!(a, export(&db).unwrap());
        // The closing digest covers everything before it.
        let (body, digest) = a.split_at(a.len() - DIGEST_LEN);
        assert_eq!(blake3::hash(body).as_bytes(), digest);
        // The content is there; derived data is not part of it.
        let has = |needle: &[u8]| a.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"doc-v1") && has(b"doc-v2") && has(b"new-note") && has(b"cites") && has(b"note") && has(b"ann"));
        // Audit keeps no content for the old revision of a `note`.
        assert!(!has(b"old-note"));
        // A change of state changes the bytes.
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.create(new_unit("doc", b"gamma")).unwrap();
        cs.commit().unwrap();
        assert_ne!(a, export(&db).unwrap());
        // The snapshot entry point answers like the database one.
        assert_eq!(export_snapshot(&db.snapshot().unwrap()).unwrap(), export(&db).unwrap());
    }

    #[test]
    fn rules_emulated_log() {
        rules::<EmuLog>();
    }

    #[test]
    fn rules_kerno_db() {
        rules::<KernoDb>();
    }

    /// The logical state, not the engine, decides the bytes.
    #[test]
    fn same_bytes_on_both_engines() {
        let log = open::<EmuLog>("/db");
        let kdb = open::<KernoDb>("/db");
        populate(&log);
        populate(&kdb);
        assert_eq!(export(&log).unwrap(), export(&kdb).unwrap());
    }
}
