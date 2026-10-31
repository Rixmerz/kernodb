//! Plain data shared by the vector encoder and the independent decoder: the
//! decoded form of each structure of `units-v1.md`. It holds no parsing or
//! encoding code, and every field is a raw integer or byte string so that a
//! negative vector can carry a value the format rejects.

pub type UnitId = [u8; 16];

#[derive(Clone, Debug, PartialEq)]
pub struct Hash {
    pub alg: u8,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: Vec<u8>,
    pub layer_version: u32,
    pub history_policy: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    pub unit_id: UnitId,
    pub unit_type: Vec<u8>,
    pub schema_version: u32,
    pub lifecycle: u8,
    pub head_rev: u64,
    pub oldest_rev: u64,
    pub created_seq: u64,
    pub updated_seq: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub content_hash: Hash,
    pub payload_len: u32,
    pub payload_codec: u8,
    pub metadata: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RevMeta {
    pub unit_id: UnitId,
    pub rev_no: u64,
    pub kind: u8,
    pub parent_rev: u64,
    pub commit_seq: u64,
    pub created_at: i64,
    pub schema_version: u32,
    pub content_hash: Hash,
    pub payload_len: u32,
    pub payload_codec: u8,
    pub payload_present: u8,
    pub restored_from: u64,
    pub metadata: Vec<u8>,
    pub actor_ref: Vec<u8>,
    pub source_ref: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RevPayload {
    pub unit_id: UnitId,
    pub rev_no: u64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeSet {
    pub commit_seq: u64,
    pub created_at: i64,
    pub member_count: u32,
    pub purpose: u8,
    pub reason: Vec<u8>,
    pub actor_ref: Vec<u8>,
    pub source_ref: Vec<u8>,
    pub correlation: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub commit_seq: u64,
    pub ordinal: u32,
    pub unit_id: UnitId,
    pub rev_no: u64,
    pub kind: u8,
}

/// A decoded key and value.
#[derive(Clone, Debug, PartialEq)]
pub enum Rec {
    Layer(Layer),
    Head(Head),
    RevMeta(RevMeta),
    RevPayload(RevPayload),
    ChangeSet(ChangeSet),
    Member(Member),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Corruption,
    IncompatibleLayer,
}
