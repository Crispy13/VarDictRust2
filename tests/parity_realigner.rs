mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

        // Load reference from FASTA
        let reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
        let reference = Arc::new(reference);

        // Load CigarParser golden as input
        let cp_golden = common::load_golden_data("cigar_parser", region_str);
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
            HashSet::new(),
            VariantPrinter::Out,
            variation_data,
        );

        let result_scope = variation_realigner::process(scope);

        let result_json = serde_json::to_string(&result_scope.data).unwrap_or_else(|e| {
            panic!("Failed to serialize realigner output for {region_str}: {e}")
        });

        common::assert_module_parity("realigner", region_str, &result_json);
    }
}
