use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::time::SystemTime;

use rust_htslib::faidx;
use rustc_hash::FxHashMap;
use serde::de::Deserializer;
use serde::ser::{SerializeMap, Serializer};
use smallvec::{SmallVec, smallvec};

use crate::data::Region;
use crate::patterns::{UNABLE_FIND_CONTIG, WRONG_START_OR_END};
use crate::prelude::HashMap;

// Java: Configuration.SEED_1 L203
const SEED_1: i32 = 17;
// Java: Configuration.SEED_2 L208
const SEED_2: i32 = 12;
const MAX_REFERENCE_SEED_LEN: usize = SEED_1 as usize;

thread_local! {
    static THREAD_LOCAL_FASTA_FILES: RefCell<HashMap<String, faidx::Reader>> = RefCell::new(HashMap::default());
}

const _: fn() = {
    fn check() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<Reference>();
        assert_send_sync::<ReferenceResource>();
    }

    check
};

fn wrong_fasta_or_bam_message(chr: &str) -> String {
    format!(
        "The name of this chromosome \"{}\" is missing in your fasta file. Please be sure that chromosome names in BAM, fasta and BED are in correspondence with each other and you use correct fasta for your BAM (can be checked in BAM header).",
        chr
    )
}

fn region_boundaries_message(chr: &str, start: i32, end: i32) -> String {
    format!(
        "The region {}:{}-{} is wrong. We have problem while reading it, possible the start is after the end of the region or the fasta doesn't contain this region.",
        chr, start, end
    )
}

// Java: Reference L9-L49
#[derive(Clone, Debug, Default)]
pub struct ReferenceSequenceMap {
    spans: Vec<ReferenceSequenceSpan>,
}

#[derive(Clone, Debug)]
struct ReferenceSequenceSpan {
    start: i32,
    bases: Vec<u8>,
}

impl ReferenceSequenceMap {
    pub fn get(&self, position: &i32) -> Option<&u8> {
        self.spans.iter().find_map(|span| span.get(*position))
    }

    pub fn contains_key(&self, position: &i32) -> bool {
        self.get(position).is_some()
    }

    pub fn max_position(&self) -> Option<i32> {
        self.spans
            .iter()
            .filter_map(|span| span.end_exclusive().checked_sub(1))
            .max()
    }

    pub fn insert(&mut self, position: i32, base: u8) -> Option<u8> {
        for span in &mut self.spans {
            if let Some(previous) = span.replace(position, base) {
                return Some(previous);
            }
            if span.try_extend(position, base) {
                return None;
            }
        }
        self.spans.push(ReferenceSequenceSpan {
            start: position,
            bases: vec![base],
        });
        None
    }
}

impl Extend<(i32, u8)> for ReferenceSequenceMap {
    fn extend<T: IntoIterator<Item = (i32, u8)>>(&mut self, iter: T) {
        for (position, base) in iter {
            self.insert(position, base);
        }
    }
}

impl FromIterator<(i32, u8)> for ReferenceSequenceMap {
    fn from_iter<T: IntoIterator<Item = (i32, u8)>>(iter: T) -> Self {
        let mut map = Self::default();
        map.extend(iter);
        map
    }
}

impl Index<&i32> for ReferenceSequenceMap {
    type Output = u8;

    fn index(&self, index: &i32) -> &Self::Output {
        self.get(index)
            .expect("reference sequence position is not loaded")
    }
}

impl ReferenceSequenceSpan {
    fn end_exclusive(&self) -> i32 {
        self.start + i32::try_from(self.bases.len()).expect("reference span length exceeds i32")
    }

    fn index_of(&self, position: i32) -> Option<usize> {
        (position >= self.start && position < self.end_exclusive()).then(|| {
            usize::try_from(position - self.start)
                .expect("validated reference offset is non-negative")
        })
    }

    fn get(&self, position: i32) -> Option<&u8> {
        self.index_of(position).map(|index| &self.bases[index])
    }

    fn replace(&mut self, position: i32, base: u8) -> Option<u8> {
        let index = self.index_of(position)?;
        Some(std::mem::replace(&mut self.bases[index], base))
    }

    fn try_extend(&mut self, position: i32, base: u8) -> bool {
        if position == self.end_exclusive() {
            self.bases.push(base);
            true
        } else if position + 1 == self.start {
            self.start = position;
            self.bases.insert(0, base);
            true
        } else {
            false
        }
    }
}

impl serde::Serialize for ReferenceSequenceMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let len = self.spans.iter().map(|span| span.bases.len()).sum();
        let mut map = serializer.serialize_map(Some(len))?;
        for span in &self.spans {
            for (offset, base) in span.bases.iter().enumerate() {
                let position =
                    span.start + i32::try_from(offset).expect("reference span offset exceeds i32");
                map.serialize_entry(&position, base)?;
            }
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for ReferenceSequenceMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = BTreeMap::<i32, u8>::deserialize(deserializer)?;
        let mut map = Self::default();
        for (position, base) in entries {
            map.insert(position, base);
        }
        Ok(map)
    }
}

pub type ReferenceSeedPositions = SmallVec<[i32; 1]>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReferenceSeedKey {
    len: u8,
    bytes: [u8; MAX_REFERENCE_SEED_LEN],
}

impl ReferenceSeedKey {
    fn from_sequence(sequence: &str) -> Self {
        let sequence_bytes = sequence.as_bytes();
        assert!(
            sequence_bytes.len() <= MAX_REFERENCE_SEED_LEN,
            "reference seed exceeds maximum seed length"
        );

        let mut bytes = [0; MAX_REFERENCE_SEED_LEN];
        bytes[..sequence_bytes.len()].copy_from_slice(sequence_bytes);
        Self {
            len: u8::try_from(sequence_bytes.len()).expect("reference seed length exceeds u8"),
            bytes,
        }
    }

    fn as_str(&self) -> &str {
        // Safety: `bytes[..len]` were copied from a `&str` in `from_sequence`, so they
        // are guaranteed valid UTF-8. Skipping the O(len) validation scan saves ~12B
        // CPU cycles on the chr16 hot path where this is called ~46M times during
        // reference seed-map construction.
        unsafe { std::str::from_utf8_unchecked(&self.bytes[..usize::from(self.len)]) }
    }
}

impl Borrow<str> for ReferenceSeedKey {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Hash for ReferenceSeedKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ReferenceSeedMap {
    entries: FxHashMap<ReferenceSeedKey, ReferenceSeedPositions>,
}

impl ReferenceSeedMap {
    /// Reserve capacity for `additional` more seed entries, sizing the map to the
    /// per-region window up front. The seed map is built by inserting ~2 entries
    /// (SEED_1 + SEED_2) for every base of the exon window; starting from an empty
    /// map otherwise forces an O(log n) rehash storm during construction, which the
    /// allocation profile showed as the single largest allocation site (~20% of all
    /// allocation). Pure capacity hint -- no effect on contents or lookups.
    pub fn reserve(&mut self, additional: usize) {
        self.entries.reserve(additional);
    }

    pub fn get(&self, sequence: &str) -> Option<&ReferenceSeedPositions> {
        self.entries.get(sequence)
    }

    fn get_mut(&mut self, sequence: &str) -> Option<&mut ReferenceSeedPositions> {
        self.entries.get_mut(sequence)
    }

    pub fn insert<S: AsRef<str>>(
        &mut self,
        sequence: S,
        positions: ReferenceSeedPositions,
    ) -> Option<ReferenceSeedPositions> {
        self.entries.insert(
            ReferenceSeedKey::from_sequence(sequence.as_ref()),
            positions,
        )
    }
}

impl Index<&str> for ReferenceSeedMap {
    type Output = ReferenceSeedPositions;

    fn index(&self, sequence: &str) -> &Self::Output {
        self.get(sequence)
            .expect("reference seed sequence is not loaded")
    }
}

impl serde::Serialize for ReferenceSeedMap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (seed, positions) in &self.entries {
            map.serialize_entry(seed.as_str(), positions)?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for ReferenceSeedMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = HashMap::<String, ReferenceSeedPositions>::deserialize(deserializer)?;
        let mut seed = Self::default();
        for (sequence, positions) in entries {
            seed.insert(sequence, positions);
        }
        Ok(seed)
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Reference {
    #[serde(rename = "loadedRegions")]
    pub loaded_regions: Vec<LoadedRegion>,

    #[serde(rename = "referenceSequences")]
    pub reference_sequences: ReferenceSequenceMap,

    pub seed: ReferenceSeedMap,
}

impl Reference {
    /// Ported from: Reference.Reference() L14-L18
    pub fn new() -> Self {
        Self::default()
    }
}

// Java: Reference.LoadedRegion L23-L48
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LoadedRegion {
    pub chr: String,

    #[serde(rename = "sequenceStart")]
    pub sequence_start: i32,

    #[serde(rename = "sequenceEnd")]
    pub sequence_end: i32,
}

impl LoadedRegion {
    /// Ported from: Reference.LoadedRegion.LoadedRegion() L28-L32
    pub fn new(chr: impl Into<String>, sequence_start: i32, sequence_end: i32) -> Self {
        Self {
            chr: chr.into(),
            sequence_start,
            sequence_end,
        }
    }
}

#[derive(Debug)]
pub enum ReferenceError {
    OpenReference {
        file: String,
        source: rust_htslib::errors::Error,
    },
    WrongFastaOrBam {
        chr: String,
        details: String,
    },
    RegionBoundaries {
        chr: String,
        start: i32,
        end: i32,
        details: String,
    },
    Htslib(String),
}

impl ReferenceError {
    fn open_reference(file: &str, source: rust_htslib::errors::Error) -> Self {
        Self::OpenReference {
            file: file.to_string(),
            source,
        }
    }

    fn wrong_fasta_or_bam(chr: &str, details: String) -> Self {
        Self::WrongFastaOrBam {
            chr: chr.to_string(),
            details,
        }
    }

    fn region_boundaries(chr: &str, start: i32, end: i32, details: String) -> Self {
        Self::RegionBoundaries {
            chr: chr.to_string(),
            start,
            end,
            details,
        }
    }
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenReference { file, source } => {
                write!(
                    formatter,
                    "Couldn't open reference file: {} ({source})",
                    file
                )
            }
            Self::WrongFastaOrBam { chr, .. } => {
                formatter.write_str(&wrong_fasta_or_bam_message(chr))
            }
            Self::RegionBoundaries {
                chr, start, end, ..
            } => formatter.write_str(&region_boundaries_message(chr, *start, *end)),
            Self::Htslib(message) => formatter.write_str(message),
        }
    }
}

impl Error for ReferenceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::OpenReference { source, .. } => Some(source),
            Self::WrongFastaOrBam { .. } | Self::RegionBoundaries { .. } | Self::Htslib(_) => None,
        }
    }
}

// Java: ReferenceResource L23-L168
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ReferenceResource {
    pub fasta: String,

    #[serde(rename = "referenceExtension")]
    pub reference_extension: i32,

    #[serde(rename = "numberNucleotideToExtend")]
    pub number_nucleotide_to_extend: i32,

    #[serde(rename = "chrLengths")]
    pub chr_lengths: HashMap<String, i32>,

    pub y: bool,
}

impl ReferenceResource {
    /// Ported from: ReferenceResource.java:L23-L27
    pub fn new(
        fasta: impl Into<String>,
        reference_extension: i32,
        number_nucleotide_to_extend: i32,
        chr_lengths: HashMap<String, i32>,
        y: bool,
    ) -> Self {
        Self {
            fasta: fasta.into(),
            reference_extension,
            number_nucleotide_to_extend,
            chr_lengths,
            y,
        }
    }

    /// Ported from: ReferenceResource.fetchFasta() L29-L40
    fn with_fasta_reader<T, F>(&self, file: &str, operation: F) -> Result<T, ReferenceError>
    where
        F: FnOnce(&faidx::Reader) -> Result<T, ReferenceError>,
    {
        THREAD_LOCAL_FASTA_FILES.with(|thread_local_files| {
            let mut thread_local_files = thread_local_files.borrow_mut();
            if !thread_local_files.contains_key(file) {
                let reader = faidx::Reader::from_path(file)
                    .map_err(|err| ReferenceError::open_reference(file, err))?;
                thread_local_files.insert(file.to_string(), reader);
            }

            let reader = thread_local_files
                .get(file)
                .expect("thread local fasta reader must exist after insertion");
            operation(reader)
        })
    }

    /// Ported from: ReferenceResource.retrieveSubSeq() L50-L65
    pub fn retrieve_sub_seq(
        &self,
        fasta: &str,
        chr: &str,
        start: i32,
        end: i32,
    ) -> Result<[String; 2], ReferenceError> {
        if start < 1 || end < start {
            return Err(ReferenceError::region_boundaries(
                chr,
                start,
                end,
                "Malformed query".to_string(),
            ));
        }

        self.with_fasta_reader(fasta, |reader| {
            let begin =
                usize::try_from(start - 1).expect("start must be positive after validation");
            let end_offset =
                usize::try_from(end - 1).expect("end must be positive after validation");
            reader
                .fetch_seq_string(chr, begin, end_offset)
                .map(|bases| [format!(">{}:{}-{}", chr, start, end), bases])
                .map_err(|err| {
                    let details = err.to_string();
                    if UNABLE_FIND_CONTIG.is_match(&details) || details.contains("FaidxBadSeqName")
                    {
                        ReferenceError::wrong_fasta_or_bam(chr, details)
                    } else if WRONG_START_OR_END.is_match(&details) {
                        ReferenceError::region_boundaries(chr, start, end, details)
                    } else {
                        ReferenceError::Htslib(details)
                    }
                })
        })
    }

    /// Ported from: ReferenceResource.getReference() L72-L75
    pub fn get_reference(&self, region: &Region) -> Result<Reference, ReferenceError> {
        let reference = Reference::new();
        self.get_reference_with_extension(region, self.reference_extension, reference)
    }

    /// Ported from: ReferenceResource.getReference() L84-L132
    pub fn get_reference_with_extension(
        &self,
        region: &Region,
        extension: i32,
        mut reference: Reference,
    ) -> Result<Reference, ReferenceError> {
        let sequence_start = if region.start - self.number_nucleotide_to_extend - extension < 1 {
            1
        } else {
            region.start - self.number_nucleotide_to_extend - extension
        };
        let len = self.chr_lengths.get(&region.chr).copied().unwrap_or(0);
        let sequence_end = if region.end + self.number_nucleotide_to_extend + extension > len {
            len
        } else {
            region.end + self.number_nucleotide_to_extend + extension
        };

        if self.y {
            eprintln!("TIME: Getting REF: {:?}", SystemTime::now());
        }

        let [_header, exon_sequence] =
            self.retrieve_sub_seq(&self.fasta, &region.chr, sequence_start, sequence_end)?;
        let mut exon = exon_sequence.into_bytes();
        exon.make_ascii_uppercase();

        if Self::is_loaded(&region.chr, sequence_start, sequence_end, &reference) {
            return Ok(reference);
        }

        let exon_length = i32::try_from(exon.len()).expect("reference exon length exceeds i32");
        let site_end = if len == sequence_end {
            exon_length
        } else {
            exon_length - SEED_1
        };
        reference.loaded_regions.push(LoadedRegion::new(
            region.chr.clone(),
            sequence_start,
            sequence_end,
        ));

        // Pre-size the seed map to this window's seed count (2 seeds per base: SEED_1 + SEED_2),
        // eliminating the FxHashMap rehash storm during per-region construction. Use `reserve`
        // (additional capacity), not `with_capacity`, because this may extend an already-loaded
        // `Reference` on the realign/SV path.
        reference.seed.reserve(2usize * (site_end.max(0) as usize));

        for i in 0..site_end.max(0) {
            let position = i + sequence_start;
            if reference.reference_sequences.contains_key(&position) {
                continue;
            }

            let exon_index = usize::try_from(i).expect("negative exon index is invalid");
            reference
                .reference_sequences
                .insert(position, exon[exon_index]);

            if len == sequence_end && i > exon_length - SEED_1 {
                continue;
            }

            let seed_1_end = exon_index + SEED_1 as usize;
            // Safety: `exon` was produced from the reference FASTA and made ASCII uppercase
            // by `exon.make_ascii_uppercase()`. All reference bases (A/C/G/T/N) are
            // single-byte ASCII (≤0x7F) and thus valid UTF-8.
            let key_sequence =
                unsafe { std::str::from_utf8_unchecked(&exon[exon_index..seed_1_end]) };
            Self::add_positions_to_seed_sequence(&mut reference, sequence_start, i, key_sequence);

            let seed_2_end = exon_index + SEED_2 as usize;
            // Safety: same as above.
            let key_sequence =
                unsafe { std::str::from_utf8_unchecked(&exon[exon_index..seed_2_end]) };
            Self::add_positions_to_seed_sequence(&mut reference, sequence_start, i, key_sequence);
        }

        if self.y {
            eprintln!("TIME: Got REF: {:?}", SystemTime::now());
        }

        Ok(reference)
    }

    /// Ported from: ReferenceResource.addPositionsToSeedSequence() L142-L147
    fn add_positions_to_seed_sequence(
        reference: &mut Reference,
        sequence_start: i32,
        i: i32,
        key_sequence: &str,
    ) {
        let position = i + sequence_start;
        if let Some(seed_positions) = reference.seed.get_mut(key_sequence) {
            seed_positions.push(position);
        } else {
            reference.seed.insert(key_sequence, smallvec![position]);
        }
    }

    /// Ported from: ReferenceResource.isLoaded() L157-L167
    pub fn is_loaded(
        chr: &str,
        sequence_start: i32,
        sequence_end: i32,
        reference: &Reference,
    ) -> bool {
        if reference.loaded_regions.is_empty() {
            return false;
        }

        for region in &reference.loaded_regions {
            if chr == region.chr
                && sequence_start >= region.sequence_start
                && sequence_end <= region.sequence_end
            {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_loaded_returns_false_for_empty_reference() {
        let reference = Reference::new();
        assert!(!ReferenceResource::is_loaded("20", 10, 20, &reference));
    }

    #[test]
    fn is_loaded_returns_true_for_contained_region() {
        let mut reference = Reference::new();
        reference
            .loaded_regions
            .push(LoadedRegion::new("20", 10, 30));

        assert!(ReferenceResource::is_loaded("20", 12, 28, &reference));
    }

    #[test]
    fn seed_positions_append_in_genomic_order() {
        let mut reference = Reference::new();

        ReferenceResource::add_positions_to_seed_sequence(&mut reference, 100, 0, "ACGT");
        ReferenceResource::add_positions_to_seed_sequence(&mut reference, 100, 2, "ACGT");

        assert_eq!(reference.seed["ACGT"].as_slice(), &[100, 102]);
    }

    #[test]
    fn seed_map_serializes_with_string_keys() {
        let mut reference = Reference::new();
        ReferenceResource::add_positions_to_seed_sequence(&mut reference, 100, 0, "ACGT");

        let json = serde_json::to_string(&reference.seed).unwrap();
        assert!(json.contains("\"ACGT\""));

        let roundtrip: ReferenceSeedMap = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip["ACGT"].as_slice(), &[100]);
    }
}
