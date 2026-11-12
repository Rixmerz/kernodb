#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::sst::{self, SstExpect};

const DB: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

// File 42 with no FileMeta to cross-check, as the SST vectors.
fuzz_target!(|data: &[u8]| {
    let ex = SstExpect { file_number: 42, db_id: DB, meta: None, last_seq: u64::MAX >> 1 };
    let _ = sst::read_sst(data, &ex);
});
