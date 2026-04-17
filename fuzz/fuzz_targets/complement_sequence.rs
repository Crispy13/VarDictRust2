#![no_main]

use libfuzzer_sys::fuzz_target;
use vardict_rs::utils::complement_sequence;

fuzz_target!(|data: &[u8]| {
    // Property: complement is an involution on ACGT; non-ACGT bytes pass through.
    // Must not panic on any input.
    let once = complement_sequence(data);
    let twice = complement_sequence(&once);
    // For pure-ACGT inputs twice == data; always, len is preserved.
    assert_eq!(twice.len(), data.len());
});
