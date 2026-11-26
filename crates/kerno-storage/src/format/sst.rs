//! SST files (v1.md §7): index (§7.7), footer (§7.8), the open-time checks
//! SST-O1..O7, the per-block checks SST-R1..R3, the `verify()` checks
//! SST-V1..V4 and the deterministic writer of §7.10.

use super::block::{BLOCK_TRAILER_LEN, DataBlockBuilder, Entry, MIN_DATA_BLOCK_PAYLOAD, append_block, open_block};
use super::bloom;
use super::coding::{Reader, le_u32, le_u64, put_bytes, put_u32, put_u64, put_varint};
use super::crc::{crc_ctx, file_ctx};
use super::filter::{Filter, decode_filter};
use super::header::{FileKind, HEADER_LEN, Header, HeaderExpect, decode_header};
use super::limits::{MAX_BLOCK_SIZE, MAX_HEADER_LEN, MAX_KEY_LEN, MAX_META_PAYLOAD, MIN_BLOCK_SIZE};
use super::manifest::FileMeta;
use super::props::{Props, check_props, decode_props};
use super::rangedel::{Fragment, check_fragment_seqs, covering_seq, decode_rangedel, encode_rangedel};
use super::{ByteSource, FormatError, ReadError, Result};

pub const FOOTER_LEN: usize = 80;
/// Smallest SST: header and footer (SST-O1).
pub const MIN_SST_LEN: u64 = (HEADER_LEN + FOOTER_LEN) as u64;
/// Smallest stored data block length (SST-O5).
pub const MIN_DATA_BLOCK_LEN: u64 = (BLOCK_TRAILER_LEN + MIN_DATA_BLOCK_PAYLOAD) as u64;
/// Footer magic (the SST header magic).
pub const FOOTER_MAGIC: [u8; 8] = *b"KRNO_SST";

/// Offset and stored length (`payload_len + 44`) of a block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockHandle {
    pub offset: u64,
    pub len: u64,
}

impl BlockHandle {
    fn end(&self) -> Option<u64> {
        self.offset.checked_add(self.len)
    }
}

/// The 80-byte footer at `len − 80`. `rdel` is all zero when the table has
/// no range block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footer {
    pub index: BlockHandle,
    pub props: BlockHandle,
    pub filter: BlockHandle,
    pub rdel: BlockHandle,
}

impl Footer {
    pub fn encode(&self, file_number: u64) -> [u8; FOOTER_LEN] {
        let mut b = Vec::with_capacity(FOOTER_LEN);
        for h in [self.index, self.props, self.filter, self.rdel] {
            put_u64(&mut b, h.offset);
            put_u64(&mut b, h.len);
        }
        b.extend_from_slice(&FOOTER_MAGIC);
        put_u32(&mut b, 0);
        let crc = crc_ctx(&file_ctx(file_number), &b);
        put_u32(&mut b, crc);
        b.try_into().unwrap()
    }
}

/// Recomputes the CRC of a whole (possibly patched) footer in place.
pub fn reseal_footer(footer: &mut [u8], file_number: u64) {
    let crc = crc_ctx(&file_ctx(file_number), &footer[..76]);
    footer[76..80].copy_from_slice(&crc.to_le_bytes());
}

/// SST-O2: the 80 footer bytes (`base` is their offset): CRC, magic,
/// reserved, in that order.
pub fn decode_footer(buf: &[u8], file_number: u64, base: u64) -> Result<Footer> {
    let o2 = |at: u64, detail| FormatError::corruption("SST-O2", base + at, detail);
    if buf.len() != FOOTER_LEN {
        return Err(o2(0, "footer is not 80 bytes"));
    }
    if crc_ctx(&file_ctx(file_number), &buf[..76]) != le_u32(buf, 76) {
        return Err(o2(76, "footer CRC mismatch"));
    }
    if buf[64..72] != FOOTER_MAGIC {
        return Err(o2(64, "bad footer magic"));
    }
    if le_u32(buf, 72) != 0 {
        return Err(o2(72, "reserved footer bytes are not zero"));
    }
    let h = |i: usize| BlockHandle { offset: le_u64(buf, 16 * i), len: le_u64(buf, 16 * i + 8) };
    Ok(Footer { index: h(0), props: h(1), filter: h(2), rdel: h(3) })
}

/// SST-O3: the meta blocks are contiguous and end at the footer. Returns
/// `D`, the end of the data blocks (`rdel_off` with a range block, else
/// `filter_off`). With no range block both `rdel` fields are zero.
pub fn check_layout(f: &Footer, file_len: u64) -> Result<u64> {
    let o3 =
        || Err(FormatError::corruption("SST-O3", file_len.saturating_sub(FOOTER_LEN as u64), "inconsistent layout"));
    let has_rdel = f.rdel != BlockHandle::default();
    if has_rdel && (f.rdel.offset == 0 || f.rdel.len == 0 || f.rdel.end() != Some(f.filter.offset)) {
        return o3();
    }
    if f.filter.end() != Some(f.props.offset)
        || f.props.end() != Some(f.index.offset)
        || f.index.end() != file_len.checked_sub(FOOTER_LEN as u64)
    {
        return o3();
    }
    let d = if has_rdel { f.rdel.offset } else { f.filter.offset };
    if d < HEADER_LEN as u64 {
        return o3();
    }
    Ok(d)
}

/// One index entry: the last key of a data block and its handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry<B = Vec<u8>> {
    pub last_key: B,
    pub handle: BlockHandle,
}

pub fn encode_index<B: AsRef<[u8]>>(entries: &[IndexEntry<B>]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, entries.len() as u64);
    for e in entries {
        put_bytes(&mut out, e.last_key.as_ref());
        put_varint(&mut out, e.handle.offset);
        put_varint(&mut out, e.handle.len);
    }
    out
}

/// Parses an index payload (§7.7); a payload cut short or with trailing
/// bytes is SST-O4. The semantic checks are [`check_index`].
pub fn decode_index(payload: &[u8], base: u64) -> Result<Vec<IndexEntry>> {
    let mut r = Reader::new(payload, base, "SST-O4");
    let count = r.varint()?;
    let mut out = Vec::with_capacity((count as usize).min(payload.len() / 3));
    for _ in 0..count {
        let last_key = r.bytes(MAX_KEY_LEN)?.to_vec();
        let offset = r.varint()?;
        let len = r.varint()?;
        out.push(IndexEntry { last_key, handle: BlockHandle { offset, len } });
    }
    r.finish()?;
    Ok(out)
}

/// What the checks and the readers ask of a table's index, whatever holds it.
pub trait BlockIndex {
    /// How many data blocks the index lists.
    fn blocks(&self) -> usize;
    /// The last key of data block `i`.
    fn last_key(&self, i: usize) -> &[u8];
    fn handle(&self, i: usize) -> BlockHandle;

    /// The first block whose last key is `>= key` (`blocks()` if none): the one that holds
    /// `key` if the table does.
    fn block_for(&self, key: &[u8]) -> usize {
        let (mut lo, mut hi) = (0, self.blocks());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.last_key(mid) < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

impl<B: AsRef<[u8]>> BlockIndex for [IndexEntry<B>] {
    fn blocks(&self) -> usize {
        self.len()
    }

    fn last_key(&self, i: usize) -> &[u8] {
        self[i].last_key.as_ref()
    }

    fn handle(&self, i: usize) -> BlockHandle {
        self[i].handle
    }
}

impl<B: AsRef<[u8]>> BlockIndex for Vec<IndexEntry<B>> {
    fn blocks(&self) -> usize {
        self.len()
    }

    fn last_key(&self, i: usize) -> &[u8] {
        self[i].last_key.as_ref()
    }

    fn handle(&self, i: usize) -> BlockHandle {
        self[i].handle
    }
}

/// An index that can be parsed from its block payload.
pub trait DecodeIndex: BlockIndex + Sized {
    /// As [`decode_index`]: the same checks, the same errors.
    fn decode(payload: Vec<u8>, base: u64) -> Result<Self>;
}

impl DecodeIndex for Vec<IndexEntry> {
    fn decode(payload: Vec<u8>, base: u64) -> Result<Self> {
        decode_index(&payload, base)
    }
}

/// The index of an open table as the engine keeps it: the block payload as it was read, and
/// where each entry's key lies in it. [`decode_index`] makes one allocation per data block,
/// which was most of the cost of loading a table; this makes two per table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatIndex {
    payload: Vec<u8>,
    entries: Vec<FlatEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FlatEntry {
    key_at: u32,
    key_len: u32,
    handle: BlockHandle,
}

impl FlatIndex {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl BlockIndex for FlatIndex {
    fn blocks(&self) -> usize {
        self.entries.len()
    }

    fn last_key(&self, i: usize) -> &[u8] {
        let e = &self.entries[i];
        &self.payload[e.key_at as usize..(e.key_at + e.key_len) as usize]
    }

    fn handle(&self, i: usize) -> BlockHandle {
        self.entries[i].handle
    }
}

impl DecodeIndex for FlatIndex {
    fn decode(payload: Vec<u8>, base: u64) -> Result<Self> {
        // A meta block payload is at most `MAX_META_PAYLOAD` long, so positions fit in 32 bits.
        debug_assert!(payload.len() as u64 <= MAX_META_PAYLOAD);
        match flat_entries(&payload) {
            Some(entries) => Ok(FlatIndex { payload, entries }),
            // Not a valid index: the checked parser names the rule and the offset.
            None => Err(decode_index(&payload, base)
                .err()
                .unwrap_or_else(|| FormatError::corruption("SST-O4", base, "index block does not parse"))),
        }
    }
}

/// A varint of a valid index at `*pos`, or `None` for anything [`decode_varint`] refuses.
#[inline(always)]
fn flat_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for i in 0..10 {
        let b = *buf.get(*pos + i)?;
        v |= u64::from(b & 0x7f) << (7 * i);
        if b < 0x80 {
            if (i == 9 && b > 1) || (b == 0 && i > 0) {
                return None;
            }
            *pos += i + 1;
            return Some(v);
        }
    }
    None
}

/// The entries of an index payload [`decode_index`] accepts, and `None` for one it does not:
/// the same grammar, without building an error on the way (a table's index is decoded every
/// time the table is loaded, and a large table has thousands of entries).
fn flat_entries(payload: &[u8]) -> Option<Vec<FlatEntry>> {
    let mut pos = 0;
    let count = flat_varint(payload, &mut pos)?;
    let mut entries = Vec::with_capacity((count as usize).min(payload.len() / 3));
    for _ in 0..count {
        let key_len = flat_varint(payload, &mut pos)?;
        if key_len > MAX_KEY_LEN as u64 || key_len > (payload.len() - pos) as u64 {
            return None;
        }
        let key_at = pos;
        pos += key_len as usize;
        let offset = flat_varint(payload, &mut pos)?;
        let len = flat_varint(payload, &mut pos)?;
        entries.push(FlatEntry { key_at: key_at as u32, key_len: key_len as u32, handle: BlockHandle { offset, len } });
    }
    (pos == payload.len()).then_some(entries)
}

/// SST-O5: the entries cover exactly `[96, D)` in order, each length in
/// `55..=80 MiB + 44`, last keys strictly increasing, `count = data_blocks`,
/// and `count = 0` exactly when `point_entries = 0`. Errors carry `at`, the
/// offset of the index block.
pub fn check_index<I: BlockIndex + ?Sized>(index: &I, data_end: u64, props: &Props, at: u64) -> Result<()> {
    let o5 = |detail| Err(FormatError::corruption("SST-O5", at, detail));
    let mut next = HEADER_LEN as u64;
    for i in 0..index.blocks() {
        let handle = index.handle(i);
        if handle.offset != next {
            return o5("index entries are not contiguous from offset 96");
        }
        if handle.len < MIN_DATA_BLOCK_LEN
            || handle.len > super::block::MAX_STORED_DATA_PAYLOAD + BLOCK_TRAILER_LEN as u64
        {
            return o5("data block length out of range");
        }
        if i > 0 && index.last_key(i - 1) >= index.last_key(i) {
            return o5("index keys not strictly increasing");
        }
        next = match handle.end() {
            Some(n) => n,
            None => return o5("index offsets overflow"),
        };
    }
    if next != data_end {
        return o5("index does not cover the data blocks exactly");
    }
    if index.blocks() as u64 != props.data_blocks {
        return o5("index count differs from data_blocks");
    }
    if (index.blocks() == 0) != (props.point_entries == 0) {
        return o5("index count 0 does not match point_entries 0");
    }
    Ok(())
}

/// What an SST is checked against when it is opened.
#[derive(Clone, Copy, Debug)]
pub struct SstExpect<'a> {
    /// The number in the file name.
    pub file_number: u64,
    /// The MANIFEST's `db_id`.
    pub db_id: [u8; 16],
    /// The MANIFEST's `FileMeta` for this table (SST-O1, SST-O6).
    pub meta: Option<&'a FileMeta>,
    /// The MANIFEST's `last_seq` (SST-O6).
    pub last_seq: u64,
}

/// The resident part of an open SST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SstMeta {
    pub header: Header,
    pub footer: Footer,
    /// `D`: end of the data blocks.
    pub data_end: u64,
    pub index: Vec<IndexEntry>,
    pub props: Props,
    pub filter: Filter,
    pub fragments: Vec<Fragment>,
}

fn block_slice(file: &[u8], h: BlockHandle) -> &[u8] {
    &file[h.offset as usize..(h.offset + h.len) as usize]
}

/// Opens an SST held in memory: SST-O1..O7 in order. The meta region
/// (`[D, len − 80)`) is decoded; data blocks are read with
/// [`read_data_block`].
///
/// Order: header (HDR-1..10), `len = size` and `len ≥ 176` (SST-O1), footer
/// (SST-O2), layout (SST-O3), encapsulation of the meta blocks in file order
/// (SST-O4), index and properties parsing (SST-O4, SST-O6), index (SST-O5),
/// properties (SST-O6), filter (SST-O7), range block (SST-O4, SST-F1) and its
/// consistency (SST-O7, SST-F1).
pub fn open_sst(file: &[u8], ex: &SstExpect<'_>) -> Result<SstMeta> {
    open_sst_from(&mut &file[..], ex).map_err(ReadError::into_format)
}

/// [`open_sst`] through a [`ByteSource`]: reads the header, the footer and
/// the meta blocks, never the data blocks. A meta block whose length is out
/// of range is rejected before it is read.
pub fn open_sst_from<S: ByteSource>(
    src: &mut S,
    ex: &SstExpect<'_>,
) -> std::result::Result<SstMeta, ReadError<S::Error>> {
    let o: SstOpen<Vec<IndexEntry>> = open_sst_as(src, ex, true)?;
    let filter = o.filter.expect("the filter was asked for");
    Ok(SstMeta {
        header: o.header,
        footer: o.footer,
        data_end: o.data_end,
        index: o.index,
        props: o.props,
        filter,
        fragments: o.fragments,
    })
}

/// What [`open_sst_as`] read: [`SstMeta`] with the index kept as `I`, and the filter only if it
/// was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SstOpen<I> {
    pub header: Header,
    pub footer: Footer,
    /// `D`: end of the data blocks.
    pub data_end: u64,
    pub index: I,
    pub props: Props,
    /// `None` when the open left the filter block unread ([`read_filter`] reads it later).
    pub filter: Option<Filter>,
    pub fragments: Vec<Fragment>,
}

/// Reads a meta block: `open_block`'s length checks before the read, then SST-O4.
fn read_meta_block<S: ByteSource>(
    src: &mut S,
    file_number: u64,
    h: BlockHandle,
) -> std::result::Result<Vec<u8>, ReadError<S::Error>> {
    if h.len < BLOCK_TRAILER_LEN as u64 {
        return Err(FormatError::corruption("SST-O4", h.offset, "block shorter than its trailer").into());
    }
    if h.len - BLOCK_TRAILER_LEN as u64 > MAX_META_PAYLOAD {
        return Err(FormatError::corruption("SST-O4", h.offset, "block payload above the format limit").into());
    }
    let mut block = vec![0; h.len as usize];
    src.read_exact_at(h.offset, &mut block).map_err(ReadError::Source)?;
    let payload_len = open_block(&block, file_number, h.offset, MAX_META_PAYLOAD, "SST-O4")?.len();
    block.truncate(payload_len);
    Ok(block)
}

/// The filter block of a table whose open left it unread: SST-O4 of the block, then SST-O7 of
/// the filter, as [`open_sst_from`] applies them. `footer` is the open's.
pub fn read_filter<S: ByteSource>(
    src: &mut S,
    file_number: u64,
    footer: &Footer,
) -> std::result::Result<Filter, ReadError<S::Error>> {
    let payload = read_meta_block(src, file_number, footer.filter)?;
    Ok(decode_filter(&payload, footer.filter.offset)?.to_owned_filter())
}

/// [`open_sst_from`] with the index kept as `I`. Without `with_filter` the filter block is
/// neither read nor checked (its handle still is, by SST-O3): the rules that concern it
/// (SST-O4 of its block, SST-O7) are then [`read_filter`]'s, whenever the caller needs the
/// filter. Every other rule is evaluated, in the same order.
pub fn open_sst_as<S: ByteSource, I: DecodeIndex>(
    src: &mut S,
    ex: &SstExpect<'_>,
    with_filter: bool,
) -> std::result::Result<SstOpen<I>, ReadError<S::Error>> {
    let len = src.len();
    // The header: 96 bytes, or the longer header_len a future version
    // declares (needed to verify its CRC before rejecting it by version).
    let mut prefix = vec![0; len.min(HEADER_LEN as u64) as usize];
    src.read_exact_at(0, &mut prefix).map_err(ReadError::Source)?;
    if prefix.len() == HEADER_LEN {
        let declared = u64::from(le_u32(&prefix, 12));
        if declared > HEADER_LEN as u64 && declared <= len.min(u64::from(MAX_HEADER_LEN)) {
            prefix.resize(declared as usize, 0);
            src.read_exact_at(0, &mut prefix).map_err(ReadError::Source)?;
        }
    }
    let expect = HeaderExpect { db_id: Some(ex.db_id), file_number: Some(ex.file_number) };
    let header = decode_header(&prefix, len, FileKind::Sst, &expect)?;
    if ex.meta.is_some_and(|m| m.size != len) {
        return Err(FormatError::corruption("SST-O1", 0, "file size differs from the MANIFEST").into());
    }
    if len < MIN_SST_LEN {
        return Err(FormatError::corruption("SST-O1", 0, "file shorter than header and footer").into());
    }
    let footer_at = len - FOOTER_LEN as u64;
    let mut footer_bytes = [0u8; FOOTER_LEN];
    src.read_exact_at(footer_at, &mut footer_bytes).map_err(ReadError::Source)?;
    let footer = decode_footer(&footer_bytes, ex.file_number, footer_at)?;
    let data_end = check_layout(&footer, len)?;
    let has_rdel = footer.rdel != BlockHandle::default();
    // SST-O4 in file order, with open_block's length checks before reading.
    let mut read_meta = |h: BlockHandle| read_meta_block(src, ex.file_number, h);
    let rdel_payload = if has_rdel { Some(read_meta(footer.rdel)?) } else { None };
    let filter_payload = if with_filter { Some(read_meta(footer.filter)?) } else { None };
    let props_payload = read_meta(footer.props)?;
    let index_payload = read_meta(footer.index)?;
    let index_at = footer.index.offset;
    // The order of the two is the order of their errors (SST-O4 of the index first).
    let index = I::decode(index_payload, index_at)?;
    let props = decode_props(&props_payload, footer.props.offset)?;
    check_index(&index, data_end, &props, footer.index.offset)?;
    check_props(&props, ex.meta, ex.last_seq, footer.props.offset)?;
    let filter = match &filter_payload {
        Some(p) => Some(decode_filter(p, footer.filter.offset)?.to_owned_filter()),
        None => None,
    };
    let fragments: Vec<Fragment> = match &rdel_payload {
        Some(p) => decode_rangedel(p, footer.rdel.offset)?.iter().map(Fragment::to_owned_fragment).collect(),
        None => Vec::new(),
    };
    // Without a range block, a fragment count above 0 is the properties' fault.
    check_fragments(&fragments, &props, if has_rdel { footer.rdel.offset } else { footer.props.offset })?;
    Ok(SstOpen { header, footer, data_end, index, props, filter, fragments })
}

/// SST-V3 applied at open to the keys the open already holds: the last key of
/// every data block is a point key of the table (SST-R2 makes the block agree
/// with its index entry when it is read), so a filter built over the table's
/// keys contains it. One hash per data block, no data block read: a `get`
/// never pays to distrust the filter, and a filter that was zeroed or
/// overwritten wholesale is refused before it can hide keys. The check is
/// partial by construction: a filter that lacks only keys in the middle of a
/// block passes, and only `verify()` (a full read) finds those.
///
/// It is not part of [`open_sst`]: that function evaluates the rules of
/// `docs/file-format/v1.md` §13.1 in their frozen order, and this check would
/// answer SST-V3 for a table whose index lies (SST-R1, SST-R2) before the
/// block that proves it is read. The engine (`Table::open`) and `verify()` call
/// it right after the open instead. The error carries the offset of the filter
/// block and no key bytes.
pub fn check_filter_holds_index_keys(sst: &SstMeta) -> Result<()> {
    check_filter_holds_keys(&sst.filter, &sst.index, sst.footer.filter.offset, usize::MAX)
}

/// [`check_filter_holds_index_keys`] over at most `sample` of the index keys, evenly spread and
/// always with the first and the last. What the check is for is a filter that is not this
/// table's (zeroed, or another table's bytes under a valid CRC), and such a filter misses
/// nearly every key: one that holds a share `p` of the keys passes a sample of `n` with
/// probability `p^n`. A table opened for a few reads (`Config::lazy_tables`) asks for a
/// sample, because hashing every index key costs more than those reads; an eager open and
/// `verify()` ask for all of them.
pub fn check_filter_holds_keys<I: BlockIndex + ?Sized>(
    filter: &Filter,
    index: &I,
    filter_offset: u64,
    sample: usize,
) -> Result<()> {
    let n = index.blocks();
    let holds = |i: usize| filter.may_contain(bloom::hash(index.last_key(i)));
    let ok = if n <= sample || sample < 2 {
        (0..n).all(holds)
    } else {
        // `sample` positions from 0 to n - 1 inclusive.
        (0..sample).all(|j| holds(j * (n - 1) / (sample - 1)))
    };
    if ok {
        return Ok(());
    }
    Err(FormatError::corruption("SST-V3", filter_offset, "the last key of a data block is missing from the filter"))
}

/// SST-O7 and SST-F1 for the resident fragments: as many as
/// `range_fragments` (SST-O7), sequences within the table's range (SST-F1),
/// and `smallest_key ≤ start` and `end ≤ largest_key` for each fragment
/// (SST-O7). With `largest_exclusive = 0` the last fragment ends at or before
/// the last point key, so `end ≤ largest_key` holds in both cases (§7.6).
/// Errors carry `at`, the offset of the range block (of the properties block
/// when the table has none).
pub fn check_fragments<B: AsRef<[u8]>>(fragments: &[Fragment<B>], props: &Props, at: u64) -> Result<()> {
    if fragments.len() as u64 != props.range_fragments {
        return Err(FormatError::corruption("SST-O7", at, "fragment count differs from range_fragments"));
    }
    check_fragment_seqs(fragments, props.smallest_seq, props.largest_seq, at)?;
    let within =
        |f: &Fragment<B>| f.start.as_ref() >= &props.smallest_key[..] && f.end.as_ref() <= &props.largest_key[..];
    if !fragments.iter().all(within) {
        return Err(FormatError::corruption("SST-O7", at, "fragment outside the table's key range"));
    }
    Ok(())
}

/// Reads data block `i`: SST-R1 (encapsulation and structure), SST-R2 (last
/// key equals the index's; first key above the previous block's last key,
/// or at least `smallest_key` for block 0) and SST-R3 (sequences within the
/// table's range). `block` is exactly the block's bytes.
///
/// `payload` is the block's data-block payload, from
/// [`data_block_payload`](super::block::data_block_payload) (which checks the
/// encapsulation and decompresses).
pub fn read_data_block<'a, I: BlockIndex + ?Sized>(
    payload: &'a [u8],
    index: &I,
    i: usize,
    props: &Props,
) -> Result<Vec<Entry<&'a [u8]>>> {
    let h = index.handle(i);
    let entries = super::block::parse_data_block(payload, h.offset)?;
    let r2 = |detail| Err(FormatError::corruption("SST-R2", h.offset, detail));
    let (first, last) = (entries[0].key, entries[entries.len() - 1].key);
    if last != index.last_key(i) {
        return r2("last key differs from the index");
    }
    let first_ok = if i == 0 { first >= &props.smallest_key[..] } else { first > index.last_key(i - 1) };
    if !first_ok {
        return r2("first key not above the previous block");
    }
    if entries.iter().any(|e| e.seq < props.smallest_seq || e.seq > props.largest_seq) {
        return Err(FormatError::corruption("SST-R3", h.offset, "sequence outside the table's range"));
    }
    Ok(entries)
}

/// [`read_data_block`] for a reader that keeps the payload and not the entries: the same rules
/// in the same order (all of SST-R1 over the block, then SST-R2, then SST-R3), no allocation.
pub fn check_data_block<I: BlockIndex + ?Sized>(payload: &[u8], index: &I, i: usize, props: &Props) -> Result<()> {
    let h = index.handle(i);
    let (mut first, mut last): (&[u8], &[u8]) = (&[], &[]);
    let (mut seen, mut seq_ok) = (false, true);
    super::block::walk_data_block(payload, h.offset, |_, e| {
        if !seen {
            first = e.key;
            seen = true;
        }
        last = e.key;
        seq_ok &= e.seq >= props.smallest_seq && e.seq <= props.largest_seq;
    })?;
    let r2 = |detail| Err(FormatError::corruption("SST-R2", h.offset, detail));
    if last != index.last_key(i) {
        return r2("last key differs from the index");
    }
    let first_ok = if i == 0 { first >= &props.smallest_key[..] } else { first > index.last_key(i - 1) };
    if !first_ok {
        return r2("first key not above the previous block");
    }
    if !seq_ok {
        return Err(FormatError::corruption("SST-R3", h.offset, "sequence outside the table's range"));
    }
    Ok(())
}

/// The `verify()` checks over a full read of an SST (SST-V1..V4). Feed every
/// entry of every block in order with [`SstVerifier::observe`], then call
/// [`SstVerifier::finish`]. SST-V3 and SST-V4 errors carry the offset of the
/// data block holding the entry; SST-V1 and SST-V2 errors, the offset of the
/// properties block.
pub struct SstVerifier<'m> {
    meta: &'m SstMeta,
    entries: u64,
    deletes: u64,
    key_bytes: u64,
    value_bytes: u64,
    first_key: Option<Vec<u8>>,
    last_key: Option<Vec<u8>>,
}

impl<'m> SstVerifier<'m> {
    pub fn new(meta: &'m SstMeta) -> Self {
        SstVerifier { meta, entries: 0, deletes: 0, key_bytes: 0, value_bytes: 0, first_key: None, last_key: None }
    }

    /// SST-V3 (the key is in the filter) and SST-V4 (no fragment of this
    /// table with a higher sequence covers it). `block_offset` is the offset
    /// of the data block holding `e`.
    pub fn observe<B: AsRef<[u8]>>(&mut self, e: &Entry<B>, block_offset: u64) -> Result<()> {
        let key = e.key.as_ref();
        if !self.meta.filter.may_contain(bloom::hash(key)) {
            return Err(FormatError::corruption("SST-V3", block_offset, "point key missing from the filter"));
        }
        if covering_seq(&self.meta.fragments, key).is_some_and(|s| s > e.seq) {
            return Err(FormatError::corruption(
                "SST-V4",
                block_offset,
                "point covered by a newer fragment of the table",
            ));
        }
        self.entries += 1;
        self.key_bytes += key.len() as u64;
        match &e.value {
            Some(v) => self.value_bytes += v.as_ref().len() as u64,
            None => self.deletes += 1,
        }
        if self.first_key.is_none() {
            self.first_key = Some(key.to_vec());
        }
        self.last_key = Some(key.to_vec());
        Ok(())
    }

    /// SST-V1 (counts and byte sums) and SST-V2 (real extremes).
    pub fn finish(self) -> Result<()> {
        let p = &self.meta.props;
        let at = self.meta.footer.props.offset;
        if (self.entries, self.deletes, self.key_bytes, self.value_bytes)
            != (p.point_entries, p.point_deletes, p.raw_key_bytes, p.raw_value_bytes)
        {
            return Err(FormatError::corruption("SST-V1", at, "entry counts differ from the properties"));
        }
        let (smallest, largest, exclusive) = key_extremes(
            self.first_key.as_deref(),
            self.last_key.as_deref(),
            self.meta.fragments.first().map(|f| &f.start[..]),
            self.meta.fragments.last().map(|f| &f.end[..]),
        );
        if (smallest, largest, exclusive) != (&p.smallest_key[..], &p.largest_key[..], p.largest_exclusive) {
            return Err(FormatError::corruption("SST-V2", at, "key range differs from the properties"));
        }
        Ok(())
    }
}

/// `smallest_key`, `largest_key` and `largest_exclusive` of a table from its
/// first and last point keys and its first fragment start and last fragment
/// end (§7.6, tags 9–11).
pub fn key_extremes<'k>(
    first_point: Option<&'k [u8]>,
    last_point: Option<&'k [u8]>,
    first_start: Option<&'k [u8]>,
    last_end: Option<&'k [u8]>,
) -> (&'k [u8], &'k [u8], bool) {
    let smallest = match (first_point, first_start) {
        (Some(p), Some(s)) => p.min(s),
        (Some(p), None) => p,
        (None, Some(s)) => s,
        (None, None) => &[],
    };
    match (last_point, last_end) {
        (Some(p), Some(e)) if e > p => (smallest, e, true),
        (Some(p), _) => (smallest, p, false),
        (None, Some(e)) => (smallest, e, true),
        (None, None) => (smallest, &[], false),
    }
}

/// An SST read whole: the open-time checks, every data block (SST-R1..R3)
/// and the `verify()` checks (SST-V1..V4). Used by tests and vectors.
pub fn read_sst(file: &[u8], ex: &SstExpect<'_>) -> Result<(SstMeta, Vec<Entry>)> {
    let meta = open_sst(file, ex)?;
    let mut all = Vec::new();
    let mut v = SstVerifier::new(&meta);
    for i in 0..meta.index.len() {
        let block = block_slice(file, meta.index[i].handle);
        let lz4 = meta.header.incompat & super::features::BLOCK_LZ4 != 0;
        let payload = super::block::data_block_payload(block, ex.file_number, meta.index[i].handle.offset, lz4)?;
        for e in read_data_block(&payload, &meta.index, i, &meta.props)? {
            v.observe(&e, meta.index[i].handle.offset)?;
            all.push(e.to_owned_entry());
        }
    }
    v.finish()?;
    Ok((meta, all))
}

/// Deterministic SST writer (§7.10). Points are added in strictly
/// increasing key order; the output can be taken incrementally with
/// [`SstBuilder::take_ready`] and is completed by [`SstBuilder::finish`].
/// With the same entries, file number, `db_id` and block size the bytes are
/// identical.
pub struct SstBuilder {
    /// Data blocks are stored with a codec byte (§4.4).
    lz4: bool,
    /// Whether a block is tried with LZ4; off, every block gets codec 0.
    try_lz4: bool,
    file_number: u64,
    block_size: u32,
    pending: Vec<u8>,
    taken: u64,
    block: DataBlockBuilder,
    index: Vec<IndexEntry>,
    hashes: Vec<u64>,
    first_key: Option<Vec<u8>>,
    last_key: Vec<u8>,
    props: Props,
    /// Closed data blocks waiting for their LZ4 pass, with the last key of each: compressed
    /// together on several threads, then appended in order (see [`SstBuilder::drain`]). Only
    /// used when blocks are compressed; empty otherwise.
    queue: Vec<(Vec<u8>, Vec<u8>)>,
    /// Upper bound of the bytes the queued blocks will take once stored.
    queued_bytes: u64,
}

/// Data blocks compressed together by one [`SstBuilder::drain`].
const QUEUE_BLOCKS: usize = 1024;
/// Fewer queued blocks than this are compressed on the calling thread.
const PARALLEL_MIN_BLOCKS: usize = 16;

impl SstBuilder {
    /// A writer for SST `file_number`. `block_size` is both the data block
    /// closing threshold (§7.10) and the header field, which readers bound
    /// to 512..=1 MiB (HDR-9).
    ///
    /// # Panics
    ///
    /// If `block_size` is outside 512..=1 MiB: the table would be unreadable.
    pub fn new(db_id: [u8; 16], file_number: u64, block_size: u32) -> Self {
        Self::with_compression(db_id, file_number, block_size, false)
    }

    /// [`SstBuilder::new`], with LZ4 data blocks when `lz4` is set (the header then has
    /// `BLOCK_LZ4`, §4.4).
    ///
    /// # Panics
    ///
    /// As [`SstBuilder::new`].
    pub fn with_compression(db_id: [u8; 16], file_number: u64, block_size: u32, lz4: bool) -> Self {
        assert!(
            (MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size),
            "SST block_size {block_size} outside 512..=1 MiB (HDR-9)"
        );
        let mut header = Header::sst(db_id, file_number, block_size);
        if lz4 {
            header.incompat |= super::features::BLOCK_LZ4;
        }
        SstBuilder {
            lz4,
            try_lz4: lz4,
            file_number,
            block_size,
            pending: header.encode().to_vec(),
            taken: 0,
            block: DataBlockBuilder::new(),
            index: Vec::new(),
            hashes: Vec::new(),
            first_key: None,
            last_key: Vec::new(),
            props: Props { smallest_seq: u64::MAX, ..Props::default() },
            queue: Vec::new(),
            queued_bytes: 0,
        }
    }

    /// Skips the LZ4 pass: the header and the codec byte stay as with `lz4`
    /// (§4.4), but every data block is stored with codec 0, as is. For a
    /// writer that must be fast (the close-time flush); a no-op without `lz4`.
    pub fn without_lz4_pass(mut self) -> Self {
        self.try_lz4 = false;
        self
    }

    /// Bytes produced so far (taken or pending), not counting the blocks still queued for
    /// compression: see [`len_at_most`](Self::len_at_most) and [`drain`](Self::drain).
    pub fn len(&self) -> u64 {
        self.taken + self.pending.len() as u64
    }

    /// An upper bound of [`len`](Self::len) once the queued blocks are stored (a stored block
    /// is at most its payload, a codec byte and the trailer).
    pub fn len_at_most(&self) -> u64 {
        self.len() + self.queued_bytes
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Payload bytes of the data block that is still open: not yet part of
    /// [`SstBuilder::len`]. Lets a caller that cuts files at a target size
    /// count the entries it has added.
    pub fn open_block_len(&self) -> usize {
        if self.block.is_empty() { 0 } else { self.block.payload_len() }
    }

    /// Adds a point entry; `value: None` is a point tombstone.
    pub fn add(&mut self, key: &[u8], seq: u64, value: Option<&[u8]>) {
        debug_assert!(self.first_key.is_none() || key > &self.last_key[..], "keys must increase strictly");
        if self.first_key.is_none() {
            self.first_key = Some(key.to_vec());
        }
        self.block.add(key, seq, value);
        self.hashes.push(bloom::hash(key));
        self.last_key.clear();
        self.last_key.extend_from_slice(key);
        let p = &mut self.props;
        p.point_entries += 1;
        p.raw_key_bytes += key.len() as u64;
        match value {
            Some(v) => p.raw_value_bytes += v.len() as u64,
            None => p.point_deletes += 1,
        }
        p.smallest_seq = p.smallest_seq.min(seq);
        p.largest_seq = p.largest_seq.max(seq);
        if self.block.payload_len() >= self.block_size as usize {
            self.close_block();
        }
    }

    fn close_block(&mut self) {
        let raw = self.block.finish();
        self.props.data_blocks += 1;
        if self.lz4 && self.try_lz4 {
            // Compressed later, with the blocks that follow it.
            self.queued_bytes += (raw.len() + 1 + BLOCK_TRAILER_LEN) as u64;
            self.queue.push((raw, self.last_key.clone()));
            if self.queue.len() >= QUEUE_BLOCKS {
                self.drain();
            }
            return;
        }
        let payload = if self.lz4 { super::block::store_raw_data_payload(&raw) } else { raw };
        self.append_data(&payload, self.last_key.clone());
    }

    fn append_data(&mut self, payload: &[u8], last_key: Vec<u8>) {
        let offset = self.len();
        append_block(&mut self.pending, payload, self.file_number, offset);
        let handle = BlockHandle { offset, len: (payload.len() + BLOCK_TRAILER_LEN) as u64 };
        self.index.push(IndexEntry { last_key, handle });
    }

    /// Compresses the queued data blocks, on several threads when there are many, and appends
    /// them in order: the bytes are those of compressing and appending each block as it closes
    /// (a block's compression depends on its bytes alone; its offset only on the blocks before).
    pub fn drain(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let queue = std::mem::take(&mut self.queue);
        self.queued_bytes = 0;
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        let stored: Vec<Vec<u8>> = if threads == 1 || queue.len() < PARALLEL_MIN_BLOCKS {
            queue.iter().map(|(raw, _)| super::block::store_data_payload(raw)).collect()
        } else {
            let per = queue.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let parts: Vec<_> = queue
                    .chunks(per)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter().map(|(raw, _)| super::block::store_data_payload(raw)).collect::<Vec<_>>()
                        })
                    })
                    .collect();
                parts.into_iter().flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
            })
        };
        for (payload, (_, last_key)) in stored.iter().zip(queue) {
            self.append_data(payload, last_key);
        }
    }

    /// Bytes ready to be appended to the file.
    pub fn take_ready(&mut self) -> Vec<u8> {
        self.taken += self.pending.len() as u64;
        std::mem::take(&mut self.pending)
    }

    fn append_meta(&mut self, payload: &[u8]) -> BlockHandle {
        let offset = self.len();
        append_block(&mut self.pending, payload, self.file_number, offset);
        BlockHandle { offset, len: (payload.len() + BLOCK_TRAILER_LEN) as u64 }
    }

    /// Writes the range block (if `fragments` is not empty; sorted and
    /// disjoint, SST-F1), filter, properties, index and footer. Returns the
    /// remaining bytes, the properties and the file length.
    pub fn finish<B: AsRef<[u8]>>(mut self, fragments: &[Fragment<B>], creation_reason: u8) -> (Vec<u8>, Props, u64) {
        if !self.block.is_empty() {
            self.close_block();
        }
        self.drain();
        let mut footer = Footer::default();
        if !fragments.is_empty() {
            footer.rdel = self.append_meta(&encode_rangedel(fragments));
        }
        footer.filter = self.append_meta(&Filter::build(&self.hashes).encode());
        let mut p = std::mem::take(&mut self.props);
        p.range_fragments = fragments.len() as u64;
        for f in fragments {
            p.smallest_seq = p.smallest_seq.min(f.seq);
            p.largest_seq = p.largest_seq.max(f.seq);
        }
        let (smallest, largest, exclusive) = key_extremes(
            self.first_key.as_deref(),
            self.first_key.as_ref().map(|_| &self.last_key[..]),
            fragments.first().map(|f| f.start.as_ref()),
            fragments.last().map(|f| f.end.as_ref()),
        );
        p.smallest_key = smallest.to_vec();
        p.largest_key = largest.to_vec();
        p.largest_exclusive = exclusive;
        p.creation_reason = creation_reason;
        footer.props = self.append_meta(&p.encode());
        footer.index = self.append_meta(&encode_index(&self.index));
        let encoded = footer.encode(self.file_number);
        self.pending.extend_from_slice(&encoded);
        let len = self.len();
        (std::mem::take(&mut self.pending), p, len)
    }
}

/// The `FileMeta` the MANIFEST records for a table with these properties.
pub fn file_meta(level: u8, number: u64, size: u64, p: &Props) -> FileMeta {
    FileMeta {
        level,
        number,
        size,
        point_entries: p.point_entries,
        range_fragments: p.range_fragments,
        smallest_seq: p.smallest_seq,
        largest_seq: p.largest_seq,
        largest_exclusive: p.largest_exclusive,
        smallest: p.smallest_key.clone(),
        largest: p.largest_key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The entries of data block `i` of a table `SstBuilder::new` made (file number 7).
    fn block_entries(block: &[u8], offset: u64, index: &[IndexEntry], i: usize, props: &Props) -> Vec<Entry> {
        let payload = super::super::block::data_block_payload(block, 7, offset, false).unwrap();
        read_data_block(&payload, index, i, props).unwrap().iter().map(Entry::to_owned_entry).collect()
    }
    use crate::format::props::REASON_FLUSH;
    use crate::format::testing::Recording;

    /// Blocks compressed together (on several threads when there are many) give the bytes of
    /// compressing each block as it closes.
    #[test]
    fn batched_compression_writes_the_bytes_of_one_block_at_a_time() {
        let build = |each: bool| {
            let mut b = SstBuilder::with_compression([2; 16], 9, 512, true);
            let mut out = Vec::new();
            for i in 0..40_000u32 {
                let v = format!("value {} {}", i % 17, "x".repeat((i % 40) as usize));
                b.add(format!("k{i:07}").as_bytes(), 1 + u64::from(i), Some(v.as_bytes()));
                if each {
                    b.drain();
                }
                out.extend(b.take_ready());
            }
            let (rest, props, len) = b.finish(&[] as &[Fragment<Vec<u8>>], REASON_FLUSH);
            out.extend(rest);
            (out, props.data_blocks, len)
        };
        let (batched, blocks, len) = build(false);
        assert!(blocks as usize > 2 * QUEUE_BLOCKS, "several full batches: {blocks}");
        assert_eq!(batched.len() as u64, len);
        assert!(batched == build(true).0, "same bytes");
    }

    #[test]
    fn open_reads_only_header_footer_and_meta_blocks() {
        let mut b = SstBuilder::new([1; 16], 7, 512);
        for i in 0..200u32 {
            b.add(format!("k{i:05}").as_bytes(), 1 + u64::from(i), Some(&[b'v'; 30]));
        }
        let frag = Fragment { start: b"z".to_vec(), end: b"zz".to_vec(), seq: 5 };
        let (file, props, len) = b.finish(&[frag], REASON_FLUSH);
        let meta = file_meta(0, 7, len, &props);
        let ex = SstExpect { file_number: 7, db_id: [1; 16], meta: Some(&meta), last_seq: 1000 };
        let mut src = Recording::new(&file, file.len());
        let opened = open_sst_from(&mut src, &ex).map_err(ReadError::into_format).unwrap();
        assert_eq!(opened, open_sst(&file, &ex).unwrap());
        assert!(opened.index.len() > 5);
        // No read touches the data blocks [96, D).
        for &(from, to) in &src.reads {
            assert!(to <= HEADER_LEN as u64 || from >= opened.data_end, "read {from}..{to} hits the data blocks");
        }
        // Every data block then reads and verifies on its own.
        let mut v = SstVerifier::new(&opened);
        for (i, e) in opened.index.iter().enumerate() {
            let block = &file[e.handle.offset as usize..(e.handle.offset + e.handle.len) as usize];
            for entry in block_entries(block, e.handle.offset, &opened.index, i, &opened.props) {
                v.observe(&entry, e.handle.offset).unwrap();
            }
        }
        v.finish().unwrap();
    }

    fn small_table(block_size: u32) -> (Vec<u8>, Props, u64) {
        let mut b = SstBuilder::new([1; 16], 7, block_size);
        for i in 0..50u32 {
            b.add(format!("k{i:03}").as_bytes(), 1 + u64::from(i), Some(b"value"));
        }
        b.finish(&[] as &[Fragment], REASON_FLUSH)
    }

    #[test]
    fn builder_block_sizes_in_range_are_readable() {
        for block_size in [MIN_BLOCK_SIZE, 4096, MAX_BLOCK_SIZE] {
            let (file, _, _) = small_table(block_size);
            let ex = SstExpect { file_number: 7, db_id: [1; 16], meta: None, last_seq: 1000 };
            read_sst(&file, &ex).unwrap();
        }
    }

    #[test]
    #[should_panic(expected = "HDR-9")]
    fn builder_rejects_block_size_below_512() {
        small_table(MIN_BLOCK_SIZE - 1);
    }

    #[test]
    #[should_panic(expected = "HDR-9")]
    fn builder_rejects_block_size_above_1_mib() {
        small_table(MAX_BLOCK_SIZE + 1);
    }

    #[test]
    fn semantic_errors_carry_the_block_offset() {
        let mut b = SstBuilder::new([1; 16], 7, 512);
        for i in 0..100u32 {
            b.add(format!("k{i:03}").as_bytes(), 1 + u64::from(i), Some(&[b'v'; 20]));
        }
        let frag = Fragment { start: b"z".to_vec(), end: b"zz".to_vec(), seq: 5 };
        let (file, props, len) = b.finish(std::slice::from_ref(&frag), REASON_FLUSH);
        let meta = file_meta(0, 7, len, &props);
        let ex = SstExpect { file_number: 7, db_id: [1; 16], meta: Some(&meta), last_seq: 1000 };
        let m = open_sst(&file, &ex).unwrap();
        // SST-O6 (properties against the MANIFEST): the properties block.
        let other = FileMeta { point_entries: 99, ..meta.clone() };
        let e = open_sst(&file, &SstExpect { meta: Some(&other), ..ex }).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-O6", m.footer.props.offset));
        let e = open_sst(&file, &SstExpect { last_seq: 50, ..ex }).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-O6", m.footer.props.offset));
        // SST-O5 (index): the index block.
        let e = check_index(&m.index, m.data_end + 1, &m.props, m.footer.index.offset).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-O5", m.footer.index.offset));
        // SST-O7 and SST-F1 (fragments): the range block.
        let fewer = Props { range_fragments: 2, ..m.props.clone() };
        let e = check_fragments(&m.fragments, &fewer, m.footer.rdel.offset).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-O7", m.footer.rdel.offset));
        let newer = Props { smallest_seq: 6, ..m.props.clone() };
        let e = check_fragments(&m.fragments, &newer, m.footer.rdel.offset).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-F1", m.footer.rdel.offset));
        // SST-V3 (filter): the data block holding the entry.
        let empty = SstMeta { filter: Filter::build(&[]), ..m.clone() };
        let mut v = SstVerifier::new(&empty);
        let block = m.index[2].handle;
        let entries = block_entries(block_slice(&file, block), block.offset, &m.index, 2, &m.props);
        let e = v.observe(&entries[0], block.offset).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-V3", block.offset));
        // SST-V4 (a newer fragment covers a point): the data block.
        let covering = SstMeta {
            fragments: vec![Fragment { start: b"k000".to_vec(), end: b"k001".to_vec(), seq: 99 }],
            ..m.clone()
        };
        let first = m.index[0].handle;
        let entries = block_entries(block_slice(&file, first), first.offset, &m.index, 0, &m.props);
        let e = SstVerifier::new(&covering).observe(&entries[0], first.offset).unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-V4", first.offset));
        // SST-V1 (counts): the properties block.
        let e = SstVerifier::new(&m).finish().unwrap_err();
        assert_eq!((e.rule, e.offset), ("SST-V1", m.footer.props.offset));
    }
}
