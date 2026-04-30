//! ToVarsBuilder — transforms raw Variation maps into final Variant objects.
//!
//! Ported from: ToVarsBuilder.java (1,194 LOC)
//! Pipeline stage: `Scope<RealignedVariationData>` → `Scope<AlignedVarsData>`

use std::collections::HashMap;

use regex::Regex;

use crate::config::{Configuration, SVFLANK};
use crate::data::{AlignedVarsData, Region, Variant, VariationMap, Vars};
use crate::patterns::{
    AMP_ATGC, ANY_SV, BEGIN_DIGITS, BEGIN_MINUS_NUMBER, BEGIN_MINUS_NUMBER_CARET, CARET_ATGNC,
    DUP_NUM, HASH_GROUP_CARET_GROUP, INV_NUM, SOME_SV_NUMBERS,
};
use crate::scope::GlobalReadOnlyScope;
use crate::utils::{round_half_even, substr, substr_with_len};
use crate::variations::{
    get_or_put_vars, get_variation_maybe, get_variation_maybe_mut, join_ref, strand_bias,
};

// Java: ToVarsBuilder.REF_20_BASES .. REF_70_BASES
const REF_20_BASES: i32 = 20;
const REF_30_BASES: i32 = 30;
const REF_50_BASES: i32 = 50;
const REF_70_BASES: i32 = 70;

fn chromosome_limit(region: &Region, ref_map: &HashMap<i32, u8>) -> i32 {
    GlobalReadOnlyScope::instance()
        .chr_lengths
        .get(&region.chr)
        .copied()
        .or_else(|| ref_map.keys().copied().max())
        .unwrap_or(0)
}

/// IUPAC ambiguity code replacements.
/// Ported from: ToVarsBuilder.java:L54-L65
fn iupac_replacement(code: char) -> Option<char> {
    match code {
        'M' => Some('A'),
        'R' => Some('A'),
        'W' => Some('A'),
        'S' => Some('C'),
        'Y' => Some('C'),
        'K' => Some('G'),
        'V' => Some('A'),
        'H' => Some('A'),
        'D' => Some('A'),
        'B' => Some('C'),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Cluster B: MSI + validateRefallele
// ---------------------------------------------------------------------------

fn repeated_motif_suffix_len(sequence: &str, motif: &str) -> usize {
    let sequence = sequence.as_bytes();
    let motif = motif.as_bytes();
    let motif_len = motif.len();
    if motif_len == 0 {
        return 0;
    }

    let mut matched = 0usize;
    while matched + motif_len <= sequence.len() {
        let start = sequence.len() - matched - motif_len;
        if &sequence[start..start + motif_len] != motif {
            break;
        }
        matched += motif_len;
    }
    matched
}

fn repeated_motif_prefix_len(sequence: &str, motif: &str) -> usize {
    let sequence = sequence.as_bytes();
    let motif = motif.as_bytes();
    let motif_len = motif.len();
    if motif_len == 0 {
        return 0;
    }

    let mut matched = 0usize;
    while matched + motif_len <= sequence.len() {
        if &sequence[matched..matched + motif_len] != motif {
            break;
        }
        matched += motif_len;
    }
    matched
}

/// Ported from: ToVarsBuilder.java:L573-L621
/// Find microsatellite instability by testing repeat patterns of length 1-6 bases.
///
/// Parity traps addressed: T20 (repeat matching), T14 (msint length stored not string)
pub fn find_msi(tseq1: &str, tseq2: &str, left: Option<&str>) -> (f64, i32, String) {
    let mut nmsi: usize = 1;
    let mut shift3: i32;
    let mut maxmsi = String::new();
    let mut msicnt: f64 = 0.0;

    while nmsi <= tseq1.len() && nmsi <= 6 {
        // Trap T20: regex compiled fresh each iteration — match Java behavior
        let msint_bytes = substr_with_len(tseq1.as_bytes(), -(nmsi as i32), nmsi as i32);
        let msint = String::from_utf8_lossy(&msint_bytes).to_string();

        // Java regex: ((msint)+)$
        let mut msimatch_len = repeated_motif_suffix_len(tseq1, &msint);

        // If left context is provided and non-empty, try concatenated match
        if let Some(left_str) = left {
            if !left_str.is_empty() {
                let combined = format!("{}{}", left_str, tseq1);
                // Overwrites previous match entirely (Java behavior)
                msimatch_len = repeated_motif_suffix_len(&combined, &msint);
            }
        }

        let mut curmsi = msimatch_len as f64 / nmsi as f64;

        // Java regex: ^((msint)+)
        curmsi += repeated_motif_prefix_len(tseq2, &msint) as f64 / nmsi as f64;

        if curmsi > msicnt {
            maxmsi = msint;
            msicnt = curmsi;
        }

        nmsi += 1;
    }

    // Compute shift3: compare tseq1+tseq2 character by character with tseq2
    let tseq = format!("{}{}", tseq1, tseq2);
    let tseq_bytes = tseq.as_bytes();
    let tseq2_bytes = tseq2.as_bytes();
    shift3 = 0;
    while (shift3 as usize) < tseq2_bytes.len()
        && (shift3 as usize) < tseq_bytes.len()
        && tseq_bytes[shift3 as usize] == tseq2_bytes[shift3 as usize]
    {
        shift3 += 1;
    }

    (msicnt, shift3, maxmsi)
}

/// Ported from: ToVarsBuilder.java:L221-L247
/// Compute MSI/shift3 for deletion variants.
///
/// Parity traps addressed: T21 (shift3 preserved from first call), T22 (msi <= comparison),
/// T23 (unwrap_or(0) for chrLengths)
pub fn proceed_vref_is_deletion(
    position: i32,
    dellen: i32,
    ref_map: &HashMap<i32, u8>,
    region: &Region,
    _conf: &Configuration,
) -> (f64, i32, String) {
    // left 70 bases in reference sequence
    let leftseq = join_ref(ref_map, (position - REF_70_BASES).max(1), position - 1);
    let chr0 = chromosome_limit(region, ref_map); // Trap T23: non-mutating
    // right dellen+70 bases in reference sequence
    let tseq = join_ref(
        ref_map,
        position,
        (position + dellen + REF_70_BASES).min(chr0),
    );

    let tseq_bytes = tseq.as_bytes();
    let tseq1_str = String::from_utf8_lossy(&substr_with_len(tseq_bytes, 0, dellen)).to_string();
    let tseq2_str = String::from_utf8_lossy(&substr(tseq_bytes, dellen)).to_string();

    // First findMSI call
    let (mut msi, shift3, mut msint) = find_msi(&tseq1_str, &tseq2_str, Some(&leftseq));

    // Second findMSI call — shift3 NOT updated (Trap T21)
    let (tmsi, _tshift3, tmsint) = find_msi(&leftseq, &tseq2_str, None);
    if msi < tmsi {
        msi = tmsi;
        msint = tmsint;
        // Don't change shift3 — Trap T21
    }
    // Trap T22: uses <=
    if msi <= shift3 as f64 / dellen as f64 {
        msi = shift3 as f64 / dellen as f64;
    }

    (msi, shift3, msint)
}

/// Ported from: ToVarsBuilder.java:L254-L285
/// Compute MSI/shift3 for insertion variants.
///
/// Parity traps addressed: T21 (shift3 preserved), T22 (msi <= comparison),
/// T23 (unwrap_or(0) for chrLengths)
pub fn proceed_vref_is_insertion(
    position: i32,
    vn: &str,
    ref_map: &HashMap<i32, u8>,
    region: &Region,
    _conf: &Configuration,
) -> (f64, i32, String) {
    // tseq1 = insertion sequence without leading '+'
    let tseq1 = &vn[1..];
    // left 50 bases (inclusive of position)
    let leftseq = join_ref(ref_map, (position - REF_50_BASES).max(1), position);
    let x = chromosome_limit(region, ref_map); // Trap T23: non-mutating
    // right 70 bases
    let tseq2 = join_ref(ref_map, position + 1, (position + REF_70_BASES).min(x));

    let (mut msi, shift3, mut msint) = find_msi(tseq1, &tseq2, Some(&leftseq));

    let (tmsi, _tshift3, tmsint) = find_msi(&leftseq, &tseq2, None);
    if msi < tmsi {
        msi = tmsi;
        msint = tmsint;
        // Don't change shift3 — Trap T21
    }
    // Trap T22: uses <=
    if tseq1.is_empty() {
        // avoid division by zero — shouldn't happen but guard
    } else if msi <= shift3 as f64 / tseq1.len() as f64 {
        msi = shift3 as f64 / tseq1.len() as f64;
    }

    (msi, shift3, msint)
}

/// Ported from: ToVarsBuilder.java:L1039-L1048
/// Replace IUPAC ambiguity codes with standard bases.
///
/// Parity traps addressed: T15 (replaceFirst with regex)
pub fn validate_refallele(refallele: &str) -> String {
    let mut result = refallele.to_string();
    for i in 0..refallele.len() {
        let ref_base = refallele.as_bytes()[i] as char;
        if let Some(replacement) = iupac_replacement(ref_base) {
            // Java replaceFirst — replaces first occurrence of the char
            // Single uppercase letter is a valid regex literal
            if let Some(pos) = result.find(ref_base) {
                result.replace_range(pos..pos + ref_base.len_utf8(), &replacement.to_string());
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Cluster A: Variant Construction & Statistics
// ---------------------------------------------------------------------------

/// Ported from: ToVarsBuilder.java:L548-L564
/// Sum high-quality read counts across all non-insertion variants at position.
///
/// Parity traps addressed: insertion hicov intentionally NOT added (commented out in Java)
pub fn calc_hicov(
    insertion_variations: Option<&VariationMap>,
    non_insertion_variations: &VariationMap,
) -> i32 {
    let mut hicov: i32 = 0;
    for (key, variation) in &non_insertion_variations.entries {
        if key == "SV" || key.starts_with('+') {
            continue;
        }
        hicov += variation.high_quality_reads_count;
    }
    // Insertion high-quality counts are intentionally NOT added (Java commented out)
    let _ = insertion_variations;
    hicov
}

/// Ported from: ToVarsBuilder.java:L517-L540
/// Clamp negative forward/reverse counts to zero with stderr warning.
pub fn adjust_variant_counts(p: i32, vref: &mut Variant) {
    let message = format!(
        "column in variant on position: {} {}->{}  was negative, adjusted to zero.",
        p, vref.refallele, vref.varallele
    );

    if vref.ref_forward_coverage < 0 {
        vref.ref_forward_coverage = 0;
        eprintln!("Reference forward count {}", message);
    }
    if vref.ref_reverse_coverage < 0 {
        vref.ref_reverse_coverage = 0;
        eprintln!("Reference reverse count {}", message);
    }
    if vref.vars_count_on_forward < 0 {
        vref.vars_count_on_forward = 0;
        eprintln!("Variant forward count {}", message);
    }
    if vref.vars_count_on_reverse < 0 {
        vref.vars_count_on_reverse = 0;
        eprintln!("Variant reverse count {}", message);
    }
}

/// Ported from: ToVarsBuilder.java:L320-L331
/// Sort variants by quality*coverage DESC, descriptionString ASC (stable sort).
///
/// Parity traps addressed: T7 (stable sort, Double.compare semantics)
pub fn sort_variants(var: &mut Vec<Variant>) {
    var.sort_by(|a, b| {
        let score_a = a.mean_quality * a.position_coverage as f64;
        let score_b = b.mean_quality * b.position_coverage as f64;
        // Descending by score
        let cmp = score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal);
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
        // Ascending by description string
        a.description_string.cmp(&b.description_string)
    });
}

/// Ported from: ToVarsBuilder.java:L436-L510
/// Convert non-insertion Variations into Variant objects.
///
/// Parity traps addressed: T4 (keys sorted lexicographically), T12 (extracnt > 0 not != 0),
/// T3 (locnt != 0 ? locnt : 0.5)
#[allow(clippy::too_many_arguments)]
pub fn create_variant(
    duprate: f64,
    aligned_vars: &mut HashMap<i32, Vars>,
    position: i32,
    non_insertion_variations: &VariationMap,
    total_pos_coverage: i32,
    var: &mut Vec<Variant>,
    debug_lines: &mut Vec<String>,
    keys: &[String],
    hicov: i32,
    conf: &Configuration,
) {
    for description_string in keys {
        if description_string == "SV" {
            // Trap T13: SV string format
            if let Some(sv) = &non_insertion_variations.sv {
                get_or_put_vars(aligned_vars, position).sv =
                    format!("{}-{}-{}", sv.splits, sv.pairs, sv.clusters);
            }
            continue;
        }

        let cnt = match non_insertion_variations.entries.get(description_string) {
            Some(c) => c,
            None => continue,
        };

        if cnt.vars_count == 0 {
            continue; // Skip zero-count variants
        }

        let fwd = cnt.vars_count_on_forward;
        let rev = cnt.vars_count_on_reverse;
        let bias = strand_bias(fwd, rev);
        let base_quality = round_half_even("0.0", cnt.mean_quality / cnt.vars_count as f64);
        let mapping_quality =
            round_half_even("0.0", cnt.mean_mapping_quality / cnt.vars_count as f64);
        let hicnt = cnt.high_quality_reads_count;
        let locnt = cnt.low_quality_reads_count;

        // Trap T12: extracnt > 0 (not != 0) for createVariant
        let mut ttcov = total_pos_coverage;
        if cnt.vars_count > total_pos_coverage
            && cnt.extracnt > 0
            && cnt.vars_count - total_pos_coverage < cnt.extracnt
        {
            ttcov = cnt.vars_count;
        }

        // Trap T3: locnt != 0 ? locnt : 0.5
        let lo_divisor = if locnt != 0 { locnt as f64 } else { 0.5_f64 };

        let mut tvref = Variant::default();
        tvref.description_string = description_string.clone();
        tvref.position_coverage = cnt.vars_count;
        tvref.vars_count_on_forward = fwd;
        tvref.vars_count_on_reverse = rev;
        tvref.strand_bias_flag = bias.to_string();
        tvref.frequency = round_half_even("0.0000", cnt.vars_count as f64 / ttcov as f64);
        tvref.mean_position = round_half_even("0.0", cnt.mean_position / cnt.vars_count as f64);
        tvref.is_at_least_at_2_positions = cnt.pstd;
        tvref.mean_quality = base_quality;
        tvref.has_at_least_2_diff_qualities = cnt.qstd;
        tvref.mean_mapping_quality = mapping_quality;
        tvref.high_quality_to_low_quality_ratio = hicnt as f64 / lo_divisor;
        tvref.high_quality_reads_frequency = if hicov > 0 {
            hicnt as f64 / hicov as f64
        } else {
            0.0
        };
        tvref.extra_frequency = if cnt.extracnt != 0 {
            cnt.extracnt as f64 / ttcov as f64
        } else {
            0.0
        };
        tvref.shift3 = 0;
        tvref.msi = 0.0;
        tvref.number_of_mismatches =
            round_half_even("0.0", cnt.number_of_mismatches / cnt.vars_count as f64);
        tvref.hicnt = hicnt;
        tvref.hicov = hicov;
        tvref.duprate = duprate;

        var.push(tvref);

        if conf.debug {
            debug_lines.push(format!(
                "Variant: {} Cnt={} Fwd={} Rev={}",
                description_string, cnt.vars_count, fwd, rev
            ));
        }
    }
}

/// Ported from: ToVarsBuilder.java:L340-L424
/// Convert insertion Variations into Variant objects. Returns updated totalPosCoverage.
///
/// Parity traps addressed: T12 (extracnt != 0 for insertion), T24 (cross-position mutation),
/// T25 (totalPosCoverage returned), T3 (locnt divisor)
#[allow(clippy::too_many_arguments)]
pub fn create_insertion(
    duprate: f64,
    position: i32,
    mut total_pos_coverage: i32,
    var: &mut Vec<Variant>,
    debug_lines: &mut Vec<String>,
    mut hicov: i32,
    insertion_variants: &HashMap<i32, VariationMap>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    ref_coverage: &HashMap<i32, i32>,
    ref_map: &HashMap<i32, u8>,
    conf: &Configuration,
) -> i32 {
    let insertion_variations = match insertion_variants.get(&position) {
        Some(vm) => vm,
        None => return total_pos_coverage,
    };

    let mut insertion_desc_strings: Vec<String> =
        insertion_variations.entries.keys().cloned().collect();
    insertion_desc_strings.sort(); // Trap T5: lexicographic sort

    for description_string in &insertion_desc_strings {
        // Trap T25: totalPosCoverage updated for '&' descriptions
        if description_string.contains('&') && ref_coverage.contains_key(&(position + 1)) {
            total_pos_coverage = *ref_coverage.get(&(position + 1)).unwrap();
        }

        let cnt = match insertion_variations.entries.get(description_string) {
            Some(c) => c,
            None => continue,
        };

        let fwd = cnt.vars_count_on_forward;
        let rev = cnt.vars_count_on_reverse;
        let bias = strand_bias(fwd, rev);
        let vqual = round_half_even("0.0", cnt.mean_quality / cnt.vars_count as f64);
        let mq = round_half_even("0.0", cnt.mean_mapping_quality / cnt.vars_count as f64);
        let hicnt = cnt.high_quality_reads_count;
        let locnt = cnt.low_quality_reads_count;

        // Trap T12: extracnt != 0 for createInsertion (not > 0)
        let mut ttcov = total_pos_coverage;
        if cnt.vars_count > total_pos_coverage
            && cnt.extracnt != 0
            && cnt.vars_count - total_pos_coverage < cnt.extracnt
        {
            ttcov = cnt.vars_count;
        }

        if ttcov < cnt.vars_count {
            ttcov = cnt.vars_count;
            // Trap T24: cross-position mutation
            if ref_coverage.contains_key(&(position + 1))
                && ttcov < *ref_coverage.get(&(position + 1)).unwrap() - cnt.vars_count
            {
                ttcov = *ref_coverage.get(&(position + 1)).unwrap();
                // Adjust the reference variation at position+1
                let ref_base_next = ref_map.get(&(position + 1)).copied();
                if let Some(variant_next) =
                    get_variation_maybe_mut(non_insertion_variants, position + 1, ref_base_next)
                {
                    variant_next.vars_count_on_forward -= fwd;
                    variant_next.vars_count_on_reverse -= rev;
                }
            }
            total_pos_coverage = ttcov;
        }

        if hicov < hicnt {
            hicov = hicnt;
        }

        // Trap T3: locnt != 0 ? locnt : 0.5
        let lo_divisor = if locnt != 0 { locnt as f64 } else { 0.5_f64 };

        let mut tvref = Variant::default();
        tvref.description_string = description_string.clone();
        tvref.position_coverage = cnt.vars_count;
        tvref.vars_count_on_forward = fwd;
        tvref.vars_count_on_reverse = rev;
        tvref.strand_bias_flag = bias.to_string();
        tvref.frequency = round_half_even("0.0000", cnt.vars_count as f64 / ttcov as f64);
        tvref.mean_position = round_half_even("0.0", cnt.mean_position / cnt.vars_count as f64);
        tvref.is_at_least_at_2_positions = cnt.pstd;
        tvref.mean_quality = vqual;
        tvref.has_at_least_2_diff_qualities = cnt.qstd;
        tvref.mean_mapping_quality = mq;
        tvref.high_quality_to_low_quality_ratio = hicnt as f64 / lo_divisor;
        tvref.high_quality_reads_frequency = if hicov > 0 {
            hicnt as f64 / hicov as f64
        } else {
            0.0
        };
        tvref.extra_frequency = if cnt.extracnt != 0 {
            cnt.extracnt as f64 / ttcov as f64
        } else {
            0.0
        };
        tvref.shift3 = 0;
        tvref.msi = 0.0;
        tvref.number_of_mismatches =
            round_half_even("0.0", cnt.number_of_mismatches / cnt.vars_count as f64);
        tvref.hicnt = hicnt;
        tvref.hicov = hicov;
        tvref.duprate = duprate;

        var.push(tvref);

        if conf.debug {
            debug_lines.push(format!(
                "InsVariant: {} Cnt={} Fwd={} Rev={}",
                description_string, cnt.vars_count, fwd, rev
            ));
        }
    }

    total_pos_coverage
}

/// Ported from: ToVarsBuilder.java:L293-L314
/// Distribute variants into reference vs. non-reference buckets. Returns maxfreq.
///
/// Parity traps addressed: T8 (ref.get(position) null → Option, String.valueOf(null) → "null")
pub fn collect_vars_at_position(
    aligned_variants: &mut HashMap<i32, Vars>,
    position: i32,
    var: &[Variant],
    ref_map: &HashMap<i32, u8>,
) -> f64 {
    let mut maxfreq: f64 = 0.0;
    for tvar in var {
        // Trap T8: ref.get(position) may be None.
        // Java String.valueOf(null) returns "null" which never matches a single base.
        let ref_desc = ref_map
            .get(&position)
            .map(|b| char::from(*b).to_string())
            .unwrap_or_else(|| "null".to_string());

        if tvar.description_string == ref_desc {
            get_or_put_vars(aligned_variants, position).reference_variant = Some(tvar.clone());
        } else {
            let vars = get_or_put_vars(aligned_variants, position);
            vars.variants.push(tvar.clone());
            vars.var_description_string_to_variants
                .insert(tvar.description_string.clone(), tvar.clone());
            if tvar.frequency > maxfreq {
                maxfreq = tvar.frequency;
            }
        }
    }
    maxfreq
}

/// Ported from: ToVarsBuilder.java:L199-L213
/// Check if position should be skipped because only reference variant exists.
///
/// Parity traps addressed: T27 (synthetic "I" key to prevent skipping insertion positions)
pub fn is_the_same_variation_on_ref(
    position: i32,
    vars_at_cur_position: &VariationMap,
    insertion_variants: &HashMap<i32, VariationMap>,
    ref_map: &HashMap<i32, u8>,
    conf: &Configuration,
) -> bool {
    let mut single_key: Option<&str> = None;
    for key in vars_at_cur_position.entries.keys() {
        if let Some(existing_key) = single_key {
            if existing_key != key.as_str() {
                return false;
            }
        } else {
            single_key = Some(key);
        }
    }

    // Trap T27: add synthetic "I" key
    if insertion_variants.contains_key(&position) {
        if let Some(existing_key) = single_key {
            if existing_key != "I" {
                return false;
            }
        } else {
            single_key = Some("I");
        }
    }
    if let Some(only_key) = single_key {
        if let Some(ref_base) = ref_map.get(&position) {
            if only_key.len() == 1 && only_key.as_bytes()[0] == *ref_base {
                let has_amplicon_based_calling = GlobalReadOnlyScope::with_instance(|scope| {
                    scope.amplicon_based_calling.is_some()
                });
                if !conf.do_pileup
                    && !conf.bam.as_ref().map_or(false, |b| b.has_bam2())
                    && !has_amplicon_based_calling
                {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Cluster C: collectReferenceVariants + process
// ---------------------------------------------------------------------------

/// Ported from: ToVarsBuilder.java:L1018-L1030
/// Build DEBUG field from accumulated debug lines.
fn construct_debug_lines(debug_lines: &[String], vref: &mut Variant, conf: &Configuration) {
    if conf.debug {
        let mut sb = String::new();
        for s in debug_lines {
            if !sb.is_empty() {
                sb.push_str(" & ");
            }
            sb.push_str(s);
        }
        vref.debug = sb;
    }
}

/// Ported from: ToVarsBuilder.java:L971-L1013
/// Fill reference variant fields when only reference reads exist (or pileup update).
///
/// Parity traps addressed: T36 (leftseq/rightseq empty), T30 (double-update in pileup),
/// T33 (strandBiasFlag semicolon check)
fn update_ref_variant(
    position: i32,
    total_pos_coverage: i32,
    vref: &mut Variant,
    debug_lines: &[String],
    reference_forward_coverage: i32,
    reference_reverse_coverage: i32,
    ref_map: &HashMap<i32, u8>,
    duprate: f64,
    conf: &Configuration,
) {
    vref.total_pos_coverage = total_pos_coverage;
    vref.position_coverage = 0;
    vref.frequency = 0.0;
    vref.ref_forward_coverage = reference_forward_coverage;
    vref.ref_reverse_coverage = reference_reverse_coverage;
    vref.vars_count_on_forward = 0;
    vref.vars_count_on_reverse = 0;
    vref.msi = 0.0;
    vref.msint = 0;
    // Trap T33: append ";0" only if no semicolon present
    if !vref.strand_bias_flag.contains(';') {
        vref.strand_bias_flag.push_str(";0");
    }
    vref.shift3 = 0;
    vref.start_position = position;
    vref.end_position = position;
    vref.high_quality_reads_frequency =
        round_half_even("0.0000", vref.high_quality_reads_frequency);
    let reference_base = ref_map
        .get(&position)
        .map(|b| char::from(*b).to_string())
        .unwrap_or_default();
    vref.refallele = validate_refallele(&reference_base);
    vref.varallele = validate_refallele(&reference_base);
    vref.genotype = Some(format!("{}/{}", reference_base, reference_base));
    // Trap T36: leftseq/rightseq set to empty
    vref.leftseq = String::new();
    vref.rightseq = String::new();
    vref.duprate = duprate;

    construct_debug_lines(debug_lines, vref, conf);
}

/// Ported from: ToVarsBuilder.java:L629-L969
/// Core finalization: genotype, alleles, MSI, CRISPR, flanking, strand bias.
///
/// Parity traps addressed: T9,T11,T13,T15-19,T25,T26,T29,T30,T31-32,T33,T34,T35,T36
#[allow(clippy::too_many_arguments, clippy::cognitive_complexity)]
pub fn collect_reference_variants(
    position: i32,
    mut total_pos_coverage: i32,
    variations_at_pos: &mut Vars,
    debug_lines: &[String],
    ref_map: &HashMap<i32, u8>,
    region: &Region,
    ref_coverage: &HashMap<i32, i32>,
    non_insertion_variants: &HashMap<i32, VariationMap>,
    duprate: f64,
    conf: &Configuration,
) {
    let mut reference_forward_coverage: i32 = 0;
    let mut reference_reverse_coverage: i32 = 0;

    // Step 3: Determine genotype1
    // Trap T9: referenceVariant can be None
    let genotype1: String;
    if let Some(ref rv) = variations_at_pos.reference_variant {
        if rv.frequency >= conf.freq {
            genotype1 = rv.description_string.clone();
        } else if !variations_at_pos.variants.is_empty() {
            genotype1 = variations_at_pos.variants[0].description_string.clone();
        } else {
            genotype1 = rv.description_string.clone();
        }
    } else if !variations_at_pos.variants.is_empty() {
        genotype1 = variations_at_pos.variants[0].description_string.clone();
    } else {
        // Trap T9: NPE in Java if referenceVariant is None and variants empty
        // Graceful handling: use empty string
        genotype1 = String::new();
    }

    // Step 4: reference fwd/rev coverage
    if let Some(ref rv) = variations_at_pos.reference_variant {
        reference_forward_coverage = rv.vars_count_on_forward;
        reference_reverse_coverage = rv.vars_count_on_reverse;
    }

    // Step 5: Adjust genotype1 for insertions/duplications
    let mut genotype1 = genotype1;
    if genotype1.starts_with('+') {
        if let Some(caps) = DUP_NUM.captures(&genotype1) {
            let dup_count: i32 = caps[1].parse().unwrap_or(0);
            genotype1 = format!("+{}", SVFLANK + dup_count);
        } else {
            genotype1 = format!("+{}", genotype1.len() - 1);
        }
    }

    // Step 7: Adjust reference coverage from next position
    if total_pos_coverage > *ref_coverage.get(&position).unwrap_or(&0)
        && non_insertion_variants.contains_key(&(position + 1))
        && ref_map.contains_key(&(position + 1))
    {
        let next_ref_str = char::from(*ref_map.get(&(position + 1)).unwrap()).to_string();
        if non_insertion_variants
            .get(&(position + 1))
            .map_or(false, |vm| vm.entries.contains_key(&next_ref_str))
        {
            if let Some(tpref) = get_variation_maybe(
                non_insertion_variants,
                position + 1,
                ref_map.get(&(position + 1)).copied(),
            ) {
                reference_forward_coverage = tpref.vars_count_on_forward;
                reference_reverse_coverage = tpref.vars_count_on_reverse;
            }
        }
    }

    let mut positions_for_changed_ref_variant: Vec<i32> = Vec::new();
    let scope = GlobalReadOnlyScope::instance();
    let chr0 = chromosome_limit(region, ref_map);

    // Step 9: Non-reference variants exist
    if !variations_at_pos.variants.is_empty() {
        let variant_count = variations_at_pos.variants.len();
        for vi in 0..variant_count {
            let mut genotype1current = genotype1.clone();
            let description_string = variations_at_pos.variants[vi].description_string.clone();
            let mut genotype2 = description_string.clone();
            if genotype2.starts_with('+') {
                genotype2 = format!("+{}", genotype2.len() - 1);
            }

            let mut deletion_length: i32 = 0;
            if let Some(caps) = BEGIN_MINUS_NUMBER.captures(&description_string) {
                deletion_length = caps[1].parse().unwrap_or(0);
            }

            let mut end_position = position;
            if description_string.starts_with('-') {
                end_position = position + deletion_length - 1;
            }

            let mut refallele = String::new();
            let mut varallele: String;
            let mut shift3: i32 = 0;
            let mut msi: f64 = 0.0;
            let mut msint = String::new();
            let mut start_position = position;

            // Branch: Insertion
            if description_string.starts_with('+') {
                if !description_string.contains('&')
                    && !description_string.contains('#')
                    && !description_string.contains("<dup")
                {
                    let (m, s, ms) = proceed_vref_is_insertion(
                        position,
                        &description_string,
                        ref_map,
                        region,
                        conf,
                    );
                    msi = m;
                    shift3 = s;
                    msint = ms;
                }
                if conf.move_indels_to_3 {
                    start_position += shift3;
                    end_position += shift3;
                }
                refallele = ref_map
                    .get(&position)
                    .map(|b| char::from(*b).to_string())
                    .unwrap_or_default();
                varallele = format!("{}{}", refallele, &description_string[1..]);
                let mut varallele = varallele;

                if varallele.len() as i32 > conf.svminlen {
                    end_position += varallele.len() as i32;
                    varallele = "<DUP>".to_string();
                }
                if let Some(caps) = DUP_NUM.captures(&varallele.clone()) {
                    let dup_count: i32 = caps[1].parse().unwrap_or(0);
                    end_position = start_position + (2 * SVFLANK + dup_count) - 1;
                    genotype2 = format!("+{}", 2 * SVFLANK + dup_count);
                    varallele = "<DUP>".to_string();
                }

                // Continue with the rest using varallele
                process_variant_finalization(
                    vi,
                    &description_string,
                    &mut genotype1current,
                    &mut genotype2,
                    &mut refallele,
                    varallele,
                    start_position,
                    end_position,
                    shift3,
                    msi,
                    msint,
                    &mut total_pos_coverage,
                    reference_forward_coverage,
                    reference_reverse_coverage,
                    variations_at_pos,
                    ref_map,
                    region,
                    ref_coverage,
                    duprate,
                    conf,
                    debug_lines,
                    &mut positions_for_changed_ref_variant,
                    position,
                );
                continue;
            }

            // Branch: Deletion
            if description_string.starts_with('-') {
                let matcher_inv = INV_NUM.captures(&description_string);
                let matcher_start_minus_num =
                    BEGIN_MINUS_NUMBER_CARET.is_match(&description_string);

                if deletion_length < conf.svminlen {
                    // Trap T15: replaceFirst with regex
                    let re_leading = Regex::new(r"^-\d+").unwrap();
                    varallele = re_leading.replace(&description_string, "").to_string();
                    let (m, s, ms) =
                        proceed_vref_is_deletion(position, deletion_length, ref_map, region, conf);
                    msi = m;
                    shift3 = s;
                    msint = ms;

                    if matcher_inv.is_some() {
                        varallele = "<INV>".to_string();
                        genotype2 = format!("<INV{}>", deletion_length);
                    }
                } else if matcher_start_minus_num {
                    varallele = "<INV>".to_string();
                    genotype2 = format!("<INV{}>", deletion_length);
                } else {
                    varallele = "<DEL>".to_string();
                }
                let mut varallele = varallele;

                // Trap T35: deletion startPosition-- includes preceding base
                if !description_string.contains('&')
                    && !description_string.contains('#')
                    && !description_string.contains('^')
                {
                    if conf.move_indels_to_3 {
                        start_position += shift3;
                    }
                    if varallele != "<DEL>" {
                        varallele = ref_map
                            .get(&(position - 1))
                            .map(|b| char::from(*b).to_string())
                            .unwrap_or_default();
                    }
                    refallele = ref_map
                        .get(&(position - 1))
                        .map(|b| char::from(*b).to_string())
                        .unwrap_or_default();
                    start_position -= 1;
                }

                if let Some(caps) = SOME_SV_NUMBERS.captures(&description_string) {
                    let _ = caps;
                    refallele = ref_map
                        .get(&position)
                        .map(|b| char::from(*b).to_string())
                        .unwrap_or_default();
                } else if deletion_length < conf.svminlen {
                    refallele += &join_ref(ref_map, position, position + deletion_length - 1);
                }

                process_variant_finalization(
                    vi,
                    &description_string,
                    &mut genotype1current,
                    &mut genotype2,
                    &mut refallele,
                    varallele,
                    start_position,
                    end_position,
                    shift3,
                    msi,
                    msint,
                    &mut total_pos_coverage,
                    reference_forward_coverage,
                    reference_reverse_coverage,
                    variations_at_pos,
                    ref_map,
                    region,
                    ref_coverage,
                    duprate,
                    conf,
                    debug_lines,
                    &mut positions_for_changed_ref_variant,
                    position,
                );
                continue;
            }

            // Branch: SNP/MNP (neither insertion nor deletion)
            {
                let tseq1 = join_ref(ref_map, (position - REF_30_BASES).max(1), position + 1);
                let tseq2 = join_ref(ref_map, position + 2, (position + REF_70_BASES).min(chr0));
                let (m, s, ms) = find_msi(&tseq1, &tseq2, None);
                msi = m;
                shift3 = s;
                msint = ms;
            }

            refallele = ref_map
                .get(&position)
                .map(|b| char::from(*b).to_string())
                .unwrap_or_default();
            varallele = description_string.clone();

            process_variant_finalization(
                vi,
                &description_string,
                &mut genotype1current,
                &mut genotype2,
                &mut refallele,
                varallele,
                start_position,
                end_position,
                shift3,
                msi,
                msint,
                &mut total_pos_coverage,
                reference_forward_coverage,
                reference_reverse_coverage,
                variations_at_pos,
                ref_map,
                region,
                ref_coverage,
                duprate,
                conf,
                debug_lines,
                &mut positions_for_changed_ref_variant,
                position,
            );
        }

        // Trap T29: disableSV removal after full variant construction
        if conf.disable_sv {
            variations_at_pos
                .variants
                .retain(|vref| !ANY_SV.is_match(&vref.varallele));
        }
    } else if variations_at_pos.reference_variant.is_some() {
        // No non-reference variants; fill reference variant fields
        let vref = variations_at_pos.reference_variant.as_mut().unwrap();
        update_ref_variant(
            position,
            total_pos_coverage,
            vref,
            debug_lines,
            reference_forward_coverage,
            reference_reverse_coverage,
            ref_map,
            duprate,
            conf,
        );
    } else {
        // No variants at all — create empty Variant
        variations_at_pos.reference_variant = Some(Variant::default());
    }

    // Trap T30: Pileup ref variant double-update
    if variations_at_pos.reference_variant.is_some()
        && conf.do_pileup
        && (positions_for_changed_ref_variant.contains(&position)
            || scope.amplicon_based_calling.is_some())
    {
        let vref = variations_at_pos.reference_variant.as_mut().unwrap();
        update_ref_variant(
            position,
            total_pos_coverage,
            vref,
            debug_lines,
            reference_forward_coverage,
            reference_reverse_coverage,
            ref_map,
            duprate,
            conf,
        );
    }
}

/// Helper that handles the variant finalization steps common to insertion/deletion/SNP branches.
/// This corresponds to the per-variant post-branch code in collectReferenceVariants (steps 9bb-9fff).
///
/// Parity traps addressed: T15-19 (string/regex ops), T25 (totalPosCoverage mutation),
/// T31-32 (CRISPR), T33 (strandBiasFlag), T34 (genotype cleanup), T14 (msint.length())
#[allow(clippy::too_many_arguments)]
fn process_variant_finalization(
    vi: usize,
    description_string: &str,
    genotype1current: &mut String,
    genotype2: &mut String,
    refallele: &mut String,
    mut varallele: String,
    mut start_position: i32,
    mut end_position: i32,
    shift3: i32,
    msi: f64,
    msint: String,
    total_pos_coverage: &mut i32,
    reference_forward_coverage: i32,
    reference_reverse_coverage: i32,
    variations_at_pos: &mut Vars,
    ref_map: &HashMap<i32, u8>,
    region: &Region,
    ref_coverage: &HashMap<i32, i32>,
    _duprate: f64,
    conf: &Configuration,
    debug_lines: &[String],
    positions_for_changed_ref_variant: &mut Vec<i32>,
    position: i32,
) {
    let chr0 = chromosome_limit(region, ref_map);

    // Handle '&' (followed by matched sequence) — step 9bb-9cc
    // Trap T17: replaceFirst for '&' in varallele (first only)
    if let Some(caps) = AMP_ATGC.captures(description_string) {
        let extra = caps[1].to_string();
        varallele = varallele.replacen('&', "", 1); // Trap T17: only first '&'

        let tch = join_ref(ref_map, end_position + 1, end_position + extra.len() as i32);
        refallele.push_str(&tch);
        genotype1current.push_str(&tch);
        end_position += extra.len() as i32;

        // Nested '&' in varallele
        if let Some(caps2) = AMP_ATGC.captures(&varallele.clone()) {
            let vextra = caps2[1].to_string();
            varallele = varallele.replacen('&', "", 1);
            let tch2 = join_ref(
                ref_map,
                end_position + 1,
                end_position + vextra.len() as i32,
            );
            refallele.push_str(&tch2);
            genotype1current.push_str(&tch2);
            end_position += vextra.len() as i32;
        }

        // If description starts with '+', trim prefix
        if description_string.starts_with('+') {
            *refallele = refallele[1..].to_string();
            varallele = varallele[1..].to_string();
            start_position += 1;
        }

        // Trap T25: totalPosCoverage mutation in &+<DEL> branch
        if varallele == "<DEL>" && !refallele.is_empty() {
            *refallele = ref_map
                .get(&start_position)
                .map(|b| char::from(*b).to_string())
                .unwrap_or_default();
            if ref_coverage.contains_key(&(start_position - 1)) {
                *total_pos_coverage = *ref_coverage.get(&(start_position - 1)).unwrap();
            }
            if variations_at_pos.variants[vi].position_coverage > *total_pos_coverage {
                *total_pos_coverage = variations_at_pos.variants[vi].position_coverage;
            }
            variations_at_pos.variants[vi].frequency = variations_at_pos.variants[vi]
                .position_coverage as f64
                / *total_pos_coverage as f64;
        }
    }

    // Handle '#...^...' (matched sequence + indel tail) — step 9dd-9ee
    // Trap T18: replaceFirst for '#' and '^'
    if let Some(caps) = HASH_GROUP_CARET_GROUP.captures(description_string) {
        let matched_sequence = caps[1].to_string();
        let tail = caps[2].to_string();

        end_position += matched_sequence.len() as i32;
        *refallele += &join_ref(
            ref_map,
            end_position - matched_sequence.len() as i32 + 1,
            end_position,
        );

        // If tail starts with digits, it's a deletion length
        if let Some(dcaps) = BEGIN_DIGITS.captures(&tail) {
            let deletion: i32 = dcaps[1].parse().unwrap_or(0);
            *refallele += &join_ref(ref_map, end_position + 1, end_position + deletion);
            end_position += deletion;
        }

        // Trap T18: clean special symbols from varallele
        let re_hash = Regex::new(r"#").unwrap();
        varallele = re_hash.replacen(&varallele, 1, "").to_string();
        let re_caret_num = Regex::new(r"\^(\d+)?").unwrap();
        varallele = re_caret_num.replacen(&varallele, 1, "").to_string();

        // Trap T19: replace '#' with 'm' and '^' with 'i' in genotypes
        let re_hash2 = Regex::new(r"#").unwrap();
        let re_caret = Regex::new(r"\^").unwrap();
        *genotype1current = re_hash2.replacen(genotype1current, 1, "m").to_string();
        *genotype1current = re_caret.replacen(genotype1current, 1, "i").to_string();
        *genotype2 = re_hash2.replacen(genotype2, 1, "m").to_string();
        *genotype2 = re_caret.replacen(genotype2, 1, "i").to_string();
    }

    // Handle '^' (insertion after deletion) — step 9ff-9gg
    if CARET_ATGNC.is_match(description_string) {
        let re_caret = Regex::new(r"\^").unwrap();
        varallele = re_caret.replacen(&varallele, 1, "").to_string();
        *genotype1current = re_caret.replacen(genotype1current, 1, "i").to_string();
        *genotype2 = re_caret.replacen(genotype2, 1, "i").to_string();
    }

    // CRISPR adjustment — steps 9hh-9jj
    // Trap T31-32: CRISPR prefix trimming and shifting
    let cut_site = conf.crispr_cutting_site;
    if cut_site != 0 && refallele.len() > 1 && varallele.len() > 1 {
        // 5' fix for complex variants
        let mut n = 0;
        let refallele_bytes = refallele.as_bytes();
        let varallele_bytes = varallele.as_bytes();
        while refallele_bytes.len() > n + 1
            && varallele_bytes.len() > n + 1
            && refallele_bytes[n] == varallele_bytes[n]
        {
            n += 1;
        }
        if n != 0 {
            start_position += n as i32;
            *refallele = refallele[n..].to_string();
            varallele = varallele[n..].to_string();
        }
    }

    if cut_site != 0
        && refallele.len() != varallele.len()
        && !refallele.is_empty()
        && !varallele.is_empty()
        && refallele.as_bytes()[0] == varallele.as_bytes()[0]
    {
        if !(start_position == cut_site || end_position == cut_site) {
            let mut n: i32 = 0;
            let dis = (cut_site - start_position)
                .abs()
                .min((cut_site - end_position).abs());
            if start_position < cut_site {
                while start_position + n < cut_site && n < shift3 && end_position + n != cut_site {
                    n += 1;
                }
                if (start_position + n - cut_site).abs() > dis
                    && (end_position + n - cut_site).abs() > dis
                {
                    n = 0;
                }
            }
            if end_position < cut_site && n == 0 {
                if (end_position - cut_site).abs() <= (start_position - cut_site).abs() {
                    while end_position + n < cut_site && n < shift3 {
                        n += 1;
                    }
                }
            }
            if n > 0 {
                start_position += n;
                end_position += n;
                let mut new_refallele = String::new();
                for i in start_position..=end_position {
                    if let Some(b) = ref_map.get(&i) {
                        new_refallele.push(char::from(*b));
                    }
                }
                *refallele = new_refallele;

                let mut tva = String::new();
                if refallele.len() < varallele.len() {
                    // Insertion
                    tva = varallele[1..].to_string();
                    if tva.len() > 1 {
                        let ttn = n as usize % tva.len();
                        if ttn != 0 {
                            tva = format!("{}{}", &tva[ttn..], &tva[..ttn]);
                        }
                    }
                }
                varallele = format!(
                    "{}{}",
                    ref_map
                        .get(&start_position)
                        .map(|b| char::from(*b))
                        .unwrap_or(' '),
                    tva
                );
            }
            variations_at_pos.variants[vi].crispr = n;
        }
    }

    // Set flanking sequences
    variations_at_pos.variants[vi].leftseq = join_ref(
        ref_map,
        (start_position - REF_20_BASES).max(1),
        start_position - 1,
    );
    variations_at_pos.variants[vi].rightseq = join_ref(
        ref_map,
        end_position + 1,
        (end_position + REF_20_BASES).min(chr0),
    );

    // Build genotype string — Trap T34
    let mut genotype = format!("{}/{}", genotype1current, genotype2);
    genotype = genotype.replace('&', "");
    genotype = genotype.replace('#', "");
    genotype = genotype.replace('^', "i");

    // Round and set final fields
    // Trap T14: msint field = length of MSI unit string
    let vref = &mut variations_at_pos.variants[vi];
    vref.extra_frequency = round_half_even("0.0000", vref.extra_frequency);
    vref.frequency = round_half_even("0.0000", vref.frequency);
    vref.high_quality_reads_frequency =
        round_half_even("0.0000", vref.high_quality_reads_frequency);
    vref.msi = round_half_even("0.000", msi);
    vref.msint = msint.len() as i32;
    vref.shift3 = shift3;
    vref.start_position = start_position;
    vref.end_position = end_position;
    vref.refallele = validate_refallele(refallele);
    vref.varallele = varallele;
    vref.genotype = Some(genotype);
    vref.total_pos_coverage = *total_pos_coverage;
    vref.ref_forward_coverage = reference_forward_coverage;
    vref.ref_reverse_coverage = reference_reverse_coverage;

    // Trap T33: strandBiasFlag format "refBias;varBias"
    if let Some(ref rv) = variations_at_pos.reference_variant {
        vref.strand_bias_flag = format!("{};{}", rv.strand_bias_flag, vref.strand_bias_flag);
    } else {
        vref.strand_bias_flag = format!("0;{}", vref.strand_bias_flag);
    }

    adjust_variant_counts(position, vref);

    if start_position != position && conf.do_pileup {
        positions_for_changed_ref_variant.push(position);
    }

    construct_debug_lines(debug_lines, vref, conf);
}

/// Ported from: ToVarsBuilder.java:L88-L190
/// Main entry point: loops positions, builds variants, outputs AlignedVarsData.
///
/// Parity traps addressed: T26 (SV bypass), T28 (error-and-continue per position)
pub fn process(
    max_read_length: i32,
    region: &Region,
    ref_map: &HashMap<i32, u8>,
    ref_coverage: &HashMap<i32, i32>,
    insertion_variants: &HashMap<i32, VariationMap>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    duprate: f64,
) -> AlignedVarsData {
    let conf = &GlobalReadOnlyScope::instance().conf;
    let scope = GlobalReadOnlyScope::instance();

    if conf.y {
        eprintln!(
            "Current segment: {}:{}-{} ",
            region.chr, region.start, region.end
        );
    }

    let mut aligned_variants: HashMap<i32, Vars> = HashMap::new();

    // Collect and sort positions for deterministic iteration
    // Java uses HashMap for outer map — iteration order is JVM-dependent.
    // Sort ascending so position+1 mutation from createInsertion is processed correctly.
    let mut positions: Vec<i32> = non_insertion_variants.keys().copied().collect();
    positions.sort();

    for &position in &positions {
        // Trap T28: error-and-continue — wrap in closure that can handle errors
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_position(
                position,
                &mut aligned_variants,
                ref_map,
                ref_coverage,
                insertion_variants,
                non_insertion_variants,
                region,
                duprate,
                conf,
                &scope,
            )
        }));

        if let Err(e) = result {
            eprintln!(
                "Error processing position {} in {}:{}-{}: {:?}",
                position, region.chr, region.start, region.end, e
            );
        }
    }

    if conf.y {
        eprintln!("TIME: Finish preparing vars");
    }

    AlignedVarsData {
        max_read_length,
        aligned_variants,
    }
}

/// Process a single position in the main loop.
#[allow(clippy::too_many_arguments)]
fn process_position(
    position: i32,
    aligned_variants: &mut HashMap<i32, Vars>,
    ref_map: &HashMap<i32, u8>,
    ref_coverage: &HashMap<i32, i32>,
    insertion_variants: &HashMap<i32, VariationMap>,
    non_insertion_variants: &mut HashMap<i32, VariationMap>,
    region: &Region,
    duprate: f64,
    conf: &Configuration,
    scope: &GlobalReadOnlyScope,
) {
    let vars_at_cur_position = match non_insertion_variants.get(&position) {
        Some(vm) => vm.clone(), // Clone to release borrow for later mut access
        None => return,
    };

    // Skip if empty and no insertions
    if vars_at_cur_position.entries.is_empty() && !insertion_variants.contains_key(&position) {
        return;
    }

    // Trap T26: SV positions bypass region boundary check
    if vars_at_cur_position.sv.is_none() || conf.delete_duplicate_variants {
        if position < region.start || position > region.end {
            return;
        }
    }

    // Skip if no SV and no coverage
    if vars_at_cur_position.sv.is_none() && !ref_coverage.contains_key(&position) {
        return;
    }

    // Check if only reference variant
    if is_the_same_variation_on_ref(
        position,
        &vars_at_cur_position,
        insertion_variants,
        ref_map,
        conf,
    ) {
        return;
    }

    // Check coverage
    if !ref_coverage.contains_key(&position) || *ref_coverage.get(&position).unwrap() == 0 {
        let sv_type = vars_at_cur_position
            .sv
            .as_ref()
            .and_then(|sv| sv.type_.as_deref())
            .unwrap_or("null");
        eprintln!(
            "Error tcov: {} {} {} {} {}",
            region.chr, position, region.start, region.end, sv_type
        );
        return;
    }

    let mut total_pos_coverage = *ref_coverage.get(&position).unwrap();
    let hicov = calc_hicov(insertion_variants.get(&position), &vars_at_cur_position);

    let mut var: Vec<Variant> = Vec::new();
    let mut keys: Vec<String> = vars_at_cur_position.entries.keys().cloned().collect();
    keys.sort(); // Trap T4,T5: lexicographic sort

    let mut debug_lines: Vec<String> = Vec::new();

    create_variant(
        duprate,
        aligned_variants,
        position,
        &vars_at_cur_position,
        total_pos_coverage,
        &mut var,
        &mut debug_lines,
        &keys,
        hicov,
        conf,
    );

    // Trap T25: totalPosCoverage updated by createInsertion
    total_pos_coverage = create_insertion(
        duprate,
        position,
        total_pos_coverage,
        &mut var,
        &mut debug_lines,
        hicov,
        insertion_variants,
        non_insertion_variants,
        ref_coverage,
        ref_map,
        conf,
    );

    sort_variants(&mut var);

    let maxfreq = collect_vars_at_position(aligned_variants, position, &var, ref_map);

    // Frequency gate
    if !conf.do_pileup && maxfreq <= conf.freq && scope.amplicon_based_calling.is_none() {
        if !conf.bam.as_ref().map_or(false, |b| b.has_bam2()) {
            aligned_variants.remove(&position);
            return;
        }
    }

    let variations_at_pos = get_or_put_vars(aligned_variants, position);

    collect_reference_variants(
        position,
        total_pos_coverage,
        variations_at_pos,
        &debug_lines,
        ref_map,
        region,
        ref_coverage,
        non_insertion_variants,
        duprate,
        conf,
    );

    variations_at_pos.var_description_string_to_variants = variations_at_pos
        .variants
        .iter()
        .cloned()
        .map(|variant| (variant.description_string.clone(), variant))
        .collect();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Variation;

    #[test]
    fn test_find_msi_simple_repeat() {
        let (msi, shift3, msint) = find_msi("ATATAT", "ATATGC", None);
        assert_eq!(msi, 5.0);
        assert_eq!(shift3, 4);
        assert_eq!(msint, "AT");
    }

    #[test]
    fn test_find_msi_empty_tseq1() {
        let (msi, shift3, _msint) = find_msi("", "ATGCATGC", None);
        assert_eq!(msi, 0.0);
        // shift3 depends on comparison of "" + "ATGCATGC" with "ATGCATGC"
        assert_eq!(shift3, 8); // all chars match since tseq = "" + tseq2 = tseq2
    }

    #[test]
    fn test_find_msi_with_left_context() {
        let (msi, shift3, msint) = find_msi("AT", "ATATAT", Some("ATATAT"));
        assert_eq!(msi, 7.0);
        assert_eq!(shift3, 6);
        assert_eq!(msint, "AT");
    }

    #[test]
    fn test_find_msi_mono_repeat() {
        let (msi, _shift3, msint) = find_msi("AAAA", "AAAACC", None);
        assert!(msi >= 4.0, "msi should detect mono-A repeat");
        assert_eq!(msint, "A");
    }

    #[test]
    fn test_repeated_motif_lengths_match_greedy_regex_cases() {
        assert_eq!(repeated_motif_suffix_len("CCATATAT", "AT"), 6);
        assert_eq!(repeated_motif_suffix_len("CCATATA", "AT"), 0);
        assert_eq!(repeated_motif_prefix_len("ATATGC", "AT"), 4);
        assert_eq!(repeated_motif_prefix_len("TATATG", "AT"), 0);
    }

    #[test]
    fn test_validate_refallele_no_iupac() {
        assert_eq!(validate_refallele("ATGC"), "ATGC");
    }

    #[test]
    fn test_validate_refallele_with_iupac() {
        assert_eq!(validate_refallele("M"), "A"); // M → A
        assert_eq!(validate_refallele("R"), "A"); // R → A
        assert_eq!(validate_refallele("S"), "C"); // S → C
        assert_eq!(validate_refallele("K"), "G"); // K → G
        assert_eq!(validate_refallele("AMSC"), "AACC"); // Multiple
    }

    #[test]
    fn test_validate_refallele_empty() {
        assert_eq!(validate_refallele(""), "");
    }

    #[test]
    fn test_sort_variants_by_score_desc() {
        let mut vars = vec![
            {
                let mut v = Variant::default();
                v.description_string = "A".to_string();
                v.mean_quality = 10.0;
                v.position_coverage = 5;
                v
            },
            {
                let mut v = Variant::default();
                v.description_string = "T".to_string();
                v.mean_quality = 20.0;
                v.position_coverage = 10;
                v
            },
            {
                let mut v = Variant::default();
                v.description_string = "C".to_string();
                v.mean_quality = 15.0;
                v.position_coverage = 5;
                v
            },
        ];
        sort_variants(&mut vars);
        // T: 20*10=200, C: 15*5=75, A: 10*5=50
        assert_eq!(vars[0].description_string, "T");
        assert_eq!(vars[1].description_string, "C");
        assert_eq!(vars[2].description_string, "A");
    }

    #[test]
    fn test_sort_variants_tiebreak_by_desc_asc() {
        let mut vars = vec![
            {
                let mut v = Variant::default();
                v.description_string = "T".to_string();
                v.mean_quality = 10.0;
                v.position_coverage = 5;
                v
            },
            {
                let mut v = Variant::default();
                v.description_string = "A".to_string();
                v.mean_quality = 10.0;
                v.position_coverage = 5;
                v
            },
        ];
        sort_variants(&mut vars);
        // Same score → ascending by description string
        assert_eq!(vars[0].description_string, "A");
        assert_eq!(vars[1].description_string, "T");
    }

    #[test]
    fn test_calc_hicov_basic() {
        let mut vm = VariationMap::default();
        vm.entries.insert("A".to_string(), {
            let mut v = Variation::default();
            v.high_quality_reads_count = 10;
            v
        });
        vm.entries.insert("T".to_string(), {
            let mut v = Variation::default();
            v.high_quality_reads_count = 5;
            v
        });
        assert_eq!(calc_hicov(None, &vm), 15);
    }

    #[test]
    fn test_calc_hicov_skips_sv_and_insertion() {
        let mut vm = VariationMap::default();
        vm.entries.insert("SV".to_string(), {
            let mut v = Variation::default();
            v.high_quality_reads_count = 100;
            v
        });
        vm.entries.insert("+AT".to_string(), {
            let mut v = Variation::default();
            v.high_quality_reads_count = 50;
            v
        });
        vm.entries.insert("A".to_string(), {
            let mut v = Variation::default();
            v.high_quality_reads_count = 7;
            v
        });
        assert_eq!(calc_hicov(None, &vm), 7);
    }
}
