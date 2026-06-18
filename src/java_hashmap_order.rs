/// Return keys in Java `HashMap<Integer, _>` current-key bucket order.
///
/// This mirrors Java's bucket traversal for default-constructed, non-tree
/// `HashMap`s using the current live key set. Same-bucket history is not
/// recoverable from keys alone, so ties use ascending key order deterministically.
pub(crate) fn java_hashmap_i32_order_from_keys<I>(keys: I) -> Vec<i32>
where
    I: IntoIterator<Item = i32>,
{
    let mut ordered: Vec<i32> = keys.into_iter().collect();
    if ordered.len() <= 1 {
        return ordered;
    }

    ordered.sort();
    let capacity = java_hashmap_capacity_for_len(ordered.len());
    ordered.sort_by_key(|key| java_hashmap_bucket_index_i32(*key, capacity));
    ordered
}

fn java_hashmap_capacity_for_len(len: usize) -> usize {
    if len == 0 {
        return 0;
    }

    let mut capacity = 16usize;
    while len > (capacity * 3) / 4 {
        capacity *= 2;
    }
    capacity
}

fn java_hashmap_bucket_index_i32(key: i32, capacity: usize) -> usize {
    debug_assert!(capacity.is_power_of_two());
    let hash = (key as u32) ^ ((key as u32) >> 16);
    (hash as usize) & (capacity - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hashmap_capacity_matches_default_thresholds() {
        assert_eq!(java_hashmap_capacity_for_len(0), 0);
        assert_eq!(java_hashmap_capacity_for_len(1), 16);
        assert_eq!(java_hashmap_capacity_for_len(12), 16);
        assert_eq!(java_hashmap_capacity_for_len(13), 32);
        assert_eq!(java_hashmap_capacity_for_len(24), 32);
        assert_eq!(java_hashmap_capacity_for_len(25), 64);
    }

    #[test]
    fn java_hashmap_i32_order_distinguishes_numeric_sort_from_bucket_order() {
        let ordered = java_hashmap_i32_order_from_keys([16, 1, 2]);
        assert_eq!(ordered, vec![16, 1, 2]);
    }

    #[test]
    fn java_hashmap_i32_order_uses_java_hash_spread() {
        let capacity = java_hashmap_capacity_for_len(1);
        assert_eq!(java_hashmap_bucket_index_i32(65_536, capacity), 1);
    }

    #[test]
    fn java_hashmap_i32_order_uses_deterministic_same_bucket_ties() {
        let ordered = java_hashmap_i32_order_from_keys([33, 17, 1]);
        assert_eq!(ordered, vec![1, 17, 33]);
    }
}
