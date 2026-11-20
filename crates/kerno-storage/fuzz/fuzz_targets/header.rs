#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::header::{self, FileKind, HeaderExpect};

const DB: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

// Every file kind, with and without the expectations of the vectors.
fuzz_target!(|data: &[u8]| {
    for kind in [FileKind::Manifest, FileKind::Wal, FileKind::Sst] {
        for expect in
            [HeaderExpect { db_id: None, file_number: None }, HeaderExpect { db_id: Some(DB), file_number: Some(7) }]
        {
            let _ = header::decode_header(data, data.len() as u64, kind, &expect);
        }
    }
});
