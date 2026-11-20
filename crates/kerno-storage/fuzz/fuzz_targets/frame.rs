#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::wal;

// File 1, offset 96 and expected sequence 1, as the frame vectors.
fuzz_target!(|data: &[u8]| {
    for slot in [false, true] {
        let _ = wal::decode_frame(data, 1, 96, slot, 1);
    }
});
