use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::panic;

use indexmap::IndexMap;

use crate::data::{Region, Variant, Vars};
use crate::mods::output::{AmpliconOutputVariant, VariantRegion};
use crate::scope::{GlobalReadOnlyScope, VariantPrinter};

/// Ported from: AmpliconPostProcessModule.process()
/// Java source: AmpliconPostProcessModule.java:L32-L201
pub fn amplicon_post_process(
    region: &Region,
    vars: &[HashMap<i32, Vars>],
    amplicons_on_positions: &HashMap<i32, Vec<(i32, Region)>>,
    splice: &HashSet<String>,
    variant_printer: &VariantPrinter,
) {
    let conf = GlobalReadOnlyScope::instance().conf;
    let mut positions: Vec<i32> = amplicons_on_positions.keys().copied().collect();
    positions.sort();

    for position in positions {
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let Some(amplicon_regions) = amplicons_on_positions.get(&position) else {
                return;
            };

            let mut gvs: Vec<VariantRegion> = Vec::new();
            let mut ref_variants: Vec<Variant> = Vec::new();
            let mut vref_list: Vec<Variant> = Vec::new();
            let mut goodmap: HashSet<String> = HashSet::new();
            let mut vcovs: Vec<i32> = Vec::new();
            let mut good_variants_on_amp: IndexMap<i32, Vec<Variant>> = IndexMap::new();
            let mut nocov = 0;
            let mut maxcov = 0;

            for (amplicon_number, amplicon_region) in amplicon_regions {
                let reg_str = format!(
                    "{}:{}-{}",
                    amplicon_region.chr, amplicon_region.start, amplicon_region.end
                );
                let vtmp = vars
                    .get(*amplicon_number as usize)
                    .and_then(|amp_vars| amp_vars.get(&position));
                let reference_variant = vtmp.and_then(|vars| vars.reference_variant.clone());

                if let Some(vtmp) = vtmp {
                    if !vtmp.variants.is_empty() {
                        let mut good_vars: Vec<Variant> = Vec::new();
                        for mut tv in vtmp.variants.clone() {
                            vcovs.push(tv.total_pos_coverage);
                            if tv.total_pos_coverage > maxcov {
                                maxcov = tv.total_pos_coverage;
                            }
                            tv.vartype = tv.var_type();
                            if tv.is_good_var(
                                reference_variant.as_ref(),
                                Some(&tv.vartype),
                                splice,
                                &conf,
                            ) {
                                gvs.push(VariantRegion::new(Some(tv.clone()), reg_str.clone()));
                                good_vars.push(tv.clone());
                                goodmap.insert(format!(
                                    "{}-{}-{}",
                                    amplicon_number, tv.refallele, tv.varallele
                                ));
                            }
                        }
                        if !good_vars.is_empty() {
                            good_variants_on_amp.insert(*amplicon_number, good_vars);
                        }
                    } else if let Some(reference_variant) = reference_variant.as_ref() {
                        vcovs.push(reference_variant.total_pos_coverage);
                    } else {
                        vcovs.push(0);
                    }
                } else {
                    vcovs.push(0);
                }

                if let Some(reference_variant) = reference_variant {
                    ref_variants.push(reference_variant);
                }
            }

            for coverage in &vcovs {
                if (*coverage as f64) < (maxcov as f64 / 50.0) {
                    nocov += 1;
                }
            }

            if gvs.len() > 1 {
                gvs.sort_by(|left, right| {
                    let left_freq = left.variant.as_ref().map_or(0.0, |variant| variant.frequency);
                    let right_freq = right.variant.as_ref().map_or(0.0, |variant| variant.frequency);
                    right_freq
                        .partial_cmp(&left_freq)
                        .unwrap_or(Ordering::Equal)
                });
            }
            if ref_variants.len() > 1 {
                ref_variants.sort_by(|left, right| {
                    right.total_pos_coverage.cmp(&left.total_pos_coverage)
                });
            }

            if gvs.is_empty() {
                if !conf.do_pileup {
                    return;
                }
                if let Some(reference_variant) = ref_variants.first() {
                    vref_list.push(reference_variant.clone());
                } else {
                    let output_variant = AmpliconOutputVariant::new(
                        None,
                        region,
                        &[],
                        &[],
                        position,
                        0,
                        nocov,
                        false,
                    );
                    variant_printer.print_line(&output_variant.to_tsv_line(&conf));
                    return;
                }
            } else {
                fill_vref_list(&gvs, &mut vref_list);
            }

            let mut flag = is_amp_bias_flag(&mut good_variants_on_amp);
            let mut good_variants = gvs.clone();
            for mut vref in vref_list {
                if flag {
                    let gdnt = gvs
                        .first()
                        .and_then(|entry| entry.variant.as_ref())
                        .map(|variant| variant.description_string.clone())
                        .unwrap_or_default();
                    let mut gcnt: Vec<VariantRegion> = Vec::new();
                    for (amplicon_number, amplicon_region) in amplicon_regions {
                        let Some(vtmp) = vars
                            .get(*amplicon_number as usize)
                            .and_then(|amp_vars| amp_vars.get(&position))
                        else {
                            continue;
                        };
                        let Some(variant) = vtmp.var_description_string_to_variants.get(&gdnt) else {
                            continue;
                        };
                        if variant.is_good_var(vtmp.reference_variant.as_ref(), None, splice, &conf)
                        {
                            gcnt.push(VariantRegion::new(
                                Some(variant.clone()),
                                format!(
                                    "{}:{}-{}",
                                    amplicon_region.chr,
                                    amplicon_region.start,
                                    amplicon_region.end
                                ),
                            ));
                        }
                    }
                    if gcnt.len() == gvs.len() {
                        flag = false;
                    }
                    gcnt.sort_by(|left, right| {
                        let left_freq = left.variant.as_ref().map_or(0.0, |variant| variant.frequency);
                        let right_freq = right.variant.as_ref().map_or(0.0, |variant| variant.frequency);
                        right_freq
                            .partial_cmp(&left_freq)
                            .unwrap_or(Ordering::Equal)
                    });
                    good_variants = gcnt;
                }

                let initial_gvscnt = count_variant_on_amplicons(&vref, &good_variants_on_amp);
                let mut current_gvscnt = initial_gvscnt;
                let mut bad_variants: Vec<VariantRegion> = Vec::new();
                if initial_gvscnt != i32::try_from(amplicon_regions.len()).unwrap_or(0) || flag {
                    for (amplicon_number, reg) in amplicon_regions {
                        if goodmap.contains(&format!(
                            "{}-{}-{}",
                            amplicon_number, vref.refallele, vref.varallele
                        )) {
                            continue;
                        }
                        if conf.do_pileup && vref.refallele == vref.varallele {
                            continue;
                        }
                        if vref.start_position >= reg.insert_start
                            && vref.end_position <= reg.insert_end
                        {
                            let reg_str = format!("{}:{}-{}", reg.chr, reg.start, reg.end);
                            let vtmp = vars
                                .get(*amplicon_number as usize)
                                .and_then(|amp_vars| amp_vars.get(&position));
                            if let Some(vtmp) = vtmp {
                                if let Some(variant) = vtmp.variants.first() {
                                    bad_variants
                                        .push(VariantRegion::new(Some(variant.clone()), reg_str));
                                } else if let Some(reference_variant) = vtmp.reference_variant.clone() {
                                    bad_variants
                                        .push(VariantRegion::new(Some(reference_variant), reg_str));
                                } else {
                                    bad_variants.push(VariantRegion::new(None, reg_str));
                                }
                            } else {
                                bad_variants.push(VariantRegion::new(None, reg_str));
                            }
                        } else if (vref.start_position < reg.insert_end
                            && reg.insert_end < vref.end_position)
                            || (vref.start_position < reg.insert_start
                                && reg.insert_start < vref.end_position)
                        {
                            if current_gvscnt > 1 {
                                current_gvscnt -= 1;
                            }
                        }
                    }
                }

                if flag && current_gvscnt < initial_gvscnt {
                    flag = false;
                }
                vref.vartype = vref.var_type();
                if vref.vartype == "Complex" {
                    vref.adj_complex();
                }

                let output_variant = AmpliconOutputVariant::new(
                    Some(&vref),
                    region,
                    &good_variants,
                    &bad_variants,
                    position,
                    current_gvscnt,
                    nocov,
                    flag,
                );
                variant_printer.print_line(&output_variant.to_tsv_line(&conf));
            }
        }));

        if let Err(error) = result {
            eprintln!(
                "Error processing position {} in {}:{}-{}: {:?}",
                position, region.chr, region.start, region.end, error
            );
        }
    }
}

fn count_variant_on_amplicons(vref: &Variant, good_variants_on_amp: &IndexMap<i32, Vec<Variant>>) -> i32 {
    let mut gvscnt = 0;
    for variants in good_variants_on_amp.values() {
        for variant in variants {
            if variant.refallele == vref.refallele && variant.varallele == vref.varallele {
                gvscnt += 1;
            }
        }
    }
    gvscnt
}

fn fill_vref_list(gvs: &[VariantRegion], vref_list: &mut Vec<Variant>) {
    for good_variant in gvs {
        let Some(variant_to_add) = good_variant.variant.as_ref() else {
            continue;
        };
        let already_added = vref_list.iter().any(|variant| {
            variant.varallele == variant_to_add.varallele
                && variant.refallele == variant_to_add.refallele
        });
        if !already_added {
            vref_list.push(variant_to_add.clone());
        }
    }
}

fn is_amp_bias_flag(good_variants_on_amp: &mut IndexMap<i32, Vec<Variant>>) -> bool {
    if good_variants_on_amp.is_empty() {
        return false;
    }

    let mut amplicon_list: Vec<i32> = good_variants_on_amp.keys().copied().collect();
    amplicon_list.sort();
    for window in amplicon_list.windows(2) {
        let current_amplicon = window[0];
        let next_amplicon = window[1];
        {
            let Some(current_variants) = good_variants_on_amp.get_mut(&current_amplicon) else {
                return true;
            };
            current_variants
                .sort_by(|left, right| right.total_pos_coverage.cmp(&left.total_pos_coverage));
        }
        let current_descriptions = good_variants_on_amp
            .get(&current_amplicon)
            .map(|variants| {
                variants
                    .iter()
                    .map(|variant| variant.description_string.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        {
            let Some(next_variants) = good_variants_on_amp.get_mut(&next_amplicon) else {
                return true;
            };
            next_variants
                .sort_by(|left, right| right.total_pos_coverage.cmp(&left.total_pos_coverage));
        }
        let next_descriptions = good_variants_on_amp
            .get(&next_amplicon)
            .map(|variants| {
                variants
                    .iter()
                    .map(|variant| variant.description_string.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if current_descriptions.len() != next_descriptions.len() {
            return true;
        }

        for (left, right) in current_descriptions.iter().zip(next_descriptions.iter()) {
            if left != right {
                return true;
            }
        }
    }
    false
}