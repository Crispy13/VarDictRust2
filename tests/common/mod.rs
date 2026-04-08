use std::path::PathBuf;

pub fn load_region_config() -> Vec<(String, PathBuf, PathBuf)> {
    let tsv = std::fs::read_to_string("testdata/parity_regions.tsv")
        .expect("testdata/parity_regions.tsv not found");

    tsv.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(
                fields.len(),
                3,
                "expected 3 fields in parity_regions.tsv: {line}"
            );
            (
                fields[0].to_string(),
                PathBuf::from(fields[1]),
                PathBuf::from(fields[2]),
            )
        })
        .collect()
}

pub fn golden_fixture_path(module: &str, region: &str) -> PathBuf {
    let safe_region = region.replace(':', "_").replace('-', "_");
    let filename = format!("{module}_{safe_region}.jsonl");
    PathBuf::from("testdata/fixtures")
        .join(module)
        .join(filename)
}

pub fn load_golden_data(module: &str, region: &str) -> String {
    let path = golden_fixture_path(module, region);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("Failed to read {}: {error}", path.display()));
    let lines: Vec<&str> = content.lines().collect();

    assert!(
        lines.len() >= 2,
        "Fixture {} should have at least 2 lines",
        path.display()
    );

    lines[1].to_string()
}
