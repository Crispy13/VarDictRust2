/// Microbenchmark: IndexMap<String,Variation,FxBuildHasher> (current)
/// vs Vec<(String,Variation)>-backed linear map (candidate).
///
/// Tests the real access pattern:
///   1. get_or_insert_with_default (the `get_variation` hot path)
///   2. get
///   3. shift_remove
///
/// at N = 1, 3, 5, 20, 100 entries with realistic short keys.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

// ─── Variation (minimal reproduction — same fields as src/data.rs) ───────────

#[derive(Clone, Default)]
struct Variation {
    vars_count: i32,
    vars_count_on_forward: i32,
    vars_count_on_reverse: i32,
    mean_position: f64,
    mean_quality: f64,
    mean_mapping_quality: f64,
    number_of_mismatches: f64,
    low_quality_reads_count: i32,
    high_quality_reads_count: i32,
    pstd: bool,
    qstd: bool,
    pp: i32,
    pq: f64,
    extracnt: i32,
}

// ─── Realistic key sets (matches production patterns) ────────────────────────

/// Keys representative of the first N distinct variants at a position.
fn keys_for_n(n: usize) -> Vec<String> {
    // Short keys: single-base SNVs, short insertions, short deletions, SV
    let pool = [
        "A", "T", "C", "G", "+AT", "+TG", "+CAT", "+ATCG", "-1", "-2", "-3",
        "-4", "+ATCGATCG", "+TTTT", "-10", "SV", "N", "+A", "+T", "+C",
        "+GCGCGCGCGCGCGCGCGCGC", // long key (worst case for linear)
        "+AAAAAAAAAAAAAAAAAAAAAA",
        "+TTTTTTTTTTTTTTTTTTTTTT",
        "+CCCCCCCCCCCCCCCCCCCCCC",
        "+GGGGGGGGGGGGGGGGGGGGGG",
        "-20", "-30", "-40", "-50", "-60",
        "+ATCGATCGATCGATCGATCG1",
        "+ATCGATCGATCGATCGATCG2",
        "+ATCGATCGATCGATCGATCG3",
        "+ATCGATCGATCGATCGATCG4",
        "+ATCGATCGATCGATCGATCG5",
        "+ATCGATCGATCGATCGATCG6",
        "+ATCGATCGATCGATCGATCG7",
        "+ATCGATCGATCGATCGATCG8",
        "+ATCGATCGATCGATCGATCG9",
        "+ATCGATCGATCGATCGATCGA",
        "+ATCGATCGATCGATCGATCGB",
        "+ATCGATCGATCGATCGATCGC",
        "+ATCGATCGATCGATCGATCGD",
        "+ATCGATCGATCGATCGATCGE",
        "+ATCGATCGATCGATCGATCGF",
        "+ATCGATCGATCGATCGATCGG",
        "+ATCGATCGATCGATCGATCGH",
        "+ATCGATCGATCGATCGATCGI",
        "+ATCGATCGATCGATCGATCGJ",
        "+ATCGATCGATCGATCGATCGK",
        "+ATCGATCGATCGATCGATCGL",
        "+ATCGATCGATCGATCGATCGM",
        "+ATCGATCGATCGATCGATCGN",
        "+ATCGATCGATCGATCGATCGO",
        "+ATCGATCGATCGATCGATCGP",
        "+ATCGATCGATCGATCGATCGQ",
        "+ATCGATCGATCGATCGATCGR",
        "+ATCGATCGATCGATCGATCGS",
        "+ATCGATCGATCGATCGATCGT",
        "+ATCGATCGATCGATCGATCGU",
        "+ATCGATCGATCGATCGATCGV",
        "+ATCGATCGATCGATCGATCGW",
        "+ATCGATCGATCGATCGATCGX",
        "+ATCGATCGATCGATCGATCGY",
        "+ATCGATCGATCGATCGATCGZ",
        "-100",
        "-101",
        "-102",
        "-103",
        "-104",
        "-105",
        "-106",
        "-107",
        "-108",
        "-109",
        "-110",
        "-111",
        "-112",
        "-113",
        "-114",
        "-115",
        "-116",
        "-117",
        "-118",
        "-119",
        "-120",
        "-121",
        "-122",
        "-123",
        "-124",
        "-125",
        "-126",
        "-127",
        "-128",
        "-129",
        "-130",
        "-131",
        "-132",
        "-133",
        "-134",
        "-135",
        "-136",
        "-137",
        "-138",
        "-139",
        "-140",
    ];
    pool.iter().take(n).map(|s| s.to_string()).collect()
}

// ─── Current: IndexMap<String, Variation, FxBuildHasher> ─────────────────────

type IndexMapImpl = IndexMap<String, Variation, FxBuildHasher>;

/// Mirrors `get_variation`: single-pass lookup-or-insert-default.
#[inline(never)]
fn indexmap_get_or_insert<'a>(map: &'a mut IndexMapImpl, key: &str) -> &'a mut Variation {
    map.entry(key.to_string()).or_default()
}

#[inline(never)]
fn indexmap_get(map: &IndexMapImpl, key: &str) -> bool {
    map.get(key).is_some()
}

#[inline(never)]
fn indexmap_shift_remove(map: &mut IndexMapImpl, key: &str) -> bool {
    map.shift_remove(key).is_some()
}

// ─── Candidate: Vec<(String, Variation)>-backed linear map ───────────────────

struct VecMap {
    entries: Vec<(String, Variation)>,
}

impl VecMap {
    fn new() -> Self {
        Self { entries: Vec::new() }
    }

    fn with_capacity(cap: usize) -> Self {
        Self { entries: Vec::with_capacity(cap) }
    }

    /// Single-pass get-or-insert-default (mirrors the get_variation hot path).
    #[inline]
    fn get_or_insert_default(&mut self, key: &str) -> &mut Variation {
        // Linear scan: find or insert
        let pos = self.entries.iter().position(|(k, _)| k == key);
        match pos {
            Some(i) => &mut self.entries[i].1,
            None => {
                self.entries.push((key.to_string(), Variation::default()));
                let last = self.entries.len() - 1;
                &mut self.entries[last].1
            }
        }
    }

    #[inline]
    fn get(&self, key: &str) -> Option<&Variation> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    #[inline]
    fn shift_remove(&mut self, key: &str) -> Option<Variation> {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            Some(self.entries.remove(pos).1)
        } else {
            None
        }
    }
}

#[inline(never)]
fn vecmap_get_or_insert<'a>(map: &'a mut VecMap, key: &str) -> &'a mut Variation {
    map.get_or_insert_default(key)
}

#[inline(never)]
fn vecmap_get(map: &VecMap, key: &str) -> bool {
    map.get(key).is_some()
}

#[inline(never)]
fn vecmap_shift_remove(map: &mut VecMap, key: &str) -> bool {
    map.shift_remove(key).is_some()
}

// ─── Benchmarks ──────────────────────────────────────────────────────────────

const SIZES: [usize; 5] = [1, 3, 5, 20, 100];

/// Benchmark: populate a fresh map with N entries then perform one more
/// get-or-insert of the LAST key (hit path) and one of a new key (miss path).
fn bench_get_or_insert(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("get_or_insert_hit");

    for &n in &SIZES {
        let keys = keys_for_n(n);
        let lookup_key = keys.last().unwrap().clone();

        group.bench_with_input(
            BenchmarkId::new("indexmap", n),
            &(keys.clone(), lookup_key.clone()),
            |b, (keys, lookup)| {
                b.iter(|| {
                    let mut map = IndexMapImpl::default();
                    for key in keys {
                        indexmap_get_or_insert(&mut map, key);
                    }
                    // hit: last key already in map
                    let v = indexmap_get_or_insert(black_box(&mut map), black_box(lookup));
                    black_box(v.vars_count += 1);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vecmap", n),
            &(keys.clone(), lookup_key.clone()),
            |b, (keys, lookup)| {
                b.iter(|| {
                    let mut map = VecMap::new();
                    for key in keys {
                        vecmap_get_or_insert(&mut map, key);
                    }
                    // hit: last key already in map
                    let v = vecmap_get_or_insert(black_box(&mut map), black_box(lookup));
                    black_box(v.vars_count += 1);
                });
            },
        );
    }
    group.finish();
}

fn bench_get_or_insert_miss(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("get_or_insert_miss");

    for &n in &SIZES {
        let keys = keys_for_n(n);
        let miss_key = "___NEW_KEY___".to_string();

        group.bench_with_input(
            BenchmarkId::new("indexmap", n),
            &(keys.clone(), miss_key.clone()),
            |b, (keys, miss)| {
                b.iter(|| {
                    let mut map = IndexMapImpl::default();
                    for key in keys {
                        indexmap_get_or_insert(&mut map, key);
                    }
                    // miss: insert new key
                    let v = indexmap_get_or_insert(black_box(&mut map), black_box(miss));
                    black_box(v.vars_count += 1);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vecmap", n),
            &(keys.clone(), miss_key.clone()),
            |b, (keys, miss)| {
                b.iter(|| {
                    let mut map = VecMap::new();
                    for key in keys {
                        vecmap_get_or_insert(&mut map, key);
                    }
                    // miss: insert new key
                    let v = vecmap_get_or_insert(black_box(&mut map), black_box(miss));
                    black_box(v.vars_count += 1);
                });
            },
        );
    }
    group.finish();
}

fn bench_get(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("get");

    for &n in &SIZES {
        let keys = keys_for_n(n);
        let lookup_key = keys[n / 2].clone(); // middle key (realistic)

        // Pre-built maps for get bench
        let mut im = IndexMapImpl::default();
        let mut vm = VecMap::new();
        for key in &keys {
            indexmap_get_or_insert(&mut im, key);
            vecmap_get_or_insert(&mut vm, key);
        }

        group.bench_with_input(
            BenchmarkId::new("indexmap", n),
            &lookup_key,
            |b, k| {
                b.iter(|| {
                    black_box(indexmap_get(black_box(&im), black_box(k)));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vecmap", n),
            &lookup_key,
            |b, k| {
                b.iter(|| {
                    black_box(vecmap_get(black_box(&vm), black_box(k)));
                });
            },
        );
    }
    group.finish();
}

fn bench_shift_remove(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("shift_remove");

    for &n in &SIZES {
        let keys = keys_for_n(n);
        let remove_key = keys[n / 2].clone(); // middle key (worst case for Vec)

        group.bench_with_input(
            BenchmarkId::new("indexmap", n),
            &(keys.clone(), remove_key.clone()),
            |b, (keys, rm)| {
                b.iter(|| {
                    let mut map = IndexMapImpl::default();
                    for key in keys {
                        indexmap_get_or_insert(&mut map, key);
                    }
                    black_box(indexmap_shift_remove(black_box(&mut map), black_box(rm)));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vecmap", n),
            &(keys.clone(), remove_key.clone()),
            |b, (keys, rm)| {
                b.iter(|| {
                    let mut map = VecMap::new();
                    for key in keys {
                        vecmap_get_or_insert(&mut map, key);
                    }
                    black_box(vecmap_shift_remove(black_box(&mut map), black_box(rm)));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_get_or_insert,
    bench_get_or_insert_miss,
    bench_get,
    bench_shift_remove
);
criterion_main!(benches);
