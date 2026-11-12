#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::wal::{self, WalScanParams};

const DB: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

// File 3, expected sequence 11, as the WAL vectors.
fuzz_target!(|data: &[u8]| {
    let params = WalScanParams { file_number: 3, db_id: DB, expected_seq: 11 };
    let _ = wal::scan_wal(&mut &data[..], &params, |_, _| Ok(()));
});
