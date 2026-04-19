#![no_main]

use libfuzzer_sys::fuzz_target;
use vardict_rs::mods::cigar_parser::parse_cigar_string;

fuzz_target!(|data: &[u8]| {
    if let Ok(cigar_str) = std::str::from_utf8(data) {
        let _ = parse_cigar_string(cigar_str);
    }
});