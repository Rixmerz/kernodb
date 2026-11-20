#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::coding;

fuzz_target!(|data: &[u8]| {
    let _ = coding::decode_varint(data);
});
