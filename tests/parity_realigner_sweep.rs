mod common;

use std::collections::HashSet;
use std::sync::Arc;

use vardict_rs::data::VariationData;
use vardict_rs::mods::variation_realigner;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{Scope, VariantPrinter};

const MAX_FAILURES: usize = 10;

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

    let mut failures = Vec::new();
    let mut tested = 0usize;
    let mut skipped_archives = 0usize;
    let mut completed_archives = 0usize;

    'archives: for (bam_tag, chrom, archive_path) in archives {
        completed_archives += 1;
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
        let (bam_path, ref_path) = common::bam_tag_lookup(&bam_tag);
        let reference_resource = Arc::new(ReferenceResource::new(
            ref_path,
            1200,
            0,
            chr_lengths.clone(),
            false,
        ));

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

            tested += 1;
            if tested % 1000 == 0 {
                eprintln!(
                    "  [realigner] progress: {tested} tested, {} failures, archive {completed_archives}/{total_archives}, {skipped_archives} dependency archives skipped",
                    failures.len()
                );
            }

            let region_str = mod_line.region_str();

            let region = common::parse_region(&region_str);

            let reference = reference_resource
                .get_reference(&region)
                .unwrap_or_else(|error| {
                    panic!("Failed to load reference for {region_str}: {error}")
                });
            let reference = Arc::new(reference);

            let variation_data: VariationData = serde_json::from_str(&dep_line.data)
                .unwrap_or_else(|error| {
                    panic!("Failed to deserialize cigar_parser golden for {region_str}: {error}")
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
            let result_json = serde_json::to_string(&result_scope.data)
                .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

            if let Some(message) = common::assert_v2_module_parity(
                "realigner",
                &region_str,
                &result_json,
                &mod_line.data,
            ) {
                failures.push(message);
                if failures.len() >= MAX_FAILURES {
                    eprintln!("  [realigner] Reached {MAX_FAILURES} failures, stopping early");
                    break 'archives;
                }
            }
        }
    }

    eprintln!(
        "parity_realigner_sweep: tested={tested}, archives={completed_archives}/{total_archives}, dependency_archives_skipped={skipped_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_realigner_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No v2 sweep fixtures found for realigner. Run: scripts/sweep_fixtures.sh"
    );
}
