#![no_main]

use libfuzzer_sys::fuzz_target;
use vardict_rs::utils::substr_with_len;

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }
    // Pack two small i32 coordinates into the first 4 bytes, use the rest as input.
    let begin = i32::from(data[0] as i8);
    let len = i32::from(data[1] as i8);
    let payload = &data[2..];

    // We only feed valid-range inputs; the function is a Java parity port and
    // panics on clearly out-of-range inputs (same as Java's String.substring).
    let slen = payload.len() as i32;
    if begin < 0 || begin > slen {
        return;
    }
    if len < 0 || begin + len > slen {
        return;
    }

    let out = substr_with_len(payload, begin, len);
    assert!(out.len() <= len as usize);
    assert!(out.len() <= payload.len());
});
