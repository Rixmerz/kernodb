//! Independent decoder of the units layer, written from
//! `docs/file-format/units-v1.md` alone (§2 keys, §3 values, §4 hash, §5
//! rules, §6 order of checks). It imports no encoding or parsing code: the
//! only thing it shares with the vector encoder is the plain data of
//! `units_model`.

use crate::units_model::{ChangeSet, Class, Hash, Head, Layer, Member, Rec, RevMeta, RevPayload};

const MAX_PAYLOAD_LEN: u32 = 1 << 26;
const MAX_MEMBERS: u32 = 1 << 20;

/// A rejection: the rule of §5 and its class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reject {
    pub rule: u8,
    pub class: Class,
}

fn rule(rule: u8) -> Reject {
    let class = match rule {
        2 | 7 | 16 => Class::IncompatibleLayer,
        _ => Class::Corruption,
    };
    Reject { rule, class }
}

type Res<T> = Result<T, Reject>;

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.buf.len()).ok_or(rule(9))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn arr<const N: usize>(&mut self) -> Res<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take returned N bytes"))
    }

    fn u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_be_bytes(self.arr()?))
    }

    fn u64(&mut self) -> Res<u64> {
        Ok(u64::from_be_bytes(self.arr()?))
    }

    fn i64(&mut self) -> Res<i64> {
        Ok(i64::from_be_bytes(self.arr()?))
    }

    /// `bytes(max)`: the maximum is checked before the remaining length.
    fn bytes(&mut self, max: u32) -> Res<Vec<u8>> {
        let len = self.u32()?;
        if len > max {
            return Err(rule(11));
        }
        Ok(self.take(len as usize)?.to_vec())
    }

    fn hash(&mut self) -> Res<Hash> {
        let alg = self.u8()?;
        if alg > 1 {
            return Err(rule(13));
        }
        let digest: [u8; 32] = self.arr()?;
        if alg == 0 && digest[4..].iter().any(|b| *b != 0) {
            return Err(rule(14));
        }
        Ok(Hash { alg, digest })
    }

    fn finish(&self) -> Res<()> {
        if self.pos == self.buf.len() { Ok(()) } else { Err(rule(10)) }
    }
}

/// Envelope header: `envelope_version` and `reserved_flags` (§1.1).
fn header(r: &mut Reader<'_>) -> Res<()> {
    match r.u8()? {
        0 => return Err(rule(6)),
        1 => {}
        _ => return Err(rule(7)),
    }
    if r.u8()? != 0 { Err(rule(8)) } else { Ok(()) }
}

fn payload_len(r: &mut Reader<'_>) -> Res<u32> {
    let n = r.u32()?;
    if n > MAX_PAYLOAD_LEN { Err(rule(22)) } else { Ok(n) }
}

fn codec(r: &mut Reader<'_>) -> Res<u8> {
    let c = r.u8()?;
    if c != 0 { Err(rule(13)) } else { Ok(c) }
}

fn enum_max(r: &mut Reader<'_>, last: u8) -> Res<u8> {
    let v = r.u8()?;
    if v > last { Err(rule(13)) } else { Ok(v) }
}

fn rev_kind(r: &mut Reader<'_>) -> Res<u8> {
    let k = r.u8()?;
    if (1..=6).contains(&k) { Ok(k) } else { Err(rule(13)) }
}

/// Splits a `unit_id[16] ‖ u64` key tail.
fn unit_and_u64(tail: &[u8]) -> ([u8; 16], u64) {
    (tail[..16].try_into().expect("16 bytes"), u64::from_be_bytes(tail[16..24].try_into().expect("8 bytes")))
}

pub fn decode(key: &[u8], value: &[u8]) -> Res<Rec> {
    let Some(&tag) = key.first() else { return Err(rule(1)) };
    let want = match tag {
        0x01 => None,
        0x10 => Some(17),
        0x20 | 0x21 => Some(25),
        0x30 => Some(9),
        0x31 => Some(13),
        0x00..=0x3F => return Err(rule(1)),
        _ => return Err(rule(2)),
    };
    match want {
        Some(n) if key.len() != n => return Err(rule(3)),
        None if !(2..=65).contains(&key.len()) => return Err(rule(3)),
        _ => {}
    }
    let tail = &key[1..];
    if tag == 0x01 && std::str::from_utf8(tail).is_err() {
        return Err(rule(4));
    }
    match tag {
        0x20 | 0x21 if unit_and_u64(tail).1 == 0 => return Err(rule(5)),
        0x30 | 0x31 if u64::from_be_bytes(tail[..8].try_into().expect("8 bytes")) == 0 => return Err(rule(5)),
        _ => {}
    }
    if tag == 0x21 {
        let (unit_id, rev_no) = unit_and_u64(tail);
        return Ok(Rec::RevPayload(RevPayload { unit_id, rev_no, payload: value.to_vec() }));
    }

    let mut r = Reader { buf: value, pos: 0 };
    header(&mut r)?;
    match tag {
        0x01 => {
            let layer_version = r.u32()?;
            match layer_version {
                0 => return Err(rule(15)),
                1 => {}
                _ => return Err(rule(16)),
            }
            let history_policy = enum_max(&mut r, 2)?;
            r.finish()?;
            Ok(Rec::Layer(Layer { name: tail.to_vec(), layer_version, history_policy }))
        }
        0x10 => {
            let unit_type = r.bytes(64)?;
            if unit_type.is_empty() || std::str::from_utf8(&unit_type).is_err() {
                return Err(rule(12));
            }
            let schema_version = r.u32()?;
            let lifecycle = enum_max(&mut r, 2)?;
            let (head_rev, oldest_rev, created_seq, updated_seq) = (r.u64()?, r.u64()?, r.u64()?, r.u64()?);
            let (created_at, updated_at) = (r.i64()?, r.i64()?);
            let content_hash = r.hash()?;
            let payload_len = payload_len(&mut r)?;
            let payload_codec = codec(&mut r)?;
            let metadata = r.bytes(4096)?;
            r.finish()?;
            if oldest_rev < 1 || oldest_rev > head_rev {
                return Err(rule(17));
            }
            if created_seq < 1 || created_seq > updated_seq {
                return Err(rule(18));
            }
            Ok(Rec::Head(Head {
                unit_id: tail.try_into().expect("16 bytes"),
                unit_type,
                schema_version,
                lifecycle,
                head_rev,
                oldest_rev,
                created_seq,
                updated_seq,
                created_at,
                updated_at,
                content_hash,
                payload_len,
                payload_codec,
                metadata,
            }))
        }
        0x20 => {
            let (unit_id, rev_no) = unit_and_u64(tail);
            let kind = rev_kind(&mut r)?;
            let parent_rev = r.u64()?;
            let commit_seq = r.u64()?;
            if commit_seq == 0 {
                return Err(rule(23));
            }
            let created_at = r.i64()?;
            let schema_version = r.u32()?;
            let content_hash = r.hash()?;
            let payload_len = payload_len(&mut r)?;
            let payload_codec = codec(&mut r)?;
            let payload_present = r.u8()?;
            let restored_from = r.u64()?;
            let metadata = r.bytes(4096)?;
            let actor_ref = r.bytes(256)?;
            let source_ref = r.bytes(256)?;
            r.finish()?;
            let parent_ok = if kind == 1 { parent_rev == 0 } else { (1..rev_no).contains(&parent_rev) };
            if !parent_ok {
                return Err(rule(19));
            }
            let restored_ok = if kind == 6 { (1..rev_no).contains(&restored_from) } else { restored_from == 0 };
            if !restored_ok {
                return Err(rule(20));
            }
            if payload_present > 1 || (kind == 5 && (payload_present != 0 || payload_len != 0)) {
                return Err(rule(21));
            }
            Ok(Rec::RevMeta(RevMeta {
                unit_id,
                rev_no,
                kind,
                parent_rev,
                commit_seq,
                created_at,
                schema_version,
                content_hash,
                payload_len,
                payload_codec,
                payload_present,
                restored_from,
                metadata,
                actor_ref,
                source_ref,
            }))
        }
        0x30 => {
            let created_at = r.i64()?;
            let member_count = r.u32()?;
            if member_count > MAX_MEMBERS {
                return Err(rule(24));
            }
            let purpose = enum_max(&mut r, 2)?;
            let reason = r.bytes(1024)?;
            let actor_ref = r.bytes(1024)?;
            let source_ref = r.bytes(1024)?;
            let correlation = r.bytes(1024)?;
            r.finish()?;
            Ok(Rec::ChangeSet(ChangeSet {
                commit_seq: u64::from_be_bytes(tail.try_into().expect("8 bytes")),
                created_at,
                member_count,
                purpose,
                reason,
                actor_ref,
                source_ref,
                correlation,
            }))
        }
        _ => {
            let unit_id: [u8; 16] = r.arr()?;
            let rev_no = r.u64()?;
            if rev_no == 0 {
                return Err(rule(25));
            }
            let kind = rev_kind(&mut r)?;
            r.finish()?;
            Ok(Rec::Member(Member {
                commit_seq: u64::from_be_bytes(tail[..8].try_into().expect("8 bytes")),
                ordinal: u32::from_be_bytes(tail[8..12].try_into().expect("4 bytes")),
                unit_id,
                rev_no,
                kind,
            }))
        }
    }
}
