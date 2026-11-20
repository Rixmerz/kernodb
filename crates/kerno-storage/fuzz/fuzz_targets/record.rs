#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::manifest;

// Generation 1, offset 96, expected edit sequence 1 (a vector context).
fuzz_target!(|data: &[u8]| {
    let _ = manifest::decode_record(data, 1, 96, 1);
});
