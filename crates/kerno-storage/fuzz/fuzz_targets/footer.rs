#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::sst;

fuzz_target!(|data: &[u8]| {
    let _ = sst::decode_footer(data, 5, 0);
});
