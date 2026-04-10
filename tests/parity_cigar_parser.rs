mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rust_htslib::bam::{self, Read as BamRead};

use vardict_rs::data::InitialData;
use vardict_rs::mods::cigar_parser::CigarParser;
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
fn parity_cigar_parser_all_regions() {
    let regions = common::load_region_config();

    for (region_str, bam_path, ref_path) in &regions {
        let _guard = common::init_test_scope();

        let region = common::parse_region(region_str);
        let fai_path = format!("{}.fai", ref_path.display());
        let chr_lengths = load_chr_lengths(&fai_path);

        // Build ReferenceResource with real chr_lengths so FASTA loading works
        let reference_resource = ReferenceResource::new(
            ref_path.to_str().unwrap(),
            1200, // default reference_extension
            0,    // default number_nucleotide_to_extend
            chr_lengths,
            false,
        );

        // Load reference from FASTA
        let reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|e| panic!("Failed to load reference for {region_str}: {e}"));
        let reference = Arc::new(reference);

        // Build RecordPreprocessor through SAMFileParser pipeline
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

        // Extract preprocessor data and set up CigarParser
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

        // Collect records from preprocessor into an iterator
        let mut records: Vec<bam::Record> = Vec::new();
        while let Some(record) = preprocessor.next_record() {
            records.push(record);
        }
        preprocessor.close();

        // Get header from a fresh reader for CigarParser
        let header_reader = bam::IndexedReader::from_path(bam_str)
            .unwrap_or_else(|e| panic!("Failed to open BAM {bam_str}: {e}"));
        let header = header_reader.header().to_owned();

        let mut record_iter = records.into_iter();
        let result = parser.process(&mut record_iter, &header, &chr_name);

        let result_json = serde_json::to_string(&result)
            .unwrap_or_else(|e| panic!("Failed to serialize CigarParser output for {region_str}: {e}"));

        common::assert_module_parity("cigar_parser", region_str, &result_json);
    }
}
