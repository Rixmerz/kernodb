#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::block;
use kerno_storage::format::limits::MAX_META_PAYLOAD;
use kerno_storage::format::rangedel;

// File 5 at offset 1500, as the vectors.
fuzz_target!(|data: &[u8]| {
    if let Ok(p) = block::open_block(data, 5, 1500, MAX_META_PAYLOAD, "SST-O4") {
        let _ = rangedel::decode_rangedel(p, 1500);
    }
});
