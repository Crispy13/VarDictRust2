#![no_main]

use libfuzzer_sys::fuzz_target;
use vardict_rs::fisher::FisherExact;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    // Map 4 bytes to small nonneg counts in [0, 63] to keep runtime bounded.
    let rf = (data[0] & 0x3F) as i32;
    let rr = (data[1] & 0x3F) as i32;
    let af = (data[2] & 0x3F) as i32;
    let ar = (data[3] & 0x3F) as i32;
    if rf + rr + af + ar == 0 {
        return;
    }

    let f = FisherExact::new(rf, rr, af, ar);
    let p = f.get_p_value();
    assert!(p.is_finite() && (0.0..=1.0).contains(&p), "p={p}");
    let pg = f.get_p_value_greater();
    let pl = f.get_p_value_less();
    assert!((0.0..=1.0).contains(&pg));
    assert!((0.0..=1.0).contains(&pl));
});
