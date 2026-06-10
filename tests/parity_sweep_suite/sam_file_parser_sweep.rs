use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::bounded;
use rayon::prelude::*;
use vardict_rs::data::{InitialData, Region};
use vardict_rs::mods::sam_file_parser::sam_file_parser_process;
use vardict_rs::reference::{Reference, ReferenceResource};
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;
const NUM_THREADS: usize = 10;

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
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
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

/// A pre-read tile from a v2 archive, ready for parallel processing.
struct Tile {
    bam_tag: String,
    region_str: String,
    data: String,
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

    super::common::check_sweep_manifest();
    let archive_root = base.join("v2").join("sam_file_parser");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_sam_file_parser_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = super::common::discover_v2_archives(&base, "sam_file_parser");
    if archives.is_empty() {
        eprintln!(
            "parity_sam_file_parser_sweep: skipping, no v2 archives discovered under {}",
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
                    "  [sam_file_parser] producer: archive {}/{total_archives}",
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
                let region = super::common::parse_region(&tile.region_str);
                let actual_result = collect_sam_file_parser_result(
                    bam_path,
                    &region,
                    Arc::clone(&reference_resource),
                );
                let actual_json = serde_json::to_string(&actual_result).unwrap_or_else(|error| {
                    panic!(
                        "Failed to serialize output for {}: {error}",
                        tile.region_str
                    )
                });

                let count = tested.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 10000 == 0 {
                    eprintln!(
                        "  [sam_file_parser] progress: {count} tested, {} failures",
                        failure_count.load(Ordering::Relaxed)
                    );
                }

                if let Some(message) = super::common::assert_v2_module_parity(
                    "sam_file_parser",
                    &tile.region_str,
                    &actual_json,
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
        "parity_sam_file_parser_sweep: tested={final_tested}, archives={total_archives}/{total_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_sam_file_parser_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        final_tested > 0,
        "No v2 sweep fixtures found for sam_file_parser. Run: scripts/sweep_fixtures.sh"
    );
}
