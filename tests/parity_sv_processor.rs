mod common;

use std::collections::HashMap;
use std::sync::Arc;

use vardict_rs::data::{RealignedVariationData, Sclip, VariationMap};
use vardict_rs::mods::structural_variants_processor;
use vardict_rs::reference::ReferenceResource;

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
fn parity_sv_processor_all_regions() {
    let regions = common::load_region_config();

    for (region_str, bam_path, ref_path) in &regions {
        let region = common::parse_region(region_str);
        let fai_path = format!("{}.fai", ref_path.display());
        let chr_lengths = load_chr_lengths(&fai_path);
        let _guard = common::init_test_scope(chr_lengths.clone());

        let reference_resource = Arc::new(ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths,
            false,
        ));

        // Load reference from FASTA (mutable — process() takes &mut Reference)
        let mut reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));

        // Load realigner golden as input
        let r_golden = common::load_golden_data("realigner", region_str);
        let mut data: RealignedVariationData =
            serde_json::from_str(&r_golden).unwrap_or_else(|e| {
                panic!("Failed to deserialize realigner golden for {region_str}: {e}")
            });

        let bam_str = bam_path.to_str().unwrap();
        let bams: Option<Vec<String>> = Some(vec![bam_str.to_string()]);
        let splice: Option<std::collections::BTreeSet<String>> = None;

        // Empty prev_* arguments (single-region, no prior segment)
        let mut prev_non_insertion_variants: HashMap<i32, VariationMap> = HashMap::new();
        let mut prev_ref_coverage: HashMap<i32, i32> = HashMap::new();
        let mut prev_soft_clips_3_end: HashMap<i32, Sclip> = HashMap::new();
        let mut prev_soft_clips_5_end: HashMap<i32, Sclip> = HashMap::new();
        let prev_reference_sequences: HashMap<i32, u8> = HashMap::new();
        let prev_chr = "";
        let prev_max_read_length: i32 = 0;

        structural_variants_processor::process(
            &mut data,
            &mut reference,
            &reference_resource,
            &region,
            &bams,
            &splice,
            &mut prev_non_insertion_variants,
            &mut prev_ref_coverage,
            &mut prev_soft_clips_3_end,
            &mut prev_soft_clips_5_end,
            &prev_reference_sequences,
            prev_chr,
            prev_max_read_length,
        );

        let result_json = serde_json::to_string(&data).unwrap_or_else(|e| {
            panic!("Failed to serialize sv_processor output for {region_str}: {e}")
        });

        common::assert_module_parity("sv_processor", region_str, &result_json);
    }
}
