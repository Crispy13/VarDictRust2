#![no_main]

use libfuzzer_sys::fuzz_target;
use vardict_rs::config::BamNames;

fuzz_target!(|data: &[u8]| {
    let value = String::from_utf8_lossy(data);
    let _ = BamNames::new(value.into_owned());
});