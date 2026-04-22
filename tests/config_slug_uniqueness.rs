mod common;

use std::collections::HashSet;

#[test]
fn slugs_unique_and_well_formed() {
    let preset_rows = common::load_config_presets_raw_tsv();
    let tsv_names: HashSet<String> = preset_rows
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let expected_names: HashSet<String> = common::CONFIG_PRESETS
        .iter()
        .map(|&name| name.to_string())
        .collect();

    assert_eq!(
        tsv_names, expected_names,
        "Parsed config preset names must match common::CONFIG_PRESETS"
    );

    let mut seen_slugs = HashSet::new();
    for (name, _) in preset_rows {
        let slug = common::config_name_to_slug(&name);
        assert!(!slug.is_empty(), "Slug was empty for preset name {name}");
        assert!(
            slug.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "Slug {slug} for preset name {name} must contain only lowercase ASCII letters, digits, or underscores"
        );
        assert!(
            seen_slugs.insert(slug.clone()),
            "Duplicate slug {slug} generated for preset name {name}"
        );
    }
}
