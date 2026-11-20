#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::block;
use kerno_storage::format::filter;
use kerno_storage::format::limits::MAX_META_PAYLOAD;

// File 5 at offset 2000, as the vectors.
fuzz_target!(|data: &[u8]| {
    if let Ok(p) = block::open_block(data, 5, 2000, MAX_META_PAYLOAD, "SST-O4") {
        let _ = filter::decode_filter(p, 2000);
    }
});
