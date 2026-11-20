#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::{coding, wal};

fuzz_target!(|data: &[u8]| {
    let mut r = coding::Reader::new(data, 0, "WAL-C4");
    if wal::decode_commit(&mut r).is_ok() {
        let _ = r.finish();
    }
});
