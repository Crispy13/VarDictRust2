use vardict_rs::data::Region;
use vardict_rs::mods::cigar_modifier::modify_cigar;
use vardict_rs::reference::{Reference, ReferenceResource};

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
fn parity_cigar_modifier_all_regions() {
    let regions = super::common::load_region_config();

    for (region_str, _bam_path, ref_path) in &regions {
        let region = super::common::parse_region(region_str);
        let fai_path = format!("{}.fai", ref_path.display());
        let chr_lengths = super::common::load_chr_lengths(&fai_path);
        let _guard = super::common::init_test_scope(chr_lengths.clone());

        let reference_resource =
            ReferenceResource::new(ref_path.to_str().unwrap(), 1200, 0, chr_lengths, false);
        let reference = reference_resource
            .get_reference(&region)
            .unwrap_or_else(|error| panic!("Failed to load reference for {region_str}: {error}"));

        let golden_json = super::common::load_golden_data("cigar_modifier", region_str);
        let golden_records: Vec<CigarModRecord> = serde_json::from_str(&golden_json)
            .unwrap_or_else(|error| {
                panic!("Failed to parse golden fixture for {region_str}: {error}")
            });
        let actual_records = rebuild_cigar_modifier_output(golden_records, &reference, &region);
        let actual_json = serde_json::to_string(&actual_records)
            .unwrap_or_else(|error| panic!("Failed to serialize output for {region_str}: {error}"));

        super::common::assert_module_parity("cigar_modifier", region_str, &actual_json);
    }
}
