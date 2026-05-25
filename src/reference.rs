use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::time::SystemTime;

use rust_htslib::faidx;
use rustc_hash::FxHashMap;

use crate::data::Region;
use crate::patterns::{UNABLE_FIND_CONTIG, WRONG_START_OR_END};

// Java: Configuration.SEED_1 L203
const SEED_1: i32 = 17;
// Java: Configuration.SEED_2 L208
const SEED_2: i32 = 12;

thread_local! {
    static THREAD_LOCAL_FASTA_FILES: RefCell<HashMap<String, faidx::Reader>> = RefCell::new(HashMap::new());
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
pub type ReferenceSequenceMap = FxHashMap<i32, u8>;
pub type ReferenceSeedMap = FxHashMap<String, Vec<i32>>;

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
            let key_sequence = std::str::from_utf8(&exon[exon_index..seed_1_end])
                .expect("reference seed must remain ASCII")
                .to_owned();
            Self::add_positions_to_seed_sequence(&mut reference, sequence_start, i, key_sequence);

            let seed_2_end = exon_index + SEED_2 as usize;
            let key_sequence = std::str::from_utf8(&exon[exon_index..seed_2_end])
                .expect("reference seed must remain ASCII")
                .to_owned();
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
        key_sequence: String,
    ) {
        let seed_positions = reference
            .seed
            .entry(key_sequence)
            .or_insert_with(|| Vec::with_capacity(1));
        seed_positions.push(i + sequence_start);
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
}
