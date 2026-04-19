#![no_main]

use libfuzzer_sys::fuzz_target;
use vardict_rs::utils::substr_with_len;

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }

    let begin = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let len = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let slice = &data[8..];
    let length = i64::try_from(slice.len()).expect("slice length fits in i64");
    let adjusted_begin = if begin < 0 {
        length + i64::from(begin)
    } else {
        i64::from(begin)
    };

    if len > 0 && !(0..=length).contains(&adjusted_begin) {
        return;
    }

    if len > 0 && adjusted_begin + i64::from(len) > i64::from(i32::MAX) {
        return;
    }

    if len < 0 && adjusted_begin < 0 {
        return;
    }

    if len < 0 && length + i64::from(len) < i64::from(i32::MIN) {
        return;
    }

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = substr_with_len(slice, begin, len);
    }));
});