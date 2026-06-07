use std::collections::HashSet;
use std::panic;
use std::sync::{Arc, Mutex};

use crate::config::Configuration;
use crate::data::{AlignedVarsData, CombineAnalysisData, InitialData, Region, Variant, Vars};
use crate::modes::run_pipeline;
use crate::mods::output::SomaticOutputVariant;
use crate::patterns::MINUS_NUM_NUM;
use crate::reference::ReferenceResource;
use crate::scope::{GlobalReadOnlyScope, Scope, VariantPrinter};
use crate::variations::{VarMaybeArg, VarsType, get_var_maybe_from_vars, strand_bias};

const STRONG_SOMATIC: &str = "StrongSomatic";
const SAMPLE_SPECIFIC: &str = "SampleSpecific";
const DELETION: &str = "Deletion";
const LIKELY_LOH: &str = "LikelyLOH";
const GERMLINE: &str = "Germline";
const STRONG_LOH: &str = "StrongLOH";
const LIKELY_SOMATIC: &str = "LikelySomatic";
const AF_DIFF: &str = "AFDiff";
const FALSE: &str = "FALSE";
const COMPLEX: &str = "Complex";
const SNV: &str = "SNV";

/// Ported from: SomaticPostProcessModule.accept()
/// Java source: SomaticPostProcessModule.java:L56-L93
pub fn somatic_post_process(
    scope_from_bam2: Scope<AlignedVarsData>,
    scope_from_bam1: Scope<AlignedVarsData>,
    reference_resource: &ReferenceResource,
) {
    let region = scope_from_bam1.region.clone();
    let splice = (*scope_from_bam1.splice).clone();
    let scope_instance = GlobalReadOnlyScope::instance();
    let conf = &scope_instance.conf;
    let out = scope_from_bam1.out.clone();
    let mut variations_from_bam1 = scope_from_bam1.data.aligned_variants;
    let mut variations_from_bam2 = scope_from_bam2.data.aligned_variants;
    let mut max_read_length = scope_from_bam1
        .max_read_length
        .max(scope_from_bam2.max_read_length);

    let mut all_positions: Vec<i32> = variations_from_bam1
        .keys()
        .chain(variations_from_bam2.keys())
        .copied()
        .collect();
    all_positions.sort();
    all_positions.dedup();

    for position in all_positions {
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            if position < region.start || position > region.end {
                return;
            }
            let v1 = variations_from_bam1.get_mut(&position);
            let v2 = variations_from_bam2.get_mut(&position);
            match (v1, v2) {
                (None, None) => {}
                (None, Some(v2)) => {
                    calling_for_one_sample(v2, true, DELETION, &region, &splice, &conf, &out)
                }
                (Some(v1), None) => calling_for_one_sample(
                    v1,
                    false,
                    SAMPLE_SPECIFIC,
                    &region,
                    &splice,
                    &conf,
                    &out,
                ),
                (Some(v1), Some(v2)) => calling_for_both_samples(
                    position,
                    v1,
                    v2,
                    &region,
                    &splice,
                    &conf,
                    &out,
                    reference_resource,
                    &mut max_read_length,
                ),
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

fn calling_for_one_sample(
    variants: &mut Vars,
    is_first_cover: bool,
    var_label: &str,
    region: &Region,
    splice: &HashSet<String>,
    conf: &Configuration,
    out: &crate::scope::VariantPrinter,
) {
    if variants.variants.is_empty() {
        return;
    }

    for mut variant in variants.variants.clone() {
        if variant.refallele == variant.varallele {
            continue;
        }
        variant.vartype = variant.var_type();
        if !variant.is_good_var(
            variants.reference_variant.as_ref(),
            Some(&variant.vartype),
            splice,
            conf,
        ) {
            continue;
        }
        if variant.vartype == COMPLEX {
            variant.adj_complex();
        }
        let output_variant = if is_first_cover {
            SomaticOutputVariant::new(
                Some(&variant),
                Some(&variant),
                None,
                Some(&variant),
                region,
                "",
                &variants.sv,
                var_label,
            )
        } else {
            SomaticOutputVariant::new(
                Some(&variant),
                Some(&variant),
                Some(&variant),
                None,
                region,
                &variants.sv,
                "",
                var_label,
            )
        };
        out.print_line(&output_variant.to_tsv_line(conf));
    }
}

fn calling_for_both_samples(
    position: i32,
    v1: &mut Vars,
    v2: &mut Vars,
    region: &Region,
    splice: &HashSet<String>,
    conf: &Configuration,
    out: &crate::scope::VariantPrinter,
    reference_resource: &ReferenceResource,
    max_read_length: &mut i32,
) {
    if v1.variants.is_empty() && v2.variants.is_empty() {
        return;
    }
    if !v1.variants.is_empty() {
        print_variations_from_first_sample(
            position,
            v1,
            v2,
            region,
            splice,
            conf,
            out,
            reference_resource,
            max_read_length,
        );
    } else if !v2.variants.is_empty() {
        print_variations_from_second_sample(
            position,
            v1,
            v2,
            region,
            splice,
            conf,
            out,
            reference_resource,
            max_read_length,
        );
    }
}

fn print_variations_from_first_sample(
    position: i32,
    v1: &mut Vars,
    v2: &mut Vars,
    region: &Region,
    splice: &HashSet<String>,
    conf: &Configuration,
    out: &crate::scope::VariantPrinter,
    reference_resource: &ReferenceResource,
    max_read_length: &mut i32,
) {
    let mut processed = 0usize;
    while processed < v1.variants.len() {
        let current = v1.variants[processed].clone();
        let current_type = current.var_type();
        if !current.is_good_var(
            v1.reference_variant.as_ref(),
            Some(&current_type),
            splice,
            conf,
        ) {
            break;
        }

        let mut vref = current;
        if vref.refallele == vref.varallele {
            processed += 1;
            continue;
        }

        let nt = vref.description_string.clone();
        vref.vartype = current_type;
        if vref.vartype == COMPLEX {
            vref.adj_complex();
        }

        if let Some(v2nt_ref) =
            get_var_maybe_from_vars(v2, VarsType::Varn, VarMaybeArg::Description(&nt))
        {
            let mut v2nt = v2nt_ref.clone();
            let type_ = determinate_type(v2, &vref, &mut v2nt, splice, conf);
            let output_variant = SomaticOutputVariant::new(
                Some(&vref),
                Some(&v2nt),
                Some(&vref),
                Some(&v2nt),
                region,
                &v1.sv,
                &v2.sv,
                &type_,
            );
            out.print_line(&output_variant.to_tsv_line(conf));
        } else {
            let var_for_print = if !v2.variants.is_empty() {
                let v2r = get_var_maybe_from_vars(v2, VarsType::Var, VarMaybeArg::Index(0));
                let mut var_for_print = Variant::default();
                if let Some(v2r) = v2r {
                    var_for_print.total_pos_coverage = v2r.total_pos_coverage;
                    var_for_print.ref_forward_coverage = v2r.ref_forward_coverage;
                    var_for_print.ref_reverse_coverage = v2r.ref_reverse_coverage;
                }
                Some(var_for_print)
            } else {
                v2.reference_variant.clone()
            };

            let mut rescued_variant = None;
            let mut type_ = String::from(STRONG_SOMATIC);
            if vref.vartype != SNV && (nt.len() > 10 || MINUS_NUM_NUM.is_match(&nt)) {
                let mut v2nt = Variant::default();
                v2.var_description_string_to_variants
                    .insert(nt.clone(), v2nt.clone());
                if vref.position_coverage < conf.minr + 3 && !nt.contains('<') {
                    let combine_data = combine_analysis(
                        &vref,
                        &mut v2nt,
                        &region.chr,
                        position,
                        &nt,
                        splice,
                        *max_read_length,
                        reference_resource,
                    );
                    *max_read_length = combine_data.max_read_length;
                    if combine_data.type_ == FALSE {
                        processed += 1;
                        continue;
                    }
                    if !combine_data.type_.is_empty() {
                        type_ = combine_data.type_;
                    }
                }
                rescued_variant = Some(v2nt);
            }

            let output_variant = if type_ == STRONG_SOMATIC {
                SomaticOutputVariant::new(
                    Some(&vref),
                    Some(&vref),
                    Some(&vref),
                    var_for_print.as_ref(),
                    region,
                    &v1.sv,
                    &v2.sv,
                    STRONG_SOMATIC,
                )
            } else {
                SomaticOutputVariant::new(
                    Some(&vref),
                    Some(&vref),
                    Some(&vref),
                    rescued_variant.as_ref(),
                    region,
                    &v1.sv,
                    &v2.sv,
                    &type_,
                )
            };
            out.print_line(&output_variant.to_tsv_line(conf));
        }

        processed += 1;
    }

    if processed != 0 || v2.variants.is_empty() {
        return;
    }

    for v2var_original in v2.variants.clone() {
        let mut v2var = v2var_original;
        v2var.vartype = v2var.var_type();
        if !v2var.is_good_var(
            v2.reference_variant.as_ref(),
            Some(&v2var.vartype),
            splice,
            conf,
        ) {
            continue;
        }

        let nt = v2var.description_string.clone();
        if let Some(v1nt_ref) =
            get_var_maybe_from_vars(v1, VarsType::Varn, VarMaybeArg::Description(&nt))
        {
            let mut v1nt = v1nt_ref.clone();
            if v1nt.refallele == v1nt.varallele {
                continue;
            }
            let type_ = if v1nt.frequency < conf.lofreq {
                LIKELY_LOH
            } else {
                GERMLINE
            };
            if v2var.vartype == COMPLEX {
                v1nt.adj_complex();
            }
            v1nt.vartype = v1nt.var_type();
            let output_variant = SomaticOutputVariant::new(
                Some(&v1nt),
                Some(&v2var),
                Some(&v1nt),
                Some(&v2var),
                region,
                &v1.sv,
                &v2.sv,
                type_,
            );
            out.print_line(&output_variant.to_tsv_line(conf));
        } else {
            if v2var.refallele == v2var.varallele {
                continue;
            }
            let v1var = get_var_maybe_from_vars(v1, VarsType::Var, VarMaybeArg::Index(0));
            let tcov = v1var.map_or(0, |variant| variant.total_pos_coverage);
            let v1ref = v1.reference_variant.as_ref();
            let fwd = v1ref.map_or(0, |variant| variant.vars_count_on_forward);
            let rev = v1ref.map_or(0, |variant| variant.vars_count_on_reverse);
            let genotype = if let Some(v1var) = v1var {
                v1var.genotype.clone().unwrap_or_else(|| String::from("0"))
            } else if let Some(v1ref) = v1ref {
                format!("{0}/{0}", v1ref.description_string)
            } else {
                String::from("N/N")
            };

            if v2var.vartype == COMPLEX {
                v2var.adj_complex();
            }

            let mut var_for_print = Variant::default();
            var_for_print.total_pos_coverage = tcov;
            var_for_print.ref_forward_coverage = fwd;
            var_for_print.ref_reverse_coverage = rev;
            var_for_print.genotype = Some(genotype);

            let output_variant = SomaticOutputVariant::new(
                Some(&v2var),
                Some(&v2var),
                Some(&var_for_print),
                Some(&v2var),
                region,
                "",
                &v2.sv,
                STRONG_LOH,
            );
            out.print_line(&output_variant.to_tsv_line(conf));
        }
    }
}

fn print_variations_from_second_sample(
    position: i32,
    v1: &mut Vars,
    v2: &mut Vars,
    region: &Region,
    splice: &HashSet<String>,
    conf: &Configuration,
    out: &crate::scope::VariantPrinter,
    reference_resource: &ReferenceResource,
    max_read_length: &mut i32,
) {
    for v2var_original in v2.variants.clone() {
        let mut v2var = v2var_original;
        if v2var.refallele == v2var.varallele {
            continue;
        }
        v2var.vartype = v2var.var_type();
        if !v2var.is_good_var(
            v2.reference_variant.as_ref(),
            Some(&v2var.vartype),
            splice,
            conf,
        ) {
            continue;
        }

        let description_string = v2var.description_string.clone();
        let mut type_ = String::from(STRONG_LOH);
        let mut new_type = String::new();
        let v1nt = v1
            .var_description_string_to_variants
            .entry(description_string.clone())
            .or_default();
        v1nt.position_coverage = 0;

        if v2
            .var_description_string_to_variants
            .get(&description_string)
            .is_some_and(|variant| variant.position_coverage < conf.minr + 3)
            && !description_string.contains('<')
            && (description_string.len() > 10 || MINUS_NUM_NUM.is_match(&description_string))
        {
            let combine_data = combine_analysis(
                v2.var_description_string_to_variants
                    .get(&description_string)
                    .expect("BAM2 variant description must exist"),
                v1nt,
                &region.chr,
                position,
                &description_string,
                splice,
                *max_read_length,
                reference_resource,
            );
            *max_read_length = combine_data.max_read_length;
            new_type = combine_data.type_;
            if new_type == FALSE {
                continue;
            }
        }

        let var_for_print = if !new_type.is_empty() {
            type_ = new_type;
            Some(v1nt.clone())
        } else {
            v1.reference_variant.clone()
        };

        if v2var.vartype == COMPLEX {
            v2var.adj_complex();
        }

        let output_variant = SomaticOutputVariant::new(
            Some(&v2var),
            Some(&v2var),
            var_for_print.as_ref(),
            Some(&v2var),
            region,
            "",
            &v2.sv,
            &type_,
        );
        out.print_line(&output_variant.to_tsv_line(conf));
    }
}

/// Ported from: SomaticPostProcessModule.determinateType()
/// Java source: SomaticPostProcessModule.java:L346-L369
pub fn determinate_type(
    variants: &Vars,
    standard_variant: &Variant,
    variant_to_compare: &mut Variant,
    splice: &HashSet<String>,
    conf: &Configuration,
) -> String {
    let mut type_ = if variant_to_compare.is_good_var(
        variants.reference_variant.as_ref(),
        Some(&standard_variant.vartype),
        splice,
        conf,
    ) {
        if standard_variant.frequency > (1.0 - conf.lofreq)
            && variant_to_compare.frequency < 0.8
            && variant_to_compare.frequency > 0.2
        {
            String::from(LIKELY_LOH)
        } else if variant_to_compare.frequency < conf.lofreq
            || variant_to_compare.position_coverage <= 1
        {
            String::from(LIKELY_SOMATIC)
        } else {
            String::from(GERMLINE)
        }
    } else if variant_to_compare.frequency < conf.lofreq
        || variant_to_compare.position_coverage <= 1
    {
        String::from(LIKELY_SOMATIC)
    } else {
        String::from(AF_DIFF)
    };

    if variant_to_compare.is_noise(conf) && standard_variant.vartype == SNV {
        type_ = String::from(STRONG_SOMATIC);
    }
    type_
}

/// Ported from: SomaticPostProcessModule.combineAnalysis()
/// Java source: SomaticPostProcessModule.java:L385-L476
#[allow(clippy::too_many_arguments)]
fn combine_analysis(
    variant1: &Variant,
    variant2: &mut Variant,
    chr_name: &str,
    position: i32,
    description_string: &str,
    splice: &HashSet<String>,
    mut max_read_length: i32,
    reference_resource: &ReferenceResource,
) -> CombineAnalysisData {
    let scope_instance = GlobalReadOnlyScope::instance();
    let conf = &scope_instance.conf;

    // Don't do it for structural variants
    if variant1.end_position - variant1.start_position > conf.svminlen {
        return CombineAnalysisData::new(max_read_length, "");
    }

    let rescue_region = Region::new(
        chr_name,
        variant1.start_position - max_read_length,
        variant1.end_position + max_read_length,
        "",
    );
    let reference = reference_resource
        .get_reference(&rescue_region)
        .unwrap_or_else(|error| {
            panic!(
                "Failed to fetch reference for {}: {}",
                rescue_region.print_region(),
                error
            )
        });

    let bam_names = conf.bam.as_ref().expect("BAM names must be configured");
    let combined_bam = format!("{}:{}", bam_names.get_bam1(), bam_names.get_bam2().unwrap());

    let throwaway_buffer = Arc::new(Mutex::new(String::new()));
    let throwaway_printer = VariantPrinter::Buffer(throwaway_buffer);

    let scope = Scope::new(
        combined_bam,
        rescue_region,
        Arc::new(reference),
        Arc::new(reference_resource.clone()),
        max_read_length,
        splice.clone(),
        throwaway_printer,
        InitialData::default(),
    );

    let tpl = run_pipeline(scope);
    max_read_length = tpl.max_read_length;
    let vars = tpl.data.aligned_variants;

    let vref = vars
        .get(&position)
        .and_then(|vars_at_pos| {
            get_var_maybe_from_vars(
                vars_at_pos,
                VarsType::Varn,
                VarMaybeArg::Description(description_string),
            )
        })
        .cloned();

    if let Some(vref) = vref {
        if vref.position_coverage - variant1.position_coverage >= conf.minr {
            variant2.total_pos_coverage = vref.total_pos_coverage - variant1.total_pos_coverage;
            if variant2.total_pos_coverage < 0 {
                variant2.total_pos_coverage = 0;
            }

            variant2.position_coverage = vref.position_coverage - variant1.position_coverage;
            if variant2.position_coverage < 0 {
                variant2.position_coverage = 0;
            }

            variant2.ref_forward_coverage =
                vref.ref_forward_coverage - variant1.ref_forward_coverage;
            if variant2.ref_forward_coverage < 0 {
                variant2.ref_forward_coverage = 0;
            }

            variant2.ref_reverse_coverage =
                vref.ref_reverse_coverage - variant1.ref_reverse_coverage;
            if variant2.ref_reverse_coverage < 0 {
                variant2.ref_reverse_coverage = 0;
            }

            variant2.vars_count_on_forward =
                vref.vars_count_on_forward - variant1.vars_count_on_forward;
            if variant2.vars_count_on_forward < 0 {
                variant2.vars_count_on_forward = 0;
            }

            variant2.vars_count_on_reverse =
                vref.vars_count_on_reverse - variant1.vars_count_on_reverse;
            if variant2.vars_count_on_reverse < 0 {
                variant2.vars_count_on_reverse = 0;
            }

            if variant2.position_coverage != 0 {
                let cov = variant2.position_coverage as f64;
                variant2.mean_position = (vref.mean_position * vref.position_coverage as f64
                    - variant1.mean_position * variant1.position_coverage as f64)
                    / cov;
                variant2.mean_quality = (vref.mean_quality * vref.position_coverage as f64
                    - variant1.mean_quality * variant1.position_coverage as f64)
                    / cov;
                variant2.mean_mapping_quality = (vref.mean_mapping_quality
                    * vref.position_coverage as f64
                    - variant1.mean_mapping_quality * variant1.position_coverage as f64)
                    / cov;
                variant2.high_quality_reads_frequency = (vref.high_quality_reads_frequency
                    * vref.position_coverage as f64
                    - variant1.high_quality_reads_frequency * variant1.position_coverage as f64)
                    / cov;
                variant2.extra_frequency = (vref.extra_frequency * vref.position_coverage as f64
                    - variant1.extra_frequency * variant1.position_coverage as f64)
                    / cov;
                variant2.number_of_mismatches = (vref.number_of_mismatches
                    * vref.position_coverage as f64
                    - variant1.number_of_mismatches * variant1.position_coverage as f64)
                    / cov;
            } else {
                variant2.mean_position = 0.0;
                variant2.mean_quality = 0.0;
                variant2.mean_mapping_quality = 0.0;
                variant2.high_quality_reads_frequency = 0.0;
                variant2.extra_frequency = 0.0;
                variant2.number_of_mismatches = 0.0;
            }
            variant2.is_at_least_at_2_positions = true;
            variant2.has_at_least_2_diff_qualities = true;

            if variant2.total_pos_coverage <= 0 {
                return CombineAnalysisData::new(max_read_length, FALSE);
            }

            variant2.frequency =
                variant2.position_coverage as f64 / variant2.total_pos_coverage as f64;
            variant2.high_quality_to_low_quality_ratio = variant1.high_quality_to_low_quality_ratio;
            variant2.genotype = vref.genotype.clone();
            variant2.strand_bias_flag = format!(
                "{};{}",
                strand_bias(variant2.ref_forward_coverage, variant2.ref_reverse_coverage),
                strand_bias(
                    variant2.vars_count_on_forward,
                    variant2.vars_count_on_reverse
                ),
            );
            CombineAnalysisData::new(max_read_length, GERMLINE)
        } else if vref.position_coverage < variant1.position_coverage - 2 {
            CombineAnalysisData::new(max_read_length, FALSE)
        } else {
            CombineAnalysisData::new(max_read_length, "")
        }
    } else {
        CombineAnalysisData::new(max_read_length, FALSE)
    }
}
