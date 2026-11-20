//! Files built by hand with valid checksums: a table or a MANIFEST whose
//! every CRC verifies but whose contents break a rule. A bit flip cannot make
//! these (it breaks a CRC first), so they are the only way to reach the
//! structural checks (SST-O*, SST-R*, SST-V*, MAN-S*) from outside the codec.
//!
//! `Table::grid` is an honest table; tests then change one field of it
//! (`props`, `filter`, `blocks`, ...) and encode it again, so the CRCs always
//! agree with the lie.

use std::path::PathBuf;

use kerno_storage::format::block::{BLOCK_TRAILER_LEN, Entry, append_block, encode_data_block, reseal_block};
use kerno_storage::format::bloom;
use kerno_storage::format::filter::Filter;
use kerno_storage::format::header::Header;
use kerno_storage::format::manifest::{Edit, FileMeta, Record, encode_manifest};
use kerno_storage::format::props::{Props, REASON_FLUSH};
use kerno_storage::format::rangedel::{Fragment, encode_rangedel};
use kerno_storage::format::sst::{
    BlockHandle, Footer, IndexEntry, decode_footer, encode_index, file_meta, key_extremes,
};
use kerno_testkit::vfs::{DirImage, SimVfs};

use super::{DB_ID, DIR, path};

/// An SST described by its parts; [`Table::encode`] lays them out.
#[derive(Clone)]
pub struct Table {
    pub number: u64,
    /// Entries of each data block.
    pub blocks: Vec<Vec<Entry>>,
    pub props: Props,
    pub filter: Filter,
    pub fragments: Vec<Fragment>,
    /// Payloads written instead of the encoded ones.
    pub raw_blocks: Option<Vec<Vec<u8>>>,
    pub raw_index: Option<Vec<u8>>,
    pub raw_props: Option<Vec<u8>>,
    pub raw_filter: Option<Vec<u8>>,
    pub raw_rdel: Option<Vec<u8>>,
}

/// Where [`Table::encode_layout`] put things.
pub struct Layout {
    pub index: Vec<IndexEntry>,
    pub footer: Footer,
}

impl Table {
    /// `blocks` blocks of `per_block` entries each, keys `{prefix}{i:05}`
    /// increasing across the table, sequences from `seq_base + 1` up, 100-byte
    /// values: a table that satisfies every rule.
    pub fn grid(number: u64, prefix: char, blocks: usize, per_block: usize, seq_base: u64) -> Table {
        let mut seq = seq_base + 1;
        let blocks = (0..blocks)
            .map(|b| {
                (0..per_block)
                    .map(|i| {
                        seq += 1;
                        Entry {
                            key: format!("{prefix}{:05}", b * per_block + i).into_bytes(),
                            seq: seq - 1,
                            value: Some(vec![b'v'; 100]),
                        }
                    })
                    .collect()
            })
            .collect();
        let mut t = Table {
            number,
            blocks,
            props: Props::default(),
            filter: Filter { k: 7, nbits: 64, bits: vec![0; 8] },
            fragments: Vec::new(),
            raw_blocks: None,
            raw_index: None,
            raw_props: None,
            raw_filter: None,
            raw_rdel: None,
        };
        t.refresh();
        t
    }

    /// Recomputes `props` and `filter` from `blocks` and `fragments`, as the
    /// writer would.
    pub fn refresh(&mut self) {
        let points: Vec<&Entry> = self.blocks.iter().flatten().collect();
        let mut p = Props {
            point_entries: points.len() as u64,
            point_deletes: points.iter().filter(|e| e.value.is_none()).count() as u64,
            range_fragments: self.fragments.len() as u64,
            data_blocks: self.blocks.len() as u64,
            raw_key_bytes: points.iter().map(|e| e.key.len() as u64).sum(),
            raw_value_bytes: points.iter().map(|e| e.value.as_ref().map_or(0, |v| v.len() as u64)).sum(),
            creation_reason: REASON_FLUSH,
            ..Props::default()
        };
        let seqs = points.iter().map(|e| e.seq).chain(self.fragments.iter().map(|f| f.seq));
        p.smallest_seq = seqs.clone().min().unwrap_or(0);
        p.largest_seq = seqs.max().unwrap_or(0);
        let (small, large, exclusive) = key_extremes(
            points.first().map(|e| &e.key[..]),
            points.last().map(|e| &e.key[..]),
            self.fragments.first().map(|f| &f.start[..]),
            self.fragments.last().map(|f| &f.end[..]),
        );
        p.smallest_key = small.to_vec();
        p.largest_key = large.to_vec();
        p.largest_exclusive = exclusive;
        let hashes: Vec<u64> = points.iter().map(|e| bloom::hash(&e.key)).collect();
        self.props = p;
        self.filter = Filter::build(&hashes);
    }

    /// The first and last key of the table as its blocks hold them.
    pub fn first_key(&self) -> Vec<u8> {
        self.blocks[0][0].key.clone()
    }

    pub fn encode(&self) -> Vec<u8> {
        self.encode_layout(|_| {}).0
    }

    /// The file, with `index` changed by `tweak` before it is written.
    pub fn encode_layout(&self, tweak: impl FnOnce(&mut Vec<IndexEntry>)) -> (Vec<u8>, Layout) {
        let number = self.number;
        let mut out = Header::sst(DB_ID, number, 4096).encode().to_vec();
        let mut index = Vec::new();
        for (i, entries) in self.blocks.iter().enumerate() {
            let payload = match &self.raw_blocks {
                Some(raw) => raw[i].clone(),
                None => encode_data_block(entries),
            };
            let offset = out.len() as u64;
            append_block(&mut out, &payload, number, offset);
            let last_key = entries.last().expect("a block holds entries").key.clone();
            index.push(IndexEntry {
                last_key,
                handle: BlockHandle { offset, len: (payload.len() + BLOCK_TRAILER_LEN) as u64 },
            });
        }
        tweak(&mut index);
        let meta = |out: &mut Vec<u8>, payload: &[u8]| {
            let offset = out.len() as u64;
            append_block(out, payload, number, offset);
            BlockHandle { offset, len: (payload.len() + BLOCK_TRAILER_LEN) as u64 }
        };
        let mut footer = Footer::default();
        if !self.fragments.is_empty() || self.raw_rdel.is_some() {
            let payload = self.raw_rdel.clone().unwrap_or_else(|| encode_rangedel(&self.fragments));
            footer.rdel = meta(&mut out, &payload);
        }
        footer.filter = meta(&mut out, &self.raw_filter.clone().unwrap_or_else(|| self.filter.encode()));
        footer.props = meta(&mut out, &self.raw_props.clone().unwrap_or_else(|| self.props.encode()));
        footer.index = meta(&mut out, &self.raw_index.clone().unwrap_or_else(|| encode_index(&index)));
        out.extend_from_slice(&footer.encode(number));
        (out, Layout { index, footer })
    }

    /// What the MANIFEST records for this table at `level`, from its own
    /// properties.
    pub fn meta(&self, level: u8) -> FileMeta {
        file_meta(level, self.number, self.encode().len() as u64, &self.props)
    }
}

/// Changes the footer of an encoded table and seals it again.
pub fn patch_footer(file: &mut [u8], number: u64, f: impl FnOnce(&mut Footer)) {
    let at = file.len() - 80;
    let mut footer = decode_footer(&file[at..], number, at as u64).expect("a sealed footer");
    f(&mut footer);
    file[at..].copy_from_slice(&footer.encode(number));
}

/// Changes the payload (not the slot) of the block at `handle` and seals it
/// again.
pub fn patch_block(file: &mut [u8], number: u64, handle: BlockHandle, f: impl FnOnce(&mut [u8])) {
    let block = &mut file[handle.offset as usize..(handle.offset + handle.len) as usize];
    let payload = block.len() - BLOCK_TRAILER_LEN;
    f(&mut block[..payload]);
    reseal_block(block, number, handle.offset);
}

/// A directory made of tables and the MANIFEST that references them.
#[derive(Clone)]
pub struct Dir {
    /// Each table as the MANIFEST records it, and its bytes.
    pub tables: Vec<(FileMeta, Vec<u8>)>,
    pub next_file: u64,
    pub last_seq: u64,
    pub log_number: u64,
    pub clean: bool,
}

impl Dir {
    /// A cleanly closed database holding `tables`; `last_seq` and `next_file`
    /// are the smallest the tables allow.
    pub fn new(tables: Vec<(FileMeta, Vec<u8>)>) -> Dir {
        let next_file = tables.iter().map(|(m, _)| m.number).max().unwrap_or(0) + 1;
        let last_seq = tables.iter().map(|(m, _)| m.largest_seq).max().unwrap_or(0);
        Dir { tables, next_file, last_seq, log_number: next_file, clean: true }
    }

    /// A directory with one table at `level` 0.
    pub fn single(t: &Table) -> Dir {
        Dir::new(vec![(t.meta(0), t.encode())])
    }

    pub fn image(&self) -> DirImage {
        let sim = SimVfs::new(1);
        sim.as_vfs().create_dir_all(std::path::Path::new(DIR)).unwrap();
        let mut image = sim.dump();
        let mut by_level: Vec<&(FileMeta, Vec<u8>)> = self.tables.iter().collect();
        by_level.sort_by_key(|(m, _)| m.level);
        let snapshot = Record {
            edit_seq: 1,
            edit: Edit::Snapshot {
                next_file: self.next_file,
                last_seq: self.last_seq,
                log_number: self.log_number,
                clean: self.clean,
                prev_generation: 0,
                files: by_level.iter().map(|(m, _)| m.clone()).collect(),
            },
            ignored: Vec::new(),
        };
        image.files.insert(path("MANIFEST"), encode_manifest(&Header::manifest(DB_ID, 1, 1), &[snapshot]));
        for (m, bytes) in &self.tables {
            image.files.insert(path(&format!("{:06}.sst", m.number)), bytes.clone());
        }
        image
    }
}

/// The path of table `number` in a crafted directory.
pub fn sst_path(number: u64) -> PathBuf {
    path(&format!("{number:06}.sst"))
}
