use std::fmt::Display;

use once_cell::sync::Lazy;
use regex::Regex;

// Java: Utils.getOrElse() L81-L87
// Rust idiom: `map.entry(key).or_insert(default)`
//
// Java: Utils.toInt() L89-L91
// Rust idiom: `s.parse::<i32>().unwrap()`
//
// Java: Utils.sum() L188-L194
// Rust idiom: `.iter().sum::<i32>()`

// TODO(S08/S12): printExceptionAndContinue depends on Configuration.MAX_EXCEPTION_COUNT
// + GlobalReadOnlyScope. Defer until S08 (Configuration) and S12 (Scope) are ported.
// Java: Utils.printExceptionAndContinue() L256-L279

static TRAILING_ZERO_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"0+$").unwrap());

fn decimal_places(pattern: &str) -> usize {
    pattern
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
}

pub(crate) fn format_half_even(pattern: &str, value: f64) -> String {
    let decimals = decimal_places(pattern);
    if let Some(value) = half_even_special_zero(decimals, value) {
        return format!("{:.*}", decimals, value);
    }
    format!("{:.*}", decimals, value)
}

/// Shared branch of `format_half_even`: when `decimals >= 3` and the magnitude lands exactly
/// on the `0.5` ulp boundary at this scale, the formatted value is a sign-preserving zero
/// rather than `value` itself. Returns `Some(signed_zero)` in that case, `None` otherwise, so
/// both `format_half_even` and `round_half_even` apply identical rounding semantics.
#[inline]
fn half_even_special_zero(decimals: usize, value: f64) -> Option<f64> {
    let scale = 10_f64.powi(decimals as i32);
    if decimals >= 3 && value.abs() * scale == 0.5 {
        Some(0.0_f64.copysign(value))
    } else {
        None
    }
}

/// Fixed-capacity stack buffer implementing `fmt::Write`, used to format a number without the
/// heap `String` that `format!` allocates. `write_str` only ever appends valid UTF-8, so
/// `as_str` is sound. Overflow (capacity exceeded) returns `fmt::Error`, letting callers fall
/// back to a heap `format!` for the rare oversized value.
struct StackFmtBuf {
    buf: [u8; 64],
    len: usize,
}

impl StackFmtBuf {
    #[inline]
    fn new() -> Self {
        Self { buf: [0; 64], len: 0 }
    }

    #[inline]
    fn as_str(&self) -> &str {
        // Safety: every byte in `buf[..len]` was copied from a `&str` in `write_str`.
        unsafe { std::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }
}

impl std::fmt::Write for StackFmtBuf {
    #[inline]
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let bytes = s.as_bytes();
        let end = self.len + bytes.len();
        if end > self.buf.len() {
            return Err(std::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

pub(crate) fn zero_gated_format(pattern: &str, value: f64) -> String {
    if value == 0.0 {
        String::from("0")
    } else {
        format_half_even(pattern, value)
    }
}

pub(crate) fn nm_non_fisher_format(value: f64) -> String {
    if value > 0.0 {
        format_half_even("0.0", value)
    } else {
        String::from("0")
    }
}

pub(crate) fn nm_fisher_format(value: f64) -> String {
    let clamped = value.max(0.0);
    if clamped == 0.0 {
        String::from("0")
    } else {
        format_half_even("0.0", clamped)
    }
}

pub(crate) fn hifreq_fisher_format(value: f64) -> String {
    zero_gated_format("0.0000", value)
}

macro_rules! tsv_join {
    ($delim:expr $(, $value:expr)+ $(,)?) => {{
        let delimiter = $delim;
        let mut output = String::new();
        let mut first = true;
        $(
            if !first {
                output.push_str(delimiter);
            } else {
                first = false;
            }
            {
                use std::fmt::Write as _;
                write!(&mut output, "{}", $value).expect("write to String must succeed");
            }
        )+
        output
    }};
}

pub(crate) use tsv_join;

// Java: Utils.complement(char) L243-L247
pub fn complement_base(b: u8) -> u8 {
    match b {
        b'A' => b'T',
        b'T' => b'A',
        b'C' => b'G',
        b'G' => b'C',
        b'a' => b't',
        b't' => b'a',
        b'c' => b'g',
        b'g' => b'c',
        _ => b,
    }
}

// Java: Utils.complement(byte[]) L236-L241
pub fn complement_bases(seq: &mut [u8]) {
    for base in seq {
        *base = complement_base(*base);
    }
}

// Java: Utils.complement(String) L230-L234
pub fn complement_sequence(seq: &[u8]) -> Vec<u8> {
    seq.iter().map(|&base| complement_base(base)).collect()
}

// Java: Utils.reverse(String) L226-L228
pub fn reverse_sequence(seq: &[u8]) -> Vec<u8> {
    seq.iter().rev().copied().collect()
}

// Java: Utils.substr(String, int) L117-L123
pub fn substr(s: &[u8], idx: i32) -> Vec<u8> {
    let length = i32::try_from(s.len()).expect("slice length exceeds i32");
    let begin = if idx >= 0 {
        idx.min(length)
    } else {
        (length + idx).max(0)
    };

    s[usize::try_from(begin).expect("negative begin")..].to_vec()
}

// Java: Utils.substr(String, int, int) L132-L148
pub fn substr_with_len(s: &[u8], begin: i32, len: i32) -> Vec<u8> {
    let length = i32::try_from(s.len()).expect("slice length exceeds i32");
    let begin = if begin < 0 { length + begin } else { begin };

    if len > 0 {
        let end = (begin + len).min(length);
        let begin = usize::try_from(begin).expect("negative begin");
        let end = usize::try_from(end).expect("negative end");
        return s[begin..end].to_vec();
    }

    if len == 0 {
        return Vec::new();
    }

    let end = length + len;
    if end < begin {
        return Vec::new();
    }

    let begin = usize::try_from(begin).expect("negative begin");
    let end = usize::try_from(end).expect("negative end");
    s[begin..end].to_vec()
}

// Java: Utils.charAt(String, int) L155-L164
pub fn char_at(s: &[u8], index: i32) -> Option<u8> {
    let length = i32::try_from(s.len()).expect("slice length exceeds i32");
    if index < 0 {
        let adjusted = length + index;
        if adjusted < 0 {
            return None;
        }
        return Some(s[usize::try_from(adjusted).expect("negative index")]);
    }

    Some(s[usize::try_from(index).expect("negative index")])
}

// Java: Utils.roundHalfEven() L99-L103
pub fn round_half_even(pattern: &str, value: f64) -> f64 {
    use std::fmt::Write as _;

    // Equivalent to `format_half_even(pattern, value).parse()`, but formats into a stack
    // buffer to avoid the transient heap `String` that `format!` allocates on this hot path
    // (round_half_even is invoked many times per emitted variant). The formatted bytes — and
    // thus the parsed value — are identical to `format_half_even`'s, since both apply the same
    // `decimals` precision and the same `half_even_special_zero` branch.
    let decimals = decimal_places(pattern);
    let value = half_even_special_zero(decimals, value).unwrap_or(value);

    let mut buf = StackFmtBuf::new();
    let formatted = if write!(buf, "{:.*}", decimals, value).is_ok() {
        buf.as_str().parse::<f64>()
    } else {
        // Oversized output (huge magnitude/precision) — fall back to a heap format.
        format!("{:.*}", decimals, value).parse::<f64>()
    };
    formatted.expect("formatted half-even value must parse")
}

// Java: Utils.getRoundedValueToPrint() L105-L109
pub fn get_rounded_value_to_print(pattern: &str, value: f64) -> String {
    if value == (value as i64) as f64 {
        return format_half_even("0", value);
    }

    TRAILING_ZERO_RE
        .replace_all(&format_half_even(pattern, value), "")
        .into_owned()
}

// Java: Utils.join() L39-L50
pub fn join(delim: &str, args: &[Option<&str>]) -> String {
    args.iter()
        .map(|arg| arg.unwrap_or("null"))
        .collect::<Vec<_>>()
        .join(delim)
}

// Java: Utils.joinNotNull() L58-L79
pub fn join_not_null(delim: &str, args: &[Option<&str>]) -> String {
    if args.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    for (index, arg) in args.iter().enumerate() {
        if let Some(value) = arg {
            result.push_str(value);
            if index + 1 != args.len() && args[index + 1].is_some() {
                result.push_str(delim);
            }
        } else if index + 1 != args.len() && args[index + 1].is_some() {
            result.push_str(delim);
        }
    }
    result
}

// Java: Utils.toString(Collection) L19-L31
pub fn to_string_collection<T: Display>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

// Java: Utils.globalFind() L201-L209
pub fn global_find(pattern: &Regex, text: &str) -> Vec<String> {
    pattern
        .captures_iter(text)
        .map(|captures| {
            captures
                .get(1)
                .expect("pattern must contain capture group 1")
                .as_str()
                .to_string()
        })
        .collect()
}

// Java: Utils.getReverseComplementedSequence() L217-L224
pub fn get_reverse_complemented_sequence(bases: &[u8], start_index: i32, length: i32) -> Vec<u8> {
    let read_length = i32::try_from(bases.len()).expect("slice length exceeds i32");
    let start_index = if start_index < 0 {
        read_length + start_index
    } else {
        start_index
    };
    let end_index = start_index + length;
    let start_index = usize::try_from(start_index).expect("negative start index");
    let end_index = usize::try_from(end_index).expect("negative end index");
    let mut range = bases[start_index..end_index].to_vec();
    range.reverse();
    complement_bases(&mut range);
    range
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complement_base_upper_a_to_t() {
        assert_eq!(complement_base(b'A'), b'T');
    }

    #[test]
    fn complement_base_upper_t_to_a() {
        assert_eq!(complement_base(b'T'), b'A');
    }

    #[test]
    fn complement_base_upper_c_to_g() {
        assert_eq!(complement_base(b'C'), b'G');
    }

    #[test]
    fn complement_base_upper_g_to_c() {
        assert_eq!(complement_base(b'G'), b'C');
    }

    #[test]
    fn complement_base_lower_a_to_t() {
        assert_eq!(complement_base(b'a'), b't');
    }

    #[test]
    fn complement_base_lower_t_to_a() {
        assert_eq!(complement_base(b't'), b'a');
    }

    #[test]
    fn complement_base_passes_through_n() {
        assert_eq!(complement_base(b'N'), b'N');
    }

    #[test]
    fn complement_base_passes_through_digit() {
        assert_eq!(complement_base(b'5'), b'5');
    }

    #[test]
    fn complement_base_passes_through_symbol() {
        assert_eq!(complement_base(b'-'), b'-');
    }

    #[test]
    fn complement_bases_mutates_in_place() {
        let mut seq = b"AaTtCcGgN5".to_vec();
        complement_bases(&mut seq);
        assert_eq!(seq, b"TtAaGgCcN5");
    }

    #[test]
    fn complement_bases_handles_empty_slice() {
        let mut seq = Vec::new();
        complement_bases(&mut seq);
        assert!(seq.is_empty());
    }

    #[test]
    fn complement_sequence_returns_new_vec() {
        let seq = b"ACGTNx";
        assert_eq!(complement_sequence(seq), b"TGCANx");
        assert_eq!(seq, b"ACGTNx");
    }

    #[test]
    fn complement_sequence_handles_lowercase() {
        assert_eq!(complement_sequence(b"acgt"), b"tgca");
    }

    #[test]
    fn reverse_sequence_reverses_ascii_bytes() {
        assert_eq!(reverse_sequence(b"ABCDE"), b"EDCBA");
    }

    #[test]
    fn reverse_sequence_handles_empty_input() {
        assert_eq!(reverse_sequence(b""), b"");
    }

    #[test]
    fn substr_positive_index() {
        assert_eq!(substr(b"ABCDE", 2), b"CDE");
    }

    #[test]
    fn substr_positive_index_clamps_to_end() {
        assert_eq!(substr(b"ABCDE", 10), b"");
    }

    #[test]
    fn substr_negative_index_counts_from_end() {
        assert_eq!(substr(b"ABCDE", -2), b"DE");
    }

    #[test]
    fn substr_negative_index_clamps_to_zero() {
        assert_eq!(substr(b"ABCDE", -10), b"ABCDE");
    }

    #[test]
    fn substr_empty_zero_index() {
        assert_eq!(substr(b"", 0), b"");
    }

    #[test]
    fn substr_empty_negative_index() {
        assert_eq!(substr(b"", -1), b"");
    }

    #[test]
    fn substr_with_len_positive_length() {
        assert_eq!(substr_with_len(b"ABCDE", 1, 3), b"BCD");
    }

    #[test]
    fn substr_with_len_clamps_end() {
        assert_eq!(substr_with_len(b"ABCDE", 0, 10), b"ABCDE");
    }

    #[test]
    fn substr_with_len_negative_begin_counts_from_end() {
        assert_eq!(substr_with_len(b"ABCDE", -2, 1), b"D");
    }

    #[test]
    fn substr_with_len_negative_begin_two_chars() {
        assert_eq!(substr_with_len(b"ABCDE", -2, 2), b"DE");
    }

    #[test]
    fn substr_with_len_negative_length_uses_end_offset() {
        assert_eq!(substr_with_len(b"ABCDE", 1, -1), b"BCD");
    }

    #[test]
    fn substr_with_len_negative_length_single_char() {
        assert_eq!(substr_with_len(b"ABCDE", 3, -1), b"D");
    }

    #[test]
    fn substr_with_len_zero_length_is_empty() {
        assert_eq!(substr_with_len(b"ABCDE", 1, 0), b"");
    }

    #[test]
    #[should_panic]
    fn substr_with_len_negative_begin_overflow_panics() {
        let _ = substr_with_len(b"ABCDE", -10, 2);
    }

    #[test]
    fn char_at_zero_index() {
        assert_eq!(char_at(b"ABCDE", 0), Some(b'A'));
    }

    #[test]
    fn char_at_negative_one() {
        assert_eq!(char_at(b"ABCDE", -1), Some(b'E'));
    }

    #[test]
    fn char_at_negative_length() {
        assert_eq!(char_at(b"ABCDE", -5), Some(b'A'));
    }

    #[test]
    fn char_at_negative_overflow_returns_none() {
        assert_eq!(char_at(b"ABCDE", -6), None);
    }

    #[test]
    #[should_panic]
    fn char_at_positive_oob_panics() {
        let _ = char_at(b"ABCDE", 5);
    }

    #[test]
    fn round_half_even_tie_down_to_even() {
        assert_eq!(round_half_even("0.0", 2.25), 2.2);
    }

    #[test]
    fn round_half_even_tie_up_to_even() {
        assert_eq!(round_half_even("0.0", 2.35), 2.4);
    }

    #[test]
    fn round_half_even_245_rounds_up_due_to_double_precision() {
        assert_eq!(round_half_even("0.0", 2.45), 2.5);
    }

    #[test]
    fn round_half_even_second_tie_up_to_even() {
        assert_eq!(round_half_even("0.0", 2.55), 2.5);
    }

    #[test]
    fn round_half_even_midpoint_above_rounds_up() {
        assert_eq!(round_half_even("0.0", 0.05), 0.1);
    }

    #[test]
    fn round_half_even_three_decimals() {
        assert_eq!(round_half_even("0.000", 1.2345), 1.234);
    }

    #[test]
    fn round_half_even_five_decimals() {
        assert_eq!(round_half_even("0.00000", 0.123456), 0.12346);
    }

    #[test]
    fn round_half_even_small_midpoint_rounds_to_zero_like_java() {
        assert_eq!(round_half_even("0.0000", 0.00005), 0.0);
    }

    #[test]
    fn get_rounded_value_to_print_zero_whole_number() {
        assert_eq!(get_rounded_value_to_print("0.0", 0.0), "0");
    }

    #[test]
    fn get_rounded_value_to_print_integer_whole_number() {
        assert_eq!(get_rounded_value_to_print("0.0", 1.0), "1");
    }

    #[test]
    fn get_rounded_value_to_print_strips_trailing_zeroes() {
        assert_eq!(get_rounded_value_to_print("0.0000", 0.1230), "0.123");
    }

    #[test]
    fn get_rounded_value_to_print_strips_to_single_decimal() {
        assert_eq!(get_rounded_value_to_print("0.0000", 0.1000), "0.1");
    }

    #[test]
    fn get_rounded_value_to_print_keeps_fractional_value() {
        assert_eq!(get_rounded_value_to_print("0.0", 1.5), "1.5");
    }

    #[test]
    fn get_rounded_value_to_print_keeps_trailing_dot_edge_case() {
        assert_eq!(get_rounded_value_to_print("0.0", 1.0000001), "1.");
    }

    #[test]
    fn join_basic_values() {
        assert_eq!(join("-", &[Some("A"), Some("B"), Some("C")]), "A-B-C");
    }

    #[test]
    fn join_empty_args() {
        assert_eq!(join("-", &[]), "");
    }

    #[test]
    fn join_converts_none_to_literal_null() {
        assert_eq!(join("-", &[Some("A"), None, Some("C")]), "A-null-C");
    }

    #[test]
    fn join_not_null_leading_delimiter_from_null_first() {
        assert_eq!(
            join_not_null("-", &[None, Some("A"), None, Some("B")]),
            "-A-B"
        );
    }

    #[test]
    fn join_not_null_basic_values() {
        assert_eq!(join_not_null("-", &[Some("A"), Some("B")]), "A-B");
    }

    #[test]
    fn join_not_null_consecutive_nulls_follow_java_logic() {
        assert_eq!(join_not_null("-", &[None, None, Some("A")]), "-A");
    }

    #[test]
    fn join_not_null_empty_args() {
        assert_eq!(join_not_null("-", &[]), "");
    }

    #[test]
    fn to_string_collection_multiple_items() {
        assert_eq!(to_string_collection(&[1, 2, 3]), "1 2 3");
    }

    #[test]
    fn to_string_collection_empty_items() {
        let items: [i32; 0] = [];
        assert_eq!(to_string_collection(&items), "");
    }

    #[test]
    fn to_string_collection_single_item() {
        assert_eq!(to_string_collection(&[42]), "42");
    }

    #[test]
    fn to_string_collection_strings() {
        assert_eq!(to_string_collection(&["A", "BC"]), "A BC");
    }

    #[test]
    fn global_find_collects_all_group_one_matches() {
        let pattern = Regex::new(r"(\d+)").unwrap();
        assert_eq!(global_find(&pattern, "abc123def456"), vec!["123", "456"]);
    }

    #[test]
    fn global_find_returns_empty_when_no_matches() {
        let pattern = Regex::new(r"(\d+)").unwrap();
        assert!(global_find(&pattern, "abcdef").is_empty());
    }

    #[test]
    fn global_find_supports_non_digit_capture() {
        let pattern = Regex::new(r"([ACGT]+)").unwrap();
        assert_eq!(global_find(&pattern, "NNACGTxxTT"), vec!["ACGT", "TT"]);
    }

    #[test]
    fn reverse_complemented_sequence_from_start() {
        assert_eq!(
            get_reverse_complemented_sequence(b"AACCGGTT", 2, 4),
            b"CCGG"
        );
    }

    #[test]
    fn reverse_complemented_sequence_negative_start_from_end() {
        assert_eq!(
            get_reverse_complemented_sequence(b"AACCGGTT", -4, 4),
            b"AACC"
        );
    }

    #[test]
    fn reverse_complemented_sequence_preserves_case_and_pass_through() {
        assert_eq!(get_reverse_complemented_sequence(b"AaN-", 0, 4), b"-NtT");
    }

    #[test]
    fn reverse_complemented_sequence_empty_length() {
        assert_eq!(get_reverse_complemented_sequence(b"AACCGGTT", 3, 0), b"");
    }

    mod pbt {
        use super::*;
        use proptest::prelude::*;
        use proptest::test_runner::Config as ProptestConfig;

        fn arb_finite_f64() -> impl Strategy<Value = f64> {
            prop_oneof![
                proptest::num::f64::NORMAL.prop_filter("finite", |value| value.is_finite()),
                Just(0.0),
                Just(-0.0),
                Just(f64::MIN_POSITIVE),
                Just(5e-324_f64),
                Just(f64::MAX),
                Just(f64::MIN),
                (-1e15_f64..1e15_f64),
            ]
        }

        fn arb_negative_f64() -> impl Strategy<Value = f64> {
            prop_oneof![
                Just(f64::MIN),
                Just(-f64::MIN_POSITIVE),
                Just(-5e-324_f64),
                (-1e15_f64..-f64::MIN_POSITIVE)
            ]
        }

        fn arb_pattern() -> impl Strategy<Value = &'static str> {
            prop_oneof![
                Just("0"),
                Just("0.0"),
                Just("0.00"),
                Just("0.000"),
                Just("0.0000"),
                Just("0.00000"),
            ]
        }

        fn arb_non_empty_string() -> impl Strategy<Value = String> {
            proptest::collection::vec(any::<char>(), 1..100)
                .prop_map(|chars| chars.into_iter().collect())
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 256,
                ..ProptestConfig::default()
            })]

            #[test]
            fn pbt_format_half_even_output_parseable_as_f64(
                pattern in arb_pattern(),
                value in arb_finite_f64(),
            ) {
                prop_assert!(format_half_even(pattern, value).parse::<f64>().is_ok());
            }

            #[test]
            fn pbt_round_half_even_is_idempotent(
                pattern in arb_pattern(),
                value in arb_finite_f64(),
            ) {
                let rounded_once = round_half_even(pattern, value);
                let rounded_twice = round_half_even(pattern, rounded_once);

                prop_assert_eq!(rounded_twice, rounded_once);
            }

            #[test]
            fn pbt_get_rounded_value_to_print_no_trailing_zeros(
                pattern in arb_pattern(),
                value in arb_finite_f64(),
            ) {
                let rounded = get_rounded_value_to_print(pattern, value);

                prop_assert!(
                    !rounded.contains('.') || !rounded.ends_with('0'),
                    "rounded value retained trailing zero: {rounded}"
                );
            }

            #[test]
            fn pbt_zero_gated_format_zero_returns_zero(pattern in arb_pattern()) {
                prop_assert_eq!(zero_gated_format(pattern, 0.0), "0");
                prop_assert_eq!(zero_gated_format(pattern, -0.0), "0");
            }

            #[test]
            fn pbt_nm_fisher_format_negative_returns_zero(value in arb_negative_f64()) {
                prop_assert_eq!(nm_fisher_format(value), "0");
            }

            #[test]
            fn pbt_complement_base_involution(base in any::<u8>()) {
                prop_assert_eq!(complement_base(complement_base(base)), base);
            }

            #[test]
            fn pbt_reverse_sequence_involution(seq in proptest::collection::vec(any::<u8>(), 0..100)) {
                prop_assert_eq!(reverse_sequence(&reverse_sequence(&seq)), seq);
            }

            #[test]
            fn pbt_complement_sequence_preserves_length(seq in proptest::collection::vec(any::<u8>(), 0..100)) {
                prop_assert_eq!(complement_sequence(&seq).len(), seq.len());
            }

            #[test]
            fn pbt_substr_zero_is_identity(input in arb_non_empty_string()) {
                let expected = input.as_bytes().to_vec();
                prop_assert_eq!(substr(input.as_bytes(), 0), expected);
            }
        }
    }
}
