mod common;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use vardict_rs::config::Configuration;
use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

const PUSH_INDICES: &[usize] = &[0, 1, 2, 3, 4, 35, 36, 37, 70, 71];

macro_rules! declarative_test {
    ($(($test_name:ident, $config_name:literal)),+ $(,)?) => {
        $(
            #[test]
            fn $test_name() {
                run_config_e2e_single($config_name, Some(PUSH_INDICES));
            }
        )+
    };
}

macro_rules! declarative_test_all {
    ($(($test_name:ident, $config_name:literal)),+ $(,)?) => {
        $(
            #[test]
            #[ignore = "Nightly: run with cargo nextest --run-ignored all"]
            fn $test_name() {
                run_config_e2e_single($config_name, None);
            }
        )+
    };
}

declarative_test!(
    (parity_config_e2e_push_t1_01, "T1-01"),
    (parity_config_e2e_push_t1_02, "T1-02"),
    (parity_config_e2e_push_t1_03, "T1-03"),
    (parity_config_e2e_push_t1_04, "T1-04"),
    (parity_config_e2e_push_t1_05, "T1-05"),
    (parity_config_e2e_push_t1_06, "T1-06"),
    (parity_config_e2e_push_t1_07, "T1-07"),
    (parity_config_e2e_push_t1_08, "T1-08"),
    (parity_config_e2e_push_t1_09, "T1-09"),
    (parity_config_e2e_push_t1_10, "T1-10"),
    (parity_config_e2e_push_t1_11, "T1-11"),
    (parity_config_e2e_push_t1_12, "T1-12"),
    (parity_config_e2e_push_t1_13, "T1-13"),
    (parity_config_e2e_push_t1_14, "T1-14"),
    (parity_config_e2e_push_t2_01, "T2-01"),
    (parity_config_e2e_push_t2_02, "T2-02"),
    (parity_config_e2e_push_t2_03, "T2-03"),
    (parity_config_e2e_push_t2_04, "T2-04"),
    (parity_config_e2e_push_t2_05, "T2-05"),
    (parity_config_e2e_push_t2_06, "T2-06"),
    (parity_config_e2e_push_t2_07, "T2-07"),
    (parity_config_e2e_push_t2_08, "T2-08"),
    (parity_config_e2e_push_t2_09, "T2-09"),
    (parity_config_e2e_push_t2_10, "T2-10"),
    (parity_config_e2e_push_t3_01, "T3-01"),
    (parity_config_e2e_push_t3_02, "T3-02"),
    (parity_config_e2e_push_t3_03, "T3-03"),
    (parity_config_e2e_push_t3_04, "T3-04"),
    (parity_config_e2e_push_t3_05, "T3-05"),
    (parity_config_e2e_push_t3_06, "T3-06"),
    (parity_config_e2e_push_t3_07, "T3-07"),
    (parity_config_e2e_push_t3_08, "T3-08"),
    (parity_config_e2e_push_t3_09, "T3-09"),
    (parity_config_e2e_push_t3_10, "T3-10"),
    (parity_config_e2e_push_pw_000, "PW-000"),
    (parity_config_e2e_push_pw_001, "PW-001"),
    (parity_config_e2e_push_pw_002, "PW-002"),
    (parity_config_e2e_push_pw_003, "PW-003"),
    (parity_config_e2e_push_pw_004, "PW-004"),
    (parity_config_e2e_push_pw_005, "PW-005"),
    (parity_config_e2e_push_pw_006, "PW-006"),
    (parity_config_e2e_push_pw_007, "PW-007"),
    (parity_config_e2e_push_pw_008, "PW-008"),
    (parity_config_e2e_push_pw_009, "PW-009"),
);

declarative_test_all!(
    (parity_config_e2e_all_t1_01, "T1-01"),
    (parity_config_e2e_all_t1_02, "T1-02"),
    (parity_config_e2e_all_t1_03, "T1-03"),
    (parity_config_e2e_all_t1_04, "T1-04"),
    (parity_config_e2e_all_t1_05, "T1-05"),
    (parity_config_e2e_all_t1_06, "T1-06"),
    (parity_config_e2e_all_t1_07, "T1-07"),
    (parity_config_e2e_all_t1_08, "T1-08"),
    (parity_config_e2e_all_t1_09, "T1-09"),
    (parity_config_e2e_all_t1_10, "T1-10"),
    (parity_config_e2e_all_t1_11, "T1-11"),
    (parity_config_e2e_all_t1_12, "T1-12"),
    (parity_config_e2e_all_t1_13, "T1-13"),
    (parity_config_e2e_all_t1_14, "T1-14"),
    (parity_config_e2e_all_t2_01, "T2-01"),
    (parity_config_e2e_all_t2_02, "T2-02"),
    (parity_config_e2e_all_t2_03, "T2-03"),
    (parity_config_e2e_all_t2_04, "T2-04"),
    (parity_config_e2e_all_t2_05, "T2-05"),
    (parity_config_e2e_all_t2_06, "T2-06"),
    (parity_config_e2e_all_t2_07, "T2-07"),
    (parity_config_e2e_all_t2_08, "T2-08"),
    (parity_config_e2e_all_t2_09, "T2-09"),
    (parity_config_e2e_all_t2_10, "T2-10"),
    (parity_config_e2e_all_t3_01, "T3-01"),
    (parity_config_e2e_all_t3_02, "T3-02"),
    (parity_config_e2e_all_t3_03, "T3-03"),
    (parity_config_e2e_all_t3_04, "T3-04"),
    (parity_config_e2e_all_t3_05, "T3-05"),
    (parity_config_e2e_all_t3_06, "T3-06"),
    (parity_config_e2e_all_t3_07, "T3-07"),
    (parity_config_e2e_all_t3_08, "T3-08"),
    (parity_config_e2e_all_t3_09, "T3-09"),
    (parity_config_e2e_all_t3_10, "T3-10"),
    (parity_config_e2e_all_pw_000, "PW-000"),
    (parity_config_e2e_all_pw_001, "PW-001"),
    (parity_config_e2e_all_pw_002, "PW-002"),
    (parity_config_e2e_all_pw_003, "PW-003"),
    (parity_config_e2e_all_pw_004, "PW-004"),
    (parity_config_e2e_all_pw_005, "PW-005"),
    (parity_config_e2e_all_pw_006, "PW-006"),
    (parity_config_e2e_all_pw_007, "PW-007"),
    (parity_config_e2e_all_pw_008, "PW-008"),
    (parity_config_e2e_all_pw_009, "PW-009"),
);

#[test]
fn config_preset_alignment() {
    let preset_rows = common::load_config_presets_tsv();
    let preset_names: HashSet<&str> = preset_rows.iter().map(|(name, _)| name.as_str()).collect();
    let rust_preset_names: HashSet<&str> = common::CONFIG_PRESETS.iter().copied().collect();

    assert_eq!(
        preset_names, rust_preset_names,
        "Preset names in scripts/config_presets.tsv must match common::CONFIG_PRESETS"
    );

    let defaults = Configuration::default();
    for (preset_name, java_flags) in preset_rows {
        let config = common::config_preset(&preset_name);
        assert_config_matches_java_flags(&preset_name, &java_flags, &config, &defaults);
    }
}

#[allow(dead_code)]
fn run_config_e2e_suite(indices: Option<&[usize]>, regeneration_command: &str) {
    let regions = common::load_region_config();
    let fixture_base = common::e2e_fixture_base();
    let implementation = common::resolve_impl();

    for config_name in common::CONFIG_PRESETS {
        let config = common::config_preset(config_name);
        let java_flags = common::config_preset_java_flags(config_name);

        for (_region_index, region_str, bam_path, ref_path) in select_regions(&regions, indices) {
            let bam_str = bam_path.to_str().unwrap_or_else(|| {
                panic!(
                    "BAM path for region {region_str} was not valid UTF-8: {}",
                    bam_path.display()
                )
            });
            let ref_str = ref_path.to_str().unwrap_or_else(|| {
                panic!(
                    "Reference path for region {region_str} was not valid UTF-8: {}",
                    ref_path.display()
                )
            });
            let fai_path = format!("{ref_str}.fai");
            let chr_lengths = common::load_chr_lengths(&fai_path);
            let expected = common::load_golden_tsv(
                &fixture_base,
                &region_str,
                Some(config_name),
                regeneration_command,
            );

            match implementation {
                common::VardictImpl::Rust => {
                    let actual = run_simple_mode_region_with_config(
                        &region_str,
                        bam_str,
                        ref_str,
                        chr_lengths,
                        config.clone(),
                    );
                    common::assert_tsv_parity(&actual, &expected, &region_str);
                }
                common::VardictImpl::Java => {
                    let actual =
                        common::run_java_region(&region_str, bam_str, ref_str, &java_flags);
                    common::assert_tsv_parity(&actual, &expected, &region_str);
                }
                common::VardictImpl::Both => {
                    let rust_actual = run_simple_mode_region_with_config(
                        &region_str,
                        bam_str,
                        ref_str,
                        chr_lengths.clone(),
                        config.clone(),
                    );
                    common::assert_tsv_parity(&rust_actual, &expected, &region_str);

                    let java_actual =
                        common::run_java_region(&region_str, bam_str, ref_str, &java_flags);
                    common::assert_tsv_parity(&java_actual, &expected, &region_str);
                }
            }
        }
    }
}

fn run_config_e2e_single(config_name: &str, indices: Option<&[usize]>) {
    let regions = common::load_region_config();
    let fixture_base = common::e2e_fixture_base();
    let implementation = common::resolve_impl();
    let regeneration_command = match indices {
        Some(_) => "bash scripts/gen_e2e_golden_tsv.sh --push-only --all-configs",
        None => "bash scripts/gen_e2e_golden_tsv.sh --all-configs",
    };

    let config = common::config_preset(config_name);
    let java_flags = common::config_preset_java_flags(config_name);

    for (_region_index, region_str, bam_path, ref_path) in select_regions(&regions, indices) {
        let bam_str = bam_path.to_str().unwrap_or_else(|| {
            panic!(
                "BAM path for region {region_str} was not valid UTF-8: {}",
                bam_path.display()
            )
        });
        let ref_str = ref_path.to_str().unwrap_or_else(|| {
            panic!(
                "Reference path for region {region_str} was not valid UTF-8: {}",
                ref_path.display()
            )
        });
        let fai_path = format!("{ref_str}.fai");
        let chr_lengths = common::load_chr_lengths(&fai_path);
        let expected = common::load_golden_tsv(
            &fixture_base,
            &region_str,
            Some(config_name),
            regeneration_command,
        );

        match implementation {
            common::VardictImpl::Rust => {
                let actual = run_simple_mode_region_with_config(
                    &region_str,
                    bam_str,
                    ref_str,
                    chr_lengths,
                    config.clone(),
                );
                common::assert_tsv_parity(&actual, &expected, &region_str);
            }
            common::VardictImpl::Java => {
                let actual = common::run_java_region(&region_str, bam_str, ref_str, &java_flags);
                common::assert_tsv_parity(&actual, &expected, &region_str);
            }
            common::VardictImpl::Both => {
                let rust_actual = run_simple_mode_region_with_config(
                    &region_str,
                    bam_str,
                    ref_str,
                    chr_lengths.clone(),
                    config.clone(),
                );
                common::assert_tsv_parity(&rust_actual, &expected, &region_str);

                let java_actual = common::run_java_region(&region_str, bam_str, ref_str, &java_flags);
                common::assert_tsv_parity(&java_actual, &expected, &region_str);
            }
        }
    }
}

fn select_regions(
    regions: &[(String, std::path::PathBuf, std::path::PathBuf)],
    indices: Option<&[usize]>,
) -> Vec<(usize, String, std::path::PathBuf, std::path::PathBuf)> {
    match indices {
        Some(indices) => indices
            .iter()
            .map(|&index| {
                let (region, bam_path, ref_path) = regions.get(index).unwrap_or_else(|| {
                    panic!(
                        "Requested region index {index} but testdata/parity_regions.tsv has only {} rows",
                        regions.len()
                    )
                });
                (index, region.clone(), bam_path.clone(), ref_path.clone())
            })
            .collect(),
        None => regions
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, (region, bam_path, ref_path))| (index, region, bam_path, ref_path))
            .collect(),
    }
}

fn run_simple_mode_region_with_config(
    region_str: &str,
    bam_path: &str,
    ref_path: &str,
    chr_lengths: HashMap<String, i32>,
    config: Configuration,
) -> String {
    let output = {
        let _guard =
            common::init_test_scope_with_config(config, bam_path, ref_path, chr_lengths.clone());
        let mut region = common::parse_region(region_str);

        region.gene = region.chr.clone();

        let reference_resource = ReferenceResource::new(ref_path, 1200, 0, chr_lengths, false);
        let simple_mode = SimpleMode::new(vec![vec![region]], reference_resource);
        let captured = Arc::new(Mutex::new(String::new()));
        GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Buffer(captured.clone()));
        simple_mode.not_parallel();
        take_captured_output(&captured)
    };

    output
}

fn take_captured_output(buffer: &Arc<Mutex<String>>) -> String {
    let mut output = buffer.lock().unwrap_or_else(|error| error.into_inner());
    std::mem::take(&mut *output)
}


fn assert_config_matches_java_flags(
    preset_name: &str,
    java_flags: &HashMap<String, String>,
    config: &Configuration,
    defaults: &Configuration,
) {
    assert_float_flag(preset_name, java_flags, "-f", config.freq, defaults.freq, "freq");
    assert_int_flag(preset_name, java_flags, "-r", config.minr, defaults.minr, "minr");
    assert_float_flag(
        preset_name,
        java_flags,
        "-q",
        config.goodq,
        defaults.goodq,
        "goodq",
    );
    assert_int_flag(
        preset_name,
        java_flags,
        "-m",
        config.mismatch,
        defaults.mismatch,
        "mismatch",
    );
    assert_int_flag(preset_name, java_flags, "-X", config.vext, defaults.vext, "vext");
    assert_int_flag(
        preset_name,
        java_flags,
        "-B",
        config.min_bias_reads,
        defaults.min_bias_reads,
        "min_bias_reads",
    );

    assert_eq!(
        config.print_header, defaults.print_header,
        "Preset {preset_name} unexpectedly changed print_header"
    );
    assert_eq!(
        config.delimiter, defaults.delimiter,
        "Preset {preset_name} unexpectedly changed delimiter"
    );
    assert_eq!(
        config.bed, defaults.bed,
        "Preset {preset_name} unexpectedly changed bed"
    );
    assert_eq!(
        config.number_nucleotide_to_extend, defaults.number_nucleotide_to_extend,
        "Preset {preset_name} unexpectedly changed number_nucleotide_to_extend"
    );
    assert_eq!(
        config.zero_based, defaults.zero_based,
        "Preset {preset_name} unexpectedly changed zero_based"
    );
    assert_eq!(
        config.amplicon_based_calling, defaults.amplicon_based_calling,
        "Preset {preset_name} unexpectedly changed amplicon_based_calling"
    );
    assert_eq!(
        config.column_for_chromosome, defaults.column_for_chromosome,
        "Preset {preset_name} unexpectedly changed column_for_chromosome"
    );
    assert_eq!(
        config.bed_row_format, defaults.bed_row_format,
        "Preset {preset_name} unexpectedly changed bed_row_format"
    );
    assert_eq!(
        config.sample_name_regexp, defaults.sample_name_regexp,
        "Preset {preset_name} unexpectedly changed sample_name_regexp"
    );
    assert_eq!(
        config.sample_name, defaults.sample_name,
        "Preset {preset_name} unexpectedly changed sample_name"
    );
    assert_eq!(
        config.fasta, defaults.fasta,
        "Preset {preset_name} unexpectedly changed fasta"
    );
    assert_eq!(
        config.bam, defaults.bam,
        "Preset {preset_name} unexpectedly changed bam"
    );
    assert_eq!(
        config.downsampling, defaults.downsampling,
        "Preset {preset_name} unexpectedly changed downsampling"
    );
    assert_eq!(
        config.chromosome_name_is_number,
        defaults.chromosome_name_is_number,
        "Preset {preset_name} unexpectedly changed chromosome_name_is_number"
    );
    assert_eq!(
        config.mapping_quality, defaults.mapping_quality,
        "Preset {preset_name} unexpectedly changed mapping_quality"
    );
    assert_eq!(
        config.remove_duplicated_reads, defaults.remove_duplicated_reads,
        "Preset {preset_name} unexpectedly changed remove_duplicated_reads"
    );
    assert_eq!(config.y, defaults.y, "Preset {preset_name} unexpectedly changed y");
    assert_eq!(
        config.trim_bases_after, defaults.trim_bases_after,
        "Preset {preset_name} unexpectedly changed trim_bases_after"
    );
    assert_eq!(
        config.perform_local_realignment, defaults.perform_local_realignment,
        "Preset {preset_name} unexpectedly changed perform_local_realignment"
    );
    assert_eq!(
        config.indelsize, defaults.indelsize,
        "Preset {preset_name} unexpectedly changed indelsize"
    );
    assert_float_eq(
        config.bias,
        defaults.bias,
        &format!("Preset {preset_name} unexpectedly changed bias"),
    );
    assert_eq!(
        config.debug, defaults.debug,
        "Preset {preset_name} unexpectedly changed debug"
    );
    assert_eq!(
        config.move_indels_to_3, defaults.move_indels_to_3,
        "Preset {preset_name} unexpectedly changed move_indels_to_3"
    );
    assert_eq!(
        config.samfilter, defaults.samfilter,
        "Preset {preset_name} unexpectedly changed samfilter"
    );
    assert_eq!(
        config.region_of_interest, defaults.region_of_interest,
        "Preset {preset_name} unexpectedly changed region_of_interest"
    );
    assert_eq!(
        config.read_pos_filter, defaults.read_pos_filter,
        "Preset {preset_name} unexpectedly changed read_pos_filter"
    );
    assert_float_eq(
        config.qratio,
        defaults.qratio,
        &format!("Preset {preset_name} unexpectedly changed qratio"),
    );
    assert_float_eq(
        config.mapq,
        defaults.mapq,
        &format!("Preset {preset_name} unexpectedly changed mapq"),
    );
    assert_eq!(
        config.do_pileup, defaults.do_pileup,
        "Preset {preset_name} unexpectedly changed do_pileup"
    );
    assert_float_eq(
        config.lofreq,
        defaults.lofreq,
        &format!("Preset {preset_name} unexpectedly changed lofreq"),
    );
    assert_eq!(
        config.minmatch, defaults.minmatch,
        "Preset {preset_name} unexpectedly changed minmatch"
    );
    assert_eq!(
        config.output_splicing, defaults.output_splicing,
        "Preset {preset_name} unexpectedly changed output_splicing"
    );
    assert_eq!(
        config.validation_stringency, defaults.validation_stringency,
        "Preset {preset_name} unexpectedly changed validation_stringency"
    );
    assert_eq!(
        config.include_n_in_total_depth, defaults.include_n_in_total_depth,
        "Preset {preset_name} unexpectedly changed include_n_in_total_depth"
    );
    assert_eq!(
        config.unique_mode_alignment_enabled,
        defaults.unique_mode_alignment_enabled,
        "Preset {preset_name} unexpectedly changed unique_mode_alignment_enabled"
    );
    assert_eq!(
        config.unique_mode_second_in_pair_enabled,
        defaults.unique_mode_second_in_pair_enabled,
        "Preset {preset_name} unexpectedly changed unique_mode_second_in_pair_enabled"
    );
    assert_eq!(
        config.threads, defaults.threads,
        "Preset {preset_name} unexpectedly changed threads"
    );
    assert_eq!(
        config.chimeric, defaults.chimeric,
        "Preset {preset_name} unexpectedly changed chimeric"
    );
    assert_eq!(
        config.disable_sv, defaults.disable_sv,
        "Preset {preset_name} unexpectedly changed disable_sv"
    );
    assert_eq!(
        config.delete_duplicate_variants, defaults.delete_duplicate_variants,
        "Preset {preset_name} unexpectedly changed delete_duplicate_variants"
    );
    assert_eq!(
        config.fisher, defaults.fisher,
        "Preset {preset_name} unexpectedly changed fisher"
    );
    assert_eq!(
        config.inssize, defaults.inssize,
        "Preset {preset_name} unexpectedly changed inssize"
    );
    assert_eq!(
        config.insstd, defaults.insstd,
        "Preset {preset_name} unexpectedly changed insstd"
    );
    assert_eq!(
        config.insstdamt, defaults.insstdamt,
        "Preset {preset_name} unexpectedly changed insstdamt"
    );
    assert_eq!(
        config.svminlen, defaults.svminlen,
        "Preset {preset_name} unexpectedly changed svminlen"
    );
    assert_eq!(
        config.reference_extension, defaults.reference_extension,
        "Preset {preset_name} unexpectedly changed reference_extension"
    );
    assert_eq!(
        config.printer_type, defaults.printer_type,
        "Preset {preset_name} unexpectedly changed printer_type"
    );
    assert_eq!(
        config.exception_counter.load(Ordering::Relaxed),
        defaults.exception_counter.load(Ordering::Relaxed),
        "Preset {preset_name} unexpectedly changed exception_counter"
    );
    assert_eq!(
        config.adaptor, defaults.adaptor,
        "Preset {preset_name} unexpectedly changed adaptor"
    );
    assert_eq!(
        config.crispr_filtering_bp, defaults.crispr_filtering_bp,
        "Preset {preset_name} unexpectedly changed crispr_filtering_bp"
    );
    assert_eq!(
        config.crispr_cutting_site, defaults.crispr_cutting_site,
        "Preset {preset_name} unexpectedly changed crispr_cutting_site"
    );
    assert_float_eq(
        config.monomer_msi_frequency,
        defaults.monomer_msi_frequency,
        &format!("Preset {preset_name} unexpectedly changed monomer_msi_frequency"),
    );
    assert_float_eq(
        config.non_monomer_msi_frequency,
        defaults.non_monomer_msi_frequency,
        &format!("Preset {preset_name} unexpectedly changed non_monomer_msi_frequency"),
    );
}

fn assert_int_flag(
    preset_name: &str,
    java_flags: &HashMap<String, String>,
    flag: &str,
    actual: i32,
    default_value: i32,
    field_name: &str,
) {
    let expected = java_flags
        .get(flag)
        .map(|value| {
            value.parse::<i32>().unwrap_or_else(|error| {
                panic!(
                    "Preset {preset_name} had invalid integer for {flag}: {value} ({error})"
                )
            })
        })
        .unwrap_or(default_value);

    assert_eq!(
        actual, expected,
        "Preset {preset_name} field {field_name} did not match {flag}"
    );
}

fn assert_float_flag(
    preset_name: &str,
    java_flags: &HashMap<String, String>,
    flag: &str,
    actual: f64,
    default_value: f64,
    field_name: &str,
) {
    let expected = java_flags
        .get(flag)
        .map(|value| {
            value.parse::<f64>().unwrap_or_else(|error| {
                panic!(
                    "Preset {preset_name} had invalid float for {flag}: {value} ({error})"
                )
            })
        })
        .unwrap_or(default_value);

    assert_float_eq(
        actual,
        expected,
        &format!("Preset {preset_name} field {field_name} did not match {flag}"),
    );
}

fn assert_float_eq(actual: f64, expected: f64, message: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1e-9,
        "{message}: expected {expected}, got {actual}, delta {delta}"
    );
}
