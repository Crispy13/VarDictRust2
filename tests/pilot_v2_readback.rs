mod common;

fn sample_region(module: &str, sample_path: &std::path::Path) -> String {
    let filename = sample_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("Invalid sample filename: {}", sample_path.display()));
    let stem = filename
        .strip_suffix(".jsonl")
        .unwrap_or_else(|| panic!("Expected .jsonl sample file: {filename}"));
    let after_module = stem
        .strip_prefix(&format!("{module}_"))
        .unwrap_or_else(|| panic!("Wrong sample prefix for {filename}"));
    let parts: Vec<&str> = after_module.splitn(3, '_').collect();

    assert_eq!(parts.len(), 3, "Unexpected sample filename: {filename}");

    format!("{}:{}-{}", parts[0], parts[1], parts[2])
}

#[test]
#[ignore = "Pilot readback — requires tmp/pilot from: python scripts/pilot_generate.py — remove when sweep coverage is complete"]
fn pilot_v2_readback() {
    let pilot_dir = std::path::PathBuf::from("tmp/pilot");
    assert!(
        pilot_dir.is_dir(),
        "Pilot fixture dir not found at {}. Generate with: python scripts/pilot_generate.py",
        pilot_dir.display()
    );

    let modules = ["cigar_parser", "realigner", "sv_processor", "tovars"];
    let mut pass_count = 0usize;

    for module in modules {
        let archive_path = pilot_dir.join(format!("v2/{module}/na12878_lowcov/1.jsonl.zst"));
        let samples_dir = pilot_dir.join(format!("samples/{module}"));

        assert!(archive_path.is_file(), "Missing pilot archive for {module}");
        assert!(samples_dir.is_dir(), "Missing sample dir for {module}");

        let mut sample_paths: Vec<_> = std::fs::read_dir(&samples_dir)
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", samples_dir.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| {
                        panic!("Failed to read entry in {}: {error}", samples_dir.display())
                    })
                    .path()
            })
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .collect();
        sample_paths.sort();

        for sample_path in sample_paths {
            let region = sample_region(module, &sample_path);
            let content = std::fs::read_to_string(&sample_path).unwrap_or_else(|error| {
                panic!("Failed to read {}: {error}", sample_path.display())
            });
            let lines: Vec<&str> = content.lines().collect();

            assert!(
                lines.len() >= 2,
                "Sample should have at least 2 lines: {}",
                sample_path.display()
            );

            let per_tile_data = lines[1];
            let archive_data = common::load_v2_archive_region(&archive_path, &region);

            assert_eq!(
                per_tile_data, archive_data,
                "Mismatch for {module} region {region}"
            );
            pass_count += 1;
        }
    }

    assert_eq!(pass_count, 12, "Expected 12 comparisons, got {pass_count}");
    eprintln!("pilot_v2_readback: {pass_count} tile comparisons PASSED");
}
