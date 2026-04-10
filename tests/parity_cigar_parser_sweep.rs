mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rust_htslib::bam::{self, Read as BamRead};

use vardict_rs::data::InitialData;
use vardict_rs::mods::cigar_parser::CigarParser;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;

#[test]
#[ignore = "Sweep gate: CigarParser full-sweep parity"]
fn parity_cigar_parser_sweep() {
    common::check_sweep_manifest();
    let regions = common::load_sweep_region_config();
    let total = regions.len();

    let first_ref = regions
        .first()
        .map(|(_, _, ref_path)| ref_path)
        .unwrap_or_else(|| panic!("No sweep regions found in tmp/sweep_fixtures/regions.tsv"));
    let fai_path = format!("{}.fai", first_ref.display());
    let chr_lengths = common::load_chr_lengths(&fai_path);

    let mut failures = Vec::new();
    let mut tested = 0usize;
    let mut skipped = 0usize;

    for (region_str, bam_path, ref_path) in &regions {
        let path = common::sweep_fixture_path("cigar_parser", region_str);
        if !path.exists() {
            skipped += 1;
            continue;
        }

        tested += 1;
        if tested % 1000 == 0 {
            eprintln!(
                "  [cigar_parser] progress: {tested}/{total} tested, {} failures, {skipped} skipped",
                failures.len()
            );
        }

        let _guard = common::init_test_scope();
        let region = common::parse_region(region_str);

        let reference_resource = ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200,
            0,
            chr_lengths.clone(),
            false,
        );
        let reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));
        let reference = Arc::new(reference);

        let bam_str = bam_path.to_str().unwrap();
        let initial_data = InitialData::new(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );
        let scope = Scope::new(
            bam_str,
            region.clone(),
            Arc::clone(&reference),
            Arc::new(reference_resource),
            0,
            HashSet::new(),
            VariantPrinter::Out,
            initial_data,
        );
        let scope = vardict_rs::mods::sam_file_parser::sam_file_parser_process(scope);

        let mut preprocessor = scope.data;
        let chr_name = preprocessor.get_chr_name();
        let svflag = false;

        let mut parser = CigarParser::new(svflag);
        parser.init_from_scope(
            &region,
            &reference,
            &HashSet::new(),
            0,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            0,
            0,
        );

        let mut records: Vec<bam::Record> = Vec::new();
        while let Some(record) = preprocessor.next_record() {
            records.push(record);
        }
        preprocessor.close();

        let header_reader = bam::IndexedReader::from_path(bam_str)
            .unwrap_or_else(|error| panic!("Failed to open BAM {bam_str}: {error}"));
        let header = header_reader.header().to_owned();

        let mut record_iter = records.into_iter();
        let result = parser.process(&mut record_iter, &header, &chr_name);
        let result_json = serde_json::to_string(&result)
            .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

        if let Some(message) =
            common::assert_sweep_module_parity("cigar_parser", region_str, &result_json)
        {
            failures.push(message);
            if failures.len() >= MAX_FAILURES {
                eprintln!("  [cigar_parser] Reached {MAX_FAILURES} failures, stopping early");
                break;
            }
        }
    }

    eprintln!(
        "parity_cigar_parser_sweep: tested={tested}, skipped={skipped}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_cigar_parser_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No sweep fixtures found for cigar_parser. Run: scripts/sweep_fixtures.sh"
    );
}
