mod common;

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::atomic::Ordering;

use vardict_rs::config::Configuration;

const PUSH_INDICES: &[usize] = &[0, 1, 2, 3, 4, 35, 36, 37, 70, 71];

macro_rules! declarative_test {
    ($(($test_name:ident, $config_name:literal)),+ $(,)?) => {
        $(
            #[test]
            #[ignore = "requires generated tmp/e2e_fixtures goldens not checked into the repo"]
            fn $test_name() {
                run_config_e2e_single($config_name, Some(PUSH_INDICES));
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
    (parity_config_e2e_push_cm_fisher, "CM-FISHER"),
    (parity_config_e2e_push_t2_05, "T2-05"),
    (parity_config_e2e_push_cm_pileup, "CM-PILEUP"),
    (parity_config_e2e_push_t2_07, "T2-07"),
    (parity_config_e2e_push_t2_08, "T2-08"),
    (parity_config_e2e_push_cm_nosv, "CM-NOSV"),
    (parity_config_e2e_push_t2_10, "T2-10"),
    (parity_config_e2e_push_t3_01, "T3-01"),
    (parity_config_e2e_push_t3_02, "T3-02"),
    (parity_config_e2e_push_cm_noreal, "CM-NOREAL"),
    (parity_config_e2e_push_t3_04, "T3-04"),
    (parity_config_e2e_push_t3_05, "T3-05"),
    (parity_config_e2e_push_cm_chimeric, "CM-CHIMERIC"),
    (parity_config_e2e_push_t3_07, "T3-07"),
    (parity_config_e2e_push_t3_08, "T3-08"),
    (parity_config_e2e_push_cm_mapq30, "CM-MAPQ30"),
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
                    let actual = common::run_simple_mode_region_with_config(
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
                    let rust_actual = common::run_simple_mode_region_with_config(
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
    let region_count = common::load_region_config().len();
    let idx_list: Vec<usize> = match indices {
        Some(slice) => slice.to_vec(),
        None => (0..region_count).collect(),
    };

    for idx in idx_list {
        if let Err(message) = common::run_cell(config_name, idx) {
            panic!("{message}");
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

#[test]
fn binary_b_list_terse_format_regression() {
    let output = Command::new(env!("CARGO"))
        .args([
            "test",
            "--profile",
            "debug-release",
            "--test",
            "parity_config_e2e_cells",
            "--",
            "--list",
            "--format=terse",
        ])
        .env_remove("PARITY_REGION_INDEX")
        .env("VARDICT_IMPL", "rust")
        .output()
        .expect("failed to spawn cargo test --list");

    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("non-UTF8 stdout");
    let expected_slugs: HashSet<String> = common::CONFIG_PRESETS
        .iter()
        .map(|name| common::config_name_to_slug(name))
        .collect();
    let expected_indices: HashSet<usize> = (0..100).collect();
    let mut seen_names = HashSet::new();
    let mut slug_counts: HashMap<String, usize> = HashMap::new();
    let mut slug_indices: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut trial_count = 0usize;

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_suffix(": test") {
            assert!(
                is_valid_cell_name(name),
                "trial name does not match contract: {name}"
            );

            assert!(
                seen_names.insert(name.to_string()),
                "duplicate trial name emitted: {name}"
            );

            let (slug, region_idx) = parse_cell_slug_and_index(name)
                .expect("valid trial name must parse into slug and region index");
            *slug_counts.entry(slug.to_string()).or_default() += 1;
            slug_indices
                .entry(slug.to_string())
                .or_default()
                .insert(region_idx);
            trial_count += 1;
        }
    }

    assert_eq!(
        trial_count, 4400,
        "Phase 4 expects exactly 4400 trials; got {trial_count}"
    );

    assert_eq!(
        seen_names.len(), 4400,
        "Phase 4 expects 4400 unique trial names; got {}",
        seen_names.len()
    );
    assert_eq!(
        slug_counts.len(), expected_slugs.len(),
        "unexpected slug cardinality in Binary B list output"
    );
    assert_eq!(
        slug_indices.len(), expected_slugs.len(),
        "unexpected slug coverage cardinality in Binary B list output"
    );

    for slug in &expected_slugs {
        assert_eq!(
            slug_counts.get(slug).copied(),
            Some(100),
            "slug {slug} must appear exactly 100 times"
        );
        assert_eq!(
            slug_indices.get(slug),
            Some(&expected_indices),
            "slug {slug} must cover the complete region index set 000..=099"
        );
    }
}

fn is_valid_cell_name(name: &str) -> bool {
    parse_cell_slug_and_index(name).is_some()
}

fn parse_cell_slug_and_index(name: &str) -> Option<(&str, usize)> {
    let prefix = "parity_config_e2e_cell_";
    let Some(rest) = name.strip_prefix(prefix) else {
        return None;
    };
    let Some(index) = rest.rfind("_r") else {
        return None;
    };
    let (slug, digits) = (&rest[..index], &rest[index + 2..]);
    if slug.is_empty() {
        return None;
    }
    if !slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return None;
    }
    if digits.len() != 3 {
        return None;
    }
    if !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    Some((slug, digits.parse().ok()?))
}


fn assert_config_matches_java_flags(
    preset_name: &str,
    java_flags: &HashMap<String, String>,
    config: &Configuration,
    defaults: &Configuration,
) {
    let pileup_mode = java_flags.contains_key("-p");
    assert_float_flag(
        preset_name,
        java_flags,
        "-f",
        config.freq,
        if pileup_mode { -1.0 } else { defaults.freq },
        "freq",
    );
    assert_int_flag(
        preset_name,
        java_flags,
        "-r",
        config.minr,
        if pileup_mode { 0 } else { defaults.minr },
        "minr",
    );
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
    assert_opt_int_flag(
        preset_name,
        java_flags,
        "-Q",
        config.mapping_quality,
        defaults.mapping_quality,
        "mapping_quality",
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
    assert_int_bool_flag(
        preset_name,
        java_flags,
        "-k",
        config.perform_local_realignment,
        defaults.perform_local_realignment,
        "perform_local_realignment",
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
    assert_present_bool_flag(
        java_flags,
        "-p",
        config.do_pileup,
        defaults.do_pileup,
        preset_name,
        "do_pileup",
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
    assert_present_bool_flag(
        java_flags,
        "--chimeric",
        config.chimeric,
        defaults.chimeric,
        preset_name,
        "chimeric",
    );
    assert_present_bool_flag(
        java_flags,
        "-U",
        config.disable_sv,
        defaults.disable_sv,
        preset_name,
        "disable_sv",
    );
    assert_eq!(
        config.delete_duplicate_variants, defaults.delete_duplicate_variants,
        "Preset {preset_name} unexpectedly changed delete_duplicate_variants"
    );
    assert_present_bool_flag(
        java_flags,
        "--fisher",
        config.fisher,
        defaults.fisher,
        preset_name,
        "fisher",
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

fn assert_opt_int_flag(
    preset_name: &str,
    java_flags: &HashMap<String, String>,
    flag: &str,
    actual: Option<i32>,
    default_value: Option<i32>,
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
        .or(default_value);

    assert_eq!(
        actual, expected,
        "Preset {preset_name} field {field_name} did not match {flag}"
    );
}

fn assert_int_bool_flag(
    preset_name: &str,
    java_flags: &HashMap<String, String>,
    flag: &str,
    actual: bool,
    default_value: bool,
    field_name: &str,
) {
    let expected = java_flags
        .get(flag)
        .map(|value| match value.as_str() {
            "0" => false,
            "1" => true,
            other => panic!("Preset {preset_name} had invalid boolean int for {flag}: {other}"),
        })
        .unwrap_or(default_value);

    assert_eq!(
        actual, expected,
        "Preset {preset_name} field {field_name} did not match {flag}"
    );
}

fn assert_present_bool_flag(
    java_flags: &HashMap<String, String>,
    flag: &str,
    actual: bool,
    default_value: bool,
    preset_name: &str,
    field_name: &str,
) {
    let expected = if java_flags.contains_key(flag) {
        true
    } else {
        default_value
    };

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
