//! WAL (v1.md §6): commits in canonical form (WAL-C1..C4), frames (§6.2)
//! and the replay scanner (WAL-R1..R6).

use std::collections::BTreeMap;

use super::coding::{Reader, le_u32, le_u64, put_bytes, put_u32, put_u64, put_varint, varint_len};
use super::crc::{crc_ctx_parts, file_ctx};
use super::features::{FRAME_SLOT_ALWAYS, SLOT_LEN, frame_has_slot, slot_is_zero};
use super::header::{FileKind, Header, HeaderExpect, decode_header, is_known_magic};
use super::limits::{
    MAX_COMMIT_OPS, MAX_FRAME_COMMITS, MAX_FRAME_PAYLOAD, MAX_HEADER_LEN, MAX_KEY_LEN, MAX_SEQ, MAX_VALUE_LEN,
    MIN_HEADER_LEN,
};
use super::{ByteSource, End, FormatError, ReadError, Result, Window};

/// Fixed part of a frame, before the payload.
pub const FRAME_HEADER_LEN: usize = 32;

pub const OP_PUT: u8 = 0x01;
pub const OP_DELETE: u8 = 0x02;
pub const OP_DELETE_RANGE: u8 = 0x03;

/// One operation of a write batch, at the format level (independent of
/// `kerno_core::Op`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOp<'a> {
    Put {
        key: &'a [u8],
        value: &'a [u8],
    },
    Delete {
        key: &'a [u8],
    },
    /// Deletes `[start, end)`; `start >= end` has no effect (the API rejects
    /// `start > end` before canonicalizing).
    DeleteRange {
        start: &'a [u8],
        end: &'a [u8],
    },
}

/// A commit in canonical form (WAL-C1): every range, ordered by
/// `(start, end)` strictly increasing, then every point operation, ordered
/// by key strictly increasing. `None` is a point delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalCommit<B = Vec<u8>> {
    pub ranges: Vec<(B, B)>,
    pub points: Vec<(B, Option<B>)>,
}

impl<B> Default for CanonicalCommit<B> {
    fn default() -> Self {
        CanonicalCommit { ranges: Vec::new(), points: Vec::new() }
    }
}

impl<B: AsRef<[u8]>> CanonicalCommit<B> {
    /// `op_count` of the encoded commit.
    pub fn op_count(&self) -> usize {
        self.ranges.len() + self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.op_count() == 0
    }

    pub fn encoded_len(&self) -> usize {
        let bytes = |b: &[u8]| varint_len(b.len() as u64) + b.len();
        let ranges: usize = self.ranges.iter().map(|(s, e)| 1 + bytes(s.as_ref()) + bytes(e.as_ref())).sum();
        let points: usize =
            self.points.iter().map(|(k, v)| 1 + bytes(k.as_ref()) + v.as_ref().map_or(0, |v| bytes(v.as_ref()))).sum();
        varint_len(self.op_count() as u64) + ranges + points
    }

    /// Appends `op_count varint ‖ op*` (§6.3).
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        put_varint(out, self.op_count() as u64);
        for (start, end) in &self.ranges {
            out.push(OP_DELETE_RANGE);
            put_bytes(out, start.as_ref());
            put_bytes(out, end.as_ref());
        }
        for (key, value) in &self.points {
            match value {
                Some(v) => {
                    out.push(OP_PUT);
                    put_bytes(out, key.as_ref());
                    put_bytes(out, v.as_ref());
                }
                None => {
                    out.push(OP_DELETE);
                    put_bytes(out, key.as_ref());
                }
            }
        }
    }

    pub fn to_owned_commit(&self) -> CanonicalCommit<Vec<u8>> {
        CanonicalCommit {
            ranges: self.ranges.iter().map(|(s, e)| (s.as_ref().to_vec(), e.as_ref().to_vec())).collect(),
            points: self
                .points
                .iter()
                .map(|(k, v)| (k.as_ref().to_vec(), v.as_ref().map(|v| v.as_ref().to_vec())))
                .collect(),
        }
    }
}

/// WAL-C1: processes the batch in order. `Put`/`Delete(k)` replaces the
/// previous point operation on `k`; `DeleteRange(a, b)` with `a < b` drops
/// the previous point operations with keys in `[a, b)` and adds the range;
/// `DeleteRange(a, b)` with `a >= b` has no effect. Ranges are sorted and
/// deduplicated, then the points follow in key order.
///
/// Applying the ranges first and then the points, all at one sequence,
/// gives the state of applying the batch operation by operation (WAL-C2).
/// A non-empty batch can canonicalize to an empty commit (WAL-C3): it still
/// consumes a sequence.
pub fn canonicalize<'a>(ops: impl IntoIterator<Item = CommitOp<'a>>) -> CanonicalCommit<&'a [u8]> {
    let mut points: BTreeMap<&'a [u8], Option<&'a [u8]>> = BTreeMap::new();
    let mut ranges: Vec<(&'a [u8], &'a [u8])> = Vec::new();
    for op in ops {
        match op {
            CommitOp::Put { key, value } => {
                points.insert(key, Some(value));
            }
            CommitOp::Delete { key } => {
                points.insert(key, None);
            }
            CommitOp::DeleteRange { start, end } if start < end => {
                let mut covered = points.split_off(start);
                let mut after = covered.split_off(end);
                points.append(&mut after);
                ranges.push((start, end));
            }
            CommitOp::DeleteRange { .. } => {}
        }
    }
    ranges.sort_unstable();
    ranges.dedup();
    CanonicalCommit { ranges, points: points.into_iter().collect() }
}

/// Decodes one commit, checking the canonical form (WAL-C4) and the format
/// limits (VAR-3 for lengths). `r` must use the rule `WAL-C4`.
pub fn decode_commit(r: &mut Reader<'_>) -> Result<CanonicalCommit<Vec<u8>>> {
    let op_count = r.varint()?;
    if op_count > MAX_COMMIT_OPS {
        return Err(r.error_rule("WAL-C4", "op_count above the format limit"));
    }
    let mut commit = CanonicalCommit::default();
    for _ in 0..op_count {
        let at = r.offset();
        let canon = |detail| FormatError::corruption("WAL-C4", at, detail);
        match r.u8()? {
            OP_DELETE_RANGE => {
                if !commit.points.is_empty() {
                    return Err(canon("range after a point operation"));
                }
                let start = r.bytes(MAX_KEY_LEN)?;
                let end = r.bytes(MAX_KEY_LEN)?;
                if start >= end {
                    return Err(canon("range with start >= end"));
                }
                if commit.ranges.last().is_some_and(|(s, e): &(Vec<u8>, Vec<u8>)| (&s[..], &e[..]) >= (start, end)) {
                    return Err(canon("ranges not strictly increasing"));
                }
                commit.ranges.push((start.to_vec(), end.to_vec()));
            }
            op @ (OP_PUT | OP_DELETE) => {
                let key = r.bytes(MAX_KEY_LEN)?;
                if commit.points.last().is_some_and(|(k, _)| &k[..] >= key) {
                    return Err(canon("point keys not strictly increasing"));
                }
                let value = if op == OP_PUT { Some(r.bytes(MAX_VALUE_LEN)?.to_vec()) } else { None };
                commit.points.push((key.to_vec(), value));
            }
            _ => return Err(canon("unknown operation")),
        }
    }
    Ok(commit)
}

/// A decoded frame: commit `i` has sequence `first_seq + i`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame<B = Vec<u8>> {
    pub first_seq: u64,
    pub synced_len: u64,
    pub commits: Vec<CanonicalCommit<B>>,
}

/// Length of a frame with this payload.
pub fn frame_len(payload_len: u32, slot: bool) -> u64 {
    FRAME_HEADER_LEN as u64 + u64::from(payload_len) + if slot { SLOT_LEN as u64 } else { 0 }
}

/// Seals a frame around an encoded payload of `count` commits: the 32-byte
/// head, the payload, the zero slot when `slot`, and the CRC
/// `crc(file_number; bytes[0..4] ‖ bytes[8..32] ‖ payload ‖ slot?)`.
pub fn seal_frame(
    file_number: u64,
    first_seq: u64,
    synced_len: u64,
    count: u32,
    payload: &[u8],
    slot: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame_len(payload.len() as u32, slot) as usize);
    put_u32(&mut out, payload.len() as u32);
    put_u32(&mut out, 0);
    put_u64(&mut out, first_seq);
    put_u64(&mut out, synced_len);
    put_u32(&mut out, count);
    put_u32(&mut out, 0);
    out.extend_from_slice(payload);
    if slot {
        out.resize(out.len() + SLOT_LEN, 0);
    }
    reseal_frame(&mut out, file_number);
    out
}

/// Encodes a frame (the writer's side of §6.2).
pub fn encode_frame<B: AsRef<[u8]>>(file_number: u64, frame: &Frame<B>, slot: bool) -> Vec<u8> {
    let mut payload = Vec::with_capacity(frame.commits.iter().map(CanonicalCommit::encoded_len).sum());
    for c in &frame.commits {
        c.encode_into(&mut payload);
    }
    seal_frame(file_number, frame.first_seq, frame.synced_len, frame.commits.len() as u32, &payload, slot)
}

fn frame_crc(file_number: u64, frame: &[u8]) -> u32 {
    crc_ctx_parts(&file_ctx(file_number), &[&frame[0..4], &frame[8..]])
}

/// Recomputes the CRC of a whole frame in place (`frame` is exactly the
/// frame's bytes). Used by the writer, tests and crafted-file generators.
pub fn reseal_frame(frame: &mut [u8], file_number: u64) {
    let crc = frame_crc(file_number, frame);
    frame[4..8].copy_from_slice(&crc.to_le_bytes());
}

/// Whether the stored CRC of a whole frame verifies.
pub fn frame_crc_ok(frame: &[u8], file_number: u64) -> bool {
    frame.len() >= FRAME_HEADER_LEN && le_u32(frame, 4) == frame_crc(file_number, frame)
}

/// WAL-R3, AEAD-1 and WAL-C4 on a frame whose CRC verified. `frame` is
/// exactly the frame's bytes and `offset` its offset in the file.
pub fn check_frame(frame: &[u8], offset: u64, slot: bool, expected_seq: u64) -> Result<Frame> {
    let payload_len = le_u32(frame, 0) as usize;
    let first_seq = le_u64(frame, 8);
    let synced_len = le_u64(frame, 16);
    let count = le_u32(frame, 24);
    if first_seq != expected_seq {
        return Err(FormatError::corruption("WAL-R3", offset + 8, "first_seq is not the expected sequence"));
    }
    if count == 0
        || count > MAX_FRAME_COMMITS
        || first_seq.checked_add(u64::from(count) - 1).is_none_or(|l| l > MAX_SEQ)
    {
        return Err(FormatError::corruption("WAL-R3", offset + 24, "count out of range"));
    }
    if synced_len != 0 && !(u64::from(MIN_HEADER_LEN) <= synced_len && synced_len <= offset) {
        return Err(FormatError::corruption("WAL-R3", offset + 16, "synced_len out of range"));
    }
    if le_u32(frame, 28) != 0 {
        return Err(FormatError::corruption("WAL-R3", offset + 28, "reserved frame bytes are not zero"));
    }
    let payload_end = FRAME_HEADER_LEN + payload_len;
    if slot && !slot_is_zero(&frame[payload_end..]) {
        return Err(FormatError::corruption("AEAD-1", offset + payload_end as u64, "AEAD slot is not zero"));
    }
    let mut r = Reader::new(&frame[FRAME_HEADER_LEN..payload_end], offset + FRAME_HEADER_LEN as u64, "WAL-C4");
    let mut commits = Vec::with_capacity((count as usize).min(payload_len));
    for _ in 0..count {
        commits.push(decode_commit(&mut r)?);
    }
    r.finish()?;
    Ok(Frame { first_seq, synced_len, commits })
}

/// Decodes the frame at the start of `buf` (at file offset `offset`),
/// returning it and its length. An incomplete frame, a `payload_len` out of
/// 1..=2^30 or a CRC mismatch are WAL-R2 (the replay stops there); the rest
/// is [`check_frame`].
pub fn decode_frame(
    buf: &[u8],
    file_number: u64,
    offset: u64,
    slot: bool,
    expected_seq: u64,
) -> Result<(Frame, usize)> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FormatError::corruption("WAL-R2", offset, "incomplete frame"));
    }
    let payload_len = le_u32(buf, 0);
    if payload_len == 0 || payload_len > MAX_FRAME_PAYLOAD {
        return Err(FormatError::corruption("WAL-R2", offset, "payload_len out of range"));
    }
    let len = frame_len(payload_len, slot);
    if len > buf.len() as u64 {
        return Err(FormatError::corruption("WAL-R2", offset, "incomplete frame"));
    }
    let frame = &buf[..len as usize];
    if !frame_crc_ok(frame, file_number) {
        return Err(FormatError::corruption("WAL-R2", offset + 4, "frame CRC mismatch"));
    }
    Ok((check_frame(frame, offset, slot, expected_seq)?, len as usize))
}

/// What the replay of one WAL is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalScanParams {
    /// The number in the file name (CRC context and HDR-10).
    pub file_number: u64,
    /// The MANIFEST's `db_id` (HDR-10).
    pub db_id: [u8; 16],
    /// Sequence the first commit must carry (WAL-R1).
    pub expected_seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalHeaderState {
    Valid(Header),
    /// The header failed HDR-1, HDR-3 or HDR-2 with an unknown magic (not
    /// v1, not M0), and no frame with `synced_len > 0` follows (WAL-R6): the WAL
    /// has no frames.
    Broken {
        rule: &'static str,
    },
}

/// Result of [`scan_wal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalScan {
    pub header: WalHeaderState,
    pub frames: u64,
    pub commits: u64,
    /// `P`: the end of the last valid frame (0 with a broken header).
    pub valid_len: u64,
    /// How the file ends at `valid_len`.
    pub end: End,
    /// The sequence after the last replayed commit.
    pub next_seq: u64,
}

/// WAL-R6: a header that fails HDR-1 or HDR-3, or HDR-2 with a magic that
/// is neither v1 nor M0 (a never-synced header can hold zeros or stray
/// bytes, never another file type's magic).
fn header_is_broken(e: &FormatError, prefix: &[u8]) -> bool {
    matches!(e.rule, "HDR-1" | "HDR-3") || (e.rule == "HDR-2" && !is_known_magic(&prefix[..8]))
}

/// WAL-R5 and WAL-R6: the first offset `q > p` holding a frame with
/// `payload_len` in range, that fits in the file, with
/// `first_seq ∈ [expected, expected + (q − p)]`, `p < synced_len ≤ q` and a
/// valid CRC. With `any_synced_len` (a broken header, WAL-R6) the `synced_len`
/// conditions are dropped: the writer syncs the header before it accepts the
/// first frame, so no frame can be older than a durable header.
///
/// The bound on `first_seq` holds for frames of any number of commits: each
/// frame lost in `[p, q)` has a 32-byte head and each of its commits at least
/// one payload byte (an empty commit is `op_count = 0`, WAL-C3), so fewer
/// than `q − p` commits precede the frame at `q`.
fn find_candidate<S: ByteSource>(
    w: &mut Window<'_, S>,
    p: u64,
    expected: u64,
    file_number: u64,
    slot: bool,
    any_synced_len: bool,
) -> std::result::Result<Option<u64>, S::Error> {
    let len = w.len();
    let mut q = p + 1;
    while q + FRAME_HEADER_LEN as u64 <= len {
        let (payload_len, first_seq, synced_len) = {
            let h = w.get(q, FRAME_HEADER_LEN)?;
            (le_u32(&h, 0), le_u64(&h, 8), le_u64(&h, 16))
        };
        let flen = frame_len(payload_len, slot);
        let plausible = (1..=MAX_FRAME_PAYLOAD).contains(&payload_len)
            && flen <= len - q
            && (expected..=expected.saturating_add(q - p)).contains(&first_seq)
            && (any_synced_len || (p < synced_len && synced_len <= q));
        if plausible && frame_crc_ok(&w.get(q, flen as usize)?, file_number) {
            return Ok(Some(q));
        }
        q += 1;
    }
    Ok(None)
}

/// Classifies the bytes from `p` to the end: [`End::Clean`],
/// [`End::ZeroPadded`] (WAL-R4) or [`End::Torn`] (WAL-R5), or the offset of
/// a candidate frame that makes the damage `Corruption`.
fn classify_tail<S: ByteSource>(
    w: &mut Window<'_, S>,
    p: u64,
    expected: u64,
    file_number: u64,
    slot: bool,
    any_synced_len: bool,
) -> std::result::Result<std::result::Result<End, u64>, S::Error> {
    if p == w.len() {
        return Ok(Ok(End::Clean));
    }
    if w.all_zero_from(p)? {
        return Ok(Ok(End::ZeroPadded));
    }
    Ok(match find_candidate(w, p, expected, file_number, slot, any_synced_len)? {
        Some(q) => Err(q),
        None => Ok(End::Torn),
    })
}

/// Replays one WAL (§6.4), streaming through `src` in reads of at most
/// 1 MiB (a frame larger than that is read whole). Calls `on_frame` with the
/// offset and contents of every valid frame, in order.
///
/// The frames passed to `on_frame` are **provisional**: the tail is
/// classified only after the last valid frame, and it can still fail with
/// WAL-R5, so a scan that delivered frames can end in `Corruption`. Whether
/// this WAL may end torn or have a broken header is decided later still, by
/// REC-10 over the whole chain of live WALs. The callback may only build
/// in-memory state (memtables) and must not write anything: recovery
/// classifies and validates everything first, without writing (v1.md §9.4,
/// S-1), and flushes the replayed memtables to SSTs (S-2) only after every
/// scan returned `Ok` and the chain was classified.
///
/// Errors: HDR-* (other than a broken header, WAL-R6), WAL-R1, WAL-R3,
/// WAL-C4, VAR-*, AEAD-1, WAL-R5 (damage followed by a frame written after
/// it was durable) and WAL-R6 (a broken header followed by such a frame).
/// Which WALs may end torn or have a broken header is decided by REC-10.
pub fn scan_wal<S: ByteSource>(
    src: &mut S,
    params: &WalScanParams,
    mut on_frame: impl FnMut(u64, Frame) -> std::result::Result<(), S::Error>,
) -> std::result::Result<WalScan, ReadError<S::Error>> {
    let mut w = Window::new(src);
    let len = w.len();
    let prefix = w.get(0, len.min(u64::from(MAX_HEADER_LEN)) as usize).map_err(ReadError::Source)?.into_owned();
    let expect = HeaderExpect { db_id: Some(params.db_id), file_number: Some(params.file_number) };
    let (header, slot, mut pos) = match decode_header(&prefix, len, FileKind::Wal, &expect) {
        Ok(h) => {
            if h.per_type != params.expected_seq {
                return Err(FormatError::corruption("WAL-R1", 64, "base_seq is not the expected sequence").into());
            }
            (h, frame_has_slot(h.incompat), u64::from(h.header_len))
        }
        Err(e) if header_is_broken(&e, &prefix) => {
            let tail = classify_tail(&mut w, 0, params.expected_seq, params.file_number, FRAME_SLOT_ALWAYS, true)
                .map_err(ReadError::Source)?;
            let end = tail.map_err(|q| {
                FormatError::corruption("WAL-R6", q, "broken header followed by a frame written after a sync")
            })?;
            return Ok(WalScan {
                header: WalHeaderState::Broken { rule: e.rule },
                frames: 0,
                commits: 0,
                valid_len: 0,
                end,
                next_seq: params.expected_seq,
            });
        }
        Err(e) => return Err(e.into()),
    };
    let mut expected = params.expected_seq;
    let (mut frames, mut commits) = (0u64, 0u64);
    while len - pos >= FRAME_HEADER_LEN as u64 {
        let payload_len = le_u32(&w.get(pos, FRAME_HEADER_LEN).map_err(ReadError::Source)?, 0);
        if payload_len == 0 || payload_len > MAX_FRAME_PAYLOAD {
            break;
        }
        let flen = frame_len(payload_len, slot);
        if flen > len - pos {
            break;
        }
        let bytes = w.get(pos, flen as usize).map_err(ReadError::Source)?;
        if !frame_crc_ok(&bytes, params.file_number) {
            break;
        }
        let frame = check_frame(&bytes, pos, slot, expected)?;
        let n = frame.commits.len() as u64;
        on_frame(pos, frame).map_err(ReadError::Source)?;
        expected += n;
        frames += 1;
        commits += n;
        pos += flen;
    }
    let tail = classify_tail(&mut w, pos, expected, params.file_number, slot, false).map_err(ReadError::Source)?;
    let end = tail
        .map_err(|q| FormatError::corruption("WAL-R5", q, "damage followed by a frame written after it was durable"))?;
    Ok(WalScan { header: WalHeaderState::Valid(header), frames, commits, valid_len: pos, end, next_seq: expected })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerno_testkit::rng::Rng;
    use std::collections::BTreeMap;

    type Model = BTreeMap<Vec<u8>, Vec<u8>>;

    fn apply_sequential(state: &mut Model, ops: &[CommitOp<'_>]) {
        for op in ops {
            match *op {
                CommitOp::Put { key, value } => {
                    state.insert(key.to_vec(), value.to_vec());
                }
                CommitOp::Delete { key } => {
                    state.remove(key);
                }
                CommitOp::DeleteRange { start, end } => {
                    state.retain(|k, _| !(start <= &k[..] && &k[..] < end));
                }
            }
        }
    }

    fn apply_canonical(state: &mut Model, c: &CanonicalCommit<&[u8]>) {
        for (start, end) in &c.ranges {
            state.retain(|k, _| !(*start <= &k[..] && &k[..] < *end));
        }
        for (key, value) in &c.points {
            match value {
                Some(v) => {
                    state.insert(key.to_vec(), v.to_vec());
                }
                None => {
                    state.remove(*key);
                }
            }
        }
    }

    #[test]
    fn canonicalize_matches_sequential_application() {
        let keys: Vec<Vec<u8>> = (0..12u8).map(|i| vec![b'a' + i]).collect();
        let mut rng = Rng::new(0x005E_EDC1);
        for _ in 0..20_000 {
            let mut base = Model::new();
            for k in &keys {
                if rng.chance(1, 2) {
                    base.insert(k.clone(), vec![b'0']);
                }
            }
            let values: Vec<Vec<u8>> = (0..8u8).map(|i| vec![b'v', i]).collect();
            let n = rng.below(10) as usize;
            let mut ops = Vec::new();
            for _ in 0..n {
                let k = |rng: &mut Rng| &keys[rng.below(keys.len() as u64) as usize][..];
                ops.push(match rng.below(4) {
                    0 | 1 => CommitOp::Put { key: k(&mut rng), value: &values[rng.below(8) as usize] },
                    2 => CommitOp::Delete { key: k(&mut rng) },
                    _ => {
                        let (a, b) = (k(&mut rng), k(&mut rng));
                        // Mostly a <= b; DeleteRange(a, a) and a > b are no-ops.
                        let (a, b) = if rng.chance(9, 10) && a > b { (b, a) } else { (a, b) };
                        CommitOp::DeleteRange { start: a, end: b }
                    }
                });
            }
            let mut expected = base.clone();
            // a > b ranges are rejected by the API; the model treats them as no-ops too.
            let sane: Vec<CommitOp<'_>> = ops
                .iter()
                .copied()
                .filter(|op| !matches!(op, CommitOp::DeleteRange { start, end } if start > end))
                .collect();
            apply_sequential(&mut expected, &sane);
            let canon = canonicalize(ops.iter().copied());
            let mut got = base.clone();
            apply_canonical(&mut got, &canon);
            assert_eq!(got, expected, "ops {ops:?} canonical {canon:?}");
            // The canonical form is what the decoder accepts, and it roundtrips.
            assert!(canon.ranges.windows(2).all(|w| w[0] < w[1]));
            assert!(canon.ranges.iter().all(|(s, e)| s < e));
            assert!(canon.points.windows(2).all(|w| w[0].0 < w[1].0));
            let mut enc = Vec::new();
            canon.encode_into(&mut enc);
            assert_eq!(enc.len(), canon.encoded_len());
            let mut r = Reader::new(&enc, 0, "WAL-C4");
            assert_eq!(decode_commit(&mut r).unwrap(), canon.to_owned_commit());
            assert!(r.is_empty());
        }
    }

    #[test]
    fn canonical_form_examples() {
        let c = canonicalize([
            CommitOp::Put { key: b"b", value: b"1" },
            CommitOp::DeleteRange { start: b"a", end: b"c" },
            CommitOp::Put { key: b"b", value: b"2" },
            CommitOp::Delete { key: b"z" },
            CommitOp::DeleteRange { start: b"a", end: b"c" },
            CommitOp::DeleteRange { start: b"q", end: b"q" },
        ]);
        let b2: &[u8] = b"2";
        assert_eq!(c.ranges, vec![(&b"a"[..], &b"c"[..])]);
        assert_eq!(c.points, vec![(&b"z"[..], None)]);
        assert_eq!(c.op_count(), 2);
        let c =
            canonicalize([CommitOp::DeleteRange { start: b"a", end: b"c" }, CommitOp::Put { key: b"b", value: b"2" }]);
        assert_eq!(c.points, vec![(&b"b"[..], Some(b2))]);
        // WAL-C3: only no-op ranges give an empty commit.
        assert!(canonicalize([CommitOp::DeleteRange { start: b"a", end: b"a" }]).is_empty());
    }

    #[test]
    fn scanner_streams_frames_and_classifies_tails() {
        let db = [7; 16];
        let mut file = Header::wal(db, 9, 5).encode().to_vec();
        let commit = canonicalize([CommitOp::Put { key: b"k", value: b"v" }]).to_owned_commit();
        let f1 = Frame { first_seq: 5, synced_len: 0, commits: vec![commit.clone(), commit.clone()] };
        file.extend(encode_frame(9, &f1, false));
        let second_at = file.len() as u64;
        let f2 = Frame { first_seq: 7, synced_len: second_at, commits: vec![commit] };
        file.extend(encode_frame(9, &f2, false));
        let params = WalScanParams { file_number: 9, db_id: db, expected_seq: 5 };
        let scan = |bytes: &[u8]| {
            let mut got = Vec::new();
            let r = scan_wal(&mut &bytes[..], &params, |off, f| {
                got.push((off, f));
                Ok(())
            })
            .map_err(ReadError::into_format);
            (r, got)
        };
        let (r, got) = scan(&file);
        let r = r.unwrap();
        assert_eq!((r.frames, r.commits, r.next_seq, r.end, r.valid_len), (2, 3, 8, End::Clean, file.len() as u64));
        assert_eq!(got[0], (96, f1.clone()));
        // Zero padding is a clean end.
        let mut padded = file.clone();
        padded.resize(file.len() + 300, 0);
        assert_eq!(scan(&padded).0.unwrap().end, End::ZeroPadded);
        // Damage in the last frame is a torn tail.
        let mut torn = file.clone();
        *torn.last_mut().unwrap() ^= 1;
        let r = scan(&torn).0.unwrap();
        assert_eq!((r.frames, r.end, r.valid_len), (1, End::Torn, second_at));
        // Damage in the first frame followed by a frame with synced_len > P
        // is corruption (WAL-R5).
        let mut mid = file.clone();
        mid[100] ^= 1;
        assert_eq!(scan(&mid).0.unwrap_err().rule, "WAL-R5");
        // Wrong base_seq is WAL-R1.
        let other = WalScanParams { expected_seq: 6, ..params };
        assert_eq!(
            scan_wal(&mut &file[..], &other, |_, _| Ok(())).map_err(ReadError::into_format).unwrap_err().rule,
            "WAL-R1"
        );
        // A broken header followed by a valid frame is WAL-R6 whatever its
        // synced_len (the writer syncs the header before the first frame);
        // with no frame after it, it counts as frameless.
        let mut broken = file.clone();
        broken[20] ^= 1;
        assert_eq!(scan(&broken).0.unwrap_err().rule, "WAL-R6");
        let mut zeroed = file[..second_at as usize].to_vec();
        zeroed[..96].fill(0);
        assert_eq!(scan(&zeroed).0.unwrap_err().rule, "WAL-R6");
        let mut header_only = file[..96].to_vec();
        header_only[20] ^= 1;
        let r = scan(&header_only).0.unwrap();
        assert_eq!((r.header, r.frames, r.end), (WalHeaderState::Broken { rule: "HDR-3" }, 0, End::Torn));
        // An empty file has a broken header (HDR-1) and no frames.
        let r = scan(&[]).0.unwrap();
        assert_eq!((r.header, r.end), (WalHeaderState::Broken { rule: "HDR-1" }, End::Clean));
    }

    fn scan_bytes(bytes: &[u8], params: &WalScanParams) -> std::result::Result<WalScan, FormatError> {
        scan_wal(&mut &bytes[..], params, |_, _| Ok(())).map_err(ReadError::into_format)
    }

    /// WAL-R5 and WAL-R6 with group frames: a frame holds many commits, so
    /// the candidate bound counts commits per byte, not frames.
    #[test]
    fn damage_in_a_durable_group_frame_is_corruption() {
        let db = [7; 16];
        let params = WalScanParams { file_number: 3, db_id: db, expected_seq: 1 };
        let keys: Vec<[u8; 1]> = (0..100u8).map(|i| [i]).collect();
        let small: Vec<CanonicalCommit> =
            keys.iter().map(|k| canonicalize([CommitOp::Put { key: k, value: b"v" }]).to_owned_commit()).collect();
        assert!(small.iter().all(|c| c.encoded_len() == 6));
        // WAL-C3: empty commits take one payload byte each.
        let empty: Vec<CanonicalCommit> = vec![CanonicalCommit::default(); 1024];
        for group in [small, empty] {
            let n = group.len() as u64;
            let mut file = Header::wal(db, 3, 1).encode().to_vec();
            let g1 = encode_frame(3, &Frame { first_seq: 1, synced_len: 0, commits: group }, false);
            file.extend(&g1);
            let next = canonicalize([CommitOp::Put { key: b"next", value: b"1" }]).to_owned_commit();
            // G1 was durable when G2 was written: synced_len is past P = 96.
            let g2 = Frame { first_seq: 1 + n, synced_len: 96 + g1.len() as u64, commits: vec![next] };
            file.extend(encode_frame(3, &g2, false));
            let r = scan_bytes(&file, &params).unwrap();
            assert_eq!((r.frames, r.commits, r.end), (2, n + 1, End::Clean));
            let mut damaged = file.clone();
            damaged[96 + 40] ^= 1;
            let e = scan_bytes(&damaged, &params).unwrap_err();
            assert_eq!((e.rule, e.offset), ("WAL-R5", 96 + g1.len() as u64));
            // The same frames under a broken header: G2 has synced_len > 0.
            let mut broken = damaged.clone();
            broken[20] ^= 1;
            assert_eq!(scan_bytes(&broken, &params).unwrap_err().rule, "WAL-R6");
            // Without G2 the damage is a torn tail.
            let torn = &damaged[..96 + g1.len()];
            let r = scan_bytes(torn, &params).unwrap();
            assert_eq!((r.frames, r.end, r.valid_len), (0, End::Torn, 96));
        }
    }

    /// WAL-R5's accept side: a later frame whose `synced_len` does not go
    /// past `P` (a `Buffered` writer that never synced the damaged bytes)
    /// leaves a torn tail.
    #[test]
    fn later_frame_synced_up_to_p_leaves_a_torn_tail() {
        let db = [7; 16];
        let params = WalScanParams { file_number: 3, db_id: db, expected_seq: 1 };
        let c = canonicalize([CommitOp::Put { key: b"k", value: b"v" }]).to_owned_commit();
        let mut file = Header::wal(db, 3, 1).encode().to_vec();
        file.extend(encode_frame(3, &Frame { first_seq: 1, synced_len: 0, commits: vec![c.clone()] }, false));
        let p = file.len() as u64;
        file.extend(encode_frame(3, &Frame { first_seq: 2, synced_len: p, commits: vec![c.clone()] }, false));
        let q = file.len() as u64;
        let tail = encode_frame(3, &Frame { first_seq: 3, synced_len: p, commits: vec![c.clone()] }, false);
        let mut buffered = [&file[..], &tail].concat();
        buffered[p as usize + 34] ^= 1;
        let r = scan_bytes(&buffered, &params).unwrap();
        assert_eq!((r.frames, r.end, r.valid_len, r.next_seq), (1, End::Torn, p, 2));
        // The same with the last frame synced past P is WAL-R5.
        let tail = encode_frame(3, &Frame { first_seq: 3, synced_len: q, commits: vec![c] }, false);
        let mut synced = [&file[..], &tail].concat();
        synced[p as usize + 34] ^= 1;
        assert_eq!(scan_bytes(&synced, &params).unwrap_err().rule, "WAL-R5");
    }

    /// A Sync-mode WAL of `values.len()` single-commit frames (each frame's
    /// `synced_len` is its own offset from the second one on).
    fn sync_wal(db: [u8; 16], values: &[usize]) -> (Vec<u8>, Vec<(u64, Frame)>) {
        let mut file = Header::wal(db, 3, 1).encode().to_vec();
        let mut frames = Vec::new();
        for (i, &size) in values.iter().enumerate() {
            let key = format!("k{i}");
            let value = vec![b'a' + (i % 26) as u8; size];
            let commit = canonicalize([CommitOp::Put { key: key.as_bytes(), value: &value }]).to_owned_commit();
            let at = file.len() as u64;
            let f = Frame { first_seq: 1 + i as u64, synced_len: if i == 0 { 0 } else { at }, commits: vec![commit] };
            file.extend(encode_frame(3, &f, false));
            frames.push((at, f));
        }
        (file, frames)
    }

    /// WAL-R2: the scanner reads in chunks of at most 1 MiB (a larger frame
    /// whole) and never past the end, across chunk boundaries, frames larger
    /// than a chunk and zero padding spanning several chunks.
    #[test]
    fn scanner_streams_files_larger_than_a_read_chunk() {
        use crate::format::READ_CHUNK;
        use crate::format::testing::Recording;
        let db = [7; 16];
        let params = WalScanParams { file_number: 3, db_id: db, expected_seq: 1 };
        // Frames of 300 KB to 700 KB cross the 1 MiB boundaries; one is 1.5 MiB.
        let sizes = [300_000, 700_000, 450_000, 1_572_864, 650_000, 333_333, 10];
        let (file, frames) = sync_wal(db, &sizes);
        assert!(file.len() > 3 << 20);
        let largest = frames.iter().map(|(_, f)| encode_frame(3, f, false).len()).max().unwrap();
        assert!(largest > READ_CHUNK);
        let max_read = READ_CHUNK.max(largest);
        let scan = |bytes: &[u8]| {
            let mut src = Recording::new(bytes, max_read);
            let mut got = Vec::new();
            let r = scan_wal(&mut src, &params, |off, f| {
                got.push((off, f));
                Ok(())
            })
            .map_err(ReadError::into_format);
            (r, got, src.reads.len())
        };
        let (r, got, _) = scan(&file);
        let r = r.unwrap();
        assert_eq!((r.frames, r.end, r.valid_len), (sizes.len() as u64, End::Clean, file.len() as u64));
        assert_eq!(got, frames);
        // Zero padding over several chunks.
        let p = file.len();
        let mut padded = file.clone();
        padded.resize(p + (5 << 19), 0);
        let (r, _, reads) = scan(&padded);
        assert_eq!(r.unwrap().end, End::ZeroPadded);
        assert!(reads > 6);
        // A non-zero byte in a later padding chunk is not padding (and no
        // candidate follows it: a torn tail).
        for at in [p + (3 << 19), padded.len() - 1] {
            let mut stray = padded.clone();
            stray[at] = 1;
            let r = scan(&stray).0.unwrap();
            assert_eq!((r.end, r.valid_len), (End::Torn, p as u64));
        }
        // Damage after the first MiB, followed by a frame written after it
        // was durable: WAL-R5 at that frame.
        let (at, _) = frames[4];
        assert!(at > 2 * READ_CHUNK as u64);
        let mut damaged = file.clone();
        damaged[at as usize + 100] ^= 1;
        let e = scan(&damaged).0.unwrap_err();
        assert_eq!((e.rule, e.offset), ("WAL-R5", frames[5].0));
        // Damage in the frame larger than a chunk, followed by later frames.
        let mut damaged = file.clone();
        damaged[frames[3].0 as usize + 1_000_000] ^= 1;
        let e = scan(&damaged).0.unwrap_err();
        assert_eq!((e.rule, e.offset), ("WAL-R5", frames[4].0));
        // The first frame zeroed, 1.5 MB of zeros, then a frame written after
        // it was durable: the zeros span two chunks and are not padding.
        let (small, small_frames) = sync_wal(db, &[100]);
        let a_end = small.len();
        let mut zeros = small.clone();
        zeros[96..].fill(0);
        zeros.resize(a_end + 1_500_000, 0);
        let c = canonicalize([CommitOp::Put { key: b"b", value: b"1" }]).to_owned_commit();
        let b = Frame { first_seq: 2, synced_len: a_end as u64, commits: vec![c] };
        let b_at = zeros.len() as u64;
        zeros.extend(encode_frame(3, &b, false));
        assert!(zeros.len() < 96 + 2 * READ_CHUNK);
        let e = scan(&zeros).0.unwrap_err();
        assert_eq!((e.rule, e.offset), ("WAL-R5", b_at));
        assert_eq!(small_frames.len(), 1);
    }
}
