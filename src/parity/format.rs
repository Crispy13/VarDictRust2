use indexmap::IndexMap;
use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};
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
