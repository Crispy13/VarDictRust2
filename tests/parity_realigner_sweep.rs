mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::bounded;
use rayon::prelude::*;
use vardict_rs::data::VariationData;
use vardict_rs::mods::variation_realigner;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;
const NUM_THREADS: usize = 10;

/// A pre-read tile from paired dependency and module v2 archives.
struct Tile {
    bam_tag: String,
    region_str: String,
    dep_data: String,
    data: String,
}

#[test]
#[ignore = "Sweep gate: Realigner full-sweep parity"]
fn parity_realigner_sweep() {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("tmp/sweep_fixtures"));
    if !base.is_dir() {
        eprintln!(
            "parity_realigner_sweep: skipping, no sweep fixtures at {}",
            base.display()
        );
        return;
    }

    common::check_sweep_manifest();
    let archive_root = base.join("v2").join("realigner");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_realigner_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = common::discover_v2_archives(&base, "realigner");
    if archives.is_empty() {
        eprintln!(
            "parity_realigner_sweep: skipping, no v2 archives discovered under {}",
            archive_root.display()
        );
        return;
    }

    let total_archives = archives.len();
    let (_, first_ref) = common::bam_tag_lookup(&archives[0].0);
    let fai_path = format!("{first_ref}.fai");
    let chr_lengths = common::load_chr_lengths(&fai_path);
    let _guard = common::init_test_scope(chr_lengths.clone());

    let (sender, receiver) = bounded::<Tile>(10_000);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(NUM_THREADS)
        .build()
        .expect("Failed to build rayon thread pool");

    let tested = AtomicUsize::new(0);
    let failure_count = AtomicUsize::new(0);

    let producer = std::thread::spawn(move || {
        let mut skipped_archives = 0usize;
        for (idx, (bam_tag, chrom, archive_path)) in archives.into_iter().enumerate() {
            let dep_archive_path = base
                .join("v2")
                .join("cigar_parser")
                .join(&bam_tag)
                .join(format!("{chrom}.jsonl.zst"));
            if !dep_archive_path.is_file() {
                skipped_archives += 1;
                eprintln!(
                    "  [realigner] missing dependency archive {}, skipping",
                    dep_archive_path.display()
                );
                continue;
            }

            let dep_reader = common::V2ArchiveReader::new(&dep_archive_path);
            let mod_reader = common::V2ArchiveReader::new(&archive_path);

            for (dep_line, mod_line) in dep_reader.zip(mod_reader) {
                assert_eq!(
                    dep_line.region_str(),
                    mod_line.region_str(),
                    "Lock-step mismatch for realigner {} {}: dep={} vs mod={}",
                    bam_tag,
                    chrom,
                    dep_line.region_str(),
                    mod_line.region_str()
                );

                let tile = Tile {
                    bam_tag: bam_tag.clone(),
                    region_str: mod_line.region_str(),
                    dep_data: dep_line.data,
                    data: mod_line.data,
                };
                if sender.send(tile).is_err() {
                    return skipped_archives;
                }
            }

            if (idx + 1) % 10 == 0 || idx + 1 == total_archives {
                eprintln!(
                    "  [realigner] producer: archive {}/{total_archives}",
                    idx + 1
                );
            }
        }

        skipped_archives
    });

    let failures: Vec<String> = pool.install(|| {
        receiver
            .iter()
            .par_bridge()
            .filter_map(|tile| {
                if failure_count.load(Ordering::Relaxed) >= MAX_FAILURES {
                    return None;
                }

                let (bam_path, ref_path) = common::bam_tag_lookup(&tile.bam_tag);
                let reference_resource = Arc::new(ReferenceResource::new(
                    ref_path,
                    1200,
                    0,
                    chr_lengths.clone(),
                    false,
                ));
                let region = common::parse_region(&tile.region_str);

                let reference = reference_resource
                    .get_reference(&region)
                    .unwrap_or_else(|error| {
                        panic!("Failed to load reference for {}: {error}", tile.region_str)
                    });
                let reference = Arc::new(reference);

                let variation_data: VariationData = serde_json::from_str(&tile.dep_data)
                    .unwrap_or_else(|error| {
                        panic!(
                            "Failed to deserialize cigar_parser golden for {}: {error}",
                            tile.region_str
                        )
                    });

                let scope = Scope::new(
                    bam_path,
                    region.clone(),
                    reference,
                    Arc::clone(&reference_resource),
                    variation_data.max_read_length.unwrap_or(0),
                    HashSet::new(),
                    VariantPrinter::Out,
                    variation_data,
                );

                let result_scope = variation_realigner::process(scope);
                let result_json =
                    serde_json::to_string(&result_scope.data).unwrap_or_else(|error| {
                        panic!("Failed to serialize for {}: {error}", tile.region_str)
                    });

                let count = tested.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 10000 == 0 {
                    eprintln!(
                        "  [realigner] progress: {count} tested, {} failures",
                        failure_count.load(Ordering::Relaxed)
                    );
                }

                if let Some(message) = common::assert_v2_module_parity(
                    "realigner",
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

    let skipped_archives = producer.join().expect("Producer thread panicked");

    let final_tested = tested.load(Ordering::Relaxed);

    eprintln!(
        "parity_realigner_sweep: tested={final_tested}, archives={total_archives}/{total_archives}, dependency_archives_skipped={skipped_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_realigner_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        final_tested > 0,
        "No v2 sweep fixtures found for realigner. Run: scripts/sweep_fixtures.sh"
    );
}
