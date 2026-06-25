use std::sync::Arc;
use vardict_rs::prelude::{HashMap, HashSet};

use vardict_rs::data::VariationData;
use vardict_rs::mods::variation_realigner;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{Scope, VariantPrinter};

/// Load chromosome lengths from a .fai file into a HashMap.
fn load_chr_lengths(fai_path: &str) -> HashMap<String, i32> {
    let content = std::fs::read_to_string(fai_path)
        .unwrap_or_else(|e| panic!("Failed to read FAI file {fai_path}: {e}"));
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let chr = fields[0].to_string();
            let len: i32 = fields[1].parse().unwrap_or(0);
            (chr, len)
        })
        .collect()
}

#[test]
fn parity_realigner_all_regions() {
    let regions = super::common::load_region_config();

    for (region_str, bam_path, ref_path) in &regions {
        let region = super::common::parse_region(region_str);
        let fai_path = format!("{}.fai", ref_path.display());
        let chr_lengths = load_chr_lengths(&fai_path);
        let _guard = super::common::init_test_scope(chr_lengths.clone());

        let reference_resource = Arc::new(ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths,
            false,
        ));

        // Load reference from FASTA
        let reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
        let reference = Arc::new(reference);

        // Load CigarParser golden as input
        let cp_golden = super::common::load_golden_data("cigar_parser", region_str);
        let variation_data: VariationData = serde_json::from_str(&cp_golden).unwrap_or_else(|e| {
            panic!("Failed to deserialize cigar_parser golden for {region_str}: {e}")
        });

        let bam_str = bam_path.to_str().unwrap();
        let scope = Scope::new(
            bam_str,
            region.clone(),
            reference,
            reference_resource,
            variation_data.max_read_length.unwrap_or(0),
            HashSet::default(),
            VariantPrinter::Out,
            variation_data,
        );

        let result_scope = variation_realigner::process(scope);

        let result_json = serde_json::to_string(&result_scope.data).unwrap_or_else(|e| {
            panic!("Failed to serialize realigner output for {region_str}: {e}")
        });

        super::common::assert_module_parity("realigner", region_str, &result_json);
    }
}

#[test]
fn parity_realigner_region_1_2324084_2324612() {
    let region_str = "1:2324084-2324612";
    let bam_path = std::path::PathBuf::from(
        "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    );
    let ref_path = std::path::PathBuf::from("testdata/hs37d5.fa");

    let region = super::common::parse_region(region_str);
    let fai_path = format!("{}.fai", ref_path.display());
    let chr_lengths = load_chr_lengths(&fai_path);
    let _guard = super::common::init_test_scope(chr_lengths.clone());

    let reference_resource = Arc::new(ReferenceResource::new(
        ref_path.to_str().unwrap(),
        1200,
        0,
        chr_lengths,
        false,
    ));

    let reference = reference_resource
        .get_reference(&region)
        .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
    let reference = Arc::new(reference);

    let cp_golden = super::common::load_golden_data("cigar_parser", region_str);
    let variation_data: VariationData = serde_json::from_str(&cp_golden).unwrap_or_else(|e| {
        panic!("Failed to deserialize cigar_parser golden for {region_str}: {e}")
    });

    let bam_str = bam_path.to_str().unwrap();
    let scope = Scope::new(
        bam_str,
        region.clone(),
        reference,
        reference_resource,
        variation_data.max_read_length.unwrap_or(0),
        HashSet::default(),
        VariantPrinter::Out,
        variation_data,
    );

    let result_scope = variation_realigner::process(scope);

    let result_json = serde_json::to_string(&result_scope.data)
        .unwrap_or_else(|e| panic!("Failed to serialize realigner output for {region_str}: {e}"));

    super::common::assert_module_parity("realigner", region_str, &result_json);
}

#[test]
fn parity_realigner_region_1_9967324_9968024() {
    let region_str = "1:9967324-9968024";
    let bam_path = std::path::PathBuf::from(
        "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    );
    let ref_path = std::path::PathBuf::from("testdata/hs37d5.fa");

    let region = super::common::parse_region(region_str);
    let fai_path = format!("{}.fai", ref_path.display());
    let chr_lengths = load_chr_lengths(&fai_path);
    let _guard = super::common::init_test_scope(chr_lengths.clone());

    let reference_resource = Arc::new(ReferenceResource::new(
        ref_path.to_str().unwrap(),
        1200,
        0,
        chr_lengths,
        false,
    ));

    let reference = reference_resource
        .get_reference(&region)
        .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
    let reference = Arc::new(reference);

    let cp_golden = super::common::load_golden_data("cigar_parser", region_str);
    let variation_data: VariationData = serde_json::from_str(&cp_golden).unwrap_or_else(|e| {
        panic!("Failed to deserialize cigar_parser golden for {region_str}: {e}")
    });

    let bam_str = bam_path.to_str().unwrap();
    let scope = Scope::new(
        bam_str,
        region.clone(),
        reference,
        reference_resource,
        variation_data.max_read_length.unwrap_or(0),
        HashSet::default(),
        VariantPrinter::Out,
        variation_data,
    );

    let result_scope = variation_realigner::process(scope);

    let result_json = serde_json::to_string(&result_scope.data)
        .unwrap_or_else(|e| panic!("Failed to serialize realigner output for {region_str}: {e}"));

    super::common::assert_module_parity("realigner", region_str, &result_json);
}

#[test]
fn parity_realigner_region_1_8926126_8926826() {
    let region_str = "1:8926126-8926826";
    let bam_path = std::path::PathBuf::from(
        "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    );
    let ref_path = std::path::PathBuf::from("testdata/hs37d5.fa");

    let region = super::common::parse_region(region_str);
    let fai_path = format!("{}.fai", ref_path.display());
    let chr_lengths = load_chr_lengths(&fai_path);
    let _guard = super::common::init_test_scope(chr_lengths.clone());

    let reference_resource = Arc::new(ReferenceResource::new(
        ref_path.to_str().unwrap(),
        1200,
        0,
        chr_lengths,
        false,
    ));

    let reference = reference_resource
        .get_reference(&region)
        .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
    let reference = Arc::new(reference);

    let cp_golden = super::common::load_golden_data("cigar_parser", region_str);
    let variation_data: VariationData = serde_json::from_str(&cp_golden).unwrap_or_else(|e| {
        panic!("Failed to deserialize cigar_parser golden for {region_str}: {e}")
    });

    let bam_str = bam_path.to_str().unwrap();
    let scope = Scope::new(
        bam_str,
        region.clone(),
        reference,
        reference_resource,
        variation_data.max_read_length.unwrap_or(0),
        HashSet::default(),
        VariantPrinter::Out,
        variation_data,
    );

    let result_scope = variation_realigner::process(scope);

    let result_json = serde_json::to_string(&result_scope.data)
        .unwrap_or_else(|e| panic!("Failed to serialize realigner output for {region_str}: {e}"));

    super::common::assert_module_parity("realigner", region_str, &result_json);
}

#[test]
fn parity_realigner_config_t1_01_1_155006164_155006864() {
    let config_name = "T1-01";
    let region_str = "1:155006164-155006864";
    let bam_path = std::path::PathBuf::from(
        "testdata/151002_7001448_0359_AC7F6GANXX_Sample_HG002-EEogPU_v02-KIT-Av5_AGATGTAC_L008.posiSrt.markDup.bam",
    );
    let ref_path = std::path::PathBuf::from("testdata/hs37d5.fa");

    let region = super::common::parse_region(region_str);
    let fai_path = format!("{}.fai", ref_path.display());
    let chr_lengths = load_chr_lengths(&fai_path);
    let _guard = super::common::init_test_scope_with_config(
        super::common::config_preset(config_name),
        bam_path.to_str().unwrap(),
        ref_path.to_str().unwrap(),
        chr_lengths.clone(),
    );

    let reference_resource = Arc::new(ReferenceResource::new(
        ref_path.to_str().unwrap(),
        1200,
        0,
        chr_lengths,
        false,
    ));

    let reference = reference_resource
        .get_reference(&region)
        .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
    let reference = Arc::new(reference);

    let cp_golden = super::common::load_golden_data("cigar_parser", region_str);
    let variation_data: VariationData = serde_json::from_str(&cp_golden).unwrap_or_else(|e| {
        panic!("Failed to deserialize cigar_parser golden for {region_str}: {e}")
    });

    let bam_str = bam_path.to_str().unwrap();
    let scope = Scope::new(
        bam_str,
        region.clone(),
        reference,
        reference_resource,
        variation_data.max_read_length.unwrap_or(0),
        HashSet::default(),
        VariantPrinter::Out,
        variation_data,
    );

    let result_scope = variation_realigner::process(scope);

    let result_json = serde_json::to_string(&result_scope.data)
        .unwrap_or_else(|e| panic!("Failed to serialize realigner output for {region_str}: {e}"));
    let golden =
        super::common::load_golden_data_with_config("realigner", Some(config_name), region_str);

    assert_eq!(
        result_json, golden,
        "Parity mismatch for module=realigner, config={config_name}, region={region_str}"
    );
}
