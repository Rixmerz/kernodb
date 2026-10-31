//! Level-1 structure vectors of the on-disk format v1 (v1.md §13.1).
//!
//! [`vectors`] builds every vector in memory with the codec
//! (`kerno_storage::format`): a positive vector is the encoding of a value; a
//! negative vector is the encoding of a valid value with exactly one rule
//! broken (CRCs recomputed when the broken rule is not the CRC itself), with
//! the rule id and class a reader must report. The bytes are committed under
//! `tests/vectors/v1/structs/<name>.bin`. Changing a byte of an existing
//! vector follows v1.md §15; to regenerate after such a change:
//!
//! ```text
//! cargo test --release -p kerno-storage --test struct_vectors -- --ignored regenerate_struct_vectors
//! ```
//!
//! `spec_decoder` is an independent decoder written from v1.md alone; both
//! must accept and reject exactly the same inputs.

mod spec_decoder;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use kerno_storage::format::block::{self, Entry};
use kerno_storage::format::coding::{self, put_varint};
use kerno_storage::format::features::{AEAD, frame_has_slot};
use kerno_storage::format::filter::{self, Filter};
use kerno_storage::format::header::{self, FileKind, Header, HeaderExpect};
use kerno_storage::format::limits::{MAX_META_PAYLOAD, MAX_SEQ};
use kerno_storage::format::manifest::{self, Edit, FileMeta, ManifestState, Record};
use kerno_storage::format::props::{self, Props};
use kerno_storage::format::rangedel::{self, Fragment};
use kerno_storage::format::sst::{self, BlockHandle, Footer, IndexEntry, SstBuilder, SstExpect};
use kerno_storage::format::tlv::{put_tlv, put_tlv_u8, put_tlv_varint};
use kerno_storage::format::wal::{self, CanonicalCommit, CommitOp, Frame, WalHeaderState, WalScanParams};
use kerno_storage::format::{End, ErrorClass, FormatError, ReadError};
use kerno_testkit::rng::Rng;
use spec_decoder::{Class, Ctx, FileType, Meta};

const DB: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const OTHER_DB: [u8; 16] = [0xaa; 16];
const C: ErrorClass = ErrorClass::Corruption;
const I: ErrorClass = ErrorClass::Incompatible;

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/v1/structs")
}

// ------------------------------------------------------------------ values

/// A decoded (or to-be-encoded) structure.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Varint(u64),
    Header(Header),
    Commit(CanonicalCommit),
    Frame(Frame),
    Data(Vec<Entry>),
    Filter(Filter),
    RangeDel(Vec<Fragment>),
    Props(Props),
    Index(Vec<IndexEntry>),
    Footer(Footer),
    Record(Record),
    /// `header: None` is a broken header (the whole file is `tail`).
    Wal {
        header: Option<Header>,
        frames: Vec<Frame>,
        tail: Vec<u8>,
        end: End,
    },
    Sst {
        header: Header,
        entries: Vec<Entry>,
        fragments: Vec<Fragment>,
        reason: u8,
    },
    Manifest {
        header: Header,
        records: Vec<Record>,
        tail: Vec<u8>,
        clean: bool,
        end: End,
    },
}

enum Expect {
    Accept(Value),
    Reject(&'static str, ErrorClass),
}

struct Vector {
    name: &'static str,
    ctx: Ctx,
    bytes: Vec<u8>,
    expect: Expect,
}

fn file_type(k: FileKind) -> FileType {
    match k {
        FileKind::Manifest => FileType::Manifest,
        FileKind::Wal => FileType::Wal,
        FileKind::Sst => FileType::Sst,
    }
}

fn file_kind(t: FileType) -> FileKind {
    match t {
        FileType::Manifest => FileKind::Manifest,
        FileType::Wal => FileKind::Wal,
        FileType::Sst => FileKind::Sst,
    }
}

fn to_format_meta(m: &Meta) -> FileMeta {
    FileMeta {
        level: m.level,
        number: m.number,
        size: m.size,
        point_entries: m.point_entries,
        range_fragments: m.range_fragments,
        smallest_seq: m.smallest_seq,
        largest_seq: m.largest_seq,
        largest_exclusive: m.largest_exclusive,
        smallest: m.smallest.clone(),
        largest: m.largest.clone(),
    }
}

fn to_spec_meta(m: &FileMeta) -> Meta {
    Meta {
        level: m.level,
        number: m.number,
        size: m.size,
        point_entries: m.point_entries,
        range_fragments: m.range_fragments,
        smallest_seq: m.smallest_seq,
        largest_seq: m.largest_seq,
        largest_exclusive: m.largest_exclusive,
        smallest: m.smallest.clone(),
        largest: m.largest.clone(),
    }
}

// ------------------------------------------------------------------ encode

fn build_sst(h: &Header, entries: &[Entry], fragments: &[Fragment], reason: u8) -> (Vec<u8>, Props) {
    let mut b = SstBuilder::new(h.db_id, h.file_number, h.per_type as u32);
    let mut out = Vec::new();
    for e in entries {
        b.add(&e.key, e.seq, e.value.as_deref());
        out.extend(b.take_ready());
    }
    let (rest, props, len) = b.finish(fragments, reason);
    out.extend(rest);
    assert_eq!(out.len() as u64, len);
    (out, props)
}

fn encode(ctx: &Ctx, v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    match (ctx, v) {
        (_, Value::Varint(x)) => put_varint(&mut out, *x),
        (_, Value::Header(h)) => out.extend_from_slice(&h.encode()),
        (_, Value::Commit(c)) => c.encode_into(&mut out),
        (Ctx::Frame { file_number, slot, .. }, Value::Frame(f)) => out = wal::encode_frame(*file_number, f, *slot),
        (Ctx::DataBlock { file_number, offset }, Value::Data(es)) => {
            out = block::seal_block(&block::encode_data_block(es), *file_number, *offset)
        }
        (Ctx::FilterBlock { file_number, offset }, Value::Filter(f)) => {
            out = block::seal_block(&f.encode(), *file_number, *offset)
        }
        (Ctx::RangeDelBlock { file_number, offset }, Value::RangeDel(fs)) => {
            out = block::seal_block(&rangedel::encode_rangedel(fs), *file_number, *offset)
        }
        (Ctx::PropsBlock { file_number, offset }, Value::Props(p)) => {
            out = block::seal_block(&p.encode(), *file_number, *offset)
        }
        (Ctx::IndexBlock { file_number, offset }, Value::Index(ix)) => {
            out = block::seal_block(&sst::encode_index(ix), *file_number, *offset)
        }
        (Ctx::Footer { file_number }, Value::Footer(f)) => out.extend_from_slice(&f.encode(*file_number)),
        (Ctx::Record { generation, .. }, Value::Record(r)) => out = r.encode(*generation),
        (Ctx::Wal { file_number, .. }, Value::Wal { header, frames, tail, .. }) => {
            if let Some(h) = header {
                out.extend_from_slice(&h.encode());
                for f in frames {
                    out.extend(wal::encode_frame(*file_number, f, frame_has_slot(h.incompat)));
                }
            }
            out.extend_from_slice(tail);
        }
        (Ctx::Sst { .. }, Value::Sst { header, entries, fragments, reason }) => {
            out = build_sst(header, entries, fragments, *reason).0
        }
        (Ctx::Manifest, Value::Manifest { header, records, tail, .. }) => {
            out = manifest::encode_manifest(header, records);
            out.extend_from_slice(tail);
        }
        _ => panic!("value does not match its context"),
    }
    out
}

// ------------------------------------------------------------------ decode + summary

fn hx(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn s_header(h: &Header) -> String {
    let k = match h.kind {
        FileKind::Manifest => "MAN",
        FileKind::Wal => "WAL",
        FileKind::Sst => "SST",
    };
    format!(
        "header {k} v{} hl{} compat{:x} ro{:x} incompat{:x} db{} file{} t{}",
        h.format_version,
        h.header_len,
        h.compat,
        h.ro_compat,
        h.incompat,
        hx(&h.db_id),
        h.file_number,
        h.per_type
    )
}

fn s_commit(c: &CanonicalCommit) -> String {
    let mut ops: Vec<String> = c.ranges.iter().map(|(s, e)| format!("R{}-{}", hx(s), hx(e))).collect();
    for (k, v) in &c.points {
        ops.push(match v {
            Some(v) => format!("P{}={}", hx(k), hx(v)),
            None => format!("D{}", hx(k)),
        });
    }
    format!("commit[{}]", ops.join(","))
}

fn s_frame(f: &Frame) -> String {
    let commits: Vec<String> = f.commits.iter().map(s_commit).collect();
    format!("frame first{} synced{} {}", f.first_seq, f.synced_len, commits.join(";"))
}

fn s_entries(es: &[Entry]) -> String {
    let parts: Vec<String> = es
        .iter()
        .map(|e| match &e.value {
            Some(v) => format!("{}@{}={}", hx(&e.key), e.seq, hx(v)),
            None => format!("{}@{}~", hx(&e.key), e.seq),
        })
        .collect();
    parts.join(",")
}

fn s_frags(fs: &[Fragment]) -> String {
    let parts: Vec<String> = fs.iter().map(|f| format!("{}-{}@{}", hx(&f.start), hx(&f.end), f.seq)).collect();
    parts.join(",")
}

fn s_ign(ign: &[(u64, Vec<u8>)]) -> String {
    let parts: Vec<String> = ign.iter().map(|(t, v)| format!("{t}:{}", hx(v))).collect();
    format!("ign[{}]", parts.join(","))
}

fn s_props(p: &Props) -> String {
    format!(
        "props pe{} pd{} rf{} ss{} ls{} db{} kb{} vb{} sk{} lk{} lx{} cr{} {}",
        p.point_entries,
        p.point_deletes,
        p.range_fragments,
        p.smallest_seq,
        p.largest_seq,
        p.data_blocks,
        p.raw_key_bytes,
        p.raw_value_bytes,
        hx(&p.smallest_key),
        hx(&p.largest_key),
        u8::from(p.largest_exclusive),
        p.creation_reason,
        s_ign(&p.ignored)
    )
}

fn s_index(ix: &[IndexEntry]) -> String {
    let parts: Vec<String> =
        ix.iter().map(|e| format!("{}@{}+{}", hx(&e.last_key), e.handle.offset, e.handle.len)).collect();
    format!("index[{}]", parts.join(","))
}

fn s_footer(f: &Footer) -> String {
    format!(
        "footer idx{}+{} props{}+{} filter{}+{} rdel{}+{}",
        f.index.offset,
        f.index.len,
        f.props.offset,
        f.props.len,
        f.filter.offset,
        f.filter.len,
        f.rdel.offset,
        f.rdel.len
    )
}

fn s_meta(m: &FileMeta) -> String {
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

fn s_record(r: &Record) -> String {
    let files = |fs: &[FileMeta]| fs.iter().map(s_meta).collect::<Vec<_>>().join(";");
    let body = match &r.edit {
        Edit::Snapshot { next_file, last_seq, log_number, clean, prev_generation, files: fs } => format!(
            "SNAPSHOT next{next_file} last{last_seq} log{log_number} clean{} prev{prev_generation} files[{}]",
            u8::from(*clean),
            files(fs)
        ),
        Edit::SessionOpen { next_file, last_seq, log_number, files: fs } => {
            format!("SESSION_OPEN next{next_file} last{last_seq} log{log_number} files[{}]", files(fs))
        }
        Edit::Flush { next_file, last_seq, log_number, reason, files: fs } => format!(
            "FLUSH next{next_file} last{last_seq} log{log_number} reason{} files[{}]",
            reason.map_or("-".to_string(), |r| r.to_string()),
            files(fs)
        ),
        Edit::Compaction { reason, next_file, add, delete } => {
            let del: Vec<String> = delete.iter().map(|(l, n)| format!("{l}:{n}")).collect();
            format!(
                "COMPACTION reason{reason} next{} add[{}] del[{}]",
                next_file.map_or("-".to_string(), |n| n.to_string()),
                files(add),
                del.join(",")
            )
        }
    };
    format!("rec{} {body} {}", r.edit_seq, s_ign(&r.ignored))
}

fn s_end(e: End) -> &'static str {
    match e {
        End::Clean => "clean",
        End::ZeroPadded => "zero",
        End::Torn => "torn",
    }
}

fn s_state(st: &ManifestState) -> String {
    let mut s = format!("next{} last{} log{}", st.next_file, st.last_seq, st.log_number);
    for (i, l) in st.levels.iter().enumerate() {
        let nums: Vec<String> = l.iter().map(|f| f.number.to_string()).collect();
        s.push_str(&format!(" L{i}[{}]", nums.join(",")));
    }
    s
}

/// Decodes `b` with the codec as `ctx` says: the value and its summary.
fn fmt_decode(ctx: &Ctx, b: &[u8]) -> Result<(Value, String), FormatError> {
    let open_meta = |f: u64, o: u64| block::open_block(b, f, o, MAX_META_PAYLOAD, "SST-O4");
    Ok(match ctx {
        Ctx::Varint => {
            let (v, n) = coding::decode_varint(b)?;
            (Value::Varint(v), format!("varint {v} len{n}"))
        }
        Ctx::Header { kind, db_id, file_number } => {
            let expect = HeaderExpect { db_id: *db_id, file_number: *file_number };
            let h = header::decode_header(b, b.len() as u64, file_kind(*kind), &expect)?;
            (Value::Header(h), s_header(&h))
        }
        Ctx::Commit => {
            let mut r = coding::Reader::new(b, 0, "WAL-C4");
            let c = wal::decode_commit(&mut r)?;
            r.finish()?;
            let s = s_commit(&c);
            (Value::Commit(c), s)
        }
        Ctx::Frame { file_number, offset, slot, expected_seq } => {
            let (f, n) = wal::decode_frame(b, *file_number, *offset, *slot, *expected_seq)?;
            let s = format!("{} len{n}", s_frame(&f));
            (Value::Frame(f), s)
        }
        Ctx::DataBlock { file_number, offset } => {
            let es: Vec<Entry> =
                block::decode_data_block(b, *file_number, *offset)?.iter().map(Entry::to_owned_entry).collect();
            let s = format!("data[{}]", s_entries(&es));
            (Value::Data(es), s)
        }
        Ctx::FilterBlock { file_number, offset } => {
            let f = filter::decode_filter(open_meta(*file_number, *offset)?, *offset)?.to_owned_filter();
            let s = format!("filter k{} nbits{} {}", f.k, f.nbits, hx(&f.bits));
            (Value::Filter(f), s)
        }
        Ctx::RangeDelBlock { file_number, offset } => {
            let fs: Vec<Fragment> = rangedel::decode_rangedel(open_meta(*file_number, *offset)?, *offset)?
                .iter()
                .map(Fragment::to_owned_fragment)
                .collect();
            let s = format!("rdel[{}]", s_frags(&fs));
            (Value::RangeDel(fs), s)
        }
        Ctx::PropsBlock { file_number, offset } => {
            let p = props::decode_props(open_meta(*file_number, *offset)?, *offset)?;
            let s = s_props(&p);
            (Value::Props(p), s)
        }
        Ctx::IndexBlock { file_number, offset } => {
            let ix = sst::decode_index(open_meta(*file_number, *offset)?, *offset)?;
            let s = s_index(&ix);
            (Value::Index(ix), s)
        }
        Ctx::Footer { file_number } => {
            let f = sst::decode_footer(b, *file_number, 0)?;
            (Value::Footer(f), s_footer(&f))
        }
        Ctx::Record { generation, offset, expected_edit_seq } => {
            let (r, n) = manifest::decode_record(b, *generation, *offset, *expected_edit_seq)?;
            let s = format!("{} len{n}", s_record(&r));
            (Value::Record(r), s)
        }
        Ctx::Wal { file_number, db_id, expected_seq } => {
            let params = WalScanParams { file_number: *file_number, db_id: *db_id, expected_seq: *expected_seq };
            let mut frames = Vec::new();
            let scan = wal::scan_wal(&mut &b[..], &params, |off, f| {
                frames.push((off, f));
                Ok(())
            })
            .map_err(ReadError::into_format)?;
            let parts: Vec<String> = frames.iter().map(|(o, f)| format!("{o}:{}", s_frame(f))).collect();
            let end = format!("end {}@{} next{}", s_end(scan.end), scan.valid_len, scan.next_seq);
            match scan.header {
                WalHeaderState::Valid(h) => {
                    let s =
                        format!("{} frames[{}] {end}", s_header(&h).replacen("header", "wal", 1), parts.join(" / "));
                    let frames = frames.into_iter().map(|(_, f)| f).collect();
                    let tail = b[scan.valid_len as usize..].to_vec();
                    (Value::Wal { header: Some(h), frames, tail, end: scan.end }, s)
                }
                WalHeaderState::Broken { rule } => {
                    let s = format!("wal broken {rule} frames[] {end}");
                    (Value::Wal { header: None, frames: Vec::new(), tail: b.to_vec(), end: scan.end }, s)
                }
            }
        }
        Ctx::Sst { file_number, db_id, meta, last_seq } => {
            let fm = meta.as_ref().map(to_format_meta);
            let ex = SstExpect { file_number: *file_number, db_id: *db_id, meta: fm.as_ref(), last_seq: *last_seq };
            let (m, entries) = sst::read_sst(b, &ex)?;
            let s = format!(
                "sst {} blocks{} entries[{}] rdel[{}] filter k{} nbits{}",
                s_props(&m.props),
                m.index.len(),
                s_entries(&entries),
                s_frags(&m.fragments),
                m.filter.k,
                m.filter.nbits
            );
            (Value::Sst { header: m.header, entries, fragments: m.fragments, reason: m.props.creation_reason }, s)
        }
        Ctx::Manifest => {
            let mut records = Vec::new();
            let r = manifest::replay_manifest(&mut &b[..], |off, rec| {
                records.push((off, rec.clone()));
                Ok(())
            })
            .map_err(ReadError::into_format)?;
            let parts: Vec<String> = records.iter().map(|(o, rec)| format!("{o}:{}", s_record(rec))).collect();
            let s = format!(
                "manifest {} records[{}] {} clean{} end {}@{} nextedit{}",
                s_header(&r.header),
                parts.join(" / "),
                s_state(&r.state),
                u8::from(r.clean),
                s_end(r.end),
                r.valid_len,
                r.next_edit_seq
            );
            let records = records.into_iter().map(|(_, rec)| rec).collect();
            let tail = b[r.valid_len as usize..].to_vec();
            (Value::Manifest { header: r.header, records, tail, clean: r.clean, end: r.end }, s)
        }
    })
}

// ------------------------------------------------------------------ builders

fn pos(name: &'static str, ctx: Ctx, value: Value) -> Vector {
    let bytes = encode(&ctx, &value);
    Vector { name, ctx, bytes, expect: Expect::Accept(value) }
}

fn neg(name: &'static str, ctx: Ctx, bytes: Vec<u8>, rule: &'static str, class: ErrorClass) -> Vector {
    Vector { name, ctx, bytes, expect: Expect::Reject(rule, class) }
}

fn hctx(kind: FileKind, db: Option<[u8; 16]>, file: Option<u64>) -> Ctx {
    Ctx::Header { kind: file_type(kind), db_id: db, file_number: file }
}

fn patched_header(h: &Header, f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut b = h.encode().to_vec();
    f(&mut b);
    header::reseal_header(&mut b);
    b
}

fn put<'a>(k: &'a [u8], v: &'a [u8]) -> CommitOp<'a> {
    CommitOp::Put { key: k, value: v }
}

fn del(k: &[u8]) -> CommitOp<'_> {
    CommitOp::Delete { key: k }
}

fn range<'a>(s: &'a [u8], e: &'a [u8]) -> CommitOp<'a> {
    CommitOp::DeleteRange { start: s, end: e }
}

fn commit(ops: &[CommitOp<'_>]) -> CanonicalCommit {
    wal::canonicalize(ops.iter().copied()).to_owned_commit()
}

fn raw_commit(build: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut b = Vec::new();
    build(&mut b);
    b
}

fn op_bytes(out: &mut Vec<u8>, op: u8, parts: &[&[u8]]) {
    out.push(op);
    for p in parts {
        coding::put_bytes(out, p);
    }
}

fn pe(k: &[u8], seq: u64, v: &[u8]) -> Entry {
    Entry { key: k.to_vec(), seq, value: Some(v.to_vec()) }
}

fn de(k: &[u8], seq: u64) -> Entry {
    Entry { key: k.to_vec(), seq, value: None }
}

fn frag(s: &[u8], e: &[u8], seq: u64) -> Fragment {
    Fragment { start: s.to_vec(), end: e.to_vec(), seq }
}

fn fmeta(level: u8, number: u64, s: &[u8], l: &[u8], ss: u64, ls: u64) -> FileMeta {
    FileMeta {
        level,
        number,
        size: 4096 + number,
        point_entries: 10,
        range_fragments: 0,
        smallest_seq: ss,
        largest_seq: ls,
        largest_exclusive: false,
        smallest: s.to_vec(),
        largest: l.to_vec(),
    }
}

fn rec(edit_seq: u64, edit: Edit) -> Record {
    Record { edit_seq, edit, ignored: Vec::new() }
}

fn snapshot(next: u64, last: u64, log: u64, clean: bool, prev: u64, files: Vec<FileMeta>) -> Edit {
    Edit::Snapshot { next_file: next, last_seq: last, log_number: log, clean, prev_generation: prev, files }
}

fn flush(next: u64, last: u64, log: u64, files: Vec<FileMeta>) -> Edit {
    Edit::Flush { next_file: next, last_seq: last, log_number: log, reason: None, files }
}

/// Seals a record with a hand-built payload.
fn raw_record(generation: u64, edit_seq: u64, kind: u8, payload: &[u8]) -> Vec<u8> {
    manifest::seal_record(generation, edit_seq, kind, payload)
}

fn tv(out: &mut Vec<u8>, tag: u64, v: u64) {
    put_tlv_varint(out, tag, v);
}

fn meta_value(m: &FileMeta) -> Vec<u8> {
    let mut v = Vec::new();
    m.encode_into(&mut v);
    v
}

/// The standard SNAPSHOT fields (tags 1..7) of a creation record.
fn snapshot_fields(out: &mut Vec<u8>, next: u64, last: u64, log: u64, flags: u64, prev: u64) {
    tv(out, 1, next);
    tv(out, 2, last);
    tv(out, 3, log);
    put_tlv_u8(out, 4, 7);
    tv(out, 5, flags);
    tv(out, 7, prev);
}

/// Assembles an SST from hand-made parts with valid encapsulation, index
/// (unless `index` is given) and footer.
struct SstParts {
    file: u64,
    blocks: Vec<Vec<u8>>,
    last_keys: Vec<Vec<u8>>,
    rdel: Option<Vec<u8>>,
    filter: Vec<u8>,
    props: Vec<u8>,
    index: Option<Vec<u8>>,
}

impl SstParts {
    /// The parts of an SST written by the codec.
    fn of(file: &[u8], file_number: u64) -> SstParts {
        let ex = SstExpect { file_number, db_id: DB, meta: None, last_seq: u64::MAX >> 1 };
        let m = sst::open_sst(file, &ex).unwrap();
        let payload = |h: BlockHandle| file[h.offset as usize..(h.offset + h.len) as usize - 44].to_vec();
        SstParts {
            file: file_number,
            blocks: m.index.iter().map(|e| payload(e.handle)).collect(),
            last_keys: m.index.iter().map(|e| e.last_key.clone()).collect(),
            rdel: (m.footer.rdel != BlockHandle::default()).then(|| payload(m.footer.rdel)),
            filter: payload(m.footer.filter),
            props: payload(m.footer.props),
            index: None,
        }
    }

    fn props(&self) -> Props {
        props::decode_props(&self.props, 0).unwrap()
    }

    fn assemble(&self) -> Vec<u8> {
        let mut out = Header::sst(DB, self.file, 512).encode().to_vec();
        let mut index = Vec::new();
        for (payload, last) in self.blocks.iter().zip(&self.last_keys) {
            let offset = out.len() as u64;
            block::append_block(&mut out, payload, self.file, offset);
            index.push(IndexEntry {
                last_key: last.clone(),
                handle: BlockHandle { offset, len: payload.len() as u64 + 44 },
            });
        }
        let meta = |out: &mut Vec<u8>, p: &[u8]| {
            let offset = out.len() as u64;
            block::append_block(out, p, self.file, offset);
            BlockHandle { offset, len: p.len() as u64 + 44 }
        };
        let mut footer = Footer::default();
        if let Some(r) = &self.rdel {
            footer.rdel = meta(&mut out, r);
        }
        footer.filter = meta(&mut out, &self.filter);
        footer.props = meta(&mut out, &self.props);
        let index = self.index.clone().unwrap_or_else(|| sst::encode_index(&index));
        footer.index = meta(&mut out, &index);
        out.extend_from_slice(&footer.encode(self.file));
        out
    }
}

/// The FileMeta of an SST written by the codec.
fn sst_meta(file: &[u8], number: u64) -> FileMeta {
    let ex = SstExpect { file_number: number, db_id: DB, meta: None, last_seq: u64::MAX >> 1 };
    let m = sst::open_sst(file, &ex).unwrap();
    sst::file_meta(0, number, file.len() as u64, &m.props)
}

fn sst_ctx(number: u64, meta: Option<&FileMeta>, last_seq: u64) -> Ctx {
    Ctx::Sst { file_number: number, db_id: DB, meta: meta.map(to_spec_meta), last_seq }
}

// ------------------------------------------------------------------ the table

#[allow(clippy::vec_init_then_push)]
fn vectors() -> Vec<Vector> {
    let mut v = Vec::new();

    // ---- varints (§1.1)
    v.push(pos("varint_0", Ctx::Varint, Value::Varint(0)));
    v.push(pos("varint_300", Ctx::Varint, Value::Varint(300)));
    v.push(pos("varint_max", Ctx::Varint, Value::Varint(u64::MAX)));
    v.push(neg("neg_var1_eleven_bytes", Ctx::Varint, vec![0xff; 11], "VAR-1", C));
    v.push(neg("neg_var1_tenth_byte", Ctx::Varint, [vec![0xff; 9], vec![0x02]].concat(), "VAR-1", C));
    v.push(neg("neg_var1_truncated", Ctx::Varint, vec![0x80], "VAR-1", C));
    v.push(neg("neg_var2_non_minimal", Ctx::Varint, vec![0x80, 0x00], "VAR-2", C));

    // ---- headers (§3)
    let wal_h = Header::wal(DB, 1, 1);
    let man_h = Header::manifest(DB, 3, 17);
    let sst_h = Header::sst(DB, 42, 4096);
    let wal_ctx = hctx(FileKind::Wal, Some(DB), Some(1));
    let man_ctx = hctx(FileKind::Manifest, None, None);
    let sst_ctx_h = hctx(FileKind::Sst, Some(DB), Some(42));
    v.push(pos("hdr_wal_draft_example", wal_ctx.clone(), Value::Header(wal_h)));
    v.push(pos("hdr_manifest", man_ctx.clone(), Value::Header(man_h)));
    v.push(pos("hdr_sst", sst_ctx_h.clone(), Value::Header(sst_h)));
    v.push(pos("hdr_sst_unknown_ro_compat", sst_ctx_h.clone(), Value::Header(Header { ro_compat: 1 << 5, ..sst_h })));
    v.push(pos(
        "hdr_wal_unknown_compat",
        hctx(FileKind::Wal, Some(DB), Some(7)),
        Value::Header(Header { compat: 1 << 9, ..Header::wal(DB, 7, 100) }),
    ));
    v.push(pos("hdr_wal_without_draft_bit", wal_ctx.clone(), Value::Header(Header { incompat: 0, ..wal_h })));
    v.push(neg("neg_hdr1_short", wal_ctx.clone(), wal_h.encode()[..95].to_vec(), "HDR-1", C));
    v.push(neg(
        "neg_hdr2_unknown_magic",
        wal_ctx.clone(),
        patched_header(&wal_h, |b| b[0..8].copy_from_slice(b"KRNO_XYZ")),
        "HDR-2",
        C,
    ));
    v.push(neg("neg_hdr2_other_type", wal_ctx.clone(), sst_h.encode().to_vec(), "HDR-2", C));
    v.push(neg(
        "neg_hdr2_m0_manifest",
        man_ctx.clone(),
        patched_header(&man_h, |b| b[0..8].copy_from_slice(b"KRNOMAN1")),
        "HDR-2",
        I,
    ));
    v.push(neg("neg_hdr3_header_len_95", wal_ctx.clone(), patched_header(&wal_h, |b| b[12] = 95), "HDR-3", C));
    v.push(neg(
        "neg_hdr3_header_len_beyond_file",
        wal_ctx.clone(),
        {
            let mut b = wal_h.encode().to_vec();
            b[12] = 104;
            b
        },
        "HDR-3",
        C,
    ));
    v.push(neg(
        "neg_hdr3_crc",
        wal_ctx.clone(),
        {
            let mut b = wal_h.encode().to_vec();
            b[45] ^= 0x10;
            b
        },
        "HDR-3",
        C,
    ));
    v.push(neg("neg_hdr4_version_0", wal_ctx.clone(), patched_header(&wal_h, |b| b[8] = 0), "HDR-4", C));
    v.push(neg("neg_hdr4_version_2", wal_ctx.clone(), patched_header(&wal_h, |b| b[8] = 2), "HDR-4", I));
    v.push(neg(
        "neg_hdr5_header_len_104",
        wal_ctx.clone(),
        {
            let mut b = wal_h.encode().to_vec();
            b.resize(104, 0);
            b[12] = 104;
            header::reseal_header(&mut b);
            b
        },
        "HDR-5",
        C,
    ));
    v.push(neg("neg_hdr6_unknown_incompat", wal_ctx.clone(), patched_header(&wal_h, |b| b[32] |= 1 << 5), "HDR-6", I));
    v.push(neg("neg_hdr6_aead", wal_ctx.clone(), patched_header(&wal_h, |b| b[32] |= AEAD as u8), "HDR-6", I));
    v.push(neg("neg_hdr9_reserved", wal_ctx.clone(), patched_header(&wal_h, |b| b[88] = 1), "HDR-9", C));
    v.push(neg("neg_hdr9_per_type_bytes", wal_ctx.clone(), patched_header(&wal_h, |b| b[72] = 1), "HDR-9", C));
    v.push(neg(
        "neg_hdr9_block_size",
        sst_ctx_h.clone(),
        patched_header(&sst_h, |b| b[64..68].copy_from_slice(&100u32.to_le_bytes())),
        "HDR-9",
        C,
    ));
    v.push(neg("neg_hdr10_db_id", hctx(FileKind::Wal, Some(OTHER_DB), Some(1)), wal_h.encode().to_vec(), "HDR-10", C));
    v.push(neg("neg_hdr10_file_number", hctx(FileKind::Wal, Some(DB), Some(2)), wal_h.encode().to_vec(), "HDR-10", C));

    // ---- commits (§6.3)
    v.push(pos(
        "commit_canonical",
        Ctx::Commit,
        Value::Commit(commit(&[
            put(b"b", b"2"),
            range(b"a", b"c"),
            put(b"b", b"3"),
            del(b"d"),
            range(b"x", b"z"),
            put(b"e", b""),
            range(b"q", b"q"),
        ])),
    ));
    v.push(pos("commit_empty", Ctx::Commit, Value::Commit(CanonicalCommit::default())));
    let two = |a: &[u8], b: &[u8]| {
        raw_commit(|o| {
            put_varint(o, 2);
            o.extend_from_slice(a);
            o.extend_from_slice(b);
        })
    };
    let op = |code: u8, parts: &[&[u8]]| {
        let mut o = Vec::new();
        op_bytes(&mut o, code, parts);
        o
    };
    v.push(neg(
        "neg_walc4_points_unordered",
        Ctx::Commit,
        two(&op(1, &[b"b", b"1"]), &op(1, &[b"a", b"1"])),
        "WAL-C4",
        C,
    ));
    v.push(neg("neg_walc4_duplicate_point", Ctx::Commit, two(&op(1, &[b"a", b"1"]), &op(2, &[b"a"])), "WAL-C4", C));
    v.push(neg(
        "neg_walc4_range_after_point",
        Ctx::Commit,
        two(&op(1, &[b"a", b"1"]), &op(3, &[b"b", b"c"])),
        "WAL-C4",
        C,
    ));
    v.push(neg(
        "neg_walc4_ranges_unordered",
        Ctx::Commit,
        two(&op(3, &[b"b", b"c"]), &op(3, &[b"a", b"z"])),
        "WAL-C4",
        C,
    ));
    v.push(neg(
        "neg_walc4_empty_range",
        Ctx::Commit,
        raw_commit(|o| {
            put_varint(o, 1);
            op_bytes(o, 3, &[b"a", b"a"]);
        }),
        "WAL-C4",
        C,
    ));
    v.push(neg(
        "neg_walc4_unknown_op",
        Ctx::Commit,
        raw_commit(|o| {
            put_varint(o, 1);
            op_bytes(o, 4, &[b"a"]);
        }),
        "WAL-C4",
        C,
    ));
    v.push(neg("neg_walc4_op_count", Ctx::Commit, raw_commit(|o| put_varint(o, (1 << 24) + 1)), "WAL-C4", C));
    v.push(neg(
        "neg_walc4_trailing",
        Ctx::Commit,
        raw_commit(|o| {
            put_varint(o, 1);
            op_bytes(o, 2, &[b"a"]);
            o.push(0);
        }),
        "WAL-C4",
        C,
    ));
    v.push(neg(
        "neg_var3_key_beyond_commit",
        Ctx::Commit,
        raw_commit(|o| {
            put_varint(o, 1);
            o.push(1);
            put_varint(o, 10);
            o.extend_from_slice(b"ab");
        }),
        "VAR-3",
        C,
    ));
    v.push(neg(
        "neg_var3_key_above_limit",
        Ctx::Commit,
        raw_commit(|o| {
            put_varint(o, 1);
            o.push(2);
            put_varint(o, 16 * 1024 + 1);
        }),
        "VAR-3",
        C,
    ));
    v.push(neg("neg_var2_op_count", Ctx::Commit, vec![0x81, 0x00], "VAR-2", C));

    // ---- frames (§6.2)
    let fctx =
        |slot: bool, off: u64, expected: u64| Ctx::Frame { file_number: 1, offset: off, slot, expected_seq: expected };
    let one = Frame { first_seq: 1, synced_len: 0, commits: vec![commit(&[put(b"k1", b"v1")])] };
    let one_bytes = wal::encode_frame(1, &one, false);
    v.push(pos("frame_one_commit", fctx(false, 96, 1), Value::Frame(one.clone())));
    v.push(pos(
        "frame_group_3",
        fctx(false, 96, 10),
        Value::Frame(Frame {
            first_seq: 10,
            synced_len: 0,
            commits: vec![
                commit(&[put(b"a", b"1"), del(b"b")]),
                commit(&[range(b"q", b"q")]),
                commit(&[range(b"c", b"f"), put(b"d", b"4")]),
            ],
        }),
    ));
    v.push(pos(
        "frame_synced_len",
        fctx(false, 250, 4),
        Value::Frame(Frame { first_seq: 4, synced_len: 200, commits: vec![commit(&[del(b"gone")])] }),
    ));
    v.push(pos("frame_with_slot", fctx(true, 96, 1), Value::Frame(one.clone())));
    let patch_frame = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = one_bytes.clone();
        f(&mut b);
        wal::reseal_frame(&mut b, 1);
        b
    };
    v.push(neg(
        "neg_walr2_crc",
        fctx(false, 96, 1),
        {
            let mut b = one_bytes.clone();
            b[35] ^= 1;
            b
        },
        "WAL-R2",
        C,
    ));
    v.push(neg("neg_walr2_payload_len_0", fctx(false, 96, 1), patch_frame(&|b| b[0..4].fill(0)), "WAL-R2", C));
    v.push(neg(
        "neg_walr2_payload_len_above_limit",
        fctx(false, 96, 1),
        patch_frame(&|b| b[0..4].copy_from_slice(&((1u32 << 30) + 1).to_le_bytes())),
        "WAL-R2",
        C,
    ));
    v.push(neg("neg_walr2_incomplete", fctx(false, 96, 1), one_bytes[..one_bytes.len() - 1].to_vec(), "WAL-R2", C));
    v.push(neg("neg_walr3_first_seq", fctx(false, 96, 2), one_bytes.clone(), "WAL-R3", C));
    v.push(neg("neg_walr3_count_0", fctx(false, 96, 1), patch_frame(&|b| b[24..28].fill(0)), "WAL-R3", C));
    v.push(neg(
        "neg_walr3_count_above_limit",
        fctx(false, 96, 1),
        patch_frame(&|b| b[24..28].copy_from_slice(&65_537u32.to_le_bytes())),
        "WAL-R3",
        C,
    ));
    v.push(neg(
        "neg_walr3_synced_len_beyond_offset",
        fctx(false, 96, 1),
        patch_frame(&|b| b[16..24].copy_from_slice(&97u64.to_le_bytes())),
        "WAL-R3",
        C,
    ));
    v.push(neg(
        "neg_walr3_synced_len_in_header",
        fctx(false, 96, 1),
        patch_frame(&|b| b[16..24].copy_from_slice(&50u64.to_le_bytes())),
        "WAL-R3",
        C,
    ));
    v.push(neg("neg_walr3_reserved", fctx(false, 96, 1), patch_frame(&|b| b[28] = 1), "WAL-R3", C));
    v.push(neg(
        "neg_aead1_frame_slot",
        fctx(true, 96, 1),
        {
            let mut b = wal::encode_frame(1, &one, true);
            let at = b.len() - 1;
            b[at] = 1;
            wal::reseal_frame(&mut b, 1);
            b
        },
        "AEAD-1",
        C,
    ));
    let payload_of = |cs: &[CanonicalCommit]| {
        let mut p = Vec::new();
        for c in cs {
            c.encode_into(&mut p);
        }
        p
    };
    let c1 = commit(&[put(b"k1", b"v1")]);
    v.push(neg(
        "neg_walc4_frame_extra_commit",
        fctx(false, 96, 1),
        wal::seal_frame(1, 1, 0, 1, &payload_of(&[c1.clone(), c1.clone()]), false),
        "WAL-C4",
        C,
    ));
    v.push(neg(
        "neg_walc4_frame_missing_commit",
        fctx(false, 96, 1),
        wal::seal_frame(1, 1, 0, 2, &payload_of(std::slice::from_ref(&c1)), false),
        "WAL-C4",
        C,
    ));

    // ---- data blocks (§7.3)
    let dctx = Ctx::DataBlock { file_number: 5, offset: 96 };
    let data = vec![pe(b"a", 3, b"x"), de(b"b", 2), pe(b"c", 9, b"a longer value")];
    v.push(pos("data_block", dctx.clone(), Value::Data(data.clone())));
    v.push(pos("data_block_min", dctx.clone(), Value::Data(vec![de(b"", 1)])));
    let data_payload = block::encode_data_block(&data);
    let sealed = |p: &[u8]| block::seal_block(p, 5, 96);
    v.push(neg(
        "neg_sstr1_crc",
        dctx.clone(),
        {
            let mut b = sealed(&data_payload);
            b[2] ^= 1;
            b
        },
        "SST-R1",
        C,
    ));
    v.push(neg("neg_sstr1_short_block", dctx.clone(), sealed(&data_payload)[..43].to_vec(), "SST-R1", C));
    v.push(neg(
        "neg_sstr1_keys_unordered",
        dctx.clone(),
        sealed(&block::encode_data_block(&[de(b"b", 1), de(b"a", 1)])),
        "SST-R1",
        C,
    ));
    v.push(neg(
        "neg_sstr1_duplicate_key",
        dctx.clone(),
        sealed(&block::encode_data_block(&[de(b"a", 2), de(b"a", 1)])),
        "SST-R1",
        C,
    ));
    let mut bad_kind = data_payload.clone();
    bad_kind[2] = 3; // entry 0: key_len 1, seq 3, kind
    v.push(neg("neg_sstr1_kind", dctx.clone(), sealed(&bad_kind), "SST-R1", C));
    let mut seq0 = data_payload.clone();
    seq0[1] = 0;
    v.push(neg("neg_sstr1_seq_zero", dctx.clone(), sealed(&seq0), "SST-R1", C));
    v.push(neg("neg_sstr1_count_zero", dctx.clone(), sealed(&[0u8; 8]), "SST-R1", C));
    let mut bad_offset = data_payload.clone();
    let n = bad_offset.len();
    bad_offset[n - 8] += 1; // offsets[2]
    v.push(neg("neg_sstr1_offset", dctx.clone(), sealed(&bad_offset), "SST-R1", C));
    let one_entry = block::encode_data_block(&[de(b"a", 1)]);
    let mut trailing = one_entry[..one_entry.len() - 8].to_vec();
    trailing.push(0);
    trailing.extend_from_slice(&one_entry[one_entry.len() - 8..]);
    v.push(neg("neg_sstr1_trailing_entry_bytes", dctx.clone(), sealed(&trailing), "SST-R1", C));
    let mut long_value = data_payload.clone();
    long_value[3] = 100; // entry 0 value_len
    v.push(neg("neg_var3_value_beyond_entry", dctx.clone(), sealed(&long_value), "VAR-3", C));
    v.push(neg(
        "neg_aead1_block_slot",
        dctx.clone(),
        {
            let mut b = sealed(&data_payload);
            b[data_payload.len() + 7] = 9;
            block::reseal_block(&mut b, 5, 96);
            b
        },
        "AEAD-1",
        C,
    ));

    // ---- filter, range and properties blocks (§7.4–§7.6)
    let filter_ctx = Ctx::FilterBlock { file_number: 5, offset: 2000 };
    let filter = Filter::build(&[b"apple".as_slice(), b"banana", b"cherry"].map(kerno_storage::format::bloom::hash));
    v.push(pos("filter_block", filter_ctx.clone(), Value::Filter(filter.clone())));
    let fsealed = |p: &[u8]| block::seal_block(p, 5, 2000);
    let filter_payload = filter.encode();
    let mut k31 = filter_payload.clone();
    k31[1] = 31;
    v.push(neg("neg_ssto7_filter_k", filter_ctx.clone(), fsealed(&k31), "SST-O7", C));
    let mut kind2 = filter_payload.clone();
    kind2[0] = 2;
    v.push(neg("neg_ssto7_filter_kind", filter_ctx.clone(), fsealed(&kind2), "SST-O7", C));
    let mut nbits = filter_payload.clone();
    nbits[4] = 68;
    v.push(neg("neg_ssto7_filter_nbits", filter_ctx.clone(), fsealed(&nbits), "SST-O7", C));
    let mut longer = filter_payload.clone();
    longer.push(0);
    v.push(neg("neg_ssto7_filter_length", filter_ctx.clone(), fsealed(&longer), "SST-O7", C));
    v.push(neg(
        "neg_ssto4_meta_crc",
        filter_ctx.clone(),
        {
            let mut b = fsealed(&filter_payload);
            b[9] ^= 0x40;
            b
        },
        "SST-O4",
        C,
    ));

    let rctx = Ctx::RangeDelBlock { file_number: 5, offset: 1500 };
    let rsealed = |p: &[u8]| block::seal_block(p, 5, 1500);
    v.push(pos("rangedel_block", rctx.clone(), Value::RangeDel(vec![frag(b"a", b"c", 5), frag(b"m", b"p", 7)])));
    v.push(neg(
        "neg_sstf1_overlap",
        rctx.clone(),
        rsealed(&rangedel::encode_rangedel(&[frag(b"a", b"d", 5), frag(b"c", b"f", 6)])),
        "SST-F1",
        C,
    ));
    v.push(neg(
        "neg_sstf1_start_ge_end",
        rctx.clone(),
        rsealed(&rangedel::encode_rangedel(&[frag(b"c", b"a", 5)])),
        "SST-F1",
        C,
    ));
    v.push(neg("neg_sstf1_count_zero", rctx.clone(), rsealed(&[0]), "SST-F1", C));
    let mut rd_trailing = rangedel::encode_rangedel(&[frag(b"a", b"c", 5)]);
    rd_trailing.push(7);
    v.push(neg("neg_ssto4_rdel_trailing", rctx.clone(), rsealed(&rd_trailing), "SST-O4", C));

    let pctx = Ctx::PropsBlock { file_number: 5, offset: 1600 };
    let psealed = |p: &[u8]| block::seal_block(p, 5, 1600);
    let props = Props {
        point_entries: 3,
        point_deletes: 1,
        range_fragments: 2,
        smallest_seq: 2,
        largest_seq: 9,
        data_blocks: 1,
        raw_key_bytes: 3,
        raw_value_bytes: 15,
        smallest_key: b"a".to_vec(),
        largest_key: b"p".to_vec(),
        largest_exclusive: true,
        creation_reason: props::REASON_FLUSH,
        ignored: Vec::new(),
    };
    v.push(pos("props_block", pctx.clone(), Value::Props(props.clone())));
    v.push(pos(
        "props_block_ignored_tag",
        pctx.clone(),
        Value::Props(Props { ignored: vec![(2000, b"future".to_vec())], ..props.clone() }),
    ));
    let props_tlv = |f: &dyn Fn(u64, &mut Vec<u8>) -> bool| {
        // Re-emits the twelve fields of `props`, letting `f` replace any.
        let enc = props.encode();
        let mut out = Vec::new();
        let mut it = kerno_storage::format::tlv::TlvIter::new(&enc, 0, "x");
        while let Some(Ok(t)) = it.next() {
            if !f(t.tag, &mut out) {
                put_tlv(&mut out, t.tag, t.value);
            }
        }
        out
    };
    v.push(neg("neg_ssto6_missing_tag", pctx.clone(), psealed(&props_tlv(&|t, _| t == 12)), "SST-O6", C));
    v.push(neg(
        "neg_ssto6_unknown_tag",
        pctx.clone(),
        psealed(&props_tlv(&|t, o| {
            if t == 12 {
                put_tlv_u8(o, 12, 1);
                put_tlv_u8(o, 13, 0);
                true
            } else {
                false
            }
        })),
        "SST-O6",
        C,
    ));
    v.push(neg(
        "neg_ssto6_tags_unordered",
        pctx.clone(),
        psealed(&props_tlv(&|t, o| match t {
            1 => {
                tv(o, 2, 1);
                true
            }
            2 => {
                tv(o, 1, 3);
                true
            }
            _ => false,
        })),
        "SST-O6",
        C,
    ));
    v.push(neg(
        "neg_ssto6_creation_reason",
        pctx.clone(),
        psealed(&props_tlv(&|t, o| {
            if t == 12 {
                put_tlv_u8(o, 12, 5);
            }
            t == 12
        })),
        "SST-O6",
        C,
    ));
    v.push(neg(
        "neg_ssto6_largest_exclusive",
        pctx.clone(),
        psealed(&props_tlv(&|t, o| {
            if t == 11 {
                put_tlv_u8(o, 11, 2);
            }
            t == 11
        })),
        "SST-O6",
        C,
    ));
    v.push(neg(
        "neg_ssto6_varint_len",
        pctx.clone(),
        psealed(&props_tlv(&|t, o| {
            if t == 1 {
                put_tlv(o, 1, &[3, 0]);
            }
            t == 1
        })),
        "SST-O6",
        C,
    ));

    let ictx = Ctx::IndexBlock { file_number: 5, offset: 1700 };
    let index = vec![
        IndexEntry { last_key: b"c".to_vec(), handle: BlockHandle { offset: 96, len: 100 } },
        IndexEntry { last_key: b"m".to_vec(), handle: BlockHandle { offset: 196, len: 80 } },
    ];
    v.push(pos("index_block", ictx.clone(), Value::Index(index.clone())));
    let mut ix_trailing = sst::encode_index(&index);
    ix_trailing.push(0);
    v.push(neg("neg_ssto4_index_trailing", ictx.clone(), block::seal_block(&ix_trailing, 5, 1700), "SST-O4", C));

    // ---- footer (§7.8)
    let footer_ctx = Ctx::Footer { file_number: 5 };
    let footer = Footer {
        index: BlockHandle { offset: 1700, len: 60 },
        props: BlockHandle { offset: 1600, len: 100 },
        filter: BlockHandle { offset: 1500, len: 100 },
        rdel: BlockHandle { offset: 1400, len: 100 },
    };
    v.push(pos("sst_footer", footer_ctx.clone(), Value::Footer(footer)));
    let patch_footer = |f: &dyn Fn(&mut [u8])| {
        let mut b = footer.encode(5).to_vec();
        f(&mut b);
        sst::reseal_footer(&mut b, 5);
        b
    };
    v.push(neg(
        "neg_ssto2_crc",
        footer_ctx.clone(),
        {
            let mut b = footer.encode(5).to_vec();
            b[3] ^= 1;
            b
        },
        "SST-O2",
        C,
    ));
    v.push(neg("neg_ssto2_magic", footer_ctx.clone(), patch_footer(&|b| b[64] = b'X'), "SST-O2", C));
    v.push(neg("neg_ssto2_reserved", footer_ctx.clone(), patch_footer(&|b| b[73] = 1), "SST-O2", C));

    // ---- MANIFEST records (§8.2–§8.4)
    let rctx = |generation: u64, expected: u64| Ctx::Record { generation, offset: 96, expected_edit_seq: expected };
    let create = rec(1, snapshot(1, 0, 1, true, 0, vec![]));
    v.push(pos("rec_snapshot_create", rctx(1, 1), Value::Record(create.clone())));
    let l0a = FileMeta { range_fragments: 1, largest_exclusive: true, ..fmeta(0, 12, b"c", b"x", 401, 500) };
    let l0b = fmeta(0, 11, b"a", b"m", 300, 400);
    let files = vec![
        l0a.clone(),
        l0b.clone(),
        fmeta(1, 5, b"a", b"f", 1, 200),
        fmeta(1, 6, b"g", b"p", 1, 250),
        fmeta(2, 3, b"0", b"z", 1, 100),
    ];
    v.push(pos(
        "rec_snapshot_files",
        rctx(4, 30),
        Value::Record(rec(30, snapshot(20, 500, 19, false, 3, files.clone()))),
    ));
    v.push(pos(
        "rec_session_open",
        rctx(4, 31),
        Value::Record(rec(
            31,
            Edit::SessionOpen {
                next_file: 21,
                last_seq: 520,
                log_number: 20,
                files: vec![fmeta(0, 19, b"b", b"d", 501, 520)],
            },
        )),
    ));
    v.push(pos(
        "rec_flush",
        rctx(4, 32),
        Value::Record(rec(
            32,
            Edit::Flush {
                next_file: 22,
                last_seq: 600,
                log_number: 21,
                reason: Some(manifest::REASON_CLOSE),
                files: vec![fmeta(0, 20, b"a", b"z", 521, 600)],
            },
        )),
    ));
    v.push(pos("rec_flush_without_files", rctx(4, 33), Value::Record(rec(33, flush(22, 610, 22, vec![])))));
    v.push(pos(
        "rec_compaction",
        rctx(4, 34),
        Value::Record(rec(
            34,
            Edit::Compaction {
                reason: manifest::REASON_BACKGROUND,
                next_file: Some(25),
                add: vec![fmeta(1, 23, b"a", b"k", 1, 500), fmeta(1, 24, b"l", b"z", 1, 500)],
                delete: vec![(0, 12), (0, 11), (1, 5), (1, 6)],
            },
        )),
    ));
    v.push(pos(
        "rec_trivial_move",
        rctx(4, 35),
        Value::Record(rec(
            35,
            Edit::Compaction {
                reason: manifest::REASON_TRIVIAL_MOVE,
                next_file: None,
                add: vec![fmeta(2, 24, b"l", b"z", 1, 500)],
                delete: vec![(1, 24)],
            },
        )),
    ));
    v.push(pos(
        "rec_ignored_tag",
        rctx(4, 36),
        Value::Record(Record { ignored: vec![(4096, b"later".to_vec())], ..rec(36, flush(22, 620, 22, vec![])) }),
    ));
    let create_bytes = create.encode(1);
    v.push(neg(
        "neg_manr2_crc",
        rctx(1, 1),
        {
            let mut b = create_bytes.clone();
            b[26] ^= 1;
            b
        },
        "MAN-R2",
        C,
    ));
    v.push(neg("neg_manr2_edit_seq", rctx(1, 2), create_bytes.clone(), "MAN-R2", C));
    let patch_rec = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = create_bytes.clone();
        f(&mut b);
        manifest::reseal_record(&mut b, 1);
        b
    };
    v.push(neg("neg_manr2_kind_reserved", rctx(1, 1), patch_rec(&|b| b[16] = 5), "MAN-R2", C));
    v.push(neg("neg_manr2_header_flags", rctx(1, 1), patch_rec(&|b| b[17] = 1), "MAN-R2", C));
    v.push(neg("neg_manr2_header_reserved", rctx(1, 1), patch_rec(&|b| b[23] = 1), "MAN-R2", C));
    v.push(neg(
        "neg_aead1_record_slot",
        rctx(1, 1),
        patch_rec(&|b| {
            let at = b.len() - 3;
            b[at] = 1;
        }),
        "AEAD-1",
        C,
    ));
    let snap_payload = |extra: &dyn Fn(&mut Vec<u8>)| {
        let mut p = Vec::new();
        snapshot_fields(&mut p, 1, 0, 1, 1, 0);
        extra(&mut p);
        p
    };
    v.push(neg(
        "neg_manr2_unknown_tag",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            tv(&mut p, 1, 1);
            tv(&mut p, 2, 0);
            tv(&mut p, 3, 1);
            put_tlv_u8(&mut p, 4, 7);
            tv(&mut p, 5, 1);
            tv(&mut p, 7, 0);
            tv(&mut p, 8, 0);
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_tags_decreasing",
        rctx(1, 1),
        raw_record(1, 1, 1, &snap_payload(&|p| tv(p, 1, 1))),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_repeated_tag",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            tv(&mut p, 1, 1);
            tv(&mut p, 1, 1);
            tv(&mut p, 2, 0);
            tv(&mut p, 3, 1);
            put_tlv_u8(&mut p, 4, 7);
            tv(&mut p, 5, 1);
            tv(&mut p, 7, 0);
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_missing_field",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            tv(&mut p, 1, 1);
            tv(&mut p, 2, 0);
            tv(&mut p, 3, 1);
            put_tlv_u8(&mut p, 4, 7);
            tv(&mut p, 5, 1);
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_field_not_allowed",
        rctx(1, 1),
        raw_record(1, 1, manifest::KIND_FLUSH, &{
            let mut p = Vec::new();
            tv(&mut p, 1, 1);
            tv(&mut p, 2, 0);
            tv(&mut p, 3, 1);
            put_tlv_u8(&mut p, 4, 7);
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_clean_flag_bits",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            snapshot_fields(&mut p, 1, 0, 1, 2, 0);
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_reason",
        rctx(1, 1),
        raw_record(1, 1, manifest::KIND_COMPACTION, &{
            let mut p = Vec::new();
            put_tlv_u8(&mut p, 6, 9);
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_compaction_level0",
        rctx(1, 1),
        raw_record(1, 1, manifest::KIND_COMPACTION, &{
            let mut p = Vec::new();
            put_tlv_u8(&mut p, 6, 1);
            put_tlv(&mut p, 10, &meta_value(&fmeta(0, 3, b"a", b"b", 1, 1)));
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_filemeta_flags",
        rctx(1, 1),
        raw_record(
            1,
            1,
            1,
            &snap_payload(&|p| {
                let mut m = meta_value(&fmeta(1, 3, b"a", b"b", 1, 1));
                let flags_at = m.len() - 5; // flags, then bytes("a"), bytes("b")
                m[flags_at] = 2;
                put_tlv(p, 10, &m);
            }),
        ),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_snapshot_levels_unordered",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            snapshot_fields(&mut p, 9, 9, 9, 0, 0);
            put_tlv(&mut p, 10, &meta_value(&fmeta(2, 3, b"a", b"b", 1, 1)));
            put_tlv(&mut p, 10, &meta_value(&fmeta(1, 4, b"a", b"b", 1, 1)));
            p
        }),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_mans5_num_levels_8",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            tv(&mut p, 1, 1);
            tv(&mut p, 2, 0);
            tv(&mut p, 3, 1);
            put_tlv_u8(&mut p, 4, 8);
            tv(&mut p, 5, 1);
            tv(&mut p, 7, 0);
            p
        }),
        "MAN-S5",
        I,
    ));
    v.push(neg(
        "neg_mans5_num_levels_1",
        rctx(1, 1),
        raw_record(1, 1, 1, &{
            let mut p = Vec::new();
            tv(&mut p, 1, 1);
            tv(&mut p, 2, 0);
            tv(&mut p, 3, 1);
            put_tlv_u8(&mut p, 4, 1);
            tv(&mut p, 5, 1);
            tv(&mut p, 7, 0);
            p
        }),
        "MAN-S5",
        C,
    ));
    v.push(neg(
        "neg_mans5_level_7",
        rctx(1, 1),
        raw_record(1, 1, 1, &snap_payload(&|p| put_tlv(p, 10, &meta_value(&fmeta(7, 3, b"a", b"b", 1, 1))))),
        "MAN-S5",
        C,
    ));

    // ---- SST files (§7)
    let points: Vec<Entry> = (0..40u32)
        .map(|i| {
            let key = format!("key-{i:03}").into_bytes();
            let value = format!("value-{i:03}-{}", "x".repeat((i % 7) as usize * 9)).into_bytes();
            Entry { key, seq: 100 + u64::from(i * 3 % 37), value: (i % 5 != 3).then_some(value) }
        })
        .collect();
    let sst_hdr = Header::sst(DB, 9, 512);
    let mixed_points = vec![pe(b"b", 30, b"bee"), de(b"d", 31), pe(b"g", 40, b"gee"), pe(b"h", 26, b"aitch")];
    let mixed_frags = vec![frag(b"a", b"c", 20), frag(b"e", b"f", 35), frag(b"g", b"k", 25)];
    let ranges_only = vec![frag(b"a", b"f", 50), frag(b"k", b"n", 60)];
    let cases: [(&'static str, Vec<Entry>, Vec<Fragment>, u8); 3] = [
        ("sst_points", points.clone(), vec![], props::REASON_FLUSH),
        ("sst_ranges_only", vec![], ranges_only.clone(), props::REASON_COMPACTION),
        ("sst_mixed", mixed_points.clone(), mixed_frags.clone(), props::REASON_RECOVERY),
    ];
    for (name, entries, fragments, reason) in cases {
        let (bytes, _) = build_sst(&sst_hdr, &entries, &fragments, reason);
        let meta = sst_meta(&bytes, 9);
        v.push(pos(name, sst_ctx(9, Some(&meta), 1000), Value::Sst { header: sst_hdr, entries, fragments, reason }));
    }
    let (mixed, _) = build_sst(&sst_hdr, &mixed_points, &mixed_frags, props::REASON_RECOVERY);
    let mixed_meta = sst_meta(&mixed, 9);
    let (pts, _) = build_sst(&sst_hdr, &points, &[], props::REASON_FLUSH);
    let parts = SstParts::of(&mixed, 9);
    assert_eq!(parts.assemble(), mixed, "SstParts must reproduce the codec's output");
    let with_props = |f: &dyn Fn(&mut Props)| {
        let mut p = parts.props();
        f(&mut p);
        SstParts { props: p.encode(), ..SstParts::of(&mixed, 9) }.assemble()
    };
    let none = |last_seq| sst_ctx(9, None, last_seq);
    v.push(neg(
        "neg_ssto1_size",
        sst_ctx(9, Some(&FileMeta { size: mixed_meta.size + 1, ..mixed_meta.clone() }), 1000),
        mixed.clone(),
        "SST-O1",
        C,
    ));
    v.push(neg("neg_ssto1_short", none(1000), [sst_hdr.encode().to_vec(), vec![0; 60]].concat(), "SST-O1", C));
    v.push(neg("neg_hdr10_sst_renamed", sst_ctx(10, None, 1000), mixed.clone(), "HDR-10", C));
    let patch_sst_footer = |file: &[u8], f: &dyn Fn(&mut [u8])| {
        let mut b = file.to_vec();
        let at = b.len() - 80;
        f(&mut b[at..]);
        sst::reseal_footer(&mut b[at..], 9);
        b
    };
    v.push(neg(
        "neg_ssto3_layout",
        none(1000),
        patch_sst_footer(&mixed, &|f| {
            let l = u64::from_le_bytes(f[8..16].try_into().unwrap());
            f[8..16].copy_from_slice(&(l - 1).to_le_bytes());
        }),
        "SST-O3",
        C,
    ));
    v.push(neg(
        "neg_ssto3_half_range_handle",
        none(1000),
        patch_sst_footer(&pts, &|f| f[48..56].copy_from_slice(&96u64.to_le_bytes())),
        "SST-O3",
        C,
    ));
    v.push(neg(
        "neg_ssto4_props_crc",
        none(1000),
        {
            let m =
                sst::open_sst(&mixed, &SstExpect { file_number: 9, db_id: DB, meta: None, last_seq: 1000 }).unwrap();
            let mut b = mixed.clone();
            b[m.footer.props.offset as usize + 1] ^= 4;
            b
        },
        "SST-O4",
        C,
    ));
    v.push(neg(
        "neg_aead1_sst_meta_slot",
        none(1000),
        {
            let m =
                sst::open_sst(&mixed, &SstExpect { file_number: 9, db_id: DB, meta: None, last_seq: 1000 }).unwrap();
            let mut b = mixed.clone();
            let h = m.footer.index;
            let blk = &mut b[h.offset as usize..(h.offset + h.len) as usize];
            let at = blk.len() - 10;
            blk[at] = 0x55;
            block::reseal_block(blk, 9, h.offset);
            b
        },
        "AEAD-1",
        C,
    ));
    v.push(neg(
        "neg_ssto5_index_gap",
        none(1000),
        {
            let mut p = SstParts::of(&mixed, 9);
            let mut b = p.assemble();
            // Index entry 0 claims to start one byte later.
            let mut ix =
                sst::open_sst(&b, &SstExpect { file_number: 9, db_id: DB, meta: None, last_seq: 1000 }).unwrap().index;
            ix[0].handle.offset += 1;
            ix[0].handle.len -= 1;
            p.index = Some(sst::encode_index(&ix));
            b = p.assemble();
            b
        },
        "SST-O5",
        C,
    ));
    v.push(neg("neg_ssto5_data_blocks", none(1000), with_props(&|p| p.data_blocks += 1), "SST-O5", C));
    v.push(neg(
        "neg_ssto6_meta_mismatch",
        sst_ctx(9, Some(&FileMeta { smallest_seq: mixed_meta.smallest_seq + 1, ..mixed_meta.clone() }), 1000),
        mixed.clone(),
        "SST-O6",
        C,
    ));
    v.push(neg("neg_ssto6_above_last_seq", none(mixed_meta.largest_seq - 1), mixed.clone(), "SST-O6", C));
    v.push(neg(
        "neg_ssto6_point_deletes",
        none(1000),
        with_props(&|p| p.point_deletes = p.point_entries + 1),
        "SST-O6",
        C,
    ));
    v.push(neg("neg_ssto7_fragment_count", none(1000), with_props(&|p| p.range_fragments -= 1), "SST-O7", C));
    v.push(neg(
        "neg_ssto7_fragment_outside_keys",
        none(1000),
        with_props(&|p| p.largest_key = b"j".to_vec()),
        "SST-O7",
        C,
    ));
    v.push(neg(
        "neg_sstf1_fragment_seq",
        none(1000),
        SstParts {
            rdel: Some(rangedel::encode_rangedel(&[frag(b"a", b"c", 20), frag(b"e", b"f", 35), frag(b"g", b"k", 41)])),
            ..SstParts::of(&mixed, 9)
        }
        .assemble(),
        "SST-F1",
        C,
    ));
    let pts_parts = SstParts::of(&pts, 9);
    assert!(pts_parts.blocks.len() >= 3, "sst_points must span several blocks");
    v.push(neg(
        "neg_sstr2_last_key",
        none(1000),
        {
            let mut last_keys = pts_parts.last_keys.clone();
            last_keys[0].push(b'~'); // above block 0's last key, below block 1's first
            SstParts { last_keys, ..SstParts::of(&pts, 9) }.assemble()
        },
        "SST-R2",
        C,
    ));
    v.push(neg(
        "neg_sstr2_first_key",
        none(1000),
        {
            let mut p = SstParts::of(&pts, 9);
            let mut props = p.props();
            props.smallest_key = b"key-000~".to_vec();
            p.props = props.encode();
            p.assemble()
        },
        "SST-R2",
        C,
    ));
    v.push(neg(
        "neg_sstr3_seq",
        none(1000),
        {
            let mut p = SstParts::of(&pts, 9);
            let mut es: Vec<Entry> =
                block::parse_data_block(&p.blocks[1], 0).unwrap().iter().map(Entry::to_owned_entry).collect();
            es[0].seq = p.props().largest_seq + 1;
            p.blocks[1] = block::encode_data_block(&es);
            p.assemble()
        },
        "SST-R3",
        C,
    ));
    v.push(neg("neg_sstv1_counts", none(1000), with_props(&|p| p.raw_value_bytes += 1), "SST-V1", C));
    v.push(neg("neg_sstv2_extremes", none(1000), with_props(&|p| p.smallest_key = b"0".to_vec()), "SST-V2", C));
    v.push(neg(
        "neg_sstv3_filter",
        none(1000),
        {
            let keys: Vec<&[u8]> = vec![b"b", b"d", b"g"]; // "h" left out
            let f = Filter::build(&keys.iter().map(|k| kerno_storage::format::bloom::hash(k)).collect::<Vec<_>>());
            assert!(!f.may_contain(kerno_storage::format::bloom::hash(b"h")));
            SstParts { filter: f.encode(), ..SstParts::of(&mixed, 9) }.assemble()
        },
        "SST-V3",
        C,
    ));
    v.push(neg(
        "neg_sstv4_covered_point",
        none(1000),
        build_sst(&sst_hdr, &[pe(b"c", 5, b"old")], &[frag(b"b", b"d", 9)], props::REASON_FLUSH).0,
        "SST-V4",
        C,
    ));

    // ---- WAL files (§6)
    let wctx = |expected| Ctx::Wal { file_number: 3, db_id: DB, expected_seq: expected };
    let wh = Header::wal(DB, 3, 11);
    let f1 = Frame { first_seq: 11, synced_len: 0, commits: vec![commit(&[put(b"a", b"1")]), commit(&[del(b"b")])] };
    let f1_len = wal::encode_frame(3, &f1, false).len() as u64;
    let f2 = Frame { first_seq: 13, synced_len: 96 + f1_len, commits: vec![commit(&[range(b"c", b"e")])] };
    let wal_value =
        |tail: Vec<u8>, end| Value::Wal { header: Some(wh), frames: vec![f1.clone(), f2.clone()], tail, end };
    v.push(pos("wal_two_frames", wctx(11), wal_value(vec![], End::Clean)));
    v.push(pos("wal_zero_padded", wctx(11), wal_value(vec![0; 64], End::ZeroPadded)));
    v.push(pos("wal_empty", wctx(11), Value::Wal { header: Some(wh), frames: vec![], tail: vec![], end: End::Clean }));
    let f2_bytes = wal::encode_frame(3, &f2, false);
    v.push(pos(
        "wal_torn_tail",
        wctx(11),
        Value::Wal {
            header: Some(wh),
            frames: vec![f1.clone()],
            tail: f2_bytes[..f2_bytes.len() - 5].to_vec(),
            end: End::Torn,
        },
    ));
    let two_frames = encode(&wctx(11), &wal_value(vec![], End::Clean));
    // M1: the writer syncs the header before the first frame, so a valid frame
    // under a broken header is damage even when its `synced_len` is 0.
    v.push(neg(
        "neg_walr6_broken_header_unsynced",
        wctx(11),
        {
            let mut b = two_frames[..(96 + f1_len) as usize].to_vec();
            b[..96].fill(0);
            b
        },
        "WAL-R6",
        C,
    ));
    // A header the crash broke before it was durable, with no frame behind it.
    v.push(pos(
        "wal_broken_header_no_frames",
        wctx(11),
        Value::Wal { header: None, frames: vec![], tail: vec![0; 96], end: End::ZeroPadded },
    ));
    v.push(neg("neg_walr1_base_seq", wctx(12), two_frames.clone(), "WAL-R1", C));
    v.push(neg(
        "neg_walr5_mid_damage",
        wctx(11),
        {
            let mut b = two_frames.clone();
            b[96 + 40] ^= 1;
            b
        },
        "WAL-R5",
        C,
    ));
    v.push(neg(
        "neg_walr6_broken_header_synced",
        wctx(11),
        {
            let mut b = two_frames.clone();
            b[50] ^= 1;
            b
        },
        "WAL-R6",
        C,
    ));
    // WAL-R5 and WAL-R6 with group frames: the candidate bound on first_seq
    // grows with the bytes after P (one commit can take a single byte), not
    // with one commit per 33 bytes.
    let group = |commits: Vec<CanonicalCommit>| {
        let n = commits.len() as u64;
        let g1 = wal::encode_frame(3, &Frame { first_seq: 11, synced_len: 0, commits }, false);
        let g2 = Frame { first_seq: 11 + n, synced_len: 96 + g1.len() as u64, commits: vec![commit(&[del(b"z")])] };
        [&wh.encode()[..], &g1, &wal::encode_frame(3, &g2, false)].concat()
    };
    let keys: Vec<[u8; 1]> = (0..20u8).map(|i| [b'a' + i]).collect();
    let small_group = group(keys.iter().map(|k| commit(&[put(k, b"v")])).collect());
    v.push(neg(
        "neg_walr5_group_small_commits",
        wctx(11),
        {
            let mut b = small_group.clone();
            b[96 + 40] ^= 1;
            b
        },
        "WAL-R5",
        C,
    ));
    v.push(neg(
        "neg_walr5_group_empty_commits",
        wctx(11),
        {
            // WAL-C3: 40 empty commits, one payload byte each.
            let mut b = group(vec![CanonicalCommit::default(); 40]);
            b[96 + 40] ^= 1;
            b
        },
        "WAL-R5",
        C,
    ));
    v.push(neg(
        "neg_walr6_group_broken_header",
        wctx(11),
        {
            let mut b = small_group.clone();
            b[50] ^= 1;
            b
        },
        "WAL-R6",
        C,
    ));
    // WAL-R5's accept side: damage followed by a frame whose synced_len does
    // not go past P (Buffered: the damaged bytes were never synced) is a torn
    // tail; the same frame synced past P makes it corruption.
    let f3 = |synced_len| Frame { first_seq: 14, synced_len, commits: vec![commit(&[put(b"f", b"3")])] };
    let buffered_tail = |synced_len| {
        let mut b = [f2_bytes.clone(), wal::encode_frame(3, &f3(synced_len), false)].concat();
        b[34] ^= 1;
        b
    };
    v.push(pos(
        "wal_torn_buffered",
        wctx(11),
        Value::Wal { header: Some(wh), frames: vec![f1.clone()], tail: buffered_tail(96 + f1_len), end: End::Torn },
    ));
    v.push(neg(
        "neg_walr5_synced_after_damage",
        wctx(11),
        [&wh.encode()[..], &wal::encode_frame(3, &f1, false), &buffered_tail(96 + f1_len + f2_bytes.len() as u64)]
            .concat(),
        "WAL-R5",
        C,
    ));
    v.push(neg(
        "neg_walr3_sequence_gap",
        wctx(11),
        {
            let mut b = two_frames.clone();
            let at = (96 + f1_len) as usize;
            b[at + 8] += 1;
            wal::reseal_frame(&mut b[at..], 3);
            b
        },
        "WAL-R3",
        C,
    ));
    v.push(neg(
        "neg_walc4_in_file",
        wctx(11),
        {
            let mut b = wh.encode().to_vec();
            b.extend(wal::seal_frame(3, 11, 0, 1, &two(&op(2, &[b"z"]), &op(2, &[b"y"])), false));
            b
        },
        "WAL-C4",
        C,
    ));
    v.push(neg(
        "neg_hdr6_wal_aead",
        wctx(11),
        [patched_header(&wh, |b| b[32] |= AEAD as u8), wal::encode_frame(3, &f1, true)].concat(),
        "HDR-6",
        I,
    ));
    // A valid header of another type is not a torn header (WAL-R6): HDR-2.
    v.push(neg(
        "neg_hdr2_wal_foreign_magic",
        wctx(11),
        [patched_header(&wh, |b| b[0..8].copy_from_slice(b"KRNO_SST")), wal::encode_frame(3, &f1, false)].concat(),
        "HDR-2",
        C,
    ));

    // ---- MANIFEST files (§8)
    let mh1 = Header::manifest(DB, 1, 1);
    let man_value = |h: Header, records: Vec<Record>, tail: Vec<u8>, clean: bool, end: End| Value::Manifest {
        header: h,
        records,
        tail,
        clean,
        end,
    };
    v.push(pos("man_created", Ctx::Manifest, man_value(mh1, vec![create.clone()], vec![], true, End::Clean)));
    v.push(pos(
        "man_zero_padded",
        Ctx::Manifest,
        man_value(mh1, vec![create.clone()], vec![0; 100], true, End::ZeroPadded),
    ));
    let mh2 = Header::manifest(DB, 2, 5);
    let small = vec![
        rec(
            5,
            snapshot(
                10,
                100,
                9,
                false,
                1,
                vec![fmeta(0, 7, b"a", b"m", 50, 100), fmeta(1, 4, b"a", b"k", 1, 40), fmeta(1, 5, b"m", b"z", 1, 45)],
            ),
        ),
        rec(6, Edit::SessionOpen { next_file: 11, last_seq: 100, log_number: 10, files: vec![] }),
        rec(7, flush(12, 150, 11, vec![fmeta(0, 10, b"a", b"z", 101, 150)])),
        rec(
            8,
            Edit::Compaction {
                reason: manifest::REASON_BACKGROUND,
                next_file: Some(14),
                add: vec![fmeta(1, 12, b"a", b"m", 1, 150), fmeta(1, 13, b"n", b"z", 1, 150)],
                delete: vec![(0, 7), (0, 10), (1, 4), (1, 5)],
            },
        ),
    ];
    v.push(pos("man_small", Ctx::Manifest, man_value(mh2, small.clone(), vec![], false, End::Clean)));
    let last = small[3].encode(2);
    v.push(pos(
        "man_torn_tail",
        Ctx::Manifest,
        man_value(mh2, small[..3].to_vec(), last[..last.len() - 7].to_vec(), false, End::Torn),
    ));
    let man = |h: &Header, recs: &[Record]| manifest::encode_manifest(h, recs);
    let small_bytes = man(&mh2, &small);
    v.push(neg("neg_manr1_empty", Ctx::Manifest, mh2.encode().to_vec(), "MAN-R1", C));
    v.push(neg(
        "neg_manr1_generation_0",
        Ctx::Manifest,
        man(&Header::manifest(DB, 0, 1), std::slice::from_ref(&create)),
        "MAN-R1",
        C,
    ));
    v.push(neg(
        "neg_manr1_record0_torn",
        Ctx::Manifest,
        small_bytes[..96 + small[0].encode(2).len() - 1].to_vec(),
        "MAN-R1",
        C,
    ));
    v.push(neg("neg_manr1_not_snapshot", Ctx::Manifest, man(&mh2, &small[1..]), "MAN-R1", C));
    v.push(neg("neg_manr1_edit_seq", Ctx::Manifest, man(&Header::manifest(DB, 2, 4), &small), "MAN-R1", C));
    v.push(neg("neg_manr1_prev_generation", Ctx::Manifest, man(&Header::manifest(DB, 3, 5), &small), "MAN-R1", C));
    v.push(neg(
        "neg_manr2_snapshot_not_first",
        Ctx::Manifest,
        man(&mh1, &[create.clone(), rec(2, snapshot(1, 0, 1, false, 0, vec![]))]),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_clean_log_number",
        Ctx::Manifest,
        man(&mh1, &[rec(1, snapshot(3, 0, 2, true, 0, vec![]))]),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_manr2_delete_missing",
        Ctx::Manifest,
        man(
            &mh2,
            &[
                small[0].clone(),
                rec(
                    6,
                    Edit::Compaction {
                        reason: manifest::REASON_BACKGROUND,
                        next_file: None,
                        add: vec![],
                        delete: vec![(1, 7)],
                    },
                ),
            ],
        ),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_mana1_next_file_decreases",
        Ctx::Manifest,
        man(&mh2, &[small[0].clone(), rec(6, flush(9, 100, 9, vec![]))]),
        "MAN-A1",
        C,
    ));
    v.push(neg(
        "neg_mana1_last_seq_decreases",
        Ctx::Manifest,
        man(&mh2, &[small[0].clone(), rec(6, flush(10, 99, 9, vec![]))]),
        "MAN-A1",
        C,
    ));
    v.push(neg(
        "neg_manr3_mid_damage",
        Ctx::Manifest,
        {
            let mut b = small_bytes.clone();
            let at = 96 + small[0].encode(2).len() + 30;
            b[at] ^= 1;
            b
        },
        "MAN-R3",
        C,
    ));
    let one_snap = |files: Vec<FileMeta>, next: u64, last: u64, log: u64| {
        man(&mh2, &[rec(5, snapshot(next, last, log, false, 1, files))])
    };
    v.push(neg("neg_mans1_log_number", Ctx::Manifest, one_snap(vec![], 5, 0, 6), "MAN-S1", C));
    // Sequences read from the MANIFEST within the format limits of §1.4.
    v.push(pos(
        "man_sequences_at_limit",
        Ctx::Manifest,
        man_value(
            Header::manifest(DB, 2, MAX_SEQ),
            vec![rec(MAX_SEQ, snapshot(9, MAX_SEQ, 9, false, 1, vec![]))],
            vec![],
            false,
            End::Clean,
        ),
    ));
    v.push(neg("neg_mans1_last_seq_above_limit", Ctx::Manifest, one_snap(vec![], 9, MAX_SEQ + 1, 9), "MAN-S1", C));
    v.push(neg(
        "neg_manr1_edit_seq_above_limit",
        Ctx::Manifest,
        man(&Header::manifest(DB, 2, MAX_SEQ + 1), &[rec(MAX_SEQ + 1, snapshot(9, 9, 9, false, 1, vec![]))]),
        "MAN-R1",
        C,
    ));
    v.push(neg(
        "neg_manr2_edit_seq_above_limit",
        Ctx::Manifest,
        man(
            &Header::manifest(DB, 2, MAX_SEQ),
            &[rec(MAX_SEQ, snapshot(9, 9, 9, false, 1, vec![])), rec(MAX_SEQ + 1, flush(9, 9, 9, vec![]))],
        ),
        "MAN-R2",
        C,
    ));
    v.push(neg(
        "neg_mans1_duplicate_number",
        Ctx::Manifest,
        one_snap(vec![fmeta(1, 3, b"a", b"b", 1, 1), fmeta(2, 3, b"a", b"b", 1, 1)], 9, 9, 9),
        "MAN-S1",
        C,
    ));
    v.push(neg(
        "neg_mans1_number_not_below_next_file",
        Ctx::Manifest,
        one_snap(vec![fmeta(1, 9, b"a", b"b", 1, 1)], 9, 9, 9),
        "MAN-S1",
        C,
    ));
    v.push(neg(
        "neg_mans2_smallest_above_largest",
        Ctx::Manifest,
        one_snap(vec![fmeta(1, 3, b"q", b"b", 1, 1)], 9, 9, 9),
        "MAN-S2",
        C,
    ));
    v.push(neg(
        "neg_mans2_largest_seq_above_last_seq",
        Ctx::Manifest,
        one_snap(vec![fmeta(1, 3, b"a", b"b", 1, 10)], 9, 9, 9),
        "MAN-S2",
        C,
    ));
    v.push(neg(
        "neg_mans3_l1_overlap",
        Ctx::Manifest,
        one_snap(vec![fmeta(1, 3, b"a", b"k", 1, 1), fmeta(1, 4, b"k", b"z", 1, 1)], 9, 9, 9),
        "MAN-S3",
        C,
    ));
    v.push(neg(
        "neg_mans4_l0_order",
        Ctx::Manifest,
        one_snap(vec![fmeta(0, 3, b"a", b"k", 1, 5), fmeta(0, 4, b"c", b"z", 6, 9)], 9, 9, 9),
        "MAN-S4",
        C,
    ));
    v
}

// ------------------------------------------------------------------ tests

fn read_bin(name: &str) -> Vec<u8> {
    let path = vectors_dir().join(format!("{name}.bin"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (regenerate the vectors)", path.display()))
}

fn spec_class(c: ErrorClass) -> Class {
    match c {
        ErrorClass::Corruption => Class::Corruption,
        ErrorClass::Incompatible => Class::Incompatible,
    }
}

#[test]
#[ignore = "rewrites the committed vectors; only through the process of v1.md §15"]
fn regenerate_struct_vectors() {
    let dir = vectors_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "bin") {
            std::fs::remove_file(path).unwrap();
        }
    }
    for v in vectors() {
        std::fs::write(dir.join(format!("{}.bin", v.name)), &v.bytes).unwrap();
    }
}

#[test]
fn struct_vectors_encode_exact() {
    let all = vectors();
    let names: BTreeSet<&str> = all.iter().map(|v| v.name).collect();
    assert_eq!(names.len(), all.len(), "duplicate vector names");
    let negatives = all.iter().filter(|v| matches!(v.expect, Expect::Reject(..))).count();
    assert!(negatives >= 40, "v1.md §13.1 asks for at least 40 negative vectors, got {negatives}");
    let on_disk: BTreeSet<String> = std::fs::read_dir(vectors_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    let expected: BTreeSet<String> = names.iter().map(|s| s.to_string()).collect();
    assert_eq!(on_disk, expected, "the committed .bin files differ from the vector table");
    for v in &all {
        assert!(read_bin(v.name) == v.bytes, "vector {} no longer encodes to its committed bytes", v.name);
    }
}

#[test]
fn struct_vectors_decode_roundtrip() {
    for v in vectors() {
        let Expect::Accept(want) = &v.expect else { continue };
        let bytes = read_bin(v.name);
        let (got, _) = fmt_decode(&v.ctx, &bytes).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(&got, want, "{}: decoded value differs from its input", v.name);
        assert!(encode(&v.ctx, &got) == bytes, "{}: re-encoding the decoded value changes the bytes", v.name);
    }
}

#[test]
fn struct_negative_vectors_rejected() {
    let mut rules = BTreeSet::new();
    for v in vectors() {
        let Expect::Reject(rule, class) = v.expect else { continue };
        let bytes = read_bin(v.name);
        match fmt_decode(&v.ctx, &bytes) {
            Ok((_, s)) => panic!("{}: accepted, expected {rule} ({class:?}): {s}", v.name),
            Err(e) => assert_eq!((e.rule, e.class), (rule, class), "{}: wrong rejection ({e})", v.name),
        }
        rules.insert(rule);
    }
    for rule in [
        "VAR-1", "VAR-2", "VAR-3", "HDR-1", "HDR-2", "HDR-3", "HDR-4", "HDR-5", "HDR-6", "HDR-9", "HDR-10", "AEAD-1",
        "WAL-C4", "WAL-R1", "WAL-R2", "WAL-R3", "WAL-R5", "WAL-R6", "SST-O1", "SST-O2", "SST-O3", "SST-O4", "SST-O5",
        "SST-O6", "SST-O7", "SST-F1", "SST-R1", "SST-R2", "SST-R3", "SST-V1", "SST-V2", "SST-V3", "SST-V4", "MAN-R1",
        "MAN-R2", "MAN-R3", "MAN-A1", "MAN-S1", "MAN-S2", "MAN-S3", "MAN-S4", "MAN-S5",
    ] {
        assert!(rules.contains(rule), "no negative vector for {rule}");
    }
}

/// Bytes derived from `base` by a few random edits; with `reseal`, the CRC
/// of the structure is recomputed so that the inner parser runs.
fn mutate(rng: &mut Rng, ctx: &Ctx, base: &[u8], reseal: bool) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..1 + rng.below(3) {
        match rng.below(6) {
            0 | 1 if !b.is_empty() => {
                let i = rng.below(b.len() as u64) as usize;
                b[i] ^= 1 << rng.below(8);
            }
            2 if !b.is_empty() => {
                let i = rng.below(b.len() as u64) as usize;
                b[i] = rng.below(256) as u8;
            }
            3 if !b.is_empty() => b.truncate(rng.below(b.len() as u64) as usize),
            4 => {
                let n = rng.below(16) as usize;
                b.extend(rng.bytes(n));
            }
            _ if b.len() > 2 => {
                let i = rng.below(b.len() as u64 - 1) as usize;
                b.insert(i, b[i + 1]);
            }
            _ => {}
        }
    }
    if reseal {
        match ctx {
            Ctx::Header { .. } | Ctx::Wal { .. } | Ctx::Sst { .. } | Ctx::Manifest => {
                header::reseal_header(&mut b);
            }
            Ctx::Frame { file_number, .. } if b.len() >= 32 => wal::reseal_frame(&mut b, *file_number),
            Ctx::DataBlock { file_number, offset }
            | Ctx::FilterBlock { file_number, offset }
            | Ctx::RangeDelBlock { file_number, offset }
            | Ctx::PropsBlock { file_number, offset }
            | Ctx::IndexBlock { file_number, offset }
                if b.len() >= 4 =>
            {
                block::reseal_block(&mut b, *file_number, *offset)
            }
            Ctx::Footer { file_number } if b.len() == 80 => sst::reseal_footer(&mut b, *file_number),
            Ctx::Record { generation, .. } if b.len() >= 24 => manifest::reseal_record(&mut b, *generation),
            _ => {}
        }
    }
    b
}

/// For whole files: edits inside one SST block, WAL frame or MANIFEST record
/// of a valid `base`, with that structure's CRC recomputed, so that the
/// semantic checks (SST-O5..V4, WAL-R3, MAN-S*) run on damaged contents.
fn structured_mutation(rng: &mut Rng, ctx: &Ctx, base: &[u8]) -> Option<Vec<u8>> {
    let mut b = base.to_vec();
    let edit = |rng: &mut Rng, b: &mut [u8], from: usize, to: usize| {
        for _ in 0..1 + rng.below(3) {
            let i = from + rng.below((to - from) as u64) as usize;
            if rng.chance(1, 2) {
                b[i] ^= 1 << rng.below(8);
            } else {
                b[i] = rng.below(256) as u8;
            }
        }
    };
    match ctx {
        Ctx::Sst { file_number, .. } => {
            let ex = SstExpect { file_number: *file_number, db_id: DB, meta: None, last_seq: u64::MAX >> 1 };
            let m = sst::open_sst(base, &ex).ok()?;
            let mut handles: Vec<BlockHandle> = m.index.iter().map(|e| e.handle).collect();
            handles.extend([m.footer.filter, m.footer.props, m.footer.index]);
            if m.footer.rdel != BlockHandle::default() {
                handles.push(m.footer.rdel);
            }
            let h = handles[rng.below(handles.len() as u64) as usize];
            let (from, to) = (h.offset as usize, (h.offset + h.len) as usize);
            edit(rng, &mut b, from, to - 44);
            block::reseal_block(&mut b[from..to], *file_number, h.offset);
        }
        Ctx::Wal { file_number, db_id, expected_seq } => {
            let params = WalScanParams { file_number: *file_number, db_id: *db_id, expected_seq: *expected_seq };
            let mut frames = Vec::new();
            wal::scan_wal(&mut &base[..], &params, |off, f| {
                frames.push((off as usize, wal::encode_frame(*file_number, &f, false).len()));
                Ok(())
            })
            .ok()?;
            let &(off, len) = frames.get(rng.below(frames.len().max(1) as u64) as usize)?;
            edit(rng, &mut b, off + 8, off + len);
            wal::reseal_frame(&mut b[off..off + len], *file_number);
        }
        Ctx::Manifest => {
            let mut recs = Vec::new();
            let r = manifest::replay_manifest(&mut &base[..], |off, rec| {
                recs.push((off as usize, rec.clone()));
                Ok(())
            })
            .ok()?;
            let (off, rec) = recs.get(rng.below(recs.len() as u64) as usize)?.clone();
            let len = rec.encode(r.header.file_number).len();
            edit(rng, &mut b, off + 8, off + len);
            manifest::reseal_record(&mut b[off..off + len], r.header.file_number);
        }
        _ => return None,
    }
    Some(b)
}

fn ctx_kind(ctx: &Ctx) -> std::mem::Discriminant<Ctx> {
    std::mem::discriminant(ctx)
}

/// Random inputs for one decoder: purely random bytes, and edits of that
/// decoder's vectors with and without recomputed CRCs.
fn random_inputs(all: &[Vector], kind: std::mem::Discriminant<Ctx>, n: usize, seed: u64) -> Vec<(Ctx, Vec<u8>)> {
    let bases: Vec<&Vector> = all.iter().filter(|v| ctx_kind(&v.ctx) == kind).collect();
    let mut rng = Rng::new(seed);
    (0..n)
        .map(|_| {
            let base = bases[rng.below(bases.len() as u64) as usize];
            let bytes = match rng.below(5) {
                0 => {
                    let len = rng.below(base.bytes.len() as u64 * 2 + 64) as usize;
                    rng.bytes(len)
                }
                1 => mutate(&mut rng, &base.ctx, &base.bytes, false),
                2 => mutate(&mut rng, &base.ctx, &base.bytes, true),
                _ => match structured_mutation(&mut rng, &base.ctx, &base.bytes) {
                    Some(b) => b,
                    None => mutate(&mut rng, &base.ctx, &base.bytes, true),
                },
            };
            (base.ctx.clone(), bytes)
        })
        .collect()
}

#[test]
fn decoders_never_panic_on_random_bytes() {
    let all = vectors();
    let mut kinds: Vec<std::mem::Discriminant<Ctx>> = Vec::new();
    for v in &all {
        if !kinds.contains(&ctx_kind(&v.ctx)) {
            kinds.push(ctx_kind(&v.ctx));
        }
    }
    assert_eq!(kinds.len(), 14, "every decoder has vectors");
    let n: usize = std::env::var("KERNO_FUZZ_INPUTS").ok().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    for (i, kind) in kinds.iter().enumerate() {
        let mut accepted = 0;
        for (ctx, bytes) in random_inputs(&all, *kind, n, 0xF0_0000 + i as u64) {
            accepted += usize::from(fmt_decode(&ctx, &bytes).is_ok());
        }
        assert!(accepted < n, "decoder {i} accepted every random input");
    }
}

#[test]
fn spec_decoder_agrees() {
    let all = vectors();
    for v in &all {
        let bytes = read_bin(v.name);
        let spec = spec_decoder::decode(&v.ctx, &bytes);
        match (&v.expect, &spec) {
            (Expect::Accept(_), Ok(s)) => {
                let (_, mine) = fmt_decode(&v.ctx, &bytes).unwrap();
                assert_eq!(s, &mine, "{}: decoded contents differ", v.name);
            }
            (Expect::Reject(rule, class), Err(got)) => {
                assert_eq!(*got, (*rule, spec_class(*class)), "{}: spec decoder rejects with another rule", v.name)
            }
            (Expect::Accept(_), Err(e)) => panic!("{}: spec decoder rejects a positive vector: {e:?}", v.name),
            (Expect::Reject(rule, _), Ok(s)) => panic!("{}: spec decoder accepts (expected {rule}): {s}", v.name),
        }
    }
    // Agreement beyond the vectors: edited and random inputs of every
    // decoder, including inputs that break several rules at once.
    let n: usize = std::env::var("KERNO_AGREE_INPUTS").ok().and_then(|s| s.parse().ok()).unwrap_or(5_000);
    let mut kinds: Vec<std::mem::Discriminant<Ctx>> = Vec::new();
    for v in &all {
        if !kinds.contains(&ctx_kind(&v.ctx)) {
            kinds.push(ctx_kind(&v.ctx));
        }
    }
    for (i, kind) in kinds.iter().enumerate() {
        for (ctx, bytes) in random_inputs(&all, *kind, n, 0xA9_0000 + i as u64) {
            let mine = fmt_decode(&ctx, &bytes).map(|(_, s)| s).map_err(|e| (e.rule, spec_class(e.class)));
            let spec = spec_decoder::decode(&ctx, &bytes);
            assert_eq!(mine, spec, "decoders disagree on {ctx:?} input {}", hx(&bytes));
        }
    }
}

#[test]
fn spec_hex_matches_vectors() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/file-format/v1.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut blocks: Vec<(String, Vec<u8>)> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(name) = line.strip_prefix("```hex vector=") else { continue };
        let mut bytes = Vec::new();
        for l in lines.by_ref() {
            if l.starts_with("```") {
                break;
            }
            for tok in l.split_whitespace() {
                bytes.push(u8::from_str_radix(tok, 16).unwrap_or_else(|_| panic!("bad hex {tok:?} in {name}")));
            }
        }
        blocks.push((name.trim().to_string(), bytes));
    }
    assert!(blocks.len() >= 5, "v1.md §13.5 must carry at least 5 hex vectors, found {}", blocks.len());
    for kind in ["hdr_", "frame_", "data_block", "sst_footer", "rec_snapshot"] {
        assert!(blocks.iter().any(|(n, _)| n.starts_with(kind)), "no {kind}* hex vector in v1.md");
    }
    for (name, bytes) in blocks {
        assert!(read_bin(&name) == bytes, "hex block {name} in v1.md differs from {name}.bin");
    }
}
