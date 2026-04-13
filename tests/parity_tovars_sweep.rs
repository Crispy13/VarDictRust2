mod common;

use vardict_rs::data::RealignedVariationData;
use vardict_rs::mods::to_vars_builder;
use vardict_rs::reference::ReferenceResource;

const MAX_FAILURES: usize = 10;

fn reference_fetch_region(
    region: &vardict_rs::data::Region,
    data: &RealignedVariationData,
) -> vardict_rs::data::Region {
    let min_position = data
        .non_insertion_variants
        .keys()
        .chain(data.insertion_variants.keys())
        .copied()
        .min()
        .unwrap_or(region.start)
        .min(region.start);
    let max_position = data
        .non_insertion_variants
        .keys()
        .chain(data.insertion_variants.keys())
        .copied()
        .max()
        .unwrap_or(region.end)
        .max(region.end);

    vardict_rs::data::Region::new(&region.chr, min_position, max_position, "")
}

#[test]
#[ignore = "Sweep gate: ToVarsBuilder full-sweep parity"]
fn parity_tovars_sweep() {
    let base = std::env::var_os("VARDICT_SWEEP_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("tmp/sweep_fixtures"));
    if !base.is_dir() {
        eprintln!(
            "parity_tovars_sweep: skipping, no sweep fixtures at {}",
            base.display()
        );
        return;
    }

    common::check_sweep_manifest();
    let archive_root = base.join("v2").join("tovars");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_tovars_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = common::discover_v2_archives(&base, "tovars");
    if archives.is_empty() {
        eprintln!(
            "parity_tovars_sweep: skipping, no v2 archives discovered under {}",
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
            .join("sv_processor")
            .join(&bam_tag)
            .join(format!("{chrom}.jsonl.zst"));
        if !dep_archive_path.is_file() {
            skipped_archives += 1;
            eprintln!(
                "  [tovars] missing dependency archive {}, skipping",
                dep_archive_path.display()
            );
            continue;
        }

        let dep_reader = common::V2ArchiveReader::new(&dep_archive_path);
        let mod_reader = common::V2ArchiveReader::new(&archive_path);
        let (_, ref_path) = common::bam_tag_lookup(&bam_tag);

        for (dep_line, mod_line) in dep_reader.zip(mod_reader) {
            assert_eq!(
                dep_line.region_str(),
                mod_line.region_str(),
                "Lock-step mismatch for tovars {} {}: dep={} vs mod={}",
                bam_tag,
                chrom,
                dep_line.region_str(),
                mod_line.region_str()
            );

            tested += 1;
            if tested % 1000 == 0 {
                eprintln!(
                    "  [tovars] progress: {tested} tested, {} failures, archive {completed_archives}/{total_archives}, {skipped_archives} dependency archives skipped",
                    failures.len()
                );
            }

            let region_str = mod_line.region_str();

            let _guard = common::init_test_scope();
            let region = common::parse_region(&region_str);

            let reference_resource = ReferenceResource::new(
                ref_path,
                1200,
                0,
                chr_lengths.clone(),
                false,
            );

            let data: RealignedVariationData =
                serde_json::from_str(&dep_line.data).unwrap_or_else(|error| {
                    panic!("Failed to deserialize sv_processor golden for {region_str}: {error}")
                });

            let fetch_region = reference_fetch_region(&region, &data);
            let reference = reference_resource
                .get_reference(&fetch_region)
                .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));
            let ref_map = &reference.reference_sequences;

            let max_read_length = data.max_read_length.unwrap_or(0);
            let ref_coverage = &data.ref_coverage;
            let insertion_variants = &data.insertion_variants;
            let mut non_insertion_variants = data.non_insertion_variants;
            let duprate = data.duprate;

            let result = to_vars_builder::process(
                max_read_length,
                &region,
                ref_map,
                ref_coverage,
                insertion_variants,
                &mut non_insertion_variants,
                duprate,
            );

            let result_json = serde_json::to_string(&result)
                .unwrap_or_else(|error| panic!("Failed to serialize for {region_str}: {error}"));

            if let Some(message) = common::assert_v2_module_parity(
                "tovars",
                &region_str,
                &result_json,
                &mod_line.data,
            ) {
                failures.push(message);
                if failures.len() >= MAX_FAILURES {
                    eprintln!("  [tovars] Reached {MAX_FAILURES} failures, stopping early");
                    break 'archives;
                }
            }
        }
    }

    eprintln!(
        "parity_tovars_sweep: tested={tested}, archives={completed_archives}/{total_archives}, dependency_archives_skipped={skipped_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_tovars_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No v2 sweep fixtures found for tovars. Run: scripts/sweep_fixtures.sh"
    );
}
