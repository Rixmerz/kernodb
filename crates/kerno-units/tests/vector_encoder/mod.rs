//! The vector encoder: builds the `(key, value)` bytes of a [`Rec`]
//! following `units-v1.md` §2–§4. Test helper only (E3-02 owns the real
//! codecs); it shares nothing with `spec_decoder` but the plain data of
//! `units_model`. It encodes whatever the record holds, valid or not.

use crate::units_model::{ChangeSet, Hash, Head, Layer, Member, Rec, RevMeta, RevPayload};

/// The envelope header every value but `REV_PAYLOAD` starts with.
fn envelope() -> Vec<u8> {
    vec![1, 0]
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&u32::try_from(b.len()).expect("a vector field fits u32").to_be_bytes());
    out.extend_from_slice(b);
}

fn put_hash(out: &mut Vec<u8>, h: &Hash) {
    out.push(h.alg);
    out.extend_from_slice(&h.digest);
}

pub fn encode(rec: &Rec) -> (Vec<u8>, Vec<u8>) {
    match rec {
        Rec::Layer(Layer { name, layer_version, history_policy }) => {
            let mut key = vec![0x01];
            key.extend_from_slice(name);
            let mut v = envelope();
            v.extend_from_slice(&layer_version.to_be_bytes());
            v.push(*history_policy);
            (key, v)
        }
        Rec::Head(h) => {
            let Head { unit_id, unit_type, schema_version, lifecycle, head_rev, oldest_rev, .. } = h;
            let mut key = vec![0x10];
            key.extend_from_slice(unit_id);
            let mut v = envelope();
            put_bytes(&mut v, unit_type);
            v.extend_from_slice(&schema_version.to_be_bytes());
            v.push(*lifecycle);
            for n in [head_rev, oldest_rev, &h.created_seq, &h.updated_seq] {
                v.extend_from_slice(&n.to_be_bytes());
            }
            v.extend_from_slice(&h.created_at.to_be_bytes());
            v.extend_from_slice(&h.updated_at.to_be_bytes());
            put_hash(&mut v, &h.content_hash);
            v.extend_from_slice(&h.payload_len.to_be_bytes());
            v.push(h.payload_codec);
            put_bytes(&mut v, &h.metadata);
            (key, v)
        }
        Rec::RevMeta(m) => {
            let RevMeta { unit_id, rev_no, .. } = m;
            let mut key = vec![0x20];
            key.extend_from_slice(unit_id);
            key.extend_from_slice(&rev_no.to_be_bytes());
            let mut v = envelope();
            v.push(m.kind);
            v.extend_from_slice(&m.parent_rev.to_be_bytes());
            v.extend_from_slice(&m.commit_seq.to_be_bytes());
            v.extend_from_slice(&m.created_at.to_be_bytes());
            v.extend_from_slice(&m.schema_version.to_be_bytes());
            put_hash(&mut v, &m.content_hash);
            v.extend_from_slice(&m.payload_len.to_be_bytes());
            v.push(m.payload_codec);
            v.push(m.payload_present);
            v.extend_from_slice(&m.restored_from.to_be_bytes());
            put_bytes(&mut v, &m.metadata);
            put_bytes(&mut v, &m.actor_ref);
            put_bytes(&mut v, &m.source_ref);
            (key, v)
        }
        Rec::RevPayload(RevPayload { unit_id, rev_no, payload }) => {
            let mut key = vec![0x21];
            key.extend_from_slice(unit_id);
            key.extend_from_slice(&rev_no.to_be_bytes());
            (key, payload.clone())
        }
        Rec::ChangeSet(c) => {
            let ChangeSet { commit_seq, .. } = c;
            let mut key = vec![0x30];
            key.extend_from_slice(&commit_seq.to_be_bytes());
            let mut v = envelope();
            v.extend_from_slice(&c.created_at.to_be_bytes());
            v.extend_from_slice(&c.member_count.to_be_bytes());
            v.push(c.purpose);
            for b in [&c.reason, &c.actor_ref, &c.source_ref, &c.correlation] {
                put_bytes(&mut v, b);
            }
            (key, v)
        }
        Rec::Member(Member { commit_seq, ordinal, unit_id, rev_no, kind }) => {
            let mut key = vec![0x31];
            key.extend_from_slice(&commit_seq.to_be_bytes());
            key.extend_from_slice(&ordinal.to_be_bytes());
            let mut v = envelope();
            v.extend_from_slice(unit_id);
            v.extend_from_slice(&rev_no.to_be_bytes());
            v.push(*kind);
            (key, v)
        }
    }
}
