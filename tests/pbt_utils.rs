//! Property-based tests (M6) for leaf utility functions and Fisher exact.
//!
//! These tests target mathematically well-defined properties: involutions,
//! bounds, idempotence, and symmetry. They complement parity tests by
//! catching regressions in leaf logic that input-driven parity fixtures
//! may not reach.

use proptest::prelude::*;
use vardict_rs::fisher::FisherExact;
use vardict_rs::utils::{
    char_at, complement_base, complement_sequence, get_reverse_complemented_sequence,
    reverse_sequence, substr_with_len,
};

// ── utils.rs: sequence operations ───────────────────────────────────────────

proptest! {
    #[test]
    fn prop_complement_base_is_involution(b in prop_oneof![
        Just(b'A'), Just(b'C'), Just(b'G'), Just(b'T'),
        Just(b'a'), Just(b'c'), Just(b'g'), Just(b't'),
    ]) {
        prop_assert_eq!(complement_base(complement_base(b)), b);
    }

    #[test]
    fn prop_complement_base_unknown_is_identity(b in 0u8..=255) {
        // VarDict convention: non-ACGT bases are returned unchanged.
        let result = complement_base(b);
        let known = matches!(b, b'A' | b'C' | b'G' | b'T'
                              | b'a' | b'c' | b'g' | b't');
        if !known {
            prop_assert_eq!(result, b);
        }
    }

    #[test]
    fn prop_complement_sequence_is_involution(
        seq in proptest::collection::vec(
            prop_oneof![Just(b'A'), Just(b'C'), Just(b'G'), Just(b'T')],
            0..64,
        ),
    ) {
        let double = complement_sequence(&complement_sequence(&seq));
        prop_assert_eq!(double, seq);
    }

    #[test]
    fn prop_reverse_sequence_is_involution(
        seq in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let double = reverse_sequence(&reverse_sequence(&seq));
        prop_assert_eq!(double, seq);
    }

    #[test]
    fn prop_reverse_complement_is_involution(
        seq in proptest::collection::vec(
            prop_oneof![Just(b'A'), Just(b'C'), Just(b'G'), Just(b'T')],
            1..64,
        ),
    ) {
        let len = seq.len() as i32;
        let once = get_reverse_complemented_sequence(&seq, 0, len);
        let twice = get_reverse_complemented_sequence(&once, 0, len);
        prop_assert_eq!(twice, seq);
    }

    #[test]
    fn prop_substr_with_len_respects_length(
        (s, begin, len) in proptest::collection::vec(any::<u8>(), 1..32)
            .prop_flat_map(|s| {
                let slen = s.len() as i32;
                // Java callers pass begin in [0, len] and positive len.
                (Just(s), 0i32..=slen, 1i32..=slen)
            }),
    ) {
        let out = substr_with_len(&s, begin, len);
        prop_assert!(out.len() <= len as usize);
        prop_assert!(out.len() <= s.len());
    }

    #[test]
    fn prop_char_at_valid_range(
        s in proptest::collection::vec(any::<u8>(), 1..32),
    ) {
        let slen = s.len() as i32;
        // Valid Java semantics: idx in [-slen, slen-1].
        for idx in -slen..slen {
            let result = char_at(&s, idx).expect("valid idx must yield Some");
            prop_assert!(s.contains(&result));
        }
    }
}

// ── fisher.rs: Fisher exact test ────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_fisher_pvalue_in_unit_interval(
        ref_fwd in 0i32..50,
        ref_rev in 0i32..50,
        alt_fwd in 0i32..50,
        alt_rev in 0i32..50,
    ) {
        // Skip fully degenerate tables (zero totals), which are undefined in Java too.
        prop_assume!(ref_fwd + ref_rev + alt_fwd + alt_rev > 0);
        let f = FisherExact::new(ref_fwd, ref_rev, alt_fwd, alt_rev);
        let p = f.get_p_value();
        prop_assert!(p.is_finite(), "p-value must be finite: got {p}");
        prop_assert!((0.0..=1.0).contains(&p), "p-value out of [0,1]: {p}");
        let pg = f.get_p_value_greater();
        let pl = f.get_p_value_less();
        prop_assert!((0.0..=1.0).contains(&pg));
        prop_assert!((0.0..=1.0).contains(&pl));
    }

    #[test]
    fn prop_fisher_row_swap_preserves_two_sided_pvalue(
        ref_fwd in 0i32..40,
        ref_rev in 0i32..40,
        alt_fwd in 0i32..40,
        alt_rev in 0i32..40,
    ) {
        prop_assume!(ref_fwd + ref_rev + alt_fwd + alt_rev > 0);
        // Fisher's exact two-sided p-value is invariant under
        // simultaneous row swap in a 2x2 contingency table.
        let a = FisherExact::new(ref_fwd, ref_rev, alt_fwd, alt_rev).get_p_value();
        let b = FisherExact::new(alt_fwd, alt_rev, ref_fwd, ref_rev).get_p_value();
        prop_assert!((a - b).abs() < 1e-6, "two-sided p should be row-swap invariant: {a} vs {b}");
    }
}
