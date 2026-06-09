use indexmap::IndexMap;
use serde::de::Deserializer;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::rc::Rc;

use crate::data::VecMap;

/// Serializes a HashMap<i32, V, H> as sorted array of pairs: [[key, value], ...]
pub fn serialize_sorted_int_map<V: Serialize, H: BuildHasher, S: Serializer>(
    map: &HashMap<i32, V, H>,
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
) -> Result<HashMap<i32, V, H>, D::Error>
where
    V: Deserialize<'de>,
    H: BuildHasher + Default,
    D: Deserializer<'de>,
{
    let entries = Vec::<(i32, V)>::deserialize(deserializer)?;
    let mut map = HashMap::with_capacity_and_hasher(entries.len(), H::default());
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

/// Serializes an Option<Rc<RefCell<Variant>>> by serializing the inner Variant (or null).
pub fn serialize_option_rc_variant<S: Serializer>(
    opt: &Option<Rc<RefCell<crate::data::Variant>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match opt {
        Some(cell) => serializer.serialize_some(&*cell.borrow()),
        None => serializer.serialize_none(),
    }
}

/// Deserializes an Option<Variant> → Option<Rc<RefCell<Variant>>>.
pub fn deserialize_option_rc_variant<'de, D>(
    deserializer: D,
) -> Result<Option<Rc<RefCell<crate::data::Variant>>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<crate::data::Variant>::deserialize(deserializer)?;
    Ok(opt.map(|v| Rc::new(RefCell::new(v))))
}

/// Serializes a Vec<Rc<RefCell<Variant>>> by serializing each inner Variant.
pub fn serialize_vec_rc_variant<S: Serializer>(
    vec: &[Rc<RefCell<crate::data::Variant>>],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for cell in vec {
        seq.serialize_element(&*cell.borrow())?;
    }
    seq.end()
}

/// Deserializes a Vec<Variant> → Vec<Rc<RefCell<Variant>>>.
pub fn deserialize_vec_rc_variant<'de, D>(
    deserializer: D,
) -> Result<Vec<Rc<RefCell<crate::data::Variant>>>, D::Error>
where
    D: Deserializer<'de>,
{
    let entries = Vec::<crate::data::Variant>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|v| Rc::new(RefCell::new(v)))
        .collect())
}

/// Serializes a BTreeMap<String, Rc<RefCell<Variant>>> as sorted array of pairs: [["key", value], ...]
/// Matches Java LinkedHashMap serialization format used in golden fixtures.
/// Serializes the inner Variant value (not the Rc/RefCell wrapper) to preserve byte-identical output.
pub fn serialize_btreemap_as_pairs<S: Serializer>(
    map: &std::collections::BTreeMap<String, Rc<RefCell<crate::data::Variant>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (key, cell) in map {
        seq.serialize_element(&(key, &*cell.borrow()))?;
    }
    seq.end()
}

/// Deserializes [["key", value], ...] JSON array → BTreeMap<String, Rc<RefCell<Variant>>>
/// Mirror of serialize_btreemap_as_pairs.
/// Wraps each deserialized Variant into Rc<RefCell<_>>.
pub fn deserialize_btreemap_as_pairs<'de, D>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, Rc<RefCell<crate::data::Variant>>>, D::Error>
where
    D: Deserializer<'de>,
{
    let entries = Vec::<(String, crate::data::Variant)>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|(k, v)| (k, Rc::new(RefCell::new(v))))
        .collect())
}

#[cfg(test)]
mod tests {
    use crate::data::{
        AlignedVarsData, CoverageMap, PositionMap, RealignedVariationData, SortedStringMap,
        Variation, VariationData, VariationEntries, VariationMap, Vars,
    };
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
