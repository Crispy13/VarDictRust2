mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vardict_rs::data::{InitialData, Region};
use vardict_rs::mods::sam_file_parser::sam_file_parser_process;
use vardict_rs::reference::{Reference, ReferenceResource};
use vardict_rs::scope::{Scope, VariantPrinter};

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecordFingerprint {
    read_name: String,
    start: i32,
    cigar: String,
    flags: i32,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SamFileParserResult {
    total_reads: i32,
    duplicate_reads: i32,
    records: Vec<RecordFingerprint>,
}

fn collect_sam_file_parser_result(
    bam_path: &str,
    region: &Region,
    reference_resource: Arc<ReferenceResource>,
) -> SamFileParserResult {
    let initial_data = InitialData::new(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
    );
    let scope = Scope::new(
        bam_path,
        region.clone(),
        Arc::new(Reference::new()),
        reference_resource,
        0,
        HashSet::new(),
        VariantPrinter::Out,
        initial_data,
    );
    let scope = sam_file_parser_process(scope);

    let mut preprocessor = scope.data;
    let mut records = Vec::new();
    while let Some(record) = preprocessor.next_record() {
        records.push(RecordFingerprint {
            read_name: std::str::from_utf8(record.qname()).unwrap().to_string(),
            start: record.pos() as i32 + 1,
            cigar: record.cigar().to_string(),
            flags: record.flags() as i32,
        });
    }

    let result = SamFileParserResult {
        total_reads: preprocessor.total_reads,
        duplicate_reads: preprocessor.duplicate_reads,
        records,
    };
    preprocessor.close();
    result
}

#[test]
fn parity_sam_file_parser_all_regions() {
    let regions = common::load_region_config();

    for (region_str, bam_path, ref_path) in &regions {
        let region = common::parse_region(region_str);
        let fai_path = format!("{}.fai", ref_path.display());
        let chr_lengths = common::load_chr_lengths(&fai_path);
        let _guard = common::init_test_scope(chr_lengths.clone());

        let reference_resource = Arc::new(ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths,
            false,
        ));
        let actual_result = collect_sam_file_parser_result(
            bam_path.to_str().unwrap(),
            &region,
            reference_resource,
        );
        let actual_json = serde_json::to_string(&actual_result)
            .unwrap_or_else(|error| panic!("Failed to serialize output for {region_str}: {error}"));

        common::assert_module_parity("sam_file_parser", region_str, &actual_json);
    }
}