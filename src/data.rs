use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};

use indexmap::IndexMap;
use serde::de::Deserializer;
use serde::ser::{SerializeSeq, Serializer};

use crate::config::Configuration;
use crate::patterns::ANY_SV;
use crate::utils::{substr, substr_with_len};

// ─── SortedStringMap: BTreeMap<String, V> with array-of-pairs serde ─────────

/// Transparent newtype around `BTreeMap<String, V>` that serializes as
/// `[["key", value], ...]` (sorted by key) to match Java's LinkedHashMap
/// serialization in golden fixtures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortedStringMap<V>(pub BTreeMap<String, V>);

impl<V> SortedStringMap<V> {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }
}

impl<V> std::ops::Deref for SortedStringMap<V> {
    type Target = BTreeMap<String, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<V> std::ops::DerefMut for SortedStringMap<V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<V> From<BTreeMap<String, V>> for SortedStringMap<V> {
    fn from(map: BTreeMap<String, V>) -> Self {
        Self(map)
    }
}

impl<V, const N: usize> From<[(String, V); N]> for SortedStringMap<V> {
    fn from(arr: [(String, V); N]) -> Self {
        Self(BTreeMap::from(arr))
    }
}

impl<V> IntoIterator for SortedStringMap<V> {
    type Item = (String, V);
    type IntoIter = std::collections::btree_map::IntoIter<String, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, V> IntoIterator for &'a SortedStringMap<V> {
    type Item = (&'a String, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, String, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<V: serde::Serialize> serde::Serialize for SortedStringMap<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            seq.serialize_element(&(key, value))?;
        }
        seq.end()
    }
}

impl<'de, V: serde::Deserialize<'de>> serde::Deserialize<'de> for SortedStringMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = Vec::<(String, V)>::deserialize(deserializer)?;
        Ok(SortedStringMap(entries.into_iter().collect()))
    }
}

// ─── SortedIntMap: BTreeMap<i32, V> with array-of-pairs serde ───────────────

/// Transparent newtype around `BTreeMap<i32, V>` that serializes as
/// `[[key, value], ...]` (sorted by key) to match Java BTreeMap serialization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortedIntMap<V>(pub BTreeMap<i32, V>);

impl<V> SortedIntMap<V> {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }
}

impl<V> std::ops::Deref for SortedIntMap<V> {
    type Target = BTreeMap<i32, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<V> std::ops::DerefMut for SortedIntMap<V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<V> From<BTreeMap<i32, V>> for SortedIntMap<V> {
    fn from(map: BTreeMap<i32, V>) -> Self {
        Self(map)
    }
}

impl<V, const N: usize> From<[(i32, V); N]> for SortedIntMap<V> {
    fn from(arr: [(i32, V); N]) -> Self {
        Self(BTreeMap::from(arr))
    }
}

impl<V> IntoIterator for SortedIntMap<V> {
    type Item = (i32, V);
    type IntoIter = std::collections::btree_map::IntoIter<i32, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<V> std::iter::FromIterator<(i32, V)> for SortedIntMap<V> {
    fn from_iter<I: IntoIterator<Item = (i32, V)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a, V> IntoIterator for &'a SortedIntMap<V> {
    type Item = (&'a i32, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, i32, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<V: serde::Serialize> serde::Serialize for SortedIntMap<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            seq.serialize_element(&(key, value))?;
        }
        seq.end()
    }
}

impl<'de, V: serde::Deserialize<'de>> serde::Deserialize<'de> for SortedIntMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = Vec::<(i32, V)>::deserialize(deserializer)?;
        Ok(SortedIntMap(entries.into_iter().collect()))
    }
}

// Java: Region L9-L106
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Region {
    pub chr: String,
    pub start: i32,
    pub end: i32,
    pub gene: String,
    pub insert_start: i32,
    pub insert_end: i32,
}

impl Region {
    pub fn new(chr: impl Into<String>, start: i32, end: i32, gene: impl Into<String>) -> Self {
        Self::new_with_insert_range(chr, start, end, gene, 0, 0)
    }

    pub fn new_with_insert_range(
        chr: impl Into<String>,
        start: i32,
        end: i32,
        gene: impl Into<String>,
        insert_start: i32,
        insert_end: i32,
    ) -> Self {
        Self {
            chr: chr.into(),
            start,
            end,
            gene: gene.into(),
            insert_start,
            insert_end,
        }
    }

    pub fn new_modified_region(region: &Self, changed_start: i32, changed_end: i32) -> Self {
        Self::new_with_insert_range(
            region.chr.clone(),
            changed_start,
            changed_end,
            region.gene.clone(),
            region.insert_start,
            region.insert_end,
        )
    }

    pub fn print_region(&self) -> String {
        format!("{}:{}-{}", self.chr, self.start, self.end)
    }
}

impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        self.chr == other.chr && self.start == other.start && self.end == other.end
    }
}

impl Eq for Region {}

impl Hash for Region {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.chr.hash(state);
        self.start.hash(state);
        self.end.hash(state);
    }
}

// Java: Side L6-L11
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Three,
    Five,
    Unknown,
}

impl Side {
    pub fn from_int(side: i32) -> Self {
        match side {
            3 => Self::Three,
            5 => Self::Five,
            _ => Self::Unknown,
        }
    }
}

// Java: Variation L10-L125
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Variation {
    #[serde(rename = "varsCount")]
    pub vars_count: i32,

    #[serde(rename = "varsCountOnForward")]
    pub vars_count_on_forward: i32,

    #[serde(rename = "varsCountOnReverse")]
    pub vars_count_on_reverse: i32,

    #[serde(rename = "meanPosition")]
    pub mean_position: f64,

    #[serde(rename = "meanQuality")]
    pub mean_quality: f64,

    #[serde(rename = "meanMappingQuality")]
    pub mean_mapping_quality: f64,

    #[serde(rename = "numberOfMismatches")]
    pub number_of_mismatches: f64,

    #[serde(rename = "lowQualityReadsCount")]
    pub low_quality_reads_count: i32,

    #[serde(rename = "highQualityReadsCount")]
    pub high_quality_reads_count: i32,

    pub pstd: bool,

    pub qstd: bool,

    pub pp: i32,

    pub pq: f64,

    pub extracnt: i32,
}

impl Variation {
    /// Ported from: Variation.incDir() L81-L86
    /// dir: false = forward, true = reverse
    pub fn inc_dir(&mut self, dir: bool) {
        if dir {
            self.vars_count_on_reverse += 1;
        } else {
            self.vars_count_on_forward += 1;
        }
    }

    /// Ported from: Variation.decDir() L92-L97
    pub fn dec_dir(&mut self, dir: bool) {
        if dir {
            self.vars_count_on_reverse -= 1;
        } else {
            self.vars_count_on_forward -= 1;
        }
    }
}

// Java: Mate L4-L30
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Mate {
    #[serde(rename = "mateStart_ms")]
    pub mate_start_ms: i32,

    #[serde(rename = "mateEnd_me")]
    pub mate_end_me: i32,

    #[serde(rename = "mateLength_mlen")]
    pub mate_length_mlen: i32,

    #[serde(rename = "start_s")]
    pub start_s: i32,

    #[serde(rename = "end_e")]
    pub end_e: i32,

    #[serde(rename = "pmean_rp")]
    pub pmean_rp: f64,

    #[serde(rename = "qmean_q")]
    pub qmean_q: f64,

    #[serde(rename = "Qmean_Q")]
    pub qmean_qq: f64,

    pub nm: f64,
}

// Java: Cluster L6-L41
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Cluster {
    #[serde(flatten)]
    pub base: Mate,

    pub cnt: i32,
}

impl Cluster {
    /// Ported from: Cluster.java:L9-L15
    pub fn new(cnt: i32, mate_start_ms: i32, mate_end_me: i32, start_s: i32, end_e: i32) -> Self {
        Self {
            base: Mate {
                mate_start_ms,
                mate_end_me,
                start_s,
                end_e,
                ..Mate::default()
            },
            cnt,
        }
    }

    /// Ported from: Cluster.java:L17-L29
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_metrics(
        mate_start_ms: i32,
        mate_end_me: i32,
        cnt: i32,
        mate_length_mlen: i32,
        start_s: i32,
        end_e: i32,
        pmean_rp: f64,
        qmean_q: f64,
        qmean_qq: f64,
        nm: f64,
    ) -> Self {
        Self {
            base: Mate {
                mate_start_ms,
                mate_end_me,
                mate_length_mlen,
                start_s,
                end_e,
                pmean_rp,
                qmean_q,
                qmean_qq,
                nm,
            },
            cnt,
        }
    }
}

impl fmt::Display for Cluster {
    /// Ported from: Cluster.toString() L31-L40
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Cluster{{cnt={}, mateStart_ms={}, mateEnd_me={}, start_s={}, end_e={}}}",
            self.cnt,
            self.base.mate_start_ms,
            self.base.mate_end_me,
            self.base.start_s,
            self.base.end_e
        )
    }
}

// Java: Sclip L8-L48 (extends Variation)
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Sclip {
    #[serde(flatten)]
    pub base: Variation,
    pub nt: SortedIntMap<SortedStringMap<i32>>,
    pub seq: SortedIntMap<SortedStringMap<Variation>>,
    pub sequence: Option<String>,
    pub used: bool,
    pub start: i32,
    pub end: i32,
    pub mstart: i32,
    pub mend: i32,
    pub mlen: i32,
    pub disc: i32,
    pub softp: i32,
    #[serde(serialize_with = "crate::parity::format::serialize_indexmap_as_pairs")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_indexmap_as_pairs")]
    pub soft: IndexMap<i32, i32>,
    pub mates: Vec<Mate>,
}

// Java: VariationMap.SV (inner class) L24-L40
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct VariationMapSV {
    #[serde(rename = "type")]
    pub type_: Option<String>,

    pub pairs: i32,

    pub splits: i32,

    pub clusters: i32,
}

// Java: VariationMap L8-L65 (extends LinkedHashMap<String, Variation>)
// Always serializes as {"entries": [[k,v], ...], "sv": null|{...}}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct VariationMap {
    #[serde(serialize_with = "crate::parity::format::serialize_indexmap_as_pairs")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_indexmap_as_pairs")]
    pub entries: IndexMap<String, Variation>,

    pub sv: Option<VariationMapSV>,
}

// Java: SVStructures.java L1-L45
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SVStructures {
    pub svdelfend: i32,
    pub svdelrend: i32,
    pub svfdel: Vec<Sclip>,
    pub svrdel: Vec<Sclip>,

    pub svdupfend: i32,
    pub svduprend: i32,
    pub svfdup: Vec<Sclip>,
    pub svrdup: Vec<Sclip>,

    pub svinvfend3: i32,
    pub svinvrend3: i32,
    pub svfinv3: Vec<Sclip>,
    pub svrinv3: Vec<Sclip>,

    pub svinvfend5: i32,
    pub svinvrend5: i32,
    pub svfinv5: Vec<Sclip>,
    pub svrinv5: Vec<Sclip>,

    #[serde(serialize_with = "crate::parity::format::serialize_sorted_string_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_string_map")]
    pub svffus: HashMap<String, Vec<Sclip>>,

    #[serde(serialize_with = "crate::parity::format::serialize_sorted_string_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_string_map")]
    pub svrfus: HashMap<String, Vec<Sclip>>,

    #[serde(serialize_with = "crate::parity::format::serialize_sorted_string_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_string_map")]
    pub svfusfend: HashMap<String, i32>,

    #[serde(serialize_with = "crate::parity::format::serialize_sorted_string_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_string_map")]
    pub svfusrend: HashMap<String, i32>,
}

// Java: InitialData L14-L42
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct InitialData {
    #[serde(rename = "nonInsertionVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub non_insertion_variants: HashMap<i32, VariationMap>,

    #[serde(rename = "insertionVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub insertion_variants: HashMap<i32, VariationMap>,

    #[serde(rename = "refCoverage")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub ref_coverage: HashMap<i32, i32>,

    #[serde(rename = "softClips5End")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub soft_clips_5_end: HashMap<i32, Sclip>,

    #[serde(rename = "softClips3End")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub soft_clips_3_end: HashMap<i32, Sclip>,
}

impl InitialData {
    pub fn new(
        non_insertion_variants: HashMap<i32, VariationMap>,
        insertion_variants: HashMap<i32, VariationMap>,
        ref_coverage: HashMap<i32, i32>,
        soft_clips_3_end: HashMap<i32, Sclip>,
        soft_clips_5_end: HashMap<i32, Sclip>,
    ) -> Self {
        Self {
            non_insertion_variants,
            insertion_variants,
            ref_coverage,
            soft_clips_5_end,
            soft_clips_3_end,
        }
    }
}

// Java: VariationData L11-L40 (CigarParser output boundary type)
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct VariationData {
    #[serde(rename = "nonInsertionVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub non_insertion_variants: HashMap<i32, VariationMap>,

    #[serde(rename = "insertionVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub insertion_variants: HashMap<i32, VariationMap>,

    #[serde(rename = "positionToInsertionCount")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub position_to_insertion_count: HashMap<i32, SortedStringMap<i32>>,

    #[serde(rename = "positionToDeletionsCount")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub position_to_deletions_count: HashMap<i32, SortedStringMap<i32>>,

    #[serde(rename = "refCoverage")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub ref_coverage: HashMap<i32, i32>,

    #[serde(rename = "softClips5End")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub soft_clips_5_end: HashMap<i32, Sclip>,

    #[serde(rename = "softClips3End")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub soft_clips_3_end: HashMap<i32, Sclip>,

    #[serde(rename = "svStructures")]
    pub sv_structures: SVStructures,

    #[serde(rename = "maxReadLength")]
    pub max_read_length: Option<i32>,

    pub duprate: f64,

    pub splice: Option<BTreeSet<String>>,

    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub mnp: HashMap<i32, SortedStringMap<i32>>,

    #[serde(rename = "spliceCount")]
    pub splice_count: SortedStringMap<Vec<i32>>,
}

// Java: SortPositionSclip L8-L24
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SortPositionSclip {
    pub position: i32,
    pub soft_clip: Sclip,
    pub count: i32,
}

impl SortPositionSclip {
    pub fn new(position: i32, soft_clip: Sclip, count: i32) -> Self {
        Self {
            position,
            soft_clip,
            count,
        }
    }
}

impl PartialEq for SortPositionSclip {
    fn eq(&self, other: &Self) -> bool {
        self.position == other.position && self.count == other.count
    }
}

impl Eq for SortPositionSclip {}

impl Hash for SortPositionSclip {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.position.hash(state);
        self.count.hash(state);
    }
}

// Java: BaseInsertion L7-L24
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BaseInsertion {
    pub base_insert: Option<i32>,
    pub insertion_sequence: String,
    pub base_insert2: Option<i32>,
}

impl BaseInsertion {
    pub fn new(base_insert: i32, insertion_sequence: impl Into<String>, base_insert2: i32) -> Self {
        Self {
            base_insert: Some(base_insert),
            insertion_sequence: insertion_sequence.into(),
            base_insert2: Some(base_insert2),
        }
    }
}

// Java: Match L6-L13
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Match {
    pub base_position: i32,
    pub matched_sequence: String,
}

impl Match {
    pub fn new(base_position: i32, matched_sequence: impl Into<String>) -> Self {
        Self {
            base_position,
            matched_sequence: matched_sequence.into(),
        }
    }
}

// Java: Match35 L6-L23
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Match35 {
    pub matched_5_end: i32,
    pub matched_3_end: i32,
    pub max_matched_length: i32,
}

impl Match35 {
    pub fn new(matched_5_end: i32, matched_3_end: i32, max_matched_length: i32) -> Self {
        Self {
            matched_5_end,
            matched_3_end,
            max_matched_length,
        }
    }
}

// Java: ModifiedCigar L3-L14
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ModifiedCigar {
    pub position: i32,
    pub cigar: String,
    pub query_sequence: String,
    pub query_quality: String,
}

impl ModifiedCigar {
    pub fn new(
        position: i32,
        cigar: impl Into<String>,
        query_sequence: impl Into<String>,
        query_quality: impl Into<String>,
    ) -> Self {
        Self {
            position,
            cigar: cigar.into(),
            query_sequence: query_sequence.into(),
            query_quality: query_quality.into(),
        }
    }
}

// Java: CurrentSegment L6-L15
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CurrentSegment {
    pub chr: String,
    pub start: i32,
    pub end: i32,
}

impl CurrentSegment {
    pub fn new(chr: impl Into<String>, start: i32, end: i32) -> Self {
        Self {
            chr: chr.into(),
            start,
            end,
        }
    }
}

// Java: Variant L15-L190
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Variant {
    #[serde(rename = "descriptionString")]
    pub description_string: String,

    #[serde(rename = "positionCoverage")]
    pub position_coverage: i32,

    #[serde(rename = "varsCountOnForward")]
    pub vars_count_on_forward: i32,

    #[serde(rename = "varsCountOnReverse")]
    pub vars_count_on_reverse: i32,

    #[serde(rename = "strandBiasFlag")]
    pub strand_bias_flag: String,

    #[serde(rename = "frequency")]
    pub frequency: f64,

    #[serde(rename = "meanPosition")]
    pub mean_position: f64,

    #[serde(rename = "isAtLeastAt2Positions")]
    pub is_at_least_at_2_positions: bool,

    #[serde(rename = "meanQuality")]
    pub mean_quality: f64,

    #[serde(rename = "hasAtLeast2DiffQualities")]
    pub has_at_least_2_diff_qualities: bool,

    #[serde(rename = "meanMappingQuality")]
    pub mean_mapping_quality: f64,

    #[serde(rename = "highQualityToLowQualityRatio")]
    pub high_quality_to_low_quality_ratio: f64,

    #[serde(rename = "highQualityReadsFrequency")]
    pub high_quality_reads_frequency: f64,

    #[serde(rename = "extraFrequency")]
    pub extra_frequency: f64,

    #[serde(rename = "shift3")]
    pub shift3: i32,

    #[serde(rename = "msi")]
    pub msi: f64,

    #[serde(rename = "msint")]
    pub msint: i32,

    #[serde(rename = "numberOfMismatches")]
    pub number_of_mismatches: f64,

    #[serde(rename = "hicnt")]
    pub hicnt: i32,

    #[serde(rename = "hicov")]
    pub hicov: i32,

    #[serde(rename = "leftseq")]
    pub leftseq: String,

    #[serde(rename = "rightseq")]
    pub rightseq: String,

    #[serde(rename = "startPosition")]
    pub start_position: i32,

    #[serde(rename = "endPosition")]
    pub end_position: i32,

    #[serde(rename = "refReverseCoverage")]
    pub ref_reverse_coverage: i32,

    #[serde(rename = "refForwardCoverage")]
    pub ref_forward_coverage: i32,

    #[serde(rename = "totalPosCoverage")]
    pub total_pos_coverage: i32,

    #[serde(rename = "duprate")]
    pub duprate: f64,

    #[serde(rename = "genotype")]
    pub genotype: Option<String>,

    #[serde(rename = "varallele")]
    pub varallele: String,

    #[serde(rename = "refallele")]
    pub refallele: String,

    #[serde(rename = "vartype")]
    pub vartype: String,

    #[serde(rename = "DEBUG")]
    pub debug: String,

    #[serde(rename = "crispr")]
    pub crispr: i32,
}

impl Default for Variant {
    fn default() -> Self {
        Self {
            description_string: String::new(),
            position_coverage: 0,
            vars_count_on_forward: 0,
            vars_count_on_reverse: 0,
            strand_bias_flag: String::from("0"),
            frequency: 0.0,
            mean_position: 0.0,
            is_at_least_at_2_positions: false,
            mean_quality: 0.0,
            has_at_least_2_diff_qualities: false,
            mean_mapping_quality: 0.0,
            high_quality_to_low_quality_ratio: 0.0,
            high_quality_reads_frequency: 0.0,
            extra_frequency: 0.0,
            shift3: 0,
            msi: 0.0,
            msint: 0,
            number_of_mismatches: 0.0,
            hicnt: 0,
            hicov: 0,
            leftseq: String::new(),
            rightseq: String::new(),
            start_position: 0,
            end_position: 0,
            ref_reverse_coverage: 0,
            ref_forward_coverage: 0,
            total_pos_coverage: 0,
            duprate: 0.0,
            genotype: None,
            varallele: String::new(),
            refallele: String::new(),
            vartype: String::new(),
            debug: String::new(),
            crispr: 0,
        }
    }
}

impl Variant {
    /// Ported from: Variant.isNoise()
    /// Java source: Variant.java:L177-L195
    pub fn is_noise(&mut self, conf: &Configuration) -> bool {
        let qual = self.mean_quality;
        if ((qual < 4.5 || (qual < 12.0 && !self.has_at_least_2_diff_qualities))
            && self.position_coverage <= 3)
            || (qual < conf.goodq
                && self.frequency < 2.0 * conf.lofreq
                && self.position_coverage <= 1)
        {
            self.total_pos_coverage = self.total_pos_coverage.wrapping_sub(self.position_coverage);
            self.position_coverage = 0;
            self.vars_count_on_forward = 0;
            self.vars_count_on_reverse = 0;
            self.frequency = 0.0;
            self.high_quality_reads_frequency = 0.0;
            return true;
        }

        false
    }

    /// Ported from: Variant.adjComplex()
    /// Java source: Variant.java:L201-L241
    pub fn adj_complex(&mut self) {
        let mut ref_allele = self.refallele.clone();
        let mut var_allele = self.varallele.clone();

        if var_allele.as_bytes().first() == Some(&b'<') {
            return;
        }

        let ref_bytes = ref_allele.as_bytes();
        let var_bytes = var_allele.as_bytes();
        let mut n = 0_i32;
        while i32::try_from(ref_bytes.len()).expect("ref allele length exceeds i32") - n > 1
            && i32::try_from(var_bytes.len()).expect("var allele length exceeds i32") - n > 1
            && ref_bytes[usize::try_from(n).expect("negative prefix index")]
                == var_bytes[usize::try_from(n).expect("negative prefix index")]
        {
            n += 1;
        }

        if n > 0 {
            self.start_position += n;
            self.refallele =
                String::from_utf8_lossy(&substr(ref_allele.as_bytes(), n)).into_owned();
            self.varallele =
                String::from_utf8_lossy(&substr(var_allele.as_bytes(), n)).into_owned();
            self.leftseq
                .push_str(&String::from_utf8_lossy(&substr_with_len(
                    ref_allele.as_bytes(),
                    0,
                    n,
                )));
            self.leftseq =
                String::from_utf8_lossy(&substr(self.leftseq.as_bytes(), n)).into_owned();
        }

        ref_allele = self.refallele.clone();
        var_allele = self.varallele.clone();
        n = 1;
        while i32::try_from(ref_allele.len()).expect("ref allele length exceeds i32") - n > 0
            && i32::try_from(var_allele.len()).expect("var allele length exceeds i32") - n > 0
            && substr_with_len(ref_allele.as_bytes(), -n, 1)
                == substr_with_len(var_allele.as_bytes(), -n, 1)
        {
            n += 1;
        }

        if n > 1 {
            self.end_position -= n - 1;
            self.refallele =
                String::from_utf8_lossy(&substr_with_len(ref_allele.as_bytes(), 0, 1 - n))
                    .into_owned();
            self.varallele =
                String::from_utf8_lossy(&substr_with_len(var_allele.as_bytes(), 0, 1 - n))
                    .into_owned();
            self.rightseq = format!(
                "{}{}",
                String::from_utf8_lossy(&substr_with_len(ref_allele.as_bytes(), 1 - n, n - 1)),
                String::from_utf8_lossy(&substr_with_len(self.rightseq.as_bytes(), 0, 1 - n))
            );
        }
    }

    /// Ported from: Variant.varType()
    /// Java source: Variant.java:L249-L274
    #[allow(clippy::collapsible_if)]
    pub fn var_type(&self) -> String {
        if self.refallele == self.varallele && self.refallele.len() == 1 {
            return String::new();
        }

        if self.refallele.len() == 1 && self.varallele.len() == 1 {
            return String::from("SNV");
        }

        if let Some(captures) = ANY_SV.captures(&self.varallele) {
            if let Some(kind) = captures.get(1) {
                return kind.as_str().to_string();
            }
        }

        if self.refallele.is_empty() || self.varallele.is_empty() {
            return String::from("Complex");
        }

        if self.refallele.as_bytes()[0] != self.varallele.as_bytes()[0] {
            return String::from("Complex");
        }

        if self.refallele.len() == 1
            && self.varallele.len() > 1
            && self.varallele.starts_with(&self.refallele)
        {
            return String::from("Insertion");
        }

        if self.refallele.len() > 1
            && self.varallele.len() == 1
            && self.refallele.starts_with(&self.varallele)
        {
            return String::from("Deletion");
        }

        String::from("Complex")
    }

    /// Ported from: Variant.isGoodVar(Variant, String, Set<String>)
    /// Java source: Variant.java:L283-L348
    #[allow(clippy::collapsible_if)]
    pub fn is_good_var(
        &self,
        reference_var: Option<&Variant>,
        var_type: Option<&str>,
        splice: &HashSet<String>,
        conf: &Configuration,
    ) -> bool {
        if self.refallele.is_empty() {
            return false;
        }

        let resolved_type = match var_type {
            Some(kind) if !kind.is_empty() => kind.to_string(),
            _ => self.var_type(),
        };

        if self.frequency < conf.freq
            || self.hicnt < conf.minr
            || self.mean_position < f64::from(conf.read_pos_filter)
            || self.mean_quality < conf.goodq
        {
            return false;
        }

        if let Some(reference_var) = reference_var {
            if reference_var.hicnt > conf.minr && self.frequency < 0.25 {
                let d = self.mean_mapping_quality
                    + self.refallele.len() as f64
                    + self.varallele.len() as f64;
                let f = (1.0 + d) / (reference_var.mean_mapping_quality + 1.0);
                if ((d - 2.0 < 5.0) && reference_var.mean_mapping_quality > 20.0) || f < 0.25 {
                    return false;
                }
            }
        }

        if resolved_type == "Deletion"
            && splice.contains(&format!("{}-{}", self.start_position, self.end_position))
        {
            return false;
        }

        if self.high_quality_to_low_quality_ratio < conf.qratio {
            return false;
        }

        if self.frequency > 0.30 {
            return true;
        }

        if self.mean_mapping_quality < conf.mapq {
            return false;
        }

        if self.msi >= 15.0 && self.frequency <= conf.monomer_msi_frequency && self.msint == 1 {
            return false;
        }

        if self.msi >= 12.0 && self.frequency <= conf.non_monomer_msi_frequency && self.msint > 1 {
            return false;
        }

        if self.strand_bias_flag == "2;1" && self.frequency < 0.20 {
            if resolved_type == "SNV" || (self.refallele.len() < 3 && self.varallele.len() < 3) {
                return false;
            }
        }

        true
    }
}

// Java: Vars L11-L27
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Vars {
    #[serde(rename = "referenceVariant")]
    pub reference_variant: Option<Variant>,

    #[serde(rename = "variants")]
    pub variants: Vec<Variant>,

    #[serde(rename = "varDescriptionStringToVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_btreemap_as_pairs")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_btreemap_as_pairs")]
    pub var_description_string_to_variants: BTreeMap<String, Variant>,

    #[serde(rename = "sv")]
    pub sv: String,
}

// Java: RealignedVariationData (VariationRealigner output boundary type)
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RealignedVariationData {
    #[serde(rename = "nonInsertionVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub non_insertion_variants: HashMap<i32, VariationMap>,

    #[serde(rename = "insertionVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub insertion_variants: HashMap<i32, VariationMap>,

    #[serde(rename = "softClips5End")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub soft_clips_5_end: HashMap<i32, Sclip>,

    #[serde(rename = "softClips3End")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub soft_clips_3_end: HashMap<i32, Sclip>,

    #[serde(rename = "refCoverage")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub ref_coverage: HashMap<i32, i32>,

    #[serde(rename = "maxReadLength")]
    pub max_read_length: Option<i32>,

    #[serde(rename = "svStructures")]
    pub sv_structures: SVStructures,

    pub duprate: f64,

    #[serde(rename = "CURSEG")]
    pub curseg: CurrentSegment,

    #[serde(rename = "SOFTP2SV")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub softp2sv: HashMap<i32, Vec<Sclip>>,

    // Java: RealignerJsonl always emits "previousScope":null (hardcoded)
    #[serde(rename = "previousScope")]
    pub previous_scope: Option<()>,
}

// Java: AlignedVarsData (ToVarsBuilder output boundary type)
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct AlignedVarsData {
    #[serde(rename = "maxReadLength")]
    pub max_read_length: i32,

    #[serde(rename = "alignedVariants")]
    #[serde(serialize_with = "crate::parity::format::serialize_sorted_int_map")]
    #[serde(deserialize_with = "crate::parity::format::deserialize_sorted_int_map")]
    pub aligned_variants: HashMap<i32, Vars>,
}

// Java: CombineAnalysisData L7-L15
#[derive(Debug)]
pub struct CombineAnalysisData {
    pub max_read_length: i32,
    pub type_: String,
}

impl CombineAnalysisData {
    pub fn new(max_read_length: i32, type_: impl Into<String>) -> Self {
        Self {
            max_read_length,
            type_: type_.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::Value;
    use std::collections::hash_map::DefaultHasher;

    fn hash_region(region: &Region) -> u64 {
        let mut hasher = DefaultHasher::new();
        region.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn region_new_sets_default_insert_range() {
        let region = Region::new("chr1", 10, 20, "GENE1");

        assert_eq!(region.chr, "chr1");
        assert_eq!(region.start, 10);
        assert_eq!(region.end, 20);
        assert_eq!(region.gene, "GENE1");
        assert_eq!(region.insert_start, 0);
        assert_eq!(region.insert_end, 0);
        assert_eq!(region.print_region(), "chr1:10-20");
    }

    #[test]
    fn region_modified_region_preserves_metadata() {
        let region = Region::new_with_insert_range("chr2", 100, 200, "GENE2", 110, 190);
        let modified = Region::new_modified_region(&region, 90, 210);

        assert_eq!(modified.chr, "chr2");
        assert_eq!(modified.start, 90);
        assert_eq!(modified.end, 210);
        assert_eq!(modified.gene, "GENE2");
        assert_eq!(modified.insert_start, 110);
        assert_eq!(modified.insert_end, 190);
    }

    #[test]
    fn region_equality_and_hash_only_use_coordinates() {
        let left = Region::new_with_insert_range("chr3", 7, 9, "GENE_A", 1, 2);
        let right = Region::new_with_insert_range("chr3", 7, 9, "GENE_B", 10, 20);

        assert_eq!(left, right);
        assert_eq!(hash_region(&left), hash_region(&right));
    }

    #[test]
    fn side_from_int_maps_java_values() {
        assert_eq!(Side::from_int(3), Side::Three);
        assert_eq!(Side::from_int(5), Side::Five);
        assert_eq!(Side::from_int(0), Side::Unknown);
        assert_eq!(Side::from_int(-1), Side::Unknown);
    }

    #[test]
    fn small_data_types_construct() {
        let sort_position = SortPositionSclip::new(12, Sclip::default(), 3);
        let insertion = BaseInsertion::new(8, "AC", 6);
        let matched = Match::new(15, "TT");
        let matched_ends = Match35::new(5, 9, 4);
        let modified_cigar = ModifiedCigar::new(20, "10M1I5M", "ACGT", "!!!!");
        let segment = CurrentSegment::new("chr7", 30, 40);

        assert_eq!(sort_position.position, 12);
        assert_eq!(sort_position.count, 3);
        assert_eq!(insertion.base_insert, Some(8));
        assert_eq!(insertion.insertion_sequence, "AC");
        assert_eq!(insertion.base_insert2, Some(6));
        assert_eq!(matched.base_position, 15);
        assert_eq!(matched.matched_sequence, "TT");
        assert_eq!(matched_ends.matched_5_end, 5);
        assert_eq!(matched_ends.matched_3_end, 9);
        assert_eq!(matched_ends.max_matched_length, 4);
        assert_eq!(modified_cigar.position, 20);
        assert_eq!(modified_cigar.cigar, "10M1I5M");
        assert_eq!(modified_cigar.query_sequence, "ACGT");
        assert_eq!(modified_cigar.query_quality, "!!!!");
        assert_eq!(segment.chr, "chr7");
        assert_eq!(segment.start, 30);
        assert_eq!(segment.end, 40);
    }

    #[test]
    fn initial_data_default_creates_empty_maps() {
        let initial_data = InitialData::default();

        assert!(initial_data.non_insertion_variants.is_empty());
        assert!(initial_data.insertion_variants.is_empty());
        assert!(initial_data.ref_coverage.is_empty());
        assert!(initial_data.soft_clips_5_end.is_empty());
        assert!(initial_data.soft_clips_3_end.is_empty());
    }

    #[test]
    fn combine_analysis_data_new_sets_fields() {
        let combine_analysis_data = CombineAnalysisData::new(150, "SNV");

        assert_eq!(combine_analysis_data.max_read_length, 150);
        assert_eq!(combine_analysis_data.type_, "SNV");
    }

    #[test]
    fn variation_data_default_serializes_all_expected_keys() {
        let serialized = serde_json::to_value(VariationData::default()).unwrap();
        let Value::Object(object) = serialized else {
            panic!("expected VariationData to serialize as a JSON object");
        };

        assert_eq!(object.len(), 13);
        assert!(object.contains_key("nonInsertionVariants"));
        assert!(object.contains_key("insertionVariants"));
        assert!(object.contains_key("positionToInsertionCount"));
        assert!(object.contains_key("positionToDeletionsCount"));
        assert!(object.contains_key("refCoverage"));
        assert!(object.contains_key("softClips5End"));
        assert!(object.contains_key("softClips3End"));
        assert!(object.contains_key("svStructures"));
        assert!(object.contains_key("maxReadLength"));
        assert!(object.contains_key("duprate"));
        assert!(object.contains_key("splice"));
        assert!(object.contains_key("mnp"));
        assert!(object.contains_key("spliceCount"));
    }

    #[test]
    fn variant_default_serializes_all_expected_keys() {
        let serialized = serde_json::to_value(Variant::default()).unwrap();
        let Value::Object(object) = serialized else {
            panic!("expected Variant to serialize as a JSON object");
        };

        assert_eq!(object.len(), 34);
        assert_eq!(
            object.get("strandBiasFlag"),
            Some(&Value::String(String::from("0")))
        );
        assert!(object.contains_key("DEBUG"));
    }

    fn base_config() -> Configuration {
        Configuration::default()
    }

    fn baseline_variant() -> Variant {
        Variant {
            refallele: String::from("A"),
            varallele: String::from("T"),
            frequency: 0.31,
            hicnt: 4,
            mean_position: 12.0,
            mean_quality: 30.0,
            high_quality_to_low_quality_ratio: 2.0,
            mean_mapping_quality: 25.0,
            strand_bias_flag: String::from("0"),
            start_position: 100,
            end_position: 100,
            ..Variant::default()
        }
    }

    #[test]
    fn variant_is_noise_zeroes_fields_for_low_quality_low_coverage_calls() {
        let mut variant = Variant {
            mean_quality: 4.4,
            has_at_least_2_diff_qualities: false,
            position_coverage: 3,
            total_pos_coverage: 10,
            vars_count_on_forward: 2,
            vars_count_on_reverse: 1,
            frequency: 0.15,
            high_quality_reads_frequency: 0.12,
            ..Variant::default()
        };

        assert!(variant.is_noise(&base_config()));
        assert_eq!(variant.total_pos_coverage, 7);
        assert_eq!(variant.position_coverage, 0);
        assert_eq!(variant.vars_count_on_forward, 0);
        assert_eq!(variant.vars_count_on_reverse, 0);
        assert_eq!(variant.frequency, 0.0);
        assert_eq!(variant.high_quality_reads_frequency, 0.0);
    }

    #[test]
    fn variant_adj_complex_trims_shared_prefix_and_suffix() {
        let mut variant = Variant {
            refallele: String::from("AACG"),
            varallele: String::from("AATG"),
            leftseq: String::from("TTAA"),
            rightseq: String::from("GGCC"),
            start_position: 10,
            end_position: 13,
            ..Variant::default()
        };

        variant.adj_complex();

        assert_eq!(variant.start_position, 12);
        assert_eq!(variant.end_position, 12);
        assert_eq!(variant.refallele, "C");
        assert_eq!(variant.varallele, "T");
        assert_eq!(variant.leftseq, "AAAA");
        assert_eq!(variant.rightseq, "GGGC");
    }

    #[test]
    fn variant_var_type_matches_java_classification_rules() {
        let snv = Variant {
            refallele: String::from("A"),
            varallele: String::from("T"),
            ..Variant::default()
        };
        let insertion = Variant {
            refallele: String::from("A"),
            varallele: String::from("AT"),
            ..Variant::default()
        };
        let deletion = Variant {
            refallele: String::from("AT"),
            varallele: String::from("A"),
            ..Variant::default()
        };
        let structural = Variant {
            refallele: String::from("A"),
            varallele: String::from("<DEL>"),
            ..Variant::default()
        };

        assert_eq!(snv.var_type(), "SNV");
        assert_eq!(insertion.var_type(), "Insertion");
        assert_eq!(deletion.var_type(), "Deletion");
        assert_eq!(structural.var_type(), "DEL");
    }

    #[test]
    fn variant_is_good_var_rejects_splice_deletions() {
        let variant = Variant {
            refallele: String::from("AT"),
            varallele: String::from("A"),
            start_position: 100,
            end_position: 101,
            ..baseline_variant()
        };
        let mut splice = HashSet::new();
        splice.insert(String::from("100-101"));

        assert!(!variant.is_good_var(None, Some("Deletion"), &splice, &base_config()));
    }

    #[test]
    fn variant_is_good_var_rejects_low_quality_ratio_before_mapq_checks() {
        let mut config = base_config();
        config.qratio = 1.5;
        let variant = Variant {
            high_quality_to_low_quality_ratio: 1.49,
            frequency: 0.29,
            ..baseline_variant()
        };

        assert!(!variant.is_good_var(None, Some("SNV"), &HashSet::new(), &config));
    }

    #[test]
    fn variant_is_good_var_applies_reference_mapq_penalty() {
        let variant = Variant {
            refallele: String::from("A"),
            varallele: String::from("AT"),
            frequency: 0.2,
            mean_mapping_quality: 3.0,
            ..baseline_variant()
        };
        let reference_variant = Variant {
            hicnt: 5,
            mean_mapping_quality: 30.0,
            ..Variant::default()
        };

        assert!(!variant.is_good_var(
            Some(&reference_variant),
            Some("Insertion"),
            &HashSet::new(),
            &base_config()
        ));
    }

    #[test]
    fn variant_is_good_var_accepts_high_frequency_call_early() {
        let mut config = base_config();
        config.mapq = 60.0;
        let variant = baseline_variant();

        assert!(variant.is_good_var(None, None, &HashSet::new(), &config));
    }

    #[test]
    fn vars_default_serializes_expected_keys() {
        let serialized = serde_json::to_value(Vars::default()).unwrap();
        let Value::Object(object) = serialized else {
            panic!("expected Vars to serialize as a JSON object");
        };

        assert_eq!(object.len(), 4);
        assert!(object.contains_key("referenceVariant"));
        assert!(object.contains_key("variants"));
        assert!(object.contains_key("varDescriptionStringToVariants"));
        assert!(object.contains_key("sv"));
    }

    #[test]
    fn sv_structures_default_matches_java_initialization() {
        let sv_structures = SVStructures::default();

        assert_eq!(sv_structures.svdelfend, 0);
        assert_eq!(sv_structures.svdelrend, 0);
        assert_eq!(sv_structures.svdupfend, 0);
        assert_eq!(sv_structures.svduprend, 0);
        assert_eq!(sv_structures.svinvfend3, 0);
        assert_eq!(sv_structures.svinvrend3, 0);
        assert_eq!(sv_structures.svinvfend5, 0);
        assert_eq!(sv_structures.svinvrend5, 0);
        assert!(sv_structures.svfdel.is_empty());
        assert!(sv_structures.svrdel.is_empty());
        assert!(sv_structures.svfdup.is_empty());
        assert!(sv_structures.svrdup.is_empty());
        assert!(sv_structures.svfinv3.is_empty());
        assert!(sv_structures.svfinv5.is_empty());
        assert!(sv_structures.svrinv3.is_empty());
        assert!(sv_structures.svrinv5.is_empty());
        assert!(sv_structures.svffus.is_empty());
        assert!(sv_structures.svrfus.is_empty());
        assert!(sv_structures.svfusfend.is_empty());
        assert!(sv_structures.svfusrend.is_empty());
    }

    #[test]
    fn sv_structures_fields_mutate_as_expected() {
        let mut sv_structures = SVStructures::default();
        let forward_clip = Sclip {
            sequence: Some(String::from("ACGT")),
            start: 101,
            end: 125,
            ..Sclip::default()
        };
        let reverse_clip = Sclip {
            sequence: Some(String::from("TGCA")),
            start: 130,
            end: 160,
            ..Sclip::default()
        };

        sv_structures.svdelfend = 11;
        sv_structures.svdelrend = 12;
        sv_structures.svdupfend = 21;
        sv_structures.svduprend = 22;
        sv_structures.svinvfend3 = 31;
        sv_structures.svinvrend3 = 32;
        sv_structures.svinvfend5 = 41;
        sv_structures.svinvrend5 = 42;
        sv_structures.svfdel.push(forward_clip.clone());
        sv_structures.svrdel.push(reverse_clip.clone());
        sv_structures.svfdup.push(forward_clip.clone());
        sv_structures.svrdup.push(reverse_clip.clone());
        sv_structures.svfinv3.push(forward_clip.clone());
        sv_structures.svfinv5.push(reverse_clip.clone());
        sv_structures.svrinv3.push(forward_clip.clone());
        sv_structures.svrinv5.push(reverse_clip.clone());
        sv_structures
            .svffus
            .insert(String::from("chr2"), vec![forward_clip.clone()]);
        sv_structures
            .svrfus
            .insert(String::from("chr3"), vec![reverse_clip.clone()]);
        sv_structures.svfusfend.insert(String::from("chr2"), 500);
        sv_structures.svfusrend.insert(String::from("chr3"), 750);

        assert_eq!(sv_structures.svdelfend, 11);
        assert_eq!(sv_structures.svdelrend, 12);
        assert_eq!(sv_structures.svdupfend, 21);
        assert_eq!(sv_structures.svduprend, 22);
        assert_eq!(sv_structures.svinvfend3, 31);
        assert_eq!(sv_structures.svinvrend3, 32);
        assert_eq!(sv_structures.svinvfend5, 41);
        assert_eq!(sv_structures.svinvrend5, 42);
        assert_eq!(sv_structures.svfdel.len(), 1);
        assert_eq!(sv_structures.svrdel.len(), 1);
        assert_eq!(sv_structures.svfdup.len(), 1);
        assert_eq!(sv_structures.svrdup.len(), 1);
        assert_eq!(sv_structures.svfinv3.len(), 1);
        assert_eq!(sv_structures.svfinv5.len(), 1);
        assert_eq!(sv_structures.svrinv3.len(), 1);
        assert_eq!(sv_structures.svrinv5.len(), 1);
        assert_eq!(
            sv_structures.svffus["chr2"][0].sequence.as_deref(),
            Some("ACGT")
        );
        assert_eq!(
            sv_structures.svrfus["chr3"][0].sequence.as_deref(),
            Some("TGCA")
        );
        assert_eq!(sv_structures.svfusfend["chr2"], 500);
        assert_eq!(sv_structures.svfusrend["chr3"], 750);
    }

    #[test]
    fn sv_structures_serde_roundtrip_preserves_fields() {
        let mut sv_structures = SVStructures {
            svdelfend: 10,
            svdelrend: 11,
            svdupfend: 20,
            svduprend: 21,
            svinvfend3: 30,
            svinvrend3: 31,
            svinvfend5: 40,
            svinvrend5: 41,
            ..SVStructures::default()
        };
        let fusion_clip = Sclip {
            sequence: Some(String::from("GGGG")),
            start: 201,
            end: 230,
            ..Sclip::default()
        };
        sv_structures.svfinv3.push(fusion_clip.clone());
        sv_structures
            .svffus
            .insert(String::from("chr10"), vec![fusion_clip.clone()]);
        sv_structures.svfusfend.insert(String::from("chr10"), 901);
        sv_structures.svfusrend.insert(String::from("chr11"), 902);

        let json = serde_json::to_string(&sv_structures).unwrap();
        let roundtrip: SVStructures = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.svdelfend, 10);
        assert_eq!(roundtrip.svdupfend, 20);
        assert_eq!(roundtrip.svinvrend5, 41);
        assert_eq!(roundtrip.svfinv3.len(), 1);
        assert_eq!(
            roundtrip.svffus["chr10"][0].sequence.as_deref(),
            Some("GGGG")
        );
        assert_eq!(roundtrip.svfusfend["chr10"], 901);
        assert_eq!(roundtrip.svfusrend["chr11"], 902);
    }

    #[test]
    fn variation_map_sv_default_matches_java_zero_state() {
        let sv = VariationMapSV::default();

        assert_eq!(sv.type_, None);
        assert_eq!(sv.pairs, 0);
        assert_eq!(sv.splits, 0);
        assert_eq!(sv.clusters, 0);
    }

    proptest! {
        #[test]
        fn variant_f64_roundtrip(
            freq in proptest::num::f64::NORMAL,
            msi in proptest::num::f64::NORMAL,
            duprate in proptest::num::f64::NORMAL,
        ) {
            let variant = Variant {
                frequency: freq,
                msi,
                duprate,
                ..Variant::default()
            };

            let json = serde_json::to_string(&variant).unwrap();
            let roundtrip: Variant = serde_json::from_str(&json).unwrap();

            prop_assert_eq!(roundtrip.frequency, variant.frequency);
            prop_assert_eq!(roundtrip.msi, variant.msi);
            prop_assert_eq!(roundtrip.duprate, variant.duprate);
        }
    }

    #[test]
    fn realigned_variation_data_default_serializes_expected_keys() {
        let serialized = serde_json::to_value(RealignedVariationData::default()).unwrap();
        let Value::Object(object) = serialized else {
            panic!("expected RealignedVariationData to serialize as a JSON object");
        };

        assert_eq!(object.len(), 11);
        assert!(object.contains_key("nonInsertionVariants"));
        assert!(object.contains_key("insertionVariants"));
        assert!(object.contains_key("softClips5End"));
        assert!(object.contains_key("softClips3End"));
        assert!(object.contains_key("refCoverage"));
        assert!(object.contains_key("maxReadLength"));
        assert!(object.contains_key("svStructures"));
        assert!(object.contains_key("duprate"));
        assert!(object.contains_key("CURSEG"));
        assert!(object.contains_key("SOFTP2SV"));
        // Java always emits "previousScope":null (RealignerJsonl.java L57)
        assert!(object.contains_key("previousScope"));
        assert!(object["previousScope"].is_null());
    }

    #[test]
    fn aligned_vars_data_default_serializes_expected_keys() {
        let serialized = serde_json::to_value(AlignedVarsData::default()).unwrap();
        let Value::Object(object) = serialized else {
            panic!("expected AlignedVarsData to serialize as a JSON object");
        };

        assert_eq!(object.len(), 2);
        assert!(object.contains_key("maxReadLength"));
        assert!(object.contains_key("alignedVariants"));
    }
}
