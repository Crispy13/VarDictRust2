/// Ported from: CigarModifier.java:L1-L787
/// CIGAR string normalization before variant detection.
use std::collections::HashSet;

use crate::config::{LOWQUAL, SEED_2};
use crate::data::{ModifiedCigar, Region};
use crate::patterns::*;
use crate::reference::{Reference, ReferenceSeedMap, ReferenceSequenceMap};
use crate::utils::{complement_sequence, global_find, reverse_sequence, substr, substr_with_len};
use crate::variations::{
    is_has_and_equals_base, is_has_and_equals_str, is_has_and_not_equals_base,
    is_has_and_not_equals_str,
};

use crate::scope::GlobalReadOnlyScope;

/// Ported from: CigarModifier.java:L27-L37 (instance fields)
struct CigarModifierState<'a> {
    position: i32,
    cigar_str: String,
    original_cigar: String,
    query_sequence: String,
    query_quality: String,
    reference: &'a ReferenceSequenceMap,
    seed: &'a ReferenceSeedMap,
    indel: i32,
    max_read_length: i32,
    region: Region,
}

/// Ported from: CigarModifier.java:L39-L51 (constructor)
impl<'a> CigarModifierState<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        position: i32,
        cigar_str: String,
        query_sequence: String,
        query_quality: String,
        reference_data: &'a Reference,
        indel: i32,
        region: &Region,
        max_read_length: i32,
    ) -> Self {
        Self {
            position,
            original_cigar: cigar_str.clone(),
            cigar_str,
            query_sequence,
            query_quality,
            reference: &reference_data.reference_sequences,
            seed: &reference_data.seed,
            indel,
            max_read_length,
            region: region.clone(),
        }
    }
}

/// Ported from: CigarModifier.java:L57-L248
/// Master method: applies all CIGAR transformations in sequence.
/// Wrapped in catch_unwind to replicate Java's try/catch that returns partially-modified state.
#[allow(clippy::too_many_arguments)]
pub fn modify_cigar(
    position: i32,
    cigar_str: &str,
    query_sequence: &str,
    query_quality: &str,
    reference_data: &Reference,
    indel: i32,
    region: &Region,
    max_read_length: i32,
) -> ModifiedCigar {
    let mut state = CigarModifierState::new(
        position,
        cigar_str.to_string(),
        query_sequence.to_string(),
        query_quality.to_string(),
        reference_data,
        indel,
        region,
        max_read_length,
    );

    // Trap T5: Exception swallowing — wrap in catch_unwind to replicate Java try/catch
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        modify_cigar_inner(&mut state);
    }));

    if let Err(_e) = result {
        // Ported from: CigarModifier.java:L245-L247 — catch block logs and returns partially-modified state
        eprintln!(
            "WARNING: Exception in CigarModifier at {} {} for region {}",
            state.position,
            state.original_cigar,
            state.region.print_region()
        );
    }

    ModifiedCigar::new(
        state.position,
        state.cigar_str,
        state.query_sequence,
        state.query_quality,
    )
}

/// Ported from: CigarModifier.java:L57-L248 (core modifyCigar body)
#[allow(clippy::collapsible_if)]
fn modify_cigar_inner(s: &mut CigarModifierState<'_>) {
    // Trap T6: flag loop control — flag starts true
    let mut flag = true;

    // Phase 1: Strip boundary deletions (L63-L72)
    if let Some(caps) = BEGIN_NUMBER_D.captures(&s.cigar_str) {
        let n: i32 = caps[1].parse().unwrap();
        s.position += n;
        s.cigar_str = BEGIN_NUMBER_D.replace(&s.cigar_str, "").to_string();
    }
    if END_NUMBER_D.is_match(&s.cigar_str) {
        s.cigar_str = END_NUMBER_D.replace(&s.cigar_str, "").to_string();
    }

    // Phase 2: Convert boundary insertions to soft clips (L73-L82)
    if let Some(caps) = BEGIN_NUMBER_I.captures(&s.cigar_str) {
        let n: i32 = caps[1].parse().unwrap();
        let replacement = format!("{}S", n);
        s.cigar_str = BEGIN_NUMBER_I
            .replace(&s.cigar_str, replacement.as_str())
            .to_string();
    }
    if let Some(caps) = END_NUMBER_I.captures(&s.cigar_str) {
        let n: i32 = caps[1].parse().unwrap();
        let replacement = format!("{}S", n);
        s.cigar_str = END_NUMBER_I
            .replace(&s.cigar_str, replacement.as_str())
            .to_string();
    }

    // Phase 3: Chimeric soft-clip detection (L83-L131)
    // Trap T9: 5' and 3' are if/else if — only one fires
    let sc5_caps = SA_CIGAR_D_S_5clip_GROUP.captures(&s.cigar_str);
    let sc3_caps = SA_CIGAR_D_S_3clip_GROUP.captures(&s.cigar_str);

    if let Some(caps5) = sc5_caps {
        let cigar_element: i32 = caps5[1].parse().unwrap();
        let (chimeric, debug_y) =
            GlobalReadOnlyScope::with_instance(|scope| (scope.conf.chimeric, scope.conf.y));
        if !chimeric && cigar_element >= SEED_2 {
            let sseq = substr_with_len(s.query_sequence.as_bytes(), 0, cigar_element);
            let reversed = reverse_sequence(&sseq);
            let sequence = complement_sequence(&reversed);
            let sequence_str = String::from_utf8_lossy(&sequence).to_string();
            if (SEED_2 as usize) <= sequence_str.len() {
                let reverse_complemented_seed = &sequence_str[..SEED_2 as usize];

                if let Some(positions) = s.seed.get(reverse_complemented_seed) {
                    if positions.len() == 1
                        && (s.position - positions[0]).abs() < 2 * s.max_read_length
                    {
                        // Match Java jregex replacer semantics: repeatedly strip leading
                        // soft-clip runs until the anchored pattern no longer matches.
                        while SA_CIGAR_D_S_5clip_GROUP_Repl.is_match(&s.cigar_str) {
                            s.cigar_str = SA_CIGAR_D_S_5clip_GROUP_Repl
                                .replace(&s.cigar_str, "")
                                .to_string();
                        }
                        s.query_sequence = String::from_utf8_lossy(&substr(
                            s.query_sequence.as_bytes(),
                            cigar_element,
                        ))
                        .to_string();
                        s.query_quality = String::from_utf8_lossy(&substr(
                            s.query_quality.as_bytes(),
                            cigar_element,
                        ))
                        .to_string();
                        if debug_y {
                            eprintln!(
                                "{} at 5' is a chimeric at {} by SEED {}",
                                sequence_str, s.position, SEED_2
                            );
                        }
                    }
                }
            }
        }
    } else if let Some(caps3) = sc3_caps {
        let cigar_element: i32 = caps3[1].parse().unwrap();
        let (chimeric, debug_y) =
            GlobalReadOnlyScope::with_instance(|scope| (scope.conf.chimeric, scope.conf.y));
        if !chimeric && cigar_element >= SEED_2 {
            // Trap T7b: substr with negative index for 3' end
            let sseq = substr_with_len(s.query_sequence.as_bytes(), -cigar_element, cigar_element);
            let reversed = reverse_sequence(&sseq);
            let sequence = complement_sequence(&reversed);
            let sequence_str = String::from_utf8_lossy(&sequence).to_string();
            // Trap T8c: last SEED_2 chars for 3'
            let reverse_complemented_seed =
                String::from_utf8_lossy(&substr_with_len(sequence_str.as_bytes(), -SEED_2, SEED_2))
                    .to_string();

            if let Some(positions) = s.seed.get(&reverse_complemented_seed) {
                if positions.len() == 1 && (s.position - positions[0]).abs() < 2 * s.max_read_length
                {
                    // Match Java jregex replacer semantics: repeatedly strip trailing
                    // soft-clip runs until the anchored pattern no longer matches.
                    while SA_CIGAR_D_S_3clip_GROUP_Repl.is_match(&s.cigar_str) {
                        s.cigar_str = SA_CIGAR_D_S_3clip_GROUP_Repl
                            .replace(&s.cigar_str, "")
                            .to_string();
                    }
                    let seq_len = s.query_sequence.len() as i32;
                    s.query_sequence = String::from_utf8_lossy(&substr_with_len(
                        s.query_sequence.as_bytes(),
                        0,
                        seq_len - cigar_element,
                    ))
                    .to_string();
                    let qual_len = s.query_quality.len() as i32;
                    s.query_quality = String::from_utf8_lossy(&substr_with_len(
                        s.query_quality.as_bytes(),
                        0,
                        qual_len - cigar_element,
                    ))
                    .to_string();
                    if debug_y {
                        eprintln!(
                            "{} at 3' is a chimeric at {} by SEED {}",
                            sequence_str, s.position, SEED_2
                        );
                    }
                }
            }
        }
    }

    // Phase 4: Iterative indel normalization loop (L133-L229)
    // Trap T6: flag starts true, set to false at iteration start, re-set by any matching sub-check
    // Trap T1: Clause ordering is critical — sequential if/else chain, first match wins
    while flag && s.indel > 0 {
        flag = false;

        // 4a: Leading S+I/D collapse (L136-L145)
        if let Some(caps) = BEGIN_NUMBER_S_NUMBER_IorD.captures(&s.cigar_str) {
            let g1: i32 = caps[1].parse().unwrap();
            let g2: i32 = caps[2].parse().unwrap();
            let g3 = &caps[3];
            let tslen = g1 + if g3 == "I" { g2 } else { 0 };
            s.position += if g3 == "D" { g2 } else { 0 };
            let replacement = format!("{}S", tslen);
            s.cigar_str = BEGIN_NUMBER_S_NUMBER_IorD
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
            flag = true;
        }

        // 4b: Trailing I/D+S collapse (L146-L153)
        if let Some(caps) = NUMBER_IorD_NUMBER_S_END.captures(&s.cigar_str) {
            let g1: i32 = caps[1].parse().unwrap();
            let g2 = &caps[2];
            let g3: i32 = caps[3].parse().unwrap();
            let tslen = g3 + if g2 == "I" { g1 } else { 0 };
            let replacement = format!("{}S", tslen);
            s.cigar_str = NUMBER_IorD_NUMBER_S_END
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
            flag = true;
        }

        // 4c: Leading S+short-M+I/D collapse (L154-L165)
        if let Some(caps) = BEGIN_NUMBER_S_NUMBER_M_NUMBER_IorD.captures(&s.cigar_str) {
            let g1: i32 = caps[1].parse().unwrap();
            let tmid: i32 = caps[2].parse().unwrap();
            let g3: i32 = caps[3].parse().unwrap();
            let g4 = &caps[4];
            if tmid <= 10 {
                let tslen = g1 + tmid + if g4 == "I" { g3 } else { 0 };
                s.position += tmid + if g4 == "D" { g3 } else { 0 };
                let replacement = format!("{}S", tslen);
                s.cigar_str = BEGIN_NUMBER_S_NUMBER_M_NUMBER_IorD
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
                flag = true;
            }
        }

        // 4d: Trailing I/D+short-M+S collapse (L166-L176)
        if let Some(caps) = NUMBER_IorD_NUMBER_M_NUMBER_S_END.captures(&s.cigar_str) {
            let g1: i32 = caps[1].parse().unwrap();
            let g2 = &caps[2];
            let tmid: i32 = caps[3].parse().unwrap();
            let g4: i32 = caps[4].parse().unwrap();
            if tmid <= 10 {
                let tslen = g4 + tmid + if g2 == "I" { g1 } else { 0 };
                let replacement = format!("{}S", tslen);
                s.cigar_str = NUMBER_IorD_NUMBER_M_NUMBER_S_END
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
                flag = true;
            }
        }

        // 4e: Leading short-M+I/D+M → soft-clip (L180-L184)
        let cigar_snap = s.cigar_str.clone();
        if let Some(caps) = BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M.captures(&cigar_snap) {
            flag = begin_digit_m_number_i_or_d_number_m(s, &caps);
        }

        // 4f: Trailing I/D+short-M → soft-clip (L185-L191)
        // Trap T4_mismatch: match uses DIGIT (single digit) but replace uses NUMBER (multi-digit)
        if let Some(caps) = NUMBER_IorD_DIGIT_M_END.captures(&s.cigar_str) {
            let g1: i32 = caps[1].parse().unwrap();
            let g2 = &caps[2];
            let tmid: i32 = caps[3].parse().unwrap();
            let tslen = tmid + if g2 == "I" { g1 } else { 0 };
            let replacement = format!("{}S", tslen);
            s.cigar_str = NUMBER_IorD_NUMBER_M_END
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
            flag = true;
        }

        // 4g: Complex indel merging (L194-L210)
        // Trap T1: two_deletions_insertion checked first, then three_deletions, then three_indels
        let cigar_snap2 = s.cigar_str.clone();
        let mm_two_del_ins = D_M_D_DD_M_D_I_D_M_D_DD.captures(&cigar_snap2);
        let mm_three_del = threeDeletionsPattern.captures(&cigar_snap2);
        let mm_three_indel = threeIndelsPattern.captures(&cigar_snap2);

        if let Some(caps) = mm_two_del_ins {
            flag = two_deletions_insertion_to_complex(s, &caps, flag);
        } else if let Some(caps) = mm_three_del {
            flag = three_deletions(s, &caps, flag);
        } else if let Some(caps) = mm_three_indel {
            flag = three_indels(s, &caps, flag);
        }

        // 4h: D-M-D/I merge (L208-L212)
        let cigar_snap3 = s.cigar_str.clone();
        if let Some(caps) = DIG_D_DIG_M_DIG_DI_DIGI.captures(&cigar_snap3) {
            flag = combine_to_close_to_correct(s, &caps, flag);
        }

        // 4i: Non-D-prefix I-M-D/I merge (L214-L217)
        // Trap T3: match uses NOTDIG pattern but replace uses DIG pattern
        let cigar_snap4 = s.cigar_str.clone();
        if let Some(caps) = NOTDIG_DIG_I_DIG_M_DIG_DI_DIGI.captures(&cigar_snap4) {
            let g1 = &caps[1];
            if g1 != "D" && g1 != "H" {
                flag = combine_to_close_to_one(s, &caps, flag);
            }
        }

        // 4j: Adjacent D+D or I+I merging (L218-L227)
        if let Some(caps) = DIG_D_DIG_D.captures(&s.cigar_str) {
            let dlen: i32 = caps[1].parse::<i32>().unwrap() + caps[2].parse::<i32>().unwrap();
            let replacement = format!("{}D", dlen);
            s.cigar_str = DIG_D_DIG_D
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
            flag = true;
        }
        if let Some(caps) = DIG_I_DIG_I.captures(&s.cigar_str) {
            let ilen: i32 = caps[1].parse::<i32>().unwrap() + caps[2].parse::<i32>().unwrap();
            let replacement = format!("{}I", ilen);
            s.cigar_str = DIG_I_DIG_I
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
            flag = true;
        }
    }

    // Phase 5: Post-loop soft-clip realignment (L231-L247)
    if let Some(caps) = ANY_NUMBER_M_NUMBER_S_END.captures(&s.cigar_str) {
        let ov5 = caps[1].to_string();
        let mch: i32 = caps[2].parse().unwrap();
        let soft: i32 = caps[3].parse().unwrap();
        capture_mis_softly_ms(s, &ov5, mch, soft);
    } else if let Some(caps) = BEGIN_ANY_DIG_M_END.captures(&s.cigar_str) {
        let ov5 = caps[1].to_string();
        let mch: i32 = caps[2].parse().unwrap();
        capture_mis_softly_3_mismatches(s, &ov5, mch);
    }

    // Phase 6: 5' soft-clip realignment (L248-L253)
    if let Some(caps) = DIG_S_DIG_M.captures(&s.cigar_str) {
        let soft: i32 = caps[1].parse().unwrap();
        let mch: i32 = caps[2].parse().unwrap();
        combine_dig_s_dig_m(s, soft, mch);
    } else if let Some(caps) = BEGIN_DIG_M.captures(&s.cigar_str) {
        let mch: i32 = caps[1].parse().unwrap();
        combine_begin_dig_m(s, mch);
    }
}

/// Ported from: CigarModifier.java:L255-L278 (combineBeginDigM)
/// Convert leading mismatches to soft-clip.
fn combine_begin_dig_m(s: &mut CigarModifierState<'_>, mut mch: i32) {
    let mut rn: i32 = 0;
    let mut rrn: i32 = 0;
    let mut rmch: i32 = 0;
    while rrn < mch && rn < mch {
        if !s.reference.contains_key(&(s.position + rrn)) {
            break;
        }
        if is_has_and_not_equals_str(&s.reference, s.position + rrn, &s.query_sequence, rrn) {
            rn = rrn + 1;
            rmch = 0;
        } else if is_has_and_equals_str(&s.reference, s.position + rrn, &s.query_sequence, rrn) {
            rmch += 1;
        }
        rrn += 1;
        if rmch >= 3 {
            break;
        }
    }
    // Trap T13: range guard rn <= 3
    if rn > 0 && rn <= 3 {
        mch -= rn;
        let replacement = format!("{}S{}M", rn, mch);
        s.cigar_str = BEGIN_DIG_M
            .replace(&s.cigar_str, replacement.as_str())
            .to_string();
        s.position += rn;
    }
}

/// Ported from: CigarModifier.java:L284-L358 (combineDigSDigM)
/// Adjust the boundary between a leading soft-clip and matched region.
fn combine_dig_s_dig_m(s: &mut CigarModifierState<'_>, mut soft: i32, mut mch: i32) {
    let mut rn: i32 = 0;
    let mut rn_set: HashSet<u8> = HashSet::new();

    // Part A: Extend M leftward into S (L290-L307)
    // Trap T7: quality threshold is > LOWQUAL (strict >)
    while rn < soft
        && is_has_and_equals_str(
            &s.reference,
            s.position - rn - 1,
            &s.query_sequence,
            soft - rn - 1,
        )
        && (s.query_quality.as_bytes()[(soft - rn - 1) as usize] as i32 - 33) > LOWQUAL
    {
        rn += 1;
    }
    if rn > 0 {
        mch += rn;
        soft -= rn;
        if soft > 0 {
            let replacement = format!("{}S{}M", soft, mch);
            s.cigar_str = DIG_S_DIG_M
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
        } else {
            let replacement = format!("{}M", mch);
            s.cigar_str = DIG_S_DIG_M
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
        }
        s.position -= rn;
        rn = 0;
    }

    // Part B: Extended leftward extension with gap tolerance (L308-L327)
    if soft > 0 {
        // Trap T15: re-creates the matcher each time against current cigarStr
        while rn + 1 < soft
            && is_has_and_equals_str(
                &s.reference,
                s.position - rn - 2,
                &s.query_sequence,
                soft - rn - 2,
            )
            && (s.query_quality.as_bytes()[(soft - rn - 2) as usize] as i32 - 33) > LOWQUAL
        {
            rn += 1;
            // Trap T4: post-increment rn — rn is already incremented here
            if let Some(&base) = s.reference.get(&(s.position - rn - 2)) {
                rn_set.insert(base);
            }
        }
        let rn_nt = rn_set.len();
        // Trap T6_homo: homopolymer guard — (rn > 4 && rn_nt > 1) OR immediate boundary match
        if (rn > 4 && rn_nt > 1)
            || is_has_and_equals_str(&s.reference, s.position - 1, &s.query_sequence, soft - 1)
        {
            mch += rn + 1;
            soft -= rn + 1;
            // Trap T15: re-create matcher against current cigarStr
            if soft > 0 {
                let replacement = format!("{}S{}M", soft, mch);
                s.cigar_str = DIG_S_DIG_M
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
            } else {
                let replacement = format!("{}M", mch);
                s.cigar_str = DIG_S_DIG_M
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
            }
            s.position -= rn + 1;
        }

        // Part C: Extend S rightward for mismatches (L328-L357)
        if rn == 0 {
            let mut rrn: i32 = 0;
            let mut rmch: i32 = 0;
            while rrn < mch && rn < mch {
                if !s.reference.contains_key(&(s.position + rrn)) {
                    break;
                }
                if is_has_and_not_equals_str(
                    &s.reference,
                    s.position + rrn,
                    &s.query_sequence,
                    soft + rrn,
                ) {
                    rn = rrn + 1;
                    rmch = 0;
                } else if is_has_and_equals_str(
                    &s.reference,
                    s.position + rrn,
                    &s.query_sequence,
                    soft + rrn,
                ) {
                    rmch += 1;
                }
                rrn += 1;
                if rmch >= 3 {
                    break;
                }
            }
            // Trap T13: no upper bound on rn, just rn > 0 && rn < mch
            if rn > 0 && rn < mch {
                soft += rn;
                mch -= rn;
                let replacement = format!("{}S{}M", soft, mch);
                s.cigar_str = DIG_S_DIG_M
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
                s.position += rn;
            }
        }
    }
}

/// Helper: compute reference offset from CIGAR prefix using ALIGNED_LENGTH_MND pattern
fn compute_ref_offset(ov5: &str) -> i32 {
    if ov5.is_empty() {
        return 0;
    }
    let matches = global_find(&ALIGNED_LENGTH_MND, ov5);
    matches.iter().map(|s| s.parse::<i32>().unwrap_or(0)).sum()
}

/// Helper: compute read offset from CIGAR prefix using SOFT_CLIPPED pattern
fn compute_read_offset(ov5: &str) -> i32 {
    if ov5.is_empty() {
        return 0;
    }
    let matches = global_find(&SOFT_CLIPPED, ov5);
    matches.iter().map(|s| s.parse::<i32>().unwrap_or(0)).sum()
}

/// Ported from: CigarModifier.java:L364-L399 (captureMisSoftly3Mismatches)
/// For CIGARs ending with M only: convert trailing mismatches to soft-clip.
fn capture_mis_softly_3_mismatches(s: &mut CigarModifierState<'_>, ov5: &str, mut mch: i32) {
    let mut refoff = s.position + mch;
    let mut rdoff = mch;
    if !ov5.is_empty() {
        refoff += compute_ref_offset(ov5);
        rdoff += compute_read_offset(ov5);
    }
    let mut rn: i32 = 0;
    let mut rrn: i32 = 0;
    let mut rmch: i32 = 0;
    while rrn < mch && rn < mch {
        if !s.reference.contains_key(&(refoff - rrn - 1)) {
            break;
        }
        if rrn < rdoff
            && is_has_and_not_equals_str(
                &s.reference,
                refoff - rrn - 1,
                &s.query_sequence,
                rdoff - rrn - 1,
            )
        {
            rn = rrn + 1;
            rmch = 0;
        } else if rrn < rdoff
            && is_has_and_equals_str(
                &s.reference,
                refoff - rrn - 1,
                &s.query_sequence,
                rdoff - rrn - 1,
            )
        {
            rmch += 1;
        }
        rrn += 1;
        if rmch >= 3 {
            break;
        }
    }
    mch -= rn;
    // Trap T13: range guard rn <= 3
    if rn > 0 && rn <= 3 {
        let replacement = format!("{}M{}S", mch, rn);
        s.cigar_str = DIG_M_END
            .replace(&s.cigar_str, replacement.as_str())
            .to_string();
    }
}

/// Ported from: CigarModifier.java:L405-L486 (captureMisSoftlyMS)
/// For CIGARs with trailing M-S: extend or retract M↔S boundary.
fn capture_mis_softly_ms(s: &mut CigarModifierState<'_>, ov5: &str, mut mch: i32, mut soft: i32) {
    // Part A: Compute offsets
    let mut refoff = s.position + mch;
    let mut rdoff = mch;
    if !ov5.is_empty() {
        refoff += compute_ref_offset(ov5);
        rdoff += compute_read_offset(ov5);
    }

    // Part B: Extend M rightward (L418-L431)
    let mut rn: i32 = 0;
    let mut rn_set: HashSet<u8> = HashSet::new();
    while rn < soft
        && is_has_and_equals_str(&s.reference, refoff + rn, &s.query_sequence, rdoff + rn)
        && (s.query_quality.as_bytes()[(rdoff + rn) as usize] as i32 - 33) > LOWQUAL
    {
        rn += 1;
    }
    if rn > 0 {
        mch += rn;
        soft -= rn;
        if soft > 0 {
            let replacement = format!("{}M{}S", mch, soft);
            s.cigar_str = DIG_M_DIG_S_END
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
        } else {
            let replacement = format!("{}M", mch);
            s.cigar_str = DIG_M_DIG_S_END
                .replace(&s.cigar_str, replacement.as_str())
                .to_string();
        }
        rn = 0;
    }

    // Part C: Extended rightward with gap tolerance (L433-L451)
    if soft > 0 {
        // Trap T4: post-increment rn indexing
        while rn + 1 < soft
            && is_has_and_equals_str(
                &s.reference,
                refoff + rn + 1,
                &s.query_sequence,
                rdoff + rn + 1,
            )
            && (s.query_quality.as_bytes()[(rdoff + rn + 1) as usize] as i32 - 33) > LOWQUAL
        {
            rn += 1;
            // Trap T4: post-increment rn — uses already-incremented value
            if let Some(&base) = s.reference.get(&(refoff + rn + 1)) {
                rn_set.insert(base);
            }
        }
        let rn_nt = rn_set.len();
        // Note: captureMisSoftlyMS Part C does NOT have the OR isHasAndEquals boundary condition
        // that combineDigSDigM Part B has. This asymmetry is intentional.
        if rn > 4 && rn_nt > 1 {
            mch += rn + 1;
            soft -= rn + 1;
            if soft > 0 {
                let replacement = format!("{}M{}S", mch, soft);
                s.cigar_str = DIG_M_DIG_S_END
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
            } else {
                let replacement = format!("{}M", mch);
                s.cigar_str = DIG_M_DIG_S_END
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
            }
        }

        // Part D: Retract M into S for mismatches (L452-L480)
        // Trap T19: rn was last set by the if (rn == 0) guard
        if rn == 0 {
            let mut rrn: i32 = 0;
            let mut rmch: i32 = 0;
            while rrn < mch && rn < mch {
                if !s.reference.contains_key(&(refoff - rrn - 1)) {
                    break;
                }
                if rrn < rdoff
                    && is_has_and_not_equals_str(
                        &s.reference,
                        refoff - rrn - 1,
                        &s.query_sequence,
                        rdoff - rrn - 1,
                    )
                {
                    rn = rrn + 1;
                    rmch = 0;
                } else if rrn < rdoff
                    && is_has_and_equals_str(
                        &s.reference,
                        refoff - rrn - 1,
                        &s.query_sequence,
                        rdoff - rrn - 1,
                    )
                {
                    rmch += 1;
                }
                rrn += 1;
                if rmch >= 3 {
                    break;
                }
            }
            if rn > 0 && rn < mch {
                soft += rn;
                mch -= rn;
                let replacement = format!("{}M{}S", mch, soft);
                s.cigar_str = DIG_M_DIG_S_END
                    .replace(&s.cigar_str, replacement.as_str())
                    .to_string();
            }
        }
    }
}

/// Ported from: CigarModifier.java:L494-L527 (combineToCloseToOne)
/// Merge I-M-D/I complex where internal M ≤ 15bp into single D+I.
/// Trap T3: match pattern includes leading non-digit, replace pattern excludes it.
fn combine_to_close_to_one(
    s: &mut CigarModifierState<'_>,
    caps: &regex::Captures,
    mut flag: bool,
) -> bool {
    let op = &caps[5];
    let g2: i32 = caps[2].parse().unwrap();
    let g3: i32 = caps[3].parse().unwrap();
    let g4: i32 = caps[4].parse().unwrap();

    if g3 <= 15 {
        let mut dlen = g3;
        let mut ilen = g2 + g3;
        if op == "I" {
            ilen += g4;
        } else if op == "D" {
            dlen += g4;
            if let Some(g6) = caps.get(6) {
                let istr = g6.as_str();
                // Strip trailing 'I' to get the number
                ilen += istr[..istr.len() - 1].parse::<i32>().unwrap_or(0);
            }
        }
        // Trap T3: replace uses DIG_I_DIG_M_DIG_DI_DIGI (without leading non-digit)
        let replacement = format!("{}D{}I", dlen, ilen);
        s.cigar_str = DIG_I_DIG_M_DIG_DI_DIGI
            .replace(&s.cigar_str, replacement.as_str())
            .to_string();
        flag = true;
    }
    flag
}

/// Ported from: CigarModifier.java:L535-L569 (combineToCloseToCorrect)
/// Merge D-M-D/I complex where internal M ≤ 15bp into single D+I.
fn combine_to_close_to_correct(
    s: &mut CigarModifierState<'_>,
    caps: &regex::Captures,
    mut flag: bool,
) -> bool {
    let g1: i32 = caps[1].parse().unwrap();
    let g2: i32 = caps[2].parse().unwrap();
    let g3: i32 = caps[3].parse().unwrap();
    if g2 <= 15 {
        let op = &caps[4];
        let mut dlen = g1 + g2;
        let mut ilen = g2;
        if op == "I" {
            ilen += g3;
        } else if op == "D" {
            dlen += g3;
            if let Some(g5) = caps.get(5) {
                let istr = g5.as_str();
                ilen += istr[..istr.len() - 1].parse::<i32>().unwrap_or(0);
            }
        }
        let replacement = format!("{}D{}I", dlen, ilen);
        s.cigar_str = DIG_D_DIG_M_DIG_DI_DIGI
            .replace(&s.cigar_str, replacement.as_str())
            .to_string();
        flag = true;
    }
    flag
}

/// Ported from: CigarModifier.java:L578-L642 (threeIndels)
/// Merge 3-indel complex M-D/I-M-D/I-M-D/I-M into simplified D+I+M form.
fn three_indels(s: &mut CigarModifierState<'_>, caps: &regex::Captures, mut flag: bool) -> bool {
    let mut tslen: i32 = caps[5].parse::<i32>().unwrap() + caps[8].parse::<i32>().unwrap();
    if &caps[4] == "I" {
        tslen += caps[3].parse::<i32>().unwrap();
    }
    if &caps[7] == "I" {
        tslen += caps[6].parse::<i32>().unwrap();
    }
    if &caps[10] == "I" {
        tslen += caps[9].parse::<i32>().unwrap();
    }

    let mut dlen: i32 = caps[5].parse::<i32>().unwrap() + caps[8].parse::<i32>().unwrap();
    if &caps[4] == "D" {
        dlen += caps[3].parse::<i32>().unwrap();
    }
    if &caps[7] == "D" {
        dlen += caps[6].parse::<i32>().unwrap();
    }
    if &caps[10] == "D" {
        dlen += caps[9].parse::<i32>().unwrap();
    }

    let mid: i32 = caps[5].parse::<i32>().unwrap() + caps[8].parse::<i32>().unwrap();
    let ov5 = &caps[1];
    let g2: i32 = caps[2].parse().unwrap();
    let mut refoff = s.position + g2;
    let mut rdoff = g2;
    let mut rdoff_upper = g2;
    let mut rm: i32 = caps[11].parse().unwrap();

    if !ov5.is_empty() {
        refoff += compute_ref_offset(ov5);
        rdoff += compute_read_offset(ov5);
    }

    let mut rn: i32 = 0;
    while (rdoff + rn) < s.query_sequence.len() as i32
        && is_has_and_equals_base(
            s.query_sequence.as_bytes()[(rdoff + rn) as usize],
            &s.reference,
            refoff + rn,
        )
    {
        rn += 1;
    }
    rdoff_upper += rn;
    dlen -= rn;
    tslen -= rn;

    let new_cigar_str;
    if tslen <= 0 {
        dlen -= tslen;
        rm += tslen;
        if dlen == 0 {
            rdoff_upper += rm;
            new_cigar_str = format!("{}M", rdoff_upper);
        } else if dlen < 0 {
            tslen = -dlen;
            rm += dlen;
            if rm < 0 {
                rdoff_upper += rm;
                new_cigar_str = format!("{}M{}I", rdoff_upper, tslen);
            } else {
                new_cigar_str = format!("{}M{}I{}M", rdoff_upper, tslen, rm);
            }
        } else {
            new_cigar_str = format!("{}M{}D{}M", rdoff_upper, dlen, rm);
        }
    } else if dlen == 0 {
        new_cigar_str = format!("{}M{}I{}M", rdoff_upper, tslen, rm);
    } else if dlen < 0 {
        rm += dlen;
        new_cigar_str = format!("{}M{}I{}M", rdoff_upper, tslen, rm);
    } else {
        new_cigar_str = format!("{}M{}D{}I{}M", rdoff_upper, dlen, tslen, rm);
    }

    if mid <= 15 {
        s.cigar_str = DIGM_D_DI_DIGM_D_DI_DIGM_DI_DIGM
            .replace(&s.cigar_str, new_cigar_str.as_str())
            .to_string();
        flag = true;
    }
    flag
}

/// Ported from: CigarModifier.java:L651-L701 (threeDeletions)
/// Merge M-D-M-D-M-D-M into D+optional-I+M form.
fn three_deletions(s: &mut CigarModifierState<'_>, caps: &regex::Captures, mut flag: bool) -> bool {
    let mut tslen: i32 = caps[4].parse::<i32>().unwrap() + caps[6].parse::<i32>().unwrap();
    let mut dlen: i32 = caps[3].parse::<i32>().unwrap()
        + caps[4].parse::<i32>().unwrap()
        + caps[5].parse::<i32>().unwrap()
        + caps[6].parse::<i32>().unwrap()
        + caps[7].parse::<i32>().unwrap();
    let mid: i32 = caps[4].parse::<i32>().unwrap() + caps[6].parse::<i32>().unwrap();

    let ov5 = &caps[1];
    let g2: i32 = caps[2].parse().unwrap();
    let mut refoff = s.position + g2;
    let mut rdoff = g2;
    let mut rdoff_upper = g2;
    let mut rm: i32 = caps[8].parse().unwrap();

    if !ov5.is_empty() {
        refoff += compute_ref_offset(ov5);
        rdoff += compute_read_offset(ov5);
    }

    let mut rn: i32 = 0;
    while (rdoff + rn) < s.query_sequence.len() as i32
        && is_has_and_equals_base(
            s.query_sequence.as_bytes()[(rdoff + rn) as usize],
            &s.reference,
            refoff + rn,
        )
    {
        rn += 1;
    }
    rdoff_upper += rn;
    dlen -= rn;
    tslen -= rn;

    let new_cigar_str = if tslen <= 0 {
        dlen -= tslen;
        rm += tslen;
        format!("{}M{}D{}M", rdoff_upper, dlen, rm)
    } else {
        format!("{}M{}D{}I{}M", rdoff_upper, dlen, tslen, rm)
    };

    if mid <= 15 {
        s.cigar_str = DM_DD_DM_DD_DM_DD_DM
            .replace(&s.cigar_str, new_cigar_str.as_str())
            .to_string();
        flag = true;
    }
    flag
}

/// Ported from: CigarModifier.java:L710-L762 (twoDeletionsInsertionToComplex)
/// Merge M-D-M-I-M-D-M into D+I+M form.
fn two_deletions_insertion_to_complex(
    s: &mut CigarModifierState<'_>,
    caps: &regex::Captures,
    mut flag: bool,
) -> bool {
    let mut tslen: i32 = caps[4].parse::<i32>().unwrap()
        + caps[5].parse::<i32>().unwrap()
        + caps[6].parse::<i32>().unwrap();
    let mut dlen: i32 = caps[3].parse::<i32>().unwrap()
        + caps[4].parse::<i32>().unwrap()
        + caps[6].parse::<i32>().unwrap()
        + caps[7].parse::<i32>().unwrap();
    let mid: i32 = caps[4].parse::<i32>().unwrap() + caps[6].parse::<i32>().unwrap();

    let ov5 = &caps[1];
    let g2: i32 = caps[2].parse().unwrap();
    let mut refoff = s.position + g2;
    let mut rdoff = g2;
    let mut rdoff_upper = g2;
    let mut rm: i32 = caps[8].parse().unwrap();

    if !ov5.is_empty() {
        refoff += compute_ref_offset(ov5);
        rdoff += compute_read_offset(ov5);
    }

    let mut rn: i32 = 0;
    while (rdoff + rn) < s.query_sequence.len() as i32
        && is_has_and_equals_base(
            s.query_sequence.as_bytes()[(rdoff + rn) as usize],
            &s.reference,
            refoff + rn,
        )
    {
        rn += 1;
    }
    rdoff_upper += rn;
    dlen -= rn;
    tslen -= rn;

    let new_cigar_str = if tslen <= 0 {
        dlen -= tslen;
        rm += tslen;
        format!("{}M{}D{}M", rdoff_upper, dlen, rm)
    } else {
        format!("{}M{}D{}I{}M", rdoff_upper, dlen, tslen, rm)
    };

    if mid <= 15 {
        s.cigar_str = D_M_D_DD_M_D_I_D_M_D_DD_prim
            .replace(&s.cigar_str, new_cigar_str.as_str())
            .to_string();
        flag = true;
    }
    flag
}

/// Ported from: CigarModifier.java:L771-L787 (beginDigitMNumberIorDNumberM)
/// Convert leading short match + indel into soft-clip.
/// Trap T2: In Java, match uses jregex but replace uses java.util.regex. In Rust, same regex engine.
fn begin_digit_m_number_i_or_d_number_m(
    s: &mut CigarModifierState<'_>,
    caps: &regex::Captures,
) -> bool {
    let tmid: i32 = caps[1].parse().unwrap();
    let g2: i32 = caps[2].parse().unwrap();
    let g3 = &caps[3];
    let mut mlen: i32 = caps[4].parse().unwrap();
    let mut tn: i32 = 0;

    let mut tslen = tmid + if g3 == "I" { g2 } else { 0 };
    s.position += tmid + if g3 == "D" { g2 } else { 0 };

    while tn < mlen
        && is_has_and_not_equals_base(
            s.query_sequence.as_bytes()[(tslen + tn) as usize],
            &s.reference,
            s.position + tn,
        )
    {
        tn += 1;
    }
    tslen += tn;
    mlen -= tn;
    s.position += tn;
    let replacement = format!("{}S{}M", tslen, mlen);
    s.cigar_str = BEGIN_DIGIT_M_NUMBER_IorD_NUMBER_M_
        .replace(&s.cigar_str, replacement.as_str())
        .to_string();
    true // always sets flag
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Configuration;
    use once_cell::sync::Lazy;
    use std::collections::HashMap;
    use std::sync::Mutex;

    static TEST_SCOPE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn init_test_scope() {
        crate::scope::GlobalReadOnlyScope::clear_thread_local();
        let conf = Configuration::default();
        crate::scope::GlobalReadOnlyScope::init_thread_local(
            conf,
            HashMap::new(),
            "test",
            None,
            None,
            HashMap::new(),
            HashMap::new(),
        );
    }

    fn make_reference(bases: &[(i32, u8)]) -> Reference {
        let mut reference_sequences = ReferenceSequenceMap::default();
        for &(pos, base) in bases {
            reference_sequences.insert(pos, base);
        }
        let mut r = Reference::new();
        r.reference_sequences = reference_sequences;
        r
    }

    #[test]
    fn modify_cigar_strips_leading_deletion() {
        let _guard = TEST_SCOPE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        init_test_scope();

        let reference = make_reference(&[]);
        let region = Region::new("chr1", 1, 1000, "gene");
        let result = modify_cigar(
            100,
            "5D50M",
            "A".repeat(50).as_str(),
            "I".repeat(50).as_str(),
            &reference,
            5,
            &region,
            101,
        );
        assert_eq!(result.position, 105);
        assert!(!result.cigar.starts_with("5D"));

        crate::scope::GlobalReadOnlyScope::clear_thread_local();
    }

    #[test]
    fn modify_cigar_strips_repeated_leading_soft_clips_in_chimeric_path() {
        let _guard = TEST_SCOPE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        init_test_scope();

        let mut reference = make_reference(&[]);
        reference
            .seed
            .insert("TTTTTTTTTTTT".to_string(), smallvec::smallvec![100]);
        let region = Region::new("chr1", 1, 1000, "gene");
        let query_sequence = format!("{}{}", "A".repeat(35), "C".repeat(66));
        let query_quality = "I".repeat(101);

        let result = modify_cigar(
            100,
            "35I66S",
            &query_sequence,
            &query_quality,
            &reference,
            35,
            &region,
            101,
        );

        assert_eq!(result.cigar, "");
        assert_eq!(result.query_sequence, "C".repeat(66));
        assert_eq!(result.query_quality, "I".repeat(66));

        crate::scope::GlobalReadOnlyScope::clear_thread_local();
    }

    #[test]
    fn modify_cigar_strips_repeated_trailing_soft_clips_in_chimeric_path() {
        let _guard = TEST_SCOPE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        init_test_scope();

        let mut reference = make_reference(&[]);
        reference
            .seed
            .insert("TTTTTTTTTTTT".to_string(), smallvec::smallvec![100]);
        let region = Region::new("chr1", 1, 1000, "gene");
        let query_sequence = format!("{}{}{}", "C".repeat(66), "G".repeat(31), "A".repeat(35));
        let query_quality = "I".repeat(132);

        let result = modify_cigar(
            100,
            "66M31S35S",
            &query_sequence,
            &query_quality,
            &reference,
            0,
            &region,
            101,
        );

        assert_eq!(result.cigar, "66M");
        assert_eq!(
            result.query_sequence,
            format!("{}{}", "C".repeat(66), "G".repeat(31))
        );
        assert_eq!(result.query_quality, "I".repeat(97));

        crate::scope::GlobalReadOnlyScope::clear_thread_local();
    }
}
