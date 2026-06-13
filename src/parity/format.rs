use indexmap::IndexMap;
use serde::de::Deserializer;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap as StdHashMap;
use std::hash::BuildHasher;

use crate::prelude::HashMap;

use crate::data::VecMap;

/// Serializes a HashMap<i32, V, H> as sorted array of pairs: [[key, value], ...]
pub fn serialize_sorted_int_map<V: Serialize, H: BuildHasher, S: Serializer>(
    map: &StdHashMap<i32, V, H>,
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
pub fn serialize_indexmap_as_pairs<K, V, H, S>(
    map: &IndexMap<K, V, H>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    K: Serialize,
    V: Serialize,
    H: BuildHasher,
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (key, value) in map {
        seq.serialize_element(&(key, value))?;
    }
    seq.end()
}

/// Serializes a VecMap<V> as array of pairs in insertion order: [["key", value], ...]
pub fn serialize_vecmap_as_pairs<V: Serialize, S: Serializer>(
    map: &VecMap<V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (key, value) in map.iter() {
        seq.serialize_element(&(key, value))?;
    }
    seq.end()
}

/// Deserializes [["key", value], ...] JSON array → VecMap<V>
/// Mirror of serialize_vecmap_as_pairs.
pub fn deserialize_vecmap_as_pairs<'de, V, D>(deserializer: D) -> Result<VecMap<V>, D::Error>
where
    V: Deserialize<'de>,
    D: Deserializer<'de>,
{
    let entries = Vec::<(String, V)>::deserialize(deserializer)?;
    let mut map = VecMap::new();
    for (k, v) in entries {
        map.insert(k, v);
    }
    Ok(map)
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

/// Deserializes [[i32, V], ...] JSON array → HashMap<i32, V, H>
/// Mirror of serialize_sorted_int_map.
pub fn deserialize_sorted_int_map<'de, V, H, D>(
    deserializer: D,
) -> Result<StdHashMap<i32, V, H>, D::Error>
where
    V: Deserialize<'de>,
    H: BuildHasher + Default,
    D: Deserializer<'de>,
{
    let entries = Vec::<(i32, V)>::deserialize(deserializer)?;
    let mut map = StdHashMap::with_capacity_and_hasher(entries.len(), H::default());
    map.extend(entries);
    Ok(map)
}

/// Deserializes [[K, V], ...] JSON array → IndexMap<K, V>
/// Mirror of serialize_indexmap_as_pairs.
pub fn deserialize_indexmap_as_pairs<'de, K, V, H, D>(
    deserializer: D,
) -> Result<IndexMap<K, V, H>, D::Error>
where
    K: Deserialize<'de> + std::hash::Hash + Eq,
    V: Deserialize<'de>,
    H: BuildHasher + Default,
    D: Deserializer<'de>,
{
    let entries = Vec::<(K, V)>::deserialize(deserializer)?;
    let mut map = IndexMap::with_capacity_and_hasher(entries.len(), H::default());
    map.extend(entries);
    Ok(map)
}

#[cfg(test)]
mod tests {
    use crate::data::{
        AlignedVarsData, CoverageMap, PositionMap, RealignedVariationData, SortedStringMap,
        Variation, VariationData, VariationEntries, VariationMap, Vars,
    };
    use crate::prelude::HashMap;

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
        let mut entries = VariationEntries::default();
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
        let mut non_ins = PositionMap::default();
        let mut vmap1 = VariationEntries::default();
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

        let mut ref_cov = CoverageMap::default();
        ref_cov.insert(100, 50);
        ref_cov.insert(101, 55);

        let mut mnp = PositionMap::default();
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
        let mut non_ins = PositionMap::default();
        let mut vmap = VariationEntries::default();
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

        let mut ref_cov = CoverageMap::default();
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
        let mut aligned = HashMap::default();
        aligned.insert(300, Vars::default());
        let avdata = AlignedVarsData {
            max_read_length: 150,
            aligned_variants: aligned,
        };
        assert_round_trip(&avdata);
    }

    #[test]
    fn test_vars_with_variants_round_trip() {
        use crate::data::Variant;
        // Build a Vars with one variant in the arena/list/varn
        let mut vars = Vars::default();
        let v = Variant {
            description_string: String::from("A"),
            frequency: 0.5,
            ..Variant::default()
        };
        let idx = vars.arena.len();
        let desc = v.description_string.clone();
        vars.arena.push(v);
        vars.variants.push(idx);
        vars.var_description_string_to_variants.insert(desc, idx);
        assert_round_trip(&vars);
    }
}
