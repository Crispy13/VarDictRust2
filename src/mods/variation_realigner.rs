//! Ported from: `VarDictJava/src/main/java/com/astrazeneca/vardict/modules/VariationRealigner.java`
//!
//! VariationRealigner is the second pipeline stage after CigarParser.
//! It takes raw variant maps and performs local realignment: SV filtering,
//! MNP adjustment, short indel absorption, and large indel discovery from soft clips.
//!
//! Cluster A: Pure utility functions with no inter-cluster dependencies.
//! All other clusters call into these.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rust_htslib::bam::{self, Read as BamRead};

use crate::config::{EXTENSION, MINSVCDIST, SEED_1, SVFLANK, SVMAXLEN};
use crate::data::{
    BaseInsertion, Cluster, CurrentSegment, InitialData, Match35, RealignedVariationData, Region,
    Sclip, SortPositionSclip, Variation, VariationData, VariationMap, VariationMapSV,
};
use crate::patterns::{
    AMP_ATGC, ATGSs_AMP_ATGSs_END, BEGIN_MINUS_NUMBER, BEGIN_MINUS_NUMBER_ANY, BEGIN_PLUS_ATGC,
    CARET_ATGC_END, CARET_ATGNC, DUP_NUM_ATGC, HASH_ATGC, MINUS_NUMBER_AMP_ATGCs_END,
    MINUS_NUMBER_ATGNC_SV_ATGNC_END, UP_NUMBER_END,
};
use crate::reference::{Reference, ReferenceResource};
use crate::scope::{GlobalReadOnlyScope, Scope, VariantPrinter};
use crate::utils::{char_at, substr, substr_with_len};
use crate::variations::{
    adj_cnt, adj_cnt_with_reference, correct_cnt, find_conseq, get_dir, get_variation,
    get_variation_maybe, get_variation_maybe_mut, inc_cnt, is_equals, is_has_and_equals_base,
    is_has_and_not_equals_base, is_not_equals, join_ref, join_ref_f64, join_ref_for_3_lgins,
    join_ref_for_5_lgins, sub_dir,
};

thread_local! {
    static NO_PASSING_READERS: RefCell<HashMap<String, bam::IndexedReader>> =
        RefCell::new(HashMap::new());
}

// ─── Supporting types ────────────────────────────────────────────────

/// Ported from: VariationRealigner.SortPositionDescription (inner class)
/// Java: VariationRealigner.java#L2190-L2207
#[derive(Clone, Debug)]
pub struct SortPositionDescription {
    pub position: i32,
    pub description_string: String,
    pub count: i32,
}

impl SortPositionDescription {
    pub fn new(position: i32, description_string: impl Into<String>, count: i32) -> Self {
        Self {
            position,
            description_string: description_string.into(),
            count,
        }
    }
}

/// Ported from: VariationRealigner.MismatchResult (inner class)
/// Java: VariationRealigner.java#L2842-L2870
#[derive(Clone, Debug)]
pub struct MismatchResult {
    pub mismatches: Vec<Mismatch>,
    pub scp: Vec<i32>,
    pub nm: i32,
    pub misp: i32,
    pub misnt: String,
}

impl MismatchResult {
    pub fn new(
        mismatches: Vec<Mismatch>,
        scp: Vec<i32>,
        nm: i32,
        misp: i32,
        misnt: String,
    ) -> Self {
        Self {
            mismatches,
            scp,
            nm,
            misp,
            misnt,
        }
    }
}

/// Ported from: VariationRealigner.Mismatch (inner class)
/// Java: VariationRealigner.java#L2872-L2890
#[derive(Clone, Debug)]
pub struct Mismatch {
    pub mismatch_sequence: String,
    pub mismatch_position: i32,
    pub end: i32,
}

impl Mismatch {
    pub fn new(mismatch_sequence: impl Into<String>, mismatch_position: i32, end: i32) -> Self {
        Self {
            mismatch_sequence: mismatch_sequence.into(),
            mismatch_position,
            end,
        }
    }
}

// ─── COMP2 / COMP3 comparators ──────────────────────────────────────

/// Java: VariationRealigner.java#L340-L348  (COMP2)
/// Sort by descending softClip.varsCount, then ascending position.
pub fn comp2(a: &SortPositionSclip, b: &SortPositionSclip) -> std::cmp::Ordering {
    b.soft_clip
        .base
        .vars_count
        .cmp(&a.soft_clip.base.vars_count)
        .then(a.position.cmp(&b.position))
}

/// Java: VariationRealigner.java#L350-L358  (COMP3)
/// Sort by descending count field, then ascending position.
pub fn comp3(a: &SortPositionSclip, b: &SortPositionSclip) -> std::cmp::Ordering {
    b.count.cmp(&a.count).then(a.position.cmp(&b.position))
}

// ─── Cluster A: Utility methods ─────────────────────────────────────

/// Ported from: VariationRealigner.fillAndSortTmp()
/// Java: VariationRealigner.java#L2155-L2196
///
/// Flatten a position→{description→count} map into a sorted vec.
/// Sort: descending count → ascending position → descending description_string.
/// **Parity trap T1**: tertiary sort is DESC descriptionString (reversed compareTo).
pub fn fill_and_sort_tmp<M>(changes: &HashMap<i32, M>) -> Vec<SortPositionDescription>
where
    for<'a> &'a M: IntoIterator<Item = (&'a String, &'a i32)>,
{
    // Java: VariationRealigner.java#L2160-L2175
    let mut tmp = Vec::new();
    for (&position, v) in changes {
        for (description_string, cnt) in v {
            tmp.push(SortPositionDescription::new(
                position,
                description_string.clone(),
                *cnt,
            ));
        }
    }

    // Java: VariationRealigner.java#L2176-L2195
    // Sort: desc count, asc position, desc descriptionString
    tmp.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(a.position.cmp(&b.position))
            .then(b.description_string.cmp(&a.description_string))
    });

    tmp
}

/// Ported from: VariationRealigner.ismatch(String, String, int)
/// Java: VariationRealigner.java#L2313-L2325
///
/// Find if sequences match with no more than 3 mismatches.
pub fn ismatch(seq1: &str, seq2: &str, dir: i32) -> bool {
    // Java: VariationRealigner.java#L2316
    ismatch_with_mm(seq1, seq2, dir, 3)
}

/// Ported from: VariationRealigner.ismatch(String, String, int, int)
/// Java: VariationRealigner.java#L2327-L2345
///
/// Find if sequences match with no more than MM mismatches and total mismatches
/// no more than 15% of length of sequence.
/// **Parity trap T17**: direction formula: `seq2[dir*n - (dir==-1 ? 1 : 0)]`
pub fn ismatch_with_mm(seq1: &str, seq2: &str, dir: i32, mm_threshold: i32) -> bool {
    if GlobalReadOnlyScope::instance().conf.y {
        eprintln!(
            "    Matching two seqs {} {} {} {}",
            seq1, seq2, dir, mm_threshold
        );
    }
    // Java: VariationRealigner.java#L2334 — strip #/^ from seq2
    let seq2_clean: String = seq2.chars().filter(|c| *c != '#' && *c != '^').collect();
    let seq1_bytes = seq1.as_bytes();
    let seq2_bytes = seq2_clean.as_bytes();

    let mut mm = 0i32;
    for n in 0..seq1_bytes.len().min(seq2_bytes.len()) {
        // Java: VariationRealigner.java#L2337
        // seq1.charAt(n) != substr(seq2, dir * n - (dir == -1 ? 1 : 0), 1).charAt(0)
        let n_i32 = n as i32;
        let idx = dir * n_i32 - if dir == -1 { 1 } else { 0 };
        let ch2 = char_at(seq2_bytes, idx);
        if let Some(ch2) = ch2 {
            if seq1_bytes[n] != ch2 {
                mm += 1;
            }
        } else {
            mm += 1;
        }
    }

    // Java: VariationRealigner.java#L2342
    mm <= mm_threshold && (mm as f64 / seq1_bytes.len() as f64) < 0.15
}

/// Ported from: VariationRealigner.islowcomplexseq()
/// Java: VariationRealigner.java#L2353-L2385
///
/// Returns true if >75% single base or <3 distinct bases.
pub fn islowcomplexseq(seq: &str) -> bool {
    let len = seq.len();
    if len == 0 {
        return true;
    }

    // Java: VariationRealigner.java#L2361
    let mut ntcnt = 0;

    let a = count_char(seq, 'A');
    if a > 0 {
        ntcnt += 1;
    }
    if a as f64 / len as f64 > 0.75 {
        return true;
    }

    let t = count_char(seq, 'T');
    if t > 0 {
        ntcnt += 1;
    }
    if t as f64 / len as f64 > 0.75 {
        return true;
    }

    let g = count_char(seq, 'G');
    if g > 0 {
        ntcnt += 1;
    }
    if g as f64 / len as f64 > 0.75 {
        return true;
    }

    let c = count_char(seq, 'C');
    if c > 0 {
        ntcnt += 1;
    }
    if c as f64 / len as f64 > 0.75 {
        return true;
    }

    ntcnt < 3
}

/// Ported from: VariationRealigner.count()
/// Java: VariationRealigner.java#L2392-L2401
///
/// Count character occurrences in a string.
pub fn count_char(s: &str, ch: char) -> i32 {
    // Java: VariationRealigner.java#L2393-L2399
    let mut cnt = 0i32;
    for c in s.chars() {
        if c == ch {
            cnt += 1;
        }
    }
    cnt
}

/// Ported from: VariationRealigner.adjRefCnt()
/// Java: VariationRealigner.java#L2608-L2647
///
/// Adjust the reference count by position-derived factor.
/// **Parity trap T9**: integer division in factor computation.
pub fn adj_ref_cnt(tv: &Variation, ref_var: Option<&mut Variation>, len: i32) {
    // Java: VariationRealigner.java#L2614
    let ref_var = match ref_var {
        Some(r) => r,
        None => return,
    };

    if GlobalReadOnlyScope::instance().conf.y {
        let ref_cnt = if ref_var.vars_count != 0 {
            ref_var.vars_count.to_string()
        } else {
            "NA".to_string()
        };
        eprintln!(
            "    AdjRefCnt: '+' {} {} {} {} Ref: {}",
            ref_var.vars_count,
            tv.vars_count,
            get_dir(ref_var, false),
            get_dir(tv, false),
            ref_cnt
        );
        eprintln!(
            "    AdjRefCnt: '-' {} {} {} {} Ref: {}",
            ref_var.vars_count,
            tv.vars_count,
            get_dir(ref_var, true),
            get_dir(tv, true),
            ref_cnt
        );
    }

    // Java: VariationRealigner.java#L2627
    // the adjustment factor
    let f: f64 = if tv.mean_position != 0.0 {
        (tv.mean_position / tv.vars_count as f64 - len as f64 + 1.0)
            / (tv.mean_position / tv.vars_count as f64)
    } else {
        0.0
    };

    if f < 0.0 {
        return;
    }

    let f = if f > 1.0 { 1.0 } else { f };

    // Java: VariationRealigner.java#L2636-L2644
    ref_var.vars_count -= (f * tv.vars_count as f64) as i32;
    ref_var.high_quality_reads_count -= (f * tv.high_quality_reads_count as f64) as i32;
    ref_var.low_quality_reads_count -= (f * tv.low_quality_reads_count as f64) as i32;
    ref_var.mean_position -= f * tv.mean_position;
    ref_var.mean_quality -= f * tv.mean_quality;
    ref_var.mean_mapping_quality -= f * tv.mean_mapping_quality;
    ref_var.number_of_mismatches -= f * tv.number_of_mismatches;
    sub_dir(ref_var, true, (f * get_dir(tv, true) as f64) as i32);
    sub_dir(ref_var, false, (f * get_dir(tv, false) as f64) as i32);
    correct_cnt(ref_var);
}

/// Ported from: VariationRealigner.adjRefFactor()
/// Java: VariationRealigner.java#L2653-L2695
///
/// Adjust the reference by multiplicative factor.
/// **Parity trap T10**: sign preservation with Java operator precedence.
pub fn adj_ref_factor(ref_var: Option<&mut Variation>, factor_f: f64) {
    // Java: VariationRealigner.java#L2654
    let ref_var = match ref_var {
        Some(r) => r,
        None => return,
    };

    let mut factor_f = factor_f;

    // Java: VariationRealigner.java#L2658
    if factor_f > 1.0 {
        factor_f = 1.0;
    }

    // Java: VariationRealigner.java#L2662
    if factor_f < -1.0 {
        return;
    }

    if GlobalReadOnlyScope::instance().conf.y {
        eprintln!("    AdjRefFactor: {} {}", ref_var.vars_count, factor_f);
    }

    // Java: VariationRealigner.java#L2668
    let old_vars_count = ref_var.vars_count;
    ref_var.vars_count -= (factor_f * ref_var.vars_count as f64) as i32;
    ref_var.high_quality_reads_count -= (factor_f * ref_var.high_quality_reads_count as f64) as i32;
    ref_var.low_quality_reads_count -= (factor_f * ref_var.low_quality_reads_count as f64) as i32;

    // Java: VariationRealigner.java#L2674
    // Adjust mean mapping quality, mean quality and mean position only on number of changed counts
    let factor_cnt: f64 = if old_vars_count != 0 {
        ((ref_var.vars_count - old_vars_count) as f64).abs() / old_vars_count as f64
    } else {
        1.0
    };

    // Java: VariationRealigner.java#L2677
    // Factors must be the same sign
    // Note: &&  binds tighter than || in Java, same as Rust.
    // Since factorCnt from abs() is always >= 0, the second branch (factor_f > 0 && factorCnt < 0) never triggers.
    // Preserve verbatim for parity.
    let factor_cnt = if (factor_f < 0.0 && factor_cnt > 0.0) || (factor_f > 0.0 && factor_cnt < 0.0)
    {
        -factor_cnt
    } else {
        factor_cnt
    };

    // Java: VariationRealigner.java#L2680-L2682
    ref_var.mean_position -= ref_var.mean_position * factor_cnt;
    ref_var.mean_quality -= ref_var.mean_quality * factor_cnt;
    ref_var.mean_mapping_quality -= ref_var.mean_mapping_quality * factor_cnt;

    // Java: VariationRealigner.java#L2684-L2686
    ref_var.number_of_mismatches -= factor_f * ref_var.number_of_mismatches;
    ref_var.vars_count_on_forward -= (factor_f * ref_var.vars_count_on_forward as f64) as i32;
    ref_var.vars_count_on_reverse -= (factor_f * ref_var.vars_count_on_reverse as f64) as i32;

    correct_cnt(ref_var);
}

// PARITY: VariationRealigner.realignlgins30()/realignlgins() mutate the reference
// Variation fetched from nonInsertionVariants in place in Java
// (VariationRealigner.java:L1555-L1572, L1752-L1763, L1888-L1934).
// Rust clones that entry to satisfy borrowing, so the decremented clone must be
// written back after adj_cnt_with_reference().
fn write_back_cloned_reference_variation(
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    reference_sequences: &HashMap<i32, u8>,
    position: i32,
    reference_var: Option<Variation>,
) {
    let Some(reference_var) = reference_var else {
        return;
    };
    let Some(reference_base) = reference_sequences.get(&position) else {
        return;
    };
    let reference_key = (*reference_base as char).to_string();
    if let Some(entry) = non_insertion_variants
        .get_mut(&position)
        .and_then(|m| m.entries.get_mut(&reference_key))
    {
        *entry = reference_var;
    }
}

/// Ported from: VariationRealigner.addVarFactor()
/// Java: VariationRealigner.java#L2701-L2722
///
/// Add counts of variation by factor.
pub fn add_var_factor(vref: &mut Variation, factor_f: f64) {
    // Java: VariationRealigner.java#L2702
    if factor_f < -1.0 {
        return;
    }

    // Java: VariationRealigner.java#L2710-L2721
    vref.vars_count += (factor_f * vref.vars_count as f64) as i32;
    vref.high_quality_reads_count += (factor_f * vref.high_quality_reads_count as f64) as i32;
    vref.low_quality_reads_count += (factor_f * vref.low_quality_reads_count as f64) as i32;
    vref.mean_position += factor_f * vref.mean_position;
    vref.mean_quality += factor_f * vref.mean_quality;
    vref.mean_mapping_quality += factor_f * vref.mean_mapping_quality;
    vref.number_of_mismatches += factor_f * vref.number_of_mismatches;
    vref.vars_count_on_forward += (factor_f * vref.vars_count_on_forward as f64) as i32;
    vref.vars_count_on_reverse += (factor_f * vref.vars_count_on_reverse as f64) as i32;
}

/// Ported from: VariationRealigner.rmCnt()
/// Java: VariationRealigner.java#L2936-L2949
///
/// Subtract counts of one variation from another.
pub fn rm_cnt(vref: &mut Variation, tv: &Variation) {
    // Java: VariationRealigner.java#L2938-L2947
    vref.vars_count -= tv.vars_count;
    vref.high_quality_reads_count -= tv.high_quality_reads_count;
    vref.low_quality_reads_count -= tv.low_quality_reads_count;
    vref.mean_position -= tv.mean_position;
    vref.mean_quality -= tv.mean_quality;
    vref.mean_mapping_quality -= tv.mean_mapping_quality;
    sub_dir(vref, true, get_dir(tv, true));
    sub_dir(vref, false, get_dir(tv, false));
    correct_cnt(vref);
}

/// Ported from: VariationRealigner.ismatchref(String, Map<Integer, Character>, int, int)
/// Java: VariationRealigner.java#L2890-L2901
///
/// Check if sequence matches reference with default 3 mismatches.
pub fn ismatchref(sequence: &str, ref_map: &HashMap<i32, u8>, position: i32, dir: i32) -> bool {
    // Java: VariationRealigner.java#L2895
    ismatchref_with_mm(sequence, ref_map, position, dir, 3)
}

/// Ported from: VariationRealigner.ismatchref(String, Map<Integer, Character>, int, int, int)
/// Java: VariationRealigner.java#L2907-L2929
///
/// Check if sequence matches reference with a specific mismatch threshold.
pub fn ismatchref_with_mm(
    sequence: &str,
    ref_map: &HashMap<i32, u8>,
    position: i32,
    dir: i32,
    mm_threshold: i32,
) -> bool {
    if GlobalReadOnlyScope::instance().conf.y {
        eprintln!(
            "      Matching REF {} {} {} {}",
            sequence, position, dir, mm_threshold
        );
    }

    let seq_bytes = sequence.as_bytes();
    let mut mm = 0i32;
    for n in 0..seq_bytes.len() {
        let n_i32 = n as i32;
        // Java: VariationRealigner.java#L2920
        let ref_ch = ref_map.get(&(position + dir * n_i32));
        if ref_ch.is_none() {
            return false;
        }
        let ref_ch = *ref_ch.unwrap();

        // Java: VariationRealigner.java#L2923
        // charAt(sequence, dir == 1 ? n : dir * n - 1)
        let seq_idx = if dir == 1 { n_i32 } else { dir * n_i32 - 1 };
        let seq_ch = char_at(seq_bytes, seq_idx);
        if let Some(sch) = seq_ch {
            if sch != ref_ch {
                mm += 1;
            }
        } else {
            mm += 1;
        }
    }

    // Java: VariationRealigner.java#L2927
    mm <= mm_threshold && (mm as f64 / sequence.len() as f64) < 0.15
}

/// Ported from: VariationRealigner.adjInsPos()
/// Java: VariationRealigner.java#L2408-L2423
///
/// Adjust the insertion position if necessary (left-align).
pub fn adj_ins_pos(bi: i32, ins: &str, ref_map: &HashMap<i32, u8>) -> BaseInsertion {
    let mut bi = bi;
    let mut n = 1i32;
    let len = ins.len() as i32;
    let ins_bytes = ins.as_bytes();

    // Java: VariationRealigner.java#L2412
    while let Some(&ref_ch) = ref_map.get(&bi) {
        // ins.charAt(ins.length() - n)
        let ins_idx = ins_bytes.len() as i32 - n;
        if ins_idx < 0 {
            break;
        }
        if ref_ch != ins_bytes[ins_idx as usize] {
            break;
        }
        n += 1;
        // Java: VariationRealigner.java#L2414
        if n > len {
            n = 1;
        }
        bi -= 1;
    }

    // Java: VariationRealigner.java#L2418-L2420
    let new_ins = if n > 1 {
        // ins = substr(ins, 1 - n) + substr(ins, 0, 1 - n)
        let part1 = String::from_utf8_lossy(&substr(ins_bytes, 1 - n)).to_string();
        let part2 = String::from_utf8_lossy(&substr_with_len(ins_bytes, 0, 1 - n)).to_string();
        format!("{}{}", part1, part2)
    } else {
        ins.to_string()
    };

    BaseInsertion::new(bi, new_ins, bi)
}

/// Ported from: VariationRealigner.find35match()
/// Java: VariationRealigner.java#L2214-L2260
///
/// Test whether the two soft-clipped reads match.
/// Returns the breakpoints in 5' and 3' soft-clipped reads.
/// **Parity trap T5**: returns FIRST qualifying match, not best.
pub fn find35match(seq5: &str, seq3: &str) -> Match35 {
    let long_mismatch = 2;
    let mut max_matched_length = 0;
    let mut b3 = 0;
    let mut b5 = 0;

    let seq5_bytes = seq5.as_bytes();
    let seq3_bytes = seq3.as_bytes();
    let seq5_len = seq5_bytes.len() as i32;
    let seq3_len = seq3_bytes.len() as i32;

    // Java: VariationRealigner.java#L2222 — outer loop i over seq5
    for i in 0..(seq5_len - 8) {
        // Java: VariationRealigner.java#L2223 — inner loop j over seq3
        for j in 1..(seq3_len - 8) {
            let mut number_of_mismatch = 0;
            let mut total_length = 0i32;

            // Java: VariationRealigner.java#L2226
            while total_length + j <= seq3_len && i + total_length <= seq5_len {
                // Java: VariationRealigner.java#L2227
                // substr(seq3, -j - totalLength, 1) vs substr(seq5, i + totalLength, 1)
                let seq3_sub = substr_with_len(seq3_bytes, -j - total_length, 1);
                let seq5_sub = substr_with_len(seq5_bytes, i + total_length, 1);
                if seq3_sub != seq5_sub {
                    number_of_mismatch += 1;
                }
                if number_of_mismatch > long_mismatch {
                    break;
                }
                total_length += 1;
            }

            // Java: VariationRealigner.java#L2235-L2252
            if total_length - number_of_mismatch > max_matched_length
                && total_length - number_of_mismatch > 8
                && (number_of_mismatch as f64 / total_length as f64) < 0.1
                && (total_length + j >= seq3_len || i + total_length >= seq5_len)
            {
                max_matched_length = total_length - number_of_mismatch;
                b3 = j;
                b5 = i;
                if GlobalReadOnlyScope::instance().conf.y {
                    eprintln!(
                        "      Found 35 Match, {} {} {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        seq5,
                        seq3,
                        total_length + j,
                        seq3_len,
                        i + total_length,
                        seq5_len,
                        total_length,
                        number_of_mismatch,
                        i,
                        j
                    );
                }
                // **Parity trap T5**: Return FIRST qualifying match immediately
                return Match35::new(b5, b3, max_matched_length);
            }
        }
    }

    Match35::new(b5, b3, max_matched_length)
}

// ─── Cluster B stubs ────────────────────────────────────────────────

/// Ported from: VariationRealigner.findMM5()
/// Java: VariationRealigner.java#L2730-L2780
///
/// Given a variant sequence, find the mismatches and potential softclipping positions
/// walking in the 5' direction from variant position.
/// **Parity trap T4**: Side effect — marks soft clips used.
/// **Parity trap T16**: Uses `mcnt < longmm` (≤3 iterations, contrast with findMM3's ≤4).
pub fn find_mm5(
    ref_map: &HashMap<i32, u8>,
    position: i32,
    wupseq: &str,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
) -> MismatchResult {
    // Java: VariationRealigner.java#L2735
    let seq: String = wupseq.chars().filter(|c| *c != '#' && *c != '^').collect();
    let seq_bytes = seq.as_bytes();
    let longmm = 3i32;
    let mut mismatches: Vec<Mismatch> = Vec::new();
    let mut n = 0i32;
    let mut mn = 0i32;
    let mut mcnt = 0i32;
    let mut str_buf = String::new();
    let mut sc5p: Vec<i32> = Vec::new();

    // Java: VariationRealigner.java#L2743
    // while (isHasAndNotEquals(charAt(seq, -1 - n), ref, position - n) && mcnt < longmm)
    // **Parity trap T16**: mcnt < longmm (strict less than → ≤3 iterations)
    while mcnt < longmm {
        let ch = char_at(seq_bytes, -1 - n);
        match ch {
            Some(ch_val) => {
                if !is_has_and_not_equals_base(ch_val, ref_map, position - n) {
                    break;
                }
                str_buf.insert(0, ch_val as char);
                mismatches.push(Mismatch::new(str_buf.clone(), position - n, 5));
                n += 1;
                mcnt += 1;
            }
            None => break,
        }
    }

    // Java: VariationRealigner.java#L2749
    sc5p.push(position + 1);

    // Adjust clipping position if only one mismatch
    let mut misp = 0i32;
    let mut misnt: Option<u8> = None;

    // Java: VariationRealigner.java#L2752
    if str_buf.len() == 1 {
        // Java: while (isHasAndEquals(charAt(seq, -1 - n), ref, position - n))
        loop {
            let ch = char_at(seq_bytes, -1 - n);
            match ch {
                Some(ch_val) => {
                    if !is_has_and_equals_base(ch_val, ref_map, position - n) {
                        break;
                    }
                    n += 1;
                    if n != 0 {
                        mn += 1;
                    }
                }
                None => break,
            }
        }

        // Java: VariationRealigner.java#L2760
        if mn > 1 {
            let mut n2 = 0i32;
            // Java: while (-1 - n - 1 - n2 >= 0
            //        && isHasAndEquals(charAt(seq, -1 - n - 1 - n2), ref, position - n - 1 - n2))
            loop {
                if -1 - n - 1 - n2 < 0 {
                    break;
                }
                let ch = char_at(seq_bytes, -1 - n - 1 - n2);
                match ch {
                    Some(ch_val) => {
                        if !is_has_and_equals_base(ch_val, ref_map, position - n - 1 - n2) {
                            break;
                        }
                        n2 += 1;
                    }
                    None => break,
                }
            }

            // Java: VariationRealigner.java#L2766
            if n2 > 2 {
                sc5p.push(position - n - n2);
                misp = position - n;
                misnt = char_at(seq_bytes, -1 - n);
                // Java: VariationRealigner.java#L2770 — side effect: mark soft clip used
                if let Some(sc) = soft_clips_5_end.get_mut(&(position - n - n2)) {
                    sc.used = true;
                }
                mn += n2;
            } else {
                sc5p.push(position - n);
                // Java: VariationRealigner.java#L2774 — side effect: mark soft clip used
                if let Some(sc) = soft_clips_5_end.get_mut(&(position - n)) {
                    sc.used = true;
                }
            }
        }
    }

    // Java: VariationRealigner.java#L2779
    MismatchResult::new(
        mismatches,
        sc5p,
        mn,
        misp,
        misnt.map_or(String::new(), |c| (c as char).to_string()),
    )
}

/// Ported from: VariationRealigner.findMM3()
/// Java: VariationRealigner.java#L2788-L2840
///
/// Given a variant sequence, find the mismatches and potential softclipping positions
/// walking in the 3' direction from variant position.
/// **Parity trap T4**: Side effect — marks soft clips used.
/// **Parity trap T16**: Uses `mcnt <= longmm` (≤4 iterations, contrast with findMM5's ≤3).
pub fn find_mm3(
    ref_map: &HashMap<i32, u8>,
    p: i32,
    sanpseq: &str,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
) -> MismatchResult {
    // Java: VariationRealigner.java#L2793
    let seq: String = sanpseq.chars().filter(|c| *c != '#' && *c != '^').collect();
    let seq_bytes = seq.as_bytes();
    let longmm = 3i32;
    let mut mismatches: Vec<Mismatch> = Vec::new();
    let mut n = 0i32;
    let mut mn = 0i32;
    let mut mcnt = 0i32;
    let mut sc3p: Vec<i32> = Vec::new();
    let mut str_buf = String::new();

    // Java: VariationRealigner.java#L2801
    // Walk matches FIRST (opposite of findMM5 which walks mismatches first)
    // while (n < seq.length() && ref.containsKey(p + n) && isEquals(ref.get(p + n), seq.charAt(n)))
    while (n as usize) < seq_bytes.len() {
        match ref_map.get(&(p + n)) {
            Some(&ref_ch) => {
                if !is_equals(Some(ref_ch), Some(seq_bytes[n as usize])) {
                    break;
                }
                n += 1;
            }
            None => break,
        }
    }

    // Java: VariationRealigner.java#L2805
    sc3p.push(p + n);
    let tbp = p + n;

    // Java: VariationRealigner.java#L2807
    // while (mcnt <= longmm && n < seq.length() && isNotEquals(ref.get(p + n), seq.charAt(n)))
    // **Parity trap T16**: mcnt <= longmm (less-than-or-equal → ≤4 iterations)
    while mcnt <= longmm && (n as usize) < seq_bytes.len() {
        let ref_ch = ref_map.get(&(p + n)).copied();
        let seq_ch = Some(seq_bytes[n as usize]);
        if !is_not_equals(ref_ch, seq_ch) {
            break;
        }
        str_buf.push(seq_bytes[n as usize] as char);
        mismatches.push(Mismatch::new(str_buf.clone(), tbp, 3));
        n += 1;
        mcnt += 1;
    }

    // Adjust clipping position if only one mismatch
    let mut misp = 0i32;
    let mut misnt: Option<u8> = None;

    // Java: VariationRealigner.java#L2816
    if str_buf.len() == 1 {
        // Java: while (n < seq.length() && isHasAndEquals(seq.charAt(n), ref, p + n))
        while (n as usize) < seq_bytes.len() {
            if !is_has_and_equals_base(seq_bytes[n as usize], ref_map, p + n) {
                break;
            }
            n += 1;
            if n != 0 {
                mn += 1;
            }
        }

        // Java: VariationRealigner.java#L2824
        if mn > 1 {
            let mut n2 = 0i32;
            // Java: while (n + n2 + 1 < seq.length()
            //        && isHasAndEquals(seq.charAt(n + n2 + 1), ref, p + n + 1 + n2))
            while ((n + n2 + 1) as usize) < seq_bytes.len() {
                if !is_has_and_equals_base(
                    seq_bytes[(n + n2 + 1) as usize],
                    ref_map,
                    p + n + 1 + n2,
                ) {
                    break;
                }
                n2 += 1;
            }

            // Java: VariationRealigner.java#L2829
            if n2 > 2 && ((n + n2 + 1) as usize) < seq_bytes.len() {
                sc3p.push(p + n + n2);
                misp = p + n;
                misnt = Some(seq_bytes[n as usize]);
                // Java: VariationRealigner.java#L2833 — side effect: mark soft clip used
                if let Some(sc) = soft_clips_3_end.get_mut(&(p + n + n2)) {
                    sc.used = true;
                }
                mn += n2;
            } else {
                sc3p.push(p + n);
                // Java: VariationRealigner.java#L2837 — side effect: mark soft clip used
                if let Some(sc) = soft_clips_3_end.get_mut(&(p + n)) {
                    sc.used = true;
                }
            }
        }
    }

    // Java: VariationRealigner.java#L2840
    MismatchResult::new(
        mismatches,
        sc3p,
        mn,
        misp,
        misnt.map_or(String::new(), |c| (c as char).to_string()),
    )
}

/// Ported from: VariationRealigner.findbi()
/// Java: VariationRealigner.java#L2431-L2538
///
/// Find insertion breakpoint from soft-clipped sequence.
/// **Parity trap T14**: 5' (dir=-1) uses reverse+append, 3' (dir=1) uses left-align+rotate.
pub fn findbi(
    seq: &str,
    position: i32,
    ref_map: &HashMap<i32, u8>,
    dir: i32,
    chr: &str,
) -> BaseInsertion {
    // Java: VariationRealigner.java#L2437
    let maxmm = 3i32;
    let dir_ext = if dir == -1 { 1 } else { 0 };
    let mut score = 0i32;
    let mut bi = 0i32;
    let mut ins = String::new();
    let mut bi2 = 0i32;

    let seq_bytes = seq.as_bytes();
    let seq_len = seq_bytes.len() as i32;
    let chr_len = GlobalReadOnlyScope::instance()
        .chr_lengths
        .get(chr)
        .copied()
        .unwrap_or(0);

    // Java: VariationRealigner.java#L2444 — for (int n = 6; n < seq.length(); n++)
    for n in 6..seq_len {
        // Java: VariationRealigner.java#L2445
        if position + 6 >= chr_len {
            break;
        }
        let mut mm = 0i32;
        let mut i = 0i32;
        let mut m: HashSet<u8> = HashSet::new();

        // Java: VariationRealigner.java#L2449 — for (i = 0; i + n < seq.length(); i++)
        while i + n < seq_len {
            let ref_pos = position + dir * i - dir_ext;
            if ref_pos < 1 {
                break;
            }
            if ref_pos > chr_len {
                break;
            }
            // Java: VariationRealigner.java#L2456
            let seq_ch = seq_bytes[(i + n) as usize];
            let ref_ch = ref_map.get(&ref_pos).copied();
            if is_not_equals(Some(seq_ch), ref_ch) {
                mm += 1;
            } else {
                m.insert(seq_ch);
            }
            if mm > maxmm {
                break;
            }
            i += 1;
        }

        let mnt = m.len() as i32;
        // Java: VariationRealigner.java#L2463
        if mnt < 2 {
            continue;
        }

        // Java: VariationRealigner.java#L2466
        if (mnt >= 3 && i + n >= seq_len - 1 && i >= 8 && (mm as f64 / i as f64) < 0.15)
            || (mnt >= 2 && mm == 0 && i + n == seq_len && n >= 20 && i >= 8)
        {
            // Java: VariationRealigner.java#L2468
            let mut insert_str = String::from_utf8_lossy(&seq_bytes[0..n as usize]).to_string();
            let mut extra = String::new();
            let mut ept = 0i32;

            // Java: VariationRealigner.java#L2470
            // while (n + ept + 1 < seq.length()
            //   && (!isEquals(seq.charAt(n + ept), ref.get(position + ept * dir - dirExt))
            //       || !isEquals(seq.charAt(n + ept + 1), ref.get(position + (ept + 1) * dir - dirExt))))
            while (n + ept + 1) < seq_len {
                let c1_eq = is_equals(
                    Some(seq_bytes[(n + ept) as usize]),
                    ref_map.get(&(position + ept * dir - dir_ext)).copied(),
                );
                let c2_eq = is_equals(
                    Some(seq_bytes[(n + ept + 1) as usize]),
                    ref_map
                        .get(&(position + (ept + 1) * dir - dir_ext))
                        .copied(),
                );
                if c1_eq && c2_eq {
                    break;
                }
                extra.push(seq_bytes[(n + ept) as usize] as char);
                ept += 1;
            }

            // Java: VariationRealigner.java#L2476
            if dir == -1 {
                // **Parity trap T14**: 5' direction uses reverse+append
                insert_str.push_str(&extra);
                // Java: insert.reverse()
                insert_str = insert_str.chars().rev().collect();
                // Java: if (extra.length() > 0) insert.insert(insert.length() - extra.length(), "&")
                if !extra.is_empty() {
                    let pos = insert_str.len() - extra.len();
                    insert_str.insert(pos, '&');
                }

                // Java: VariationRealigner.java#L2283
                if mm == 0 && i + n == seq_len {
                    bi = position - 1 - extra.len() as i32;
                    ins = insert_str;
                    bi2 = position - 1;
                    if extra.is_empty() {
                        let tpl = adj_ins_pos(bi, &ins, ref_map);
                        bi = tpl.base_insert.unwrap_or(0);
                        ins = tpl.insertion_sequence;
                        bi2 = tpl.base_insert2.unwrap_or(0);
                    }
                    return BaseInsertion::new(bi, ins, bi2);
                } else if i - mm > score {
                    bi = position - 1 - extra.len() as i32;
                    ins = insert_str;
                    bi2 = position - 1;
                    score = i - mm;
                }
            } else {
                // **Parity trap T14**: 3' direction uses left-align+rotate
                let mut s = -1i32;
                if !extra.is_empty() {
                    insert_str.push('&');
                    insert_str.push_str(&extra);
                } else {
                    // Java: while (s >= -n && isEquals(charAt(insert, s), ref.get(position + s)))
                    let ins_bytes = insert_str.as_bytes();
                    while s >= -n {
                        let ins_ch = char_at(ins_bytes, s);
                        let ref_ch = ref_map.get(&(position + s)).copied();
                        if !is_equals(ins_ch, ref_ch) {
                            break;
                        }
                        s -= 1;
                    }
                    if s < -1 {
                        // Java: String tins = substr(insert.toString(), s + 1, 1 - s)
                        let ins_b = insert_str.as_bytes();
                        let tins = String::from_utf8_lossy(&substr_with_len(ins_b, s + 1, 1 - s))
                            .to_string();
                        // Java: insert.delete(insert.length() + s + 1, insert.length())
                        let del_start = (insert_str.len() as i32 + s + 1) as usize;
                        insert_str.truncate(del_start);
                        // Java: insert.insert(0, tins)
                        insert_str.insert_str(0, &tins);
                    }
                }

                // Java: VariationRealigner.java#L2311
                if mm == 0 && i + n == seq_len {
                    bi = position + s;
                    ins = insert_str;
                    bi2 = position + s + extra.len() as i32;
                    if extra.is_empty() {
                        let tpl = adj_ins_pos(bi, &ins, ref_map);
                        bi = tpl.base_insert.unwrap_or(0);
                        ins = tpl.insertion_sequence;
                        bi2 = tpl.base_insert2.unwrap_or(0);
                    }
                    return BaseInsertion::new(bi, ins, bi2);
                } else if i - mm > score {
                    bi = position + s;
                    ins = insert_str;
                    bi2 = position + s + extra.len() as i32;
                    score = i - mm;
                }
            }
        }
    }

    // Java: VariationRealigner.java#L2534
    if bi2 == bi && !ins.is_empty() && bi != 0 {
        let tpl = adj_ins_pos(bi, &ins, ref_map);
        bi = tpl.base_insert.unwrap_or(0);
        ins = tpl.insertion_sequence;
    }
    BaseInsertion::new(bi, ins, bi2)
}

/// Ported from: VariationRealigner.findbp()
/// Java: VariationRealigner.java#L2545-L2600
///
/// Find deletion breakpoint position in sequence.
/// **Parity trap T9**: `maxmm - n/100` uses integer division.
pub fn findbp(
    sequence: &str,
    start_position: i32,
    ref_map: &HashMap<i32, u8>,
    direction: i32,
    chr: &str,
) -> i32 {
    // Java: VariationRealigner.java#L2553
    let maxmm = 3i32;
    let mut bp = 0i32;
    let mut score = 0i32;
    let instance = GlobalReadOnlyScope::instance();
    let idx = instance
        .chr_lengths
        .get(chr)
        .copied()
        .or_else(|| ref_map.keys().copied().max())
        .unwrap_or(0);
    let seq_bytes = sequence.as_bytes();
    let seq_len = seq_bytes.len() as i32;

    // Java: VariationRealigner.java#L2557 — for (int n = 0; n < instance().conf.indelsize; n++)
    for n in 0..instance.conf.indelsize {
        let mut mm = 0i32;
        let mut i = 0i32;
        let mut m: HashSet<u8> = HashSet::new();

        // Java: for (i = 0; i < sequence.length(); i++)
        while i < seq_len {
            // Java: VariationRealigner.java#L2562-L2565
            let ref_pos = start_position + direction * n + direction * i;
            if ref_pos < 1 {
                break;
            }
            if ref_pos > idx {
                break;
            }
            // Java: VariationRealigner.java#L2566
            let seq_ch = seq_bytes[i as usize];
            let ref_ch = ref_map.get(&ref_pos).copied();
            if is_equals(Some(seq_ch), ref_ch) {
                m.insert(seq_ch);
            } else {
                mm += 1;
            }
            // Java: VariationRealigner.java#L2571 — **Parity trap T9**: integer division
            if mm > maxmm - n / 100 {
                break;
            }
            i += 1;
        }

        // Java: VariationRealigner.java#L2574
        if m.len() < 3 {
            continue;
        }

        // Java: VariationRealigner.java#L2577
        if mm <= maxmm - n / 100
            && i >= seq_len - 2
            && i >= 8 + n / 10
            && (mm as f64 / i as f64) < 0.12
        {
            let lbp = start_position + direction * n - if direction < 0 { direction } else { 0 };
            // Java: VariationRealigner.java#L2579
            if mm == 0 && i == seq_len {
                if instance.conf.y {
                    eprintln!(
                        "  Findbp: {} {} {} {} {}",
                        sequence, start_position, lbp, mm, i
                    );
                }
                return lbp;
            } else if i - mm > score {
                bp = lbp;
                score = i - mm;
            }
        }
    }

    // Java: VariationRealigner.java#L2596
    if GlobalReadOnlyScope::instance().conf.y && bp != 0 {
        eprintln!(
            "  Findbp with mismatches: {} {} {} {} {}",
            sequence, start_position, bp, direction, score
        );
    }
    bp
}

/// Ported from: VariationRealigner.noPassingReads()
/// Java: VariationRealigner.java#L2270-L2306
///
/// Check for reads spanning a deletion gap.
/// Opens BAM file(s) via SamView, queries the region chr:start-end,
/// and checks if any reads fully span the gap.
pub fn no_passing_reads(chr: &str, start: i32, end: i32, bams: &[String]) -> bool {
    // Java: VariationRealigner.java#L2272
    let mut cnt = 0i32;
    let mut midcnt = 0i32;
    let dlen = end - start;
    let dlenqr = format!("{}D", dlen);
    let instance = GlobalReadOnlyScope::instance();

    NO_PASSING_READERS.with(|cached_readers| {
        let mut cached_readers = cached_readers.borrow_mut();

        // Java: VariationRealigner.java#L2276 — for (String bam : bams)
        for bam_path in bams {
            // Java creates a SamView for each call. Reusing the same IndexedReader preserves
            // fetch/read semantics while avoiding repeated index reloads for identical BAMs.
            let reader = match cached_readers.entry(bam_path.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let reader_result = bam::IndexedReader::from_path(bam_path);
                    let mut reader = match reader_result {
                        Ok(r) => r,
                        Err(_) => continue, // Java catches exceptions and continues
                    };
                    reader.set_threads(1).ok();
                    entry.insert(reader)
                }
            };

            // Java: queryOverlapping(chr, start, end) — 1-based inclusive
            let region_str = format!("{}:{}-{}", chr, start, end);
            if reader.fetch(region_str.as_str()).is_err() {
                continue;
            }

            let mut record = bam::Record::new();
            // Java: while ((record = reader.read()) != null)
            while reader.read(&mut record).is_some() {
                // Java: VariationRealigner.java#L2281 — record.getCigarString().contains(dlenqr)
                let cigar = record.cigar();
                let cigar_str = format!("{}", cigar);
                if cigar_str.contains(&dlenqr) {
                    continue;
                }

                // Java: VariationRealigner.java#L2284
                let read_start = record.pos() as i32 + 1; // 0-based to 1-based

                // Java: getAlignedLength(record.getCigar()) — sum M+D
                let read_length_aligned: i32 = cigar
                    .iter()
                    .filter(|op| {
                        matches!(
                            op,
                            rust_htslib::bam::record::Cigar::Match(_)
                                | rust_htslib::bam::record::Cigar::Del(_)
                        )
                    })
                    .map(|op| op.len() as i32)
                    .sum();

                let read_end = read_start + read_length_aligned;

                // Java: VariationRealigner.java#L2288
                if read_end > end + 2 && read_start < start - 2 {
                    cnt += 1;
                }
                // Java: VariationRealigner.java#L2291
                if read_start < start - 2 && read_end > start && read_end < end {
                    midcnt += 1;
                }
            }
        }
    });

    // Java: VariationRealigner.java#L2297
    if instance.conf.y {
        eprintln!(
            "    Passing Read CNT: {} {} {} {} {}",
            cnt, chr, start, end, midcnt
        );
    }
    // Java: return cnt <= 0 && midcnt + 1 > 0;
    cnt <= 0 && midcnt + 1 > 0
}

/// Ported from: VariationRealigner.filterSV()
/// Java: VariationRealigner.java#L383-L425
///
/// For each SV: cluster mates, apply disc/cnt filter, populate SOFTP2SV.
/// **Parity trap**: disc/cnt nested negation: `!(cnt/disc >= 0.35 && cnt >= 5)`
pub fn filter_sv(
    sv_list: &mut Vec<Sclip>,
    max_read_length: i32,
    softp2sv: &mut HashMap<i32, Vec<Sclip>>,
) {
    // Java: VariationRealigner.java#L384 — for (Sclip sv: svList_sva)
    for sv in sv_list.iter_mut() {
        // Java: checkCluster(sv.mates, maxReadLength)
        // Java catches exceptions and continues; we'll match that behavior
        if sv.mates.is_empty() {
            sv.used = true;
            continue;
        }
        let cluster = check_cluster(&mut sv.mates, max_read_length);

        // Java: VariationRealigner.java#L389
        if cluster.base.mate_start_ms != 0 {
            sv.mstart = cluster.base.mate_start_ms;
            sv.mend = cluster.base.mate_end_me;
            sv.base.vars_count = cluster.cnt;
            sv.mlen = cluster.base.mate_length_mlen;
            sv.start = cluster.base.start_s;
            sv.end = cluster.base.end_e;
            sv.base.mean_position = cluster.base.pmean_rp;
            sv.base.mean_quality = cluster.base.qmean_q;
            sv.base.mean_mapping_quality = cluster.base.qmean_qq;
            sv.base.number_of_mismatches = cluster.base.nm;
        } else {
            sv.used = true;
        }

        // Java: VariationRealigner.java#L403
        // Too many unhappy mates are false positive
        if sv.disc != 0 && (sv.base.vars_count as f64 / sv.disc as f64) < 0.5 {
            // Java: if (!(sv.varsCount / (double) sv.disc >= 0.35 && sv.varsCount >= 5))
            if !((sv.base.vars_count as f64 / sv.disc as f64) >= 0.35 && sv.base.vars_count >= 5) {
                sv.used = true;
            }
        }

        // Java: VariationRealigner.java#L409-L413
        // Sort sv.soft entries by descending value
        let mut soft_entries: Vec<(i32, i32)> = sv.soft.iter().map(|(&k, &v)| (k, v)).collect();
        soft_entries.sort_by(|a, b| b.1.cmp(&a.1));

        // Java: sv.softp = soft.size() > 0 ? soft.get(0).getKey() : 0
        sv.softp = if !soft_entries.is_empty() {
            soft_entries[0].0
        } else {
            0
        };

        // Java: VariationRealigner.java#L416-L419
        if sv.softp != 0 {
            softp2sv
                .entry(sv.softp)
                .or_insert_with(Vec::new)
                .push(sv.clone());
        }

        // Java: VariationRealigner.java#L421 — debug print
        if GlobalReadOnlyScope::instance().conf.y {
            eprintln!(
                "SV cluster: {} {} {} {} Cnt: {} Discordant Cnt: {} Softp: {} Used: {}",
                cluster.base.start_s,
                cluster.base.end_e,
                cluster.base.mate_start_ms,
                cluster.base.mate_end_me,
                sv.base.vars_count,
                sv.disc,
                sv.softp,
                sv.used
            );
        }
    }
}

/// Ported from: VariationRealigner.checkCluster()
/// Java: VariationRealigner.java#L432-L482
///
/// Cluster mates by position proximity, return dominant cluster.
/// **Parity critical**: First mate BOTH initializes cluster 0 AND is processed in the loop.
pub fn check_cluster(mates: &mut Vec<crate::data::Mate>, rlen: i32) -> Cluster {
    // Java: VariationRealigner.java#L433
    mates.sort_by(|a, b| a.mate_start_ms.cmp(&b.mate_start_ms));

    // Java: VariationRealigner.java#L435-L437
    let first_mate = &mates[0];
    let mut clusters: Vec<Cluster> = vec![Cluster::new(
        0,
        first_mate.mate_start_ms,
        first_mate.mate_end_me,
        first_mate.start_s,
        first_mate.end_e,
    )];

    let mut cur = 0usize;

    // Java: VariationRealigner.java#L440 — for (Mate mate_m : mates)
    // Note: first mate is BOTH used to init cluster AND processed in loop (cnt starts at 1 after first iteration)
    for mate_m in mates.iter() {
        // Java: VariationRealigner.java#L441
        let needs_new_cluster = {
            let current_cluster = &clusters[cur];
            mate_m.mate_start_ms - current_cluster.base.mate_end_me
                > (MINSVCDIST * rlen as f64) as i32
        };

        if needs_new_cluster {
            cur += 1;
            // Java: clusters.add(cur, new Cluster(0, mate_m.mateStart_ms, mate_m.mateEnd_me, mate_m.start_s, mate_m.end_e))
            clusters.insert(
                cur,
                Cluster::new(
                    0,
                    mate_m.mate_start_ms,
                    mate_m.mate_end_me,
                    mate_m.start_s,
                    mate_m.end_e,
                ),
            );
        }

        let current_cluster = &mut clusters[cur];
        // Java: VariationRealigner.java#L448
        current_cluster.cnt += 1;
        current_cluster.base.mate_length_mlen += mate_m.mate_length_mlen;

        if mate_m.mate_end_me > current_cluster.base.mate_end_me {
            current_cluster.base.mate_end_me = mate_m.mate_end_me;
        }
        if mate_m.start_s < current_cluster.base.start_s {
            current_cluster.base.start_s = mate_m.start_s;
        }
        if mate_m.end_e > current_cluster.base.end_e {
            current_cluster.base.end_e = mate_m.end_e;
        }
        current_cluster.base.pmean_rp += mate_m.pmean_rp;
        current_cluster.base.qmean_q += mate_m.qmean_q;
        current_cluster.base.qmean_qq += mate_m.qmean_qq;
        current_cluster.base.nm += mate_m.nm;
    }

    // Java: VariationRealigner.java#L462
    clusters.sort_by(|a, b| b.cnt.cmp(&a.cnt));

    if GlobalReadOnlyScope::instance().conf.y {
        eprint!("Clusters; ");
        for cluster in &clusters {
            eprint!(
                "{}; {}; {}; {}; {}; ",
                cluster.cnt,
                cluster.base.start_s,
                cluster.base.end_e,
                cluster.base.mate_start_ms,
                cluster.base.mate_end_me
            );
        }
        eprintln!("; out of; {}", mates.len());
    }

    let first_cluster = &clusters[0];
    // Java: VariationRealigner.java#L472
    if first_cluster.cnt as f64 / mates.len() as f64 >= 0.60 {
        // Java: mateLength_mlen/firstCluster.cnt — integer division
        Cluster::new_with_metrics(
            first_cluster.base.mate_start_ms,
            first_cluster.base.mate_end_me,
            first_cluster.cnt,
            first_cluster.base.mate_length_mlen / first_cluster.cnt,
            first_cluster.base.start_s,
            first_cluster.base.end_e,
            first_cluster.base.pmean_rp,
            first_cluster.base.qmean_q,
            first_cluster.base.qmean_qq,
            first_cluster.base.nm,
        )
    } else {
        // Java: return new Cluster(0,0,0,0,0,0,0,0.0,0,0)
        Cluster::new_with_metrics(0, 0, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0.0)
    }
}

/// Ported from: VariationRealigner.filterAllSVStructures()
/// Java: VariationRealigner.java#L353-L377
///
/// Filter all SV structure lists to remove false positives,
/// then sort SOFTP2SV entries by descending varsCount.
pub fn filter_all_sv_structures(
    sv_structures: &mut crate::data::SVStructures,
    max_read_length: i32,
    softp2sv: &mut HashMap<i32, Vec<Sclip>>,
) {
    // Java: VariationRealigner.java#L354-L361
    filter_sv(&mut sv_structures.svfinv3, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svrinv3, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svfinv5, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svrinv5, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svfdel, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svrdel, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svfdup, max_read_length, softp2sv);
    filter_sv(&mut sv_structures.svrdup, max_read_length, softp2sv);

    // Java: VariationRealigner.java#L363-L365
    for (_chr, sv_list) in sv_structures.svffus.iter_mut() {
        filter_sv(sv_list, max_read_length, softp2sv);
    }
    for (_chr, sv_list) in sv_structures.svrfus.iter_mut() {
        filter_sv(sv_list, max_read_length, softp2sv);
    }

    // Java: VariationRealigner.java#L367-L373
    // Sort SOFTP2SV entries by descending varsCount
    for (_key, sclips) in softp2sv.iter_mut() {
        sclips.sort_by(|a, b| b.base.vars_count.cmp(&a.base.vars_count));
    }
}

// ─── Cluster C: adjustMNP + Short Indel Realignment ─────────────────

/// Ported from: VariationRealigner.adjustMNP()
/// Java: VariationRealigner.java#L487-L569
///
/// Merges partial MNP sub-variants back into their parent MNP,
/// and absorbs matching soft clips.
/// **Parity trap T3**: left uses `<= 0`, right uses `< 0`.
pub fn adjust_mnp(
    mnp: &HashMap<i32, HashMap<String, i32>>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference_sequences: &HashMap<i32, u8>,
) {
    // Java: VariationRealigner.java#L488
    let tmp = fill_and_sort_tmp(mnp);
    for tpl in &tmp {
        let position = tpl.position;
        let vn = &tpl.description_string;

        // Java: VariationRealigner.java#L494
        if !non_insertion_variants.contains_key(&position) {
            continue;
        }
        // Check if vref exists (already consumed?)
        if non_insertion_variants
            .get(&position)
            .and_then(|m| m.entries.get(vn))
            .is_none()
        {
            continue;
        }

        if GlobalReadOnlyScope::instance().conf.y {
            let vc = non_insertion_variants
                .get(&position)
                .and_then(|m| m.entries.get(vn))
                .map(|v| v.vars_count)
                .unwrap_or(0);
            eprintln!("  AdjMnt: {} {} {}", position, vn, vc);
        }

        // Java: VariationRealigner.java#L503
        let mnt = vn.replacen('&', "", 1);

        // Java: VariationRealigner.java#L504
        for i in 0..(mnt.len() as i32 - 1) {
            let i_usize = i as usize;

            // Java: VariationRealigner.java#L505 — left fragment
            let mut left = mnt[0..i_usize + 1].to_string();
            if left.len() > 1 {
                // Java: sb.insert(1, "&")
                left.insert(1, '&');
            }

            // Java: VariationRealigner.java#L511 — right fragment
            let right_start = mnt.len() - (mnt.len() - i_usize - 1);
            let mut right = mnt[right_start..].to_string();
            if right.len() > 1 {
                right.insert(1, '&');
            }

            // Java: VariationRealigner.java#L518 — Left check
            // **Parity trap T3**: left uses <= 0
            {
                let tref_clone = non_insertion_variants
                    .get(&position)
                    .and_then(|m| m.entries.get(&left))
                    .cloned();
                if let Some(tref_clone) = tref_clone {
                    if tref_clone.vars_count <= 0 {
                        // Java: VariationRealigner.java#L520 — ASYMMETRY: left uses <= 0
                        continue;
                    }
                    let vref_vars_count = non_insertion_variants
                        .get(&position)
                        .and_then(|m| m.entries.get(vn))
                        .map(|v| v.vars_count)
                        .unwrap_or(0);
                    if tref_clone.vars_count < vref_vars_count
                        && tref_clone.mean_position / tref_clone.vars_count as f64 <= (i + 1) as f64
                    {
                        if GlobalReadOnlyScope::instance().conf.y {
                            eprintln!(
                                "    AdjMnt Left: {} {} Left: {} Cnt: {}",
                                position, vn, left, tref_clone.vars_count
                            );
                        }
                        // Java: adjCnt(vref, tref)
                        let vref = non_insertion_variants
                            .get_mut(&position)
                            .unwrap()
                            .entries
                            .get_mut(vn)
                            .unwrap();
                        adj_cnt(vref, &tref_clone);
                        non_insertion_variants
                            .get_mut(&position)
                            .unwrap()
                            .entries
                            .shift_remove(&left);
                    }
                }
            }

            // Java: VariationRealigner.java#L530 — Right check
            // **Parity trap T3**: right uses < 0
            let right_pos = position + i + 1;
            {
                let tref_clone = non_insertion_variants
                    .get(&right_pos)
                    .and_then(|m| m.entries.get(&right))
                    .cloned();
                if let Some(tref_clone) = tref_clone {
                    if tref_clone.vars_count < 0 {
                        // Java: VariationRealigner.java#L533 — ASYMMETRY: right uses < 0
                        continue;
                    }
                    let vref_vars_count = non_insertion_variants
                        .get(&position)
                        .and_then(|m| m.entries.get(vn))
                        .map(|v| v.vars_count)
                        .unwrap_or(0);
                    if tref_clone.vars_count < vref_vars_count {
                        if GlobalReadOnlyScope::instance().conf.y {
                            eprintln!(
                                "    AdjMnt Right: {} {} Right: {} Cnt: {}",
                                position, vn, right, tref_clone.vars_count
                            );
                        }
                        let vref = non_insertion_variants
                            .get_mut(&position)
                            .unwrap()
                            .entries
                            .get_mut(vn)
                            .unwrap();
                        adj_cnt(vref, &tref_clone);
                        inc_cnt(ref_coverage, position, tref_clone.vars_count);
                        non_insertion_variants
                            .get_mut(&right_pos)
                            .unwrap()
                            .entries
                            .shift_remove(&right);
                    }
                }
            }
        }

        // Java: VariationRealigner.java#L547 — 3' soft clip check
        let sc3_info = soft_clips_3_end.get_mut(&position).and_then(|sc3v| {
            if sc3v.used {
                return None;
            }
            let seq = find_conseq(sc3v, 0);
            if seq.starts_with(&mnt) {
                if seq.len() == mnt.len()
                    || ismatchref(
                        &seq[mnt.len()..],
                        reference_sequences,
                        position + mnt.len() as i32,
                        1,
                    )
                {
                    let base_clone = sc3v.base.clone();
                    let vars_count = sc3v.base.vars_count;
                    sc3v.used = true;
                    return Some((base_clone, vars_count));
                }
            }
            None
        });
        if let Some((base_clone, vars_count)) = sc3_info {
            let vref = non_insertion_variants
                .get_mut(&position)
                .unwrap()
                .entries
                .get_mut(vn)
                .unwrap();
            adj_cnt(vref, &base_clone);
            inc_cnt(ref_coverage, position, vars_count);
        }

        // Java: VariationRealigner.java#L558 — 5' soft clip check
        let sc5_pos = position + mnt.len() as i32;
        let sc5_info = soft_clips_5_end.get_mut(&sc5_pos).and_then(|sc5v| {
            if sc5v.used {
                return None;
            }
            let seq = find_conseq(sc5v, 0);
            if seq.is_empty() || seq.len() < mnt.len() {
                return None;
            }
            let seq_rev: String = seq.chars().rev().collect();
            if seq_rev.ends_with(&mnt) {
                if seq_rev.len() == mnt.len()
                    || ismatchref(
                        &seq_rev[0..seq_rev.len() - mnt.len()],
                        reference_sequences,
                        position - 1,
                        -1,
                    )
                {
                    let base_clone = sc5v.base.clone();
                    let vars_count = sc5v.base.vars_count;
                    sc5v.used = true;
                    return Some((base_clone, vars_count));
                }
            }
            None
        });
        if let Some((base_clone, vars_count)) = sc5_info {
            let vref = non_insertion_variants
                .get_mut(&position)
                .unwrap()
                .entries
                .get_mut(vn)
                .unwrap();
            adj_cnt(vref, &base_clone);
            inc_cnt(ref_coverage, position, vars_count);
        }
    }
}

/// Ported from: VariationRealigner.realigndel()
/// Java: VariationRealigner.java#L593-L858
///
/// Absorbs nearby mismatches and soft-clipped reads into known short deletions.
/// Two passes: forward (absorb mismatches/clips) and reverse (merge complex deletion variants).
/// **Parity trap T2**: bams parameter shadowing — only null-ness matters.
/// **Parity trap T12**: Pass 2 loop `i > 0` (skips element 0).
pub fn realigndel(
    bams_parameter: Option<&[String]>,
    instance_bams: &Option<Vec<String>>,
    position_to_deletions_count: &HashMap<i32, HashMap<String, i32>>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference_sequences: &HashMap<i32, u8>,
    chr: &str,
    max_read_length: i32,
) {
    let instance = GlobalReadOnlyScope::instance();

    // Java: VariationRealigner.java#L596-L600
    // **Parity trap T2**: bams shadowing — only null-ness matters
    let bams: Option<&Vec<String>> = if bams_parameter.is_none() {
        None
    } else {
        instance_bams.as_ref()
    };

    // Java: VariationRealigner.java#L603
    let tmp = fill_and_sort_tmp(position_to_deletions_count);

    // ── Pass 1: forward absorption ──
    // Java: VariationRealigner.java#L605
    for tpl in &tmp {
        let p = tpl.position;
        let vn = &tpl.description_string;
        let dcnt = tpl.count;

        if instance.conf.y {
            eprintln!(
                "  Realigndel for: {} {} {} cov: {:?}",
                p,
                vn,
                dcnt,
                ref_coverage.get(&p)
            );
        }

        // Java: VariationRealigner.java#L613 — ensure vref exists
        get_variation(non_insertion_variants, p, vn);

        // Java: VariationRealigner.java#L614-L618 — parse dellen
        let mut dellen = 0i32;
        if let Some(caps) = BEGIN_MINUS_NUMBER.captures(vn) {
            dellen = caps[1].parse::<i32>().unwrap_or(0);
        }
        if let Some(caps) = UP_NUMBER_END.captures(vn) {
            dellen += caps[1].parse::<i32>().unwrap_or(0);
        }

        // Java: VariationRealigner.java#L619-L632 — parse extra, extrains, inv5, inv3
        let mut extrains = String::new();
        let mut extra = String::new();
        let mut inv5 = String::new();
        let mut inv3 = String::new();

        if let Some(caps) = MINUS_NUMBER_ATGNC_SV_ATGNC_END.captures(vn) {
            inv5 = caps[1].to_string();
            inv3 = caps[2].to_string();
        } else if let Some(caps) = BEGIN_MINUS_NUMBER_ANY.captures(vn) {
            extra = caps[1]
                .chars()
                .filter(|c| *c != '^' && *c != '&' && *c != '#')
                .collect();
            if let Some(caps2) = CARET_ATGNC.captures(vn) {
                extrains = caps2[1].to_string();
            }
        }

        // Java: VariationRealigner.java#L634-L636
        let wustart = if p - 200 > 1 { p - 200 } else { 1 };
        let mut wupseq = format!("{}{}", join_ref(reference_sequences, wustart, p - 1), extra);
        if !inv3.is_empty() {
            wupseq = inv3.clone();
        }

        // Java: VariationRealigner.java#L638-L641
        let chr_len = instance.chr_lengths.get(chr).copied();
        let sanend = match chr_len {
            Some(cl) if p + 200 > cl => cl,
            _ => p + 200,
        };

        // Java: VariationRealigner.java#L644
        let mut sanpseq = format!(
            "{}{}",
            extra,
            join_ref(
                reference_sequences,
                p + dellen + extra.len() as i32 - extrains.len() as i32,
                sanend
            )
        );
        if !inv5.is_empty() {
            sanpseq = inv5.clone();
        }

        // Java: VariationRealigner.java#L649-L650
        let r3 = find_mm3(reference_sequences, p, &sanpseq, soft_clips_3_end);
        let r5 = find_mm5(
            reference_sequences,
            p + dellen + extra.len() as i32 - extrains.len() as i32 - 1,
            &wupseq,
            soft_clips_5_end,
        );

        let mm3 = &r3.mismatches;
        let sc3p = &r3.scp;
        let nm3 = r3.nm;
        let misp3 = r3.misp;
        let misnt3 = &r3.misnt;

        let mm5 = &r5.mismatches;
        let sc5p = &r5.scp;
        let nm5 = r5.nm;
        let misp5 = r5.misp;
        let misnt5 = &r5.misnt;

        if instance.conf.y {
            eprintln!(
                "  Mismatches: misp3: {}-{} misp5: {}-{} sclip3: {:?} sclip5: {:?}",
                misp3, misnt3, misp5, misnt5, sc3p, sc5p
            );
        }

        // Java: VariationRealigner.java#L666 — combine mismatches
        let mut mmm: Vec<Mismatch> = Vec::new();
        mmm.extend_from_slice(mm3);
        mmm.extend_from_slice(mm5);

        // Java: VariationRealigner.java#L668 — mismatch absorption loop
        for mismatch in &mmm {
            let mut mm = mismatch.mismatch_sequence.clone();
            let mp = mismatch.mismatch_position;
            let me = mismatch.end;

            // Java: VariationRealigner.java#L672 — MNP encoding
            if mm.len() > 1 {
                mm = format!("{}&{}", &mm[0..1], &mm[1..]);
            }

            // Java: VariationRealigner.java#L675
            if !non_insertion_variants.contains_key(&mp) {
                continue;
            }

            // Clone tv for borrow-checker safety
            let tv_clone = non_insertion_variants
                .get(&mp)
                .and_then(|m| m.entries.get(&mm))
                .cloned();
            let Some(tv_clone) = tv_clone else {
                continue;
            };
            if tv_clone.vars_count == 0 {
                continue;
            }
            // Java: VariationRealigner.java#L684 — quality check
            if tv_clone.mean_quality / (tv_clone.vars_count as f64) < instance.conf.goodq as f64 {
                continue;
            }
            // Java: VariationRealigner.java#L688 — position check
            let pos_threshold = if me == 3 { nm3 + 4 } else { nm5 + 4 };
            if tv_clone.mean_position / tv_clone.vars_count as f64 > pos_threshold as f64 {
                continue;
            }
            // Java: VariationRealigner.java#L691
            if tv_clone.vars_count >= dcnt + dellen || tv_clone.vars_count / dcnt >= 8 {
                continue;
            }

            if instance.conf.y {
                eprintln!(
                    "  Realigndel Adj: {} {} {} {} {} {} {} {} cov: {:?}",
                    mm,
                    mp,
                    me,
                    nm3,
                    nm5,
                    p,
                    tv_clone.vars_count,
                    tv_clone.mean_quality,
                    ref_coverage.get(&p)
                );
            }

            // Java: VariationRealigner.java#L698 — Adjust ref cnt
            if mp > p && me == 5 {
                let f = if tv_clone.mean_position != 0.0 {
                    (mp - p) as f64 / (tv_clone.mean_position / tv_clone.vars_count as f64)
                } else {
                    1.0
                };
                let f = if f > 1.0 { 1.0 } else { f };
                inc_cnt(ref_coverage, p, (tv_clone.vars_count as f64 * f) as i32);
                // Java: adjRefCnt(tv, getVariationMaybe(nonInsertionVariants, p, ref.get(p)), dellen)
                let ref_var = get_variation_maybe_mut(
                    non_insertion_variants,
                    p,
                    reference_sequences.get(&p).copied(),
                );
                adj_ref_cnt(&tv_clone, ref_var, dellen);
            }

            // Java: VariationRealigner.java#L707-L711 — lref
            let ref_base_str = reference_sequences
                .get(&p)
                .map(|b| (*b as char).to_string());
            let need_lref = mp > p && me == 3;
            let mut lref_clone: Option<Variation> = if need_lref {
                ref_base_str.as_ref().and_then(|rbs| {
                    non_insertion_variants
                        .get(&p)
                        .and_then(|m| m.entries.get(rbs))
                        .cloned()
                })
            } else {
                None
            };

            // Java: adjCnt(vref, tv, lref)
            {
                let vref = non_insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .get_mut(vn)
                    .unwrap();
                adj_cnt_with_reference(vref, &tv_clone, lref_clone.as_mut());
            }
            // Write lref back if it was modified
            if let (Some(lref), Some(rbs)) = (lref_clone, ref_base_str.as_ref()) {
                if let Some(entry) = non_insertion_variants
                    .get_mut(&p)
                    .and_then(|m| m.entries.get_mut(rbs))
                {
                    *entry = lref;
                }
            }

            // Java: VariationRealigner.java#L712-L715 — remove mm
            if let Some(map) = non_insertion_variants.get_mut(&mp) {
                map.entries.shift_remove(&mm);
                if map.entries.is_empty() {
                    non_insertion_variants.remove(&mp);
                }
            }

            if instance.conf.y {
                let vref_info = non_insertion_variants
                    .get(&p)
                    .and_then(|m| m.entries.get(vn));
                eprintln!(
                    "  Realigndel AdjA: {} {} {} {} {} {} {} {} cov: {:?}",
                    mm,
                    mp,
                    me,
                    nm3,
                    nm5,
                    p,
                    vref_info.map_or(0, |v| v.vars_count),
                    vref_info.map_or(0.0, |v| v.mean_quality),
                    ref_coverage.get(&p)
                );
            }
        }

        // Java: VariationRealigner.java#L722-L729 — single-mismatch cleanup
        if misp3 != 0
            && mm3.len() == 1
            && non_insertion_variants
                .get(&misp3)
                .and_then(|m| m.entries.get(misnt3))
                .map_or(false, |v| v.vars_count < dcnt)
        {
            if let Some(map) = non_insertion_variants.get_mut(&misp3) {
                map.entries.shift_remove(misnt3);
            }
        }
        if misp5 != 0
            && mm5.len() == 1
            && non_insertion_variants
                .get(&misp5)
                .and_then(|m| m.entries.get(misnt5))
                .map_or(false, |v| v.vars_count < dcnt)
        {
            if let Some(map) = non_insertion_variants.get_mut(&misp5) {
                map.entries.shift_remove(misnt5);
            }
        }

        // Java: VariationRealigner.java#L731-L753 — 5' soft clip absorption
        for &sc5pp in sc5p {
            let sc5_info = soft_clips_5_end.get_mut(&sc5pp).and_then(|tv| {
                if tv.used {
                    return None;
                }
                let seq = find_conseq(tv, 0);
                // Java: Make sure a couple of bogus mapping won't scoop up several fold soft-clip reads
                if dcnt <= 2 && tv.base.vars_count / dcnt > 5 {
                    return None;
                }
                if instance.conf.y {
                    let wupseq_rev: String = wupseq.chars().rev().collect();
                    eprintln!(
                        "  Realigndel 5: {} {} seq: '{}' Wuseq: {} cnt: {} {} {} {} cov: {:?}",
                        p,
                        sc5pp,
                        seq,
                        wupseq_rev,
                        tv.base.vars_count,
                        dcnt,
                        vn,
                        p,
                        ref_coverage.get(&p)
                    );
                }
                if !seq.is_empty() && ismatch(&seq, &wupseq, -1) {
                    let base_clone = tv.base.clone();
                    let vars_count = tv.base.vars_count;
                    let should_inc_ref = sc5pp > p;
                    tv.used = true;
                    if instance.conf.y {
                        let wupseq_rev: String = wupseq.chars().rev().collect();
                        eprintln!(
                            "  Realigndel 5: {} {} {} {} {} {} {} {} used cov: {:?}",
                            p,
                            sc5pp,
                            seq,
                            wupseq_rev,
                            vars_count,
                            dcnt,
                            vn,
                            p,
                            ref_coverage.get(&p)
                        );
                    }
                    Some((base_clone, vars_count, should_inc_ref))
                } else {
                    None
                }
            });
            if let Some((base_clone, vars_count, should_inc_ref)) = sc5_info {
                if should_inc_ref {
                    inc_cnt(ref_coverage, p, vars_count);
                }
                let vref = non_insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .get_mut(vn)
                    .unwrap();
                adj_cnt(vref, &base_clone);
            }
        }

        // Java: VariationRealigner.java#L755-L782 — 3' soft clip absorption
        for &sc3pp in sc3p {
            let sc3_info = soft_clips_3_end.get_mut(&sc3pp).and_then(|tv| {
                if tv.used {
                    return None;
                }
                let seq = find_conseq(tv, 0);
                if dcnt <= 2 && tv.base.vars_count / dcnt > 5 {
                    return None;
                }
                // Java: VariationRealigner.java#L766
                let sanpseq_sub =
                    String::from_utf8_lossy(&substr(sanpseq.as_bytes(), sc3pp - p)).to_string();
                if instance.conf.y {
                    eprintln!(
                        "  Realigndel 3: {} {} seq '{}' Sanseq: {} cnt: {} {} {} {} {} {}",
                        p,
                        sc3pp,
                        seq,
                        sanpseq,
                        tv.base.vars_count,
                        dcnt,
                        vn,
                        p,
                        dellen,
                        sanpseq_sub
                    );
                }
                if !seq.is_empty() && ismatch(&seq, &sanpseq_sub, 1) {
                    if instance.conf.y {
                        eprintln!(
                            "  Realigndel 3: {} {} {} {} {} {} {} {} used",
                            p, sc3pp, seq, sanpseq, tv.base.vars_count, dcnt, vn, p
                        );
                    }
                    let base_clone = tv.base.clone();
                    let vars_count = tv.base.vars_count;
                    let should_inc_ref = sc3pp <= p;
                    let need_lref = sc3pp > p;
                    tv.used = true;
                    Some((base_clone, vars_count, should_inc_ref, need_lref))
                } else {
                    None
                }
            });
            if let Some((base_clone, vars_count, should_inc_ref, need_lref)) = sc3_info {
                if should_inc_ref {
                    inc_cnt(ref_coverage, p, vars_count);
                }
                // Java: Variation lref = sc3pp <= p ? null : getVariationMaybe(...)
                let mut lref_clone: Option<Variation> = if need_lref {
                    get_variation_maybe(
                        non_insertion_variants,
                        p,
                        reference_sequences.get(&p).copied(),
                    )
                    .cloned()
                } else {
                    None
                };
                let vref = non_insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .get_mut(vn)
                    .unwrap();
                adj_cnt_with_reference(vref, &base_clone, lref_clone.as_mut());
                // Write lref back
                if let Some(lref) = lref_clone {
                    let ref_base_str = reference_sequences
                        .get(&p)
                        .map(|b| (*b as char).to_string());
                    if let Some(rbs) = ref_base_str {
                        if let Some(entry) = non_insertion_variants
                            .get_mut(&p)
                            .and_then(|m| m.entries.get_mut(&rbs))
                        {
                            *entry = lref;
                        }
                    }
                }
            }
        }

        // Java: VariationRealigner.java#L785-L793 — noPassingReads check
        let pe = p + dellen + extra.len() as i32 - extrains.len() as i32;
        let h_clone = get_variation_maybe(
            non_insertion_variants,
            p,
            reference_sequences.get(&p).copied(),
        )
        .cloned();
        if let Some(bams_ref) = bams {
            if !bams_ref.is_empty() && pe - p >= 5 && pe - p < max_read_length - 10 {
                if let Some(ref h) = h_clone {
                    if h.vars_count != 0 && no_passing_reads(chr, p, pe, bams_ref) {
                        let vref_vc = non_insertion_variants
                            .get(&p)
                            .and_then(|m| m.entries.get(vn))
                            .map(|v| v.vars_count)
                            .unwrap_or(0);
                        if vref_vc as f64
                            > 2.0
                                * h.vars_count as f64
                                * (1.0 - (pe - p) as f64 / max_read_length as f64)
                        {
                            // Java: adjCnt(vref, h, h)
                            let mut h_lref = h.clone();
                            let vref = non_insertion_variants
                                .get_mut(&p)
                                .unwrap()
                                .entries
                                .get_mut(vn)
                                .unwrap();
                            adj_cnt_with_reference(vref, h, Some(&mut h_lref));
                            // Write h_lref back
                            let ref_base_str = reference_sequences
                                .get(&p)
                                .map(|b| (*b as char).to_string());
                            if let Some(rbs) = ref_base_str {
                                if let Some(entry) = non_insertion_variants
                                    .get_mut(&p)
                                    .and_then(|m| m.entries.get_mut(&rbs))
                                {
                                    *entry = h_lref;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Pass 2: reverse merge ──
    // Java: VariationRealigner.java#L833
    // **Parity trap T12**: i > 0 skips element 0
    for i in (1..tmp.len()).rev() {
        let tpl = &tmp[i];
        let p = tpl.position;
        let vn = &tpl.description_string;

        if !non_insertion_variants.contains_key(&p) {
            continue;
        }
        let vref_clone = non_insertion_variants
            .get(&p)
            .and_then(|m| m.entries.get(vn))
            .cloned();
        let Some(vref_clone) = vref_clone else {
            continue;
        };

        // Java: VariationRealigner.java#L845
        if let Some(caps) = MINUS_NUMBER_AMP_ATGCs_END.captures(vn) {
            let tn = caps[1].to_string();
            let tref_exists_and_bigger = non_insertion_variants
                .get(&p)
                .and_then(|m| m.entries.get(&tn))
                .map_or(false, |tref| vref_clone.vars_count < tref.vars_count);

            if tref_exists_and_bigger {
                // Java: adjCnt(tref, vref), remove vn
                let tref = non_insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .get_mut(&tn)
                    .unwrap();
                adj_cnt(tref, &vref_clone);
                non_insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .shift_remove(vn);
            }
        }
    }
}

/// Ported from: VariationRealigner.realignins()
/// Java: VariationRealigner.java#L864-L1170
///
/// Absorbs nearby mismatches and soft clips into known short insertions.
/// Returns NEWINS string (modified insertion description, or empty).
/// **Parity trap T12**: Pass 2 loop `i > 0` (skips element 0).
pub fn realignins(
    position_to_insertion_count: &HashMap<i32, HashMap<String, i32>>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference_sequences: &HashMap<i32, u8>,
    chr: &str,
    max_read_length: i32,
) -> String {
    let instance = GlobalReadOnlyScope::instance();
    // Java: VariationRealigner.java#L865
    let tmp = fill_and_sort_tmp(position_to_insertion_count);
    let mut newins_result = String::new();

    // ── Pass 1: forward absorption ──
    // Java: VariationRealigner.java#L868
    for tpl in &tmp {
        let position = tpl.position;
        let vn = tpl.description_string.clone();
        let insertion_count = tpl.count;

        if instance.conf.y {
            eprintln!("  Realign Ins: {} {} {}", position, vn, insertion_count);
        }

        // Java: VariationRealigner.java#L877 — parse insert
        let insert = match BEGIN_PLUS_ATGC.captures(&vn) {
            Some(caps) => caps[1].to_string(),
            None => continue,
        };

        // Java: VariationRealigner.java#L883 — parse ins3, inslen
        let mut ins3 = String::new();
        let mut inslen = insert.len() as i32;
        if let Some(caps) = DUP_NUM_ATGC.captures(&vn) {
            ins3 = caps[2].to_string();
            inslen += caps[1].parse::<i32>().unwrap_or(0) + ins3.len() as i32;
        }

        // Java: VariationRealigner.java#L889 — parse extra
        let mut extra = String::new();
        if let Some(caps) = AMP_ATGC.captures(&vn) {
            extra = caps[1].to_string();
        }

        // Java: VariationRealigner.java#L893
        let mut compm = String::new();
        if let Some(caps) = HASH_ATGC.captures(&vn) {
            compm = caps[1].to_string();
        }

        // Java: VariationRealigner.java#L898
        let mut _newins = String::new();
        if let Some(caps) = CARET_ATGC_END.captures(&vn) {
            _newins = caps[1].to_string();
        }

        // Java: VariationRealigner.java#L903
        let mut newdel = 0i32;
        if let Some(caps) = UP_NUMBER_END.captures(&vn) {
            newdel = caps[1].parse::<i32>().unwrap_or(0);
        }

        // Java: VariationRealigner.java#L908-L912 — build tn
        let tn = build_tn(&vn);

        // Java: VariationRealigner.java#L914-L915
        let wustart = if position - 150 > 1 {
            position - 150
        } else {
            1
        };
        let wupseq = format!("{}{}", join_ref(reference_sequences, wustart, position), tn);

        // Java: VariationRealigner.java#L921-L922
        let chr_len = instance.chr_lengths.get(chr).copied();
        let mut sanend = position + vn.len() as i32 + 100;
        if let Some(tend) = chr_len {
            if tend < sanend {
                sanend = tend;
            }
        }

        // Java: VariationRealigner.java#L933-L943
        let sanpseq;
        let find_mm3_result;
        if !ins3.is_empty() {
            let p3 = position + inslen - ins3.len() as i32 + crate::config::SVFLANK;
            let mut sp = String::new();
            if ins3.len() as i32 > crate::config::SVFLANK {
                let sub = substr(ins3.as_bytes(), crate::config::SVFLANK - ins3.len() as i32);
                sp = String::from_utf8_lossy(&sub).to_string();
            }
            sp += &join_ref(reference_sequences, position + 1, position + 101);
            sanpseq = sp;
            find_mm3_result = find_mm3(reference_sequences, p3 + 1, &sanpseq, soft_clips_3_end);
        } else {
            sanpseq = format!(
                "{}{}",
                tn,
                join_ref(
                    reference_sequences,
                    position + extra.len() as i32 + 1 + compm.len() as i32 + newdel,
                    sanend
                )
            );
            find_mm3_result = find_mm3(
                reference_sequences,
                position + 1,
                &sanpseq,
                soft_clips_3_end,
            );
        }

        // Java: VariationRealigner.java#L946
        let find_mm5_result = find_mm5(
            reference_sequences,
            position + extra.len() as i32 + compm.len() as i32 + newdel,
            &wupseq,
            soft_clips_5_end,
        );

        let mm3 = &find_mm3_result.mismatches;
        let sc3p = &find_mm3_result.scp;
        let nm3 = find_mm3_result.nm;
        let misp3 = find_mm3_result.misp;
        let misnt3 = &find_mm3_result.misnt;

        let mm5 = &find_mm5_result.mismatches;
        let sc5p = &find_mm5_result.scp;
        let nm5 = find_mm5_result.nm;
        let misp5 = find_mm5_result.misp;
        let misnt5 = &find_mm5_result.misnt;

        // Java: VariationRealigner.java#L961
        let mut mmm: Vec<Mismatch> = Vec::new();
        mmm.extend_from_slice(mm3);
        mmm.extend_from_slice(mm5);

        // Java: VariationRealigner.java#L962 — ensure vref exists in insertionVariants
        get_variation(insertion_variants, position, &vn);

        // Java: VariationRealigner.java#L963 — mismatch absorption loop
        for mismatch in &mmm {
            let mut mismatch_bases = mismatch.mismatch_sequence.clone();
            let mismatch_position = mismatch.mismatch_position;
            let mismatch_end = mismatch.end;

            // Java: VariationRealigner.java#L970 — MNP encoding
            if mismatch_bases.len() > 1 {
                mismatch_bases = format!("{}&{}", &mismatch_bases[0..1], &mismatch_bases[1..]);
            }
            if !non_insertion_variants.contains_key(&mismatch_position) {
                continue;
            }

            // Clone variation for borrow-checker safety
            let variation_clone = non_insertion_variants
                .get(&mismatch_position)
                .and_then(|m| m.entries.get(&mismatch_bases))
                .cloned();
            let Some(variation_clone) = variation_clone else {
                continue;
            };
            if variation_clone.vars_count == 0 {
                continue;
            }
            if variation_clone.mean_quality / (variation_clone.vars_count as f64)
                < instance.conf.goodq as f64
            {
                continue;
            }
            let pos_threshold = if mismatch_end == 3 { nm3 + 4 } else { nm5 + 4 };
            if variation_clone.mean_position / variation_clone.vars_count as f64
                > pos_threshold as f64
            {
                continue;
            }
            // Parity guard: Java's realignins wraps this body in try/catch
            // (VariationRealigner.java#L676,#L937). With insertion_count == 0 the Java
            // expression throws ArithmeticException and the outer catch continues. Rust
            // must short-circuit BEFORE the division; we treat insertion_count == 0 as
            // "skip this mismatch" which is observationally equivalent for parity output
            // (post-mismatch-loop work does not depend on insertion_count).
            if variation_clone.vars_count >= insertion_count + insert.len() as i32
                || insertion_count == 0
                || variation_clone.vars_count / insertion_count >= 8
            {
                continue;
            }

            if instance.conf.y {
                eprintln!(
                    "    insMM: {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:?}",
                    mismatch_bases,
                    mismatch_position,
                    mismatch_end,
                    nm3,
                    nm5,
                    vn,
                    insertion_count,
                    variation_clone.vars_count,
                    variation_clone.mean_quality,
                    variation_clone.mean_position,
                    ref_coverage.get(&position)
                );
            }

            // Java: VariationRealigner.java#L998 — Adjust ref cnt
            if mismatch_position > position && mismatch_end == 5 {
                inc_cnt(ref_coverage, position, variation_clone.vars_count);
            }

            // Java: VariationRealigner.java#L1001-L1009 — lref
            let ref_base_str = reference_sequences
                .get(&position)
                .map(|b| (*b as char).to_string());
            let mut lref_clone: Option<Variation> = if mismatch_position > position
                && mismatch_end == 3
                && non_insertion_variants.contains_key(&position)
                && reference_sequences.contains_key(&position)
            {
                ref_base_str.as_ref().and_then(|rbs| {
                    non_insertion_variants
                        .get(&position)
                        .and_then(|m| m.entries.get(rbs))
                        .cloned()
                })
            } else {
                None
            };

            // Java: adjCnt(vref, variation, lref)
            {
                let vref = insertion_variants
                    .get_mut(&position)
                    .unwrap()
                    .entries
                    .get_mut(&vn)
                    .unwrap();
                adj_cnt_with_reference(vref, &variation_clone, lref_clone.as_mut());
            }
            // Write lref back to nonInsertionVariants
            if let (Some(lref), Some(rbs)) = (lref_clone, ref_base_str.as_ref()) {
                if let Some(entry) = non_insertion_variants
                    .get_mut(&position)
                    .and_then(|m| m.entries.get_mut(rbs))
                {
                    *entry = lref;
                }
            }

            // Java: VariationRealigner.java#L1011-L1014
            if let Some(map) = non_insertion_variants.get_mut(&mismatch_position) {
                map.entries.shift_remove(&mismatch_bases);
                if map.entries.is_empty() {
                    non_insertion_variants.remove(&mismatch_position);
                }
            }
        }

        // Java: VariationRealigner.java#L1016-L1023 — single-mismatch cleanup
        if misp3 != 0
            && mm3.len() == 1
            && non_insertion_variants
                .get(&misp3)
                .and_then(|m| m.entries.get(misnt3))
                .map_or(false, |v| v.vars_count < insertion_count)
        {
            if let Some(map) = non_insertion_variants.get_mut(&misp3) {
                map.entries.shift_remove(misnt3);
            }
        }
        if misp5 != 0
            && mm5.len() == 1
            && non_insertion_variants
                .get(&misp5)
                .and_then(|m| m.entries.get(misnt5))
                .map_or(false, |v| v.vars_count < insertion_count)
        {
            if let Some(map) = non_insertion_variants.get_mut(&misp5) {
                map.entries.shift_remove(misnt5);
            }
        }

        // Java: VariationRealigner.java#L1024-L1049 — 5' soft clip absorption
        for &sc5pp in sc5p {
            if instance.conf.y {
                eprintln!(
                    "    55: {} {} VN: '{}'  5' seq: ^{}^",
                    position, sc5pp, vn, wupseq
                );
            }
            let sc5_info = soft_clips_5_end.get_mut(&sc5pp).and_then(|tv| {
                if tv.used {
                    return None;
                }
                let seq = find_conseq(tv, 0);
                if instance.conf.y {
                    eprintln!(
                        "    ins5: {} {} {} {} VN: {} iCnt: {} vCnt: {}",
                        position, sc5pp, seq, wupseq, vn, insertion_count, tv.base.vars_count
                    );
                }
                if !seq.is_empty() && ismatch(&seq, &wupseq, -1) {
                    if instance.conf.y {
                        eprintln!(
                            "      ins5: {} {} {} {} VN: {} iCnt: {} cCnt: {} used",
                            position, sc5pp, seq, wupseq, vn, insertion_count, tv.base.vars_count
                        );
                    }
                    let base_clone = tv.base.clone();
                    let vars_count = tv.base.vars_count;
                    let should_inc_ref = sc5pp > position;
                    tv.used = true;
                    Some((base_clone, vars_count, should_inc_ref))
                } else {
                    None
                }
            });
            if let Some((base_clone, vars_count, should_inc_ref)) = sc5_info {
                if should_inc_ref {
                    inc_cnt(ref_coverage, position, vars_count);
                }
                let vref = insertion_variants
                    .get_mut(&position)
                    .unwrap()
                    .entries
                    .get_mut(&vn)
                    .unwrap();
                adj_cnt(vref, &base_clone);
            }
        }

        // Java: VariationRealigner.java#L1050-L1117 — 3' soft clip absorption
        for &sc3pp in sc3p {
            if instance.conf.y {
                eprintln!(
                    "    33: {} {} VN: '{}'  3' seq: ^{}^",
                    position, sc3pp, vn, sanpseq
                );
            }
            let sc3_info = soft_clips_3_end.get_mut(&sc3pp).and_then(|tv| {
                if tv.used {
                    return None;
                }
                let seq = find_conseq(tv, 0);
                if instance.conf.y {
                    eprintln!(
                        "    ins3: {} {} {} {} VN: {} iCnt: {} vCnt: {}",
                        position, sc3pp, seq, sanpseq, vn, insertion_count, tv.base.vars_count
                    );
                }
                // Java: String mseq = !ins3.isEmpty() ? sanpseq : substr(sanpseq, sc3pp - position - 1)
                let mseq = if !ins3.is_empty() {
                    sanpseq.clone()
                } else {
                    String::from_utf8_lossy(&substr(sanpseq.as_bytes(), sc3pp - position - 1))
                        .to_string()
                };
                if !seq.is_empty() && ismatch(&seq, &mseq, 1) {
                    if instance.conf.y {
                        eprintln!(
                            "      ins3: {} {} {} VN: {} iCnt: {} vCnt: {} used",
                            position, sc3pp, seq, vn, insertion_count, tv.base.vars_count
                        );
                    }
                    let base_clone = tv.base.clone();
                    let vars_count = tv.base.vars_count;
                    let mean_pos = tv.base.mean_position;
                    let should_inc_ref =
                        sc3pp <= position || insert.len() as f64 > mean_pos / vars_count as f64;
                    let need_lref = sc3pp > position;
                    let null_lref_if_insert_gt = insert.len() as f64 > mean_pos / vars_count as f64;
                    tv.used = true;

                    // Java: VariationRealigner.java#L1086-L1117 — NEWINS mutation
                    // Check happens inside the closure while we still hold &mut tv
                    let newins_info = if insert.len() + 1 == vn.len()
                        && insert.len() as i32 > max_read_length
                        && sc3pp >= position + 1 + insert.len() as i32
                    {
                        let mut flag = 0i32;
                        let offset = ((sc3pp - position - 1) as usize) % insert.len();
                        let mut tvn_bytes = vn.as_bytes().to_vec();
                        for seqi in 0..seq.len() {
                            if seqi + offset >= insert.len() {
                                break;
                            }
                            if seq.as_bytes()[seqi] != insert.as_bytes()[seqi + offset] {
                                flag += 1;
                                let shift = seqi + offset + 1;
                                if shift < tvn_bytes.len() {
                                    tvn_bytes[shift] = seq.as_bytes()[seqi];
                                }
                            }
                        }
                        if flag > 0 {
                            Some(String::from_utf8_lossy(&tvn_bytes).to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    Some((
                        base_clone,
                        vars_count,
                        should_inc_ref,
                        need_lref,
                        null_lref_if_insert_gt,
                        newins_info,
                    ))
                } else {
                    None
                }
            });
            if let Some((
                base_clone,
                vars_count,
                should_inc_ref,
                need_lref,
                null_lref_if_insert_gt,
                newins_info,
            )) = sc3_info
            {
                if should_inc_ref {
                    inc_cnt(ref_coverage, position, vars_count);
                }
                // Java: lref logic
                let ref_base_str = reference_sequences
                    .get(&position)
                    .map(|b| (*b as char).to_string());
                let mut lref_clone: Option<Variation> = if need_lref && !null_lref_if_insert_gt {
                    ref_base_str.as_ref().and_then(|rbs| {
                        non_insertion_variants
                            .get(&position)
                            .and_then(|m| m.entries.get(rbs))
                            .cloned()
                    })
                } else {
                    None
                };
                {
                    let vref = insertion_variants
                        .get_mut(&position)
                        .unwrap()
                        .entries
                        .get_mut(&vn)
                        .unwrap();
                    adj_cnt_with_reference(vref, &base_clone, lref_clone.as_mut());
                }
                // Write lref back
                if let (Some(lref), Some(rbs)) = (lref_clone, ref_base_str.as_ref()) {
                    if let Some(entry) = non_insertion_variants
                        .get_mut(&position)
                        .and_then(|m| m.entries.get_mut(rbs))
                    {
                        *entry = lref;
                    }
                }

                // Java: VariationRealigner.java#L1086-L1107 — NEWINS key rename
                if let Some(tvn) = newins_info {
                    if let Some(map) = insertion_variants.get_mut(&position) {
                        if let Some((_, variation)) = map.entries.shift_remove_entry(&vn) {
                            map.entries.insert(tvn.clone(), variation);
                            newins_result = tvn;
                        }
                    }
                }
            }
        }

        // Java: VariationRealigner.java#L1118-L1130 — adjRefFactor
        let first3 = sc3p.first().copied().unwrap_or(0);
        let first5 = sc5p.first().copied().unwrap_or(0);
        if !sc3p.is_empty()
            && !sc5p.is_empty()
            && first3 > first5 + 3
            // Match Java's mixed int/double comparison exactly; truncating the
            // 0.75 threshold suppresses boundary-case ref-factor adjustments.
            && ((first3 - first5) as f64) < max_read_length as f64 * 0.75
        {
            let ref_base_str = reference_sequences
                .get(&position)
                .map(|b| (*b as char).to_string());
            let factor = (first3 - first5 - 1) as f64 / max_read_length as f64;

            // Java: adjRefFactor(nonInsertionVariants.get(position).get(ref.get(position).toString()), factor)
            if let Some(rbs) = &ref_base_str {
                if non_insertion_variants
                    .get(&position)
                    .and_then(|m| m.entries.get(rbs))
                    .is_some()
                {
                    let ref_var = non_insertion_variants
                        .get_mut(&position)
                        .unwrap()
                        .entries
                        .get_mut(rbs)
                        .unwrap();
                    adj_ref_factor(Some(ref_var), factor);
                }
            }
            // Java: adjRefFactor(vref, -factor)
            if let Some(vref) = insertion_variants
                .get_mut(&position)
                .and_then(|m| m.entries.get_mut(&vn))
            {
                adj_ref_factor(Some(vref), -factor);
            }
        }
    }

    // ── Pass 2: reverse merge ──
    // Java: VariationRealigner.java#L1146
    // **Parity trap T12**: i > 0 skips element 0
    for i in (1..tmp.len()).rev() {
        let tpl = &tmp[i];
        let p = tpl.position;
        let vn = &tpl.description_string;

        if !insertion_variants.contains_key(&p) {
            continue;
        }
        let vref_clone = insertion_variants
            .get(&p)
            .and_then(|m| m.entries.get(vn))
            .cloned();
        let Some(vref_clone) = vref_clone else {
            continue;
        };

        // Java: VariationRealigner.java#L1158
        if let Some(caps) = ATGSs_AMP_ATGSs_END.captures(vn) {
            let tn = caps[1].to_string();
            let tref_exists_and_bigger = insertion_variants
                .get(&p)
                .and_then(|m| m.entries.get(&tn))
                .map_or(false, |tref| vref_clone.vars_count < tref.vars_count);

            if tref_exists_and_bigger {
                // Java: adjCnt(tref, vref, getVariationMaybe(nonInsertionVariants, p, ref.get(p)))
                let mut lref_clone = get_variation_maybe(
                    non_insertion_variants,
                    p,
                    reference_sequences.get(&p).copied(),
                )
                .cloned();
                let tref = insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .get_mut(&tn)
                    .unwrap();
                adj_cnt_with_reference(tref, &vref_clone, lref_clone.as_mut());
                // Write lref back
                if let Some(lref) = lref_clone {
                    let ref_base_str = reference_sequences
                        .get(&p)
                        .map(|b| (*b as char).to_string());
                    if let Some(rbs) = ref_base_str {
                        if let Some(entry) = non_insertion_variants
                            .get_mut(&p)
                            .and_then(|m| m.entries.get_mut(&rbs))
                        {
                            *entry = lref;
                        }
                    }
                }
                insertion_variants
                    .get_mut(&p)
                    .unwrap()
                    .entries
                    .shift_remove(vn);
            }
        }
    }

    newins_result
}

/// Helper: build `tn` from `vn` by stripping markers.
/// Java: vn.replaceFirst("^\\+", "").replaceFirst("&", "").replaceFirst("#", "")
///       .replaceFirst("\\^\\d+$", "").replaceFirst("\\^", "")
fn build_tn(vn: &str) -> String {
    let mut tn = vn.to_string();
    // replaceFirst("^\\+", "")
    if tn.starts_with('+') {
        tn = tn[1..].to_string();
    }
    // replaceFirst("&", "")
    if let Some(pos) = tn.find('&') {
        tn.remove(pos);
    }
    // replaceFirst("#", "")
    if let Some(pos) = tn.find('#') {
        tn.remove(pos);
    }
    // replaceFirst("\\^\\d+$", "")
    if let Some(caret_pos) = tn.rfind('^') {
        let after = &tn[caret_pos + 1..];
        if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
            tn.truncate(caret_pos);
        }
    }
    // replaceFirst("\\^", "")
    if let Some(pos) = tn.find('^') {
        tn.remove(pos);
    }
    tn
}

// ─── Cluster D helpers ──────────────────────────────────────────────

/// Get-or-create SV metadata for a position in non_insertion_variants.
/// Java: VariationMap.getSV()
fn get_sv(
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    pos: i32,
) -> &mut VariationMapSV {
    let vmap = non_insertion_variants
        .entry(pos)
        .or_insert_with(VariationMap::default);
    if vmap.sv.is_none() {
        vmap.sv = Some(VariationMapSV::default());
        vmap.entries.insert("SV".to_string(), Variation::default());
    }
    if !vmap.entries.contains_key("SV") {
        vmap.entries.insert("SV".to_string(), Variation::default());
    }
    vmap.sv.as_mut().unwrap()
}

/// Remove SV metadata at a position. Java: VariationMap.removeSV()
fn remove_sv(non_insertion_variants: &mut HashMap<i32, VariationMap>, pos: i32) {
    if let Some(vmap) = non_insertion_variants.get_mut(&pos) {
        vmap.sv = None;
        vmap.entries.shift_remove("SV");
    }
}

struct PartialPipelineContext<'a> {
    bam: &'a str,
    region: &'a Region,
    reference_resource: &'a Arc<ReferenceResource>,
    splice: &'a HashSet<String>,
    out: &'a Arc<VariantPrinter>,
}

/// Ported from: AbstractMode.partialPipeline()
/// Java: AbstractMode.java:L79-L85
///
/// Re-enters SAMFileParser + CigarParser(true) on an adjacent boundary region and writes the
/// updated maps back into the live realigner state.
fn run_partial_pipeline(
    context: &PartialPipelineContext<'_>,
    modified_start: i32,
    modified_end: i32,
    max_read_length: i32,
    reference: &mut Reference,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
) {
    if modified_start > modified_end {
        return;
    }

    let modified_region = Region::new_modified_region(context.region, modified_start, modified_end);
    if !ReferenceResource::is_loaded(
        &modified_region.chr,
        modified_region.start,
        modified_region.end,
        reference,
    ) {
        let current_reference = std::mem::take(reference);
        *reference = context
            .reference_resource
            .get_reference_with_extension(&modified_region, max_read_length, current_reference)
            .unwrap_or_else(|error| {
                panic!(
                    "Failed to fetch reference for {}: {}",
                    modified_region.print_region(),
                    error
                )
            });
    }

    let initial_data = InitialData::new(
        std::mem::take(non_insertion_variants),
        std::mem::take(insertion_variants),
        std::mem::take(ref_coverage),
        std::mem::take(soft_clips_3_end),
        std::mem::take(soft_clips_5_end),
    );
    let scope = Scope {
        bam: context.bam.to_string(),
        region: modified_region,
        region_ref: Arc::new(reference.clone()),
        reference_resource: Arc::clone(context.reference_resource),
        max_read_length,
        splice: Arc::new(context.splice.clone()),
        out: Arc::clone(context.out),
        data: initial_data,
    };
    let parsed_scope = crate::mods::sam_file_parser::sam_file_parser_process(scope);
    let Scope {
        region,
        region_ref,
        max_read_length,
        splice,
        mut data,
        ..
    } = parsed_scope;
    let header = data
        .header_view()
        .expect("current BAM header must exist before partial CIGAR parsing");
    let chr_name =
        crate::mods::sam_file_parser::get_chr_name(&region, &GlobalReadOnlyScope::instance().conf);
    let InitialData {
        non_insertion_variants: partial_non_insertion_variants,
        insertion_variants: partial_insertion_variants,
        ref_coverage: partial_ref_coverage,
        soft_clips_5_end: partial_soft_clips_5_end,
        soft_clips_3_end: partial_soft_clips_3_end,
    } = std::mem::take(&mut data.initial_data);
    let total_reads = data.total_reads;
    let duplicate_reads = data.duplicate_reads;
    let mut parser = crate::mods::cigar_parser::CigarParser::new(true);
    parser.init_from_scope(
        &region,
        &region_ref,
        &splice,
        max_read_length,
        partial_non_insertion_variants,
        partial_insertion_variants,
        partial_ref_coverage,
        partial_soft_clips_3_end,
        partial_soft_clips_5_end,
        total_reads,
        duplicate_reads,
    );
    let variation_data = parser.process_preprocessor(&mut data, &header, &chr_name);
    *non_insertion_variants = variation_data.non_insertion_variants;
    *insertion_variants = variation_data.insertion_variants;
    *ref_coverage = variation_data.ref_coverage;
    *soft_clips_3_end = variation_data.soft_clips_3_end;
    *soft_clips_5_end = variation_data.soft_clips_5_end;
}

// ─── Cluster D implementation ───────────────────────────────────────

/// Ported from: VariationRealigner.realignlgdel()
/// Java: VariationRealigner.java#L973-L1594
///
/// Discovers large deletions from unpaired soft clips.
/// Two asymmetric passes: 5' (softClips5End) then 3' (softClips3End).
fn realignlgdel(
    instance_bams: &Option<Vec<String>>,
    svfdel: &mut Vec<Sclip>,
    svrdel: &mut Vec<Sclip>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference: &mut Reference,
    chr: &str,
    max_read_length: i32,
    region_start: i32,
    region_end: i32,
    partial_pipeline_context: &PartialPipelineContext<'_>,
) {
    let instance = GlobalReadOnlyScope::instance();
    let mut reference_sequences = &reference.reference_sequences;
    // Java: VariationRealigner.java#L975
    let longmm = 3i32;

    // ── 5' pass ──────────────────────────────────────────────────────
    // Java: VariationRealigner.java#L978-L988
    let mut tmp: Vec<SortPositionSclip> = Vec::new();
    for (&p, sc) in soft_clips_5_end.iter() {
        if p < region_start - EXTENSION || p > region_end + EXTENSION {
            continue;
        }
        tmp.push(SortPositionSclip::new(p, sc.clone(), 0));
    }
    tmp.sort_by(comp2);

    let mut svcov = 0i32;
    let mut clusters = 0i32;
    let mut pairs = 0i32;

    // Java: VariationRealigner.java#L993
    for t in &tmp {
        let p = t.position;
        let cnt;
        let seq;
        {
            // Must access the live sclip from the map (not the snapshot clone)
            let sc5v = match soft_clips_5_end.get_mut(&p) {
                Some(s) => s,
                None => continue,
            };
            cnt = sc5v.base.vars_count;

            // Java: VariationRealigner.java#L999
            if cnt < instance.conf.minr {
                break;
            }
            // Java: VariationRealigner.java#L1002
            if sc5v.used {
                continue;
            }
            // Java: VariationRealigner.java#L1005
            seq = find_conseq(sc5v, 5);
        }
        if seq.is_empty() {
            continue;
        }
        if seq.len() < 7 {
            continue;
        }

        if instance.conf.y {
            eprintln!("  Working Realignlgdel: 5' {} '{}' {}", p, seq, cnt);
        }

        // Java: VariationRealigner.java#L1016
        let mut bp = findbp(&seq, p - 5, reference_sequences, -1, chr);

        let mut extra = String::new();
        let mut extra_upper = String::new(); // Java: EXTRA

        // Java: VariationRealigner.java#L1021 — if bp == 0, try findMatch
        if bp == 0 {
            if islowcomplexseq(&seq) {
                continue;
            }
            // Java: VariationRealigner.java#L1026 — findMatch
            let match_result =
                super::structural_variants_processor::find_match(&seq, reference, p, -1, SEED_1, 1);
            bp = match_result.base_position;
            extra_upper = match_result.matched_sequence;
            // Java: VariationRealigner.java#L1029
            if !(bp != 0 && p - bp > 15 && p - bp < SVMAXLEN) {
                continue;
            }
            bp += 1; // Java: VariationRealigner.java#L1033
            // Java: VariationRealigner.java#L1034 — markSV
            let sv_mark = super::structural_variants_processor::mark_sv(
                bp,
                p,
                &mut [&mut *svfdel, &mut *svrdel],
                max_read_length,
            );
            svcov = sv_mark.0;
            clusters = sv_mark.1;
            pairs = sv_mark.2;
            if svcov == 0 {
                if cnt <= instance.conf.minr {
                    continue;
                }
            }

            // Java: VariationRealigner.java#L1044-L1049
            {
                let sv = get_sv(non_insertion_variants, bp);
                sv.type_ = Some("DEL".to_string());
                sv.pairs += pairs;
                sv.splits += cnt;
                sv.clusters += clusters;
            }

            // Java: VariationRealigner.java#L1051 — partialPipeline if bp < region.start
            if bp < region_start {
                let tts = bp - max_read_length;
                let mut tte = bp + max_read_length;
                if bp + max_read_length >= region_start {
                    tte = region_start - 1;
                }
                run_partial_pipeline(
                    partial_pipeline_context,
                    tts,
                    tte,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
                reference_sequences = &reference.reference_sequences;
            }
        }

        // Java: VariationRealigner.java#L1067
        let mut dellen = p - bp;
        let mut en = 0i32;
        let mut gt = format!("-{}", dellen);

        // Java: VariationRealigner.java#L1070-L1080
        if extra_upper.is_empty() {
            let seq_bytes = seq.as_bytes();
            while (en as usize) < seq_bytes.len()
                && is_not_equals(
                    Some(seq_bytes[en as usize]),
                    reference_sequences.get(&(bp - en - 1)).copied(),
                )
            {
                extra.push(seq_bytes[en as usize] as char);
                en += 1;
            }
            if !extra.is_empty() {
                let extra_rev: String = extra.chars().rev().collect();
                extra = extra_rev;
                gt = format!("-{}&{}", dellen, extra);
                bp -= extra.len() as i32;
            }
        } else {
            // Java mutates dellen before the short-del coverage update below.
            dellen -= extra_upper.len() as i32;
            if dellen == 0 {
                gt = format!("-{}^{}", extra_upper.len(), extra_upper);
            } else {
                gt = format!("-{}&{}", dellen, extra_upper);
            }
        }

        if instance.conf.y {
            eprintln!(
                "  Found Realignlgdel: {} {} 5' {} {} {}",
                bp, gt, p, seq, cnt
            );
        }

        // Java: VariationRealigner.java#L1091-L1098 — Find matching 3' clip position
        let mut n = 0i32;
        if extra.is_empty() && extra_upper.is_empty() {
            while reference_sequences.contains_key(&(bp + n))
                && reference_sequences.contains_key(&(bp + dellen + n))
                && is_equals(
                    reference_sequences.get(&(bp + n)).copied(),
                    reference_sequences.get(&(bp + dellen + n)).copied(),
                )
            {
                n += 1;
            }
        }
        let mut sc3p = bp + n;

        // Java: VariationRealigner.java#L1099-L1112 — mismatch walk
        let mut str_buf = String::new();
        let mut mcnt = 0i32;
        while mcnt <= longmm
            && reference_sequences.contains_key(&(bp + n))
            && reference_sequences.contains_key(&(bp + dellen + n))
            && is_not_equals(
                reference_sequences.get(&(bp + n)).copied(),
                reference_sequences.get(&(bp + dellen + n)).copied(),
            )
        {
            if let Some(&ch) = reference_sequences.get(&(bp + dellen + n)) {
                str_buf.push(ch as char);
            }
            n += 1;
            mcnt += 1;
        }

        // Java: VariationRealigner.java#L1113-L1122
        if str_buf.len() == 1 {
            let mut nm = 0i32;
            while reference_sequences.contains_key(&(bp + n))
                && reference_sequences.contains_key(&(bp + dellen + n))
                && is_equals(
                    reference_sequences.get(&(bp + n)).copied(),
                    reference_sequences.get(&(bp + dellen + n)).copied(),
                )
            {
                n += 1;
                if n != 0 {
                    nm += 1;
                }
            }
            if nm >= 3 && !soft_clips_3_end.contains_key(&sc3p) {
                sc3p = bp + n;
            }
        }

        // Java: VariationRealigner.java#L1125-L1130 — likely false positive check
        let has_sv_at_bp = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.sv.as_ref())
            .is_some();
        if has_sv_at_bp && !soft_clips_3_end.contains_key(&sc3p) {
            if svcov == 0 && cnt <= instance.conf.minr {
                remove_sv(non_insertion_variants, bp);
                continue;
            }
        }

        // Java: VariationRealigner.java#L1132
        let tv = get_variation(non_insertion_variants, bp, &gt);
        tv.qstd = true;
        tv.pstd = true;

        // Java: VariationRealigner.java#L1137 — adjCnt(tv, sc5v)
        {
            let sc5v_base = soft_clips_5_end.get(&p).map(|s| s.base.clone());
            if let Some(base) = sc5v_base {
                let tv_ref = get_variation(non_insertion_variants, bp, &gt);
                adj_cnt(tv_ref, &base);
            }
        }

        // Java: VariationRealigner.java#L1139 — sc5v.used = bp != 0
        // **Parity trap T8**: 5' pass uses bp != 0
        if let Some(sc5v) = soft_clips_5_end.get_mut(&p) {
            sc5v.used = bp != 0;
        }

        // Java: VariationRealigner.java#L1141-L1143 — refCoverage fallback
        if !ref_coverage.contains_key(&bp) {
            if let Some(&cov) = ref_coverage.get(&p) {
                ref_coverage.insert(bp, cov);
            }
        }

        // Java: VariationRealigner.java#L1144-L1148 — incCnt for short dels
        let sc5v_vars_count = soft_clips_5_end
            .get(&p)
            .map(|s| s.base.vars_count)
            .unwrap_or(0);
        if dellen < instance.conf.indelsize {
            for tp in bp..(bp + dellen) {
                inc_cnt(ref_coverage, tp, sc5v_vars_count);
            }
        }

        // Java: VariationRealigner.java#L1150-L1175 — matching 3' clip
        if soft_clips_3_end.get(&sc3p).map_or(false, |s| !s.used) {
            let sclip_base_clone;
            let sclip_vars_count;
            let sc3p_gt_bp;
            {
                let sclip = soft_clips_3_end.get(&sc3p).unwrap();
                sclip_base_clone = sclip.base.clone();
                sclip_vars_count = sclip.base.vars_count;
                sc3p_gt_bp = sc3p > bp;
            }

            // Java: VariationRealigner.java#L1153-L1158
            if sc3p_gt_bp {
                let lref = get_variation_maybe(
                    non_insertion_variants,
                    bp,
                    reference_sequences.get(&bp).copied(),
                )
                .cloned();
                let tv_ref = get_variation(non_insertion_variants, bp, &gt);
                let mut lref_opt = lref;
                adj_cnt_with_reference(tv_ref, &sclip_base_clone, lref_opt.as_mut());
                // Write lref back if modified
                if let Some(lref_val) = lref_opt {
                    let ref_base_str = reference_sequences
                        .get(&bp)
                        .map(|b| (*b as char).to_string());
                    if let Some(rbs) = ref_base_str {
                        if let Some(entry) = non_insertion_variants
                            .get_mut(&bp)
                            .and_then(|m| m.entries.get_mut(&rbs))
                        {
                            *entry = lref_val;
                        }
                    }
                }
            } else {
                let tv_ref = get_variation(non_insertion_variants, bp, &gt);
                adj_cnt(tv_ref, &sclip_base_clone);
            }

            // Java: VariationRealigner.java#L1160-L1165
            if sc3p == bp {
                if dellen < instance.conf.indelsize {
                    for tp in bp..(bp + dellen) {
                        inc_cnt(ref_coverage, tp, sclip_vars_count);
                    }
                }
            }

            // Java: VariationRealigner.java#L1167-L1175 — rmCnt for intermediate positions
            for ip in (bp + 1)..sc3p {
                let ref_ch = reference_sequences
                    .get(&(dellen + ip))
                    .map(|b| (*b as char).to_string());
                if let Some(ref ref_ch_str) = ref_ch {
                    let sclip_base_for_rm = sclip_base_clone.clone();
                    let vv = get_variation(non_insertion_variants, ip, ref_ch_str);
                    rm_cnt(vv, &sclip_base_for_rm);
                    let vv_vc = non_insertion_variants
                        .get(&ip)
                        .and_then(|m| m.entries.get(ref_ch_str))
                        .map(|v| v.vars_count)
                        .unwrap_or(0);
                    if vv_vc == 0 {
                        if let Some(map) = non_insertion_variants.get_mut(&ip) {
                            map.entries.shift_remove(ref_ch_str);
                        }
                    }
                    let map_empty = non_insertion_variants
                        .get(&ip)
                        .map_or(true, |m| m.entries.is_empty());
                    if map_empty {
                        non_insertion_variants.remove(&ip);
                    }
                }
            }

            // Java: VariationRealigner.java#L1176 — sclip.used = bp != 0
            if let Some(sclip) = soft_clips_3_end.get_mut(&sc3p) {
                sclip.used = bp != 0;
            }
        }

        // Java: VariationRealigner.java#L1178-L1179 — recursive realigndel with singleton map
        let tv_vc = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.entries.get(&gt))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        let mut dels5: HashMap<i32, HashMap<String, i32>> = HashMap::new();
        let mut inner = HashMap::new();
        inner.insert(gt.clone(), tv_vc);
        dels5.insert(bp, inner);
        realigndel(
            instance_bams.as_ref().map(|v| v.as_slice()),
            instance_bams,
            &dels5,
            non_insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference_sequences,
            chr,
            max_read_length,
        );

        // Java: VariationRealigner.java#L1181-L1183 — SV splits update
        let tv_vc_after_splits = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.entries.get(&gt))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        if let Some(sv) = non_insertion_variants
            .get_mut(&bp)
            .and_then(|m| m.sv.as_mut())
        {
            sv.splits += tv_vc_after_splits - tv_vc;
        }

        // Java: VariationRealigner.java#L1185-L1187 — addVarFactor
        let tv_vc_after = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.entries.get(&gt))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        if svcov > tv_vc_after {
            let factor = (svcov - tv_vc_after) as f64 / tv_vc_after as f64;
            if let Some(tv_ref) = non_insertion_variants
                .get_mut(&bp)
                .and_then(|m| m.entries.get_mut(&gt))
            {
                add_var_factor(tv_ref, factor);
            }
        }

        if instance.conf.y {
            let tv_vc_dbg = non_insertion_variants
                .get(&bp)
                .and_then(|m| m.entries.get(&gt))
                .map(|v| v.vars_count)
                .unwrap_or(0);
            eprintln!(
                "  Found lgdel done: {} {} {} 5' {} {}\n",
                bp, gt, p, seq, tv_vc_dbg
            );
        }
    }

    // ── 3' pass ──────────────────────────────────────────────────────
    // Java: VariationRealigner.java#L1198-L1209
    let mut tmp3: Vec<SortPositionSclip> = Vec::new();
    for (&p, sc) in soft_clips_3_end.iter() {
        if p < region_start - EXTENSION || p > region_end + EXTENSION {
            continue;
        }
        tmp3.push(SortPositionSclip::new(p, sc.clone(), 0));
    }
    tmp3.sort_by(comp2);
    svcov = 0;
    clusters = 0;
    pairs = 0;

    // Java: VariationRealigner.java#L1214
    for t in &tmp3 {
        let p = t.position;
        let cnt;
        let seq;
        {
            let sc3v = match soft_clips_3_end.get_mut(&p) {
                Some(s) => s,
                None => continue,
            };
            cnt = sc3v.base.vars_count;
            // Java: VariationRealigner.java#L1220
            if cnt < instance.conf.minr {
                break;
            }
            if sc3v.used {
                continue;
            }
            seq = find_conseq(sc3v, 3);
        }
        if seq.is_empty() {
            continue;
        }
        if seq.len() < 7 {
            continue;
        }

        if instance.conf.y {
            eprintln!("  Working Realignlgdel: 3' {} '{}' {}", p, seq, cnt);
        }

        // Java: VariationRealigner.java#L1238
        let mut bp = findbp(&seq, p + 5, reference_sequences, 1, chr);
        let mut extra = String::new();
        let mut extra_upper = String::new(); // Java: EXTRA

        // Java: VariationRealigner.java#L1243
        if bp == 0 {
            if islowcomplexseq(&seq) {
                continue;
            }
            // Java: VariationRealigner.java#L1248 — findMatch
            let match_result =
                super::structural_variants_processor::find_match(&seq, reference, p, 1, SEED_1, 1);
            bp = match_result.base_position;
            extra_upper = match_result.matched_sequence;
            // Java: VariationRealigner.java#L1251
            if !(bp != 0 && bp - p > 15 && p - bp < SVMAXLEN) {
                continue;
            }

            // Java: VariationRealigner.java#L1255 — markSV
            let sv_mark = super::structural_variants_processor::mark_sv(
                p,
                bp,
                &mut [&mut *svfdel, &mut *svrdel],
                max_read_length,
            );
            svcov = sv_mark.0;
            clusters = sv_mark.1;
            pairs = sv_mark.2;
            if svcov == 0 {
                if cnt <= instance.conf.minr {
                    continue;
                }
            }

            // Java: VariationRealigner.java#L1264-L1269
            {
                let sv = get_sv(non_insertion_variants, p);
                sv.type_ = Some("DEL".to_string());
                sv.pairs += pairs;
                sv.splits += cnt;
                sv.clusters += clusters;
            }

            // Java: VariationRealigner.java#L1271 — partialPipeline if bp > region.end
            if bp > region_end {
                let mut tts = bp - max_read_length;
                let tte = bp + max_read_length;
                if bp - max_read_length <= region_end {
                    tts = region_end + 1;
                }
                run_partial_pipeline(
                    partial_pipeline_context,
                    tts,
                    tte,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
                reference_sequences = &reference.reference_sequences;
            }
        }

        // Java: VariationRealigner.java#L1285
        let mut dellen = bp - p;
        let mut en = 0i32;
        // Java: VariationRealigner.java#L1287-L1295
        if !extra_upper.is_empty() {
            dellen -= extra_upper.len() as i32;
        } else {
            let seq_bytes = seq.as_bytes();
            while (en as usize) < seq_bytes.len()
                && is_not_equals(
                    Some(seq_bytes[en as usize]),
                    reference_sequences.get(&(bp + en)).copied(),
                )
            {
                extra.push(seq_bytes[en as usize] as char);
                en += 1;
            }
        }

        // Java: VariationRealigner.java#L1296
        let mut gt = format!("-{}", dellen);
        let mut sc5p = bp;
        bp = p; // Java: Set it to 5'

        // Java: VariationRealigner.java#L1299-L1325
        if !extra.is_empty() {
            gt = format!("-{}&{}", dellen, extra);
            sc5p += extra.len() as i32;
        } else if !extra_upper.is_empty() {
            gt = format!("-{}&{}", dellen, extra_upper);
        } else {
            // Java: VariationRealigner.java#L1304-L1312 — 5' left-shift walk
            while reference_sequences.contains_key(&(bp - 1))
                && reference_sequences.contains_key(&(bp + dellen - 1))
                && is_equals(
                    reference_sequences.get(&(bp - 1)).copied(),
                    reference_sequences.get(&(bp + dellen - 1)).copied(),
                )
            {
                bp -= 1;
                if bp != 0 {
                    sc5p -= 1;
                }
            }
            // Java: VariationRealigner.java#L1313-L1321 — SV copy on left-shift
            if bp != p {
                let sv_from = non_insertion_variants.get(&p).and_then(|m| m.sv.clone());
                if let Some(sv_data) = sv_from {
                    let sv_to = get_sv(non_insertion_variants, bp);
                    sv_to.clusters = sv_data.clusters;
                    sv_to.pairs = sv_data.pairs;
                    sv_to.splits = sv_data.splits;
                    sv_to.type_ = sv_data.type_;
                    remove_sv(non_insertion_variants, p);
                }
            }
        }

        // Java: VariationRealigner.java#L1327-L1332 — false positive check (3' pass)
        let has_sv_at_bp_3 = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.sv.as_ref())
            .is_some();
        if has_sv_at_bp_3 && !soft_clips_5_end.contains_key(&sc5p) {
            if svcov == 0 && cnt <= instance.conf.minr {
                remove_sv(non_insertion_variants, bp);
                continue;
            }
        }

        if instance.conf.y {
            eprintln!(
                "  Found Realignlgdel: bp: {} {} 3' {} 5'clip: {} '{}' {}",
                bp, gt, p, sc5p, seq, cnt
            );
        }

        // Java: VariationRealigner.java#L1339
        let tv = get_variation(non_insertion_variants, bp, &gt);
        tv.qstd = true;
        tv.pstd = true;

        // Java: VariationRealigner.java#L1343-L1347 — incCnt for short dels
        let sc3v_vars_count = soft_clips_3_end
            .get(&p)
            .map(|s| s.base.vars_count)
            .unwrap_or(0);
        if dellen < instance.conf.indelsize {
            for tp in bp..(bp + dellen + extra.len() as i32 + extra_upper.len() as i32) {
                inc_cnt(ref_coverage, tp, sc3v_vars_count);
            }
        }

        // Java: VariationRealigner.java#L1349-L1353 — refCoverage fallback
        if !ref_coverage.contains_key(&bp) {
            if let Some(&cov) = ref_coverage.get(&(p - 1)) {
                ref_coverage.insert(bp, cov);
            } else {
                ref_coverage.insert(bp, sc3v_vars_count);
            }
        }

        // Java: VariationRealigner.java#L1354
        // **Parity trap T18**: 3' pass adds dellen * varsCount to meanPosition BEFORE adjCnt
        if let Some(sc3v) = soft_clips_3_end.get_mut(&p) {
            sc3v.base.mean_position += dellen as f64 * sc3v.base.vars_count as f64;
        }

        // Java: VariationRealigner.java#L1355 — adjCnt(tv, sc3v)
        {
            let sc3v_base = soft_clips_3_end.get(&p).map(|s| s.base.clone());
            if let Some(base) = sc3v_base {
                let tv_ref = get_variation(non_insertion_variants, bp, &gt);
                adj_cnt(tv_ref, &base);
            }
        }

        // Java: VariationRealigner.java#L1356 — sc3v.used = p + dellen != 0
        // **Parity trap T8**: 3' pass uses p + dellen != 0
        if let Some(sc3v) = soft_clips_3_end.get_mut(&p) {
            sc3v.used = p + dellen != 0;
        }

        // Java: VariationRealigner.java#L1358-L1363 — recursive realigndel with HashMap
        let tv_vc = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.entries.get(&gt))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        let mut dels5: HashMap<i32, HashMap<String, i32>> = HashMap::new();
        let mut inner = HashMap::new();
        inner.insert(gt.clone(), tv_vc);
        dels5.insert(bp, inner);
        realigndel(
            instance_bams.as_ref().map(|v| v.as_slice()),
            instance_bams,
            &dels5,
            non_insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference_sequences,
            chr,
            max_read_length,
        );

        // Java: VariationRealigner.java#L1365-L1367 — SV splits update
        let tv_vc_after_splits = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.entries.get(&gt))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        if let Some(sv) = non_insertion_variants
            .get_mut(&bp)
            .and_then(|m| m.sv.as_mut())
        {
            sv.splits += tv_vc_after_splits - tv_vc;
        }

        if instance.conf.y {
            let tv_vc_dbg = non_insertion_variants
                .get(&bp)
                .and_then(|m| m.entries.get(&gt))
                .map(|v| v.vars_count)
                .unwrap_or(0);
            eprintln!(
                "  Found lgdel: {} {} {} 3' '{}' {}\n",
                bp, gt, p, seq, tv_vc_dbg
            );
        }

        // Java: VariationRealigner.java#L1372-L1374 — addVarFactor
        let tv_vc_after = non_insertion_variants
            .get(&bp)
            .and_then(|m| m.entries.get(&gt))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        if svcov > tv_vc_after {
            let factor = (svcov - tv_vc_after) as f64 / tv_vc_after as f64;
            if let Some(tv_ref) = non_insertion_variants
                .get_mut(&bp)
                .and_then(|m| m.entries.get_mut(&gt))
            {
                add_var_factor(tv_ref, factor);
            }
        }
    }

    if instance.conf.y {
        eprintln!("  Done: Realignlgdel\n");
    }
}

// ─── Cluster E: Large Insertion Discovery ───────────────────────────

/// Ported from: VariationRealigner.realignlgins30()
/// Java: VariationRealigner.java#L1377-L1597
///
/// Try to realign large insertions (typically larger than 30bp) by pairing
/// 5' and 3' soft clips.
pub fn realignlgins30(
    instance_bams: &Option<Vec<String>>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference: &Reference,
    reference_sequences: &HashMap<i32, u8>,
    chr: &str,
    max_read_length: i32,
    region_start: i32,
    region_end: i32,
) {
    let instance = GlobalReadOnlyScope::instance();

    // Java: VariationRealigner.java#L1380-L1391
    let mut tmp5: Vec<SortPositionSclip> = Vec::new();
    for (&p, sc) in soft_clips_5_end.iter() {
        if p < region_start - EXTENSION || p > region_end + EXTENSION {
            continue;
        }
        // Java: COMP3 uses varsCount as count field
        tmp5.push(SortPositionSclip::new(p, sc.clone(), sc.base.vars_count));
    }
    // **Parity trap T11**: COMP3 sorts by .count field, not softClip.varsCount
    tmp5.sort_by(comp3);

    // Java: VariationRealigner.java#L1393-L1403
    let mut tmp3: Vec<SortPositionSclip> = Vec::new();
    for (&p, sc) in soft_clips_3_end.iter() {
        if p < region_start - EXTENSION || p > region_end + EXTENSION {
            continue;
        }
        tmp3.push(SortPositionSclip::new(p, sc.clone(), sc.base.vars_count));
    }
    tmp3.sort_by(comp3);

    // Java: VariationRealigner.java#L1405
    for t5 in &tmp5 {
        let p5 = t5.position;
        let cnt5 = t5.count;

        // Java: VariationRealigner.java#L1410
        if soft_clips_5_end.get(&p5).map_or(true, |s| s.used) {
            continue;
        }

        // Java: VariationRealigner.java#L1413
        for t3 in &tmp3 {
            let p3 = t3.position;
            let cnt3 = t3.count;

            // Java: VariationRealigner.java#L1418 — if sc5v.used, break inner
            if soft_clips_5_end.get(&p5).map_or(true, |s| s.used) {
                break;
            }
            // Java: VariationRealigner.java#L1421
            if soft_clips_3_end.get(&p3).map_or(true, |s| s.used) {
                continue;
            }
            // Java: VariationRealigner.java#L1424
            if p5 - p3 > (max_read_length as f64 * 2.5) as i32 {
                continue;
            }
            // Java: VariationRealigner.java#L1427
            if p3 - p5 > max_read_length - 10 {
                continue;
            }

            // Java: VariationRealigner.java#L1429-L1430
            let seq5 = {
                let sc5v = match soft_clips_5_end.get_mut(&p5) {
                    Some(s) => s,
                    None => break,
                };
                find_conseq(sc5v, 5)
            };
            let seq3 = {
                let sc3v = match soft_clips_3_end.get_mut(&p3) {
                    Some(s) => s,
                    None => continue,
                };
                find_conseq(sc3v, 3)
            };

            // Java: VariationRealigner.java#L1432
            if seq5.len() <= 10 || seq3.len() <= 10 {
                continue;
            }

            if instance.conf.y {
                let seq5_rev: String = seq5.chars().rev().collect();
                eprintln!(
                    "  Working lgins30: {} {} 3: {} {} 5: {} {}",
                    p3, p5, seq3, cnt3, seq5_rev, cnt5
                );
            }

            // Java: VariationRealigner.java#L1439
            if !(cnt5 as f64 / cnt3 as f64 >= 0.08 && cnt5 as f64 / cnt3 as f64 <= 12.0) {
                continue;
            }

            // Java: VariationRealigner.java#L1442
            let match35 = find35match(&seq5, &seq3);
            let bp5 = match35.matched_5_end;
            let bp3 = match35.matched_3_end;
            let score = match35.max_matched_length;

            // Java: VariationRealigner.java#L1449
            if score == 0 {
                continue;
            }

            // Java: VariationRealigner.java#L1452 — integer division
            let smscore = score / 2;

            // Java: VariationRealigner.java#L1454
            let mut ins = if bp3 + smscore > 1 {
                String::from_utf8_lossy(&substr_with_len(seq3.as_bytes(), 0, -(bp3 + smscore) + 1))
                    .to_string()
            } else {
                seq3.clone()
            };

            // Java: VariationRealigner.java#L1455-L1457
            if bp5 + smscore > 0 {
                let piece = substr_with_len(seq5.as_bytes(), 0, bp5 + smscore);
                let reversed: String = piece.iter().rev().map(|&b| b as char).collect();
                ins += &reversed;
            }

            // Java: VariationRealigner.java#L1458
            if islowcomplexseq(&ins) {
                if instance.conf.y {
                    eprintln!("  Discard low complexity insertion found {}.", ins);
                }
                continue;
            }

            let mut bi;
            let vref_key: String;
            let mut use_insertion_variants = false;

            if instance.conf.y {
                eprintln!("  Found candidate lgins30: {} {} {}", p3, p5, ins);
            }

            // Java: VariationRealigner.java#L1466 — Branch: p5 > p3 (overlap)
            if p5 > p3 {
                // Java: VariationRealigner.java#L1467-L1472
                if seq3.len() > ins.len()
                    && !ismatch(
                        &String::from_utf8_lossy(&substr(seq3.as_bytes(), ins.len() as i32))
                            .to_string(),
                        &join_ref(
                            reference_sequences,
                            p5,
                            p5 + seq3.len() as i32 - ins.len() as i32 + 2,
                        ),
                        1,
                    )
                {
                    continue;
                }
                // Java: VariationRealigner.java#L1471-L1474
                if seq5.len() > ins.len()
                    && !ismatch(
                        &String::from_utf8_lossy(&substr(seq5.as_bytes(), ins.len() as i32))
                            .to_string(),
                        &join_ref(
                            reference_sequences,
                            p3 - (seq5.len() as i32 - ins.len() as i32) - 2,
                            p3 - 1,
                        ),
                        -1,
                    )
                {
                    continue;
                }

                if instance.conf.y {
                    eprintln!(
                        "  Found lgins30 complex: {} {} {} {}",
                        p3,
                        p5,
                        ins.len(),
                        ins
                    );
                }

                // Java: VariationRealigner.java#L1479
                let tmp_str = join_ref(reference_sequences, p3, p5 - 1);
                if tmp_str.len() > ins.len() {
                    // Java: VariationRealigner.java#L1481 — deletion is longer
                    ins = format!("{}^{}", p3 - p5, ins);
                    bi = p3;
                    use_insertion_variants = false;
                    vref_key = ins.clone();
                } else if tmp_str.len() < ins.len() {
                    // Java: VariationRealigner.java#L1486-L1490
                    let sub1 = String::from_utf8_lossy(&substr_with_len(
                        ins.as_bytes(),
                        0,
                        ins.len() as i32 - tmp_str.len() as i32,
                    ))
                    .to_string();
                    let sub2 =
                        String::from_utf8_lossy(&substr(ins.as_bytes(), p3 - p5)).to_string();
                    ins = format!("{}&{}", sub1, sub2);
                    ins = format!("+{}", ins);
                    bi = p3 - 1;
                    use_insertion_variants = true;
                    vref_key = ins.clone();
                } else {
                    // Java: VariationRealigner.java#L1493 — long MNP
                    ins = format!("-{}^{}", ins.len(), ins);
                    bi = p3;
                    use_insertion_variants = false;
                    vref_key = ins.clone();
                }
            } else {
                // Java: VariationRealigner.java#L1497-L1510 — p5 <= p3
                if seq3.len() > ins.len()
                    && !ismatch(
                        &String::from_utf8_lossy(&substr(seq3.as_bytes(), ins.len() as i32))
                            .to_string(),
                        &join_ref(
                            reference_sequences,
                            p5,
                            p5 + seq3.len() as i32 - ins.len() as i32 + 2,
                        ),
                        1,
                    )
                {
                    continue;
                }
                if seq5.len() > ins.len()
                    && !ismatch(
                        &String::from_utf8_lossy(&substr(seq5.as_bytes(), ins.len() as i32))
                            .to_string(),
                        &join_ref(
                            reference_sequences,
                            p3 - (seq5.len() as i32 - ins.len() as i32) - 2,
                            p3 - 1,
                        ),
                        -1,
                    )
                {
                    continue;
                }

                // Java: VariationRealigner.java#L1514-L1542
                let mut tmp_str = String::new();
                if ins.len() as i32 <= p3 - p5 {
                    // Java: VariationRealigner.java#L1515 — Tandem duplication
                    let mut rpt = 2i32;
                    let mut tnr = 3i32;
                    while ((p3 - p5 + ins.len() as i32) as f64 / tnr as f64) / ins.len() as f64
                        > 1.0
                    {
                        if (p3 - p5 + ins.len() as i32) % tnr == 0 {
                            rpt += 1;
                        }
                        tnr += 1;
                    }
                    // Java: VariationRealigner.java#L1525 — joinRef with f64 "to"
                    let to_f64 = p5 as f64 + (p3 - p5 + ins.len() as i32) as f64 / rpt as f64
                        - ins.len() as f64;
                    tmp_str += &join_ref_f64(reference_sequences, p5, to_f64);
                    ins = format!("+{}{}", tmp_str, ins);
                } else {
                    // Java: VariationRealigner.java#L1529
                    tmp_str += &join_ref(reference_sequences, p5, p3 - 1);
                    if (ins.len() - tmp_str.len()) % 2 == 0 {
                        // Java: VariationRealigner.java#L1531
                        let tex = (ins.len() - tmp_str.len()) / 2;
                        let left = format!(
                            "{}{}",
                            tmp_str,
                            String::from_utf8_lossy(&substr_with_len(
                                ins.as_bytes(),
                                0,
                                tex as i32
                            ))
                        );
                        let right = String::from_utf8_lossy(&substr(ins.as_bytes(), tex as i32))
                            .to_string();
                        if left == right {
                            ins = format!(
                                "+{}",
                                String::from_utf8_lossy(&substr(ins.as_bytes(), tex as i32))
                            );
                        } else {
                            ins = format!("+{}{}", tmp_str, ins);
                        }
                    } else {
                        ins = format!("+{}{}", tmp_str, ins);
                    }
                }

                if instance.conf.y {
                    eprintln!(
                        "Found lgins30: {} {} {} {} + {}",
                        p3,
                        p5,
                        ins.len(),
                        tmp_str,
                        ins
                    );
                }
                bi = p5 - 1;
                use_insertion_variants = true;
                vref_key = ins.clone();
            }

            // Java: VariationRealigner.java#L1547-L1548 — mark both clips used
            if let Some(sc3v) = soft_clips_3_end.get_mut(&p3) {
                sc3v.used = true;
            }
            if let Some(sc5v) = soft_clips_5_end.get_mut(&p5) {
                sc5v.used = true;
            }

            // Java: VariationRealigner.java#L1549-L1551 — set pstd/qstd
            {
                let vref = if use_insertion_variants {
                    get_variation(insertion_variants, bi, &vref_key)
                } else {
                    get_variation(non_insertion_variants, bi, &vref_key)
                };
                vref.pstd = true;
                vref.qstd = true;
            }

            // Java: VariationRealigner.java#L1552
            let sc5v_vc = soft_clips_5_end
                .get(&p5)
                .map(|s| s.base.vars_count)
                .unwrap_or(0);
            inc_cnt(ref_coverage, bi, sc5v_vc);

            if instance.conf.y {
                eprintln!(" lgins30 Found: '{}' {} {} {}", ins, bi, bp3, bp5);
            }

            // Java: VariationRealigner.java#L1558-L1591 — Branch on ins prefix
            if ins.starts_with('+') {
                // Java: VariationRealigner.java#L1559
                let mut mvref_clone = get_variation_maybe(
                    non_insertion_variants,
                    bi,
                    reference_sequences.get(&bi).copied(),
                )
                .cloned();
                let sc3v_clone = soft_clips_3_end.get(&p3).map(|s| s.base.clone());
                let sc5v_clone = soft_clips_5_end.get(&p5).map(|s| s.base.clone());

                {
                    let vref = get_variation(insertion_variants, bi, &vref_key);
                    if let Some(ref sc3b) = sc3v_clone {
                        adj_cnt_with_reference(vref, sc3b, mvref_clone.as_mut());
                    }
                    if let Some(ref sc5b) = sc5v_clone {
                        adj_cnt(vref, sc5b);
                    }
                }
                write_back_cloned_reference_variation(
                    non_insertion_variants,
                    reference_sequences,
                    bi,
                    mvref_clone,
                );

                // Java: VariationRealigner.java#L1563-L1569
                if let Some(bams_vec) = instance_bams.as_ref() {
                    let mut mvref_for_check = get_variation_maybe(
                        non_insertion_variants,
                        bi,
                        reference_sequences.get(&bi).copied(),
                    )
                    .cloned();
                    if !bams_vec.is_empty() && p3 - p5 >= 5 && p3 - p5 < max_read_length - 10 {
                        if let Some(ref mut mvref) = mvref_for_check {
                            if mvref.vars_count != 0 && no_passing_reads(chr, p5, p3, bams_vec) {
                                let vref_vc = insertion_variants
                                    .get(&bi)
                                    .and_then(|m| m.entries.get(&vref_key))
                                    .map(|v| v.vars_count)
                                    .unwrap_or(0);
                                if vref_vc > 2 * mvref.vars_count {
                                    let mvref_copy = mvref.clone();
                                    let vref = get_variation(insertion_variants, bi, &vref_key);
                                    adj_cnt_with_reference(vref, &mvref_copy, Some(mvref));
                                }
                            }
                        }
                    }
                    write_back_cloned_reference_variation(
                        non_insertion_variants,
                        reference_sequences,
                        bi,
                        mvref_for_check,
                    );
                }

                // Java: VariationRealigner.java#L1570-L1575 — recursive realignins
                let vref_vc = insertion_variants
                    .get(&bi)
                    .and_then(|m| m.entries.get(&vref_key))
                    .map(|v| v.vars_count)
                    .unwrap_or(0);
                let mut tins: HashMap<i32, HashMap<String, i32>> = HashMap::new();
                let mut inner = HashMap::new();
                inner.insert(ins.clone(), vref_vc);
                tins.insert(bi, inner);
                realignins(
                    &tins,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                    reference_sequences,
                    chr,
                    max_read_length,
                );
            } else if ins.starts_with('-') {
                // Java: VariationRealigner.java#L1576-L1584
                let mut mvref_clone = get_variation_maybe(
                    non_insertion_variants,
                    bi,
                    reference_sequences.get(&bi).copied(),
                )
                .cloned();
                let sc3v_clone = soft_clips_3_end.get(&p3).map(|s| s.base.clone());
                let sc5v_clone = soft_clips_5_end.get(&p5).map(|s| s.base.clone());

                {
                    let vref = get_variation(non_insertion_variants, bi, &vref_key);
                    if let Some(ref sc3b) = sc3v_clone {
                        adj_cnt_with_reference(vref, sc3b, mvref_clone.as_mut());
                    }
                    if let Some(ref sc5b) = sc5v_clone {
                        adj_cnt(vref, sc5b);
                    }
                }
                write_back_cloned_reference_variation(
                    non_insertion_variants,
                    reference_sequences,
                    bi,
                    mvref_clone,
                );

                // Java: VariationRealigner.java#L1580-L1584 — recursive realigndel
                let vref_vc = non_insertion_variants
                    .get(&bi)
                    .and_then(|m| m.entries.get(&vref_key))
                    .map(|v| v.vars_count)
                    .unwrap_or(0);
                let mut tdel: HashMap<i32, HashMap<String, i32>> = HashMap::new();
                let mut inner = HashMap::new();
                inner.insert(ins.clone(), vref_vc);
                tdel.insert(bi, inner);
                realigndel(
                    None, // Java: realigndel(null, tdel) — bams parameter is null
                    instance_bams,
                    &tdel,
                    non_insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                    reference_sequences,
                    chr,
                    max_read_length,
                );
            } else {
                // Java: VariationRealigner.java#L1585-L1587 — MNP case
                let sc3v_clone = soft_clips_3_end.get(&p3).map(|s| s.base.clone());
                let sc5v_clone = soft_clips_5_end.get(&p5).map(|s| s.base.clone());
                let vref = get_variation(non_insertion_variants, bi, &vref_key);
                if let Some(ref sc3b) = sc3v_clone {
                    adj_cnt(vref, sc3b);
                }
                if let Some(ref sc5b) = sc5v_clone {
                    adj_cnt(vref, sc5b);
                }
            }

            // Java: VariationRealigner.java#L1589 — break inner loop after first match
            break;
        }
    }

    if instance.conf.y {
        eprintln!("Done: lgins30\n");
    }
}

/// Ported from: VariationRealigner.realignlgins()
/// Java: VariationRealigner.java#L1605-L1939
///
/// Realign large insertions that are not present in alignment.
/// Two asymmetric passes: 5' then 3'.
fn realignlgins(
    instance_bams: &Option<Vec<String>>,
    svfdup: &mut Vec<Sclip>,
    svrdup: &mut Vec<Sclip>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference: &mut Reference,
    chr: &str,
    max_read_length: i32,
    region_start: i32,
    region_end: i32,
    partial_pipeline_context: &PartialPipelineContext<'_>,
) {
    let instance = GlobalReadOnlyScope::instance();
    let mut reference_sequences = &reference.reference_sequences;

    // ── 5' pass ──────────────────────────────────────────────────────
    // Java: VariationRealigner.java#L1610-L1620
    let mut tmp: Vec<SortPositionSclip> = Vec::new();
    for (&p, sc) in soft_clips_5_end.iter() {
        if p < region_start - EXTENSION || p > region_end + EXTENSION {
            continue;
        }
        tmp.push(SortPositionSclip::new(p, sc.clone(), 0));
    }
    tmp.sort_by(comp2);

    // Java: VariationRealigner.java#L1623
    for t in &tmp {
        let p = t.position;

        let (cnt, seq) = {
            let sc5v = match soft_clips_5_end.get_mut(&p) {
                Some(s) => s,
                None => continue,
            };
            let cnt = sc5v.base.vars_count;
            if cnt < instance.conf.minr {
                break; // Java: VariationRealigner.java#L1629
            }
            if sc5v.used {
                continue; // Java: VariationRealigner.java#L1633
            }
            let seq = find_conseq(sc5v, 0);
            (cnt, seq)
        };

        if seq.is_empty() {
            continue;
        }
        if instance.conf.y {
            eprintln!("  Working lgins: 5: {} {} cnt: {}", p, seq, cnt);
        }
        // Java: VariationRealigner.java#L1643
        if seq.len() < 12 {
            continue;
        }

        // Java: VariationRealigner.java#L1647
        let tpl = findbi(&seq, p, reference_sequences, -1, chr);
        let mut bi = tpl.base_insert.unwrap_or(0);
        let mut ins = tpl.insertion_sequence;
        let mut extra = String::new();

        // Java: VariationRealigner.java#L1652 — if bi == 0
        if bi == 0 {
            if islowcomplexseq(&seq) {
                continue;
            }
            // Java: VariationRealigner.java#L1658 — findMatch
            let match_result =
                super::structural_variants_processor::find_match(&seq, reference, p, -1, SEED_1, 1);
            bi = match_result.base_position;
            extra = match_result.matched_sequence;
            // Java: VariationRealigner.java#L1661
            if !(bi != 0 && bi - p > 15 && bi - p < SVMAXLEN) {
                continue;
            }

            // Java: VariationRealigner.java#L1665-L1679 — partialPipeline for bi > region.end
            if bi > region_end {
                let mut tts = bi - max_read_length;
                let tte = bi + max_read_length;
                if bi - max_read_length <= region_end {
                    tts = region_end + 1;
                }
                run_partial_pipeline(
                    partial_pipeline_context,
                    tts,
                    tte,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
                reference_sequences = &reference.reference_sequences;
            }

            // Java: VariationRealigner.java#L1680-L1688
            if bi - p > instance.conf.svminlen + 2 * SVFLANK {
                ins = join_ref(reference_sequences, p, p + SVFLANK - 1);
                ins += &format!("<dup{}>", bi - p - 2 * SVFLANK + 1);
                ins +=
                    &join_ref_for_5_lgins(reference_sequences, bi - SVFLANK + 1, bi, &seq, &extra);
            } else {
                ins = join_ref_for_5_lgins(reference_sequences, p, bi, &seq, &extra);
            }
            ins += &extra;

            // Java: VariationRealigner.java#L1693 — markDUPSV
            let tp2 = super::structural_variants_processor::mark_dup_sv(
                p,
                bi,
                &mut [&mut *svfdup, &mut *svrdup],
                max_read_length,
            );
            let clusters = tp2.0;
            let pairs = tp2.1;

            // Java: VariationRealigner.java#L1696-L1705 — refCoverage adjustment
            if !ref_coverage.contains_key(&(p - 1))
                || (ref_coverage.contains_key(&bi)
                    && ref_coverage.contains_key(&(p - 1))
                    && *ref_coverage.get(&(p - 1)).unwrap() < *ref_coverage.get(&bi).unwrap())
            {
                if ref_coverage.contains_key(&bi) {
                    let val = *ref_coverage.get(&bi).unwrap();
                    ref_coverage.insert(p - 1, val);
                } else {
                    ref_coverage.insert(p - 1, cnt);
                }
            } else {
                if cnt > *ref_coverage.get(&(p - 1)).unwrap() {
                    inc_cnt(ref_coverage, p - 1, cnt);
                }
            }

            // Java: VariationRealigner.java#L1708-L1714
            bi = p - 1;
            {
                let sv = get_sv(non_insertion_variants, bi);
                sv.type_ = Some("DUP".to_string());
                sv.pairs += pairs;
                sv.splits += cnt;
                sv.clusters += clusters;
            }
        }

        if instance.conf.y {
            eprintln!(
                "  Found candidate lgins from 5: {} +{} {} {}",
                bi, ins, p, seq
            );
        }

        // Java: VariationRealigner.java#L1720-L1723
        {
            let iref = get_variation(insertion_variants, bi, &format!("+{}", ins));
            iref.pstd = true;
            iref.qstd = true;
            let sc5v_clone = soft_clips_5_end.get(&p).map(|s| s.base.clone());
            if let Some(ref sc5b) = sc5v_clone {
                adj_cnt(iref, sc5b);
            }
        }

        // Java: VariationRealigner.java#L1724-L1730 — rpflag check
        let mut rpflag = true;
        for i in 0..ins.len() {
            if !is_equals(
                reference_sequences.get(&(bi + 1 + i as i32)).copied(),
                Some(ins.as_bytes()[i]),
            ) {
                rpflag = false;
                break;
            }
        }

        // Java: VariationRealigner.java#L1732-L1734
        if non_insertion_variants.contains_key(&bi)
            && non_insertion_variants.get(&bi).unwrap().sv.is_none()
        {
            inc_cnt(ref_coverage, bi, cnt);
        }

        // Java: VariationRealigner.java#L1735-L1738
        let mut len = ins.len() as i32;
        if ins.contains('&') {
            len -= 1;
        }

        // Java: VariationRealigner.java#L1739-L1752 — Extend remaining bases from sc5v.seq
        // **Parity trap T15**: 5' starts at ii = len + 1
        {
            let seq_data: Option<Vec<(i32, Vec<(String, Variation)>)>> = soft_clips_5_end
                .get(&p)
                .filter(|sc| !sc.seq.is_empty())
                .map(|sc| &sc.seq)
                .map(|seq_map| {
                    let seq_len = seq_map.keys().last().map(|k| k + 1).unwrap_or(0);
                    let mut result = Vec::new();
                    for ii in (len + 1)..seq_len {
                        if let Some(inner_map) = seq_map.get(&ii) {
                            let entries: Vec<(String, Variation)> = inner_map
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect();
                            result.push((ii, entries));
                        }
                    }
                    result
                });

            if let Some(seq_entries) = seq_data {
                for (ii, entries) in &seq_entries {
                    let pii = bi - ii + len;
                    for (tnt, tv) in entries {
                        let tvr = get_variation(non_insertion_variants, pii, tnt);
                        adj_cnt(tvr, tv);
                        tvr.pstd = true;
                        tvr.qstd = true;
                        inc_cnt(ref_coverage, pii, tv.vars_count);
                    }
                }
            }
        }

        // Java: VariationRealigner.java#L1753 — sc5v.used = bi + len != 0
        if let Some(sc5v) = soft_clips_5_end.get_mut(&p) {
            sc5v.used = bi + len != 0;
        }

        // Java: VariationRealigner.java#L1755-L1756 — recursive realignins
        let iref_vc = insertion_variants
            .get(&bi)
            .and_then(|m| m.entries.get(&format!("+{}", ins)))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        let ins_key = format!("+{}", ins);
        let mut tins: HashMap<i32, HashMap<String, i32>> = HashMap::new();
        let mut inner = HashMap::new();
        inner.insert(ins_key.clone(), iref_vc);
        tins.insert(bi, inner);
        let newins = realignins(
            &tins,
            non_insertion_variants,
            insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference_sequences,
            chr,
            max_read_length,
        );

        // Java: VariationRealigner.java#L1757-L1759
        let newins_key = if newins.is_empty() {
            ins_key.clone()
        } else {
            newins
        };

        // Java: VariationRealigner.java#L1762-L1764 — SV splits update
        if non_insertion_variants
            .get(&bi)
            .and_then(|m| m.sv.as_ref())
            .is_some()
        {
            let kref_vc = insertion_variants
                .get(&bi)
                .and_then(|m| m.entries.get(&newins_key))
                .map(|v| v.vars_count)
                .unwrap_or(0);
            let original_vc = tins
                .get(&bi)
                .and_then(|m| m.get(&ins_key))
                .copied()
                .unwrap_or(0);
            if let Some(sv) = non_insertion_variants
                .get_mut(&bi)
                .and_then(|m| m.sv.as_mut())
            {
                sv.splits += kref_vc - original_vc;
            }
        }

        // Java: VariationRealigner.java#L1765-L1771 — rpflag-based adjustment
        let mut mref_clone = get_variation_maybe(
            non_insertion_variants,
            bi,
            reference_sequences.get(&bi).copied(),
        )
        .cloned();
        if let Some(bams_vec) = instance_bams.as_ref() {
            if rpflag
                && !bams_vec.is_empty()
                && ins.len() >= 5
                && ins.len() < (max_read_length - 10) as usize
            {
                if let Some(ref mut mref) = mref_clone {
                    if mref.vars_count != 0
                        && no_passing_reads(chr, bi, bi + ins.len() as i32, bams_vec)
                    {
                        let kref_vc = insertion_variants
                            .get(&bi)
                            .and_then(|m| m.entries.get(&newins_key))
                            .map(|v| v.vars_count)
                            .unwrap_or(0);
                        if kref_vc > 2 * mref.vars_count {
                            let mref_copy = mref.clone();
                            let kref = get_variation(insertion_variants, bi, &newins_key);
                            adj_cnt_with_reference(kref, &mref_copy, Some(mref));
                        }
                    }
                }
            }
        }
        write_back_cloned_reference_variation(
            non_insertion_variants,
            reference_sequences,
            bi,
            mref_clone,
        );
    }

    // ── 3' pass ──────────────────────────────────────────────────────
    // Java: VariationRealigner.java#L1779-L1790
    let mut tmp: Vec<SortPositionSclip> = Vec::new();
    for (&p, sc) in soft_clips_3_end.iter() {
        if p < region_start - EXTENSION || p > region_end + EXTENSION {
            continue;
        }
        tmp.push(SortPositionSclip::new(p, sc.clone(), 0));
    }
    tmp.sort_by(comp2);

    // Java: VariationRealigner.java#L1792
    for t in &tmp {
        let original_p = t.position;
        let mut p = original_p;
        let (cnt, seq, sc3v_snapshot) = {
            let sc3v = match soft_clips_3_end.get_mut(&original_p) {
                Some(s) => s,
                None => continue,
            };
            let cnt = sc3v.base.vars_count;
            if cnt < instance.conf.minr {
                break;
            }
            if sc3v.used {
                continue; // Java: VariationRealigner.java#L1790-L1792
            }
            let seq = find_conseq(sc3v, 0);
            (cnt, seq, sc3v.clone())
        };

        if seq.is_empty() {
            continue;
        }
        if instance.conf.y {
            eprintln!("  Working lgins 3: {} {} cnt: {}", p, seq, cnt);
        }
        // Java: VariationRealigner.java#L1813
        if seq.len() < 12 {
            continue;
        }

        // Java: VariationRealigner.java#L1817
        let tpl = findbi(&seq, p, reference_sequences, 1, chr);
        let mut bi = tpl.base_insert.unwrap_or(0);
        let mut ins = tpl.insertion_sequence;
        let mut extra = String::new();

        // Java: VariationRealigner.java#L1822 — if bi == 0
        if bi == 0 {
            if islowcomplexseq(&seq) {
                continue;
            }
            // Java: VariationRealigner.java#L1829 — findMatch
            let match_result =
                super::structural_variants_processor::find_match(&seq, reference, p, 1, SEED_1, 1);
            bi = match_result.base_position;
            extra = match_result.matched_sequence;
            // Java: VariationRealigner.java#L1832
            if !(bi != 0 && p - bi > 15 && p - bi < SVMAXLEN) {
                continue;
            }

            // Java: VariationRealigner.java#L1836-L1850 — partialPipeline for bi < region.start
            if bi < region_start {
                let tts = bi - max_read_length;
                let mut tte = bi + max_read_length;
                if bi + max_read_length >= region_start {
                    tte = region_start - 1;
                }
                run_partial_pipeline(
                    partial_pipeline_context,
                    tts,
                    tte,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
                reference_sequences = &reference.reference_sequences;
            }

            // Java: VariationRealigner.java#L1851-L1856 — shift5 walk
            let mut shift5 = 0i32;
            while reference_sequences.contains_key(&(p - 1))
                && reference_sequences.contains_key(&(bi - 1))
                && is_equals(
                    reference_sequences.get(&(p - 1)).copied(),
                    reference_sequences.get(&(bi - 1)).copied(),
                )
            {
                p -= 1;
                bi -= 1;
                shift5 += 1;
            }

            // Java: VariationRealigner.java#L1857-L1869
            if p - bi > instance.conf.svminlen + 2 * SVFLANK {
                ins = join_ref_for_3_lgins(
                    reference_sequences,
                    bi,
                    bi + SVFLANK - 1,
                    shift5,
                    &seq,
                    &extra,
                );
                ins += &format!("<dup{}>", p - bi - 2 * SVFLANK);
                ins += &join_ref(reference_sequences, p - SVFLANK, p - 1);
            } else {
                ins = join_ref_for_3_lgins(reference_sequences, bi, p - 1, shift5, &seq, &extra);
            }
            ins += &extra;

            // Java: VariationRealigner.java#L1871 — markDUPSV
            let tp2 = super::structural_variants_processor::mark_dup_sv(
                bi,
                p - 1,
                &mut [&mut *svfdup, &mut *svrdup],
                max_read_length,
            );
            let clusters = tp2.0;
            let pairs = tp2.1;

            // Java: VariationRealigner.java#L1874
            bi -= 1;

            // Java: VariationRealigner.java#L1876-L1881
            {
                let sv = get_sv(non_insertion_variants, bi);
                sv.type_ = Some("DUP".to_string());
                sv.pairs += pairs;
                sv.splits += cnt;
                sv.clusters += clusters;
            }

            // Java: VariationRealigner.java#L1882-L1892 — refCoverage adjustment
            if !ref_coverage.contains_key(&bi)
                || (ref_coverage.contains_key(&p)
                    && ref_coverage.contains_key(&bi)
                    && *ref_coverage.get(&bi).unwrap() < *ref_coverage.get(&p).unwrap())
            {
                if ref_coverage.contains_key(&p) {
                    let val = *ref_coverage.get(&p).unwrap();
                    ref_coverage.insert(bi, val);
                } else {
                    ref_coverage.insert(bi, cnt);
                }
            } else {
                if cnt > *ref_coverage.get(&bi).unwrap() {
                    inc_cnt(ref_coverage, bi, cnt);
                }
            }
        }

        if instance.conf.y {
            eprintln!(
                "  Found candidate lgins from 3: {} +{} {} {}",
                bi, ins, p, seq
            );
        }

        // Java: VariationRealigner.java#L1899-L1906
        {
            let iref = get_variation(insertion_variants, bi, &format!("+{}", ins));
            iref.pstd = true;
            iref.qstd = true;

            // Java: VariationRealigner.java#L1902
            let mut lref_clone = get_variation_maybe(
                non_insertion_variants,
                bi,
                reference_sequences.get(&bi).copied(),
            )
            .cloned();

            // Java: VariationRealigner.java#L1903 — null lref if p - bi > meanPosition / cnt
            let sc3_mean_pos = sc3v_snapshot.base.mean_position;
            let nullify_lref = if lref_clone.is_some() {
                (p - bi) as f64 > sc3_mean_pos / cnt as f64
            } else {
                true
            };
            if nullify_lref {
                lref_clone = None;
            }

            adj_cnt_with_reference(iref, &sc3v_snapshot.base, lref_clone.as_mut());
            write_back_cloned_reference_variation(
                non_insertion_variants,
                reference_sequences,
                bi,
                lref_clone,
            );
        }

        // Java: VariationRealigner.java#L1907-L1913 — rpflag check
        let mut rpflag = true;
        for i in 0..ins.len() {
            if !is_equals(
                reference_sequences.get(&(bi + 1 + i as i32)).copied(),
                Some(ins.as_bytes()[i]),
            ) {
                rpflag = false;
                break;
            }
        }

        // Java: VariationRealigner.java#L1917-L1920
        let mut len = ins.len() as i32;
        if ins.contains('&') {
            len -= 1;
        }

        // Java: VariationRealigner.java#L1921-L1933 — Extend remaining bases from sc3v.seq
        // **Parity trap T15**: 3' starts at ii = len (NOT len + 1)
        {
            let seq_data: Option<Vec<(i32, Vec<(String, Variation)>)>> =
                (!sc3v_snapshot.seq.is_empty())
                    .then_some(&sc3v_snapshot.seq)
                    .map(|seq_map| {
                        let len_seq = seq_map.keys().last().map(|k| k + 1).unwrap_or(0);
                        let mut result = Vec::new();
                        for ii in len..len_seq {
                            if let Some(inner_map) = seq_map.get(&ii) {
                                let entries: Vec<(String, Variation)> = inner_map
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect();
                                result.push((ii, entries));
                            }
                        }
                        result
                    });

            if let Some(seq_entries) = seq_data {
                for (ii, entries) in &seq_entries {
                    let pii = p + ii - len;
                    for (tnt, tv) in entries {
                        let vref = get_variation(non_insertion_variants, pii, tnt);
                        adj_cnt(vref, tv);
                        vref.pstd = true;
                        vref.qstd = true;
                        inc_cnt(ref_coverage, pii, tv.vars_count);
                    }
                }
            }
        }

        // Java: VariationRealigner.java#L1934 — sc3v.used = true (unconditional)
        if let Some(sc3v) = soft_clips_3_end.get_mut(&original_p) {
            sc3v.used = true;
        }

        // Java: VariationRealigner.java#L1935-L1936 — recursive realignins
        let iref_vc = insertion_variants
            .get(&bi)
            .and_then(|m| m.entries.get(&format!("+{}", ins)))
            .map(|v| v.vars_count)
            .unwrap_or(0);
        let ins_key = format!("+{}", ins);
        let mut tins: HashMap<i32, HashMap<String, i32>> = HashMap::new();
        let mut inner = HashMap::new();
        inner.insert(ins_key.clone(), iref_vc);
        tins.insert(bi, inner);
        realignins(
            &tins,
            non_insertion_variants,
            insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference_sequences,
            chr,
            max_read_length,
        );

        // Java: VariationRealigner.java#L1937-L1939 — SV splits update
        if non_insertion_variants
            .get(&bi)
            .and_then(|m| m.sv.as_ref())
            .is_some()
        {
            let original_vc = tins
                .get(&bi)
                .and_then(|m| m.get(&ins_key))
                .copied()
                .unwrap_or(0);
            let iref_vc_after = insertion_variants
                .get(&bi)
                .and_then(|m| m.entries.get(&ins_key))
                .map(|v| v.vars_count)
                .unwrap_or(0);
            if let Some(sv) = non_insertion_variants
                .get_mut(&bi)
                .and_then(|m| m.sv.as_mut())
            {
                sv.splits += iref_vc_after - original_vc;
            }
        }

        // Java: VariationRealigner.java#L1940-L1946 — rpflag-based adjustment
        let mut mref_clone = get_variation_maybe(
            non_insertion_variants,
            bi,
            reference_sequences.get(&bi).copied(),
        )
        .cloned();
        if let Some(bams_vec) = instance_bams.as_ref() {
            if rpflag
                && !bams_vec.is_empty()
                && ins.len() >= 5
                && ins.len() < (max_read_length - 10) as usize
            {
                if let Some(ref mut mref) = mref_clone {
                    if mref.vars_count != 0
                        && no_passing_reads(chr, bi, bi + ins.len() as i32, bams_vec)
                    {
                        let iref_vc_now = insertion_variants
                            .get(&bi)
                            .and_then(|m| m.entries.get(&ins_key))
                            .map(|v| v.vars_count)
                            .unwrap_or(0);
                        if iref_vc_now > 2 * mref.vars_count {
                            let mref_copy = mref.clone();
                            let iref = get_variation(insertion_variants, bi, &ins_key);
                            adj_cnt_with_reference(iref, &mref_copy, Some(mref));
                        }
                    }
                }
            }
        }
        write_back_cloned_reference_variation(
            non_insertion_variants,
            reference_sequences,
            bi,
            mref_clone,
        );
    }
}

// ─── Cluster F: Orchestration ────────────────────────────────────────

/// Ported from: VariationRealigner.realignIndels()
/// Java: VariationRealigner.java#L393-L410
///
/// Sequences the five realignment sub-procedures in exact order.
/// Mutation order is load-bearing — each step's mutations are visible to subsequent steps.
pub fn realign_indels(
    instance_bams: &Option<Vec<String>>,
    position_to_deletions_count: &HashMap<i32, HashMap<String, i32>>,
    position_to_insertion_count: &HashMap<i32, HashMap<String, i32>>,
    sv_structures: &mut crate::data::SVStructures,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &mut HashMap<i32, i32>,
    soft_clips_3_end: &mut HashMap<i32, Sclip>,
    soft_clips_5_end: &mut HashMap<i32, Sclip>,
    reference: &mut Reference,
    chr: &str,
    max_read_length: i32,
    region_start: i32,
    region_end: i32,
    region: &Region,
    reference_resource: &Arc<ReferenceResource>,
    splice: &HashSet<String>,
    out: &Arc<VariantPrinter>,
    bam: &str,
) {
    let instance = GlobalReadOnlyScope::instance();
    let partial_pipeline_context = PartialPipelineContext {
        bam,
        region,
        reference_resource,
        splice,
        out,
    };

    // Java: VariationRealigner.java#L394-L395
    if instance.conf.y {
        eprintln!("Start Realigndel");
    }
    // Java: VariationRealigner.java#L396
    {
        let reference_sequences = &reference.reference_sequences;
        realigndel(
            Some(&instance_bams.clone().unwrap_or_default()),
            instance_bams,
            position_to_deletions_count,
            non_insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference_sequences,
            chr,
            max_read_length,
        );
    }

    // Java: VariationRealigner.java#L397-L398
    if instance.conf.y {
        eprintln!("Start Realignins");
    }
    // Java: VariationRealigner.java#L399
    {
        let reference_sequences = &reference.reference_sequences;
        realignins(
            position_to_insertion_count,
            non_insertion_variants,
            insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference_sequences,
            chr,
            max_read_length,
        );
    }

    // Java: VariationRealigner.java#L400-L401
    if instance.conf.y {
        eprintln!("Start Realignlgdel");
    }
    // Java: VariationRealigner.java#L402
    realignlgdel(
        instance_bams,
        &mut sv_structures.svfdel,
        &mut sv_structures.svrdel,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        chr,
        max_read_length,
        region_start,
        region_end,
        &partial_pipeline_context,
    );

    // Java: VariationRealigner.java#L403-L404
    if instance.conf.y {
        eprintln!("Start Realignlgins30");
    }
    // Java: VariationRealigner.java#L405
    {
        let reference_sequences = &reference.reference_sequences;
        realignlgins30(
            instance_bams,
            non_insertion_variants,
            insertion_variants,
            ref_coverage,
            soft_clips_3_end,
            soft_clips_5_end,
            reference,
            reference_sequences,
            chr,
            max_read_length,
            region_start,
            region_end,
        );
    }

    // Java: VariationRealigner.java#L406-L407
    if instance.conf.y {
        eprintln!("Start Realignlgins");
    }
    // Java: VariationRealigner.java#L408
    realignlgins(
        instance_bams,
        &mut sv_structures.svfdup,
        &mut sv_structures.svrdup,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        chr,
        max_read_length,
        region_start,
        region_end,
        &partial_pipeline_context,
    );
}

/// Ported from: VariationRealigner.process()
/// Java: VariationRealigner.java#L75-L103
///
/// Entry point for the realignment pipeline stage.
/// 1. Extract fields from scope (initFromScope)
/// 2. Create CURSEG from region
/// 3. If !disableSV: filterAllSVStructures()
/// 4. adjustMNP()
/// 5. If performLocalRealignment: realignIndels()
/// 6. Construct RealignedVariationData from instance fields
/// 7. Return Scope with realigned data
pub fn process(scope: Scope<VariationData>) -> Scope<RealignedVariationData> {
    let instance = GlobalReadOnlyScope::instance();

    // Java: VariationRealigner.java#L76 — initFromScope(scope)
    // Destructure scope to extract all fields.
    let Scope {
        bam,
        region,
        region_ref,
        reference_resource,
        max_read_length,
        splice,
        out,
        data,
    } = scope;
    let mut reference = (*region_ref).clone();

    let chr = crate::mods::sam_file_parser::get_chr_name(&region, &instance.conf);
    let instance_bams: Option<Vec<String>> = if bam.is_empty() {
        None
    } else {
        Some(bam.split(':').map(|s| s.to_string()).collect())
    };

    let mut non_insertion_variants = data.non_insertion_variants;
    let mut insertion_variants = data.insertion_variants;
    let position_to_insertion_count: HashMap<i32, HashMap<String, i32>> = data
        .position_to_insertion_count
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();
    let position_to_deletions_count: HashMap<i32, HashMap<String, i32>> = data
        .position_to_deletions_count
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();
    let mut ref_coverage = data.ref_coverage;
    let mut soft_clips_5_end = data.soft_clips_5_end;
    let mut soft_clips_3_end = data.soft_clips_3_end;
    let mut sv_structures = data.sv_structures;
    let duprate = data.duprate;
    let mnp: HashMap<i32, HashMap<String, i32>> = data
        .mnp
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();

    let reference_sequences = &reference.reference_sequences;

    // Java: VariationRealigner.java#L77
    let curseg = CurrentSegment::new(region.chr.clone(), region.start, region.end);

    // Java: VariationRealigner.java#L79-L81
    let mut softp2sv: HashMap<i32, Vec<Sclip>> = HashMap::new();
    if !instance.conf.disable_sv {
        filter_all_sv_structures(&mut sv_structures, max_read_length, &mut softp2sv);
    }

    // Java: VariationRealigner.java#L83
    adjust_mnp(
        &mnp,
        &mut non_insertion_variants,
        &mut ref_coverage,
        &mut soft_clips_3_end,
        &mut soft_clips_5_end,
        reference_sequences,
    );

    // Java: VariationRealigner.java#L85-L87 (debug timing — skip, diagnostic only)
    if instance.conf.y {
        eprintln!("TIME: Start realign");
    }

    // Java: VariationRealigner.java#L89-L91
    if instance.conf.perform_local_realignment {
        realign_indels(
            &instance_bams,
            &position_to_deletions_count,
            &position_to_insertion_count,
            &mut sv_structures,
            &mut non_insertion_variants,
            &mut insertion_variants,
            &mut ref_coverage,
            &mut soft_clips_3_end,
            &mut soft_clips_5_end,
            &mut reference,
            &chr,
            max_read_length,
            region.start,
            region.end,
            &region,
            &reference_resource,
            splice.as_ref(),
            &out,
            bam.as_str(),
        );
    }

    // Java: VariationRealigner.java#L93-L97 — Skip JSONL writer (diagnostic only, not parity-relevant)

    // Java: VariationRealigner.java#L99-L103
    let realigned_data = RealignedVariationData {
        non_insertion_variants,
        insertion_variants,
        soft_clips_3_end,
        soft_clips_5_end,
        ref_coverage,
        max_read_length: Some(max_read_length),
        sv_structures,
        duprate,
        curseg,
        softp2sv,
        previous_scope: None,
    };

    // Build output scope preserving non-data fields from input scope
    Scope {
        bam,
        region,
        region_ref: Arc::new(reference),
        reference_resource,
        max_read_length,
        splice,
        out,
        data: realigned_data,
    }
}

// ─── Unit tests for Cluster A ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::config::Configuration;
    use crate::scope::GlobalReadOnlyScope;

    /// Helper to initialize the global scope for testing.
    fn init_test_scope() {
        // Clear previous scope if any
        GlobalReadOnlyScope::clear();
        let conf = Configuration::default();
        GlobalReadOnlyScope::init(
            conf,
            HashMap::new(),
            "test_sample",
            None,
            None,
            HashMap::new(),
            HashMap::new(),
        );
    }

    #[test]
    fn test_fill_and_sort_tmp_basic() {
        let mut changes: HashMap<i32, BTreeMap<String, i32>> = HashMap::new();
        let mut inner = BTreeMap::new();
        inner.insert("A".to_string(), 10);
        inner.insert("B".to_string(), 20);
        changes.insert(100, inner);
        let mut inner2 = BTreeMap::new();
        inner2.insert("C".to_string(), 20);
        changes.insert(50, inner2);

        let result = fill_and_sort_tmp(&changes);
        assert_eq!(result.len(), 3);
        // First two entries have count=20; among those, position 50 < 100
        assert_eq!(result[0].count, 20);
        assert_eq!(result[1].count, 20);

        // When count is equal and position is equal, desc description takes effect
        // C@50 count=20, B@100 count=20: 50 < 100, so C first
        assert_eq!(result[0].position, 50);
        assert_eq!(result[0].description_string, "C");
        assert_eq!(result[1].position, 100);
        assert_eq!(result[1].description_string, "B");
        assert_eq!(result[2].count, 10);
        assert_eq!(result[2].description_string, "A");
    }

    #[test]
    fn test_fill_and_sort_tmp_tertiary_sort() {
        // Same count, same position — must sort descending by description
        let mut changes: HashMap<i32, BTreeMap<String, i32>> = HashMap::new();
        let mut inner = BTreeMap::new();
        inner.insert("AAA".to_string(), 5);
        inner.insert("ZZZ".to_string(), 5);
        inner.insert("MMM".to_string(), 5);
        changes.insert(100, inner);

        let result = fill_and_sort_tmp(&changes);
        assert_eq!(result.len(), 3);
        // Descending description: ZZZ > MMM > AAA
        assert_eq!(result[0].description_string, "ZZZ");
        assert_eq!(result[1].description_string, "MMM");
        assert_eq!(result[2].description_string, "AAA");
    }

    #[test]
    fn test_islowcomplexseq() {
        assert!(islowcomplexseq(""));
        assert!(islowcomplexseq("AAAAAAAAAA"));
        assert!(islowcomplexseq("ATATATATAT")); // only 2 distinct bases
        assert!(!islowcomplexseq("ATCGATCGAT")); // 4 distinct bases, no single > 75%
        assert!(islowcomplexseq("AAAAAAAAAT")); // A=90% > 75%
    }

    #[test]
    fn test_count_char() {
        assert_eq!(count_char("ATCGATCG", 'A'), 2);
        assert_eq!(count_char("ATCGATCG", 'T'), 2);
        assert_eq!(count_char("AAAA", 'A'), 4);
        assert_eq!(count_char("ATCG", 'N'), 0);
        assert_eq!(count_char("", 'A'), 0);
    }

    #[test]
    fn test_ismatch_basic() {
        init_test_scope();
        // Forward direction, perfect match
        assert!(ismatch("ATCG", "ATCG", 1));
        // Reverse direction, perfect match — seq2 read from end
        assert!(ismatch("GCTA", "ATCG", -1));
        // Too many mismatches
        assert!(!ismatch("XXXX", "ATCG", 1));
    }

    #[test]
    fn test_ismatchref_basic() {
        init_test_scope();
        let mut ref_map: HashMap<i32, u8> = HashMap::new();
        ref_map.insert(100, b'A');
        ref_map.insert(101, b'T');
        ref_map.insert(102, b'C');
        ref_map.insert(103, b'G');

        // Forward direction match
        assert!(ismatchref("ATCG", &ref_map, 100, 1));
        // Mismatch
        assert!(!ismatchref("XXXX", &ref_map, 100, 1));
    }

    #[test]
    fn test_adj_ins_pos_no_change() {
        init_test_scope();
        // When ref base at bi doesn't match last char of ins, no adjustment
        let mut ref_map: HashMap<i32, u8> = HashMap::new();
        ref_map.insert(100, b'X');
        ref_map.insert(99, b'X');

        let result = adj_ins_pos(100, "ATCG", &ref_map);
        assert_eq!(result.base_insert, Some(100));
        assert_eq!(result.insertion_sequence, "ATCG");
    }

    #[test]
    fn test_adj_ins_pos_shift_left() {
        init_test_scope();
        // ref[100]=G, ins="ATCG" -> last char is G, shift left
        // ref[99]=C -> ins shifted: "GATC" at 99
        // ref[98]=T -> ins shifted: "CGAT" at 98
        // ref[97]=A -> ins shifted: "TCGA" at 97
        // ref[96]=X -> stop
        let mut ref_map: HashMap<i32, u8> = HashMap::new();
        ref_map.insert(100, b'G');
        ref_map.insert(99, b'C');
        ref_map.insert(98, b'T');
        ref_map.insert(97, b'A');
        ref_map.insert(96, b'X');

        let result = adj_ins_pos(100, "ATCG", &ref_map);
        // Should shift bi from 100 down to 96, rotating ins
        assert_eq!(result.base_insert, Some(96));
    }

    #[test]
    fn test_find35match_basic() {
        init_test_scope();
        // Trivial case: no match when sequences are too short
        let result = find35match("ATCGATCG", "ATCGATCG");
        // seq5.length()-8 = 0, so outer loop doesn't execute
        assert_eq!(result.max_matched_length, 0);
    }

    #[test]
    fn test_rm_cnt() {
        let mut vref = Variation {
            vars_count: 100,
            high_quality_reads_count: 50,
            low_quality_reads_count: 20,
            mean_position: 10.0,
            mean_quality: 30.0,
            mean_mapping_quality: 40.0,
            vars_count_on_forward: 30,
            vars_count_on_reverse: 70,
            ..Variation::default()
        };
        let tv = Variation {
            vars_count: 10,
            high_quality_reads_count: 5,
            low_quality_reads_count: 2,
            mean_position: 1.0,
            mean_quality: 3.0,
            mean_mapping_quality: 4.0,
            vars_count_on_forward: 3,
            vars_count_on_reverse: 7,
            ..Variation::default()
        };
        rm_cnt(&mut vref, &tv);
        assert_eq!(vref.vars_count, 90);
        assert_eq!(vref.high_quality_reads_count, 45);
        assert_eq!(vref.low_quality_reads_count, 18);
        assert!((vref.mean_position - 9.0).abs() < 1e-10);
        assert!((vref.mean_quality - 27.0).abs() < 1e-10);
        assert!((vref.mean_mapping_quality - 36.0).abs() < 1e-10);
        assert_eq!(vref.vars_count_on_forward, 27);
        assert_eq!(vref.vars_count_on_reverse, 63);
    }

    #[test]
    fn test_adj_ref_factor_basic() {
        init_test_scope();
        let mut ref_var = Variation {
            vars_count: 100,
            high_quality_reads_count: 50,
            low_quality_reads_count: 20,
            mean_position: 10.0,
            mean_quality: 30.0,
            mean_mapping_quality: 40.0,
            number_of_mismatches: 5.0,
            vars_count_on_forward: 60,
            vars_count_on_reverse: 40,
            ..Variation::default()
        };

        adj_ref_factor(Some(&mut ref_var), 0.5);
        // varsCount -= (0.5 * 100) = 50
        assert_eq!(ref_var.vars_count, 50);
        // hqrc -= (0.5 * 50) = 25
        assert_eq!(ref_var.high_quality_reads_count, 25);
    }

    #[test]
    fn test_add_var_factor_basic() {
        let mut vref = Variation {
            vars_count: 100,
            high_quality_reads_count: 50,
            low_quality_reads_count: 20,
            mean_position: 10.0,
            mean_quality: 30.0,
            mean_mapping_quality: 40.0,
            number_of_mismatches: 5.0,
            vars_count_on_forward: 60,
            vars_count_on_reverse: 40,
            ..Variation::default()
        };

        add_var_factor(&mut vref, 0.5);
        // varsCount += (0.5 * 100) = 50 → 150
        assert_eq!(vref.vars_count, 150);
        assert_eq!(vref.high_quality_reads_count, 75);
    }

    #[test]
    fn test_add_var_factor_negative_limit() {
        let mut vref = Variation {
            vars_count: 100,
            ..Variation::default()
        };
        // factor_f < -1.0 should be a no-op
        add_var_factor(&mut vref, -1.5);
        assert_eq!(vref.vars_count, 100);
    }

    #[test]
    fn test_adj_ref_cnt_null_ref() {
        // When ref_var is None, should be a no-op
        let tv = Variation {
            vars_count: 10,
            ..Variation::default()
        };
        adj_ref_cnt(&tv, None, 5);
        // No panic = pass
    }

    #[test]
    fn test_adj_ref_factor_null() {
        // When ref_var is None, should be a no-op
        adj_ref_factor(None, 0.5);
        // No panic = pass
    }

    #[test]
    fn test_comp2_ordering() {
        use crate::data::{Sclip, SortPositionSclip};

        let mut a_clip = Sclip::default();
        a_clip.base.vars_count = 10;
        let a = SortPositionSclip::new(100, a_clip, 0);

        let mut b_clip = Sclip::default();
        b_clip.base.vars_count = 20;
        let b = SortPositionSclip::new(50, b_clip, 0);

        // b has higher varsCount, so it should come first
        let result = comp2(&a, &b);
        assert_eq!(result, std::cmp::Ordering::Greater);
    }

    #[test]
    fn test_comp3_ordering() {
        use crate::data::{Sclip, SortPositionSclip};

        let a = SortPositionSclip::new(100, Sclip::default(), 10);
        let b = SortPositionSclip::new(50, Sclip::default(), 20);

        // b has higher count, so it should come first
        let result = comp3(&a, &b);
        assert_eq!(result, std::cmp::Ordering::Greater);
    }
}
