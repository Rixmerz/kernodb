#![no_main]
use libfuzzer_sys::fuzz_target;

use kerno_storage::format::manifest;

fuzz_target!(|data: &[u8]| {
    let _ = manifest::replay_manifest(&mut &data[..], |_, _| Ok(()));
});
