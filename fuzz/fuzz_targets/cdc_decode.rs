#![no_main]

use libfuzzer_sys::fuzz_target;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_storage::CdcEntry;

fuzz_target!(|data: &[u8]| {
    let Ok(entry) = decode_cbor::<CdcEntry>(data) else {
        return;
    };

    // Round-trip canonicalization should not panic for decoded entries.
    let _ = encode_cbor(&entry);
});

