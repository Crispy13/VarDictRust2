mod common;

use std::collections::HashMap;
use std::sync::Arc;

use vardict_rs::data::{RealignedVariationData, Sclip, VariationMap};
use vardict_rs::mods::structural_variants_processor;
use vardict_rs::reference::ReferenceResource;

const MAX_FAILURES: usize = 10;

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

    common::check_sweep_manifest();
    let archive_root = base.join("v2").join("sv_processor");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_sv_processor_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = common::discover_v2_archives(&base, "sv_processor");
    if archives.is_empty() {
        eprintln!(
            "parity_sv_processor_sweep: skipping, no v2 archives discovered under {}",
            archive_root.display()
        );
        return;
    }

    let total_archives = archives.len();
    let (_, first_ref) = common::bam_tag_lookup(&archives[0].0);
    let fai_path = format!("{first_ref}.fai");
    let chr_lengths = common::load_chr_lengths(&fai_path);

    let mut failures = Vec::new();
    let mut tested = 0usize;
    let mut skipped_archives = 0usize;
    let mut completed_archives = 0usize;

    'archives: for (bam_tag, chrom, archive_path) in archives {
        completed_archives += 1;
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

        let dep_reader = common::V2ArchiveReader::new(&dep_archive_path);
        let mod_reader = common::V2ArchiveReader::new(&archive_path);
        let (bam_path, ref_path) = common::bam_tag_lookup(&bam_tag);

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

            tested += 1;
            if tested % 1000 == 0 {
                eprintln!(
                    "  [sv_processor] progress: {tested} tested, {} failures, archive {completed_archives}/{total_archives}, {skipped_archives} dependency archives skipped",
                    failures.len()
                );
            }

            let region_str = mod_line.region_str();

            let _guard = common::init_test_scope();
            let region = common::parse_region(&region_str);

            let reference_resource = Arc::new(ReferenceResource::new(
                ref_path,
                1200,
                0,
                chr_lengths.clone(),
                false,
            ));
            let mut reference = reference_resource
                .get_reference(&region)
                .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));

            let mut data: RealignedVariationData = serde_json::from_str(&dep_line.data)
                .unwrap_or_else(|error| {
                    panic!("Failed to deserialize realigner golden for {region_str}: {error}")
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

            let result_json = serde_json::to_string(&data)
                .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

            if let Some(message) = common::assert_v2_module_parity(
                "sv_processor",
                &region_str,
                &result_json,
                &mod_line.data,
            ) {
                failures.push(message);
                if failures.len() >= MAX_FAILURES {
                    eprintln!("  [sv_processor] Reached {MAX_FAILURES} failures, stopping early");
                    break 'archives;
                }
            }
        }
    }

    eprintln!(
        "parity_sv_processor_sweep: tested={tested}, archives={completed_archives}/{total_archives}, dependency_archives_skipped={skipped_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_sv_processor_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No v2 sweep fixtures found for sv_processor. Run: scripts/sweep_fixtures.sh"
    );
}
