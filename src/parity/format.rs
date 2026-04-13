use indexmap::IndexMap;
use serde::de::Deserializer;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;

/// Serializes a HashMap<i32, V> as sorted array of pairs: [[key, value], ...]
pub fn serialize_sorted_int_map<V: Serialize, S: Serializer>(
    map: &HashMap<i32, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(key, _)| **key);
    let mut seq = serializer.serialize_seq(Some(entries.len()))?;
    for (key, value) in entries {
        seq.serialize_element(&(key, value))?;
    }
    seq.end()
}

/// Serializes an IndexMap<K, V> as array of pairs in insertion order: [[key, value], ...]
pub fn serialize_indexmap_as_pairs<K: Serialize, V: Serialize, S: Serializer>(
    map: &IndexMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (key, value) in map {
        seq.serialize_element(&(key, value))?;
    }
    seq.end()
}

/// Serializes a HashMap<String, V> as sorted array of pairs: [[key, value], ...]
pub fn serialize_sorted_string_map<V: Serialize, S: Serializer>(
    map: &HashMap<String, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(key, _)| (*key).clone());
    let mut seq = serializer.serialize_seq(Some(entries.len()))?;
    for (key, value) in entries {
        seq.serialize_element(&(key, value))?;
    }
    seq.end()
}

pub fn deserialize_sorted_string_map<'de, V, D>(
    deserializer: D,
) -> Result<HashMap<String, V>, D::Error>
where
    V: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let entries = Vec::<(String, V)>::deserialize(deserializer)?;
    Ok(entries.into_iter().collect())
}

/// Deserializes [[i32, V], ...] JSON array → HashMap<i32, V>
/// Mirror of serialize_sorted_int_map.
pub fn deserialize_sorted_int_map<'de, V, D>(deserializer: D) -> Result<HashMap<i32, V>, D::Error>
where
    V: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let entries = Vec::<(i32, V)>::deserialize(deserializer)?;
    Ok(entries.into_iter().collect())
}

/// Deserializes [[K, V], ...] JSON array → IndexMap<K, V>
/// Mirror of serialize_indexmap_as_pairs.
pub fn deserialize_indexmap_as_pairs<'de, K, V, D>(
    deserializer: D,
) -> Result<IndexMap<K, V>, D::Error>
where
    K: Deserialize<'de> + std::hash::Hash + Eq,
    V: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let entries = Vec::<(K, V)>::deserialize(deserializer)?;
    Ok(entries.into_iter().collect())
}

/// Serializes a BTreeMap<String, V> as sorted array of pairs: [["key", value], ...]
/// Matches Java LinkedHashMap serialization format used in golden fixtures.
pub fn serialize_btreemap_as_pairs<V: Serialize, S: Serializer>(
    map: &std::collections::BTreeMap<String, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (key, value) in map {
        seq.serialize_element(&(key, value))?;
    }
    seq.end()
}

/// Deserializes [["key", value], ...] JSON array → BTreeMap<String, V>
/// Mirror of serialize_btreemap_as_pairs.
pub fn deserialize_btreemap_as_pairs<'de, V, D>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, V>, D::Error>
where
    V: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let entries = Vec::<(String, V)>::deserialize(deserializer)?;
    Ok(entries.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use crate::data::{
        AlignedVarsData, RealignedVariationData, SortedStringMap, Variation, VariationData,
        VariationMap, Vars,
    };
    use indexmap::IndexMap;
    use std::collections::{BTreeMap, HashMap};

    /// Helper: serialize → deserialize → re-serialize, assert JSON strings are byte-equal.
    fn assert_round_trip<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let json1 = serde_json::to_string(value).expect("serialize 1");
        let deserialized: T = serde_json::from_str(&json1).expect("deserialize");
        let json2 = serde_json::to_string(&deserialized).expect("serialize 2");
        assert_eq!(json1, json2, "round-trip mismatch");
    }

    #[test]
    fn test_variation_map_round_trip() {
        let mut entries = IndexMap::new();
        entries.insert(
            "A".to_string(),
            Variation {
                vars_count: 5,
                vars_count_on_forward: 3,
                mean_quality: 30.5,
                ..Default::default()
            },
        );
        entries.insert(
            "T".to_string(),
            Variation {
                vars_count: 2,
                ..Default::default()
            },
        );
        let vmap = VariationMap { entries, sv: None };
        assert_round_trip(&vmap);
    }

    #[test]
    fn test_variation_data_round_trip() {
        let mut non_ins = HashMap::new();
        let mut vmap1 = IndexMap::new();
        vmap1.insert(
            "C".to_string(),
            Variation {
                vars_count: 10,
                ..Default::default()
            },
        );
        non_ins.insert(
            100,
            VariationMap {
                entries: vmap1,
                sv: None,
            },
        );

        let mut ref_cov = HashMap::new();
        ref_cov.insert(100, 50);
        ref_cov.insert(101, 55);

        let mut mnp = HashMap::new();
        let mut inner_mnp = SortedStringMap::new();
        inner_mnp.insert("AG".to_string(), 3);
        mnp.insert(200, inner_mnp);

        let vdata = VariationData {
            non_insertion_variants: non_ins,
            ref_coverage: ref_cov,
            mnp,
            ..Default::default()
        };
        assert_round_trip(&vdata);
    }

    #[test]
    fn test_realigned_variation_data_round_trip() {
        let mut non_ins = HashMap::new();
        let mut vmap = IndexMap::new();
        vmap.insert(
            "G".to_string(),
            Variation {
                vars_count: 7,
                ..Default::default()
            },
        );
        non_ins.insert(
            50,
            VariationMap {
                entries: vmap,
                sv: None,
            },
        );

        let mut ref_cov = HashMap::new();
        ref_cov.insert(50, 42);

        let rdata = RealignedVariationData {
            non_insertion_variants: non_ins,
            ref_coverage: ref_cov,
            duprate: 0.01,
            previous_scope: None,
            ..Default::default()
        };
        assert_round_trip(&rdata);

        // Verify previousScope serializes as null
        let json = serde_json::to_string(&rdata).unwrap();
        assert!(
            json.contains("\"previousScope\":null"),
            "previousScope must serialize as null, got: {}",
            json
        );
    }

    #[test]
    fn test_aligned_vars_data_round_trip() {
        let mut aligned = HashMap::new();
        aligned.insert(
            300,
            Vars {
                reference_variant: None,
                variants: vec![],
                var_description_string_to_variants: BTreeMap::new(),
                sv: String::new(),
            },
        );
        let avdata = AlignedVarsData {
            max_read_length: 150,
            aligned_variants: aligned,
        };
        assert_round_trip(&avdata);
    }
}
