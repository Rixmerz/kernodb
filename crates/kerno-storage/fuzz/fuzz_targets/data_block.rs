#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::block;

fuzz_target!(|data: &[u8]| {
    let _ = block::decode_data_block(data, 5, 96);
});
