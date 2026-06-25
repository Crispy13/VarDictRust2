use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use vardict_rs::prelude::HashSet;

use crossbeam_channel::bounded;
use rayon::prelude::*;
use rust_htslib::bam::{self, Read as BamRead};

use vardict_rs::data::{InitialData, PositionMap};
use vardict_rs::mods::cigar_parser::CigarParser;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;
const NUM_THREADS: usize = 10;

/// A pre-read tile from a v2 archive, ready for parallel processing.
struct Tile {
    bam_tag: String,
    region_str: String,
    data: String,
}

#[test]
#[ignore = "Sweep gate: CigarParser full-sweep parity"]
fn parity_cigar_parser_sweep() {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("tmp/sweep_fixtures"));
    if !base.is_dir() {
        eprintln!(
            "parity_cigar_parser_sweep: skipping, no sweep fixtures at {}",
            base.display()
        );
        return;
    }

    super::common::check_sweep_manifest();
    let archive_root = base.join("v2").join("cigar_parser");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_cigar_parser_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = super::common::discover_v2_archives(&base, "cigar_parser");
    if archives.is_empty() {
        eprintln!(
            "parity_cigar_parser_sweep: skipping, no v2 archives discovered under {}",
            archive_root.display()
        );
        return;
    }

    let total_archives = archives.len();
    let (first_bam, first_ref) = super::common::bam_tag_lookup(&archives[0].0);
    let fai_path = format!("{first_ref}.fai");
    let chr_lengths = super::common::load_chr_lengths(&fai_path);
    let _guard =
        super::common::init_test_scope_with_bam_global(first_bam, first_ref, chr_lengths.clone());

    let (sender, receiver) = bounded::<Tile>(10_000);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(NUM_THREADS)
        .build()
        .expect("Failed to build rayon thread pool");

    let tested = AtomicUsize::new(0);
    let failure_count = AtomicUsize::new(0);

    let producer = std::thread::spawn(move || {
        for (idx, (bam_tag, _chrom, archive_path)) in archives.into_iter().enumerate() {
            let mut archive_reader = super::common::V2ArchiveReader::new(&archive_path);
            for line in &mut archive_reader {
                let tile = Tile {
                    bam_tag: bam_tag.clone(),
                    region_str: line.region_str(),
                    data: line.data,
                };
                if sender.send(tile).is_err() {
                    return;
                }
            }
            if (idx + 1) % 10 == 0 || idx + 1 == total_archives {
                eprintln!(
                    "  [cigar_parser] producer: archive {}/{total_archives}",
                    idx + 1
                );
            }
        }
    });

    let failures: Vec<String> = pool.install(|| {
        receiver
            .iter()
            .par_bridge()
            .filter_map(|tile| {
                if failure_count.load(Ordering::Relaxed) >= MAX_FAILURES {
                    return None;
                }

                let (bam_path, ref_path) = super::common::bam_tag_lookup(&tile.bam_tag);
                let reference_resource = Arc::new(ReferenceResource::new(
                    ref_path,
                    1200,
                    0,
                    chr_lengths.clone(),
                    false,
                ));
                let header_reader = bam::IndexedReader::from_path(bam_path)
                    .unwrap_or_else(|error| panic!("Failed to open BAM {bam_path}: {error}"));
                let header = header_reader.header().to_owned();
                let region = super::common::parse_region(&tile.region_str);

                let reference = reference_resource
                    .get_reference(&region)
                    .unwrap_or_else(|error| {
                        panic!("Failed to load reference for {}: {error}", tile.region_str)
                    });
                let reference = Arc::new(reference);

                let initial_data = InitialData::new(
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    PositionMap::default(),
                    PositionMap::default(),
                );
                let scope = Scope::new(
                    bam_path,
                    region.clone(),
                    Arc::clone(&reference),
                    Arc::clone(&reference_resource),
                    0,
                    HashSet::default(),
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
                    &HashSet::default(),
                    0,
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    PositionMap::default(),
                    PositionMap::default(),
                    0, // NOTE: total_reads=0 - real value from RecordPreprocessor not available in sweep. Affects duprate only.
                    0, // NOTE: duplicate_reads=0 - same limitation.
                );

                let mut records: Vec<bam::Record> = Vec::new();
                while let Some(record) = preprocessor.next_record() {
                    records.push(record);
                }
                preprocessor.close();

                let mut record_iter = records.into_iter();
                let result = parser.process(&mut record_iter, &header, &chr_name);
                let result_json = serde_json::to_string(&result).unwrap_or_else(|error| {
                    panic!("Failed to serialize for {}: {error}", tile.region_str)
                });

                let count = tested.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 10000 == 0 {
                    eprintln!(
                        "  [cigar_parser] progress: {count} tested, {} failures",
                        failure_count.load(Ordering::Relaxed)
                    );
                }

                if let Some(message) = super::common::assert_v2_module_parity(
                    "cigar_parser",
                    &tile.region_str,
                    &result_json,
                    &tile.data,
                ) {
                    failure_count.fetch_add(1, Ordering::Relaxed);
                    Some(message)
                } else {
                    None
                }
            })
            .collect()
    });

    producer.join().expect("Producer thread panicked");

    let final_tested = tested.load(Ordering::Relaxed);

    eprintln!(
        "parity_cigar_parser_sweep: tested={final_tested}, archives={total_archives}/{total_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_cigar_parser_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        final_tested > 0,
        "No v2 sweep fixtures found for cigar_parser. Run: scripts/sweep_fixtures.sh"
    );
}
