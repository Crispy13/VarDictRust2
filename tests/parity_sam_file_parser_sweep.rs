mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vardict_rs::data::{InitialData, Region};
use vardict_rs::mods::sam_file_parser::sam_file_parser_process;
use vardict_rs::reference::{Reference, ReferenceResource};
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;

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
#[ignore = "Sweep gate: SAMFileParser full-sweep parity"]
fn parity_sam_file_parser_sweep() {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("tmp/sweep_fixtures"));
    if !base.is_dir() {
        eprintln!(
            "parity_sam_file_parser_sweep: skipping, no sweep fixtures at {}",
            base.display()
        );
        return;
    }

    common::check_sweep_manifest();
    let archive_root = base.join("v2").join("sam_file_parser");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_sam_file_parser_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = common::discover_v2_archives(&base, "sam_file_parser");
    if archives.is_empty() {
        eprintln!(
            "parity_sam_file_parser_sweep: skipping, no v2 archives discovered under {}",
            archive_root.display()
        );
        return;
    }

    let total_archives = archives.len();
    let (_, first_ref) = common::bam_tag_lookup(&archives[0].0);
    let fai_path = format!("{first_ref}.fai");
    let chr_lengths = common::load_chr_lengths(&fai_path);
    let _guard = common::init_test_scope(chr_lengths.clone());

    let mut failures = Vec::new();
    let mut tested = 0usize;
    let mut completed_archives = 0usize;

    'archives: for (bam_tag, _chrom, archive_path) in archives {
        completed_archives += 1;
        let (bam_path, ref_path) = common::bam_tag_lookup(&bam_tag);
        let reference_resource = Arc::new(ReferenceResource::new(
            ref_path,
            1200,
            0,
            chr_lengths.clone(),
            false,
        ));
        let mut archive_reader = common::V2ArchiveReader::new(&archive_path);

        for line in &mut archive_reader {
            tested += 1;
            if tested % 1000 == 0 {
                eprintln!(
                    "  [sam_file_parser] progress: {tested} tested, {} failures, archive {completed_archives}/{total_archives}",
                    failures.len()
                );
            }

            let region_str = line.region_str();
            let region = common::parse_region(&region_str);
            let actual_result = collect_sam_file_parser_result(
                bam_path,
                &region,
                Arc::clone(&reference_resource),
            );
            let actual_json = serde_json::to_string(&actual_result)
                .unwrap_or_else(|error| panic!("Failed to serialize output for {region_str}: {error}"));

            if let Some(message) = common::assert_v2_module_parity(
                "sam_file_parser",
                &region_str,
                &actual_json,
                &line.data,
            ) {
                failures.push(message);
                if failures.len() >= MAX_FAILURES {
                    eprintln!(
                        "  [sam_file_parser] Reached {MAX_FAILURES} failures, stopping early"
                    );
                    break 'archives;
                }
            }
        }
    }

    eprintln!(
        "parity_sam_file_parser_sweep: tested={tested}, archives={completed_archives}/{total_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_sam_file_parser_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No v2 sweep fixtures found for sam_file_parser. Run: scripts/sweep_fixtures.sh"
    );
}