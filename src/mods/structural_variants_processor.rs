//! Ported from: StructuralVariantsProcessor.java
//!
//! S17 — Cluster A: Utility functions (findMatch, findMatchRev, isOverlap,
//! checkPairs, markSV, markDUPSV, fillAndSortTmpSV, PairsData).
//! Cluster B: SV finding routines (findDEL, findINV, findsv, findDELdisc, findINVdisc, findDUPdisc).
//! Cluster C: adjSNV, outputClipping.
//! Cluster D: process() entry point.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::config::{DISCPAIRQUAL, MINSVCDIST, SEED_1, SEED_2, SVFLANK};
use crate::data::{
    CoverageMap, CurrentSegment, InitialData, Match, PositionMap, RealignedVariationData, Region,
    SVStructures, Sclip, Side, SortPositionSclip, Variation, VariationMap, VariationMapSV,
};
use crate::java_hashmap_order::java_hashmap_i32_order_from_keys;
use crate::reference::{Reference, ReferenceResource, ReferenceSequenceMap};
use crate::scope::{GlobalReadOnlyScope, Scope, VariantPrinter};
use crate::utils::{
    char_at, complement_base, complement_sequence, get_reverse_complemented_sequence,
    reverse_sequence, substr_with_len,
};
use crate::variations::{
    adj_cnt, adj_cnt_with_reference, find_conseq, get_variation, inc_cnt, is_has_and_equals_base,
    is_has_and_equals_index, is_has_and_not_equals_base, is_not_equals, join_ref,
};

use super::variation_realigner::ismatchref_with_mm;

// ─── PairsData ──────────────────────────────────────────────────────

/// Java: StructuralVariantsProcessor.PairsData (inner class)
/// Java: StructuralVariantsProcessor.java#L2077-L2110
#[derive(Clone, Debug)]
pub struct PairsData {
    pub pairs: i32,
    pub pmean: f64,
    pub qmean: f64,
    pub q_mean: f64, // Java: Qmean (mean mapping quality)
    pub nm: f64,
}

impl PairsData {
    pub fn new(pairs: i32, pmean: f64, qmean: f64, q_mean: f64, nm: f64) -> Self {
        // Java: StructuralVariantsProcessor.java#L2099-L2106
        Self {
            pairs,
            pmean,
            qmean,
            q_mean,
            nm,
        }
    }
}

// ─── findMatch (forward, 6-arg static) ──────────────────────────────

/// Ported from: StructuralVariantsProcessor.findMatch(String, Reference, int, int, int, int)
/// Java: StructuralVariantsProcessor.java#L1910-L2072
///
/// Core forward-strand seed-based alignment.
pub fn find_match(
    seq: &str,
    reference: &Reference,
    _position: i32,
    dir: i32,
    seed_len: i32,
    mm: i32,
) -> Match {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L1911-L1913
    // dir == -1 means 5' clip → reverse the sequence
    let seq_owned: String = if dir == -1 {
        let rev = reverse_sequence(seq.as_bytes());
        String::from_utf8_lossy(&rev).to_string()
    } else {
        seq.to_string()
    };

    if instance.conf.y {
        eprintln!(
            "    Working Match {} {} {} SEED: {}",
            _position, seq_owned, dir, seed_len
        );
    }

    let seq_bytes = seq_owned.as_bytes();
    let mut extra = String::new();
    let reference_sequences = &reference.reference_sequences;

    // Java: StructuralVariantsProcessor.java#L1919-L2067
    let start_i = (seq_bytes.len() as i32) - seed_len;
    for i in (0..=start_i).rev() {
        let seed_start = i as usize;
        let seed_end = (i + seed_len) as usize;
        if seed_end > seq_bytes.len() {
            continue;
        }
        let seed = &seq_owned[seed_start..seed_end];

        // Java: StructuralVariantsProcessor.java#L1922
        if let Some(seeds) = reference.seed.get(seed) {
            // Java: StructuralVariantsProcessor.java#L1924 — seeds must have exactly 1 occurrence
            if seeds.len() == 1 {
                let first_seed = seeds[0];

                // Java: StructuralVariantsProcessor.java#L1926
                let mut bp: i32 = if dir == 1 {
                    first_seed - i
                } else {
                    first_seed + (seq_bytes.len() as i32) - i - 1
                };

                // Java: StructuralVariantsProcessor.java#L1928 — primary match
                if ismatchref_with_mm(&seq_owned, reference_sequences, bp, dir, mm) {
                    // Java: StructuralVariantsProcessor.java#L1929-L1937
                    // Extra extraction: walk backward collecting mismatched boundary bases
                    let mut mm_idx: i32 = if dir == -1 { -1 } else { 0 };
                    loop {
                        let ch = char_at(seq_bytes, mm_idx);
                        match ch {
                            Some(ch_val) => {
                                if !is_has_and_not_equals_base(ch_val, reference_sequences, bp) {
                                    break;
                                }
                                // Java: extra += substr(seq, mm, 1)
                                extra.push(ch_val as char);
                                bp += dir;
                                mm_idx += dir;
                            }
                            None => break,
                        }
                    }
                    // Java: StructuralVariantsProcessor.java#L1935-L1936
                    if !extra.is_empty() && dir == -1 {
                        let rev = reverse_sequence(extra.as_bytes());
                        extra = String::from_utf8_lossy(&rev).to_string();
                    }
                    if instance.conf.y {
                        eprintln!(
                            "      Found SV BP: {} BP: {} SEEDpos {} {} {} {} {} extra: {}",
                            dir, bp, first_seed, _position, seed, i, seq_owned, extra
                        );
                    }
                    return Match::new(bp, extra);
                } else {
                    // Java: StructuralVariantsProcessor.java#L1944-L2065
                    // Complex indel fallback — up to 15bp trimming
                    let mut sseq = seq_owned.clone();
                    let mut eqcnt = 0i32;
                    for ii in 1..=15 {
                        bp += dir;
                        // Java: StructuralVariantsProcessor.java#L1948-L1949
                        sseq = if dir == 1 {
                            // substr(sseq, 1) — trim first char
                            if sseq.len() > 1 {
                                sseq[1..].to_string()
                            } else {
                                String::new()
                            }
                        } else {
                            // substr(sseq, 0, -1) — trim last char
                            if sseq.len() > 1 {
                                sseq[..sseq.len() - 1].to_string()
                            } else {
                                String::new()
                            }
                        };
                        if sseq.is_empty() {
                            break;
                        }

                        let sseq_bytes = sseq.as_bytes();
                        if dir == 1 {
                            // Java: StructuralVariantsProcessor.java#L1950-L1958
                            if let Some(ch0) = char_at(sseq_bytes, 0) {
                                if is_has_and_not_equals_base(ch0, reference_sequences, bp) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            eqcnt += 1;
                            if let Some(ch1) = char_at(sseq_bytes, 1) {
                                if is_has_and_not_equals_base(ch1, reference_sequences, bp + 1) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            // Java: extra = substr(seq, 0, ii)
                            let e_end = std::cmp::min(ii as usize, seq_bytes.len());
                            extra = String::from_utf8_lossy(&seq_bytes[..e_end]).to_string();
                        } else {
                            // dir == -1
                            // Java: StructuralVariantsProcessor.java#L1960-L1968
                            if let Some(ch_last) = char_at(sseq_bytes, -1) {
                                if is_has_and_not_equals_base(ch_last, reference_sequences, bp) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            eqcnt += 1;
                            if let Some(ch_second_last) = char_at(sseq_bytes, -2) {
                                if is_has_and_not_equals_base(
                                    ch_second_last,
                                    reference_sequences,
                                    bp - 1,
                                ) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            // Java: extra = substr(seq, -ii)
                            let start = if (ii as usize) >= seq_bytes.len() {
                                0
                            } else {
                                seq_bytes.len() - ii as usize
                            };
                            extra = String::from_utf8_lossy(&seq_bytes[start..]).to_string();
                        }
                        // Java: StructuralVariantsProcessor.java#L1970
                        if eqcnt >= 3 && (eqcnt as f64) / (ii as f64) > 0.5 {
                            break;
                        }
                        if instance.conf.y {
                            eprintln!(
                                "      FoundSEED SV BP: {} BP: {} SEEDpos{} {} {} {} {} EXTRA: {}",
                                dir, bp, first_seed, _position, seed, i, seq_owned, extra
                            );
                        }
                        // Java: StructuralVariantsProcessor.java#L1976
                        if ismatchref_with_mm(&sseq, reference_sequences, bp, dir, 1) {
                            return Match::new(bp, extra);
                        }
                    }
                }
            }
        }
    }
    // Java: StructuralVariantsProcessor.java#L2071
    Match::new(0, String::new())
}

/// Ported from: StructuralVariantsProcessor.findMatch(String, Reference, int, int)
/// Java: StructuralVariantsProcessor.java#L1894-L1897
///
/// Convenience wrapper: forward-strand matching with default MM=3.
pub fn find_match_default(seq: &str, reference: &Reference, position: i32, dir: i32) -> Match {
    // Java: StructuralVariantsProcessor.java#L1895-L1896
    find_match(seq, reference, position, dir, SEED_1, 3)
}

// ─── findMatchRev (reverse, 6-arg) ─────────────────────────────────

/// Ported from: StructuralVariantsProcessor.findMatchRev(String, Reference, int, int, int, int)
/// Java: StructuralVariantsProcessor.java#L1815-L1889
///
/// Core reverse-strand seed-based alignment.
/// Note: complement only, NOT reverseComplement. Reverse is done separately when dir==1.
pub fn find_match_rev(
    seq: &str,
    reference: &Reference,
    _position: i32,
    dir: i32,
    seed_len: i32,
    mm: i32,
) -> Match {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L1817-L1819
    // dir == 1 means from 3' soft clip — reverse first
    let mut seq_owned: String = if dir == 1 {
        let rev = reverse_sequence(seq.as_bytes());
        String::from_utf8_lossy(&rev).to_string()
    } else {
        seq.to_string()
    };
    // Java: StructuralVariantsProcessor.java#L1821
    // complement (NOT reverseComplement)
    let comp = complement_sequence(seq_owned.as_bytes());
    seq_owned = String::from_utf8_lossy(&comp).to_string();

    if instance.conf.y {
        eprintln!("    Working MatchRev {} {} {}", _position, seq_owned, dir);
    }

    let seq_bytes = seq_owned.as_bytes();
    let reference_sequences = &reference.reference_sequences;
    let mut extra = String::new();

    // Java: StructuralVariantsProcessor.java#L1826-L1886
    let start_i = (seq_bytes.len() as i32) - seed_len;
    for i in (0..=start_i).rev() {
        let seed_start = i as usize;
        let seed_end = (i + seed_len) as usize;
        if seed_end > seq_bytes.len() {
            continue;
        }
        let seed = &seq_owned[seed_start..seed_end];

        // Java: StructuralVariantsProcessor.java#L1829
        if let Some(seeds) = reference.seed.get(seed) {
            if seeds.len() == 1 {
                let first_seed = seeds[0];

                // Java: StructuralVariantsProcessor.java#L1833
                let mut bp: i32 = if dir == 1 {
                    first_seed + (seq_bytes.len() as i32) - i - 1
                } else {
                    first_seed - i
                };

                // Java: StructuralVariantsProcessor.java#L1834 — primary match with -1*dir
                if ismatchref_with_mm(&seq_owned, reference_sequences, bp, -1 * dir, mm) {
                    if instance.conf.y {
                        eprintln!(
                            "      Found SV BP (reverse): {} BP: {} SEEDpos: {} {} {} {} {}",
                            dir, bp, first_seed, _position, seed, i, seq_owned
                        );
                    }
                    return Match::new(bp, extra);
                } else {
                    // Java: StructuralVariantsProcessor.java#L1844-L1884
                    // Complex indel fallback
                    let mut sseq = seq_owned.clone();
                    let mut eqcnt = 0i32;
                    for j in 1..=15 {
                        bp -= dir;
                        sseq = if dir == -1 {
                            // Java: substr(sseq, 1)
                            if sseq.len() > 1 {
                                sseq[1..].to_string()
                            } else {
                                String::new()
                            }
                        } else {
                            // Java: substr(sseq, 0, -1)
                            if sseq.len() > 1 {
                                sseq[..sseq.len() - 1].to_string()
                            } else {
                                String::new()
                            }
                        };
                        if sseq.is_empty() {
                            break;
                        }

                        let sseq_bytes = sseq.as_bytes();
                        if dir == -1 {
                            // Java: StructuralVariantsProcessor.java#L1849-L1857
                            if let Some(ch0) = char_at(sseq_bytes, 0) {
                                if is_has_and_not_equals_base(ch0, reference_sequences, bp) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            eqcnt += 1;
                            if let Some(ch1) = char_at(sseq_bytes, 1) {
                                if is_has_and_not_equals_base(ch1, reference_sequences, bp + 1) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            // Java: extra = substr(seq, 0, j)
                            // Note: seq here is the complemented seq_owned
                            let e_end = std::cmp::min(j as usize, seq_bytes.len());
                            extra = String::from_utf8_lossy(&seq_bytes[..e_end]).to_string();
                        } else {
                            // dir == 1
                            // Java: StructuralVariantsProcessor.java#L1861-L1869
                            if let Some(ch_last) = char_at(sseq_bytes, -1) {
                                if is_has_and_not_equals_base(ch_last, reference_sequences, bp) {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            eqcnt += 1;
                            if let Some(ch_last2) = char_at(sseq_bytes, -2) {
                                if is_has_and_not_equals_base(ch_last2, reference_sequences, bp - 1)
                                {
                                    continue;
                                }
                            } else {
                                continue;
                            }
                            // Java: extra = substr(seq, -j)
                            let start = if (j as usize) >= seq_bytes.len() {
                                0
                            } else {
                                seq_bytes.len() - j as usize
                            };
                            extra = String::from_utf8_lossy(&seq_bytes[start..]).to_string();
                        }
                        // Java: StructuralVariantsProcessor.java#L1871
                        if eqcnt >= 3 && (eqcnt as f64) / (j as f64) > 0.5 {
                            break;
                        }
                        if instance.conf.y {
                            eprintln!(
                                "      FoundSEED SV BP (reverse): {} BP: {} SEEDpos{} {} {} {} {} EXTRA: {}",
                                dir, bp, first_seed, _position, seed, i, seq_owned, extra
                            );
                        }
                        // Java: StructuralVariantsProcessor.java#L1878
                        if ismatchref_with_mm(&sseq, reference_sequences, bp, -1 * dir, 1) {
                            return Match::new(bp, extra);
                        }
                    }
                }
            }
        }
    }
    // Java: StructuralVariantsProcessor.java#L1888
    Match::new(0, String::new())
}

/// Ported from: StructuralVariantsProcessor.findMatchRev(String, Reference, int, int)
/// Java: StructuralVariantsProcessor.java#L1799-L1802
///
/// Convenience wrapper: reverse-strand matching with default MM=3.
pub fn find_match_rev_default(seq: &str, reference: &Reference, position: i32, dir: i32) -> Match {
    // Java: StructuralVariantsProcessor.java#L1800-L1801
    find_match_rev(seq, reference, position, dir, SEED_1, 3)
}

// ─── isOverlap ──────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.isOverlap(int, int, int, int, int)
/// Java: StructuralVariantsProcessor.java#L1721-L1745
///
/// Determine if two SV intervals overlap based on geometric criteria.
pub fn is_overlap(start1: i32, end1: i32, start2: i32, end2: i32, rlen: i32) -> bool {
    // Java: StructuralVariantsProcessor.java#L1727
    if start1 >= end2 || start2 >= end1 {
        return false;
    }

    // Java: StructuralVariantsProcessor.java#L1730-L1731
    let mut positions = vec![start1, end1, start2, end2];
    positions.sort();

    // Java: StructuralVariantsProcessor.java#L1733
    let ins = positions[2] - positions[1];

    // Java: StructuralVariantsProcessor.java#L1734-L1737
    if (end1 != start1)
        && (end2 != start2)
        && (ins as f64) / ((end1 - start1) as f64) > 0.75
        && (ins as f64) / ((end2 - start2) as f64) > 0.75
    {
        return true;
    }

    // Java: StructuralVariantsProcessor.java#L1739-L1741
    if positions[1] - positions[0] + positions[3] - positions[2] < 3 * rlen {
        return true;
    }

    false
}

// ─── checkPairs ─────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.checkPairs(String, int, int, List<List<Sclip>>, int)
/// Java: StructuralVariantsProcessor.java#L1752-L1790
///
/// Check if discordant pair clusters support a candidate SV.
/// Takes the cluster with highest varsCount as representative, but marks ALL overlapping clusters as used.
pub fn check_pairs(
    chr: &str,
    start: i32,
    end: i32,
    sv_lists: &mut [&mut Vec<Sclip>],
    max_read_length: i32,
) -> PairsData {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L1753-L1759
    let mut pairs = 0i32;
    let mut pairs_data = PairsData::new(0, 0.0, 0.0, 0.0, 0.0);

    // Java: StructuralVariantsProcessor.java#L1761-L1788
    for sv_cluster in sv_lists.iter_mut() {
        for svr in sv_cluster.iter_mut() {
            if svr.used {
                continue;
            }
            // Java: StructuralVariantsProcessor.java#L1766-L1770
            let mut s = (svr.start + svr.end) / 2; // integer division
            let mut e = (svr.mstart + svr.mend) / 2; // integer division
            if s > e {
                std::mem::swap(&mut s, &mut e);
            }

            // Java: StructuralVariantsProcessor.java#L1773
            if !is_overlap(start, end, s, e, max_read_length) {
                continue;
            }

            // Java: StructuralVariantsProcessor.java#L1776-L1778
            if svr.base.vars_count > pairs {
                pairs_data = PairsData::new(
                    svr.base.vars_count,
                    svr.base.mean_position,
                    svr.base.mean_quality,
                    svr.base.mean_mapping_quality,
                    svr.base.number_of_mismatches,
                );
                pairs = svr.base.vars_count;
            }
            // Java: StructuralVariantsProcessor.java#L1781
            svr.used = true;

            if instance.conf.y {
                eprintln!(
                    "      Pair [{}:{}-{}] overlapping [{}:{}-{}] found and marked.",
                    chr, s, e, chr, start, end
                );
            }
        }
    }
    pairs_data
}

// ─── markSV ─────────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.markSV(int, int, List<List<Sclip>>, int)
/// Java: StructuralVariantsProcessor.java#L1628-L1668
///
/// Mark overlapping SV clusters as used (static).
/// Uses INNER coordinates: end/mstart (when start < mstart) or mend/start (otherwise).
/// Returns (cov, cnt, pairs).
pub fn mark_sv(
    start: i32,
    end: i32,
    sv_lists: &mut [&mut Vec<Sclip>],
    rlen: i32,
) -> (i32, i32, i32) {
    let instance = GlobalReadOnlyScope::instance();

    let mut cov = 0i32;
    let mut pairs = 0i32;
    let mut cnt = 0i32;

    // Java: StructuralVariantsProcessor.java#L1637-L1665
    for current_sclips in sv_lists.iter_mut() {
        for sv_r in current_sclips.iter_mut() {
            // Java: StructuralVariantsProcessor.java#L1641-L1647
            let (start2, end2) = if sv_r.start < sv_r.mstart {
                (sv_r.end, sv_r.mstart) // inner coordinates
            } else {
                (sv_r.mend, sv_r.start)
            };

            if instance.conf.y {
                eprintln!(
                    "   Marking SV {} {} {} {} cnt: {}",
                    start, end, start2, end2, sv_r.base.vars_count
                );
            }

            // Java: StructuralVariantsProcessor.java#L1650
            if is_overlap(start, end, start2, end2, rlen) {
                if instance.conf.y {
                    eprintln!(
                        "       SV {} {} {} {} cnt: {} marked",
                        start, end, start2, end2, sv_r.base.vars_count
                    );
                }
                // Java: StructuralVariantsProcessor.java#L1655-L1662
                sv_r.used = true;
                cnt += 1;
                pairs += sv_r.base.vars_count;
                if sv_r.end != sv_r.start {
                    cov += ((sv_r.base.vars_count as i64 * rlen as i64)
                        / (sv_r.end - sv_r.start) as i64) as i32
                        + 1;
                }
            }
        }
    }
    (cov, cnt, pairs)
}

// ─── markDUPSV ──────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.markDUPSV(int, int, List<List<Sclip>>, int)
/// Java: StructuralVariantsProcessor.java#L1674-L1715
///
/// Mark overlapping DUP SV clusters as used (static).
/// Uses OUTER coordinates: start/mend (when start < mstart) or mstart/end (otherwise).
/// Returns (cnt, pairs) — different from markSV which returns (cov, cnt, pairs).
pub fn mark_dup_sv(
    start: i32,
    end: i32,
    sv_lists: &mut [&mut Vec<Sclip>],
    rlen: i32,
) -> (i32, i32) {
    let instance = GlobalReadOnlyScope::instance();

    let mut _cov = 0i32; // computed but not returned (Java returns Tuple2<cnt, pairs>)
    let mut pairs = 0i32;
    let mut cnt = 0i32;

    // Java: StructuralVariantsProcessor.java#L1685-L1712
    for current_sclips in sv_lists.iter_mut() {
        for sv_r in current_sclips.iter_mut() {
            // Java: StructuralVariantsProcessor.java#L1689-L1695
            // Critical difference from markSV: DUP uses OUTER boundaries
            let (start2, end2) = if sv_r.start < sv_r.mstart {
                (sv_r.start, sv_r.mend) // outer coordinates
            } else {
                (sv_r.mstart, sv_r.end)
            };

            if instance.conf.y {
                eprintln!(
                    "   Marking DUP SV {} {} {} {} cnt: {}",
                    start, end, start2, end2, sv_r.base.vars_count
                );
            }

            // Java: StructuralVariantsProcessor.java#L1696
            if is_overlap(start, end, start2, end2, rlen) {
                if instance.conf.y {
                    eprintln!(
                        "       DUP SV {} {} {} {} cnt: {} marked",
                        start, end, start2, end2, sv_r.base.vars_count
                    );
                }
                // Java: StructuralVariantsProcessor.java#L1701-L1709
                sv_r.used = true;
                cnt += 1;
                pairs += sv_r.base.vars_count;
                if sv_r.end != sv_r.start {
                    _cov += ((sv_r.base.vars_count as i64 * rlen as i64)
                        / (sv_r.end - sv_r.start) as i64) as i32
                        + 1;
                }
            }
        }
    }
    // Java: return new Tuple.Tuple2<>(cnt, pairs) — no cov in return
    (cnt, pairs)
}

// ─── fillAndSortTmpSV ───────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.fillAndSortTmpSV(Set<Map.Entry<Integer, Sclip>>)
/// Java: StructuralVariantsProcessor.java#L975-L991
///
/// Filter soft-clip entries to current segment and sort by count descending.
/// Java sort is stable on count only, but the source map iteration order is
/// position-sensitive in practice for parity fixtures. Preserve count priority
/// and break ties by ascending position so equal-support candidates are chosen
/// deterministically.
pub fn fill_and_sort_tmp_sv(
    entries: &PositionMap<Sclip>,
    curseg: &CurrentSegment,
) -> Vec<SortPositionSclip> {
    let mut tmp: Vec<SortPositionSclip> = Vec::new();

    // Java: StructuralVariantsProcessor.java#L977-L985
    for (&position, sclip) in entries.iter() {
        if sclip.used {
            continue;
        }
        if position < curseg.start || position > curseg.end {
            continue;
        }
        tmp.push(SortPositionSclip::new(
            position,
            sclip.clone(),
            sclip.base.vars_count,
        ));
    }

    // Java: StructuralVariantsProcessor.java#L986-L987
    // Keep equal-count candidates in ascending position order so the winning
    // soft-clip is deterministic across Rust HashMap iteration.
    tmp.sort_by(|a, b| b.count.cmp(&a.count).then(a.position.cmp(&b.position)));

    tmp
}

fn java_hashmap_i32_keys(map: &PositionMap<Sclip>) -> Vec<i32> {
    java_hashmap_i32_order_from_keys(map.keys().copied())
}

// ─── Cluster B–D stubs ──────────────────────────────────────────────

// ─── Local helper: get_sv ───────────────────────────────────────────

/// Get-or-create SV metadata for a position in non_insertion_variants.
/// Java: VariationMap.getSV()
/// Duplicate of variation_realigner's private get_sv — needed for cross-module access.
fn get_sv(non_insertion_variants: &mut PositionMap<VariationMap>, pos: i32) -> &mut VariationMapSV {
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

struct PartialPipelineContext<'a> {
    bam: &'a str,
    region: &'a Region,
    reference_resource: &'a Arc<ReferenceResource>,
    splice: &'a HashSet<String>,
    out: &'a Arc<VariantPrinter>,
}

/// Ported from: AbstractMode.partialPipeline()
/// Java: AbstractMode.java:L79-L85
fn run_partial_pipeline(
    context: &PartialPipelineContext<'_>,
    ms: i32,
    me: i32,
    max_read_length: i32,
    reference: &mut Reference,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
) {
    let modified_start = std::cmp::max(1, ms - 200);
    let modified_end = me + 200;
    if modified_start > modified_end || context.bam.is_empty() {
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
    let scope_reference = std::mem::take(reference);
    let scope = Scope {
        bam: context.bam.to_string(),
        region: modified_region,
        region_ref: Arc::new(scope_reference),
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
    drop(parser);
    *reference = Arc::try_unwrap(region_ref).unwrap_or_else(|region_ref| (*region_ref).clone());
    *non_insertion_variants = variation_data.non_insertion_variants;
    *insertion_variants = variation_data.insertion_variants;
    *ref_coverage = variation_data.ref_coverage;
    *soft_clips_3_end = variation_data.soft_clips_3_end;
    *soft_clips_5_end = variation_data.soft_clips_5_end;
}

/// Reverse-complement a whole string (mirrors Java SequenceUtil.reverseComplement).
fn reverse_complement_str(s: &str) -> String {
    let rc = get_reverse_complemented_sequence(s.as_bytes(), 0, s.len() as i32);
    String::from_utf8(rc).unwrap_or_default()
}

/// Helper: extend reference if not already loaded for [mstart, mend].
/// Java: referenceResource.getReference(Region.newModifiedRegion(region, mstart, mend), extension, reference)
/// The Rust get_reference_with_extension takes reference by value and returns it.
fn extend_reference_if_needed(
    chr: &str,
    mstart: i32,
    mend: i32,
    extension: i32,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
) {
    if !ReferenceResource::is_loaded(chr, mstart, mend, reference) {
        let modified_region = Region::new_modified_region(region, mstart, mend);
        let old_ref = std::mem::take(reference);
        match reference_resource.get_reference_with_extension(&modified_region, extension, old_ref)
        {
            Ok(new_ref) => *reference = new_ref,
            Err(e) => {
                eprintln!("Warning: get_reference failed: {}", e);
            }
        }
    }
}

/// Ported from: StructuralVariantsProcessor.findDUPdisc()
/// Java: StructuralVariantsProcessor.java#L1368-L1375,L1511-L1518
fn prefetch_dup_breakpoint_reference_if_missing(
    predicted: i32,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
) {
    if !reference.reference_sequences.contains_key(&predicted) {
        let modified_region = Region::new_modified_region(region, predicted - 150, predicted + 150);
        let old_ref = std::mem::take(reference);
        match reference_resource.get_reference_with_extension(&modified_region, 300, old_ref) {
            Ok(new_ref) => *reference = new_ref,
            Err(e) => eprintln!("Warning: get_reference failed: {}", e),
        }
    }
}

// ─── findDEL ────────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.findDEL()
/// Java: StructuralVariantsProcessor.java#L147-L681
///
/// Two parts: 5' forward (svfdel loop) + 3' reverse (svrdel loop).
pub fn find_del(
    sv_structures: &mut SVStructures,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
    bams: &Option<Vec<String>>,
    splice: &Option<std::collections::BTreeSet<String>>,
) {
    let instance = GlobalReadOnlyScope::instance();
    let partial_pipeline_bam = bams
        .as_ref()
        .map(|paths| paths.join(":"))
        .unwrap_or_default();
    let partial_pipeline_splice: HashSet<String> = splice
        .as_ref()
        .map(|sites| sites.iter().cloned().collect())
        .unwrap_or_default();
    let partial_pipeline_reference_resource = Arc::new(reference_resource.clone());
    let partial_pipeline_out = Arc::new(instance.variant_printer.clone());
    let partial_pipeline_context = PartialPipelineContext {
        bam: &partial_pipeline_bam,
        region,
        reference_resource: &partial_pipeline_reference_resource,
        splice: &partial_pipeline_splice,
        out: &partial_pipeline_out,
    };

    // ── Part 1: 5' forward deletions (svfdel loop) ──
    // Java: StructuralVariantsProcessor.java#L149-L316
    for del_idx in 0..sv_structures.svfdel.len() {
        let del = &sv_structures.svfdel[del_idx];
        if del.used {
            continue;
        }
        if del.base.vars_count < instance.conf.minr {
            continue;
        }

        // Java: StructuralVariantsProcessor.java#L159-L162 — sort soft by value descending
        let mut soft_entries: Vec<(i32, i32)> = del.soft.iter().map(|(&k, &v)| (k, v)).collect();
        soft_entries.sort_by(|a, b| b.1.cmp(&a.1));
        let softp_initial = if soft_entries.is_empty() {
            0
        } else {
            soft_entries[0].0
        };

        // Java: StructuralVariantsProcessor.java#L164
        if instance.conf.y {
            eprintln!(
                "\n\nWorking DEL 5' {} mate cluster cnt: {}",
                softp_initial, del.base.vars_count
            );
        }

        // Capture del fields before mutable borrow
        let del_end = del.end;
        let del_mstart = del.mstart;
        let del_mend = del.mend;
        let del_vars_count = del.base.vars_count;
        let del_mean_quality = del.base.mean_quality;
        let del_mean_position = del.base.mean_position;
        let del_mean_mapping_quality = del.base.mean_mapping_quality;
        let del_number_of_mismatches = del.base.number_of_mismatches;

        if softp_initial != 0 {
            // Java: StructuralVariantsProcessor.java#L167-L234 — happy path with known softp
            let mut softp = softp_initial;
            if !soft_clips_3_end.contains_key(&softp) {
                continue;
            }
            let scv = soft_clips_3_end.get_mut(&softp).unwrap();
            if scv.used {
                continue;
            }
            // Java: StructuralVariantsProcessor.java#L174
            let seq = find_conseq(scv, 0);
            if seq.is_empty() || (seq.len() as i32) < SEED_2 {
                continue;
            }

            // Java: StructuralVariantsProcessor.java#L178-L182 — extend reference + partialPipeline
            if !ReferenceResource::is_loaded(&region.chr, del_mstart, del_mend, reference) {
                extend_reference_if_needed(
                    &region.chr,
                    del_mstart,
                    del_mend,
                    300,
                    reference,
                    reference_resource,
                    region,
                );
                run_partial_pipeline(
                    &partial_pipeline_context,
                    del_mstart,
                    del_mend,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
            }

            // Java: StructuralVariantsProcessor.java#L184-L188
            let m = find_match_default(&seq, reference, softp, 1);
            let mut bp = m.base_position;
            let _extra = m.matched_sequence;
            if bp == 0 {
                continue;
            }
            // Java: StructuralVariantsProcessor.java#L192-L193
            if !(bp - softp > 30 && is_overlap(softp, bp, del_end, del_mstart, max_read_length)) {
                continue;
            }
            // Java: StructuralVariantsProcessor.java#L194
            bp -= 1;
            let dellen = bp - softp + 1;
            let ref_seqs = &reference.reference_sequences;
            // Java: StructuralVariantsProcessor.java#L197-L201 — left-alignment
            while ref_seqs.contains_key(&bp)
                && ref_seqs.contains_key(&(softp - 1))
                && ref_seqs.get(&bp) == ref_seqs.get(&(softp - 1))
            {
                bp -= 1;
                if bp != 0 {
                    softp -= 1;
                }
            }

            // Java: StructuralVariantsProcessor.java#L203-L204
            let vn = format!("-{}", dellen);
            let variation = get_variation(non_insertion_variants, softp, &vn);
            variation.vars_count = 0;

            // Java: StructuralVariantsProcessor.java#L206-L210
            let sv = get_sv(non_insertion_variants, softp);
            sv.type_ = Some("DEL".to_string());
            sv.pairs += del_vars_count;
            let scv_vars_count = soft_clips_3_end
                .get(&softp_initial)
                .map_or(0, |s| s.base.vars_count);
            sv.splits += scv_vars_count;
            sv.clusters += 1;

            // Java: StructuralVariantsProcessor.java#L212-L216
            if !(ref_coverage.contains_key(&softp)
                && *ref_coverage.get(&softp).unwrap() > del_vars_count)
            {
                ref_coverage.insert(softp, del_vars_count);
            }
            if ref_coverage.contains_key(&bp)
                && ref_coverage.get(&softp).unwrap_or(&0) < ref_coverage.get(&bp).unwrap_or(&0)
            {
                ref_coverage.insert(softp, *ref_coverage.get(&bp).unwrap());
            }

            // Java: StructuralVariantsProcessor.java#L218 — adjCnt with reference variant
            let ref_base = reference.reference_sequences.get(&softp).copied();
            let scv_clone = soft_clips_3_end.get(&softp_initial).unwrap().base.clone();
            if let Some(vmap) = non_insertion_variants.get_mut(&softp) {
                let mut variation = vmap.entries.remove(&vn).unwrap_or_default();
                let mut ref_var = ref_base.and_then(|rb| {
                    let rb_str = char::from(rb).to_string();
                    vmap.entries
                        .remove(&rb_str)
                        .map(|variation| (rb_str, variation))
                });

                adj_cnt_with_reference(
                    &mut variation,
                    &scv_clone,
                    ref_var.as_mut().map(|(_, variation)| variation),
                );

                vmap.entries.insert(vn.clone(), variation);
                if let Some((rb_str, reference_variation)) = ref_var {
                    vmap.entries.insert(rb_str, reference_variation);
                }
            }

            // Java: StructuralVariantsProcessor.java#L221-L230 — build tv from pairs
            let mcnt = del_vars_count;
            let mut tv = Variation::default();
            tv.vars_count = mcnt;
            tv.high_quality_reads_count = mcnt;
            tv.vars_count_on_forward = mcnt / 2;
            tv.vars_count_on_reverse = mcnt - mcnt / 2;
            tv.mean_quality = del_mean_quality * (mcnt as f64) / (del_vars_count as f64);
            tv.mean_position = del_mean_position * (mcnt as f64) / (del_vars_count as f64);
            tv.mean_mapping_quality =
                del_mean_mapping_quality * (mcnt as f64) / (del_vars_count as f64);
            tv.number_of_mismatches =
                del_number_of_mismatches * (mcnt as f64) / (del_vars_count as f64);

            let variation = get_variation(non_insertion_variants, softp, &vn);
            adj_cnt(variation, &tv);

            // Java: StructuralVariantsProcessor.java#L231
            sv_structures.svfdel[del_idx].used = true;

            // Java: StructuralVariantsProcessor.java#L232
            mark_sv(softp, bp, &mut [&mut sv_structures.svrdel], max_read_length);

            if instance.conf.y {
                eprintln!(
                    "    Found DEL SV from 5' softclip unhappy reads: {} -{} Cnt: {} AdjCnt: {}",
                    bp, dellen, del_vars_count, mcnt
                );
            }
        } else {
            // Java: StructuralVariantsProcessor.java#L237-L314 — no softp, scan softClips3End
            if instance.conf.y {
                eprintln!(
                    "\n\nWorking DEL 5' no softp mate cluster cnt: {}",
                    del_vars_count
                );
            }

            extend_reference_if_needed(
                &region.chr,
                del_mstart,
                del_mend,
                300,
                reference,
                reference_resource,
                region,
            );

            // Java scans the raw HashMap entrySet here and breaks on the first
            // matching soft clip. Reproduce that bucket traversal order instead
            // of sorting numerically, otherwise the wrong breakpoint wins.
            let sc3_keys = java_hashmap_i32_keys(soft_clips_3_end);
            let mut found = false;
            for i in sc3_keys {
                if found {
                    break;
                }
                let scv = match soft_clips_3_end.get_mut(&i) {
                    Some(s) => s,
                    None => continue,
                };
                if scv.used {
                    continue;
                }
                // Java: StructuralVariantsProcessor.java#L252
                if !(i >= del_end - 3 && i - del_end < 3 * max_read_length) {
                    continue;
                }
                let seq = find_conseq(scv, 0);
                if seq.is_empty() || (seq.len() as i32) < SEED_2 {
                    continue;
                }
                let softp_now = i;

                // Java: StructuralVariantsProcessor.java#L260-L270
                let m = find_match_default(&seq, reference, softp_now, 1);
                let mut bp = m.base_position;
                let _extra = m.matched_sequence;
                if bp == 0 {
                    let m2 = find_match(&seq, reference, softp_now, 1, SEED_2, 0);
                    bp = m2.base_position;
                }
                if bp == 0 {
                    continue;
                }
                // Java: StructuralVariantsProcessor.java#L272
                if !(bp - softp_now > 30
                    && is_overlap(softp_now, bp, del_end, del_mstart, max_read_length))
                {
                    continue;
                }
                bp -= 1;
                let softp = softp_now;
                let dellen = bp - softp + 1;

                // Java: StructuralVariantsProcessor.java#L278
                let vn = format!("-{}", dellen);
                let variation = get_variation(non_insertion_variants, softp, &vn);
                variation.vars_count = 0;

                let sv = get_sv(non_insertion_variants, softp);
                sv.type_ = Some("DEL".to_string());
                sv.pairs += del_vars_count;
                let scv_vc = soft_clips_3_end.get(&i).map_or(0, |s| s.base.vars_count);
                sv.splits += scv_vc;
                sv.clusters += 1;

                // Java: StructuralVariantsProcessor.java#L289-L292
                if !(ref_coverage.contains_key(&softp)
                    && *ref_coverage.get(&softp).unwrap() > del_vars_count)
                {
                    ref_coverage.insert(softp, del_vars_count);
                }
                if ref_coverage.contains_key(&bp)
                    && ref_coverage.get(&softp).unwrap_or(&0) < ref_coverage.get(&bp).unwrap_or(&0)
                {
                    ref_coverage.insert(softp, *ref_coverage.get(&bp).unwrap());
                }
                // Java: StructuralVariantsProcessor.java#L293 — adjCnt (no reference var)
                let scv_clone = soft_clips_3_end.get(&i).unwrap().base.clone();
                let variation = get_variation(non_insertion_variants, softp, &vn);
                adj_cnt(variation, &scv_clone);

                // Java: StructuralVariantsProcessor.java#L294-L303 — tv from pairs
                let mcnt = del_vars_count;
                let mut tv = Variation::default();
                tv.vars_count = mcnt;
                tv.high_quality_reads_count = mcnt;
                tv.vars_count_on_forward = mcnt / 2;
                tv.vars_count_on_reverse = mcnt - mcnt / 2;
                tv.mean_quality = del_mean_quality * (mcnt as f64) / (del_vars_count as f64);
                tv.mean_position = del_mean_position * (mcnt as f64) / (del_vars_count as f64);
                tv.mean_mapping_quality =
                    del_mean_mapping_quality * (mcnt as f64) / (del_vars_count as f64);
                tv.number_of_mismatches =
                    del_number_of_mismatches * (mcnt as f64) / (del_vars_count as f64);
                let variation = get_variation(non_insertion_variants, softp, &vn);
                adj_cnt(variation, &tv);

                sv_structures.svfdel[del_idx].used = true;
                mark_sv(softp, bp, &mut [&mut sv_structures.svrdel], max_read_length);

                if instance.conf.y {
                    eprintln!(
                        "    Found DEL SV from 5' softclip happy reads: {} -{} Cnt: {} AdjCnt: {}",
                        bp, dellen, del_vars_count, mcnt
                    );
                }
                // Java: StructuralVariantsProcessor.java#L314 — break after first match
                found = true;
            }
        }
    }

    // ── Part 2: 3' reverse deletions (svrdel loop) ──
    // Java: StructuralVariantsProcessor.java#L320-L681
    for del_idx in 0..sv_structures.svrdel.len() {
        let del = &sv_structures.svrdel[del_idx];
        if del.used {
            continue;
        }
        if del.base.vars_count < instance.conf.minr {
            continue;
        }

        // Java: StructuralVariantsProcessor.java#L329-L332 — sort soft by value descending
        let mut soft_entries: Vec<(i32, i32)> = del.soft.iter().map(|(&k, &v)| (k, v)).collect();
        soft_entries.sort_by(|a, b| b.1.cmp(&a.1));
        let softp_initial = if soft_entries.is_empty() {
            0
        } else {
            soft_entries[0].0
        };

        // Capture fields before mutable borrows
        let del_start = del.start;
        let del_mstart = del.mstart;
        let del_mend = del.mend;
        let del_vars_count = del.base.vars_count;
        let del_mean_quality = del.base.mean_quality;
        let del_mean_position = del.base.mean_position;
        let del_mean_mapping_quality = del.base.mean_mapping_quality;
        let del_number_of_mismatches = del.base.number_of_mismatches;

        if softp_initial != 0 {
            // Java: StructuralVariantsProcessor.java#L336-L413 — happy path
            let mut softp = softp_initial;
            if instance.conf.y {
                eprintln!(
                    "\n\nWorking DEL 3' {} mate cluster cnt: {}",
                    softp, del_vars_count
                );
            }
            if !soft_clips_5_end.contains_key(&softp) {
                continue;
            }
            let scv = soft_clips_5_end.get_mut(&softp).unwrap();
            if scv.used {
                continue;
            }
            let seq = find_conseq(scv, 0);
            if seq.is_empty() || (seq.len() as i32) < SEED_2 {
                continue;
            }

            if !ReferenceResource::is_loaded(&region.chr, del_mstart, del_mend, reference) {
                extend_reference_if_needed(
                    &region.chr,
                    del_mstart,
                    del_mend,
                    300,
                    reference,
                    reference_resource,
                    region,
                );
                run_partial_pipeline(
                    &partial_pipeline_context,
                    del_mstart,
                    del_mend,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
            }

            // Java: StructuralVariantsProcessor.java#L359-L368
            let m = find_match_default(&seq, reference, softp, -1);
            let mut bp = m.base_position;
            let _extra = m.matched_sequence;
            if bp == 0 {
                let m2 = find_match(&seq, reference, softp, -1, SEED_2, 0);
                bp = m2.base_position;
            }
            if bp == 0 {
                continue;
            }
            // Java: StructuralVariantsProcessor.java#L371-L372
            if !(softp - bp > 30 && is_overlap(bp, softp, del_mend, del_start, max_read_length)) {
                continue;
            }
            // Java: StructuralVariantsProcessor.java#L373-L374
            bp += 1;
            softp -= 1;
            let dellen = softp - bp + 1;

            // Java: StructuralVariantsProcessor.java#L375
            let vn = format!("-{}", dellen);
            let variation = get_variation(non_insertion_variants, bp, &vn);
            variation.vars_count = 0;

            let sv = get_sv(non_insertion_variants, bp);
            sv.type_ = Some("DEL".to_string());
            sv.pairs += del_vars_count;
            let scv_vars_count = soft_clips_5_end
                .get(&softp_initial)
                .map_or(0, |s| s.base.vars_count);
            sv.splits += scv_vars_count;
            sv.clusters += 1;

            // Java: StructuralVariantsProcessor.java#L386 — adjCnt (no reference var in 3' happy path)
            let scv_clone = soft_clips_5_end.get(&softp_initial).unwrap().base.clone();
            let variation = get_variation(non_insertion_variants, bp, &vn);
            adj_cnt(variation, &scv_clone);

            // Java: StructuralVariantsProcessor.java#L387-L391
            if !(ref_coverage.contains_key(&bp) && *ref_coverage.get(&bp).unwrap() > del_vars_count)
            {
                ref_coverage.insert(bp, del_vars_count);
            }
            if ref_coverage.contains_key(&softp)
                && ref_coverage.get(&softp).unwrap_or(&0) > ref_coverage.get(&bp).unwrap_or(&0)
            {
                ref_coverage.insert(bp, *ref_coverage.get(&softp).unwrap());
            }

            // Java: StructuralVariantsProcessor.java#L393-L402 — tv from pairs
            let mcnt = del_vars_count;
            let mut tv = Variation::default();
            tv.vars_count = mcnt;
            tv.high_quality_reads_count = mcnt;
            tv.vars_count_on_forward = mcnt / 2;
            tv.vars_count_on_reverse = mcnt - mcnt / 2;
            tv.mean_quality = del_mean_quality * (mcnt as f64) / (del_vars_count as f64);
            tv.mean_position = del_mean_position * (mcnt as f64) / (del_vars_count as f64);
            tv.mean_mapping_quality =
                del_mean_mapping_quality * (mcnt as f64) / (del_vars_count as f64);
            tv.number_of_mismatches =
                del_number_of_mismatches * (mcnt as f64) / (del_vars_count as f64);
            let variation = get_variation(non_insertion_variants, bp, &vn);
            adj_cnt(variation, &tv);

            sv_structures.svrdel[del_idx].used = true;
            mark_sv(bp, softp, &mut [&mut sv_structures.svfdel], max_read_length);

            if instance.conf.y {
                eprintln!(
                    "    Found DEL SV from 3' softclip unhappy reads: {} -+{} Cnt: {} AdjCnt: {}",
                    bp, dellen, del_vars_count, mcnt
                );
            }
        } else {
            // Java: StructuralVariantsProcessor.java#L416-L510 — no softp, scan softClips5End
            if instance.conf.y {
                eprintln!(
                    "\n\nWorking DEL 3' no softp mate cluster {} {} {} cnt: {}",
                    region.chr, del_mstart, del_mend, del_vars_count
                );
            }

            extend_reference_if_needed(
                &region.chr,
                del_mstart,
                del_mend,
                300,
                reference,
                reference_resource,
                region,
            );

            // Java scans the raw HashMap entrySet here and breaks on the first
            // matching soft clip. Reproduce that bucket traversal order instead
            // of sorting numerically, otherwise the wrong breakpoint wins.
            let sc5_keys = java_hashmap_i32_keys(soft_clips_5_end);
            for i in sc5_keys {
                let scv = match soft_clips_5_end.get_mut(&i) {
                    Some(s) => s,
                    None => continue,
                };
                if scv.used {
                    continue;
                }
                // Java: StructuralVariantsProcessor.java#L434
                if !(i <= del_start + 3 && del_start - i < 3 * max_read_length) {
                    continue;
                }
                let seq = find_conseq(scv, 0);
                if seq.is_empty() || (seq.len() as i32) < SEED_2 {
                    continue;
                }
                let mut softp = i;
                let scv_vc = scv.base.vars_count;
                let scv_clone = scv.base.clone();

                // Java: StructuralVariantsProcessor.java#L443-L453
                let m = find_match_default(&seq, reference, softp, -1);
                let mut bp = m.base_position;
                let _extra = m.matched_sequence;
                if bp == 0 {
                    let m2 = find_match(&seq, reference, softp, -1, SEED_2, 0);
                    bp = m2.base_position;
                }
                if bp == 0 {
                    continue;
                }
                if !(softp - bp > 30 && is_overlap(bp, softp, del_mend, del_start, max_read_length))
                {
                    continue;
                }
                bp += 1;
                softp -= 1;
                let dellen = softp - bp + 1;

                // Java: StructuralVariantsProcessor.java#L462
                let vn = format!("-{}", dellen);
                let variation = get_variation(non_insertion_variants, bp, &vn);
                variation.vars_count = 0;

                let sv = get_sv(non_insertion_variants, bp);
                sv.type_ = Some("DEL".to_string());
                sv.pairs += del_vars_count;
                sv.splits += scv_vc;
                sv.clusters += 1;

                // Java: StructuralVariantsProcessor.java#L475 — adjCnt (no reference var)
                let variation = get_variation(non_insertion_variants, bp, &vn);
                adj_cnt(variation, &scv_clone);

                // Java: StructuralVariantsProcessor.java#L476-L479
                if !ref_coverage.contains_key(&bp) {
                    ref_coverage.insert(bp, del_vars_count);
                }
                if ref_coverage.contains_key(&softp)
                    && ref_coverage.get(&softp).unwrap_or(&0) > ref_coverage.get(&bp).unwrap_or(&0)
                {
                    ref_coverage.insert(bp, *ref_coverage.get(&softp).unwrap());
                }
                // Java: StructuralVariantsProcessor.java#L482 — 3' no-softp calls incCnt (asymmetry!)
                inc_cnt(ref_coverage, bp, scv_vc);

                // Java: StructuralVariantsProcessor.java#L484-L493
                let mcnt = del_vars_count;
                let mut tv = Variation::default();
                tv.vars_count = mcnt;
                tv.high_quality_reads_count = mcnt;
                tv.vars_count_on_forward = mcnt / 2;
                tv.vars_count_on_reverse = mcnt - mcnt / 2;
                tv.mean_quality = del_mean_quality * (mcnt as f64) / (del_vars_count as f64);
                tv.mean_position = del_mean_position * (mcnt as f64) / (del_vars_count as f64);
                tv.mean_mapping_quality =
                    del_mean_mapping_quality * (mcnt as f64) / (del_vars_count as f64);
                tv.number_of_mismatches =
                    del_number_of_mismatches * (mcnt as f64) / (del_vars_count as f64);
                let variation = get_variation(non_insertion_variants, bp, &vn);
                adj_cnt(variation, &tv);

                sv_structures.svrdel[del_idx].used = true;
                mark_sv(bp, softp, &mut [&mut sv_structures.svfdel], max_read_length);

                if instance.conf.y {
                    eprintln!(
                        "    Found DEL SV from 3' softclip happy reads: {} -{} Cnt: {} AdjCnt: {}",
                        bp, dellen, del_vars_count, mcnt
                    );
                }
                // Java: StructuralVariantsProcessor.java#L508 — break after first match
                break;
            }
        }
    }
}

// ─── findINV ────────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.findINV()
/// Java: StructuralVariantsProcessor.java#L686-L690
///
/// Thin dispatcher: calls findINVsub for all 4 INV lists in exact order.
pub fn find_inv(
    sv_structures: &mut SVStructures,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
    bams: &Option<Vec<String>>,
    splice: &Option<std::collections::BTreeSet<String>>,
) {
    let instance = GlobalReadOnlyScope::instance();
    let partial_pipeline_bam = bams
        .as_ref()
        .map(|paths| paths.join(":"))
        .unwrap_or_default();
    let partial_pipeline_splice: HashSet<String> = splice
        .as_ref()
        .map(|sites| sites.iter().cloned().collect())
        .unwrap_or_default();
    let partial_pipeline_reference_resource = Arc::new(reference_resource.clone());
    let partial_pipeline_out = Arc::new(instance.variant_printer.clone());
    let partial_pipeline_context = PartialPipelineContext {
        bam: &partial_pipeline_bam,
        region,
        reference_resource: &partial_pipeline_reference_resource,
        splice: &partial_pipeline_splice,
        out: &partial_pipeline_out,
    };

    // Java: StructuralVariantsProcessor.java#L687
    find_inv_sub(
        &mut sv_structures.svfinv5,
        1,
        Side::Five,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        &partial_pipeline_context,
        bams,
    );
    // Java: StructuralVariantsProcessor.java#L688
    find_inv_sub(
        &mut sv_structures.svrinv5,
        -1,
        Side::Five,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        &partial_pipeline_context,
        bams,
    );
    // Java: StructuralVariantsProcessor.java#L689
    find_inv_sub(
        &mut sv_structures.svfinv3,
        1,
        Side::Three,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        &partial_pipeline_context,
        bams,
    );
    // Java: StructuralVariantsProcessor.java#L690
    find_inv_sub(
        &mut sv_structures.svrinv3,
        -1,
        Side::Three,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        &partial_pipeline_context,
        bams,
    );
}

// ─── findINVsub ─────────────────────────────────────────────────────

/// Ported from: StructuralVariantsProcessor.findINVsub()
/// Java: StructuralVariantsProcessor.java#L700-L892
///
/// Find INV structural variants from one direction/side combination.
#[allow(clippy::too_many_arguments)]
fn find_inv_sub(
    svref: &mut Vec<Sclip>,
    dir: i32,
    side: Side,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
    partial_pipeline_context: &PartialPipelineContext<'_>,
    bams: &Option<Vec<String>>,
) {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L701
    for inv_idx in 0..svref.len() {
        // Per-iteration error handling: Java catch+continue
        let result: Result<bool, ()> = (|| {
            let inv = &svref[inv_idx];
            if inv.used {
                return Ok(false);
            }
            if inv.base.vars_count < instance.conf.minr {
                return Ok(false);
            }

            // Java: StructuralVariantsProcessor.java#L711-L714 — sort soft
            let mut soft_entries: Vec<(i32, i32)> =
                inv.soft.iter().map(|(&k, &v)| (k, v)).collect();
            soft_entries.sort_by(|a, b| b.1.cmp(&a.1));
            let mut softp: i32 = if soft_entries.is_empty() {
                0
            } else {
                soft_entries[0].0
            };

            let inv = &svref[inv_idx];
            let inv_mstart = inv.mstart;
            let inv_mend = inv.mend;
            let inv_mlen = inv.mlen;
            let inv_start = inv.start;
            let inv_end = inv.end;
            let inv_vars_count = inv.base.vars_count;

            if instance.conf.y {
                eprintln!(
                    "\n\nWorking INV {} {} {:?} pair_cnt: {}",
                    softp, dir, side, inv_vars_count
                );
            }

            // Match Java: refresh local variation state after extending the INV window.
            if !ReferenceResource::is_loaded(&region.chr, inv_mstart, inv_mend, reference) {
                extend_reference_if_needed(
                    &region.chr,
                    inv_mstart,
                    inv_mend,
                    500,
                    reference,
                    reference_resource,
                    region,
                );
                run_partial_pipeline(
                    partial_pipeline_context,
                    inv_mstart,
                    inv_mend,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
            }

            let mut bp: i32 = 0;
            let mut scv_vars_count = 0i32;
            let mut scv_sclip_key: i32 = 0; // Track the original key used to find scv
            let mut seq = String::new();
            let mut extra = String::new();

            if softp != 0 {
                // Java: StructuralVariantsProcessor.java#L730-L746
                let sclip: &mut PositionMap<Sclip> = if dir == 1 {
                    soft_clips_3_end
                } else {
                    soft_clips_5_end
                };
                if !sclip.contains_key(&softp) {
                    return Ok(false);
                }
                let scv = sclip.get_mut(&softp).unwrap();
                if scv.used {
                    return Ok(false);
                }
                seq = find_conseq(scv, 0);
                if seq.is_empty() {
                    return Ok(false);
                }
                scv_vars_count = scv.base.vars_count;
                scv_sclip_key = softp;

                let m = find_match_rev_default(&seq, reference, softp, dir);
                bp = m.base_position;
                extra = m.matched_sequence;
                if bp == 0 {
                    let m2 = find_match_rev(&seq, reference, softp, dir, SEED_2, 0);
                    bp = m2.base_position;
                    extra = m2.matched_sequence;
                }
                if bp == 0 {
                    return Ok(false);
                }
            } else {
                // Java: StructuralVariantsProcessor.java#L748-L775 — scan within 2*maxReadLength
                let sclip: &mut PositionMap<Sclip> = if dir == 1 {
                    soft_clips_3_end
                } else {
                    soft_clips_5_end
                };
                let sp = if dir == 1 { inv_end } else { inv_start };
                for i in 1..=(2 * max_read_length) {
                    let cp = sp + i * dir;
                    if !sclip.contains_key(&cp) {
                        continue;
                    }
                    let scv = sclip.get_mut(&cp).unwrap();
                    if scv.used {
                        continue;
                    }
                    seq = find_conseq(scv, 0);
                    if seq.is_empty() {
                        continue;
                    }
                    scv_vars_count = scv.base.vars_count;

                    let m = find_match_rev_default(&seq, reference, cp, dir);
                    bp = m.base_position;
                    extra = m.matched_sequence;
                    if bp == 0 {
                        let m2 = find_match_rev(&seq, reference, cp, dir, SEED_2, 0);
                        bp = m2.base_position;
                        extra = m2.matched_sequence;
                    }
                    if bp == 0 {
                        continue;
                    }
                    softp = cp;
                    scv_sclip_key = cp;
                    // Java: StructuralVariantsProcessor.java#L772-L774
                    if (dir == 1
                        && ((bp - inv_mend).abs() as f64) < MINSVCDIST * (max_read_length as f64))
                        || (dir == -1
                            && ((bp - inv_mstart).abs() as f64)
                                < MINSVCDIST * (max_read_length as f64))
                    {
                        break;
                    }
                }
                if bp == 0 {
                    return Ok(false);
                }
            }

            if instance.conf.y {
                eprintln!(
                    "    {} {} {} {:?} {} pair_cnt: {} soft_cnt: {}",
                    softp, bp, dir, side, seq, inv_vars_count, scv_vars_count
                );
            }

            // Java: StructuralVariantsProcessor.java#L782-L795 — position adjustment
            if side == Side::Five {
                if dir == -1 {
                    bp -= 1;
                }
            } else {
                // side == Three
                if dir == 1 {
                    bp += 1;
                    if bp != 0 {
                        softp -= 1;
                    }
                } else {
                    softp -= 1;
                }
            }
            // Java: StructuralVariantsProcessor.java#L796-L800
            if side == Side::Three {
                let tmp = bp;
                bp = softp;
                softp = tmp;
            }

            let ref_seqs = &reference.reference_sequences;

            // Java: StructuralVariantsProcessor.java#L802-L809 — complement-based left-alignment
            if (dir == -1 && side == Side::Five) || (dir == 1 && side == Side::Three) {
                while ref_seqs.contains_key(&softp)
                    && ref_seqs.contains_key(&bp)
                    && ref_seqs.get(&softp).copied()
                        == Some(complement_base(*ref_seqs.get(&bp).unwrap_or(&0)))
                {
                    softp += 1;
                    if softp != 0 {
                        bp -= 1;
                    }
                }
            }
            // Java: StructuralVariantsProcessor.java#L810-L815
            while ref_seqs.contains_key(&(softp - 1))
                && ref_seqs.contains_key(&(bp + 1))
                && ref_seqs.get(&(softp - 1)).copied()
                    == Some(complement_base(*ref_seqs.get(&(bp + 1)).unwrap_or(&0)))
            {
                softp -= 1;
                if softp != 0 {
                    bp += 1;
                }
            }

            // Java: StructuralVariantsProcessor.java#L816
            if bp > softp
                && bp - softp > 150
                && ((bp - softp) as f64) / (inv_mlen.abs() as f64) < 1.5
            {
                let len = bp - softp + 1;
                // Java: StructuralVariantsProcessor.java#L818-L819
                let ins5 = reverse_complement_str(&join_ref(ref_seqs, bp - SVFLANK + 1, bp));
                let ins3 = reverse_complement_str(&join_ref(ref_seqs, softp, softp + SVFLANK - 1));
                let mut ins = format!("{}<inv{}>{}", ins5, len - 2 * SVFLANK, ins3);
                // Java: StructuralVariantsProcessor.java#L821-L822
                if len - 2 * SVFLANK <= 0 {
                    ins = reverse_complement_str(&join_ref(ref_seqs, softp, bp));
                }
                // Java: StructuralVariantsProcessor.java#L824-L828
                if dir == 1 && !extra.is_empty() {
                    extra = reverse_complement_str(&extra);
                    ins = format!("{}{}", extra, ins);
                } else if dir == -1 && !extra.is_empty() {
                    ins = format!("{}{}", ins, extra);
                }
                let gt = format!("-{}^{}", len, ins);

                // Java: StructuralVariantsProcessor.java#L831
                let vref = get_variation(non_insertion_variants, softp, &gt);
                vref.pstd = true;
                vref.qstd = true;

                // Java: StructuralVariantsProcessor.java#L832
                svref[inv_idx].used = true;

                // Java: StructuralVariantsProcessor.java#L836-L839
                let sv = get_sv(non_insertion_variants, softp);
                sv.type_ = Some("INV".to_string());
                sv.splits += scv_vars_count;
                sv.pairs += inv_vars_count;
                sv.clusters += 1;

                // Java: StructuralVariantsProcessor.java#L841-L843 — adjCnt with optional ref var
                let scv_base_clone = {
                    let sclip_map: &PositionMap<Sclip> = if dir == 1 {
                        soft_clips_3_end
                    } else {
                        soft_clips_5_end
                    };
                    // Use scv_sclip_key to get the correct scv base
                    if let Some(scv_entry) = sclip_map.get(&scv_sclip_key) {
                        scv_entry.base.clone()
                    } else {
                        // Construct from scv_vars_count
                        let mut v = Variation::default();
                        v.vars_count = scv_vars_count;
                        v.high_quality_reads_count = scv_vars_count;
                        v
                    }
                };

                if dir == -1 {
                    // Get reference variant for subtraction
                    let ref_base = ref_seqs.get(&softp).copied();
                    let ref_var_clone: Option<Variation> = ref_base.and_then(|rb| {
                        let rb_str = char::from(rb).to_string();
                        non_insertion_variants
                            .get(&softp)
                            .and_then(|m| m.entries.get(&rb_str).cloned())
                    });

                    let vref = get_variation(non_insertion_variants, softp, &gt);
                    adj_cnt_with_reference(vref, &scv_base_clone, None);

                    // Subtract from reference variant
                    if let Some(_ref_v) = ref_var_clone {
                        if let Some(rb) = ref_base {
                            let rb_str = char::from(rb).to_string();
                            if let Some(vmap) = non_insertion_variants.get_mut(&softp) {
                                if let Some(rvar) = vmap.entries.get_mut(&rb_str) {
                                    rvar.vars_count -= scv_base_clone.vars_count;
                                    rvar.high_quality_reads_count -=
                                        scv_base_clone.high_quality_reads_count;
                                    rvar.low_quality_reads_count -=
                                        scv_base_clone.low_quality_reads_count;
                                    rvar.mean_position -= scv_base_clone.mean_position;
                                    rvar.mean_quality -= scv_base_clone.mean_quality;
                                    rvar.mean_mapping_quality -=
                                        scv_base_clone.mean_mapping_quality;
                                    rvar.number_of_mismatches -=
                                        scv_base_clone.number_of_mismatches;
                                    rvar.vars_count_on_forward -=
                                        scv_base_clone.vars_count_on_forward;
                                    rvar.vars_count_on_reverse -=
                                        scv_base_clone.vars_count_on_reverse;
                                    crate::variations::correct_cnt(rvar);
                                }
                            }
                        }
                    }
                } else {
                    let vref = get_variation(non_insertion_variants, softp, &gt);
                    adj_cnt_with_reference(vref, &scv_base_clone, None);
                }

                // Java: StructuralVariantsProcessor.java#L844-L847 — dels5 for realigndel
                let mut dels5: HashMap<i32, HashMap<String, i32>> = HashMap::new();
                let mut map_inner: HashMap<String, i32> = HashMap::new();
                map_inner.insert(gt.clone(), inv_vars_count);
                dels5.insert(softp, map_inner);

                // Java: StructuralVariantsProcessor.java#L848
                let ref_cov_val = ref_coverage
                    .get(&(softp - 1))
                    .copied()
                    .unwrap_or(inv_vars_count);
                ref_coverage.insert(softp, ref_cov_val);

                // Java: StructuralVariantsProcessor.java#L849 — mark scv.used
                let sclip_map_mut: &mut PositionMap<Sclip> = if dir == 1 {
                    soft_clips_3_end
                } else {
                    soft_clips_5_end
                };
                // Use scv_sclip_key to mark the exact scv entry from the search phase
                if let Some(scv_entry) = sclip_map_mut.get_mut(&scv_sclip_key) {
                    scv_entry.used = true;
                }

                // Java: StructuralVariantsProcessor.java#L851-L853 — realigndel with previousScope
                let bams_slice = bams.as_ref().map(|v| v.as_slice());
                super::variation_realigner::realigndel(
                    bams_slice,
                    bams,
                    &dels5,
                    non_insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                    &reference.reference_sequences,
                    &region.chr,
                    max_read_length,
                );
                if instance.conf.y {
                    let refcov = ref_coverage.get(&softp).copied().unwrap_or(0);
                    let ratio = if inv_mlen.abs() > 0 {
                        (bp - softp) / inv_mlen.abs()
                    } else {
                        0
                    };
                    eprintln!(
                        "  Found INV SV: {} {} {} BP: {} cov: {} Cnt: {} EXTRA: {} {} {} {} cnt: {} {}\t DIR: {} Side: {:?}",
                        seq,
                        softp,
                        gt,
                        bp,
                        refcov,
                        inv_vars_count,
                        extra,
                        inv_mstart,
                        inv_mend,
                        inv_mlen,
                        scv_vars_count,
                        ratio,
                        dir,
                        side
                    );
                }
                return Ok(true); // Java: return vref — we return early on first found
            }
            Ok(false)
        })();

        match result {
            Ok(true) => return, // Found INV, return (Java returns vref, we just return)
            Ok(false) => continue,
            Err(_) => continue, // Error handling: continue to next iteration
        }
    }
    // Java: return null
}

/// Ported from: StructuralVariantsProcessor.findsv()
/// Java: StructuralVariantsProcessor.java#L697-L975
///
/// Find DEL/INV from raw soft-clip scanning — two passes: 5' then 3'.
#[allow(clippy::too_many_arguments)]
pub fn findsv(
    non_insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    sv_structures: &mut SVStructures,
    softp2sv: &HashMap<i32, Vec<Sclip>>,
    curseg: &CurrentSegment,
    region: &Region,
    max_read_length: i32,
) {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L698-L700
    let tmp5 = fill_and_sort_tmp_sv(soft_clips_5_end, curseg);
    let tmp3 = fill_and_sort_tmp_sv(soft_clips_3_end, curseg);

    // ── Pass 1: 5' soft clips ──
    // Java: StructuralVariantsProcessor.java#L703-L825
    for tuple5 in &tmp5 {
        let result: Result<(), ()> = (|| {
            let p5_orig = tuple5.position;
            let cnt5 = tuple5.count;
            // Java: StructuralVariantsProcessor.java#L709
            if cnt5 < instance.conf.minr {
                return Err(()); // break equivalent — sorted desc, so all remaining < minr
            }
            // Java: StructuralVariantsProcessor.java#L712-L713
            if let Some(sc5v) = soft_clips_5_end.get(&p5_orig) {
                if sc5v.used {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
            // Java: StructuralVariantsProcessor.java#L715-L717
            if let Some(sv_list) = softp2sv.get(&p5_orig) {
                if !sv_list.is_empty() && sv_list[0].used {
                    return Ok(());
                }
            }
            // Java: StructuralVariantsProcessor.java#L718-L721
            let seq = {
                let sc5v = soft_clips_5_end.get_mut(&p5_orig).unwrap();
                find_conseq(sc5v, 0)
            };
            if seq.is_empty() || (seq.len() as i32) < SEED_2 {
                return Ok(());
            }
            if instance.conf.y {
                eprintln!("  Finding SV 5': {} {} cnt: {}", seq, p5_orig, cnt5);
            }
            // Java: StructuralVariantsProcessor.java#L726-L728
            let m = find_match_default(&seq, reference, p5_orig, -1);
            let mut bp = m.base_position;
            let _extra = m.matched_sequence;

            if bp != 0 {
                // Java: StructuralVariantsProcessor.java#L731 — candidate deletion (bp < p5)
                if bp < p5_orig {
                    // Java: StructuralVariantsProcessor.java#L732-L739
                    let pairs_data = check_pairs(
                        &region.chr,
                        bp,
                        p5_orig,
                        &mut [&mut sv_structures.svfdel, &mut sv_structures.svrdel],
                        max_read_length,
                    );
                    let pairs = pairs_data.pairs;
                    let pmean = pairs_data.pmean;
                    let qmean = pairs_data.qmean;
                    let q_mean = pairs_data.q_mean;
                    let nm = pairs_data.nm;
                    // Java: StructuralVariantsProcessor.java#L740 — pairs==0 → continue (trap 21)
                    if pairs == 0 {
                        return Ok(());
                    }
                    // Java: StructuralVariantsProcessor.java#L742-L743
                    let p5 = p5_orig - 1;
                    bp += 1;
                    let dellen = p5 - bp + 1;

                    // Java: StructuralVariantsProcessor.java#L746
                    let vref = get_variation(non_insertion_variants, bp, &format!("-{}", dellen));
                    vref.vars_count = 0;

                    // Java: StructuralVariantsProcessor.java#L748-L752
                    let sv = get_sv(non_insertion_variants, bp);
                    sv.type_ = Some("DEL".to_string());
                    sv.pairs += pairs;
                    sv.splits += cnt5;
                    sv.clusters += if pairs != 0 { 1 } else { 0 };

                    // Java: StructuralVariantsProcessor.java#L754-L757
                    if !ref_coverage.contains_key(&bp) {
                        ref_coverage.insert(
                            bp,
                            pairs + {
                                soft_clips_5_end
                                    .get(&p5_orig)
                                    .map_or(0, |s| s.base.vars_count)
                            },
                        );
                    }
                    if ref_coverage.contains_key(&(p5 + 1))
                        && ref_coverage.get(&bp).unwrap_or(&0)
                            < ref_coverage.get(&(p5 + 1)).unwrap_or(&0)
                    {
                        ref_coverage.insert(bp, *ref_coverage.get(&(p5 + 1)).unwrap());
                    }
                    // Java: StructuralVariantsProcessor.java#L758
                    let sc5v_clone = soft_clips_5_end.get(&p5_orig).unwrap().base.clone();
                    let vref = get_variation(non_insertion_variants, bp, &format!("-{}", dellen));
                    adj_cnt(vref, &sc5v_clone);
                    // Java: StructuralVariantsProcessor.java#L759-L768
                    let mut tmp = Variation::default();
                    tmp.vars_count = pairs;
                    tmp.high_quality_reads_count = pairs;
                    tmp.vars_count_on_forward = pairs / 2;
                    tmp.vars_count_on_reverse = pairs - pairs / 2;
                    tmp.mean_position = pmean;
                    tmp.mean_quality = qmean;
                    tmp.mean_mapping_quality = q_mean;
                    tmp.number_of_mismatches = nm;
                    let vref = get_variation(non_insertion_variants, bp, &format!("-{}", dellen));
                    adj_cnt(vref, &tmp);
                    if instance.conf.y {
                        eprintln!("    Finding candidate deletion 5'");
                    }
                } else {
                    // Java: StructuralVariantsProcessor.java#L772 — candidate duplication (empty block)
                }
            } else {
                // Java: StructuralVariantsProcessor.java#L774 — candidate inversion
                let match_rev = find_match_rev_default(&seq, reference, p5_orig, -1);
                bp = match_rev.base_position;
                let extra = match_rev.matched_sequence;
                if bp == 0 {
                    return Ok(());
                }
                // Java: StructuralVariantsProcessor.java#L780
                if !((bp - p5_orig).abs() > SVFLANK) {
                    return Ok(());
                }
                let mut p5 = p5_orig;
                // Java: StructuralVariantsProcessor.java#L783-L789
                if bp > p5 {
                    // bp at 3' side — no swap
                } else {
                    // bp at 5' side — swap
                    let temp = bp;
                    bp = p5;
                    p5 = temp;
                }
                // Java: StructuralVariantsProcessor.java#L790
                bp -= 1;

                let ref_seqs = &reference.reference_sequences;
                // Java: StructuralVariantsProcessor.java#L791-L796
                while ref_seqs.contains_key(&(bp + 1))
                    && is_has_and_equals_base(
                        complement_base(*ref_seqs.get(&(bp + 1)).unwrap_or(&0)),
                        ref_seqs,
                        p5 - 1,
                    )
                {
                    p5 -= 1;
                    if p5 != 0 {
                        bp += 1;
                    }
                }

                // Java: StructuralVariantsProcessor.java#L798-L800
                let ins5 = reverse_complement_str(&join_ref(ref_seqs, bp - SVFLANK + 1, bp));
                let ins3 = reverse_complement_str(&join_ref(ref_seqs, p5, p5 + SVFLANK - 1));
                let mid = bp - p5 - ins5.len() as i32 - ins3.len() as i32 + 1;

                // Java: StructuralVariantsProcessor.java#L802-L806
                let vn = if mid <= 0 {
                    let tins = reverse_complement_str(&join_ref(ref_seqs, p5, bp));
                    format!("-{}^{}{}", bp - p5 + 1, tins, extra)
                } else {
                    format!("-{}^{}<inv{}>{}{}", bp - p5 + 1, ins5, mid, ins3, extra)
                };

                // Java: StructuralVariantsProcessor.java#L808
                let _vref = get_variation(non_insertion_variants, p5, &vn);
                // Java: StructuralVariantsProcessor.java#L810-L813
                let sv = get_sv(non_insertion_variants, p5);
                sv.type_ = Some("INV".to_string());
                sv.pairs += 0;
                sv.splits += cnt5;
                sv.clusters += 0;

                // Java: StructuralVariantsProcessor.java#L815
                let sc5v_clone = soft_clips_5_end.get(&p5_orig).unwrap().base.clone();
                let vref = get_variation(non_insertion_variants, p5, &vn);
                adj_cnt(vref, &sc5v_clone);
                inc_cnt(ref_coverage, p5, cnt5);
                // Java: StructuralVariantsProcessor.java#L817-L818
                if ref_coverage.contains_key(&bp)
                    && ref_coverage.get(&p5).unwrap_or(&0) < ref_coverage.get(&bp).unwrap_or(&0)
                {
                    ref_coverage.insert(p5, *ref_coverage.get(&bp).unwrap());
                }
                if instance.conf.y {
                    eprintln!("    Found INV: {} {} Cnt:{}", p5, vn, cnt5);
                }
            }
            Ok(())
        })();
        match result {
            Err(()) => break, // cnt5 < minr → sorted desc → break
            Ok(()) => continue,
        }
    }

    // ── Pass 2: 3' soft clips ──
    // Java: StructuralVariantsProcessor.java#L826-L975
    for tuple3 in &tmp3 {
        let result: Result<(), ()> = (|| {
            let p3_orig = tuple3.position;
            let cnt3 = tuple3.count;
            // Java: StructuralVariantsProcessor.java#L833
            if cnt3 < instance.conf.minr {
                return Err(()); // break
            }
            if let Some(sc3v) = soft_clips_3_end.get(&p3_orig) {
                if sc3v.used {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
            // Java: StructuralVariantsProcessor.java#L839-L841
            if let Some(sv_list) = softp2sv.get(&p3_orig) {
                if !sv_list.is_empty() && sv_list[0].used {
                    return Ok(());
                }
            }
            let seq = {
                let sc3v = soft_clips_3_end.get_mut(&p3_orig).unwrap();
                find_conseq(sc3v, 0)
            };
            if seq.is_empty() || (seq.len() as i32) < SEED_2 {
                return Ok(());
            }
            if instance.conf.y {
                eprintln!("  Finding SV 3': {} {} cnt: {}", seq, p3_orig, cnt3);
            }

            // Java: StructuralVariantsProcessor.java#L852-L854
            let m = find_match_default(&seq, reference, p3_orig, 1);
            let mut bp = m.base_position;
            let _extra = m.matched_sequence;

            if bp != 0 {
                if bp > p3_orig {
                    // Java: StructuralVariantsProcessor.java#L857 — candidate deletion
                    let pairs_data = check_pairs(
                        &region.chr,
                        p3_orig,
                        bp,
                        &mut [&mut sv_structures.svfdel, &mut sv_structures.svrdel],
                        max_read_length,
                    );
                    let pairs = pairs_data.pairs;
                    let pmean = pairs_data.pmean;
                    let qmean = pairs_data.qmean;
                    let q_mean = pairs_data.q_mean;
                    let nm = pairs_data.nm;
                    // Java: StructuralVariantsProcessor.java#L864 — pairs==0 → continue (trap 21)
                    if pairs == 0 {
                        return Ok(());
                    }
                    // Java: StructuralVariantsProcessor.java#L866
                    let dellen = bp - p3_orig;
                    bp -= 1;
                    let mut p3 = p3_orig;

                    let ref_seqs = &reference.reference_sequences;
                    // Java: StructuralVariantsProcessor.java#L869-L874 — left-align
                    while is_has_and_equals_index(bp, ref_seqs, p3 - 1) {
                        bp -= 1;
                        if bp != 0 {
                            p3 -= 1;
                        }
                    }

                    // Java: StructuralVariantsProcessor.java#L876
                    let vref = get_variation(non_insertion_variants, p3, &format!("-{}", dellen));
                    vref.vars_count = 0;

                    // Java: StructuralVariantsProcessor.java#L878-L882
                    let sv = get_sv(non_insertion_variants, p3);
                    sv.type_ = Some("DEL".to_string());
                    sv.pairs += pairs;
                    sv.splits += cnt3;
                    sv.clusters += if pairs != 0 { 1 } else { 0 };

                    // Java: StructuralVariantsProcessor.java#L884-L887
                    if !ref_coverage.contains_key(&p3) {
                        ref_coverage.insert(
                            p3,
                            pairs + {
                                soft_clips_3_end
                                    .get(&p3_orig)
                                    .map_or(0, |s| s.base.vars_count)
                            },
                        );
                    }
                    if ref_coverage.contains_key(&bp)
                        && ref_coverage.get(&bp).unwrap_or(&0) < ref_coverage.get(&p3).unwrap_or(&0)
                    {
                        ref_coverage.insert(bp, *ref_coverage.get(&p3).unwrap());
                    }

                    // Java: StructuralVariantsProcessor.java#L890
                    let sc3v_clone = soft_clips_3_end.get(&p3_orig).unwrap().base.clone();
                    let vref = get_variation(non_insertion_variants, p3, &format!("-{}", dellen));
                    adj_cnt(vref, &sc3v_clone);
                    // Java: StructuralVariantsProcessor.java#L891-L900
                    let mut tmp = Variation::default();
                    tmp.vars_count = pairs;
                    tmp.high_quality_reads_count = pairs;
                    tmp.vars_count_on_forward = pairs / 2;
                    tmp.vars_count_on_reverse = pairs - pairs / 2;
                    tmp.mean_position = pmean;
                    tmp.mean_quality = qmean;
                    tmp.mean_mapping_quality = q_mean;
                    tmp.number_of_mismatches = nm;
                    let vref = get_variation(non_insertion_variants, p3, &format!("-{}", dellen));
                    adj_cnt(vref, &tmp);
                    if instance.conf.y {
                        eprintln!("    Finding candidate deletion 3'");
                    }
                } else {
                    // Java: StructuralVariantsProcessor.java#L904 — candidate duplication (empty block)
                }
            } else {
                // Java: StructuralVariantsProcessor.java#L906 — candidate inversion
                let match_rev = find_match_rev_default(&seq, reference, p3_orig, 1);
                bp = match_rev.base_position;
                let extra = match_rev.matched_sequence;

                if bp == 0 {
                    return Ok(());
                }
                // Java: StructuralVariantsProcessor.java#L913
                if (bp - p3_orig).abs() <= SVFLANK {
                    return Ok(());
                }
                let mut p3 = p3_orig;
                // Java: StructuralVariantsProcessor.java#L916-L923
                if bp < p3 {
                    // bp at 5' side
                    let tmp = bp;
                    bp = p3;
                    p3 = tmp;
                    p3 += 1;
                    bp -= 1;
                } else {
                    // bp at 3' side — no action
                }

                let ref_seqs = &reference.reference_sequences;
                // Java: StructuralVariantsProcessor.java#L926-L932
                while ref_seqs.contains_key(&(bp + 1))
                    && is_has_and_equals_base(
                        complement_base(*ref_seqs.get(&(bp + 1)).unwrap_or(&0)),
                        ref_seqs,
                        p3 - 1,
                    )
                {
                    p3 -= 1;
                    if p3 != 0 {
                        bp += 1;
                    }
                }

                // Java: StructuralVariantsProcessor.java#L933-L935
                let ins5 = reverse_complement_str(&join_ref(ref_seqs, bp - SVFLANK + 1, bp));
                let ins3 = reverse_complement_str(&join_ref(ref_seqs, p3, p3 + SVFLANK - 1));
                let mid = bp - p3 - 2 * SVFLANK + 1;

                // Java: StructuralVariantsProcessor.java#L937-L941
                let vn = if mid <= 0 {
                    let tins = reverse_complement_str(&join_ref(ref_seqs, p3, bp));
                    format!("-{}^{}{}", bp - p3 + 1, extra, tins)
                } else {
                    format!("-{}^{}{}<inv{}>{}", bp - p3 + 1, extra, ins5, mid, ins3)
                };

                // Java: StructuralVariantsProcessor.java#L943
                let _vref = get_variation(non_insertion_variants, p3, &vn);

                // Java: StructuralVariantsProcessor.java#L945-L949
                let sv = get_sv(non_insertion_variants, p3);
                sv.type_ = Some("INV".to_string());
                sv.pairs += 0;
                sv.splits += cnt3;
                sv.clusters += 0;

                // Java: StructuralVariantsProcessor.java#L951
                let sc3v_clone = soft_clips_3_end.get(&p3_orig).unwrap().base.clone();
                let vref = get_variation(non_insertion_variants, p3, &vn);
                adj_cnt(vref, &sc3v_clone);
                inc_cnt(ref_coverage, p3, cnt3);

                // Java: StructuralVariantsProcessor.java#L954-L955
                if ref_coverage.contains_key(&bp)
                    && ref_coverage.get(&p3).unwrap_or(&0) < ref_coverage.get(&bp).unwrap_or(&0)
                {
                    ref_coverage.insert(p3, *ref_coverage.get(&bp).unwrap());
                }
                if instance.conf.y {
                    eprintln!(
                        "    Found INV: {} BP: {} Cov: {} {} {} EXTRA: {} Cnt: {}",
                        p3,
                        bp,
                        ref_coverage.get(&p3).unwrap_or(&0),
                        ref_coverage.get(&bp).unwrap_or(&0),
                        vn,
                        extra,
                        cnt3
                    );
                }
            }
            Ok(())
        })();
        match result {
            Err(()) => break, // cnt3 < minr → break
            Ok(()) => continue,
        }
    }
}

/// Ported from: StructuralVariantsProcessor.findDELdisc()
/// Java: StructuralVariantsProcessor.java#L1000-L1160
///
/// Find DEL SVs with discordant pairs only (no soft-clip evidence).
#[allow(clippy::too_many_arguments)]
pub fn find_del_disc(
    sv_structures: &mut SVStructures,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
    splice: &Option<std::collections::BTreeSet<String>>,
) {
    let instance = GlobalReadOnlyScope::instance();
    // Java: StructuralVariantsProcessor.java#L1001
    let mindist = 8 * max_read_length;

    // ── Pass 1: svfdel ──
    // Java: StructuralVariantsProcessor.java#L1003-L1070
    for del_idx in 0..sv_structures.svfdel.len() {
        let result: Result<(), String> = (|| {
            let del = &sv_structures.svfdel[del_idx];
            if del.used {
                return Ok(());
            }
            // Java: StructuralVariantsProcessor.java#L1008
            if let Some(sp) = splice {
                if !sp.is_empty() && del.mlen.abs() < 250000 {
                    return Ok(());
                }
            }
            if del.base.vars_count < instance.conf.minr + 5 {
                return Ok(());
            }
            if del.mstart <= del.end + mindist {
                return Ok(());
            }
            if del.base.mean_mapping_quality / (del.base.vars_count as f64) <= DISCPAIRQUAL as f64 {
                return Ok(());
            }

            // Java: StructuralVariantsProcessor.java#L1019
            let mlen = del.mstart - del.end - max_read_length / (del.base.vars_count + 1);
            if !(mlen > 0 && mlen > mindist) {
                return Ok(());
            }

            // Java: StructuralVariantsProcessor.java#L1023-L1025
            let mut bp = del.end + (max_read_length / (del.base.vars_count + 1)) / 2;
            if del.softp != 0 {
                bp = del.softp;
            }

            let del_end = del.end;
            let del_mstart = del.mstart;
            let del_mend = del.mend;
            let del_start = del.start;
            let del_vars_count = del.base.vars_count;
            let del_mean_quality = del.base.mean_quality;
            let del_mean_position = del.base.mean_position;
            let del_mean_mapping_quality = del.base.mean_mapping_quality;
            let del_number_of_mismatches = del.base.number_of_mismatches;

            // Java: StructuralVariantsProcessor.java#L1028-L1030
            if !reference.reference_sequences.contains_key(&bp) {
                let ext = if mlen < 1000 { mlen } else { 1000 };
                let modified_region = Region::new_modified_region(region, bp - 150, bp + 150);
                let old_ref = std::mem::take(reference);
                match reference_resource.get_reference_with_extension(
                    &modified_region,
                    ext,
                    old_ref,
                ) {
                    Ok(new_ref) => *reference = new_ref,
                    Err(e) => eprintln!("Warning: get_reference failed: {}", e),
                }
            }

            // Java: StructuralVariantsProcessor.java#L1033
            let vref = get_variation(non_insertion_variants, bp, &format!("-{}", mlen));
            vref.vars_count = 0;

            // Java: StructuralVariantsProcessor.java#L1034-L1040
            let sv = get_sv(non_insertion_variants, bp);
            sv.type_ = Some("DEL".to_string());
            sv.splits += soft_clips_3_end
                .get(&(del_end + 1))
                .map_or(0, |s| s.base.vars_count);
            sv.splits += soft_clips_5_end
                .get(&del_mstart)
                .map_or(0, |s| s.base.vars_count);
            sv.pairs += del_vars_count;
            sv.clusters += 1;

            if instance.conf.y {
                eprintln!(
                    "  Found DEL with discordant pairs only: cnt: {} BP: {} Len: {} {}-{}<->{}-{}",
                    del_vars_count, bp, mlen, del_start, del_end, del_mstart, del_mend
                );
            }

            // Java: StructuralVariantsProcessor.java#L1049-L1058 — doubled stats
            let mut tv = Variation::default();
            tv.vars_count = 2 * del_vars_count;
            tv.high_quality_reads_count = 2 * del_vars_count;
            tv.vars_count_on_forward = del_vars_count;
            tv.vars_count_on_reverse = del_vars_count;
            tv.mean_quality = 2.0 * del_mean_quality;
            tv.mean_position = 2.0 * del_mean_position;
            tv.mean_mapping_quality = 2.0 * del_mean_mapping_quality;
            tv.number_of_mismatches = 2.0 * del_number_of_mismatches;
            let vref = get_variation(non_insertion_variants, bp, &format!("-{}", mlen));
            adj_cnt(vref, &tv);

            // Java: StructuralVariantsProcessor.java#L1059-L1061
            if !ref_coverage.contains_key(&bp) {
                ref_coverage.insert(bp, 2 * del_vars_count);
            }
            // Java: StructuralVariantsProcessor.java#L1062
            sv_structures.svfdel[del_idx].used = true;
            // Java: StructuralVariantsProcessor.java#L1063
            mark_sv(
                del_end,
                del_mstart,
                &mut [&mut sv_structures.svrdel],
                max_read_length,
            );

            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Warning: findDELdisc svfdel: {}", e);
        }
    }

    // ── Pass 2: svrdel ──
    // Java: StructuralVariantsProcessor.java#L1068-L1160
    for del_idx in 0..sv_structures.svrdel.len() {
        let result: Result<(), String> = (|| {
            let del = &sv_structures.svrdel[del_idx];
            if del.used {
                return Ok(());
            }
            if let Some(sp) = splice {
                if !sp.is_empty() && del.mlen.abs() < 250000 {
                    return Ok(());
                }
            }
            if del.base.vars_count < instance.conf.minr + 5 {
                return Ok(());
            }
            // Java: StructuralVariantsProcessor.java#L1084
            if del.start <= del.mend + mindist {
                return Ok(());
            }
            if del.base.mean_mapping_quality / (del.base.vars_count as f64) <= DISCPAIRQUAL as f64 {
                return Ok(());
            }

            // Java: StructuralVariantsProcessor.java#L1089
            let mlen = del.start - del.mend - max_read_length / (del.base.vars_count + 1);
            if !(mlen > 0 && mlen > mindist) {
                return Ok(());
            }

            // Java: StructuralVariantsProcessor.java#L1093
            let bp = del.mend + ((max_read_length / (del.base.vars_count + 1)) / 2);

            let del_start = del.start;
            let del_end = del.end;
            let del_mstart = del.mstart;
            let del_mend = del.mend;
            let del_softp = del.softp;
            let del_vars_count = del.base.vars_count;
            let del_mean_quality = del.base.mean_quality;
            let del_mean_position = del.base.mean_position;
            let del_mean_mapping_quality = del.base.mean_mapping_quality;
            let del_number_of_mismatches = del.base.number_of_mismatches;

            // Java: StructuralVariantsProcessor.java#L1095-L1097
            if !reference.reference_sequences.contains_key(&bp) {
                let ext = if mlen < 1000 { mlen } else { 1000 };
                let modified_region = Region::new_modified_region(region, bp - 150, bp + 150);
                let old_ref = std::mem::take(reference);
                match reference_resource.get_reference_with_extension(
                    &modified_region,
                    ext,
                    old_ref,
                ) {
                    Ok(new_ref) => *reference = new_ref,
                    Err(e) => eprintln!("Warning: get_reference failed: {}", e),
                }
            }

            // Java: StructuralVariantsProcessor.java#L1100
            let vref = get_variation(non_insertion_variants, bp, &format!("-{}", mlen));
            vref.vars_count = 0;

            // Java: StructuralVariantsProcessor.java#L1102-L1108
            let sv = get_sv(non_insertion_variants, bp);
            sv.type_ = Some("DEL".to_string());
            sv.splits += soft_clips_3_end
                .get(&(del_mend + 1))
                .map_or(0, |s| s.base.vars_count);
            sv.splits += soft_clips_5_end
                .get(&del_start)
                .map_or(0, |s| s.base.vars_count);
            sv.pairs += del_vars_count;
            sv.clusters += 1;

            if instance.conf.y {
                eprintln!(
                    "  Found DEL with discordant pairs only (reverse): cnt: {} BP: {} Len: {} {}-{}<->{}-{}",
                    del_vars_count, bp, mlen, del_start, del_end, del_mstart, del_mend
                );
            }

            // Java: StructuralVariantsProcessor.java#L1118-L1119
            if del_softp != 0 {
                if let Some(sc5) = soft_clips_5_end.get_mut(&del_softp) {
                    sc5.used = true;
                }
            }

            // Java: StructuralVariantsProcessor.java#L1121-L1130 — doubled stats
            let mut tv = Variation::default();
            tv.vars_count = 2 * del_vars_count;
            tv.high_quality_reads_count = 2 * del_vars_count;
            tv.vars_count_on_forward = del_vars_count;
            tv.vars_count_on_reverse = del_vars_count;
            tv.mean_quality = 2.0 * del_mean_quality;
            tv.mean_position = 2.0 * del_mean_position;
            tv.mean_mapping_quality = 2.0 * del_mean_mapping_quality;
            tv.number_of_mismatches = 2.0 * del_number_of_mismatches;
            let vref = get_variation(non_insertion_variants, bp, &format!("-{}", mlen));
            adj_cnt(vref, &tv);

            // Java: StructuralVariantsProcessor.java#L1131-L1132
            if !ref_coverage.contains_key(&bp) {
                ref_coverage.insert(bp, 2 * del_vars_count);
            }
            // Java: StructuralVariantsProcessor.java#L1133-L1135
            if ref_coverage.contains_key(&del_start)
                && ref_coverage.get(&bp).unwrap_or(&0) < ref_coverage.get(&del_start).unwrap_or(&0)
            {
                ref_coverage.insert(bp, *ref_coverage.get(&del_start).unwrap());
            }
            // Java: StructuralVariantsProcessor.java#L1136
            sv_structures.svrdel[del_idx].used = true;
            // Java: StructuralVariantsProcessor.java#L1137
            extend_reference_if_needed(
                &region.chr,
                del_mstart - 100,
                del_mend + 100,
                200,
                reference,
                reference_resource,
                region,
            );
            // Java: StructuralVariantsProcessor.java#L1138
            mark_sv(
                del_mend,
                del_start,
                &mut [&mut sv_structures.svfdel],
                max_read_length,
            );

            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Warning: findDELdisc svrdel: {}", e);
        }
    }
}

/// Ported from: StructuralVariantsProcessor.findINVdisc()
/// Java: StructuralVariantsProcessor.java#L1166-L1390
///
/// Find INV SVs with discordant pairs only.
/// Two loops: invf5×invr5 (5' pairs) then invf3×invr3 (3' pairs).
#[allow(clippy::too_many_arguments)]
pub fn find_inv_disc(
    sv_structures: &mut SVStructures,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &PositionMap<Sclip>,
    soft_clips_5_end: &PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
) {
    let instance = GlobalReadOnlyScope::instance();

    // ── Loop 1: svfinv5 × svrinv5 ──
    // Java: StructuralVariantsProcessor.java#L1170-L1263
    for fi in 0..sv_structures.svfinv5.len() {
        if sv_structures.svfinv5[fi].used {
            continue;
        }
        let cnt = sv_structures.svfinv5[fi].base.vars_count;
        let me = sv_structures.svfinv5[fi].mend;
        let ms = sv_structures.svfinv5[fi].mstart;
        let end = sv_structures.svfinv5[fi].end;
        let start = sv_structures.svfinv5[fi].start;
        let nm = sv_structures.svfinv5[fi].base.number_of_mismatches;
        let pmean = sv_structures.svfinv5[fi].base.mean_position;
        let qmean = sv_structures.svfinv5[fi].base.mean_quality;
        let q_mean = sv_structures.svfinv5[fi].base.mean_mapping_quality;
        // Java: StructuralVariantsProcessor.java#L1183
        if !(q_mean / (cnt as f64) > DISCPAIRQUAL as f64) {
            continue;
        }

        for ri in 0..sv_structures.svrinv5.len() {
            let result: Result<(), String> = (|| {
                if sv_structures.svrinv5[ri].used {
                    return Ok(());
                }
                let rcnt = sv_structures.svrinv5[ri].base.vars_count;
                let rstart = sv_structures.svrinv5[ri].start;
                let rms = sv_structures.svrinv5[ri].mstart;
                let rnm = sv_structures.svrinv5[ri].base.number_of_mismatches;
                let rpmean = sv_structures.svrinv5[ri].base.mean_position;
                let rqmean = sv_structures.svrinv5[ri].base.mean_quality;
                let rq_mean = sv_structures.svrinv5[ri].base.mean_mapping_quality;
                // Java: StructuralVariantsProcessor.java#L1195
                if !(rq_mean / (rcnt as f64) > DISCPAIRQUAL as f64) {
                    return Ok(());
                }
                if !(cnt + rcnt > instance.conf.minr + 5) {
                    return Ok(());
                }

                if is_overlap(end, me, rstart, rms, max_read_length) {
                    // Java: StructuralVariantsProcessor.java#L1201
                    let bp = ((end + rstart) / 2).abs();
                    let pe = ((me + rms) / 2).abs();
                    // Java: StructuralVariantsProcessor.java#L1203-L1205
                    if !reference.reference_sequences.contains_key(&pe) {
                        let modified_region =
                            Region::new_modified_region(region, pe - 150, pe + 150);
                        let old_ref = std::mem::take(reference);
                        match reference_resource.get_reference_with_extension(
                            &modified_region,
                            300,
                            old_ref,
                        ) {
                            Ok(new_ref) => *reference = new_ref,
                            Err(e) => eprintln!("Warning: get_reference failed: {}", e),
                        }
                    }
                    let ref_seqs = &reference.reference_sequences;
                    let len = pe - bp + 1;
                    // Java: StructuralVariantsProcessor.java#L1209-L1210
                    let ins5 = reverse_complement_str(&join_ref(ref_seqs, bp, bp + SVFLANK - 1));
                    let ins3 = reverse_complement_str(&join_ref(ref_seqs, pe - SVFLANK + 1, pe));
                    let mut ins = format!("{}<inv{}>{}", ins3, len - 2 * SVFLANK, ins5);
                    if len - 2 * SVFLANK <= 0 {
                        ins = reverse_complement_str(&join_ref(ref_seqs, bp, pe));
                    }
                    if instance.conf.y {
                        eprintln!(
                            "  Found INV with discordant pairs only 5': cnt: {} Len: {} {}-{}<->{}-{} {}",
                            cnt, len, end, rstart, me, rms, ins
                        );
                    }
                    // Java: StructuralVariantsProcessor.java#L1218
                    let vref =
                        get_variation(non_insertion_variants, bp, &format!("-{}^{}", len, ins));
                    // Java: StructuralVariantsProcessor.java#L1220-L1221
                    sv_structures.svfinv5[fi].used = true;
                    sv_structures.svrinv5[ri].used = true;
                    vref.pstd = true;
                    vref.qstd = true;

                    // Java: StructuralVariantsProcessor.java#L1224-L1232
                    let mut tmp = Variation::default();
                    tmp.vars_count = cnt + rcnt;
                    tmp.high_quality_reads_count = cnt + rcnt;
                    tmp.vars_count_on_forward = cnt;
                    tmp.vars_count_on_reverse = rcnt;
                    tmp.mean_quality = qmean + rqmean;
                    tmp.mean_position = pmean + rpmean;
                    tmp.mean_mapping_quality = q_mean + rq_mean;
                    tmp.number_of_mismatches = nm + rnm;
                    let vref =
                        get_variation(non_insertion_variants, bp, &format!("-{}^{}", len, ins));
                    adj_cnt(vref, &tmp);

                    // Java: StructuralVariantsProcessor.java#L1234-L1238
                    let sv = get_sv(non_insertion_variants, bp);
                    sv.type_ = Some("INV".to_string());
                    sv.pairs += cnt;
                    sv.splits += soft_clips_5_end
                        .get(&start)
                        .map_or(0, |s| s.base.vars_count);
                    sv.splits += soft_clips_5_end.get(&ms).map_or(0, |s| s.base.vars_count);
                    sv.clusters += 1;

                    // Java: StructuralVariantsProcessor.java#L1240-L1242
                    if !ref_coverage.contains_key(&bp) {
                        ref_coverage.insert(bp, 2 * cnt);
                    }
                    // Java: StructuralVariantsProcessor.java#L1243
                    mark_sv(
                        bp,
                        pe,
                        &mut [&mut sv_structures.svfinv3, &mut sv_structures.svrinv3],
                        max_read_length,
                    );
                }
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("Warning: findINVdisc svfinv5×svrinv5: {}", e);
            }
        }
    }

    // ── Loop 2: svfinv3 × svrinv3 ──
    // Java: StructuralVariantsProcessor.java#L1268-L1390
    for fi in 0..sv_structures.svfinv3.len() {
        if sv_structures.svfinv3[fi].used {
            continue;
        }
        let cnt = sv_structures.svfinv3[fi].base.vars_count;
        let me = sv_structures.svfinv3[fi].mend;
        let end = sv_structures.svfinv3[fi].end;
        let nm = sv_structures.svfinv3[fi].base.number_of_mismatches;
        let pmean = sv_structures.svfinv3[fi].base.mean_position;
        let qmean = sv_structures.svfinv3[fi].base.mean_quality;
        let q_mean = sv_structures.svfinv3[fi].base.mean_mapping_quality;

        for ri in 0..sv_structures.svrinv3.len() {
            let result: Result<(), String> = (|| {
                if sv_structures.svrinv3[ri].used {
                    return Ok(());
                }
                let rcnt = sv_structures.svrinv3[ri].base.vars_count;
                let rstart = sv_structures.svrinv3[ri].start;
                let rms = sv_structures.svrinv3[ri].mstart;
                let rnm = sv_structures.svrinv3[ri].base.number_of_mismatches;
                let rpmean = sv_structures.svrinv3[ri].base.mean_position;
                let rqmean = sv_structures.svrinv3[ri].base.mean_quality;
                let rq_mean = sv_structures.svrinv3[ri].base.mean_mapping_quality;
                // Java: StructuralVariantsProcessor.java#L1291
                if !(rq_mean / (rcnt as f64) > DISCPAIRQUAL as f64) {
                    return Ok(());
                }
                if !(cnt + rcnt > instance.conf.minr + 5) {
                    return Ok(());
                }

                // Java: StructuralVariantsProcessor.java#L1295 — note: isOverlap args differ from 5'
                if is_overlap(me, end, rms, rstart, max_read_length) {
                    // Java: StructuralVariantsProcessor.java#L1296-L1297 — note: pe/bp swapped vs 5'
                    let pe = ((end + rstart) / 2).abs();
                    let bp = ((me + rms) / 2).abs();

                    if !reference.reference_sequences.contains_key(&bp) {
                        let modified_region =
                            Region::new_modified_region(region, bp - 150, bp + 150);
                        let old_ref = std::mem::take(reference);
                        match reference_resource.get_reference_with_extension(
                            &modified_region,
                            300,
                            old_ref,
                        ) {
                            Ok(new_ref) => *reference = new_ref,
                            Err(e) => eprintln!("Warning: get_reference failed: {}", e),
                        }
                    }
                    let ref_seqs = &reference.reference_sequences;
                    let len = pe - bp + 1;
                    // Java: StructuralVariantsProcessor.java#L1305-L1306
                    let ins5 = reverse_complement_str(&join_ref(ref_seqs, bp, bp + SVFLANK - 1));
                    let ins3 = reverse_complement_str(&join_ref(ref_seqs, pe - SVFLANK + 1, pe));
                    let mut ins = format!("{}<inv{}>{}", ins3, len - 2 * SVFLANK, ins5);
                    if len - 2 * SVFLANK <= 0 {
                        ins = reverse_complement_str(&join_ref(ref_seqs, bp, pe));
                    }
                    if instance.conf.y {
                        eprintln!(
                            "  Found INV with discordant pairs only 3': cnt: {} Len: {} {}-{}<->{}-{} {}",
                            cnt, len, me, rms, end, rstart, ins
                        );
                    }
                    // Java: StructuralVariantsProcessor.java#L1316
                    let vref =
                        get_variation(non_insertion_variants, bp, &format!("-{}^{}", len, ins));
                    sv_structures.svfinv3[fi].used = true;
                    sv_structures.svrinv3[ri].used = true;
                    vref.pstd = true;
                    vref.qstd = true;

                    // Java: StructuralVariantsProcessor.java#L1322-L1330
                    let mut tmp = Variation::default();
                    tmp.vars_count = cnt + rcnt;
                    tmp.high_quality_reads_count = cnt + rcnt;
                    tmp.vars_count_on_forward = cnt;
                    tmp.vars_count_on_reverse = rcnt;
                    tmp.mean_quality = qmean + rqmean;
                    tmp.mean_position = pmean + rpmean;
                    tmp.mean_mapping_quality = q_mean + rq_mean;
                    tmp.number_of_mismatches = nm + rnm;
                    let vref =
                        get_variation(non_insertion_variants, bp, &format!("-{}^{}", len, ins));
                    adj_cnt(vref, &tmp);

                    // Java: StructuralVariantsProcessor.java#L1334-L1340
                    let sv = get_sv(non_insertion_variants, bp);
                    sv.type_ = Some("INV".to_string());
                    sv.pairs += cnt;
                    sv.splits += soft_clips_3_end
                        .get(&(end + 1))
                        .map_or(0, |s| s.base.vars_count);
                    sv.splits += soft_clips_3_end
                        .get(&(me + 1))
                        .map_or(0, |s| s.base.vars_count);
                    sv.clusters += 1;

                    // Java: StructuralVariantsProcessor.java#L1342-L1344
                    if !ref_coverage.contains_key(&bp) {
                        ref_coverage.insert(bp, 2 * cnt);
                    }
                    // Java: StructuralVariantsProcessor.java#L1345
                    mark_sv(
                        bp,
                        pe,
                        &mut [&mut sv_structures.svfinv5, &mut sv_structures.svrinv5],
                        max_read_length,
                    );
                }
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("Warning: findINVdisc svfinv3×svrinv3: {}", e);
            }
        }
    }
}

/// Ported from: StructuralVariantsProcessor.findDUPdisc()
/// Java: StructuralVariantsProcessor.java#L1396-L1625
///
/// Find DUP SVs with discordant pairs only.
/// Two loops: svfdup (forward) then svrdup (reverse).
#[allow(clippy::too_many_arguments)]
pub fn find_dup_disc(
    sv_structures: &mut SVStructures,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
    bams: &Option<Vec<String>>,
    splice: &Option<std::collections::BTreeSet<String>>,
) {
    let instance = GlobalReadOnlyScope::instance();
    let partial_pipeline_bam = bams
        .as_ref()
        .map(|paths| paths.join(":"))
        .unwrap_or_default();
    let partial_pipeline_splice: HashSet<String> = splice
        .as_ref()
        .map(|sites| sites.iter().cloned().collect())
        .unwrap_or_default();
    let partial_pipeline_reference_resource = Arc::new(reference_resource.clone());
    let partial_pipeline_out = Arc::new(instance.variant_printer.clone());
    let partial_pipeline_context = PartialPipelineContext {
        bam: &partial_pipeline_bam,
        region,
        reference_resource: &partial_pipeline_reference_resource,
        splice: &partial_pipeline_splice,
        out: &partial_pipeline_out,
    };

    // ── Pass 1: svfdup ──
    // Java: StructuralVariantsProcessor.java#L1400-L1501
    for dup_idx in 0..sv_structures.svfdup.len() {
        let result: Result<(), String> = (|| {
            let dup = &sv_structures.svfdup[dup_idx];
            if dup.used {
                return Ok(());
            }
            let ms = dup.mstart;
            let me = dup.mend;
            let cnt = dup.base.vars_count;
            let mut end = dup.end;
            let start = dup.start;
            let pmean = dup.base.mean_position;
            let qmean = dup.base.mean_quality;
            let q_mean = dup.base.mean_mapping_quality;
            let nm = dup.base.number_of_mismatches;
            let softp_val = dup.softp;

            // Java: StructuralVariantsProcessor.java#L1412
            if !(cnt >= instance.conf.minr + 5) {
                return Ok(());
            }
            if !(q_mean / (cnt as f64) > DISCPAIRQUAL as f64) {
                return Ok(());
            }

            let mut mlen = end - ms + max_read_length / cnt;
            let mut bp = ms - (max_read_length / cnt) / 2;
            let mut pe = end;

            // Java: StructuralVariantsProcessor.java#L1419-L1425 — isLoaded + partialPipeline
            if !ReferenceResource::is_loaded(&region.chr, ms, me, reference) {
                prefetch_dup_breakpoint_reference_if_missing(
                    bp,
                    reference,
                    reference_resource,
                    region,
                );
                run_partial_pipeline(
                    &partial_pipeline_context,
                    ms,
                    me,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
            }

            let mut cntf = cnt;
            let mut cntr = cnt;
            let mut qmeanf = qmean;
            let mut qmeanr = qmean;
            let mut q_meanf = q_mean;
            let mut q_meanr = q_mean;
            let mut pmeanf = pmean;
            let mut pmeanr = pmean;
            let mut nmf = nm;
            let mut nmr = nm;

            // Java: StructuralVariantsProcessor.java#L1440
            let dup_soft = sv_structures.svfdup[dup_idx].soft.clone();
            if !dup_soft.is_empty() {
                // Java: StructuralVariantsProcessor.java#L1441-L1444
                let mut soft_entries: Vec<(i32, i32)> =
                    dup_soft.iter().map(|(&k, &v)| (k, v)).collect();
                soft_entries.sort_by(|a, b| b.1.cmp(&a.1));
                if !soft_entries.is_empty() {
                    pe = soft_entries[0].0;
                }
                // Java: StructuralVariantsProcessor.java#L1448-L1451
                if !soft_clips_3_end.contains_key(&pe) {
                    return Ok(());
                }
                if soft_clips_3_end.get(&pe).unwrap().used {
                    return Ok(());
                }

                let current_sclip3 = soft_clips_3_end.get_mut(&pe).unwrap();
                cntf = current_sclip3.base.vars_count;
                qmeanf = current_sclip3.base.mean_quality;
                q_meanf = current_sclip3.base.mean_mapping_quality;
                pmeanf = current_sclip3.base.mean_position;
                nmf = current_sclip3.base.number_of_mismatches;

                // Java: StructuralVariantsProcessor.java#L1458
                let seq = find_conseq(current_sclip3, 0);
                let m = find_match_default(&seq, reference, bp, 1);
                let tbp = m.base_position;

                // Java: StructuralVariantsProcessor.java#L1462-L1478
                if tbp != 0 && tbp < pe {
                    soft_clips_3_end.get_mut(&pe).unwrap().used = true;
                    let ref_seqs = &reference.reference_sequences;
                    let mut tbp_local = tbp;
                    // Java: StructuralVariantsProcessor.java#L1465
                    while is_has_and_equals_index(pe - 1, ref_seqs, tbp_local - 1) {
                        tbp_local -= 1;
                        if tbp_local != 0 {
                            pe -= 1;
                        }
                    }
                    mlen = pe - tbp_local;
                    bp = tbp_local;
                    pe -= 1;
                    end = pe;

                    if soft_clips_5_end.contains_key(&bp) {
                        let current_sclip5 = soft_clips_5_end.get(&bp).unwrap();
                        cntr = current_sclip5.base.vars_count;
                        qmeanr = current_sclip5.base.mean_quality;
                        q_meanr = current_sclip5.base.mean_mapping_quality;
                        pmeanr = current_sclip5.base.mean_position;
                        nmr = current_sclip5.base.number_of_mismatches;
                    }
                }
            }

            let ref_seqs = &reference.reference_sequences;
            // Java: StructuralVariantsProcessor.java#L1437-L1439
            let ins5 = join_ref(ref_seqs, bp, bp + SVFLANK - 1);
            let ins3 = join_ref(ref_seqs, pe - SVFLANK + 1, pe);
            let ins = format!("{}<dup{}>{}", ins5, mlen - 2 * SVFLANK, ins3);

            // Java: StructuralVariantsProcessor.java#L1441 — DUP goes into insertionVariants
            let vref = get_variation(insertion_variants, bp, &format!("+{}", ins));
            vref.vars_count = 0;

            // Java: StructuralVariantsProcessor.java#L1443-L1449 — SV metadata in nonInsertionVariants
            let sv = get_sv(non_insertion_variants, bp);
            sv.type_ = Some("DUP".to_string());
            sv.pairs += cnt;
            sv.splits += if softp_val != 0 {
                soft_clips_3_end
                    .get(&softp_val)
                    .map_or(0, |s| s.base.vars_count)
            } else {
                0
            };
            sv.clusters += 1;

            if instance.conf.y {
                eprintln!(
                    "  Found DUP with discordant pairs only (forward): cnt: {} BP: {} END: {} {} Len: {} {}-{}<->{}-{}",
                    cnt, bp, pe, ins, mlen, start, end, ms, me
                );
            }

            // Java: StructuralVariantsProcessor.java#L1458 — extracnt set for DUP
            let tcnt = cntr + cntf;
            let mut tmp = Variation::default();
            tmp.vars_count = tcnt;
            tmp.extracnt = tcnt;
            tmp.high_quality_reads_count = tcnt;
            tmp.vars_count_on_forward = cntf;
            tmp.vars_count_on_reverse = cntr;
            tmp.mean_quality = qmeanf + qmeanr;
            tmp.mean_position = pmeanf + pmeanr;
            tmp.mean_mapping_quality = q_meanf + q_meanr;
            tmp.number_of_mismatches = nmf + nmr;

            let vref = get_variation(insertion_variants, bp, &format!("+{}", ins));
            adj_cnt(vref, &tmp);

            // Java: StructuralVariantsProcessor.java#L1472
            sv_structures.svfdup[dup_idx].used = true;
            if !ref_coverage.contains_key(&bp) {
                ref_coverage.insert(bp, tcnt);
            }
            if ref_coverage.contains_key(&end)
                && ref_coverage.get(&bp).unwrap_or(&0) < ref_coverage.get(&end).unwrap_or(&0)
            {
                ref_coverage.insert(bp, *ref_coverage.get(&end).unwrap());
            }

            // Java: StructuralVariantsProcessor.java#L1478
            let (clusters, _pairs) =
                mark_dup_sv(bp, pe, &mut [&mut sv_structures.svrdup], max_read_length);
            let sv = get_sv(non_insertion_variants, bp);
            sv.clusters += clusters;

            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Warning: findDUPdisc svfdup: {}", e);
        }
    }

    // ── Pass 2: svrdup ──
    // Java: StructuralVariantsProcessor.java#L1504-L1625
    for dup_idx in 0..sv_structures.svrdup.len() {
        let result: Result<(), String> = (|| {
            let dup = &sv_structures.svrdup[dup_idx];
            if dup.used {
                return Ok(());
            }
            let ms = dup.mstart;
            let me = dup.mend;
            let cnt = dup.base.vars_count;
            let end = dup.end;
            let start = dup.start;
            let pmean = dup.base.mean_position;
            let qmean = dup.base.mean_quality;
            let q_mean = dup.base.mean_mapping_quality;
            let nm = dup.base.number_of_mismatches;

            // Java: StructuralVariantsProcessor.java#L1516
            if cnt < instance.conf.minr + 5 {
                return Ok(());
            }
            if !(q_mean / (cnt as f64) > DISCPAIRQUAL as f64) {
                return Ok(());
            }

            let mut mlen = me - start + max_read_length / cnt;
            let mut bp = start - (max_read_length / cnt) / 2;
            let mut pe = mlen + bp - 1;
            let mut tpe = pe;
            // Java: StructuralVariantsProcessor.java#L1523-L1530 — isLoaded + partialPipeline
            if !ReferenceResource::is_loaded(&region.chr, ms, me, reference) {
                prefetch_dup_breakpoint_reference_if_missing(
                    pe,
                    reference,
                    reference_resource,
                    region,
                );
                run_partial_pipeline(
                    &partial_pipeline_context,
                    ms,
                    me,
                    max_read_length,
                    reference,
                    non_insertion_variants,
                    insertion_variants,
                    ref_coverage,
                    soft_clips_3_end,
                    soft_clips_5_end,
                );
            }

            let mut cntf = cnt;
            let mut cntr = cnt;
            let mut qmeanf = qmean;
            let mut qmeanr = qmean;
            let mut q_meanf = q_mean;
            let mut q_meanr = q_mean;
            let mut pmeanf = pmean;
            let mut pmeanr = pmean;
            let mut nmf = nm;
            let mut nmr = nm;

            // Java: StructuralVariantsProcessor.java#L1542
            let dup_soft = sv_structures.svrdup[dup_idx].soft.clone();
            if !dup_soft.is_empty() {
                let mut soft_entries: Vec<(i32, i32)> =
                    dup_soft.iter().map(|(&k, &v)| (k, v)).collect();
                soft_entries.sort_by(|a, b| b.1.cmp(&a.1));
                if !soft_entries.is_empty() {
                    bp = soft_entries[0].0;
                }
                // Java: StructuralVariantsProcessor.java#L1549-L1551
                if !soft_clips_5_end.contains_key(&bp) {
                    return Ok(());
                }
                let current_sclip5 = soft_clips_5_end.get_mut(&bp).unwrap();
                if current_sclip5.used {
                    return Ok(());
                }

                cntr = current_sclip5.base.vars_count;
                qmeanr = current_sclip5.base.mean_quality;
                q_meanr = current_sclip5.base.mean_mapping_quality;
                pmeanr = current_sclip5.base.mean_position;
                nmr = current_sclip5.base.number_of_mismatches;

                // Java: StructuralVariantsProcessor.java#L1558
                let seq = find_conseq(current_sclip5, 0);
                let m = find_match_default(&seq, reference, pe, -1);
                let tbp = m.base_position;

                // Java: StructuralVariantsProcessor.java#L1561-L1577
                if tbp != 0 && tbp > bp {
                    soft_clips_5_end.get_mut(&bp).unwrap().used = true;
                    pe = tbp;
                    mlen = pe - bp + 1;
                    tpe = pe + 1;
                    let ref_seqs = &reference.reference_sequences;
                    // Java: StructuralVariantsProcessor.java#L1566
                    while is_has_and_equals_index(tpe, ref_seqs, bp + (tpe - pe - 1)) {
                        tpe += 1;
                    }
                    if soft_clips_3_end.contains_key(&tpe) {
                        let current_sclip3 = soft_clips_3_end.get(&tpe).unwrap();
                        cntf = current_sclip3.base.vars_count;
                        qmeanf = current_sclip3.base.mean_quality;
                        q_meanf = current_sclip3.base.mean_mapping_quality;
                        pmeanf = current_sclip3.base.mean_position;
                        nmf = current_sclip3.base.number_of_mismatches;
                    }
                }
            }

            let ref_seqs = &reference.reference_sequences;
            // Java: StructuralVariantsProcessor.java#L1580-L1582
            let ins5 = join_ref(ref_seqs, bp, bp + SVFLANK - 1);
            let ins3 = join_ref(ref_seqs, pe - SVFLANK + 1, pe);
            let ins = format!("{}<dup{}>{}", ins5, mlen - 2 * SVFLANK, ins3);

            // Java: StructuralVariantsProcessor.java#L1584 — DUP into insertionVariants
            let vref = get_variation(insertion_variants, bp, &format!("+{}", ins));
            vref.vars_count = 0;

            // Java: StructuralVariantsProcessor.java#L1586-L1592 — SV metadata in nonInsertionVariants
            let sv = get_sv(non_insertion_variants, bp);
            sv.type_ = Some("DUP".to_string());
            sv.pairs += cnt;
            sv.splits += soft_clips_5_end.get(&bp).map_or(0, |s| s.base.vars_count);
            sv.splits += soft_clips_3_end.get(&tpe).map_or(0, |s| s.base.vars_count);
            sv.clusters += 1;

            if instance.conf.y {
                eprintln!(
                    "  Found DUP with discordant pairs only (reverse): cnt: {} BP: {} Len: {} {}-{}<->{}-{}",
                    cnt, bp, mlen, start, end, ms, me
                );
            }

            // Java: StructuralVariantsProcessor.java#L1601 — extracnt set for DUP
            let tcnt = cntr + cntf;
            let mut tmp = Variation::default();
            tmp.vars_count = tcnt;
            tmp.extracnt = tcnt;
            tmp.high_quality_reads_count = tcnt;
            tmp.vars_count_on_forward = cntf;
            tmp.vars_count_on_reverse = cntr;
            tmp.mean_quality = qmeanf + qmeanr;
            tmp.mean_position = pmeanf + pmeanr;
            tmp.mean_mapping_quality = q_meanf + q_meanr;
            tmp.number_of_mismatches = nmf + nmr;

            let vref = get_variation(insertion_variants, bp, &format!("+{}", ins));
            adj_cnt(vref, &tmp);

            // Java: StructuralVariantsProcessor.java#L1614
            sv_structures.svrdup[dup_idx].used = true;
            if !ref_coverage.contains_key(&bp) {
                ref_coverage.insert(bp, tcnt);
            }
            if ref_coverage.contains_key(&me)
                && ref_coverage.get(&bp).unwrap_or(&0) < ref_coverage.get(&me).unwrap_or(&0)
            {
                ref_coverage.insert(bp, *ref_coverage.get(&me).unwrap());
            }

            // Java: StructuralVariantsProcessor.java#L1620
            let (clusters, _pairs) =
                mark_dup_sv(bp, pe, &mut [&mut sv_structures.svfdup], max_read_length);
            let sv = get_sv(non_insertion_variants, bp);
            sv.clusters += clusters;

            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Warning: findDUPdisc svrdup: {}", e);
        }
    }
}

/// Ported from: StructuralVariantsProcessor.findAllSVs()
/// Java: StructuralVariantsProcessor.java#L117-L145
///
/// Parity helper: Java's SOFTP2SV stores live Sclip references. Rust stores
/// cloned values, so rebuild that view from current svStructures before
/// consumers inspect used flags or serialize snapshots.
fn rebuild_softp2sv_from_sv_structures(
    sv_structures: &SVStructures,
    softp2sv: &mut HashMap<i32, Vec<Sclip>>,
) {
    fn push_softp2sv_entries(softp2sv: &mut HashMap<i32, Vec<Sclip>>, sv_list: &[Sclip]) {
        for sv in sv_list {
            if sv.softp != 0 {
                softp2sv.entry(sv.softp).or_default().push(sv.clone());
            }
        }
    }

    softp2sv.clear();

    push_softp2sv_entries(softp2sv, &sv_structures.svfinv3);
    push_softp2sv_entries(softp2sv, &sv_structures.svrinv3);
    push_softp2sv_entries(softp2sv, &sv_structures.svfinv5);
    push_softp2sv_entries(softp2sv, &sv_structures.svrinv5);
    push_softp2sv_entries(softp2sv, &sv_structures.svfdel);
    push_softp2sv_entries(softp2sv, &sv_structures.svrdel);
    push_softp2sv_entries(softp2sv, &sv_structures.svfdup);
    push_softp2sv_entries(softp2sv, &sv_structures.svrdup);

    for (_chr, sv_list) in &sv_structures.svffus {
        push_softp2sv_entries(softp2sv, sv_list);
    }
    for (_chr, sv_list) in &sv_structures.svrfus {
        push_softp2sv_entries(softp2sv, sv_list);
    }

    for sclips in softp2sv.values_mut() {
        sclips.sort_by(|a, b| b.base.vars_count.cmp(&a.base.vars_count));
    }
}

/// Ported from: StructuralVariantsProcessor.findAllSVs()
/// Java: StructuralVariantsProcessor.java#L117-L145
///
/// Thin orchestrator: findDEL → findINV → findsv → findDELdisc → findINVdisc → findDUPdisc.
/// Order is load-bearing — each routine marks clusters as used, preventing reprocessing.
#[allow(clippy::too_many_arguments)]
pub fn find_all_svs(
    sv_structures: &mut SVStructures,
    non_insertion_variants: &mut PositionMap<VariationMap>,
    insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    max_read_length: i32,
    softp2sv: &mut HashMap<i32, Vec<Sclip>>,
    curseg: &CurrentSegment,
    bams: &Option<Vec<String>>,
    splice: &Option<std::collections::BTreeSet<String>>,
) {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L118-L120
    if instance.conf.y {
        eprintln!("Start Structural Variants: DEL\n");
    }
    find_del(
        sv_structures,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        bams,
        splice,
    );

    // Java: StructuralVariantsProcessor.java#L122-L124
    if instance.conf.y {
        eprintln!("Start Structural Variants: INV\n");
    }
    find_inv(
        sv_structures,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        bams,
        splice,
    );

    rebuild_softp2sv_from_sv_structures(sv_structures, softp2sv);

    // Java: StructuralVariantsProcessor.java#L126-L128
    if instance.conf.y {
        eprintln!("Start Structural Variants\n");
    }
    findsv(
        non_insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        sv_structures,
        softp2sv,
        curseg,
        region,
        max_read_length,
    );

    // Java: StructuralVariantsProcessor.java#L130-L132
    if instance.conf.y {
        eprintln!("Start Structural Variants: DEL discordant pairs only\n");
    }
    find_del_disc(
        sv_structures,
        non_insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        splice,
    );

    // Java: StructuralVariantsProcessor.java#L134-L136
    if instance.conf.y {
        eprintln!("Start Structural Variants: INV discordant pairs only\n");
    }
    find_inv_disc(
        sv_structures,
        non_insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
    );

    // Java: StructuralVariantsProcessor.java#L138-L140
    if instance.conf.y {
        eprintln!("Start Structural Variants: DUP discordant pairs only\n");
    }
    find_dup_disc(
        sv_structures,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        reference,
        reference_resource,
        region,
        max_read_length,
        bams,
        splice,
    );

    rebuild_softp2sv_from_sv_structures(sv_structures, softp2sv);
}

/// Ported from: StructuralVariantsProcessor.adjSNV()
/// Java: StructuralVariantsProcessor.java#L1987-L2040
///
/// Rescues short soft-clipped reads (≤5 bp consensus) as SNVs by adjusting
/// counts on matching existing non-insertion variants.
pub fn adj_snv(
    non_insertion_variants: &mut PositionMap<VariationMap>,
    ref_coverage: &mut CoverageMap,
    soft_clips_5_end: &mut PositionMap<Sclip>,
    soft_clips_3_end: &mut PositionMap<Sclip>,
    reference: &Reference,
) {
    // Java: StructuralVariantsProcessor.java#L1988-L2013 — 5' soft clips
    let keys_5: Vec<i32> = soft_clips_5_end.keys().copied().collect();
    for position in keys_5 {
        let sclip = soft_clips_5_end.get_mut(&position).unwrap();
        // Java: StructuralVariantsProcessor.java#L1992
        if sclip.used {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L1995
        let seq = find_conseq(sclip, 0);
        // Java: StructuralVariantsProcessor.java#L1996
        if seq.len() > 5 {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L1999 — substr(seq, 0, 1)
        let bp = String::from_utf8_lossy(&substr_with_len(seq.as_bytes(), 0, 1)).into_owned();

        // Java: StructuralVariantsProcessor.java#L2001
        let previous_position = position - 1;
        // Java: StructuralVariantsProcessor.java#L2002-L2003
        let has_key = non_insertion_variants
            .get(&previous_position)
            .map_or(false, |vm| vm.entries.contains_key(&bp));
        if has_key {
            // Java: StructuralVariantsProcessor.java#L2004-L2007
            if seq.len() > 1 {
                if is_not_equals(
                    reference.reference_sequences.get(&(position - 2)).copied(),
                    Some(seq.as_bytes()[1]),
                ) {
                    continue;
                }
            }
            // Java: StructuralVariantsProcessor.java#L2009 — adjCnt
            let sclip_base = soft_clips_5_end.get(&position).unwrap().base.clone();
            let variation = non_insertion_variants
                .get_mut(&previous_position)
                .unwrap()
                .entries
                .get_mut(&bp)
                .unwrap();
            adj_cnt(variation, &sclip_base);
            // Java: StructuralVariantsProcessor.java#L2010 — incCnt
            inc_cnt(ref_coverage, previous_position, sclip_base.vars_count);
        }
    }

    // Java: StructuralVariantsProcessor.java#L2014-L2040 — 3' soft clips
    let keys_3: Vec<i32> = soft_clips_3_end.keys().copied().collect();
    for position in keys_3 {
        let sclip = soft_clips_3_end.get_mut(&position).unwrap();
        // Java: StructuralVariantsProcessor.java#L2019
        if sclip.used {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L2022
        let seq = find_conseq(sclip, 0);
        // Java: StructuralVariantsProcessor.java#L2023
        if seq.len() > 5 {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L2026 — substr(seq, 0, 1)
        let bp = String::from_utf8_lossy(&substr_with_len(seq.as_bytes(), 0, 1)).into_owned();
        // Java: StructuralVariantsProcessor.java#L2027-L2028
        let has_key = non_insertion_variants
            .get(&position)
            .map_or(false, |vm| vm.entries.contains_key(&bp));
        if has_key {
            // Java: StructuralVariantsProcessor.java#L2029-L2032
            if seq.len() > 1 {
                if is_not_equals(
                    reference.reference_sequences.get(&(position + 1)).copied(),
                    Some(seq.as_bytes()[1]),
                ) {
                    continue;
                }
            }
            // Java: StructuralVariantsProcessor.java#L2034 — adjCnt
            let sclip_base = soft_clips_3_end.get(&position).unwrap().base.clone();
            let variation = non_insertion_variants
                .get_mut(&position)
                .unwrap()
                .entries
                .get_mut(&bp)
                .unwrap();
            adj_cnt(variation, &sclip_base);
            // Java: StructuralVariantsProcessor.java#L2035 — incCnt
            inc_cnt(ref_coverage, position, sclip_base.vars_count);
        }
    }
}

/// Ported from: StructuralVariantsProcessor.outputClipping()
/// Java: StructuralVariantsProcessor.java#L2042-L2075
///
/// Debug output of remaining unused soft-clipped reads. Only runs when conf.y is true.
pub fn output_clipping(
    soft_clips_5_end: &mut PositionMap<Sclip>,
    soft_clips_3_end: &mut PositionMap<Sclip>,
) {
    let instance = GlobalReadOnlyScope::instance();

    // Java: StructuralVariantsProcessor.java#L2046
    eprintln!("5' Remaining clipping reads");
    let keys_5: Vec<i32> = soft_clips_5_end.keys().copied().collect();
    for position_p in keys_5 {
        let sclip_sc = soft_clips_5_end.get_mut(&position_p).unwrap();
        // Java: StructuralVariantsProcessor.java#L2051
        if sclip_sc.used {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L2054
        if sclip_sc.base.vars_count < instance.conf.minr {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L2057
        let seq = find_conseq(sclip_sc, 0);
        // Java: StructuralVariantsProcessor.java#L2058-L2060
        if !seq.is_empty() && seq.len() > SEED_2 as usize {
            let seq_rev: String = seq.chars().rev().collect();
            let vars_count = soft_clips_5_end.get(&position_p).unwrap().base.vars_count;
            eprintln!("  P: {} Cnt: {} Seq: {}", position_p, vars_count, seq_rev);
        }
    }
    // Java: StructuralVariantsProcessor.java#L2063
    eprintln!("3' Remaining clipping reads");
    let keys_3: Vec<i32> = soft_clips_3_end.keys().copied().collect();
    for position_p in keys_3 {
        let sclip_sc = soft_clips_3_end.get_mut(&position_p).unwrap();
        // Java: StructuralVariantsProcessor.java#L2067
        if sclip_sc.used {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L2070
        if sclip_sc.base.vars_count < instance.conf.minr {
            continue;
        }
        // Java: StructuralVariantsProcessor.java#L2073
        let seq = find_conseq(sclip_sc, 0);
        // Java: StructuralVariantsProcessor.java#L2074-L2075
        if !seq.is_empty() && seq.len() > SEED_2 as usize {
            let vars_count = soft_clips_3_end.get(&position_p).unwrap().base.vars_count;
            eprintln!("  P: {} Cnt: {} Seq: {}", position_p, vars_count, seq);
        }
    }
}

/// Ported from: StructuralVariantsProcessor.process()
/// Java: StructuralVariantsProcessor.java#L91-L120
///
/// Entry point for structural variant processing. Mutates the RealignedVariationData in place:
/// 1. If SV not disabled: findAllSVs()
/// 2. adjSNV() — always runs
/// 3. If debug: outputClipping()
pub fn process(
    data: &mut RealignedVariationData,
    reference: &mut Reference,
    reference_resource: &ReferenceResource,
    region: &Region,
    bams: &Option<Vec<String>>,
    splice: &Option<std::collections::BTreeSet<String>>,
    _prev_non_insertion_variants: &mut PositionMap<VariationMap>,
    _prev_ref_coverage: &mut CoverageMap,
    _prev_soft_clips_3_end: &mut PositionMap<Sclip>,
    _prev_soft_clips_5_end: &mut PositionMap<Sclip>,
    _prev_reference_sequences: &ReferenceSequenceMap,
    _prev_chr: &str,
    _prev_max_read_length: i32,
) {
    let instance = GlobalReadOnlyScope::instance();
    let max_read_length = data.max_read_length.unwrap_or(0);

    // Java: StructuralVariantsProcessor.java#L93-L95
    if !instance.conf.disable_sv {
        find_all_svs(
            &mut data.sv_structures,
            &mut data.non_insertion_variants,
            &mut data.insertion_variants,
            &mut data.ref_coverage,
            &mut data.soft_clips_3_end,
            &mut data.soft_clips_5_end,
            reference,
            reference_resource,
            region,
            max_read_length,
            &mut data.softp2sv,
            &data.curseg,
            bams,
            splice,
        );
    }

    // Java: StructuralVariantsProcessor.java#L96
    adj_snv(
        &mut data.non_insertion_variants,
        &mut data.ref_coverage,
        &mut data.soft_clips_5_end,
        &mut data.soft_clips_3_end,
        reference,
    );

    // Java: StructuralVariantsProcessor.java#L97-L99
    if instance.conf.y {
        output_clipping(&mut data.soft_clips_5_end, &mut data.soft_clips_3_end);
        eprintln!("TIME: Finish realign");
    }

    // Java: StructuralVariantsProcessor.java#L101-L106 — JSONL writer skipped (diagnostic only)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Configuration;
    use std::collections::HashMap;

    fn init_test_scope() {
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
    fn test_is_overlap_no_overlap_start_after_end() {
        // start1 >= end2
        assert!(!is_overlap(100, 200, 50, 80, 150));
    }

    #[test]
    fn test_is_overlap_no_overlap_start2_after_end1() {
        // start2 >= end1
        assert!(!is_overlap(50, 80, 100, 200, 150));
    }

    #[test]
    fn test_is_overlap_full_containment() {
        // Full overlap: inner segment > 75% of both
        assert!(is_overlap(100, 300, 110, 290, 150));
    }

    #[test]
    fn test_is_overlap_proximity_check() {
        // Partial overlap within 3*rlen proximity
        assert!(is_overlap(100, 200, 150, 300, 150));
    }

    #[test]
    fn test_is_overlap_degenerate_interval() {
        // Degenerate interval (end1 == start1) — skips fraction check,
        // falls through to proximity check
        assert!(is_overlap(100, 100, 50, 200, 100));
    }

    #[test]
    fn test_is_overlap_touching_not_overlapping() {
        // start1 == end2 means no overlap
        assert!(!is_overlap(100, 200, 50, 100, 10));
    }

    #[test]
    fn test_pairs_data_creation() {
        let pd = PairsData::new(10, 1.5, 2.5, 3.5, 0.5);
        assert_eq!(pd.pairs, 10);
        assert_eq!(pd.pmean, 1.5);
        assert_eq!(pd.qmean, 2.5);
        assert_eq!(pd.q_mean, 3.5);
        assert_eq!(pd.nm, 0.5);
    }

    #[test]
    fn test_fill_and_sort_tmp_sv_filters_by_segment() {
        let curseg = CurrentSegment::new("chr1", 100, 200);
        let mut entries = PositionMap::default();

        let mut sclip1 = Sclip::default();
        sclip1.base.vars_count = 5;
        entries.insert(150, sclip1);

        let mut sclip2 = Sclip::default();
        sclip2.base.vars_count = 10;
        entries.insert(50, sclip2); // outside segment

        let mut sclip3 = Sclip::default();
        sclip3.base.vars_count = 8;
        entries.insert(180, sclip3);

        let mut sclip4 = Sclip::default();
        sclip4.base.vars_count = 3;
        sclip4.used = true; // filtered out
        entries.insert(160, sclip4);

        let result = fill_and_sort_tmp_sv(&entries, &curseg);
        assert_eq!(result.len(), 2);
        // Sorted by count descending
        assert_eq!(result[0].count, 8);
        assert_eq!(result[1].count, 5);
    }

    #[test]
    fn test_fill_and_sort_tmp_sv_tiebreaks_by_position() {
        let curseg = CurrentSegment::new("chr1", 100, 200);
        let mut entries = PositionMap::default();

        let mut later = Sclip::default();
        later.base.vars_count = 7;
        entries.insert(180, later);

        let mut earlier = Sclip::default();
        earlier.base.vars_count = 7;
        entries.insert(120, earlier);

        let result = fill_and_sort_tmp_sv(&entries, &curseg);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].count, 7);
        assert_eq!(result[0].position, 120);
        assert_eq!(result[1].position, 180);
    }

    #[test]
    fn test_mark_sv_marks_overlapping_clusters() {
        init_test_scope();
        let mut cluster = vec![
            {
                let mut sc = Sclip::default();
                sc.start = 100;
                sc.end = 200;
                sc.mstart = 300;
                sc.mend = 400;
                sc.base.vars_count = 5;
                sc
            },
            {
                let mut sc = Sclip::default();
                sc.start = 500;
                sc.end = 600;
                sc.mstart = 700;
                sc.mend = 800;
                sc.base.vars_count = 3;
                sc
            },
        ];
        // start < mstart → inner: (end=200, mstart=300)
        // isOverlap(150, 350, 200, 300, 150) → check
        let result = mark_sv(150, 350, &mut [&mut cluster], 150);
        // First cluster should be marked
        assert!(cluster[0].used);
        // Second cluster should NOT be marked (different region)
        assert!(!cluster[1].used);
        assert!(result.1 >= 1); // cnt >= 1
    }

    #[test]
    fn test_mark_dup_sv_uses_outer_coordinates() {
        init_test_scope();
        let mut cluster = vec![{
            let mut sc = Sclip::default();
            sc.start = 100;
            sc.end = 200;
            sc.mstart = 300;
            sc.mend = 400;
            sc.base.vars_count = 5;
            sc
        }];
        // start < mstart → outer: (start=100, mend=400)
        // isOverlap(50, 450, 100, 400, 150)
        let result = mark_dup_sv(50, 450, &mut [&mut cluster], 150);
        assert!(cluster[0].used);
        assert_eq!(result.0, 1); // cnt
        assert_eq!(result.1, 5); // pairs
    }
}
