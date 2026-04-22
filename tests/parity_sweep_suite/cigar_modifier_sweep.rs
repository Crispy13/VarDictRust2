use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::bounded;
use rayon::prelude::*;
use vardict_rs::data::Region;
use vardict_rs::mods::cigar_modifier::modify_cigar;
use vardict_rs::reference::{Reference, ReferenceResource};

const MAX_FAILURES: usize = 10;
const NUM_THREADS: usize = 10;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CigarModInput {
    read_name: String,
    position: i32,
    cigar: String,
    query_sequence: String,
    query_quality: String,
    indel: i32,
    max_read_length: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CigarModOutput {
    position: i32,
    cigar: String,
    query_sequence: String,
    query_quality: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
struct CigarModRecord {
    input: CigarModInput,
    output: CigarModOutput,
}

fn rebuild_cigar_modifier_output(
    golden_records: Vec<CigarModRecord>,
    reference: &Reference,
    region: &Region,
) -> Vec<CigarModRecord> {
    golden_records
        .into_iter()
        .map(|record| {
            let actual = modify_cigar(
                record.input.position,
                &record.input.cigar,
                &record.input.query_sequence,
                &record.input.query_quality,
                reference,
                record.input.indel,
                region,
                record.input.max_read_length,
            );

            CigarModRecord {
                input: record.input,
                output: CigarModOutput {
                    position: actual.position,
                    cigar: actual.cigar,
                    query_sequence: actual.query_sequence,
                    query_quality: actual.query_quality,
                },
            }
        })
        .collect()
}

/// A pre-read tile from a v2 archive, ready for parallel processing.
struct Tile {
    bam_tag: String,
    region_str: String,
    data: String,
}

#[test]
#[ignore = "Sweep gate: CigarModifier full-sweep parity"]
fn parity_cigar_modifier_sweep() {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("tmp/sweep_fixtures"));
    if !base.is_dir() {
        eprintln!(
            "parity_cigar_modifier_sweep: skipping, no sweep fixtures at {}",
            base.display()
        );
        return;
    }

    super::common::check_sweep_manifest();
    let archive_root = base.join("v2").join("cigar_modifier");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_cigar_modifier_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = super::common::discover_v2_archives(&base, "cigar_modifier");
    if archives.is_empty() {
        eprintln!(
            "parity_cigar_modifier_sweep: skipping, no v2 archives discovered under {}",
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
                    "  [cigar_modifier] producer: archive {}/{total_archives}",
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

                let (_, ref_path) = super::common::bam_tag_lookup(&tile.bam_tag);
                let reference_resource =
                    ReferenceResource::new(ref_path, 1200, 0, chr_lengths.clone(), false);
                let region = super::common::parse_region(&tile.region_str);
                let reference = reference_resource
                    .get_reference(&region)
                    .unwrap_or_else(|error| {
                        panic!("Failed to load reference for {}: {error}", tile.region_str)
                    });
                let golden_records: Vec<CigarModRecord> = serde_json::from_str(&tile.data)
                    .unwrap_or_else(|error| {
                        panic!(
                            "Failed to parse archive data for {}: {error}",
                            tile.region_str
                        )
                    });
                let actual_records =
                    rebuild_cigar_modifier_output(golden_records, &reference, &region);
                let actual_json = serde_json::to_string(&actual_records).unwrap_or_else(|error| {
                    panic!(
                        "Failed to serialize output for {}: {error}",
                        tile.region_str
                    )
                });

                let count = tested.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 10000 == 0 {
                    eprintln!(
                        "  [cigar_modifier] progress: {count} tested, {} failures",
                        failure_count.load(Ordering::Relaxed)
                    );
                }

                if let Some(message) = super::common::assert_v2_module_parity(
                    "cigar_modifier",
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
        "parity_cigar_modifier_sweep: tested={final_tested}, archives={total_archives}/{total_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_cigar_modifier_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        final_tested > 0,
        "No v2 sweep fixtures found for cigar_modifier. Run: scripts/sweep_fixtures.sh"
    );
}