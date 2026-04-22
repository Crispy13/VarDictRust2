use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::bounded;
use rayon::prelude::*;
use vardict_rs::data::{RealignedVariationData, Sclip, VariationMap};
use vardict_rs::mods::structural_variants_processor;
use vardict_rs::reference::ReferenceResource;

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
#[ignore = "Sweep gate: SVProcessor full-sweep parity"]
fn parity_sv_processor_sweep() {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("tmp/sweep_fixtures"));
    if !base.is_dir() {
        eprintln!(
            "parity_sv_processor_sweep: skipping, no sweep fixtures at {}",
            base.display()
        );
        return;
    }

    super::common::check_sweep_manifest();
    let archive_root = base.join("v2").join("sv_processor");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_sv_processor_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = super::common::discover_v2_archives(&base, "sv_processor");
    if archives.is_empty() {
        eprintln!(
            "parity_sv_processor_sweep: skipping, no v2 archives discovered under {}",
            archive_root.display()
        );
        return;
    }

    let total_archives = archives.len();
    let (first_bam, first_ref) = super::common::bam_tag_lookup(&archives[0].0);
    let fai_path = format!("{first_ref}.fai");
    let chr_lengths = super::common::load_chr_lengths(&fai_path);
    let _guard = super::common::init_test_scope_with_bam_global(first_bam, first_ref, chr_lengths.clone());

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
                .join("realigner")
                .join(&bam_tag)
                .join(format!("{chrom}.jsonl.zst"));
            if !dep_archive_path.is_file() {
                skipped_archives += 1;
                eprintln!(
                    "  [sv_processor] missing dependency archive {}, skipping",
                    dep_archive_path.display()
                );
                continue;
            }

            let dep_reader = super::common::V2ArchiveReader::new(&dep_archive_path);
            let mod_reader = super::common::V2ArchiveReader::new(&archive_path);

            for (dep_line, mod_line) in dep_reader.zip(mod_reader) {
                assert_eq!(
                    dep_line.region_str(),
                    mod_line.region_str(),
                    "Lock-step mismatch for sv_processor {} {}: dep={} vs mod={}",
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
                    "  [sv_processor] producer: archive {}/{total_archives}",
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

                let (bam_path, ref_path) = super::common::bam_tag_lookup(&tile.bam_tag);
                let reference_resource = Arc::new(ReferenceResource::new(
                    ref_path,
                    1200,
                    0,
                    chr_lengths.clone(),
                    false,
                ));
                let region = super::common::parse_region(&tile.region_str);

                let mut reference =
                    reference_resource
                        .get_reference(&region)
                        .unwrap_or_else(|error| {
                            panic!("Failed to load reference for {}: {error}", tile.region_str)
                        });

                let mut data: RealignedVariationData = serde_json::from_str(&tile.dep_data)
                    .unwrap_or_else(|error| {
                        panic!(
                            "Failed to deserialize realigner golden for {}: {error}",
                            tile.region_str
                        )
                    });

                let bams: Option<Vec<String>> = Some(vec![bam_path.to_string()]);
                let splice: Option<std::collections::BTreeSet<String>> = None;
                let mut prev_non_insertion_variants: HashMap<i32, VariationMap> = HashMap::new();
                let mut prev_ref_coverage: HashMap<i32, i32> = HashMap::new();
                let mut prev_soft_clips_3_end: HashMap<i32, Sclip> = HashMap::new();
                let mut prev_soft_clips_5_end: HashMap<i32, Sclip> = HashMap::new();
                let prev_reference_sequences: HashMap<i32, u8> = HashMap::new();

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
                    "",
                    0,
                );

                let result_json = serde_json::to_string(&data).unwrap_or_else(|error| {
                    panic!("Failed to serialize for {}: {error}", tile.region_str)
                });

                let count = tested.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 10000 == 0 {
                    eprintln!(
                        "  [sv_processor] progress: {count} tested, {} failures",
                        failure_count.load(Ordering::Relaxed)
                    );
                }

                if let Some(message) = super::common::assert_v2_module_parity(
                    "sv_processor",
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
        "parity_sv_processor_sweep: tested={final_tested}, archives={total_archives}/{total_archives}, dependency_archives_skipped={skipped_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_sv_processor_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        final_tested > 0,
        "No v2 sweep fixtures found for sv_processor. Run: scripts/sweep_fixtures.sh"
    );
}
