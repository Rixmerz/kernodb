//! An independent decoder of the on-disk format v1, written only from
//! `docs/file-format/v1.md` (§1–§8, §12 and the rule table of §13.1) with
//! `std` and the `crc32c` crate. It deliberately does **not** import
//! `kerno_storage::format`: `spec_decoder_agrees` checks that both decoders
//! accept and reject exactly the same inputs, with the same rule, class and
//! decoded contents. Where this file and the codec disagree, one of them
//! (or the specification) is wrong.
//!
//! The style is intentionally different from the codec: whole-slice
//! functions, naive algorithms (quadratic MAN-S4, concatenating CRC
//! contexts) and a single cursor type.
//!
//! Decoded contents are rendered as a summary string (keys and values in
//! lowercase hex):
//!
//! * varint: `varint {v} len{n}`
//! * header: `header {MAN|WAL|SST} v{version} hl{header_len} compat{hex}
//!   ro{hex} incompat{hex} db{hex} file{n} t{per-type field}`
//! * commit: `commit[{op},...]`, op = `R{start}-{end}` | `P{key}={value}` |
//!   `D{key}`
//! * frame: `frame first{n} synced{n} {commit};{commit}...` (+ ` len{n}`
//!   when decoded alone)
//! * data block: `data[{key}@{seq}={value} | {key}@{seq}~, ...]`
//! * filter: `filter k{k} nbits{n} {bits}`; range block:
//!   `rdel[{start}-{end}@{seq},...]`; index: `index[{key}@{off}+{len},...]`
//! * properties: `props pe pd rf ss ls db kb vb sk lk lx cr ign[...]`
//! * footer: `footer idx{o}+{l} props{o}+{l} filter{o}+{l} rdel{o}+{l}`
//! * record: `rec{edit_seq} {TYPE fields} ign[...]` (+ ` len{n}` alone)
//! * WAL, SST and MANIFEST files: see [`wal`], [`sst`], [`manifest`].

#![allow(dead_code)]

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Corruption,
    Incompatible,
}

/// A rejection: the rule id and its class.
pub type Fail = (&'static str, Class);
type R<T> = Result<T, Fail>;

fn bad<T>(rule: &'static str) -> R<T> {
    Err((rule, Class::Corruption))
}

fn incompatible<T>(rule: &'static str) -> R<T> {
    Err((rule, Class::Incompatible))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Manifest,
    Wal,
    Sst,
}

/// A `FileMeta` (§8.4), as the decoding context of an SST.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub level: u8,
    pub number: u64,
    pub size: u64,
    pub point_entries: u64,
    pub range_fragments: u64,
    pub smallest_seq: u64,
    pub largest_seq: u64,
    pub largest_exclusive: bool,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
}

/// What the bytes are and what they are checked against.
#[derive(Clone, Debug)]
pub enum Ctx {
    Varint,
    Header { kind: FileType, db_id: Option<[u8; 16]>, file_number: Option<u64> },
    Commit,
    Frame { file_number: u64, offset: u64, slot: bool, expected_seq: u64 },
    DataBlock { file_number: u64, offset: u64 },
    FilterBlock { file_number: u64, offset: u64 },
    RangeDelBlock { file_number: u64, offset: u64 },
    PropsBlock { file_number: u64, offset: u64 },
    IndexBlock { file_number: u64, offset: u64 },
    Footer { file_number: u64 },
    Record { generation: u64, offset: u64, expected_edit_seq: u64 },
    Wal { file_number: u64, db_id: [u8; 16], expected_seq: u64 },
    Sst { file_number: u64, db_id: [u8; 16], meta: Option<Meta>, last_seq: u64 },
    Manifest,
}

// §1.4 format limits and §4 bits.
const KEY_MAX: u64 = 16 * 1024;
const VALUE_MAX: u64 = 64 * 1024 * 1024;
const OPS_MAX: u64 = 1 << 24;
const FRAME_PAYLOAD_MAX: u64 = 1 << 30;
const FRAME_COMMITS_MAX: u64 = 65_536;
const DATA_MAX: u64 = 80 * 1024 * 1024;
const META_MAX: u64 = 256 * 1024 * 1024;
const FILE_MAX: u64 = (1 << 48) - 1;
const SEQ_MAX: u64 = (1 << 63) - 1;
const AEAD: u64 = 1;
const DRAFT: u64 = 1 << 63;
const LEVELS: usize = 7;

/// Decodes `bytes` as `ctx` says, returning the summary of the contents.
pub fn decode(ctx: &Ctx, bytes: &[u8]) -> R<String> {
    match ctx {
        Ctx::Varint => {
            let mut c = Cur::new(bytes, "VAR-1");
            let v = c.var()?;
            Ok(format!("varint {v} len{}", c.p))
        }
        Ctx::Header { kind, db_id, file_number } => Ok(header(bytes, *kind, *db_id, *file_number)?.summary()),
        Ctx::Commit => {
            let mut c = Cur::new(bytes, "WAL-C4");
            let ops = commit(&mut c)?;
            c.end()?;
            Ok(commit_summary(&ops))
        }
        Ctx::Frame { file_number, offset, slot, expected_seq } => {
            let (s, n, _) = frame(bytes, *file_number, *offset, *slot, *expected_seq)?;
            Ok(format!("{s} len{n}"))
        }
        Ctx::DataBlock { file_number, offset } => {
            let p = unwrap_block(bytes, *file_number, *offset, DATA_MAX, "SST-R1")?;
            Ok(format!("data[{}]", entries_summary(&data_payload(p)?)))
        }
        Ctx::FilterBlock { file_number, offset } => {
            let p = unwrap_block(bytes, *file_number, *offset, META_MAX, "SST-O4")?;
            let (k, nbits, bits) = filter_payload(p)?;
            Ok(format!("filter k{k} nbits{nbits} {}", hx(bits)))
        }
        Ctx::RangeDelBlock { file_number, offset } => {
            let p = unwrap_block(bytes, *file_number, *offset, META_MAX, "SST-O4")?;
            Ok(format!("rdel[{}]", frags_summary(&rdel_payload(p)?)))
        }
        Ctx::PropsBlock { file_number, offset } => {
            let p = unwrap_block(bytes, *file_number, *offset, META_MAX, "SST-O4")?;
            Ok(props_payload(p)?.summary())
        }
        Ctx::IndexBlock { file_number, offset } => {
            let p = unwrap_block(bytes, *file_number, *offset, META_MAX, "SST-O4")?;
            Ok(index_summary(&index_payload(p)?))
        }
        Ctx::Footer { file_number } => Ok(footer_summary(&footer(bytes, *file_number)?)),
        Ctx::Record { generation, offset: _, expected_edit_seq } => {
            let Some(n) = record_frame(bytes, *generation) else {
                return bad("MAN-R2");
            };
            let rec = record_body(&bytes[..n], *expected_edit_seq, false)?;
            Ok(format!("{} len{n}", rec.summary()))
        }
        Ctx::Wal { file_number, db_id, expected_seq } => wal(bytes, *file_number, *db_id, *expected_seq),
        Ctx::Sst { file_number, db_id, meta, last_seq } => sst(bytes, *file_number, *db_id, meta.as_ref(), *last_seq),
        Ctx::Manifest => manifest(bytes),
    }
}

// ---------------------------------------------------------------- basics

fn hx(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// §1.2: `crc32c(ctx ‖ data)`, built by concatenation.
fn crc_with(ctx: &[u8], parts: &[&[u8]]) -> u32 {
    let mut all = ctx.to_vec();
    for p in parts {
        all.extend_from_slice(p);
    }
    crc32c::crc32c(&all)
}

/// Cursor over one structure; `rule` is reported for a field cut short or
/// leftover bytes (§13.1 "corto"/"sobrante").
struct Cur<'a> {
    b: &'a [u8],
    p: usize,
    rule: &'static str,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8], rule: &'static str) -> Self {
        Cur { b, p: 0, rule }
    }

    fn left(&self) -> usize {
        self.b.len() - self.p
    }

    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if n > self.left() {
            return bad(self.rule);
        }
        self.p += n;
        Ok(&self.b[self.p - n..self.p])
    }

    fn byte(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }

    /// §1.1 VAR-1, VAR-2.
    fn var(&mut self) -> R<u64> {
        let mut v = 0u64;
        for i in 0..10 {
            if self.p + i >= self.b.len() {
                return bad(self.rule);
            }
            let x = self.b[self.p + i];
            if i == 9 && x > 1 {
                return bad("VAR-1");
            }
            if i > 0 && x == 0 {
                return bad("VAR-2");
            }
            v |= u64::from(x & 0x7f) << (7 * i);
            if x & 0x80 == 0 {
                self.p += i + 1;
                return Ok(v);
            }
        }
        unreachable!()
    }

    /// VAR-3: a length within the limit and the remaining bytes.
    fn length(&mut self, max: u64) -> R<usize> {
        let n = self.var()?;
        if n > max || n > self.left() as u64 {
            return bad("VAR-3");
        }
        Ok(n as usize)
    }

    fn bytes(&mut self, max: u64) -> R<&'a [u8]> {
        let n = self.length(max)?;
        self.take(n)
    }

    fn end(&self) -> R<()> {
        if self.left() != 0 { bad(self.rule) } else { Ok(()) }
    }
}

fn tlv_var(val: &[u8], rule: &'static str) -> R<u64> {
    let mut c = Cur::new(val, rule);
    let v = c.var()?;
    c.end()?;
    Ok(v)
}

fn tlv_u8(val: &[u8], rule: &'static str) -> R<u8> {
    if val.len() != 1 { bad(rule) } else { Ok(val[0]) }
}

// ---------------------------------------------------------------- header (§3)

#[derive(Clone, Copy, Debug)]
pub struct Hdr {
    kind: FileType,
    version: u32,
    hl: u32,
    compat: u64,
    ro: u64,
    incompat: u64,
    db: [u8; 16],
    file: u64,
    t: u64,
}

impl Hdr {
    fn summary(&self) -> String {
        let k = match self.kind {
            FileType::Manifest => "MAN",
            FileType::Wal => "WAL",
            FileType::Sst => "SST",
        };
        format!(
            "header {k} v{} hl{} compat{:x} ro{:x} incompat{:x} db{} file{} t{}",
            self.version,
            self.hl,
            self.compat,
            self.ro,
            self.incompat,
            hx(&self.db),
            self.file,
            self.t
        )
    }
}

fn header(b: &[u8], kind: FileType, db: Option<[u8; 16]>, file: Option<u64>) -> R<Hdr> {
    if b.len() < 96 {
        return bad("HDR-1");
    }
    let want: &[u8] = match kind {
        FileType::Manifest => b"KRNO_MAN",
        FileType::Wal => b"KRNO_WAL",
        FileType::Sst => b"KRNO_SST",
    };
    let magic = &b[0..8];
    if magic != want {
        let m0: [&[u8]; 3] = [b"KRNOMAN1", b"KRNOSST1", b"KRNOLOGC"];
        return if m0.contains(&magic) { incompatible("HDR-2") } else { bad("HDR-2") };
    }
    let hl = le32(b, 12) as usize;
    if !(96..=4096).contains(&hl) || !hl.is_multiple_of(8) || hl > b.len() {
        return bad("HDR-3");
    }
    if crc32c::crc32c(&b[..hl - 4]) != le32(b, hl - 4) {
        return bad("HDR-3");
    }
    let version = le32(b, 8);
    if version == 0 {
        return bad("HDR-4");
    }
    if version > 1 {
        return incompatible("HDR-4");
    }
    if hl != 96 {
        return bad("HDR-5");
    }
    let incompat = le64(b, 32);
    if incompat & DRAFT != 0 || incompat & !AEAD != 0 || incompat & AEAD != 0 {
        return incompatible("HDR-6");
    }
    if le32(b, 88) != 0 {
        return bad("HDR-9");
    }
    let unused_from = if kind == FileType::Sst { 68 } else { 72 };
    if b[unused_from..88].iter().any(|&x| x != 0) {
        return bad("HDR-9");
    }
    let t = if kind == FileType::Sst {
        let bs = le32(b, 64);
        if !(512..=1 << 20).contains(&bs) {
            return bad("HDR-9");
        }
        u64::from(bs)
    } else {
        le64(b, 64)
    };
    let mut id = [0u8; 16];
    id.copy_from_slice(&b[40..56]);
    let number = le64(b, 56);
    if db.is_some_and(|d| d != id) || file.is_some_and(|f| f != number) {
        return bad("HDR-10");
    }
    Ok(Hdr { kind, version, hl: hl as u32, compat: le64(b, 16), ro: le64(b, 24), incompat, db: id, file: number, t })
}

// ---------------------------------------------------------------- WAL (§6)

enum Op {
    Range(Vec<u8>, Vec<u8>),
    Put(Vec<u8>, Vec<u8>),
    Del(Vec<u8>),
}

fn commit(c: &mut Cur<'_>) -> R<Vec<Op>> {
    let n = c.var()?;
    if n > OPS_MAX {
        return bad("WAL-C4");
    }
    let mut ops = Vec::new();
    let mut last_range: Option<(Vec<u8>, Vec<u8>)> = None;
    let mut last_key: Option<Vec<u8>> = None;
    for _ in 0..n {
        match c.byte()? {
            3 => {
                if last_key.is_some() {
                    return bad("WAL-C4");
                }
                let s = c.bytes(KEY_MAX)?.to_vec();
                let e = c.bytes(KEY_MAX)?.to_vec();
                if s >= e {
                    return bad("WAL-C4");
                }
                if last_range.as_ref().is_some_and(|prev| *prev >= (s.clone(), e.clone())) {
                    return bad("WAL-C4");
                }
                last_range = Some((s.clone(), e.clone()));
                ops.push(Op::Range(s, e));
            }
            t @ (1 | 2) => {
                let k = c.bytes(KEY_MAX)?.to_vec();
                if last_key.as_ref().is_some_and(|p| *p >= k) {
                    return bad("WAL-C4");
                }
                last_key = Some(k.clone());
                if t == 1 {
                    let v = c.bytes(VALUE_MAX)?.to_vec();
                    ops.push(Op::Put(k, v));
                } else {
                    ops.push(Op::Del(k));
                }
            }
            _ => return bad("WAL-C4"),
        }
    }
    Ok(ops)
}

fn commit_summary(ops: &[Op]) -> String {
    let parts: Vec<String> = ops
        .iter()
        .map(|op| match op {
            Op::Range(s, e) => format!("R{}-{}", hx(s), hx(e)),
            Op::Put(k, v) => format!("P{}={}", hx(k), hx(v)),
            Op::Del(k) => format!("D{}", hx(k)),
        })
        .collect();
    format!("commit[{}]", parts.join(","))
}

/// A frame at the start of `b` (§6.2): its summary, length and commit count.
fn frame(b: &[u8], file: u64, off: u64, slot: bool, expected: u64) -> R<(String, usize, u64)> {
    let len = frame_len_if_valid(b, file, slot).ok_or(("WAL-R2", Class::Corruption))?;
    let (s, count) = frame_body(&b[..len], off, slot, expected)?;
    Ok((s, len, count))
}

/// The length of the frame at the start of `b` if it is complete, its
/// `payload_len` in range and its CRC valid (otherwise the replay stops).
fn frame_len_if_valid(b: &[u8], file: u64, slot: bool) -> Option<usize> {
    if b.len() < 32 {
        return None;
    }
    let plen = u64::from(le32(b, 0));
    if plen == 0 || plen > FRAME_PAYLOAD_MAX {
        return None;
    }
    let flen = 32 + plen + if slot { 40 } else { 0 };
    if flen > b.len() as u64 {
        return None;
    }
    let f = &b[..flen as usize];
    (crc_with(&file.to_le_bytes(), &[&f[0..4], &f[8..]]) == le32(f, 4)).then_some(flen as usize)
}

fn frame_body(f: &[u8], off: u64, slot: bool, expected: u64) -> R<(String, u64)> {
    let plen = le32(f, 0) as usize;
    let first = le64(f, 8);
    let synced = le64(f, 16);
    let count = u64::from(le32(f, 24));
    if first != expected {
        return bad("WAL-R3");
    }
    if count == 0 || count > FRAME_COMMITS_MAX || u128::from(first) + u128::from(count) - 1 > u128::from(SEQ_MAX) {
        return bad("WAL-R3");
    }
    if synced != 0 && (synced < 96 || synced > off) {
        return bad("WAL-R3");
    }
    if le32(f, 28) != 0 {
        return bad("WAL-R3");
    }
    if slot && f[32 + plen..].iter().any(|&x| x != 0) {
        return bad("AEAD-1");
    }
    let mut c = Cur::new(&f[32..32 + plen], "WAL-C4");
    let mut commits = Vec::new();
    for _ in 0..count {
        commits.push(commit_summary(&commit(&mut c)?));
    }
    c.end()?;
    Ok((format!("frame first{first} synced{synced} {}", commits.join(";")), count))
}

/// §6.4. Summary: `wal {header | broken HDR-n} frames[{off}:{frame} / ...]
/// end {clean|zero|torn}@{P} next{seq}`.
fn wal(b: &[u8], file: u64, db: [u8; 16], expected_seq: u64) -> R<String> {
    let mut expected = expected_seq;
    let (head, mut p) = match header(b, FileType::Wal, Some(db), Some(file)) {
        Ok(h) => {
            if h.t != expected {
                return bad("WAL-R1");
            }
            (h.summary(), 96usize)
        }
        Err((rule, _)) if rule == "HDR-1" || rule == "HDR-3" || (rule == "HDR-2" && !known_magic(&b[0..8])) => {
            // WAL-R6: P = 0, no slot (FRAME_SLOT_ALWAYS is false in v1).
            let end = wal_tail(b, 0, expected, file, true).map_err(|_| ("WAL-R6", Class::Corruption))?;
            return Ok(format!("wal broken {rule} frames[] end {end}@0 next{expected}"));
        }
        Err(e) => return Err(e),
    };
    // AEAD is rejected by HDR-6, so v1 frames never carry a slot.
    let mut frames = Vec::new();
    while let Some(flen) = frame_len_if_valid(&b[p..], file, false) {
        let (s, count) = frame_body(&b[p..p + flen], p as u64, false, expected)?;
        frames.push(format!("{p}:{s}"));
        expected += count;
        p += flen;
    }
    let end = wal_tail(b, p, expected, file, false).map_err(|_| ("WAL-R5", Class::Corruption))?;
    Ok(format!("{} frames[{}] end {end}@{p} next{expected}", head.replacen("header", "wal", 1), frames.join(" / ")))
}

/// §3 and §4: the v1 magics (including the reserved `KRNO_EXP`) and M0's.
fn known_magic(m: &[u8]) -> bool {
    let all: [&[u8]; 7] = [b"KRNO_MAN", b"KRNO_WAL", b"KRNO_SST", b"KRNO_EXP", b"KRNOMAN1", b"KRNOSST1", b"KRNOLOGC"];
    all.contains(&m)
}

/// WAL-R4/R5: `Err` if a candidate frame follows `p`. Under a broken header
/// (WAL-R6) the `synced_len` conditions do not apply.
fn wal_tail(b: &[u8], p: usize, expected: u64, file: u64, broken_header: bool) -> Result<&'static str, ()> {
    if p == b.len() {
        return Ok("clean");
    }
    if b[p..].iter().all(|&x| x == 0) {
        return Ok("zero");
    }
    for q in p + 1..b.len() {
        if b.len() - q < 32 {
            break;
        }
        let plen = u64::from(le32(b, q));
        let flen = 32 + plen;
        let first = le64(b, q + 8);
        let synced = le64(b, q + 16);
        if plen == 0 || plen > FRAME_PAYLOAD_MAX || flen > (b.len() - q) as u64 {
            continue;
        }
        // first_seq ∈ [expected, expected + (q − P)].
        let hi = expected.saturating_add((q - p) as u64);
        if first < expected || first > hi || (!broken_header && (synced <= p as u64 || synced > q as u64)) {
            continue;
        }
        let f = &b[q..q + flen as usize];
        if crc_with(&file.to_le_bytes(), &[&f[0..4], &f[8..]]) == le32(f, 4) {
            return Err(());
        }
    }
    Ok("torn")
}

// ---------------------------------------------------------------- SST (§7)

/// §7.2: `payload ‖ slot[40] ‖ crc`.
fn unwrap_block<'a>(b: &'a [u8], file: u64, off: u64, max: u64, rule: &'static str) -> R<&'a [u8]> {
    if b.len() < 44 {
        return bad(rule);
    }
    let plen = b.len() - 44;
    if plen as u64 > max {
        return bad(rule);
    }
    let mut ctx = file.to_le_bytes().to_vec();
    ctx.extend_from_slice(&off.to_le_bytes());
    if crc_with(&ctx, &[&b[..b.len() - 4]]) != le32(b, b.len() - 4) {
        return bad(rule);
    }
    if b[plen..plen + 40].iter().any(|&x| x != 0) {
        return bad("AEAD-1");
    }
    Ok(&b[..plen])
}

#[derive(Clone)]
struct Ent {
    key: Vec<u8>,
    seq: u64,
    val: Option<Vec<u8>>,
}

fn entries_summary(es: &[Ent]) -> String {
    let parts: Vec<String> = es
        .iter()
        .map(|e| match &e.val {
            Some(v) => format!("{}@{}={}", hx(&e.key), e.seq, hx(v)),
            None => format!("{}@{}~", hx(&e.key), e.seq),
        })
        .collect();
    parts.join(",")
}

/// §7.3, SST-R1.
fn data_payload(p: &[u8]) -> R<Vec<Ent>> {
    if p.len() < 8 {
        return bad("SST-R1");
    }
    let count = le32(p, p.len() - 4) as u64;
    if count == 0 || 4 + 4 * count > p.len() as u64 {
        return bad("SST-R1");
    }
    let area = p.len() - 4 - 4 * count as usize;
    let mut c = Cur::new(&p[..area], "SST-R1");
    let mut out: Vec<Ent> = Vec::new();
    for i in 0..count as usize {
        if le32(p, area + 4 * i) as usize != c.p {
            return bad("SST-R1");
        }
        let kl = c.var()?;
        if kl > KEY_MAX {
            return bad("VAR-3");
        }
        let seq = c.var()?;
        if seq == 0 || seq > SEQ_MAX {
            return bad("SST-R1");
        }
        let vl = match c.byte()? {
            1 => {
                let v = c.var()?;
                if v > VALUE_MAX {
                    return bad("VAR-3");
                }
                Some(v)
            }
            2 => None,
            _ => return bad("SST-R1"),
        };
        if kl + vl.unwrap_or(0) > c.left() as u64 {
            return bad("VAR-3");
        }
        let key = c.take(kl as usize)?.to_vec();
        let val = match vl {
            Some(v) => Some(c.take(v as usize)?.to_vec()),
            None => None,
        };
        if out.last().is_some_and(|prev| prev.key >= key) {
            return bad("SST-R1");
        }
        out.push(Ent { key, seq, val });
    }
    c.end()?;
    Ok(out)
}

type Frag = (Vec<u8>, Vec<u8>, u64);

fn frags_summary(fs: &[Frag]) -> String {
    let parts: Vec<String> = fs.iter().map(|(s, e, q)| format!("{}-{}@{q}", hx(s), hx(e))).collect();
    parts.join(",")
}

/// §7.4.
fn rdel_payload(p: &[u8]) -> R<Vec<Frag>> {
    let mut c = Cur::new(p, "SST-O4");
    let n = c.var()?;
    if n == 0 {
        return bad("SST-F1");
    }
    let mut out: Vec<Frag> = Vec::new();
    for _ in 0..n {
        let s = c.bytes(KEY_MAX)?.to_vec();
        let e = c.bytes(KEY_MAX)?.to_vec();
        let q = c.var()?;
        if s >= e {
            return bad("SST-F1");
        }
        if out.last().is_some_and(|prev| prev.1 > s) {
            return bad("SST-F1");
        }
        out.push((s, e, q));
    }
    c.end()?;
    Ok(out)
}

/// §7.5, SST-O7.
fn filter_payload(p: &[u8]) -> R<(u8, u32, &[u8])> {
    if p.len() < 8 || p[0] != 1 {
        return bad("SST-O7");
    }
    let k = p[1];
    if !(1..=30).contains(&k) || p[2] != 0 || p[3] != 0 {
        return bad("SST-O7");
    }
    let nbits = le32(p, 4);
    if nbits < 64 || !nbits.is_multiple_of(8) || nbits > u32::MAX - 7 || (p.len() - 8) as u64 != u64::from(nbits / 8) {
        return bad("SST-O7");
    }
    Ok((k, nbits, &p[8..]))
}

/// §12.
fn bloom_has(bits: &[u8], k: u8, key: &[u8]) -> bool {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in key {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    let nbits = bits.len() as u64 * 8;
    let mut a = h & 0xffffffff;
    for _ in 0..k {
        let i = a % nbits;
        if bits[(i / 8) as usize] & (1 << (i % 8)) == 0 {
            return false;
        }
        a = a.wrapping_add((h >> 32) | 1);
    }
    true
}

#[derive(Default)]
struct Props {
    n: [u64; 9],
    sk: Vec<u8>,
    lk: Vec<u8>,
    lx: bool,
    cr: u8,
    ign: Vec<(u64, Vec<u8>)>,
}

fn ign_summary(ign: &[(u64, Vec<u8>)]) -> String {
    let parts: Vec<String> = ign.iter().map(|(t, v)| format!("{t}:{}", hx(v))).collect();
    format!("ign[{}]", parts.join(","))
}

impl Props {
    fn summary(&self) -> String {
        let n = &self.n;
        format!(
            "props pe{} pd{} rf{} ss{} ls{} db{} kb{} vb{} sk{} lk{} lx{} cr{} {}",
            n[1],
            n[2],
            n[3],
            n[4],
            n[5],
            n[6],
            n[7],
            n[8],
            hx(&self.sk),
            hx(&self.lk),
            u8::from(self.lx),
            self.cr,
            ign_summary(&self.ign)
        )
    }
}

/// §7.6.
fn props_payload(p: &[u8]) -> R<Props> {
    let mut c = Cur::new(p, "SST-O4");
    let mut out = Props::default();
    let mut prev: Option<u64> = None;
    let mut have = [false; 13];
    while c.left() > 0 {
        let tag = c.var()?;
        let n = c.length(u64::MAX)?;
        let val = c.take(n)?;
        if prev.is_some_and(|t| t >= tag) {
            return bad("SST-O6");
        }
        prev = Some(tag);
        match tag {
            1..=8 => out.n[tag as usize] = tlv_var(val, "SST-O6")?,
            9 | 10 => {
                if val.len() as u64 > KEY_MAX {
                    return bad("VAR-3");
                }
                if tag == 9 {
                    out.sk = val.to_vec();
                } else {
                    out.lk = val.to_vec();
                }
            }
            11 => match tlv_u8(val, "SST-O6")? {
                0 => out.lx = false,
                1 => out.lx = true,
                _ => return bad("SST-O6"),
            },
            12 => {
                let r = tlv_u8(val, "SST-O6")?;
                if !(1..=4).contains(&r) {
                    return bad("SST-O6");
                }
                out.cr = r;
            }
            t if t >= 1024 => out.ign.push((t, val.to_vec())),
            _ => return bad("SST-O6"),
        }
        if tag <= 12 {
            have[tag as usize] = true;
        }
    }
    if !have[1..].iter().all(|&h| h) {
        return bad("SST-O6");
    }
    Ok(out)
}

type IdxEnt = (Vec<u8>, u64, u64);

fn index_summary(idx: &[IdxEnt]) -> String {
    let parts: Vec<String> = idx.iter().map(|(k, o, l)| format!("{}@{o}+{l}", hx(k))).collect();
    format!("index[{}]", parts.join(","))
}

/// §7.7.
fn index_payload(p: &[u8]) -> R<Vec<IdxEnt>> {
    let mut c = Cur::new(p, "SST-O4");
    let n = c.var()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let k = c.bytes(KEY_MAX)?.to_vec();
        let o = c.var()?;
        let l = c.var()?;
        out.push((k, o, l));
    }
    c.end()?;
    Ok(out)
}

/// §7.8: `[index_off, index_len, props_off, props_len, filter_off,
/// filter_len, rdel_off, rdel_len]`.
fn footer(b: &[u8], file: u64) -> R<[u64; 8]> {
    if b.len() != 80 {
        return bad("SST-O2");
    }
    if crc_with(&file.to_le_bytes(), &[&b[..76]]) != le32(b, 76) {
        return bad("SST-O2");
    }
    if &b[64..72] != b"KRNO_SST" || le32(b, 72) != 0 {
        return bad("SST-O2");
    }
    let mut f = [0u64; 8];
    for (i, v) in f.iter_mut().enumerate() {
        *v = le64(b, 8 * i);
    }
    Ok(f)
}

fn footer_summary(f: &[u64; 8]) -> String {
    format!("footer idx{}+{} props{}+{} filter{}+{} rdel{}+{}", f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7])
}

/// §7.9 in the order of §13.1: open, every data block, `verify()`.
/// Summary: `sst {props} blocks{n} entries[...] rdel[...] filter k{k}
/// nbits{n}`.
fn sst(b: &[u8], file: u64, db: [u8; 16], meta: Option<&Meta>, last_seq: u64) -> R<String> {
    header(b, FileType::Sst, Some(db), Some(file))?;
    let len = b.len() as u64;
    if meta.is_some_and(|m| m.size != len) || len < 176 {
        return bad("SST-O1");
    }
    let [io, il, po, pl, fo, fl, ro, rl] = footer(&b[b.len() - 80..], file)?;
    let has_r = ro != 0 || rl != 0;
    if has_r && (ro == 0 || rl == 0 || ro.checked_add(rl) != Some(fo)) {
        return bad("SST-O3");
    }
    if fo.checked_add(fl) != Some(po) || po.checked_add(pl) != Some(io) || io.checked_add(il) != Some(len - 80) {
        return bad("SST-O3");
    }
    let d = if has_r { ro } else { fo };
    if d < 96 {
        return bad("SST-O3");
    }
    let at = |o: u64, l: u64| &b[o as usize..(o + l) as usize];
    let rp = if has_r { Some(unwrap_block(at(ro, rl), file, ro, META_MAX, "SST-O4")?) } else { None };
    let fp = unwrap_block(at(fo, fl), file, fo, META_MAX, "SST-O4")?;
    let pp = unwrap_block(at(po, pl), file, po, META_MAX, "SST-O4")?;
    let ip = unwrap_block(at(io, il), file, io, META_MAX, "SST-O4")?;
    let idx = index_payload(ip)?;
    let pr = props_payload(pp)?;
    // SST-O5.
    let mut next = 96u64;
    for (i, (k, o, l)) in idx.iter().enumerate() {
        if *o != next || *l < 55 || *l > DATA_MAX + 44 || (i > 0 && idx[i - 1].0 >= *k) {
            return bad("SST-O5");
        }
        next = o.checked_add(*l).ok_or(("SST-O5", Class::Corruption))?;
    }
    let (pe, pd, rf, ss, ls) = (pr.n[1], pr.n[2], pr.n[3], pr.n[4], pr.n[5]);
    if next != d || idx.len() as u64 != pr.n[6] || idx.is_empty() != (pe == 0) {
        return bad("SST-O5");
    }
    // SST-O6.
    let keys_ok = if pr.lx { pr.sk < pr.lk } else { pr.sk <= pr.lk };
    if (pe == 0 && rf == 0) || pd > pe || ss == 0 || ss > ls || ls > last_seq || ls > SEQ_MAX || !keys_ok {
        return bad("SST-O6");
    }
    if let Some(m) = meta {
        let same = (m.point_entries, m.range_fragments, m.smallest_seq, m.largest_seq, m.largest_exclusive)
            == (pe, rf, ss, ls, pr.lx)
            && m.smallest == pr.sk
            && m.largest == pr.lk;
        if !same {
            return bad("SST-O6");
        }
    }
    // SST-O7 and SST-F1.
    let (k, nbits, bits) = filter_payload(fp)?;
    let frags = match rp {
        Some(p) => rdel_payload(p)?,
        None => Vec::new(),
    };
    if frags.len() as u64 != rf {
        return bad("SST-O7");
    }
    if frags.iter().any(|f| f.2 < ss || f.2 > ls) {
        return bad("SST-F1");
    }
    if frags.iter().any(|f| f.0 < pr.sk || f.1 > pr.lk) {
        return bad("SST-O7");
    }
    // Data blocks (SST-R1..R3) and verify (SST-V3, SST-V4 per entry).
    let mut all: Vec<Ent> = Vec::new();
    for (i, (last, o, l)) in idx.iter().enumerate() {
        let ents = data_payload(unwrap_block(at(*o, *l), file, *o, DATA_MAX, "SST-R1")?)?;
        if ents.last().unwrap().key != *last {
            return bad("SST-R2");
        }
        let first = &ents[0].key;
        if (i == 0 && *first < pr.sk) || (i > 0 && *first <= idx[i - 1].0) {
            return bad("SST-R2");
        }
        if ents.iter().any(|e| e.seq < ss || e.seq > ls) {
            return bad("SST-R3");
        }
        for e in &ents {
            if !bloom_has(bits, k, &e.key) {
                return bad("SST-V3");
            }
            if frags.iter().any(|f| f.0 <= e.key && e.key < f.1 && f.2 > e.seq) {
                return bad("SST-V4");
            }
        }
        all.extend(ents);
    }
    // SST-V1.
    let deletes = all.iter().filter(|e| e.val.is_none()).count() as u64;
    let kb: u64 = all.iter().map(|e| e.key.len() as u64).sum();
    let vb: u64 = all.iter().map(|e| e.val.as_ref().map_or(0, |v| v.len() as u64)).sum();
    if (all.len() as u64, deletes, kb, vb) != (pe, pd, pr.n[7], pr.n[8]) {
        return bad("SST-V1");
    }
    // SST-V2.
    let mut lows: Vec<&Vec<u8>> = Vec::new();
    if let Some(e) = all.first() {
        lows.push(&e.key);
    }
    if let Some(f) = frags.first() {
        lows.push(&f.0);
    }
    let smallest = lows.into_iter().min().cloned().unwrap_or_default();
    let (largest, exclusive) = match (all.last(), frags.last()) {
        (Some(e), Some(f)) if f.1 > e.key => (f.1.clone(), true),
        (Some(e), _) => (e.key.clone(), false),
        (None, Some(f)) => (f.1.clone(), true),
        (None, None) => (Vec::new(), false),
    };
    if (smallest, largest, exclusive) != (pr.sk.clone(), pr.lk.clone(), pr.lx) {
        return bad("SST-V2");
    }
    Ok(format!(
        "sst {} blocks{} entries[{}] rdel[{}] filter k{k} nbits{nbits}",
        pr.summary(),
        idx.len(),
        entries_summary(&all),
        frags_summary(&frags)
    ))
}

// ---------------------------------------------------------------- MANIFEST (§8)

#[derive(Clone, Debug)]
struct Rec {
    edit_seq: u64,
    kind: u8,
    next: Option<u64>,
    last: Option<u64>,
    log: Option<u64>,
    clean: Option<bool>,
    reason: Option<u8>,
    prev: Option<u64>,
    add: Vec<Meta>,
    del: Vec<(u8, u64)>,
    ign: Vec<(u64, Vec<u8>)>,
}

fn meta_summary(m: &Meta) -> String {
    format!(
        "L{}#{} size{} pe{} rf{} ss{} ls{} lx{} {}..{}",
        m.level,
        m.number,
        m.size,
        m.point_entries,
        m.range_fragments,
        m.smallest_seq,
        m.largest_seq,
        u8::from(m.largest_exclusive),
        hx(&m.smallest),
        hx(&m.largest)
    )
}

impl Rec {
    fn summary(&self) -> String {
        let files = |fs: &[Meta]| fs.iter().map(meta_summary).collect::<Vec<_>>().join(";");
        let opt = |v: Option<u64>| v.map_or("-".to_string(), |v| v.to_string());
        let body = match self.kind {
            1 => format!(
                "SNAPSHOT next{} last{} log{} clean{} prev{} files[{}]",
                opt(self.next),
                opt(self.last),
                opt(self.log),
                u8::from(self.clean == Some(true)),
                opt(self.prev),
                files(&self.add)
            ),
            2 => format!(
                "SESSION_OPEN next{} last{} log{} files[{}]",
                opt(self.next),
                opt(self.last),
                opt(self.log),
                files(&self.add)
            ),
            3 => format!(
                "FLUSH next{} last{} log{} reason{} files[{}]",
                opt(self.next),
                opt(self.last),
                opt(self.log),
                opt(self.reason.map(u64::from)),
                files(&self.add)
            ),
            _ => {
                let del: Vec<String> = self.del.iter().map(|(l, n)| format!("{l}:{n}")).collect();
                format!(
                    "COMPACTION reason{} next{} add[{}] del[{}]",
                    opt(self.reason.map(u64::from)),
                    opt(self.next),
                    files(&self.add),
                    del.join(",")
                )
            }
        };
        format!("rec{} {body} {}", self.edit_seq, ign_summary(&self.ign))
    }
}

/// The length of the record at the start of `b` if it is complete, its
/// `payload_len` in range and its CRC valid.
fn record_frame(b: &[u8], generation: u64) -> Option<usize> {
    if b.len() < 24 {
        return None;
    }
    let plen = u64::from(le32(b, 0));
    if plen == 0 || plen > META_MAX || 64 + plen > b.len() as u64 {
        return None;
    }
    let r = &b[..64 + plen as usize];
    (crc_with(&generation.to_le_bytes(), &[&r[0..4], &r[8..]]) == le32(r, 4)).then_some(r.len())
}

fn record_body(r: &[u8], expected: u64, first: bool) -> R<Rec> {
    // §1.4: edit_seq is 1..=2^63−1.
    if le64(r, 8) != expected || le64(r, 8) == 0 || le64(r, 8) > SEQ_MAX {
        return bad(if first { "MAN-R1" } else { "MAN-R2" });
    }
    let kind = r[16];
    if first && kind != 1 {
        return bad("MAN-R1");
    }
    if !(1..=4).contains(&kind) || r[17] != 0 || r[18..24].iter().any(|&x| x != 0) {
        return bad("MAN-R2");
    }
    let plen = le32(r, 0) as usize;
    if r[24 + plen..].iter().any(|&x| x != 0) {
        return bad("AEAD-1");
    }
    fields(kind, le64(r, 8), &r[24..24 + plen])
}

fn file_meta(val: &[u8]) -> R<Meta> {
    let mut c = Cur::new(val, "MAN-R2");
    let level = c.byte()?;
    if level as usize >= LEVELS {
        return bad("MAN-S5");
    }
    let number = c.var()?;
    let size = c.var()?;
    let point_entries = c.var()?;
    let range_fragments = c.var()?;
    let smallest_seq = c.var()?;
    let largest_seq = c.var()?;
    let flags = c.byte()?;
    if flags > 1 {
        return bad("MAN-R2");
    }
    let smallest = c.bytes(KEY_MAX)?.to_vec();
    let largest = c.bytes(KEY_MAX)?.to_vec();
    c.end()?;
    Ok(Meta {
        level,
        number,
        size,
        point_entries,
        range_fragments,
        smallest_seq,
        largest_seq,
        largest_exclusive: flags == 1,
        smallest,
        largest,
    })
}

/// §8.3 with the order of §13.1.
fn fields(kind: u8, edit_seq: u64, p: &[u8]) -> R<Rec> {
    let mut rec = Rec {
        edit_seq,
        kind,
        next: None,
        last: None,
        log: None,
        clean: None,
        reason: None,
        prev: None,
        add: Vec::new(),
        del: Vec::new(),
        ign: Vec::new(),
    };
    let mut c = Cur::new(p, "MAN-R2");
    let mut prev_tag: Option<u64> = None;
    let mut have: u32 = 0;
    while c.left() > 0 {
        let tag = c.var()?;
        let n = c.length(u64::MAX)?;
        let val = c.take(n)?;
        if prev_tag.is_some_and(|pt| tag < pt || (tag == pt && tag != 10 && tag != 11)) {
            return bad("MAN-R2");
        }
        prev_tag = Some(tag);
        match tag {
            1 => rec.next = Some(tlv_var(val, "MAN-R2")?),
            2 => rec.last = Some(tlv_var(val, "MAN-R2")?),
            3 => rec.log = Some(tlv_var(val, "MAN-R2")?),
            4 => match tlv_u8(val, "MAN-R2")? {
                7 => {}
                2..=16 => return incompatible("MAN-S5"),
                _ => return bad("MAN-S5"),
            },
            5 => {
                let f = tlv_var(val, "MAN-R2")?;
                if f > 1 {
                    return bad("MAN-R2");
                }
                rec.clean = Some(f == 1);
            }
            6 => {
                let r = tlv_u8(val, "MAN-R2")?;
                if !(1..=4).contains(&r) {
                    return bad("MAN-R2");
                }
                rec.reason = Some(r);
            }
            7 => rec.prev = Some(tlv_var(val, "MAN-R2")?),
            10 => rec.add.push(file_meta(val)?),
            11 => {
                let mut d = Cur::new(val, "MAN-R2");
                let level = d.byte()?;
                if level as usize >= LEVELS {
                    return bad("MAN-S5");
                }
                let number = d.var()?;
                d.end()?;
                rec.del.push((level, number));
            }
            t if t >= 1024 => {
                rec.ign.push((t, val.to_vec()));
                continue;
            }
            _ => return bad("MAN-R2"),
        }
        have |= 1 << tag;
    }
    let set = |tags: &[u32]| tags.iter().fold(0u32, |m, t| m | 1 << t);
    let (required, optional) = match kind {
        1 => (set(&[1, 2, 3, 4, 5, 7]), set(&[10])),
        2 => (set(&[1, 2, 3]), set(&[10])),
        3 => (set(&[1, 2, 3]), set(&[6, 10])),
        _ => (set(&[6]), set(&[1, 10, 11])),
    };
    if have & required != required || have & !(required | optional) != 0 {
        return bad("MAN-R2");
    }
    let levels_ok = match kind {
        1 => rec.add.windows(2).all(|w| w[0].level <= w[1].level),
        2 | 3 => rec.add.iter().all(|f| f.level == 0),
        _ => rec.add.iter().all(|f| f.level != 0),
    };
    if !levels_ok {
        return bad("MAN-R2");
    }
    Ok(rec)
}

#[derive(Clone)]
struct State {
    next: u64,
    last: u64,
    log: u64,
    levels: Vec<Vec<Meta>>,
}

fn ends_before(f: &Meta, key: &[u8]) -> bool {
    if f.largest_exclusive { f.largest.as_slice() <= key } else { f.largest.as_slice() < key }
}

/// §8.5 and MAN-A1.
fn apply(st: &mut State, rec: &Rec, first: bool) -> R<()> {
    match rec.kind {
        1 => {
            if !first || (rec.clean == Some(true) && rec.log != rec.next) {
                return bad("MAN-R2");
            }
            let mut levels = vec![Vec::new(); LEVELS];
            for f in &rec.add {
                levels[f.level as usize].push(f.clone());
            }
            *st = State { next: rec.next.unwrap(), last: rec.last.unwrap(), log: rec.log.unwrap(), levels };
        }
        2 | 3 => {
            let (n, l, g) = (rec.next.unwrap(), rec.last.unwrap(), rec.log.unwrap());
            if n < st.next || l < st.last || g < st.log {
                return bad("MAN-A1");
            }
            st.next = n;
            st.last = l;
            st.log = g;
            let mut l0 = rec.add.clone();
            l0.append(&mut st.levels[0]);
            st.levels[0] = l0;
        }
        _ => {
            if let Some(n) = rec.next {
                if n < st.next {
                    return bad("MAN-A1");
                }
                st.next = n;
            }
            for (level, number) in &rec.del {
                let files = &mut st.levels[*level as usize];
                let Some(i) = files.iter().position(|f| f.number == *number) else {
                    return bad("MAN-R2");
                };
                files.remove(i);
            }
            for f in &rec.add {
                let files = &mut st.levels[f.level as usize];
                let mut i = 0;
                while i < files.len() && files[i].smallest <= f.smallest {
                    i += 1;
                }
                files.insert(i, f.clone());
            }
        }
    }
    Ok(())
}

/// MAN-S1..S5, naively.
fn validate(st: &State) -> R<()> {
    if st.next == 0 || st.next > FILE_MAX || st.log == 0 || st.log > st.next || st.last > SEQ_MAX {
        return bad("MAN-S1");
    }
    let all: Vec<&Meta> = st.levels.iter().flatten().collect();
    for (i, f) in all.iter().enumerate() {
        if f.number == 0 || f.number >= st.next || all[..i].iter().any(|g| g.number == f.number) {
            return bad("MAN-S1");
        }
    }
    for f in &all {
        let keys_ok = if f.largest_exclusive { f.smallest < f.largest } else { f.smallest <= f.largest };
        let total = u128::from(f.point_entries) + u128::from(f.range_fragments);
        if total == 0 || !keys_ok || f.smallest_seq == 0 || f.smallest_seq > f.largest_seq || f.largest_seq > st.last {
            return bad("MAN-S2");
        }
    }
    for level in &st.levels[1..] {
        for w in level.windows(2) {
            if !ends_before(&w[0], &w[1].smallest) {
                return bad("MAN-S3");
            }
        }
    }
    let l0 = &st.levels[0];
    for i in 0..l0.len() {
        for j in i + 1..l0.len() {
            let overlap = !ends_before(&l0[i], &l0[j].smallest) && !ends_before(&l0[j], &l0[i].smallest);
            if overlap && l0[i].smallest_seq <= l0[j].largest_seq {
                return bad("MAN-S4");
            }
        }
    }
    if st.levels.iter().any(|l| l.len() > 1 << 20) {
        return bad("MAN-S5");
    }
    Ok(())
}

/// §8.7. Summary: `manifest {header} records[{off}:{rec} / ...] next{n}
/// last{n} log{n} L0[{numbers}] ... L6[...] clean{0|1} end {clean|zero|torn}@{P}
/// nextedit{n}`.
fn manifest(b: &[u8]) -> R<String> {
    let h = header(b, FileType::Manifest, None, None)?;
    let generation = h.file;
    if generation == 0 {
        return bad("MAN-R1");
    }
    let mut p = 96usize;
    let mut expected = h.t;
    let mut st = State { next: 0, last: 0, log: 0, levels: vec![Vec::new(); LEVELS] };
    let mut clean = false;
    let mut recs = Vec::new();
    loop {
        let Some(n) = record_frame(&b[p..], generation) else {
            if recs.is_empty() {
                return bad("MAN-R1");
            }
            break;
        };
        let first = recs.is_empty();
        let rec = record_body(&b[p..p + n], expected, first)?;
        if first && rec.prev.and_then(|g| g.checked_add(1)) != Some(generation) {
            return bad("MAN-R1");
        }
        clean = rec.kind == 1 && rec.clean == Some(true);
        apply(&mut st, &rec, first)?;
        validate(&st)?;
        recs.push(format!("{p}:{}", rec.summary()));
        // edit_seq ≤ 2^63−1 was checked: no overflow.
        expected += 1;
        p += n;
    }
    let end = if p == b.len() {
        "clean"
    } else if b[p..].iter().all(|&x| x == 0) {
        "zero"
    } else {
        let hi = expected.saturating_add(((b.len() - p) / 65) as u64).saturating_add(1);
        for q in p + 1..b.len() {
            if b.len() - q < 24 {
                break;
            }
            let es = le64(b, q + 8);
            if es >= expected && es <= hi && record_frame(&b[q..], generation).is_some() {
                return bad("MAN-R3");
            }
        }
        "torn"
    };
    let mut levels = String::new();
    for (i, l) in st.levels.iter().enumerate() {
        let nums: Vec<String> = l.iter().map(|f| f.number.to_string()).collect();
        let _ = write!(levels, " L{i}[{}]", nums.join(","));
    }
    Ok(format!(
        "manifest {} records[{}] next{} last{} log{}{levels} clean{} end {end}@{p} nextedit{expected}",
        h.summary(),
        recs.join(" / "),
        st.next,
        st.last,
        st.log,
        u8::from(clean)
    ))
}
