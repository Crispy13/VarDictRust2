mod common;

use vardict_rs::data::Region;
use vardict_rs::mods::cigar_modifier::modify_cigar;
use vardict_rs::reference::{Reference, ReferenceResource};

const MAX_FAILURES: usize = 10;

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

    common::check_sweep_manifest();
    let archive_root = base.join("v2").join("cigar_modifier");
    if !archive_root.is_dir() {
        eprintln!(
            "parity_cigar_modifier_sweep: skipping, no v2 archives at {}",
            archive_root.display()
        );
        return;
    }

    let archives = common::discover_v2_archives(&base, "cigar_modifier");
    if archives.is_empty() {
        eprintln!(
            "parity_cigar_modifier_sweep: skipping, no v2 archives discovered under {}",
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
    let mut completed_archives = 0usize;

    'archives: for (bam_tag, _chrom, archive_path) in archives {
        completed_archives += 1;
        let (_, ref_path) = common::bam_tag_lookup(&bam_tag);
        let reference_resource = ReferenceResource::new(ref_path, 1200, 0, chr_lengths.clone(), false);
        let mut archive_reader = common::V2ArchiveReader::new(&archive_path);

        for line in &mut archive_reader {
            tested += 1;
            if tested % 1000 == 0 {
                eprintln!(
                    "  [cigar_modifier] progress: {tested} tested, {} failures, archive {completed_archives}/{total_archives}",
                    failures.len()
                );
            }

            let region_str = line.region_str();
            let region = common::parse_region(&region_str);
            let reference = reference_resource.get_reference(&region).unwrap_or_else(|error| {
                panic!("Failed to load reference for {region_str}: {error}")
            });
            let golden_records: Vec<CigarModRecord> = serde_json::from_str(&line.data)
                .unwrap_or_else(|error| panic!("Failed to parse archive data for {region_str}: {error}"));
            let actual_records = rebuild_cigar_modifier_output(golden_records, &reference, &region);
            let actual_json = serde_json::to_string(&actual_records)
                .unwrap_or_else(|error| panic!("Failed to serialize output for {region_str}: {error}"));

            if let Some(message) =
                common::assert_v2_module_parity("cigar_modifier", &region_str, &actual_json, &line.data)
            {
                failures.push(message);
                if failures.len() >= MAX_FAILURES {
                    eprintln!(
                        "  [cigar_modifier] Reached {MAX_FAILURES} failures, stopping early"
                    );
                    break 'archives;
                }
            }
        }
    }

    eprintln!(
        "parity_cigar_modifier_sweep: tested={tested}, archives={completed_archives}/{total_archives}, failures={}",
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "parity_cigar_modifier_sweep: {} failures:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );

    assert!(
        tested > 0,
        "No v2 sweep fixtures found for cigar_modifier. Run: scripts/sweep_fixtures.sh"
    );
}