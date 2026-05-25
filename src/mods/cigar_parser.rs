/// Ported from: CigarParser.java:L1-L2413
/// Core variant detection engine: iterates BAM records, parses CIGAR strings
/// operation-by-operation, populates variant maps, coverage, soft-clip structures,
/// and structural variant accumulators.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rust_htslib::bam::{self, HeaderView, record::Aux};

use crate::config::{Configuration, MINMAPBASE, MINSVCDIST, MINSVPOS, SEED_1};
use crate::data::{
    BaseInsertion, Mate, ModifiedCigar, PositionMap, Region, SVStructures, Sclip, SortedStringMap,
    Variation, VariationData, VariationMap,
};
use crate::mods::cigar_modifier::modify_cigar;
use crate::mods::sam_file_parser::{RecordPreprocessor, get_mate_reference_name};
use crate::patterns::*;
use crate::reference::{Reference, ReferenceSequenceMap};
use crate::scope::GlobalReadOnlyScope;
use crate::utils::{get_reverse_complemented_sequence, global_find, substr_with_len};
use crate::variations::{
    get_variation, get_variation_from_seq, inc_cnt, inc_cnt_sorted_string_map, is_equals,
    is_has_and_equals_base, is_has_and_equals_str, is_has_and_not_equals_base, is_not_equals,
    is_reference_mismatch_and_not_n_at,
};

// ─── CIGAR representation ─────────────────────────────────────────────────────

/// CIGAR operator types matching htsjdk CigarOperator
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CigarOp {
    M,  // Match/mismatch – consumes both query and reference
    I,  // Insertion to reference – consumes query only
    D,  // Deletion from reference – consumes reference only
    N,  // Skipped region from reference – consumes reference only
    S,  // Soft clip – consumes query only
    H,  // Hard clip – consumes neither
    P,  // Padding – consumes neither
    Eq, // Sequence match (=) – consumes both
    X,  // Sequence mismatch – consumes both
}

impl CigarOp {
    /// Java: CigarOperator.consumesReadBases()
    pub fn consumes_read_bases(self) -> bool {
        matches!(self, Self::M | Self::I | Self::S | Self::Eq | Self::X)
    }

    /// Java: CigarOperator.consumesReferenceBases()
    pub fn consumes_reference_bases(self) -> bool {
        matches!(self, Self::M | Self::D | Self::N | Self::Eq | Self::X)
    }

    /// Returns true if the operator consumes both read and reference bases.
    pub fn consumes_both(self) -> bool {
        self.consumes_read_bases() && self.consumes_reference_bases()
    }
}

/// A single CIGAR element: length + operator
#[derive(Clone, Debug)]
pub struct CigarElement {
    pub length: i32,
    pub operator: CigarOp,
}

impl CigarElement {
    pub fn new(length: i32, operator: CigarOp) -> Self {
        Self { length, operator }
    }
}

/// Parsed CIGAR string, equivalent to htsjdk Cigar class
#[derive(Clone, Debug)]
pub struct ParsedCigar {
    pub elements: Vec<CigarElement>,
}

impl ParsedCigar {
    pub fn new(elements: Vec<CigarElement>) -> Self {
        Self { elements }
    }

    pub fn num_cigar_elements(&self) -> usize {
        self.elements.len()
    }

    pub fn get_cigar_element(&self, index: usize) -> &CigarElement {
        &self.elements[index]
    }

    /// Java: Cigar.getReferenceLength()
    pub fn get_reference_length(&self) -> i32 {
        self.elements
            .iter()
            .filter(|e| e.operator.consumes_reference_bases())
            .map(|e| e.length)
            .sum()
    }
}

impl Default for ParsedCigar {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
        }
    }
}

impl std::fmt::Display for ParsedCigar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for elem in &self.elements {
            let op_char = match elem.operator {
                CigarOp::M => 'M',
                CigarOp::I => 'I',
                CigarOp::D => 'D',
                CigarOp::N => 'N',
                CigarOp::S => 'S',
                CigarOp::H => 'H',
                CigarOp::P => 'P',
                CigarOp::Eq => '=',
                CigarOp::X => 'X',
            };
            write!(f, "{}{}", elem.length, op_char)?;
        }
        Ok(())
    }
}

/// Parse a CIGAR string like "10M5I3D" into a ParsedCigar
pub fn parse_cigar_string(cigar_str: &str) -> ParsedCigar {
    let mut elements = Vec::new();
    let mut num_start = 0;
    for (i, c) in cigar_str.char_indices() {
        if c.is_ascii_alphabetic() || c == '=' {
            if i > num_start {
                let length: i32 = cigar_str[num_start..i].parse().unwrap_or(0);
                let operator = match c {
                    'M' => CigarOp::M,
                    'I' => CigarOp::I,
                    'D' => CigarOp::D,
                    'N' => CigarOp::N,
                    'S' => CigarOp::S,
                    'H' => CigarOp::H,
                    'P' => CigarOp::P,
                    '=' => CigarOp::Eq,
                    'X' => CigarOp::X,
                    _ => {
                        num_start = i + 1;
                        continue;
                    }
                };
                elements.push(CigarElement::new(length, operator));
            }
            num_start = i + 1;
        }
    }
    ParsedCigar { elements }
}

/// Convert a rust_htslib CigarStringView to our ParsedCigar.
pub fn cigar_from_record(record: &bam::Record) -> ParsedCigar {
    let cigar = record.cigar();
    let mut elements = Vec::new();
    for c in cigar.iter() {
        let (length, operator) = match c {
            rust_htslib::bam::record::Cigar::Match(len) => (*len as i32, CigarOp::M),
            rust_htslib::bam::record::Cigar::Ins(len) => (*len as i32, CigarOp::I),
            rust_htslib::bam::record::Cigar::Del(len) => (*len as i32, CigarOp::D),
            rust_htslib::bam::record::Cigar::RefSkip(len) => (*len as i32, CigarOp::N),
            rust_htslib::bam::record::Cigar::SoftClip(len) => (*len as i32, CigarOp::S),
            rust_htslib::bam::record::Cigar::HardClip(len) => (*len as i32, CigarOp::H),
            rust_htslib::bam::record::Cigar::Pad(len) => (*len as i32, CigarOp::P),
            rust_htslib::bam::record::Cigar::Equal(len) => (*len as i32, CigarOp::Eq),
            rust_htslib::bam::record::Cigar::Diff(len) => (*len as i32, CigarOp::X),
        };
        elements.push(CigarElement::new(length, operator));
    }
    ParsedCigar { elements }
}

// ─── Offset struct ────────────────────────────────────────────────────────────

/// Ported from: CigarParser.java:L2401-L2413 (inner class Offset)
/// Result of findOffset(): carries offset position, extracted sequence/quality,
/// and count of mismatches found.
pub struct Offset {
    pub offset: i32,
    pub sequence: String,
    pub quality_sequence: String,
    pub offset_number_of_mismatches: i32,
}

impl Offset {
    pub fn new(
        offset: i32,
        sequence: impl Into<String>,
        quality_sequence: impl Into<String>,
        offset_number_of_mismatches: i32,
    ) -> Self {
        Self {
            offset,
            sequence: sequence.into(),
            quality_sequence: quality_sequence.into(),
            offset_number_of_mismatches,
        }
    }
}

// ─── CigarParser struct ──────────────────────────────────────────────────────

/// Ported from: CigarParser.java:L30-L60 (instance fields)
/// Core CIGAR parsing state machine. Holds all output maps, scope references,
/// and per-record mutable state for the CIGAR element loop.
pub struct CigarParser {
    // ── Output maps (populated across all records) ──
    pub non_insertion_variants: PositionMap<VariationMap>,
    pub insertion_variants: PositionMap<VariationMap>,
    pub ref_coverage: PositionMap<i32>,
    pub soft_clips_3_end: HashMap<i32, Sclip>,
    pub soft_clips_5_end: HashMap<i32, Sclip>,
    pub position_to_insertion_count: PositionMap<SortedStringMap<i32>>,
    pub mnp: PositionMap<SortedStringMap<i32>>,
    pub position_to_deletion_count: PositionMap<SortedStringMap<i32>>,
    pub splice_count: SortedStringMap<Vec<i32>>,
    pub sv_structures: SVStructures,

    // ── Scope copies (set in init_from_scope) ──
    pub region: Region,
    pub splice: HashSet<String>,
    pub reference: Arc<Reference>,
    pub max_read_length: i32,

    // ── Per-record state (set/reset in parse_cigar) ──
    pub cigar: ParsedCigar,
    pub start: i32,
    pub total_reads: i32,
    pub duplicate_reads: i32,
    pub discordant_count: i32,
    pub svflag: bool,
    pub snapshot_cigar_modifier: bool,
    pub cigar_modifier_snapshots: Vec<ModifiedCigar>,
    pub offset: i32,
    pub cigar_element_length: i32,
    pub read_position_including_soft_clipped: i32,
    pub read_position_excluding_soft_clipped: i32,
}

impl CigarParser {
    fn with_conf<R>(f: impl FnOnce(&Configuration) -> R) -> R {
        GlobalReadOnlyScope::with_instance(|scope| f(&scope.conf))
    }

    // ── Constructor ───────────────────────────────────────────────────────

    /// Ported from: CigarParser.java:L64-L71 (constructor)
    /// Creates a new CigarParser with empty maps and the given svflag.
    pub fn new(svflag: bool) -> Self {
        Self {
            // Output maps — initialized empty
            non_insertion_variants: PositionMap::default(),
            insertion_variants: PositionMap::default(),
            ref_coverage: PositionMap::default(),
            soft_clips_3_end: HashMap::new(),
            soft_clips_5_end: HashMap::new(),
            position_to_insertion_count: PositionMap::default(),
            mnp: PositionMap::default(),
            position_to_deletion_count: PositionMap::default(),
            splice_count: SortedStringMap::new(),
            sv_structures: SVStructures::default(),

            // Scope copies — placeholder defaults, set by init_from_scope
            region: Region::new("", 0, 0, ""),
            splice: HashSet::new(),
            reference: Arc::new(Reference::default()),
            max_read_length: 0,

            // Per-record state — zeroed
            cigar: ParsedCigar::default(),
            start: 0,
            total_reads: 0,
            duplicate_reads: 0,
            discordant_count: 0,
            svflag,
            snapshot_cigar_modifier: std::env::var("VARDICT_PARITY_CIGAR_MODIFIER")
                .map_or(false, |value| !value.is_empty()),
            cigar_modifier_snapshots: Vec::new(),
            offset: 0,
            cigar_element_length: 0,
            read_position_including_soft_clipped: 0,
            read_position_excluding_soft_clipped: 0,
        }
    }

    // ── Scope initialization ──────────────────────────────────────────────

    /// Ported from: CigarParser.java:L173-L185 (initFromScope)
    /// Copies fields from the pipeline scope into instance variables.
    /// In Java, insertionVariants/nonInsertionVariants/refCoverage/softClips are
    /// aliased from RecordPreprocessor's initial data.
    pub fn init_from_scope(
        &mut self,
        region: &Region,
        reference: &Arc<Reference>,
        splice: &HashSet<String>,
        max_read_length: i32,
        // Fields from RecordPreprocessor / InitialData:
        non_insertion_variants: PositionMap<VariationMap>,
        insertion_variants: PositionMap<VariationMap>,
        ref_coverage: PositionMap<i32>,
        soft_clips_3_end: HashMap<i32, Sclip>,
        soft_clips_5_end: HashMap<i32, Sclip>,
        total_reads: i32,
        duplicate_reads: i32,
    ) {
        // Java: this.insertionVariants = scope.data.insertionVariants; etc.
        self.insertion_variants = insertion_variants;
        self.non_insertion_variants = non_insertion_variants;
        self.ref_coverage = ref_coverage;
        self.soft_clips_3_end = soft_clips_3_end;
        self.soft_clips_5_end = soft_clips_5_end;

        // Java: this.region = scope.region; etc.
        self.region = region.clone();
        self.splice = splice.clone();
        self.reference = Arc::clone(reference);
        self.max_read_length = max_read_length;
        self.duplicate_reads = duplicate_reads;
        self.total_reads = total_reads;
    }

    // ── Entry point ───────────────────────────────────────────────────────

    /// Ported from: CigarParser.java:L82-L171 (process)
    /// Entry point: iterates all BAM records from the preprocessor,
    /// dispatches each to parse_cigar(), then packages results into VariationData.
    ///
    /// Kept for tests and harnesses that materialize owned BAM records.
    pub fn process(
        &mut self,
        records: &mut dyn Iterator<Item = bam::Record>,
        header: &HeaderView,
        chr_name: &str,
    ) -> VariationData {
        let (sample, output_splicing, remove_duplicated_reads) = Self::process_output_settings();

        // Java: while ((record = processor.nextRecord()) != null) { ... }
        // Ported from: CigarParser.java:L96-L103
        for record in records {
            self.process_record(chr_name, &record, header);
        }

        self.finish_process(sample, output_splicing, remove_duplicated_reads)
    }

    /// Production entry point that processes the preprocessor's reusable record buffer directly.
    pub fn process_preprocessor(
        &mut self,
        preprocessor: &mut RecordPreprocessor,
        header: &HeaderView,
        chr_name: &str,
    ) -> VariationData {
        let (sample, output_splicing, remove_duplicated_reads) = Self::process_output_settings();

        preprocessor.for_each_record(|record| self.process_record(chr_name, record, header));

        self.finish_process(sample, output_splicing, remove_duplicated_reads)
    }

    fn process_output_settings() -> (String, bool, bool) {
        GlobalReadOnlyScope::with_instance(|scope| {
            (
                scope.sample.clone(),
                scope.conf.output_splicing,
                scope.conf.remove_duplicated_reads,
            )
        })
    }

    fn process_record(&mut self, chr_name: &str, record: &bam::Record, header: &HeaderView) {
        // Java: try { parseCigar(chrName, record); }
        //       catch (Exception e) { printExceptionAndContinue(e, ...); }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.parse_cigar(chr_name, record, header);
        }));

        if let Err(_e) = result {
            let read_name = String::from_utf8_lossy(record.qname()).to_string();
            eprintln!(
                "WARNING: Exception in CigarParser for record {} in region {}",
                read_name,
                self.region.print_region()
            );
        }
    }

    fn finish_process(
        &mut self,
        sample: String,
        output_splicing: bool,
        remove_duplicated_reads: bool,
    ) -> VariationData {
        // Java: CigarParser.java:L105-L118 (outputSplicing early return)
        if output_splicing {
            for (intron, cnt) in &self.splice_count {
                let count = cnt.first().copied().unwrap_or(0);
                println!("{}\t{}\t{}\t{}", sample, self.region.chr, intron, count);
            }
            return VariationData::default();
        }

        // ── Duprate computation ──
        // Ported from: CigarParser.java:L120-L128
        let duprate: f64 = if self.svflag {
            // T12: Integer division — (discordantCount + 1) / (totalReads - duplicateReads + 1)
            // Java uses integer division here; result compared > 0.5
            let numerator = self.discordant_count + 1;
            let denominator = self.total_reads - self.duplicate_reads + 1;
            if numerator / denominator > 0 {
                0.0
            } else {
                1.0
            }
        } else if remove_duplicated_reads && self.total_reads != 0 {
            // Java rounds the duplicate-rate ratio with `String.format("%.3f", ...)`
            // before parsing it back to a double.
            Self::round_duplicate_rate(self.duplicate_reads, self.total_reads)
        } else {
            0.0
        };

        // ── Package results into VariationData ──
        // Ported from: CigarParser.java:L130-L143
        VariationData {
            non_insertion_variants: std::mem::take(&mut self.non_insertion_variants),
            insertion_variants: std::mem::take(&mut self.insertion_variants),
            position_to_insertion_count: std::mem::take(&mut self.position_to_insertion_count),
            position_to_deletions_count: std::mem::take(&mut self.position_to_deletion_count),
            ref_coverage: std::mem::take(&mut self.ref_coverage),
            soft_clips_5_end: std::mem::take(&mut self.soft_clips_5_end),
            soft_clips_3_end: std::mem::take(&mut self.soft_clips_3_end),
            sv_structures: std::mem::take(&mut self.sv_structures),
            max_read_length: Some(self.max_read_length),
            duprate,
            splice: None, // splice is returned separately via scope, not in VariationData
            mnp: std::mem::take(&mut self.mnp),
            splice_count: std::mem::take(&mut self.splice_count),
        }
    }

    // ── CIGAR cleanup ─────────────────────────────────────────────────────

    /// Ported from: CigarParser.java:L1553-L1599 (cleanupCigar)
    /// Removes leading/trailing hard clips, converts leading/trailing insertions
    /// to soft clips. Modifies self.cigar in place.
    pub fn cleanup_cigar(&mut self) {
        let elems = &mut self.cigar.elements;
        if elems.is_empty() {
            return;
        }

        // First: leading elements — forward iterate until first consumes-both operator
        {
            let mut i = 0;
            let mut no_matches_yet = true;
            while i < elems.len() && no_matches_yet {
                let op = elems[i].operator;
                if op == CigarOp::I {
                    // Convert leading I to S
                    elems[i].operator = CigarOp::S;
                    i += 1;
                } else if op == CigarOp::H {
                    // Remove leading H
                    elems.remove(i);
                    // don't increment i — next element slides into position
                } else if op.consumes_read_bases() && op.consumes_reference_bases() {
                    no_matches_yet = false;
                } else {
                    i += 1;
                }
            }
        }

        // Then: trailing elements — reverse iterate same logic
        {
            if !elems.is_empty() {
                let mut i = elems.len();
                let mut no_matches_yet = true;
                while i > 0 && no_matches_yet {
                    i -= 1;
                    let op = elems[i].operator;
                    if op == CigarOp::I {
                        // Convert trailing I to S
                        elems[i].operator = CigarOp::S;
                    } else if op == CigarOp::H {
                        // Remove trailing H
                        elems.remove(i);
                    } else if op.consumes_read_bases() && op.consumes_reference_bases() {
                        no_matches_yet = false;
                    }
                }
            }
        }
    }

    // ── CIGAR operator dispatch ───────────────────────────────────────────

    /// Ported from: CigarParser.java:L1601-L1609 (getCigarOperator)
    /// Returns the CIGAR operator, treating edge insertions as soft clips.
    pub fn get_cigar_operator(&self, ci: usize) -> CigarOp {
        let operator = self.cigar.get_cigar_element(ci).operator;
        // Treat insertions at the edge as soft-clipping
        if (ci == 0 || ci == self.cigar.num_cigar_elements() - 1) && operator == CigarOp::I {
            return CigarOp::S;
        }
        operator
    }

    // ── CIGAR length helpers ──────────────────────────────────────────────

    /// Ported from: CigarParser.java:L1616-L1623 (getAlignedLength)
    /// Sum of M+D lengths — the aligned reference span.
    pub fn get_aligned_length(cigar: &ParsedCigar) -> i32 {
        cigar
            .elements
            .iter()
            .filter(|e| e.operator == CigarOp::M || e.operator == CigarOp::D)
            .map(|e| e.length)
            .sum()
    }

    /// Ported from: CigarParser.java:L1630-L1639 (getSoftClippedLength)
    /// Sum of M+I+S lengths — total length including soft clips.
    pub fn get_soft_clipped_length(cigar: &ParsedCigar) -> i32 {
        cigar
            .elements
            .iter()
            .filter(|e| {
                e.operator == CigarOp::M || e.operator == CigarOp::I || e.operator == CigarOp::S
            })
            .map(|e| e.length)
            .sum()
    }

    /// Ported from: CigarParser.java:L1645-L1653 (getMatchInsertionLength)
    /// Sum of M+I lengths — read length for matched bases.
    pub fn get_match_insertion_length(cigar: &ParsedCigar) -> i32 {
        cigar
            .elements
            .iter()
            .filter(|e| e.operator == CigarOp::M || e.operator == CigarOp::I)
            .map(|e| e.length)
            .sum()
    }

    /// Ported from: CigarParser.java:L1659-L1667 (getInsertionDeletionLength)
    /// Sum of I+D lengths.
    pub fn get_insertion_deletion_length(cigar: &ParsedCigar) -> i32 {
        cigar
            .elements
            .iter()
            .filter(|e| e.operator == CigarOp::I || e.operator == CigarOp::D)
            .map(|e| e.length)
            .sum()
    }

    // ── Base quality string ───────────────────────────────────────────────

    /// Ported from: CigarParser.java:L2383-L2399 (getBaseQualityString)
    /// Safe quality extraction: caps phred scores at MAX_PHRED_SCORE (93).
    pub fn get_base_quality_string(record: &bam::Record) -> String {
        let quals = record.qual();
        let max_phred: u8 = 93; // SAMUtils.MAX_PHRED_SCORE
        let mut result = Vec::with_capacity(quals.len());
        for &q in quals {
            let capped = if q > max_phred { max_phred } else { q };
            result.push(capped + 33);
        }
        String::from_utf8(result).expect("base qualities are capped to printable ASCII")
    }

    // ── Predicate helpers ─────────────────────────────────────────────────

    /// Ported from: CigarParser.java:L886-L888 (isNextAfterNumMatched)
    /// Checks if the CIGAR element at ci + number is an M operator.
    pub fn is_next_after_num_matched(&self, ci: usize, number: usize) -> bool {
        self.cigar.num_cigar_elements() > ci + number
            && self.cigar.get_cigar_element(ci + number).operator == CigarOp::M
    }

    /// Ported from: CigarParser.java:L903-L905 (isTwoInsertionsAhead)
    /// Checks if the element at ci+1 is an insertion.
    pub fn is_two_insertions_ahead(&self, ci: usize) -> bool {
        self.cigar.num_cigar_elements() > ci + 1
            && self.cigar.get_cigar_element(ci + 1).operator == CigarOp::I
    }

    /// Ported from: CigarParser.java:L907-L910 (isNextInsertion)
    /// Checks if the next CIGAR element is an insertion (requires performLocalRealignment).
    pub fn is_next_insertion(&self, ci: usize) -> bool {
        let perform_local_realignment =
            GlobalReadOnlyScope::with_instance(|scope| scope.conf.perform_local_realignment);
        perform_local_realignment
            && self.cigar.num_cigar_elements() > ci + 1
            && self.cigar.get_cigar_element(ci + 1).operator == CigarOp::I
    }

    /// Ported from: CigarParser.java:L911-L914 (isNextMatched)
    /// Checks if the next CIGAR element is a match (requires performLocalRealignment).
    pub fn is_next_matched(&self, ci: usize) -> bool {
        let perform_local_realignment =
            GlobalReadOnlyScope::with_instance(|scope| scope.conf.perform_local_realignment);
        perform_local_realignment
            && self.cigar.num_cigar_elements() > ci + 1
            && self.cigar.get_cigar_element(ci + 1).operator == CigarOp::M
    }

    /// Ported from: CigarParser.java:L893-L901 (isInsertionOrDeletionWithNextMatched)
    /// Checks for D/I M D/I pattern (multi-indel within VEXT).
    /// Note: accesses ci+3 — caller must ensure CIGAR is long enough.
    pub fn is_insertion_or_deletion_with_next_matched(&self, ci: usize) -> bool {
        let (perform_local_realignment, vext) = GlobalReadOnlyScope::with_instance(|scope| {
            (scope.conf.perform_local_realignment, scope.conf.vext)
        });
        let num_elems = self.cigar.num_cigar_elements();
        perform_local_realignment
            && num_elems > ci + 2
            && self.cigar.get_cigar_element(ci + 1).length <= vext
            && self.cigar.get_cigar_element(ci + 1).operator == CigarOp::M
            && (self.cigar.get_cigar_element(ci + 2).operator == CigarOp::I
                || self.cigar.get_cigar_element(ci + 2).operator == CigarOp::D)
            // Guard ci+3 access — Java accesses without bounds check but catches exceptions
            && num_elems > ci + 3
            && self.cigar.get_cigar_element(ci + 3).operator != CigarOp::I
            && self.cigar.get_cigar_element(ci + 3).operator != CigarOp::D
    }

    /// Ported from: CigarParser.java:L917-L930 (isCloserThenVextAndGoodBase)
    /// Predicate: near end of M segment with adjacent I/D and good quality base.
    pub fn is_closer_then_vext_and_good_base(
        &self,
        perform_local_realignment: bool,
        vext: i32,
        goodq: f64,
        query_sequence: &[u8],
        ref_map: &ReferenceSequenceMap,
        query_quality: &[u8],
        ci: usize,
        i: i32,
        ss_is_empty: bool,
        target_op: CigarOp,
    ) -> bool {
        let num_elems = self.cigar.num_cigar_elements();
        // Do not adjust complex if we have hard-clips after insertion/deletion
        if num_elems > ci + 2 && self.cigar.get_cigar_element(ci + 2).operator == CigarOp::H {
            return false;
        }
        let n = self.read_position_including_soft_clipped as usize;
        if !(perform_local_realignment
            && self.cigar_element_length - i <= vext
            && num_elems > ci + 1
            && self.cigar.get_cigar_element(ci + 1).operator == target_op)
        {
            return false;
        }
        let Some(&reference_base) = ref_map.get(&self.start) else {
            return false;
        };
        (!ss_is_empty || is_not_equals(query_sequence.get(n).copied(), Some(reference_base)))
            && n < query_quality.len()
            && (query_quality[n] as i32 - 33) as f64 >= goodq
    }

    /// Ported from: CigarParser.java:L655-L663 (isTrimAtOptTBases)
    /// Flag: trim reads at opt_T bases from start or end (depending on direction).
    pub fn is_trim_at_opt_t_bases(
        &self,
        direction: bool,
        total_length_including_soft_clipped: i32,
    ) -> bool {
        let trim_bases_after =
            GlobalReadOnlyScope::with_instance(|scope| scope.conf.trim_bases_after);
        if trim_bases_after != 0 {
            if !direction {
                return self.read_position_including_soft_clipped > trim_bases_after;
            } else {
                return total_length_including_soft_clipped
                    - self.read_position_including_soft_clipped
                    > trim_bases_after;
            }
        }
        false
    }

    /// Ported from: CigarParser.java:L665-L677 (skipSitesOutRegionOfInterest)
    /// CRISPR mode: skip reads outside the region of interest.
    pub fn skip_sites_out_region_of_interest(&self) -> bool {
        let (cut_site, filter_bp) = GlobalReadOnlyScope::with_instance(|scope| {
            (
                scope.conf.crispr_cutting_site,
                scope.conf.crispr_filtering_bp,
            )
        });
        if cut_site != 0 {
            // The total aligned length, excluding soft-clipped bases and insertions
            let cigar_string = self.cigar.to_string();
            let rlen3: i32 = global_find(&ALIGNED_LENGTH_MD, &cigar_string)
                .iter()
                .filter_map(|s| s.parse::<i32>().ok())
                .sum();
            if filter_bp != 0 {
                return !(cut_site - self.start > filter_bp
                    && self.start + rlen3 - cut_site > filter_bp);
            }
        }
        false
    }

    /// Ported from: CigarParser.java:L1694-L1700 (skipIndelNextToIntron)
    /// Skip insertions/deletions right after or before introns (N operator).
    pub fn skip_indel_next_to_intron(&self, ci: usize) -> bool {
        let num_elems = self.cigar.num_cigar_elements();
        if (num_elems > ci + 1 && self.cigar.get_cigar_element(ci + 1).operator == CigarOp::N)
            || (ci > 0 && self.cigar.get_cigar_element(ci - 1).operator == CigarOp::N)
        {
            return true;
        }
        false
    }

    /// Ported from: CigarParser.java:L1462-L1477 (isBEGIN_ATGC_AMP_ATGCs_END)
    /// Checks if a string matches the MNV pattern: single ATGC + '&' + one or more ATGC.
    pub fn is_begin_atgc_amp_atgcs_end(sequence: &str) -> bool {
        let bytes = sequence.as_bytes();
        if bytes.len() > 2 {
            let first = bytes[0];
            let second = bytes[1];
            if second == b'&' && Self::is_atgc(first) {
                for &b in &bytes[2..] {
                    if !Self::is_atgc(b) {
                        return false;
                    }
                }
                return true;
            }
        }
        false
    }

    /// Ported from: CigarParser.java:L1479-L1494 (isATGC)
    /// Checks if a byte is one of A, T, G, C.
    pub fn is_atgc(ch: u8) -> bool {
        matches!(ch, b'A' | b'T' | b'G' | b'C')
    }

    fn single_base_description(ch: u8) -> Option<&'static str> {
        match ch {
            b'A' => Some("A"),
            b'T' => Some("T"),
            b'G' => Some("G"),
            b'C' => Some("C"),
            _ => None,
        }
    }

    fn matching_description_mut(description: &mut Option<String>, ch: u8) -> &mut String {
        description.get_or_insert_with(|| {
            let mut owned = String::new();
            owned.push(ch as char);
            owned
        })
    }

    // ── Counter helpers ───────────────────────────────────────────────────

    /// Ported from: CigarParser.java:L1451-L1460 (increment)
    /// Increments a count in a nested Map<Integer, Map<String, Integer>>.
    pub fn increment(
        counters: &mut PositionMap<SortedStringMap<i32>>,
        index: i32,
        description_string: &str,
    ) {
        let map = counters.entry(index).or_default();
        inc_cnt(map, description_string.to_string(), 1);
    }

    // ── N operator (Not Matched / Splice) ─────────────────────────────────

    /// Ported from: CigarParser.java:L1307-L1320 (processNotMatched)
    /// Handles N operator: skip region and add to splice junctions.
    pub fn process_not_matched(&mut self) {
        // Java: String key = (start - 1) + "-" + (start + cigarElementLength - 1);
        let key = format!(
            "{}-{}",
            self.start - 1,
            self.start + self.cigar_element_length - 1
        );
        self.splice.insert(key.clone());

        let cnt = self.splice_count.entry(key).or_insert_with(|| vec![0]);
        cnt[0] += 1;

        self.start += self.cigar_element_length;
        self.offset = 0;
    }

    // ── Stubbed methods (todo!()) ─────────────────────────────────────────

    /// Helper: extract i32 from a rust_htslib Aux value.
    fn aux_to_i32(aux: &Aux) -> Option<i32> {
        match aux {
            Aux::I8(v) => Some(*v as i32),
            Aux::U8(v) => Some(*v as i32),
            Aux::I16(v) => Some(*v as i32),
            Aux::U16(v) => Some(*v as i32),
            Aux::I32(v) => Some(*v),
            Aux::U32(v) => Some(*v as i32),
            _ => None,
        }
    }

    /// Mirrors Java's `Double.parseDouble(String.format("%.3f", duplicateReads / totalReads))`
    /// for the non-negative duplicate-rate ratio used by CigarParser.
    fn round_duplicate_rate(duplicate_reads: i32, total_reads: i32) -> f64 {
        if total_reads <= 0 {
            return 0.0;
        }

        let numerator = i64::from(duplicate_reads.max(0));
        let denominator = i64::from(total_reads);
        let scaled = (numerator * 2_000 + denominator) / (2 * denominator);
        scaled as f64 / 1_000.0
    }

    /// Ported from: CigarParser.java:L224-L653 (parseCigar)
    /// Main per-record CIGAR parsing loop. Extracts variants from a single BAM record.
    #[allow(clippy::too_many_lines)]
    fn parse_cigar(&mut self, chr_name: &str, record: &bam::Record, header: &HeaderView) {
        let (
            mismatch,
            perform_local_realignment,
            minmatch,
            samfilter_is_nonzero,
            disable_sv,
            include_n_in_total_depth,
            goodq,
            vext,
            has_amplicon_based_calling,
        ) = GlobalReadOnlyScope::with_instance(|instance| {
            let conf = &instance.conf;
            (
                conf.mismatch,
                conf.perform_local_realignment,
                conf.minmatch,
                conf.samfilter != "0",
                conf.disable_sv,
                conf.include_n_in_total_depth,
                conf.goodq,
                conf.vext,
                instance.amplicon_based_calling.is_some(),
            )
        });

        // Java: CigarParser.java#L225-L228
        let mut query_sequence = String::from_utf8(record.seq().as_bytes()).unwrap_or_default();
        let mapping_quality = record.mapq() as i32;
        self.cigar = cigar_from_record(record);

        // Java: CigarParser.java#L231
        let insertion_deletion_length = Self::get_insertion_deletion_length(&self.cigar);

        // Java: CigarParser.java#L234-L266 — NM tag mismatch filter
        let mut total_number_of_mismatches: i32 = 0;
        let nm_tag: Option<i32> = record.aux(b"NM").ok().and_then(|a| Self::aux_to_i32(&a));
        let nm_star: Option<i32> = record.aux(b"nM").ok().and_then(|a| Self::aux_to_i32(&a));
        // Java: if (numberOfMismatches_STAR != null) numberOfMismatches_NM = numberOfMismatches_STAR;
        let number_of_mismatches_nm = nm_star.or(nm_tag);

        if let Some(nm_val) = number_of_mismatches_nm {
            // Java: CigarParser.java#L247
            total_number_of_mismatches = nm_val - insertion_deletion_length;
            if total_number_of_mismatches > mismatch {
                return;
            }
        } else {
            // Java: CigarParser.java#L254-L258 — skip unmapped or * cigar
            let cigar_str = self.cigar.to_string();
            if record.is_unmapped() || cigar_str == "*" || self.cigar.num_cigar_elements() == 0 {
                return;
            }
        }

        // Java: CigarParser.java#L260
        let mut query_quality = Self::get_base_quality_string(record);
        // Java: CigarParser.java#L261
        let is_mate_reference_name_equal = record.tid() == record.mtid();
        // Java: CigarParser.java#L262
        let number_of_mismatches = total_number_of_mismatches;
        // Java: CigarParser.java#L263
        let direction = record.is_reverse();

        // Java: CigarParser.java#L265-L269 — amplicon mode
        if has_amplicon_based_calling
            && self.parse_cigar_with_amp_case(record, header, is_mate_reference_name_equal)
        {
            return;
        }

        // Java: CigarParser.java#L271-L272
        self.read_position_including_soft_clipped = 0;
        self.read_position_excluding_soft_clipped = 0;

        // Java: CigarParser.java#L274-L290 — CigarModifier or raw position
        let position: i32;
        if perform_local_realignment {
            // Java: CigarParser.java#L278-L290
            let record_cigar_str = self.cigar.to_string();
            let modified = modify_cigar(
                record.pos() as i32 + 1, // Java getAlignmentStart() is 1-based
                &record_cigar_str,
                &query_sequence,
                &query_quality,
                &self.reference,
                insertion_deletion_length,
                &self.region,
                self.max_read_length,
            );
            if self.snapshot_cigar_modifier {
                self.cigar_modifier_snapshots.push(modified.clone());
            }
            position = modified.position;
            self.cigar = parse_cigar_string(&modified.cigar);
            query_sequence = modified.query_sequence;
            query_quality = modified.query_quality;
        } else {
            // Java: CigarParser.java#L291-L292
            position = record.pos() as i32 + 1; // 0-based to 1-based
            self.cigar = cigar_from_record(record);
        }

        // Java caches `cigar` before cleanup and keeps parsing against that cached object.
        // `cleanupCigar(record)` only rewrites the SAMRecord's cigar, so the parser's working
        // cigar must stay untouched here.
        // Java: CigarParser.java#L297
        self.start = position;
        self.offset = 0;

        // Java: CigarParser.java#L300-L302 — discordant count
        if get_mate_reference_name(record, header) != "=" {
            self.discordant_count += 1;
        }

        // Java: CigarParser.java#L305-L306 — double soft-clip filter
        let cigar_string = self.cigar.to_string();
        if BEGIN_dig_dig_S_ANY_dig_dig_S_END.is_match(&cigar_string) {
            return;
        }

        // Java: CigarParser.java#L309-L312
        let read_length_include_matching_and_insertions =
            Self::get_match_insertion_length(&self.cigar);
        if minmatch != 0 && read_length_include_matching_and_insertions < minmatch {
            return;
        }

        // Java: CigarParser.java#L315-L317
        let total_length_including_soft_clipped = Self::get_soft_clipped_length(&self.cigar);
        if total_length_including_soft_clipped > self.max_read_length {
            self.max_read_length = total_length_including_soft_clipped;
        }

        // Java: CigarParser.java#L320-L322 — supplementary alignment filter
        if samfilter_is_nonzero && record.is_supplementary() {
            return;
        }

        // Java: CigarParser.java#L325
        if self.skip_sites_out_region_of_interest() {
            return;
        }

        // Java: CigarParser.java#L328 — Mate direction (INVERTED logic! T-note in brief)
        let mate_direction = (record.flags() & 0x20) == 0;

        // Java: CigarParser.java#L330-L335 — SV prep
        if record.is_paired() && record.is_mate_unmapped() {
            // To be implemented (no-op in Java too)
        } else if mapping_quality > 10 && !disable_sv {
            self.prepare_sv_structures_for_analysis(
                record,
                header,
                &query_quality,
                number_of_mismatches,
                direction,
                mate_direction,
                position,
                total_length_including_soft_clipped,
            );
        }

        // Java: CigarParser.java#L336
        let mate_alignment_start = record.mpos() as i32 + 1; // 0-based to 1-based

        // Take a clone of reference sequences to avoid borrow conflicts
        let reference = Arc::clone(&self.reference);
        let ref_map = &reference.reference_sequences;
        let num_cigar_elements = self.cigar.num_cigar_elements();

        // Java: CigarParser.java#L339-L649 — processCigar: labeled loop
        let mut ci: usize = 0;
        'process_cigar: while ci < num_cigar_elements {
            // Java: CigarParser.java#L340
            if self.skip_overlapping_reads(
                record,
                header,
                position,
                direction,
                mate_alignment_start,
            ) {
                break;
            }

            // Java: CigarParser.java#L344
            self.cigar_element_length = self.cigar.get_cigar_element(ci).length;

            // Java: CigarParser.java#L347
            let operator = self.get_cigar_operator(ci);
            match operator {
                CigarOp::N => {
                    // Java: CigarParser.java#L349
                    self.process_not_matched();
                    ci += 1;
                    continue;
                }
                CigarOp::S => {
                    // Java: CigarParser.java#L351-L353
                    self.process_soft_clip(
                        chr_name,
                        record,
                        header,
                        &query_sequence,
                        mapping_quality,
                        ref_map,
                        &query_quality,
                        number_of_mismatches,
                        direction,
                        position,
                        total_length_including_soft_clipped,
                        ci,
                    );
                    ci += 1;
                    continue;
                }
                CigarOp::H => {
                    // Java: CigarParser.java#L355
                    self.offset = 0;
                    ci += 1;
                    continue;
                }
                CigarOp::I => {
                    // Java: CigarParser.java#L357-L359
                    self.offset = 0;
                    ci = self.process_insertion(
                        &query_sequence,
                        mapping_quality,
                        ref_map,
                        &query_quality,
                        number_of_mismatches,
                        direction,
                        position,
                        read_length_include_matching_and_insertions,
                        ci,
                    );
                    ci += 1; // Java for-loop ci++
                    continue;
                }
                CigarOp::D => {
                    // Java: CigarParser.java#L361-L365
                    self.offset = 0;
                    ci = self.process_deletion(
                        &query_sequence,
                        mapping_quality,
                        ref_map,
                        &query_quality,
                        number_of_mismatches,
                        direction,
                        read_length_include_matching_and_insertions,
                        ci,
                    );
                    ci += 1; // Java for-loop ci++
                    continue;
                }
                _ => {
                    // M/=/X — fall through to match processing below
                }
            }

            // ─── M-segment inner loop ─────────────────────────────────────────
            // Java: CigarParser.java#L369-L649
            let mut nmoff: i32 = 0;
            let mut moffset: i32 = 0;
            let query_seq_bytes = query_sequence.as_bytes();
            let query_qual_bytes = query_quality.as_bytes();

            let mut i = self.offset;
            while i < self.cigar_element_length {
                // Java: CigarParser.java#L373
                let trim =
                    self.is_trim_at_opt_t_bases(direction, total_length_including_soft_clipped);

                // Java: CigarParser.java#L376
                let n = self.read_position_including_soft_clipped as usize;
                if n >= query_seq_bytes.len() {
                    break;
                }
                let ch1 = query_seq_bytes[n];

                // Java: CigarParser.java#L381-L389 — skip 'N' bases
                if ch1 == b'N' {
                    if include_n_in_total_depth {
                        inc_cnt(&mut self.ref_coverage, self.start, 1);
                    }
                    self.start += 1;
                    self.read_position_including_soft_clipped += 1;
                    self.read_position_excluding_soft_clipped += 1;
                    i += 1;
                    continue;
                }
                let base_description = Self::single_base_description(ch1);
                let mut s: Option<String> = if base_description.is_some() {
                    None
                } else {
                    let mut owned = String::new();
                    owned.push(ch1 as char);
                    Some(owned)
                };
                let mut start_with_deletion = false;

                // Java: CigarParser.java#L392
                let mut q: f64 = (query_qual_bytes[n] as i32 - 33) as f64;
                let mut qbases: i32 = 1;
                let mut qibases: i32 = 0;
                let mut ss = String::new();

                // ─── MNV detection while-loop ─────────────────────────────────
                // Java: CigarParser.java#L403-L458
                while (self.start + 1) >= self.region.start
                    && (self.start + 1) <= self.region.end
                    && (i + 1) < self.cigar_element_length
                    && q >= goodq
                    && usize::try_from(self.read_position_including_soft_clipped)
                        .ok()
                        .is_some_and(|idx| {
                            is_reference_mismatch_and_not_n_at(
                                ref_map,
                                self.start,
                                query_seq_bytes,
                                idx,
                            )
                        })
                {
                    // Java: CigarParser.java#L413 — require higher quality for MNV
                    let next_n = (self.read_position_including_soft_clipped + 1) as usize;
                    if next_n >= query_qual_bytes.len()
                        || ((query_qual_bytes[next_n] as i32 - 33) as f64) < goodq + 5.0
                    {
                        break;
                    }
                    // Java: CigarParser.java#L417
                    if next_n >= query_seq_bytes.len() {
                        break;
                    }
                    let nuc = query_seq_bytes[next_n];
                    if nuc == b'N' {
                        break;
                    }
                    // Java: CigarParser.java#L421
                    if is_has_and_equals_base(b'N', ref_map, self.start + 1) {
                        break;
                    }

                    // Java: CigarParser.java#L425
                    if is_not_equals(ref_map.get(&(self.start + 1)).copied(), Some(nuc)) {
                        // Java: CigarParser.java#L427 — consecutive mismatch
                        ss.push(nuc as char);
                        q += (query_qual_bytes[next_n] as i32 - 33) as f64;
                        qbases += 1;
                        self.read_position_including_soft_clipped += 1;
                        self.read_position_excluding_soft_clipped += 1;
                        i += 1;
                        self.start += 1;
                        nmoff += 1;
                    } else {
                        // Java: CigarParser.java#L436-L458 — look ahead for more mismatches
                        let mut ssn: i32 = 0;
                        for ssi in 1..=vext {
                            if i + 1 + ssi >= self.cigar_element_length {
                                break;
                            }
                            let look_n =
                                (self.read_position_including_soft_clipped + 1 + ssi) as usize;
                            if look_n < query_seq_bytes.len()
                                && is_has_and_not_equals_base(
                                    query_seq_bytes[look_n],
                                    ref_map,
                                    self.start + 1 + ssi,
                                )
                            {
                                ssn = ssi + 1;
                                break;
                            }
                        }
                        if ssn == 0 {
                            break;
                        }
                        // Java: CigarParser.java#L450 — require higher quality for MNV
                        let ssn_n = (self.read_position_including_soft_clipped + ssn) as usize;
                        if ssn_n >= query_qual_bytes.len()
                            || ((query_qual_bytes[ssn_n] as i32 - 33) as f64) < goodq + 5.0
                        {
                            break;
                        }
                        // Java: CigarParser.java#L453-L457
                        for ssi in 1..=ssn {
                            let idx = (self.read_position_including_soft_clipped + ssi) as usize;
                            if idx < query_seq_bytes.len() {
                                ss.push(query_seq_bytes[idx] as char);
                            }
                            if idx < query_qual_bytes.len() {
                                q += (query_qual_bytes[idx] as i32 - 33) as f64;
                            }
                            qbases += 1;
                        }
                        self.read_position_including_soft_clipped += ssn;
                        self.read_position_excluding_soft_clipped += ssn;
                        i += ssn;
                        self.start += ssn;
                    }
                }

                // Java: CigarParser.java#L461-L463 — append MNV to s
                if !ss.is_empty() {
                    let s = Self::matching_description_mut(&mut s, ch1);
                    s.push('&');
                    s.push_str(&ss);
                }
                let mut ddlen: i32 = 0;

                // ─── Complex variant at end of M segment ──────────────────────
                // Java: CigarParser.java#L475-L570 — adjacent deletion case
                if self.is_closer_then_vext_and_good_base(
                    perform_local_realignment,
                    vext,
                    goodq,
                    query_seq_bytes,
                    ref_map,
                    query_qual_bytes,
                    ci,
                    i,
                    ss.is_empty(),
                    CigarOp::D,
                ) {
                    let s = Self::matching_description_mut(&mut s, ch1);
                    // Java: CigarParser.java#L477-L488 — consume remaining M bases
                    while i + 1 < self.cigar_element_length {
                        let next_n = (self.read_position_including_soft_clipped + 1) as usize;
                        if next_n < query_seq_bytes.len() {
                            s.push(query_seq_bytes[next_n] as char);
                        }
                        if next_n < query_qual_bytes.len() {
                            q += (query_qual_bytes[next_n] as i32 - 33) as f64;
                        }
                        qbases += 1;
                        i += 1;
                        self.read_position_including_soft_clipped += 1;
                        self.read_position_excluding_soft_clipped += 1;
                        self.start += 1;
                    }

                    // Java: CigarParser.java#L491 — replaceFirst("&", "")
                    if let Some(amp_pos) = s.find('&') {
                        s.replace_range(amp_pos..=amp_pos, "");
                    }
                    // Java: CigarParser.java#L493
                    let del_len = self.cigar.get_cigar_element(ci + 1).length;
                    let previous = std::mem::take(s);
                    *s = format!("-{}&{}", del_len, previous);
                    start_with_deletion = true;
                    ddlen = del_len;
                    ci += 1;

                    // Java: CigarParser.java#L499 — two insertions ahead
                    if self.is_two_insertions_ahead(ci) {
                        let ins_len = self.cigar.get_cigar_element(ci + 1).length;
                        let sub = substr_with_len(
                            query_seq_bytes,
                            self.read_position_including_soft_clipped + 1,
                            ins_len,
                        );
                        s.push('^');
                        s.push_str(&String::from_utf8_lossy(&sub));

                        // Java: CigarParser.java#L504-L510
                        let next_len = ins_len;
                        for qi in 1..=next_len {
                            let idx = (self.read_position_including_soft_clipped + 1 + qi) as usize;
                            if idx < query_qual_bytes.len() {
                                q += (query_qual_bytes[idx] as i32 - 33) as f64;
                            }
                            qibases += 1;
                        }
                        self.read_position_including_soft_clipped += next_len;
                        self.read_position_excluding_soft_clipped += next_len;
                        ci += 1;
                    }
                    // Java: CigarParser.java#L517
                    if self.is_next_after_num_matched(ci, 1) {
                        let next_m_len = self.cigar.get_cigar_element(ci + 1).length;
                        let tpl = Self::find_offset(
                            self.start + ddlen + 1,
                            self.read_position_including_soft_clipped + 1,
                            next_m_len,
                            &query_sequence,
                            &query_quality,
                            ref_map,
                            &mut self.ref_coverage,
                        );
                        let toffset = tpl.offset;
                        if toffset != 0 {
                            moffset = toffset;
                            nmoff += tpl.offset_number_of_mismatches;
                            s.push('&');
                            s.push_str(&tpl.sequence);
                            let tq_bytes = tpl.quality_sequence.as_bytes();
                            for &tq_byte in tq_bytes {
                                q += (tq_byte as i32 - 33) as f64;
                                qibases += 1;
                            }
                        }
                    }
                } else if self.is_closer_then_vext_and_good_base(
                    perform_local_realignment,
                    vext,
                    goodq,
                    query_seq_bytes,
                    ref_map,
                    query_qual_bytes,
                    ci,
                    i,
                    ss.is_empty(),
                    CigarOp::I,
                ) {
                    let s = Self::matching_description_mut(&mut s, ch1);
                    // Java: CigarParser.java#L535-L570 — adjacent insertion case
                    while i + 1 < self.cigar_element_length {
                        let next_n = (self.read_position_including_soft_clipped + 1) as usize;
                        if next_n < query_seq_bytes.len() {
                            s.push(query_seq_bytes[next_n] as char);
                        }
                        if next_n < query_qual_bytes.len() {
                            q += (query_qual_bytes[next_n] as i32 - 33) as f64;
                        }
                        qbases += 1;
                        i += 1;
                        self.read_position_including_soft_clipped += 1;
                        self.read_position_excluding_soft_clipped += 1;
                        self.start += 1;
                    }
                    // Java: CigarParser.java#L548 — replaceFirst("&", "")
                    if let Some(amp_pos) = s.find('&') {
                        s.replace_range(amp_pos..=amp_pos, "");
                    }
                    // Java: CigarParser.java#L549
                    let next_len = self.cigar.get_cigar_element(ci + 1).length;
                    let ins_sub = substr_with_len(
                        query_seq_bytes,
                        self.read_position_including_soft_clipped + 1,
                        next_len,
                    );
                    s.push_str(&String::from_utf8_lossy(&ins_sub));
                    // Java: CigarParser.java#L551 — s = substr(s, 0, nextLen) + "&" + s.substring(nextLen)
                    let s_bytes = s.as_bytes().to_vec();
                    let nl = next_len as usize;
                    let first = String::from_utf8_lossy(&substr_with_len(&s_bytes, 0, next_len))
                        .into_owned();
                    let second = if nl < s.len() { &s[nl..] } else { "" };
                    let new_s = format!("+{}&{}", first, second);
                    *s = new_s;

                    // Java: CigarParser.java#L555-L560
                    for qi in 1..=next_len {
                        let idx = (self.read_position_including_soft_clipped + 1 + qi) as usize;
                        if idx < query_qual_bytes.len() {
                            q += (query_qual_bytes[idx] as i32 - 33) as f64;
                        }
                        qibases += 1;
                    }
                    self.read_position_including_soft_clipped += next_len;
                    self.read_position_excluding_soft_clipped += next_len;
                    ci += 1;
                    qibases -= 1;
                    qbases += 1; // Java: need to add to set the correction insertion position
                }

                // Java: CigarParser.java#L571-L577 — add variation if not trimmed
                if !trim {
                    let pos = self.start - qbases + 1;
                    let s_ref = s.as_deref().unwrap_or(base_description.expect(
                        "non-owned matching descriptions should be known ASCII bases",
                    ));
                    let description_has_n = s.as_deref().is_some_and(|owned| owned.contains('N'));
                    if pos >= self.region.start && pos <= self.region.end && !description_has_n {
                        self.add_variation_for_matching_part(
                            mapping_quality,
                            number_of_mismatches,
                            direction,
                            read_length_include_matching_and_insertions,
                            nmoff,
                            s_ref,
                            start_with_deletion,
                            q,
                            qbases,
                            qibases,
                            ddlen,
                            pos,
                            goodq,
                        );
                    }
                }

                // Java: CigarParser.java#L580
                if start_with_deletion {
                    self.start += ddlen;
                }

                // Java: CigarParser.java#L584 — shift reference position
                if operator != CigarOp::I {
                    self.start += 1;
                }
                // Java: CigarParser.java#L587 — shift read position
                if operator != CigarOp::D {
                    self.read_position_including_soft_clipped += 1;
                    self.read_position_excluding_soft_clipped += 1;
                }
                // Java: CigarParser.java#L591 — skip overlap at end of inner loop
                if self.skip_overlapping_reads(
                    record,
                    header,
                    position,
                    direction,
                    mate_alignment_start,
                ) {
                    break 'process_cigar;
                }
                i += 1;
            } // end inner M-segment loop

            // Java: CigarParser.java#L595-L599
            if moffset != 0 {
                self.offset = moffset;
                self.read_position_including_soft_clipped += moffset;
                self.start += moffset;
                self.read_position_excluding_soft_clipped += moffset;
            }
            // Java: CigarParser.java#L600
            if self.start > self.region.end {
                break;
            }
            ci += 1;
        } // end process_cigar loop
    }

    /// Ported from: CigarParser.java:L672-L883 (processDeletion)
    /// Creates variant records for deletion CIGAR elements. Returns updated ci.
    fn process_deletion(
        &mut self,
        query_sequence: &str,
        mapping_quality: i32,
        ref_map: &ReferenceSequenceMap,
        query_quality: &str,
        number_of_mismatches: i32,
        direction: bool,
        read_length_include_matching_and_insertions: i32,
        ci: usize,
    ) -> usize {
        let (vext, goodq) = Self::with_conf(|conf| (conf.vext, conf.goodq));

        // Java: CigarParser.java#L673-L676 — skip indels next to introns
        if self.skip_indel_next_to_intron(ci) {
            self.read_position_excluding_soft_clipped += self.cigar_element_length;
            return ci;
        }

        let qs_bytes = query_sequence.as_bytes();
        let qq_bytes = query_quality.as_bytes();

        // Java: CigarParser.java#L679 — $s description string
        let mut desc_string = format!("-{}", self.cigar_element_length);
        // Java: CigarParser.java#L681 — $ss sequence to append if next segment matched
        let mut seq_to_append = String::new();
        // Java: CigarParser.java#L683 — $q1 quality of last base before deletion
        let q1_idx = (self.read_position_including_soft_clipped - 1) as usize;
        let quality_of_last_before_del = if q1_idx < qq_bytes.len() {
            qq_bytes[q1_idx]
        } else {
            b'!'
        };
        // Java: CigarParser.java#L685 — $q quality of this segment
        let mut quality_of_segment = String::new();

        // Java: CigarParser.java#L688-L690 — multi-indel offsets
        let mut multoffs: i32 = 0;
        let mut multoffp: i32 = 0;
        let mut nmoff: i32 = 0;
        let mut ci = ci;

        // Java: CigarParser.java#L698
        if self.is_insertion_or_deletion_with_next_matched(ci) {
            // Java: CigarParser.java#L699-L701
            let m_len = self.cigar.get_cigar_element(ci + 1).length;
            let indel_len = self.cigar.get_cigar_element(ci + 2).length;
            let begin = self.read_position_including_soft_clipped;

            // Java: CigarParser.java#L703-L704
            self.append_segments(
                query_sequence,
                query_quality,
                ci,
                &mut desc_string,
                &mut quality_of_segment,
                m_len,
                indel_len,
                begin,
                false,
            );

            // Java: CigarParser.java#L709-L710
            let ci2_op = self.cigar.get_cigar_element(ci + 2).operator;
            multoffs += m_len + if ci2_op == CigarOp::D { indel_len } else { 0 };
            multoffp += m_len + if ci2_op == CigarOp::I { indel_len } else { 0 };

            // Java: CigarParser.java#L711
            if self.is_next_after_num_matched(ci, 3) {
                let mut vsn = 0i32;
                let tn = self.read_position_including_soft_clipped + multoffp;
                let ts = self.start + multoffs + self.cigar_element_length;
                let ci3_len = self.cigar.get_cigar_element(ci + 3).length;

                // Java: CigarParser.java#L715-L735
                let mut vi = 0i32;
                while vsn <= vext && vi < ci3_len {
                    let tn_vi = (tn + vi) as usize;
                    // Java: CigarParser.java#L716-L717
                    if tn_vi >= qs_bytes.len() || qs_bytes[tn_vi] == b'N' {
                        break;
                    }
                    // Java: CigarParser.java#L720
                    if tn_vi >= qq_bytes.len() || ((qq_bytes[tn_vi] as i32 - 33) as f64) < goodq {
                        break;
                    }
                    // Java: CigarParser.java#L723
                    if is_has_and_equals_base(b'N', ref_map, ts + vi) {
                        break;
                    }
                    // Java: CigarParser.java#L726-L734
                    if let Some(&ref_ch) = ref_map.get(&(ts + vi)) {
                        if is_not_equals(Some(qs_bytes[tn_vi]), Some(ref_ch)) {
                            self.offset = vi + 1;
                            nmoff += 1;
                            vsn = 0;
                        } else {
                            vsn += 1;
                        }
                    }
                    vi += 1;
                }
                // Java: CigarParser.java#L736-L739
                if self.offset != 0 {
                    seq_to_append.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qs_bytes,
                        tn,
                        self.offset,
                    )));
                    quality_of_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qq_bytes,
                        tn,
                        self.offset,
                    )));
                }
            }
            // Java: CigarParser.java#L742 — skip next 2 CIGAR segments
            ci += 2;
        } else if self.is_next_insertion(ci) {
            // Java: CigarParser.java#L749-L751
            let ins_len = self.cigar.get_cigar_element(ci + 1).length;
            // Java: CigarParser.java#L753 — append '^' + inserted sequence
            desc_string.push('^');
            desc_string.push_str(&String::from_utf8_lossy(&substr_with_len(
                qs_bytes,
                self.read_position_including_soft_clipped,
                ins_len,
            )));
            // Java: CigarParser.java#L755
            quality_of_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
                qq_bytes,
                self.read_position_including_soft_clipped,
                ins_len,
            )));

            // Java: CigarParser.java#L759
            multoffp += ins_len;

            // Java: CigarParser.java#L760
            if self.is_next_after_num_matched(ci, 2) {
                let m_len = self.cigar.get_cigar_element(ci + 2).length;
                let mut vsn = 0i32;
                let tn = self.read_position_including_soft_clipped + multoffp;
                let ts = self.start + self.cigar_element_length;

                // Java: CigarParser.java#L764-L781
                let mut vi = 0i32;
                while vsn <= vext && vi < m_len {
                    let tn_vi = (tn + vi) as usize;
                    if tn_vi >= qs_bytes.len() || qs_bytes[tn_vi] == b'N' {
                        break;
                    }
                    if tn_vi >= qq_bytes.len() || ((qq_bytes[tn_vi] as i32 - 33) as f64) < goodq {
                        break;
                    }
                    if let Some(&ref_ch) = ref_map.get(&(ts + vi)) {
                        // Java: CigarParser.java#L773
                        if is_equals(Some(b'N'), Some(ref_ch)) {
                            break;
                        }
                        if is_not_equals(Some(qs_bytes[tn_vi]), Some(ref_ch)) {
                            self.offset = vi + 1;
                            nmoff += 1;
                            vsn = 0;
                        } else {
                            vsn += 1;
                        }
                    }
                    vi += 1;
                }
                // Java: CigarParser.java#L782-L785
                if self.offset != 0 {
                    seq_to_append.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qs_bytes,
                        tn,
                        self.offset,
                    )));
                    quality_of_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qq_bytes,
                        tn,
                        self.offset,
                    )));
                }
            }
            // Java: CigarParser.java#L788
            ci += 1;
        } else {
            // Java: CigarParser.java#L795
            if self.is_next_matched(ci) {
                let m_len = self.cigar.get_cigar_element(ci + 1).length;
                let mut vsn = 0i32;

                // Java: CigarParser.java#L799-L818
                let mut vi = 0i32;
                while vsn <= vext && vi < m_len {
                    let n_vi = (self.read_position_including_soft_clipped + vi) as usize;
                    if n_vi >= qs_bytes.len() || qs_bytes[n_vi] == b'N' {
                        break;
                    }
                    if n_vi >= qq_bytes.len() || ((qq_bytes[n_vi] as i32 - 33) as f64) < goodq {
                        break;
                    }
                    if let Some(&ref_ch) =
                        ref_map.get(&(self.start + self.cigar_element_length + vi))
                    {
                        // Java: CigarParser.java#L808
                        if is_equals(Some(b'N'), Some(ref_ch)) {
                            break;
                        }
                        if is_not_equals(Some(qs_bytes[n_vi]), Some(ref_ch)) {
                            self.offset = vi + 1;
                            nmoff += 1;
                            vsn = 0;
                        } else {
                            vsn += 1;
                        }
                    }
                    vi += 1;
                }

                // Java: CigarParser.java#L821-L824
                if self.offset != 0 {
                    seq_to_append.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qs_bytes,
                        self.read_position_including_soft_clipped,
                        self.offset,
                    )));
                    quality_of_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qq_bytes,
                        self.read_position_including_soft_clipped,
                        self.offset,
                    )));
                }
            }
        }

        // Java: CigarParser.java#L830-L832 — append '&' + ss if next segment has good match
        if self.offset > 0 {
            desc_string.push('&');
            desc_string.push_str(&seq_to_append);
        }

        // Java: CigarParser.java#L836-L842 — append best quality
        let n_offset = (self.read_position_including_soft_clipped + self.offset) as usize;
        if n_offset >= qq_bytes.len() {
            quality_of_segment.push(quality_of_last_before_del as char);
        } else {
            let q2 = qq_bytes[n_offset];
            quality_of_segment.push(if quality_of_last_before_del > q2 {
                quality_of_last_before_del as char
            } else {
                q2 as char
            });
        }

        // Java: CigarParser.java#L845-L848 — add variation if within region
        if self.start >= self.region.start && self.start <= self.region.end {
            self.add_variation_for_deletion(
                mapping_quality,
                number_of_mismatches,
                direction,
                read_length_include_matching_and_insertions,
                &desc_string,
                &quality_of_segment,
                nmoff,
            );
        }

        // Java: CigarParser.java#L852 — adjust reference position
        self.start += self.cigar_element_length + self.offset + multoffs;
        // Java: CigarParser.java#L855-L856 — adjust read position
        self.read_position_including_soft_clipped += self.offset + multoffp;
        self.read_position_excluding_soft_clipped += self.offset + multoffp;
        ci
    }

    /// Ported from: CigarParser.java:L930-L1121 (processInsertion)
    /// Creates variant records for insertion CIGAR elements. Returns updated ci.
    fn process_insertion(
        &mut self,
        query_sequence: &str,
        mapping_quality: i32,
        ref_map: &ReferenceSequenceMap,
        query_quality: &str,
        number_of_mismatches: i32,
        direction: bool,
        position: i32,
        read_length_include_matching_and_insertions: i32,
        ci: usize,
    ) -> usize {
        let (vext, goodq) = Self::with_conf(|conf| (conf.vext, conf.goodq));

        // Java: CigarParser.java#L937-L939 — skip indels next to introns
        if self.skip_indel_next_to_intron(ci) {
            self.read_position_including_soft_clipped += self.cigar_element_length;
            return ci;
        }

        let qs_bytes = query_sequence.as_bytes();
        let qq_bytes = query_quality.as_bytes();

        // Java: CigarParser.java#L942 — inserted segment of read sequence
        let mut desc_string = String::from_utf8_lossy(&substr_with_len(
            qs_bytes,
            self.read_position_including_soft_clipped,
            self.cigar_element_length,
        ))
        .into_owned();

        // Java: CigarParser.java#L945 — quality of this segment
        let mut quality_string = String::from_utf8_lossy(&substr_with_len(
            qq_bytes,
            self.read_position_including_soft_clipped,
            self.cigar_element_length,
        ))
        .into_owned();

        // Java: CigarParser.java#L947 — sequence to append if next segment matched
        let mut ss = String::new();

        // Java: CigarParser.java#L950-L952
        let mut multoffs: i32 = 0;
        let mut multoffp: i32 = 0;
        let mut nmoff: i32 = 0;
        let mut ci = ci;

        // Java: CigarParser.java#L961
        if self.is_insertion_or_deletion_with_next_matched(ci) {
            // Java: CigarParser.java#L962-L964
            let m_len = self.cigar.get_cigar_element(ci + 1).length;
            let indel_len = self.cigar.get_cigar_element(ci + 2).length;
            let begin = self.read_position_including_soft_clipped + self.cigar_element_length;

            // Java: CigarParser.java#L966-L967
            self.append_segments(
                query_sequence,
                query_quality,
                ci,
                &mut desc_string,
                &mut quality_string,
                m_len,
                indel_len,
                begin,
                true,
            );

            // Java: CigarParser.java#L971-L972
            let ci2_op = self.cigar.get_cigar_element(ci + 2).operator;
            multoffs += m_len + if ci2_op == CigarOp::D { indel_len } else { 0 };
            multoffp += m_len + if ci2_op == CigarOp::I { indel_len } else { 0 };

            // Java: CigarParser.java#L974-L975
            let ci6 = if self.cigar.num_cigar_elements() > ci + 3 {
                self.cigar.get_cigar_element(ci + 3).length
            } else {
                0
            };

            // Java: CigarParser.java#L976-L983
            if ci6 != 0 && self.cigar.get_cigar_element(ci + 3).operator == CigarOp::M {
                let tpl = Self::find_offset(
                    self.start + multoffs,
                    self.read_position_including_soft_clipped
                        + self.cigar_element_length
                        + multoffp,
                    ci6,
                    query_sequence,
                    query_quality,
                    ref_map,
                    &mut self.ref_coverage,
                );
                self.offset = tpl.offset;
                ss = tpl.sequence;
                quality_string.push_str(&tpl.quality_sequence);
            }
            // Java: CigarParser.java#L985 — skip 2 CIGAR segments
            ci += 2;
        } else {
            // Java: CigarParser.java#L990
            if self.is_next_matched(ci) {
                let mut vsn = 0i32;
                let next_m_len = self.cigar.get_cigar_element(ci + 1).length;

                // Java: CigarParser.java#L993-L1009
                let mut vi = 0i32;
                while vsn <= vext && vi < next_m_len {
                    let n_vi = (self.read_position_including_soft_clipped
                        + self.cigar_element_length
                        + vi) as usize;
                    // Java: CigarParser.java#L995
                    if n_vi >= qs_bytes.len() || qs_bytes[n_vi] == b'N' {
                        break;
                    }
                    // Java: CigarParser.java#L999
                    if n_vi >= qq_bytes.len() || ((qq_bytes[n_vi] as i32 - 33) as f64) < goodq {
                        break;
                    }
                    // Java: CigarParser.java#L1003
                    if ref_map.contains_key(&(self.start + vi)) {
                        let ref_ch = ref_map[&(self.start + vi)];
                        if is_not_equals(Some(qs_bytes[n_vi]), Some(ref_ch)) {
                            self.offset = vi + 1;
                            nmoff += 1;
                            vsn = 0;
                        } else {
                            vsn += 1;
                        }
                    }
                    vi += 1;
                }

                // Java: CigarParser.java#L1010-L1018
                if self.offset != 0 {
                    ss.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qs_bytes,
                        self.read_position_including_soft_clipped + self.cigar_element_length,
                        self.offset,
                    )));
                    quality_string.push_str(&String::from_utf8_lossy(&substr_with_len(
                        qq_bytes,
                        self.read_position_including_soft_clipped + self.cigar_element_length,
                        self.offset,
                    )));
                    // Java: CigarParser.java#L1016 — increase coverage
                    for osi in 0..self.offset {
                        inc_cnt(&mut self.ref_coverage, self.start + osi, 1);
                    }
                }
            }
        }

        // Java: CigarParser.java#L1024-L1026 — append '&' + ss if offset > 0
        if self.offset > 0 {
            desc_string.push('&');
            desc_string.push_str(&ss);
        }

        // Java: CigarParser.java#L1029
        if self.start - 1 >= self.region.start
            && self.start - 1 <= self.region.end
            && !desc_string.contains('N')
        {
            let mut insertion_position = self.start - 1;

            // Java: CigarParser.java#L1031-L1039 — adjInsPos left-alignment
            if BEGIN_ATGC_END.is_match(&desc_string) {
                let tpl = adj_ins_pos(self.start - 1, &desc_string, ref_map);
                let adjusted_pos = tpl.base_insert.unwrap_or(self.start - 1);
                let adjusted_desc = tpl.insertion_sequence;

                // Java: CigarParser.java#L1036
                let check_idx =
                    self.read_position_including_soft_clipped - 1 - (self.start - 1 - adjusted_pos);
                if check_idx > 0 {
                    insertion_position = adjusted_pos;
                    desc_string = adjusted_desc;
                }
            }

            // Java: CigarParser.java#L1042 — add to insertion count
            let ins_desc = format!("+{}", desc_string);
            {
                let ins_count_map = self
                    .position_to_insertion_count
                    .entry(insertion_position)
                    .or_default();
                inc_cnt(ins_count_map, ins_desc.clone(), 1);
            }

            // Java: CigarParser.java#L1045 — add variation to insertion variants
            let hv = get_variation(&mut self.insertion_variants, insertion_position, ins_desc);
            hv.inc_dir(direction);
            // Java: CigarParser.java#L1048
            hv.vars_count += 1;

            // Java: CigarParser.java#L1050-L1052 — min of positions from start/end
            let rlen1 = read_length_include_matching_and_insertions;
            let tp: i32 = if self.read_position_excluding_soft_clipped
                < rlen1 - self.read_position_excluding_soft_clipped
            {
                self.read_position_excluding_soft_clipped + 1
            } else {
                rlen1 - self.read_position_excluding_soft_clipped
            };

            // Java: CigarParser.java#L1055-L1058 — mean read quality
            let quality_bytes = quality_string.as_bytes();
            let mut tmpq: f64 = 0.0;
            for &b in quality_bytes {
                tmpq += (b as i32 - 33) as f64;
            }
            tmpq /= quality_bytes.len() as f64;

            // Java: CigarParser.java#L1061-L1062
            if !hv.pstd && hv.pp != 0 && tp != hv.pp {
                hv.pstd = true;
            }
            // Java: CigarParser.java#L1065-L1066
            if !hv.qstd && hv.pq != 0.0 && tmpq != hv.pq {
                hv.qstd = true;
            }
            // Java: CigarParser.java#L1067-L1072
            hv.mean_position += tp as f64;
            hv.mean_quality += tmpq;
            hv.mean_mapping_quality += mapping_quality as f64;
            hv.pp = tp;
            hv.pq = tmpq;
            if tmpq >= goodq {
                hv.high_quality_reads_count += 1;
            } else {
                hv.low_quality_reads_count += 1;
            }
            // Java: CigarParser.java#L1073
            hv.number_of_mismatches += (number_of_mismatches - nmoff) as f64;

            // Save pstd/qstd from hv before accessing non_insertion_variants
            // (hv borrows insertion_variants; we need non_insertion_variants below)
            let hv_pstd = hv.pstd;
            let hv_qstd = hv.qstd;

            // Java: CigarParser.java#L1076-L1088 — adjust reference count for insertion reads
            let index_of_insertion = self.read_position_including_soft_clipped
                - 1
                - (self.start - 1 - insertion_position);
            let idx = index_of_insertion as usize;
            if insertion_position > position
                && idx < qs_bytes.len()
                && is_has_and_equals_base(qs_bytes[idx], ref_map, insertion_position)
            {
                // Java: CigarParser.java#L1082 — getVariationMaybe + subCnt
                let desc_char = [qs_bytes[idx]];
                let desc_char =
                    std::str::from_utf8(&desc_char).expect("read base lookup keys should be ASCII");
                if let Some(map) = self.non_insertion_variants.get_mut(&insertion_position) {
                    if let Some(tv) = map.entries.get_mut(desc_char) {
                        let q_val = if idx < qq_bytes.len() {
                            (qq_bytes[idx] as i32 - 33) as f64
                        } else {
                            0.0
                        };
                        Self::sub_cnt(
                            tv,
                            direction,
                            tp,
                            q_val,
                            mapping_quality,
                            number_of_mismatches - nmoff,
                        );
                    }
                }
            }

            // Java: CigarParser.java#L1093-L1109 — edge insertion at ci==1
            if ci == 1
                && (self.cigar.get_cigar_element(0).operator == CigarOp::S
                    || self.cigar.get_cigar_element(0).operator == CigarOp::H)
            {
                if let Some(&ref_base) = ref_map.get(&insertion_position) {
                    let ref_base_str = [ref_base];
                    let ref_base_str = std::str::from_utf8(&ref_base_str)
                        .expect("reference base keys should be ASCII");
                    let ttref = get_variation(
                        &mut self.non_insertion_variants,
                        insertion_position,
                        ref_base_str,
                    );
                    // Java: CigarParser.java#L1096-L1107
                    ttref.inc_dir(direction);
                    ttref.vars_count += 1;
                    ttref.pstd = hv_pstd;
                    ttref.qstd = hv_qstd;
                    ttref.mean_position += tp as f64;
                    ttref.mean_quality += tmpq;
                    ttref.mean_mapping_quality += mapping_quality as f64;
                    ttref.pp = tp;
                    ttref.pq = tmpq;
                    ttref.number_of_mismatches += (number_of_mismatches - nmoff) as f64;
                    inc_cnt(&mut self.ref_coverage, insertion_position, 1);
                }
            }
        }

        // Java: CigarParser.java#L1113-L1115
        self.read_position_including_soft_clipped +=
            self.cigar_element_length + self.offset + multoffp;
        self.read_position_excluding_soft_clipped +=
            self.cigar_element_length + self.offset + multoffp;
        // Java: CigarParser.java#L1117
        self.start += self.offset + multoffs;
        ci
    }

    /// Ported from: CigarParser.java:L1124-L1304 (processSoftClip)
    /// Handles S operator (5' and 3' soft clips, chimeric detection).
    #[allow(clippy::too_many_arguments)]
    fn process_soft_clip(
        &mut self,
        chr_name: &str,
        record: &bam::Record,
        _header: &HeaderView,
        query_sequence: &str,
        mapping_quality: i32,
        ref_map: &ReferenceSequenceMap,
        query_quality: &str,
        number_of_mismatches: i32,
        direction: bool,
        position: i32,
        total_length_including_soft_clipped: i32,
        ci: usize,
    ) {
        let (chimeric, debug_y, chr_length) = GlobalReadOnlyScope::with_instance(|scope| {
            (
                scope.conf.chimeric,
                scope.conf.y,
                scope.chr_lengths.get(chr_name).copied().unwrap_or(i32::MAX),
            )
        });
        let qs_bytes = query_sequence.as_bytes();
        let qq_bytes = query_quality.as_bytes();

        // Java: CigarParser.java#L1118 — first CIGAR element = 5' soft clip
        if ci == 0 {
            // ── 5' soft clip ──────────────────────────────────────────────
            // Java: CigarParser.java#L1120-L1155 — chimeric detection
            if !chimeric {
                let sa_tag_string: Option<String> = record.aux(b"SA").ok().and_then(|a| match a {
                    Aux::String(s) => Some(s.to_string()),
                    _ => None,
                });

                if self.cigar_element_length >= 20 && sa_tag_string.is_some() {
                    // Java: CigarParser.java#L1125
                    if self.is_read_chimeric_with_sa(
                        record,
                        position,
                        sa_tag_string.as_deref().unwrap(),
                        direction,
                        true,
                    ) {
                        self.read_position_including_soft_clipped += self.cigar_element_length;
                        self.offset = 0;
                        self.start = position;
                        return;
                    }
                } else if self.cigar_element_length >= SEED_1 {
                    // Java: CigarParser.java#L1135-L1152
                    let seq = get_reverse_complemented_sequence(
                        &record.seq().as_bytes(),
                        0,
                        self.cigar_element_length,
                    );
                    let seed_len = SEED_1 as usize;
                    if seq.len() >= seed_len {
                        let rc_seed = String::from_utf8_lossy(&seq[..seed_len]).to_string();
                        if let Some(positions) = self.reference.seed.get(&rc_seed) {
                            if positions.len() == 1
                                && (self.start - positions[0]).abs() < 2 * self.max_read_length
                            {
                                self.read_position_including_soft_clipped +=
                                    self.cigar_element_length;
                                self.offset = 0;
                                self.start = position;
                                if debug_y {
                                    eprintln!(
                                        "{} at 5' is a chimeric at {} by SEED {}",
                                        String::from_utf8_lossy(&seq),
                                        self.start,
                                        SEED_1
                                    );
                                }
                                return;
                            }
                        }
                    }
                }
            }

            // Java: CigarParser.java#L1158-L1173 — match-back loop (5')
            while self.cigar_element_length - 1 >= 0
                && self.start - 1 > 0
                && self.start - 1 <= chr_length
                && is_has_and_equals_str(
                    ref_map,
                    self.start - 1,
                    query_sequence,
                    self.cigar_element_length - 1,
                )
                && (qq_bytes[(self.cigar_element_length - 1) as usize] as i32 - 33) > 10
            {
                // Java: CigarParser.java#L1165
                let ref_base = ref_map.get(&(self.start - 1)).unwrap();
                let ref_base_str = [*ref_base];
                let ref_base_str = std::str::from_utf8(&ref_base_str)
                    .expect("reference base keys should be ASCII");
                let variation = get_variation(
                    &mut self.non_insertion_variants,
                    self.start - 1,
                    ref_base_str,
                );
                // Java: CigarParser.java#L1167
                Self::add_cnt(
                    variation,
                    direction,
                    self.cigar_element_length,
                    (qq_bytes[(self.cigar_element_length - 1) as usize] as i32 - 33) as f64,
                    mapping_quality,
                    number_of_mismatches,
                );
                // Java: CigarParser.java#L1169
                inc_cnt(&mut self.ref_coverage, self.start - 1, 1);
                self.start -= 1;
                self.cigar_element_length -= 1;
            }

            // Java: CigarParser.java#L1175-L1201 — remaining 5' soft clip quality scan
            if self.cigar_element_length > 0 {
                let mut sum_of_read_qualities: i32 = 0;
                let mut number_of_high_quality_bases: i32 = 0;
                let mut number_of_low_quality_bases: i32 = 0;

                // Java: CigarParser.java#L1181 — loop from end of remaining soft clip
                let mut si = self.cigar_element_length - 1;
                while si >= 0 {
                    let si_u = si as usize;
                    // Java: CigarParser.java#L1183
                    if si_u < qs_bytes.len() && qs_bytes[si_u] == b'N' {
                        break;
                    }
                    // Java: CigarParser.java#L1187
                    let base_quality = if si_u < qq_bytes.len() {
                        qq_bytes[si_u] as i32 - 33
                    } else {
                        0
                    };
                    if base_quality <= 12 {
                        number_of_low_quality_bases += 1;
                    }
                    // Java: CigarParser.java#L1191
                    if number_of_low_quality_bases > 1 {
                        break;
                    }
                    sum_of_read_qualities += base_quality;
                    number_of_high_quality_bases += 1;
                    si -= 1;
                }
                // Java: CigarParser.java#L1200-L1201
                self.sclip5_high_quality_processing(
                    query_sequence,
                    mapping_quality,
                    query_quality,
                    number_of_mismatches,
                    direction,
                    sum_of_read_qualities,
                    number_of_high_quality_bases,
                    number_of_low_quality_bases,
                );
            }
            // Java: CigarParser.java#L1203 — restore cigarElementLength
            self.cigar_element_length = self.cigar.get_cigar_element(ci).length;
        } else if ci == self.cigar.num_cigar_elements() - 1 {
            // ── 3' soft clip ──────────────────────────────────────────────
            // Java: CigarParser.java#L1204-L1243 — chimeric detection (3')
            if !chimeric {
                let sa_tag_string: Option<String> = record.aux(b"SA").ok().and_then(|a| match a {
                    Aux::String(s) => Some(s.to_string()),
                    _ => None,
                });

                if self.cigar_element_length >= 20 && sa_tag_string.is_some() {
                    // Java: CigarParser.java#L1209
                    if self.is_read_chimeric_with_sa(
                        record,
                        position,
                        sa_tag_string.as_deref().unwrap(),
                        direction,
                        false,
                    ) {
                        self.read_position_including_soft_clipped += self.cigar_element_length;
                        self.offset = 0;
                        self.start = position;
                        return;
                    }
                } else if self.cigar_element_length >= SEED_1 {
                    // Java: CigarParser.java#L1219-L1237
                    let seq = get_reverse_complemented_sequence(
                        &record.seq().as_bytes(),
                        -self.cigar_element_length,
                        self.cigar_element_length,
                    );
                    let seed_len = SEED_1 as usize;
                    if seq.len() >= seed_len {
                        // Java: substr(sequence, -SEED_1, SEED_1)
                        let start_idx = if seq.len() >= seed_len {
                            seq.len() - seed_len
                        } else {
                            0
                        };
                        let rc_seed = String::from_utf8_lossy(&seq[start_idx..]).to_string();
                        if let Some(positions) = self.reference.seed.get(&rc_seed) {
                            if positions.len() == 1
                                && (self.start - positions[0]).abs() < 2 * self.max_read_length
                            {
                                self.read_position_including_soft_clipped +=
                                    self.cigar_element_length;
                                self.offset = 0;
                                self.start = position;
                                if debug_y {
                                    eprintln!(
                                        "{} at 3' is a chimeric at {} by SEED {}",
                                        String::from_utf8_lossy(&seq),
                                        self.start,
                                        SEED_1
                                    );
                                }
                                return;
                            }
                        }
                    }
                }
            }

            // Java: CigarParser.java#L1247-L1265 — match-back loop (3')
            while (self.read_position_including_soft_clipped as usize) < qs_bytes.len()
                && is_has_and_equals_str(
                    ref_map,
                    self.start,
                    query_sequence,
                    self.read_position_including_soft_clipped,
                )
                && (qq_bytes[self.read_position_including_soft_clipped as usize] as i32 - 33) > 10
            {
                // Java: CigarParser.java#L1253
                let ref_base = ref_map.get(&self.start).unwrap();
                let ref_base_str = [*ref_base];
                let ref_base_str = std::str::from_utf8(&ref_base_str)
                    .expect("reference base keys should be ASCII");
                let variation =
                    get_variation(&mut self.non_insertion_variants, self.start, ref_base_str);
                // Java: CigarParser.java#L1255
                Self::add_cnt(
                    variation,
                    direction,
                    total_length_including_soft_clipped - self.read_position_excluding_soft_clipped,
                    (qq_bytes[self.read_position_including_soft_clipped as usize] as i32 - 33)
                        as f64,
                    mapping_quality,
                    number_of_mismatches,
                );
                // Java: CigarParser.java#L1258
                inc_cnt(&mut self.ref_coverage, self.start, 1);
                self.read_position_including_soft_clipped += 1;
                self.start += 1;
                self.cigar_element_length -= 1;
                self.read_position_excluding_soft_clipped += 1;
            }

            // Java: CigarParser.java#L1266-L1290 — remaining 3' soft clip quality scan
            let remaining = qs_bytes.len() as i32 - self.read_position_including_soft_clipped;
            if remaining > 0 {
                let mut sum_of_read_qualities: i32 = 0;
                let mut number_of_high_quality_bases: i32 = 0;
                let mut number_of_low_quality_bases: i32 = 0;

                // Java: CigarParser.java#L1271 — forward loop
                for si in 0..self.cigar_element_length {
                    let idx = (self.read_position_including_soft_clipped + si) as usize;
                    // Java: CigarParser.java#L1274
                    if idx >= qs_bytes.len() || qs_bytes[idx] == b'N' {
                        break;
                    }
                    // Java: CigarParser.java#L1278
                    let base_quality = if idx < qq_bytes.len() {
                        qq_bytes[idx] as i32 - 33
                    } else {
                        0
                    };
                    if base_quality <= 12 {
                        number_of_low_quality_bases += 1;
                    }
                    // Java: CigarParser.java#L1283
                    if number_of_low_quality_bases > 1 {
                        break;
                    }
                    sum_of_read_qualities += base_quality;
                    number_of_high_quality_bases += 1;
                }
                // Java: CigarParser.java#L1289-L1290
                self.sclip3_high_quality_processing(
                    query_sequence,
                    mapping_quality,
                    query_quality,
                    number_of_mismatches,
                    direction,
                    sum_of_read_qualities,
                    number_of_high_quality_bases,
                    number_of_low_quality_bases,
                );
            }
        }
        // Java: CigarParser.java#L1293-L1295 — move read position by cigarElementLength
        self.read_position_including_soft_clipped += self.cigar_element_length;
        self.offset = 0;
        self.start = position; // Reset start due to softclipping adjustment
    }

    /// Ported from: CigarParser.java:L2017-L2310 (prepareSVStructuresForAnalysis)
    /// Fills SV structures (DEL/DUP/INV/FUS) from discordant pairs.
    fn prepare_sv_structures_for_analysis(
        &mut self,
        record: &bam::Record,
        header: &HeaderView,
        query_quality: &str,
        number_of_mismatches: i32,
        read_direction: bool,
        mate_direction: bool,
        position: i32,
        total_length_including_soft_clipped: i32,
    ) {
        let (goodq, inssize, insstdamt, insstd) = GlobalReadOnlyScope::with_instance(|scope| {
            (
                scope.conf.goodq,
                scope.conf.inssize,
                scope.conf.insstdamt,
                scope.conf.insstd,
            )
        });

        // Java: CigarParser.java#L1995
        let mate_start = record.mpos() as i32 + 1; // 0-based to 1-based
        let mend = mate_start + total_length_including_soft_clipped;
        let mut end = position;
        let cigar_string = self.cigar.to_string();
        let msegs = global_find(&ALIGNED_LENGTH_MND, &cigar_string);
        let msegs_sum: i32 = msegs.iter().filter_map(|s| s.parse::<i32>().ok()).sum();
        end += msegs_sum;

        // Java: CigarParser.java#L2004-L2011 — soft5
        let mut soft5: i32 = 0;
        if let Some(caps) = BEGIN_NUM_S_OR_BEGIN_NUM_H.captures(&cigar_string) {
            if let Some(m) = caps.get(1) {
                let tt: i32 = m.as_str().parse().unwrap_or(0);
                if tt != 0 {
                    let idx = (tt - 1) as usize;
                    if idx < query_quality.as_bytes().len()
                        && (query_quality.as_bytes()[idx] as i32 - 33) as f64 > goodq
                    {
                        soft5 = position;
                    }
                }
            }
        }

        // Java: CigarParser.java#L2013-L2019 — soft3
        let mut soft3: i32 = 0;
        if let Some(caps) = END_NUM_S_OR_NUM_H.captures(&cigar_string) {
            if let Some(m) = caps.get(1) {
                let tt: i32 = m.as_str().parse().unwrap_or(0);
                if tt != 0 {
                    let idx = query_quality.len().saturating_sub(tt as usize);
                    if idx < query_quality.as_bytes().len()
                        && (query_quality.as_bytes()[idx] as i32 - 33) as f64 > goodq
                    {
                        soft3 = end;
                    }
                }
            }
        }

        // Java: CigarParser.java#L2021-L2024
        let read_dir_num: i32 = if read_direction { -1 } else { 1 };
        let mate_dir_num: i32 = if mate_direction { 1 } else { -1 };
        let min_d: i32 = 75;

        let qq_bytes = query_quality.as_bytes();

        // Java: CigarParser.java#L2027
        if get_mate_reference_name(record, header) == "=" {
            // ── Same chromosome ──────────────────────────────────────
            let mut mlen = record.insert_size() as i32;

            // Java: CigarParser.java#L2029-L2032 — filter MC soft-clipped mates
            let mc_tag: Option<String> = record.aux(b"MC").ok().and_then(|a| match a {
                Aux::String(s) => Some(s.to_string()),
                _ => None,
            });
            if let Some(ref mc) = mc_tag {
                if MC_Z_NUM_S_ANY_NUM_S.is_match(mc) {
                    return;
                }
            }
            // Java: CigarParser.java#L2034-L2035 — filter low MQ mates
            let mq_tag: Option<i32> = record.aux(b"MQ").ok().and_then(|a| Self::aux_to_i32(&a));
            if let Some(mq) = mq_tag {
                if mq < 15 {
                    return;
                }
            }

            if read_dir_num * mate_dir_num == -1
                && (mlen * read_dir_num) > 0
                && qq_bytes.len() as i32 > MINMAPBASE
            {
                // ── Deletion candidate ──
                // Java: CigarParser.java#L2040
                mlen = if mate_start > position {
                    mend - position
                } else {
                    end - mate_start
                };
                if mlen.abs() > inssize + insstdamt * insstd {
                    if read_dir_num == 1 {
                        // Java: CigarParser.java#L2043-L2048
                        if self.sv_structures.svfdel.is_empty()
                            || (position - self.sv_structures.svdelfend) as f64
                                > MINSVCDIST * self.max_read_length as f64
                        {
                            let mut sclip = Sclip::default();
                            sclip.base.vars_count = 0;
                            self.sv_structures.svfdel.push(sclip);
                        }
                        let svref = Self::get_last_sv_structure(&mut self.sv_structures.svfdel);
                        Self::add_sv_to(
                            svref,
                            position,
                            end,
                            mate_start,
                            mend,
                            read_dir_num,
                            total_length_including_soft_clipped,
                            mlen,
                            soft3,
                            self.max_read_length as f64 / 2.0,
                            qq_bytes[MINMAPBASE as usize] as i32 - 33,
                            record.mapq() as i32,
                            number_of_mismatches,
                        );
                        self.sv_structures.svdelfend = end;
                    } else {
                        // Java: CigarParser.java#L2052-L2058
                        if self.sv_structures.svrdel.is_empty()
                            || (position - self.sv_structures.svdelrend) as f64
                                > MINSVCDIST * self.max_read_length as f64
                        {
                            let mut sclip = Sclip::default();
                            sclip.base.vars_count = 0;
                            self.sv_structures.svrdel.push(sclip);
                        }
                        let svref = Self::get_last_sv_structure(&mut self.sv_structures.svrdel);
                        Self::add_sv_to(
                            svref,
                            position,
                            end,
                            mate_start,
                            mend,
                            read_dir_num,
                            total_length_including_soft_clipped,
                            mlen,
                            soft5,
                            self.max_read_length as f64 / 2.0,
                            qq_bytes[MINMAPBASE as usize] as i32 - 33,
                            record.mapq() as i32,
                            number_of_mismatches,
                        );
                        self.sv_structures.svdelrend = end;
                    }

                    // Java: CigarParser.java#L2063-L2082 — disc count updates
                    if !self.sv_structures.svfdel.is_empty()
                        && (position - self.sv_structures.svdelfend).abs()
                            <= (MINSVCDIST * self.max_read_length as f64) as i32
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svfdel,
                        ));
                    }
                    if !self.sv_structures.svrdel.is_empty()
                        && (position - self.sv_structures.svdelrend).abs()
                            <= (MINSVCDIST * self.max_read_length as f64) as i32
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svrdel,
                        ));
                    }
                    if !self.sv_structures.svfdup.is_empty()
                        && (position - self.sv_structures.svdupfend).abs() <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svfdup,
                        ));
                    }
                    if !self.sv_structures.svrdup.is_empty()
                        && (position - self.sv_structures.svduprend).abs() <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svrdup,
                        ));
                    }
                    if !self.sv_structures.svfinv5.is_empty()
                        && (position - self.sv_structures.svinvfend5).abs() <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svfinv5,
                        ));
                    }
                    if !self.sv_structures.svrinv5.is_empty()
                        && (position - self.sv_structures.svinvrend5).abs() <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svrinv5,
                        ));
                    }
                    if !self.sv_structures.svfinv3.is_empty()
                        && (position - self.sv_structures.svinvfend3).abs() <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svfinv3,
                        ));
                    }
                    if !self.sv_structures.svrinv3.is_empty()
                        && (position - self.sv_structures.svinvrend3).abs() <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svrinv3,
                        ));
                    }
                }
            } else if read_dir_num * mate_dir_num == -1
                && read_dir_num * mlen < 0
                && qq_bytes.len() as i32 > MINMAPBASE
            {
                // ── Duplication candidate ──
                // Java: CigarParser.java#L2088-L2118
                if read_dir_num == 1 {
                    if self.sv_structures.svfdup.is_empty()
                        || (position - self.sv_structures.svdupfend) as f64
                            > MINSVCDIST * self.max_read_length as f64
                    {
                        let mut sclip = Sclip::default();
                        sclip.base.vars_count = 0;
                        self.sv_structures.svfdup.push(sclip);
                    }
                    let svref = Self::get_last_sv_structure(&mut self.sv_structures.svfdup);
                    Self::add_sv_to(
                        svref,
                        position,
                        end,
                        mate_start,
                        mend,
                        read_dir_num,
                        total_length_including_soft_clipped,
                        mlen,
                        soft3,
                        self.max_read_length as f64 / 2.0,
                        qq_bytes[MINMAPBASE as usize] as i32 - 33,
                        record.mapq() as i32,
                        number_of_mismatches,
                    );
                    self.sv_structures.svdupfend = end;
                } else {
                    if self.sv_structures.svrdup.is_empty()
                        || (position - self.sv_structures.svduprend) as f64
                            > MINSVCDIST * self.max_read_length as f64
                    {
                        let mut sclip = Sclip::default();
                        sclip.base.vars_count = 0;
                        self.sv_structures.svrdup.push(sclip);
                    }
                    let svref = Self::get_last_sv_structure(&mut self.sv_structures.svrdup);
                    Self::add_sv_to(
                        svref,
                        position,
                        end,
                        mate_start,
                        mend,
                        read_dir_num,
                        total_length_including_soft_clipped,
                        mlen,
                        soft5,
                        self.max_read_length as f64 / 2.0,
                        qq_bytes[MINMAPBASE as usize] as i32 - 33,
                        record.mapq() as i32,
                        number_of_mismatches,
                    );
                    self.sv_structures.svduprend = end;
                }
                // Java: CigarParser.java#L2121-L2142 — disc count
                if !self.sv_structures.svfdup.is_empty()
                    && (position - self.sv_structures.svdupfend).abs()
                        <= (MINSVCDIST * self.max_read_length as f64) as i32
                {
                    Self::get_last_sv_structure(&mut self.sv_structures.svfdup).disc += 1;
                }
                if !self.sv_structures.svrdup.is_empty()
                    && (position - self.sv_structures.svduprend).abs()
                        <= (MINSVCDIST * self.max_read_length as f64) as i32
                {
                    Self::get_last_sv_structure(&mut self.sv_structures.svrdup).disc += 1;
                }
                if !self.sv_structures.svfdel.is_empty()
                    && (position - self.sv_structures.svdelfend).abs() <= min_d
                {
                    Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svfdel));
                }
                if !self.sv_structures.svrdel.is_empty()
                    && (position - self.sv_structures.svdelrend).abs() <= min_d
                {
                    Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svrdel));
                }
                if !self.sv_structures.svfinv5.is_empty()
                    && (position - self.sv_structures.svinvfend5).abs() <= min_d
                {
                    Self::add_disc_cnt(Self::get_last_sv_structure(
                        &mut self.sv_structures.svfinv5,
                    ));
                }
                if !self.sv_structures.svrinv5.is_empty()
                    && (position - self.sv_structures.svinvrend5).abs() <= min_d
                {
                    Self::add_disc_cnt(Self::get_last_sv_structure(
                        &mut self.sv_structures.svrinv5,
                    ));
                }
                if !self.sv_structures.svfinv3.is_empty()
                    && (position - self.sv_structures.svinvfend3).abs() <= min_d
                {
                    Self::add_disc_cnt(Self::get_last_sv_structure(
                        &mut self.sv_structures.svfinv3,
                    ));
                }
                if !self.sv_structures.svrinv3.is_empty()
                    && (position - self.sv_structures.svinvrend3).abs() <= min_d
                {
                    Self::add_disc_cnt(Self::get_last_sv_structure(
                        &mut self.sv_structures.svrinv3,
                    ));
                }
            } else if read_dir_num * mate_dir_num == 1 && qq_bytes.len() as i32 > MINMAPBASE {
                // ── Inversion candidate ──
                // Java: CigarParser.java#L2143-L2198
                if read_dir_num == 1 && mlen != 0 {
                    if mlen < -3 * self.max_read_length {
                        // Java: CigarParser.java#L2146-L2155
                        if self.sv_structures.svfinv3.is_empty()
                            || (position - self.sv_structures.svinvfend3) as f64
                                > MINSVCDIST * self.max_read_length as f64
                        {
                            let mut sclip = Sclip::default();
                            sclip.base.vars_count = 0;
                            self.sv_structures.svfinv3.push(sclip);
                        }
                        let svref = Self::get_last_sv_structure(&mut self.sv_structures.svfinv3);
                        Self::add_sv_to(
                            svref,
                            position,
                            end,
                            mate_start,
                            mend,
                            read_dir_num,
                            total_length_including_soft_clipped,
                            mlen,
                            soft3,
                            self.max_read_length as f64 / 2.0,
                            qq_bytes[MINMAPBASE as usize] as i32 - 33,
                            record.mapq() as i32,
                            number_of_mismatches,
                        );
                        self.sv_structures.svinvfend3 = end;
                        Self::get_last_sv_structure(&mut self.sv_structures.svfinv3).disc += 1;
                    } else if mlen > 3 * self.max_read_length {
                        // Java: CigarParser.java#L2157-L2168
                        if self.sv_structures.svfinv5.is_empty()
                            || (position - self.sv_structures.svinvfend5) as f64
                                > MINSVCDIST * self.max_read_length as f64
                        {
                            let mut sclip = Sclip::default();
                            sclip.base.vars_count = 0;
                            self.sv_structures.svfinv5.push(sclip);
                        }
                        let svref = Self::get_last_sv_structure(&mut self.sv_structures.svfinv5);
                        Self::add_sv_to(
                            svref,
                            position,
                            end,
                            mate_start,
                            mend,
                            read_dir_num,
                            total_length_including_soft_clipped,
                            mlen,
                            soft3,
                            self.max_read_length as f64 / 2.0,
                            qq_bytes[MINMAPBASE as usize] as i32 - 33,
                            record.mapq() as i32,
                            number_of_mismatches,
                        );
                        self.sv_structures.svinvfend5 = end;
                        Self::get_last_sv_structure(&mut self.sv_structures.svfinv5).disc += 1;
                    }
                } else if mlen != 0 {
                    if mlen < -3 * self.max_read_length {
                        // Java: CigarParser.java#L2171-L2183
                        if self.sv_structures.svrinv3.is_empty()
                            || (position - self.sv_structures.svinvrend3) as f64
                                > MINSVCDIST * self.max_read_length as f64
                        {
                            let mut sclip = Sclip::default();
                            sclip.base.vars_count = 0;
                            self.sv_structures.svrinv3.push(sclip);
                        }
                        let svref = Self::get_last_sv_structure(&mut self.sv_structures.svrinv3);
                        Self::add_sv_to(
                            svref,
                            position,
                            end,
                            mate_start,
                            mend,
                            read_dir_num,
                            total_length_including_soft_clipped,
                            mlen,
                            soft5,
                            self.max_read_length as f64 / 2.0,
                            qq_bytes[MINMAPBASE as usize] as i32 - 33,
                            record.mapq() as i32,
                            number_of_mismatches,
                        );
                        self.sv_structures.svinvrend3 = end;
                        Self::get_last_sv_structure(&mut self.sv_structures.svrinv3).disc += 1;
                    } else if mlen > 3 * self.max_read_length {
                        // Java: CigarParser.java#L2186-L2197
                        if self.sv_structures.svrinv5.is_empty()
                            || (position - self.sv_structures.svinvrend5) as f64
                                > MINSVCDIST * self.max_read_length as f64
                        {
                            let mut sclip = Sclip::default();
                            sclip.base.vars_count = 0;
                            self.sv_structures.svrinv5.push(sclip);
                        }
                        let svref = Self::get_last_sv_structure(&mut self.sv_structures.svrinv5);
                        Self::add_sv_to(
                            svref,
                            position,
                            end,
                            mate_start,
                            mend,
                            read_dir_num,
                            total_length_including_soft_clipped,
                            mlen,
                            soft5,
                            self.max_read_length as f64 / 2.0,
                            qq_bytes[MINMAPBASE as usize] as i32 - 33,
                            record.mapq() as i32,
                            number_of_mismatches,
                        );
                        self.sv_structures.svinvrend5 = end;
                        Self::get_last_sv_structure(&mut self.sv_structures.svrinv5).disc += 1;
                    }
                }
                // Java: CigarParser.java#L2200-L2211 — disc count for inv
                if mlen != 0 {
                    if !self.sv_structures.svfdel.is_empty()
                        && (position - self.sv_structures.svdelfend) <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svfdel,
                        ));
                    }
                    if !self.sv_structures.svrdel.is_empty()
                        && (position - self.sv_structures.svdelrend) <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svrdel,
                        ));
                    }
                    if !self.sv_structures.svfdup.is_empty()
                        && (position - self.sv_structures.svdupfend) <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svfdup,
                        ));
                    }
                    if !self.sv_structures.svrdup.is_empty()
                        && (position - self.sv_structures.svduprend) <= min_d
                    {
                        Self::add_disc_cnt(Self::get_last_sv_structure(
                            &mut self.sv_structures.svrdup,
                        ));
                    }
                }
            }
        } else if qq_bytes.len() as i32 > MINMAPBASE {
            // ── Inter-chr translocation ──────────────────────────────
            // Java: CigarParser.java#L2215-L2260
            let mchr = get_mate_reference_name(record, header);

            // Filter MC soft-clipped
            let mc_tag: Option<String> = record.aux(b"MC").ok().and_then(|a| match a {
                Aux::String(s) => Some(s.to_string()),
                _ => None,
            });
            if let Some(ref mc) = mc_tag {
                if MC_Z_NUM_S_ANY_NUM_S.is_match(mc) {
                    return;
                }
            }
            // Filter low MQ mates
            let mq_tag: Option<i32> = record.aux(b"MQ").ok().and_then(|a| Self::aux_to_i32(&a));
            if let Some(mq) = mq_tag {
                if mq < 15 {
                    return;
                }
            }

            if read_dir_num == 1 {
                // Java: CigarParser.java#L2221-L2234
                let need_new = self.sv_structures.svffus.get(&mchr).is_none()
                    || (position - *self.sv_structures.svfusfend.get(&mchr).unwrap_or(&0)) as f64
                        > MINSVCDIST * self.max_read_length as f64;
                if need_new {
                    let sclips = self.sv_structures.svffus.entry(mchr.clone()).or_default();
                    let mut sclip = Sclip::default();
                    sclip.base.vars_count = 0;
                    sclips.push(sclip);
                }
                let svn = self.sv_structures.svffus.get(&mchr).unwrap().len() - 1;
                let svref = &mut self.sv_structures.svffus.get_mut(&mchr).unwrap()[svn];
                Self::add_sv_to(
                    svref,
                    position,
                    end,
                    mate_start,
                    mend,
                    read_dir_num,
                    total_length_including_soft_clipped,
                    0,
                    soft3,
                    self.max_read_length as f64 / 2.0,
                    qq_bytes[MINMAPBASE as usize] as i32 - 33,
                    record.mapq() as i32,
                    number_of_mismatches,
                );
                self.sv_structures.svfusfend.insert(mchr.clone(), end);
                self.sv_structures.svffus.get_mut(&mchr).unwrap()[svn].disc += 1;
            } else {
                // Java: CigarParser.java#L2237-L2250
                let need_new = self.sv_structures.svrfus.get(&mchr).is_none()
                    || (position - *self.sv_structures.svfusrend.get(&mchr).unwrap_or(&0)) as f64
                        > MINSVCDIST * self.max_read_length as f64;
                if need_new {
                    let sclips = self.sv_structures.svrfus.entry(mchr.clone()).or_default();
                    let mut sclip = Sclip::default();
                    sclip.base.vars_count = 0;
                    sclips.push(sclip);
                }
                let svn = self.sv_structures.svrfus.get(&mchr).unwrap().len() - 1;
                let svref = &mut self.sv_structures.svrfus.get_mut(&mchr).unwrap()[svn];
                Self::add_sv_to(
                    svref,
                    position,
                    end,
                    mate_start,
                    mend,
                    read_dir_num,
                    total_length_including_soft_clipped,
                    0,
                    soft5,
                    self.max_read_length as f64 / 2.0,
                    qq_bytes[MINMAPBASE as usize] as i32 - 33,
                    record.mapq() as i32,
                    number_of_mismatches,
                );
                self.sv_structures.svfusrend.insert(mchr.clone(), end);
                self.sv_structures.svrfus.get_mut(&mchr).unwrap()[svn].disc += 1;
            }

            // Java: CigarParser.java#L2253-L2270 — disc count for translocation
            if !self.sv_structures.svfdel.is_empty()
                && position - self.sv_structures.svdelfend <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svfdel));
            }
            if !self.sv_structures.svrdel.is_empty()
                && position - self.sv_structures.svdelrend <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svrdel));
            }
            if !self.sv_structures.svfdup.is_empty()
                && position - self.sv_structures.svdupfend <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svfdup));
            }
            if !self.sv_structures.svrdup.is_empty()
                && position - self.sv_structures.svduprend <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svrdup));
            }
            if !self.sv_structures.svfinv5.is_empty()
                && position - self.sv_structures.svinvfend5 <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svfinv5));
            }
            if !self.sv_structures.svrinv5.is_empty()
                && position - self.sv_structures.svinvrend5 <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svrinv5));
            }
            if !self.sv_structures.svfinv3.is_empty()
                && position - self.sv_structures.svinvfend3 <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svfinv3));
            }
            if !self.sv_structures.svrinv3.is_empty()
                && position - self.sv_structures.svinvrend3 <= MINSVPOS
            {
                Self::add_disc_cnt(Self::get_last_sv_structure(&mut self.sv_structures.svrinv3));
            }
        }
    }

    /// Helper to call add_sv with i32 quality/nm values.
    /// Note: standalone function (not &self) to avoid borrow conflicts.
    #[allow(clippy::too_many_arguments)]
    fn add_sv_to(
        sdref: &mut Sclip,
        start_s: i32,
        end_e: i32,
        mate_start_ms: i32,
        mate_end_me: i32,
        dir: i32,
        rlen: i32,
        mlen: i32,
        softp: i32,
        pmean_rp: f64,
        qmean_i32: i32,
        q_mean_i32: i32,
        nm: i32,
    ) {
        Self::add_sv(
            sdref,
            start_s,
            end_e,
            mate_start_ms,
            mate_end_me,
            dir,
            rlen,
            mlen,
            softp,
            pmean_rp,
            qmean_i32 as f64,
            q_mean_i32 as f64,
            nm as f64,
        );
    }

    /// Ported from: CigarParser.java:L1710-L1776 (addVariationForMatchingPart)
    /// Creates/updates Variation for variants found in M segments (SNPs, MNVs, complex).
    fn add_variation_for_matching_part(
        &mut self,
        mapping_quality: i32,
        number_of_mismatches: i32,
        direction: bool,
        read_length_include_matching_and_insertions: i32,
        nmoff: i32,
        s: &str,
        start_with_deletion: bool,
        mut q: f64,
        qbases: i32,
        qibases: i32,
        ddlen: i32,
        pos: i32,
        goodq: f64,
    ) {
        let s_bytes = s.as_bytes();
        let is_insertion = s_bytes.first() == Some(&b'+');
        let has_ampersand = s_bytes.contains(&b'&');
        // Java: CigarParser.java#L1689-L1699
        let hv: &mut Variation = if is_insertion {
            // Java: CigarParser.java#L1690
            Self::increment(&mut self.position_to_insertion_count, pos, s);
            get_variation(&mut self.insertion_variants, pos, s)
        } else {
            // Java: CigarParser.java#L1692-L1696
            if Self::is_begin_atgc_amp_atgcs_end(s) {
                Self::increment(&mut self.mnp, pos, s);
            }
            get_variation(&mut self.non_insertion_variants, pos, s)
        };
        // Java: CigarParser.java#L1698
        hv.inc_dir(direction);
        hv.vars_count += 1;

        // Java: CigarParser.java#L1704-L1706
        let rlen1 = read_length_include_matching_and_insertions;
        let tp: i32 = if self.read_position_excluding_soft_clipped
            < rlen1 - self.read_position_excluding_soft_clipped
        {
            self.read_position_excluding_soft_clipped + 1
        } else {
            rlen1 - self.read_position_excluding_soft_clipped
        };

        // Java: CigarParser.java#L1709 — average quality
        q /= (qbases + qibases) as f64;

        // Java: CigarParser.java#L1712-L1713
        if !hv.pstd && hv.pp != 0 && tp != hv.pp {
            hv.pstd = true;
        }
        // Java: CigarParser.java#L1716-L1717
        if !hv.qstd && hv.pq != 0.0 && q != hv.pq {
            hv.qstd = true;
        }
        // Java: CigarParser.java#L1718-L1723
        hv.mean_position += tp as f64;
        hv.mean_quality += q;
        hv.mean_mapping_quality += mapping_quality as f64;
        hv.pp = tp;
        hv.pq = q;
        hv.number_of_mismatches += (number_of_mismatches - nmoff) as f64;
        // Java: CigarParser.java#L1724-L1728
        if q >= goodq {
            hv.high_quality_reads_count += 1;
        } else {
            hv.low_quality_reads_count += 1;
        }

        // Java: CigarParser.java#L1729
        let shift: i32 = if is_insertion && has_ampersand { 1 } else { 0 };

        // Java: CigarParser.java#L1732-L1734 — increase coverage
        for qi in 1..=(qbases - shift) {
            inc_cnt(&mut self.ref_coverage, self.start - qi + 1, 1);
        }

        // Java: CigarParser.java#L1737-L1743 — deletion coverage
        if start_with_deletion {
            Self::increment(&mut self.position_to_deletion_count, pos, s);
            for qi in 1..ddlen {
                inc_cnt(&mut self.ref_coverage, self.start + qi, 1);
            }
        }
    }

    /// Ported from: CigarParser.java:L1781-L1835 (addVariationForDeletion)
    /// Creates/updates Variation for deletion variants.
    fn add_variation_for_deletion(
        &mut self,
        mapping_quality: i32,
        number_of_mismatches: i32,
        direction: bool,
        read_length_include_matching_and_insertions: i32,
        desc_string: &str,
        quality_of_segment: &str,
        nmoff: i32,
    ) {
        let goodq = Self::with_conf(|conf| conf.goodq);

        // Java: CigarParser.java#L1786 — add variant structure for deletion
        let hv = get_variation(&mut self.non_insertion_variants, self.start, desc_string);
        // Java: CigarParser.java#L1788 — add record for deletion in deletions map
        Self::increment(
            &mut self.position_to_deletion_count,
            self.start,
            desc_string,
        );
        // Java: CigarParser.java#L1789
        hv.inc_dir(direction);
        // Java: CigarParser.java#L1791
        hv.vars_count += 1;

        // Java: CigarParser.java#L1794-L1796 — min of positions from start/end of read
        let rlen1 = read_length_include_matching_and_insertions;
        let tp: i32 = if self.read_position_excluding_soft_clipped
            < rlen1 - self.read_position_excluding_soft_clipped
        {
            self.read_position_excluding_soft_clipped + 1
        } else {
            rlen1 - self.read_position_excluding_soft_clipped
        };

        // Java: CigarParser.java#L1799-L1802 — average quality of bases
        let qual_bytes = quality_of_segment.as_bytes();
        let mut tmpq: f64 = 0.0;
        for &b in qual_bytes {
            tmpq += (b as i32 - 33) as f64;
        }
        tmpq /= qual_bytes.len() as f64;

        // Java: CigarParser.java#L1805-L1806
        if !hv.pstd && hv.pp != 0 && tp != hv.pp {
            hv.pstd = true;
        }
        // Java: CigarParser.java#L1809-L1810
        if !hv.qstd && hv.pq != 0.0 && tmpq != hv.pq {
            hv.qstd = true;
        }
        // Java: CigarParser.java#L1811-L1816
        hv.mean_position += tp as f64;
        hv.mean_quality += tmpq;
        hv.mean_mapping_quality += mapping_quality as f64;
        hv.pp = tp;
        hv.pq = tmpq;
        // Java: CigarParser.java#L1817
        hv.number_of_mismatches += (number_of_mismatches - nmoff) as f64;
        // Java: CigarParser.java#L1818-L1822
        if tmpq >= goodq {
            hv.high_quality_reads_count += 1;
        } else {
            hv.low_quality_reads_count += 1;
        }

        // Java: CigarParser.java#L1825-L1827 — increase coverage for deleted bases
        for i in 0..self.cigar_element_length {
            inc_cnt(&mut self.ref_coverage, self.start + i, 1);
        }
    }

    /// Ported from: CigarParser.java:L1502-L1547 (findOffset)
    /// Scans forward from an indel for nearby mismatches to combine into complex variant.
    /// Note: standalone — does not need &self, reads only global config.
    fn find_offset(
        reference_position: i32,
        read_position: i32,
        cigar_length: i32,
        query_sequence: &str,
        query_quality: &str,
        reference: &ReferenceSequenceMap,
        ref_coverage: &mut PositionMap<i32>,
    ) -> Offset {
        let (vext, goodq) = Self::with_conf(|conf| (conf.vext, conf.goodq));
        let mut offset = 0i32;
        let mut ss = String::new();
        let mut q = String::new();
        let mut tnm = 0i32;
        let mut vsn = 0i32;
        let query_seq_bytes = query_sequence.as_bytes();
        let query_qual_bytes = query_quality.as_bytes();
        // Java: CigarParser.java#L1515
        let mut vi = 0i32;
        while vsn <= vext && vi < cigar_length {
            let rp = (read_position + vi) as usize;
            // Java: CigarParser.java#L1516
            if rp < query_seq_bytes.len() && query_seq_bytes[rp] == b'N' {
                break;
            }
            // Java: CigarParser.java#L1519
            if rp < query_qual_bytes.len() && ((query_qual_bytes[rp] as i32 - 33) as f64) < goodq {
                break;
            }
            // Java: CigarParser.java#L1522
            let ref_ch = reference.get(&(reference_position + vi));
            if let Some(&ref_base) = ref_ch {
                let ch = if rp < query_seq_bytes.len() {
                    query_seq_bytes[rp]
                } else {
                    break;
                };
                // Java: CigarParser.java#L1524
                if is_not_equals(Some(ch), Some(ref_base)) {
                    offset = vi + 1;
                    tnm += 1;
                    vsn = 0;
                } else {
                    vsn += 1;
                }
            }
            vi += 1;
        }
        // Java: CigarParser.java#L1534
        if offset > 0 {
            let rp = read_position as usize;
            let off = offset as usize;
            if rp + off <= query_seq_bytes.len() {
                ss = String::from_utf8_lossy(&query_seq_bytes[rp..rp + off]).into_owned();
            }
            if rp + off <= query_qual_bytes.len() {
                q = String::from_utf8_lossy(&query_qual_bytes[rp..rp + off]).into_owned();
            }
            // Java: CigarParser.java#L1537
            for osi in 0..offset {
                inc_cnt(ref_coverage, reference_position + osi, 1);
            }
        }
        Offset::new(offset, ss, q, tnm)
    }

    /// Ported from: CigarParser.java:L1429-L1445 (addCnt)
    /// Increments variant counters.
    /// Note: does not need &self — reads only global config.
    fn add_cnt(
        variation: &mut Variation,
        direction: bool,
        read_position: i32,
        base_quality: f64,
        mapping_base_quality: i32,
        number_of_mismatches: i32,
    ) {
        // Java: CigarParser.java#L1430
        variation.vars_count += 1;
        // Java: CigarParser.java#L1431
        variation.inc_dir(direction);
        // Java: CigarParser.java#L1432
        variation.mean_position += read_position as f64;
        // Java: CigarParser.java#L1433
        variation.mean_quality += base_quality;
        // Java: CigarParser.java#L1434
        variation.mean_mapping_quality += mapping_base_quality as f64;
        // Java: CigarParser.java#L1435
        variation.number_of_mismatches += number_of_mismatches as f64;
        // Java: CigarParser.java#L1436-L1440
        let goodq = GlobalReadOnlyScope::with_instance(|scope| scope.conf.goodq);
        if base_quality >= goodq {
            variation.high_quality_reads_count += 1;
        } else {
            variation.low_quality_reads_count += 1;
        }
    }

    /// Ported from: CigarParser.java:L1406-L1424 (subCnt)
    /// Decrements variant counters.
    /// Note: does not need &self — reads only global config.
    fn sub_cnt(
        variation: &mut Variation,
        direction: bool,
        read_position: i32,
        base_quality: f64,
        mapping_base_quality: i32,
        number_of_mismatches: i32,
    ) {
        // Java: CigarParser.java#L1410
        variation.vars_count -= 1;
        // Java: CigarParser.java#L1411
        variation.dec_dir(direction);
        // Java: CigarParser.java#L1412
        variation.mean_position -= read_position as f64;
        // Java: CigarParser.java#L1413
        variation.mean_quality -= base_quality;
        // Java: CigarParser.java#L1414
        variation.mean_mapping_quality -= mapping_base_quality as f64;
        // Java: CigarParser.java#L1415
        variation.number_of_mismatches -= number_of_mismatches as f64;
        // Java: CigarParser.java#L1416-L1420
        let goodq = GlobalReadOnlyScope::with_instance(|scope| scope.conf.goodq);
        if base_quality >= goodq {
            variation.high_quality_reads_count -= 1;
        } else {
            variation.low_quality_reads_count -= 1;
        }
    }

    /// Ported from: CigarParser.java:L1326-L1400 (parseCigarWithAmpCase)
    /// Amplicon-mode overlap/distance filtering. Returns true to skip the record.
    fn parse_cigar_with_amp_case(
        &self,
        record: &bam::Record,
        _header: &HeaderView,
        is_mate_reference_name_equal: bool,
    ) -> bool {
        // Java: CigarParser.java#L1318-L1327
        let amp_str = match GlobalReadOnlyScope::with_instance(|scope| {
            scope.amplicon_based_calling.clone()
        }) {
            Some(s) => s,
            None => return false,
        };
        let split: Vec<&str> = amp_str.split(':').collect();
        let (distance_to_amplicon, overlap_fraction) = {
            let d = split
                .get(0)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(10);
            let o = split
                .get(1)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.95);
            (d, o)
        };

        // Java: CigarParser.java#L1334
        let read_length_include_matched_and_deleted = Self::get_aligned_length(&self.cigar);
        let segstart_init = record.pos() as i32 + 1; // 1-based
        let segend_init = segstart_init + read_length_include_matched_and_deleted - 1;

        let num_elems = self.cigar.num_cigar_elements();
        // Java: CigarParser.java#L1338-L1370
        if num_elems > 0 && self.cigar.get_cigar_element(0).operator == CigarOp::S {
            // Java: CigarParser.java#L1340-L1344
            let ts1 = if segstart_init > self.region.start {
                segstart_init
            } else {
                self.region.start
            };
            let te1 = if segend_init < self.region.end {
                segend_init
            } else {
                self.region.end
            };
            if !((ts1 - te1).abs() as f64 / (segend_init - segstart_init) as f64 > overlap_fraction)
            {
                return true;
            }
        } else if num_elems > 0
            && self.cigar.get_cigar_element(num_elems - 1).operator == CigarOp::S
        {
            // Java: CigarParser.java#L1347-L1351
            let ts1 = if segstart_init > self.region.start {
                segstart_init
            } else {
                self.region.start
            };
            let te1 = if segend_init < self.region.end {
                segend_init
            } else {
                self.region.end
            };
            if !((te1 - ts1).abs() as f64 / (segend_init - segstart_init) as f64 > overlap_fraction)
            {
                return true;
            }
        } else {
            // Java: CigarParser.java#L1354-L1366
            let mut segstart = segstart_init;
            let mut segend = segend_init;
            if is_mate_reference_name_equal && record.insert_size() != 0 {
                if record.insert_size() > 0 {
                    segend = segstart + record.insert_size() as i32 - 1;
                } else {
                    segstart = record.mpos() as i32 + 1;
                    segend = (record.mpos() as i32 + 1) - record.insert_size() as i32 - 1;
                }
            }
            let ts1 = if segstart > self.region.start {
                segstart
            } else {
                self.region.start
            };
            let te1 = if segend < self.region.end {
                segend
            } else {
                self.region.end
            };
            if ((segstart - self.region.start).abs() > distance_to_amplicon
                || (segend - self.region.end).abs() > distance_to_amplicon)
                || ((ts1 - te1) as f64 / (segend - segstart) as f64).abs() <= overlap_fraction
            {
                return true;
            }
        }
        false
    }

    /// Ported from: CigarParser.java:L1895-L1907 (skipOverlappingReads)
    /// Skips reads to avoid double-counting in overlapping pairs.
    fn skip_overlapping_reads(
        &self,
        record: &bam::Record,
        header: &HeaderView,
        position: i32,
        direction: bool,
        mate_alignment_start: i32,
    ) -> bool {
        let (unique_mode_alignment_enabled, unique_mode_second_in_pair_enabled) =
            GlobalReadOnlyScope::with_instance(|scope| {
                (
                    scope.conf.unique_mode_alignment_enabled,
                    scope.conf.unique_mode_second_in_pair_enabled,
                )
            });
        // Java: CigarParser.java#L1896-L1898
        if unique_mode_alignment_enabled
            && self.is_paired_and_same_chromosome(record, header)
            && !direction
            && self.start >= mate_alignment_start
        {
            return true;
        }
        // Java: CigarParser.java#L1901-L1904
        if unique_mode_second_in_pair_enabled
            && (record.flags() & 0x80) != 0 // secondOfPairFlag
            && self.is_paired_and_same_chromosome(record, header)
            && self.is_reads_overlap(record, position, mate_alignment_start)
        {
            return true;
        }
        false
    }

    /// Ported from: CigarParser.java:L1914-L1926 (isReadsOverlap)
    /// Two-case overlap test.
    fn is_reads_overlap(
        &self,
        record: &bam::Record,
        position: i32,
        mate_alignment_start: i32,
    ) -> bool {
        // Java: record.getCigar().getReferenceLength() — use original record cigar
        let record_cigar = cigar_from_record(record);
        if position >= mate_alignment_start {
            // Java: CigarParser.java#L1918-L1919
            self.start >= mate_alignment_start
                && self.start <= (mate_alignment_start + record_cigar.get_reference_length() - 1)
        } else {
            // Java: CigarParser.java#L1922-L1923
            // record.getMateAlignmentStart() = mate_alignment_start (already 1-based)
            // record.getAlignmentEnd() = pos + refLen (Java 1-based inclusive)
            let alignment_end = record.pos() as i32 + 1 + record_cigar.get_reference_length() - 1;
            self.start >= mate_alignment_start && mate_alignment_start <= alignment_end
        }
    }

    /// Ported from: CigarParser.java:L1932-L1934 (isPairedAndSameChromosome)
    /// Check if read is paired and same chromosome.
    fn is_paired_and_same_chromosome(&self, record: &bam::Record, header: &HeaderView) -> bool {
        // Java: record.getReadPairedFlag() && getMateReferenceName(record).equals("=")
        record.is_paired() && get_mate_reference_name(record, header) == "="
    }

    /// Ported from: CigarParser.java:L1969-L2014 (sclip5HighQualityProcessing)
    /// Populates 5' soft-clip Sclip structures.
    fn sclip5_high_quality_processing(
        &mut self,
        query_sequence: &str,
        mapping_quality: i32,
        query_quality: &str,
        number_of_mismatches: i32,
        direction: bool,
        sum_of_read_qualities: i32,
        number_of_high_quality_bases: i32,
        number_of_low_quality_bases: i32,
    ) {
        // Java: CigarParser.java#L1963
        if number_of_high_quality_bases >= 1
            && number_of_high_quality_bases > number_of_low_quality_bases
            && self.start >= self.region.start
            && self.start <= self.region.end
        {
            // Java: CigarParser.java#L1965-L1968
            let sclip = self
                .soft_clips_5_end
                .entry(self.start)
                .or_insert_with(Sclip::default);

            let qs_bytes = query_sequence.as_bytes();
            let qq_bytes = query_quality.as_bytes();

            // Java: CigarParser.java#L1970-L1985
            // Java: for (int si = cigarElementLength - 1; cigarElementLength - si <= numberOfHighQualityBases; si--)
            let mut si = self.cigar_element_length - 1;
            while si >= 0 && self.cigar_element_length - si <= number_of_high_quality_bases {
                let char_idx = si as usize;
                if char_idx >= qs_bytes.len() {
                    si -= 1;
                    continue;
                }
                let ch = [qs_bytes[char_idx]];
                let ch = std::str::from_utf8(&ch).expect("soft-clip base keys should be ASCII");
                let idx = self.cigar_element_length - 1 - si;

                // Java: CigarParser.java#L1974-L1978
                let nt_map = &mut sclip.nt;
                let cnts = nt_map.entry(idx).or_insert_with(SortedStringMap::new);
                inc_cnt_sorted_string_map(cnts, ch, 1);

                // Java: CigarParser.java#L1979
                let variation = get_variation_from_seq(sclip, idx, ch);
                let base_quality = if char_idx < qq_bytes.len() {
                    (qq_bytes[char_idx] as i32 - 33) as f64
                } else {
                    0.0
                };
                // Java: CigarParser.java#L1980-L1981
                // Java: addCnt(seqVariation, dir, si - (cigarElementLength - numberOfHighQualityBases), ...)
                Self::add_cnt(
                    variation,
                    direction,
                    si - (self.cigar_element_length - number_of_high_quality_bases),
                    base_quality,
                    mapping_quality,
                    number_of_mismatches,
                );
                si -= 1;
            }
            // Java: CigarParser.java#L1983-L1984
            let mean_q = sum_of_read_qualities as f64 / number_of_high_quality_bases as f64;
            Self::add_cnt(
                &mut sclip.base,
                direction,
                self.cigar_element_length,
                mean_q,
                mapping_quality,
                number_of_mismatches,
            );
        }
    }

    /// Ported from: CigarParser.java:L1935-L1967 (sclip3HighQualityProcessing)
    /// Populates 3' soft-clip Sclip structures.
    fn sclip3_high_quality_processing(
        &mut self,
        query_sequence: &str,
        mapping_quality: i32,
        query_quality: &str,
        number_of_mismatches: i32,
        direction: bool,
        sum_of_read_qualities: i32,
        number_of_high_quality_bases: i32,
        number_of_low_quality_bases: i32,
    ) {
        // Java: CigarParser.java#L1928-L1929
        if number_of_high_quality_bases >= 1
            && number_of_high_quality_bases > number_of_low_quality_bases
            && self.start >= self.region.start
            && self.start <= self.region.end
        {
            // Java: CigarParser.java#L1931-L1934
            let sclip = self
                .soft_clips_3_end
                .entry(self.start)
                .or_insert_with(Sclip::default);

            let qs_bytes = query_sequence.as_bytes();
            let qq_bytes = query_quality.as_bytes();

            // Java: CigarParser.java#L1935-L1949
            for si in 0..number_of_high_quality_bases {
                let char_idx = (self.read_position_including_soft_clipped + si) as usize;
                if char_idx >= qs_bytes.len() {
                    break;
                }
                let ch = [qs_bytes[char_idx]];
                let ch = std::str::from_utf8(&ch).expect("soft-clip base keys should be ASCII");
                let idx = si;

                // Java: CigarParser.java#L1938-L1942
                let nt_map = &mut sclip.nt;
                let cnts = nt_map.entry(idx).or_insert_with(SortedStringMap::new);
                inc_cnt_sorted_string_map(cnts, ch, 1);

                // Java: CigarParser.java#L1943
                let variation = get_variation_from_seq(sclip, idx, ch);
                let base_quality = if char_idx < qq_bytes.len() {
                    (qq_bytes[char_idx] as i32 - 33) as f64
                } else {
                    0.0
                };
                // Java: CigarParser.java#L1944-L1945
                Self::add_cnt(
                    variation,
                    direction,
                    number_of_high_quality_bases - si,
                    base_quality,
                    mapping_quality,
                    number_of_mismatches,
                );
            }
            // Java: CigarParser.java#L1947-L1948
            let mean_q = sum_of_read_qualities as f64 / number_of_high_quality_bases as f64;
            Self::add_cnt(
                &mut sclip.base,
                direction,
                self.cigar_element_length,
                mean_q,
                mapping_quality,
                number_of_mismatches,
            );
        }
    }

    /// Ported from: CigarParser.java:L2319-L2377 (addSV)
    /// Adds a single SV observation to an Sclip accumulator.
    /// Note: standalone function (not &self) to avoid borrow conflicts in prepare_sv_structures_for_analysis.
    #[allow(clippy::too_many_arguments)]
    fn add_sv(
        sdref: &mut Sclip,
        start_s: i32,
        end_e: i32,
        mate_start_ms: i32,
        mate_end_me: i32,
        dir: i32,
        rlen: i32,
        mlen: i32,
        softp: i32,
        pmean_rp: f64,
        qmean: f64,
        q_mean: f64,
        nm: f64,
    ) {
        let goodq = Self::with_conf(|conf| conf.goodq);
        // Java: CigarParser.java#L2307
        sdref.base.vars_count += 1;
        // Java: CigarParser.java#L2308
        sdref.base.inc_dir(dir != 1);

        // Java: CigarParser.java#L2310-L2314
        if qmean >= goodq {
            sdref.base.high_quality_reads_count += 1;
        } else {
            sdref.base.low_quality_reads_count += 1;
        }

        // Java: CigarParser.java#L2316-L2321
        if sdref.start == 0 || sdref.start >= start_s {
            sdref.start = start_s;
        }
        if sdref.end == 0 || sdref.end <= end_e {
            sdref.end = end_e;
        }

        // Java: CigarParser.java#L2323
        sdref.mates.push(Mate {
            mate_start_ms,
            mate_end_me,
            mate_length_mlen: mlen,
            start_s,
            end_e,
            pmean_rp,
            qmean_q: qmean,
            qmean_qq: q_mean,
            nm,
        });

        // Java: CigarParser.java#L2326-L2330
        if sdref.mstart == 0 || sdref.mstart >= mate_start_ms {
            sdref.mstart = mate_start_ms;
        }
        if sdref.mend == 0 || sdref.mend <= mate_end_me {
            sdref.mend = mate_start_ms + rlen;
        }

        // Java: CigarParser.java#L2332-L2346
        if softp != 0 {
            if dir == 1 {
                if (softp - sdref.end).abs() < 10 {
                    let soft_count = sdref.soft.entry(softp).or_insert(0);
                    *soft_count += 1;
                }
            } else {
                if (softp - sdref.start).abs() < 10 {
                    let soft_count = sdref.soft.entry(softp).or_insert(0);
                    *soft_count += 1;
                }
            }
        }
    }

    /// Ported from: CigarParser.java:L1841-L1873 (isReadChimericWithSA)
    /// Checks SA tag for chimeric reads.
    fn is_read_chimeric_with_sa(
        &self,
        record: &bam::Record,
        position: i32,
        sa_tag_string: &str,
        direction: bool,
        is_5_side: bool,
    ) -> bool {
        let y = Self::with_conf(|conf| conf.y);
        // Java: CigarParser.java#L1813 — SA tag format: "rname,pos,strand,cigar,mapq,nm;..."
        // Split by comma for first entry
        let sa_tag_array: Vec<&str> = sa_tag_string.split(',').collect();
        if sa_tag_array.len() < 4 {
            return false;
        }
        let sa_chromosome = sa_tag_array[0];
        let sa_position: i32 = sa_tag_array[1].parse().unwrap_or(0);
        let sa_direction_string = sa_tag_array[2];
        let sa_cigar = sa_tag_array[3];
        let sa_direction_is_forward = sa_direction_string == "+";

        // Java: CigarParser.java#L1821-L1824
        let mm = if is_5_side {
            SA_CIGAR_D_S_5clip.is_match(sa_cigar)
        } else {
            SA_CIGAR_D_S_3clip.is_match(sa_cigar)
        };

        // Java: CigarParser.java#L1826-L1829 — get reference name from region (avoids header lookup)
        let ref_name = &self.region.chr;
        let is_chimeric_with_sa = ((direction && sa_direction_is_forward)
            || (!direction && !sa_direction_is_forward))
            && sa_chromosome == ref_name.as_str()
            && (sa_position - position).abs() < 2 * self.max_read_length
            && mm;

        // Java: CigarParser.java#L1831-L1836
        if y && is_chimeric_with_sa {
            eprintln!(
                "{} {} {} {} {} is ignored as chimeric with SA: {},{},{}",
                String::from_utf8_lossy(record.qname()),
                ref_name,
                position,
                record.mapq(),
                self.cigar,
                sa_position,
                sa_direction_string,
                sa_cigar
            );
        }

        is_chimeric_with_sa
    }

    /// Ported from: CigarParser.java:L665-L677 (skipSitesOutRegionOfInterest)
    /// (Already fully implemented above as skip_sites_out_region_of_interest.)

    /// Ported from: CigarParser.java:L1878-L1904 (appendSegments)
    /// Builds complex description string for multi-indel cases.
    fn append_segments(
        &self,
        query_sequence: &str,
        query_quality: &str,
        ci: usize,
        desc_string: &mut String,
        quality_segment: &mut String,
        m_len: i32,
        indel_len: i32,
        begin: i32,
        is_insertion: bool,
    ) {
        let qs_bytes = query_sequence.as_bytes();
        let qq_bytes = query_quality.as_bytes();

        // Java: CigarParser.java#L1885 — append '#' + matched segment
        desc_string.push('#');
        desc_string.push_str(&String::from_utf8_lossy(&substr_with_len(
            qs_bytes, begin, m_len,
        )));
        // Java: CigarParser.java#L1887 — append quality of matched segment
        quality_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
            qq_bytes, begin, m_len,
        )));

        // Java: CigarParser.java#L1891-L1893 — append '^' + indel description
        let ci2_op = self.cigar.get_cigar_element(ci + 2).operator;
        desc_string.push('^');
        if ci2_op == CigarOp::I {
            desc_string.push_str(&String::from_utf8_lossy(&substr_with_len(
                qs_bytes,
                begin + m_len,
                indel_len,
            )));
        } else {
            desc_string.push_str(&indel_len.to_string());
        }

        // Java: CigarParser.java#L1897-L1904 — append quality for indel
        if is_insertion {
            if ci2_op == CigarOp::I {
                quality_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
                    qq_bytes,
                    begin + m_len,
                    indel_len,
                )));
            } else {
                // Java: queryQuality.charAt(begin + mLen)
                let idx = (begin + m_len) as usize;
                if idx < qq_bytes.len() {
                    quality_segment.push(qq_bytes[idx] as char);
                }
            }
        } else {
            // Deletion case
            if ci2_op == CigarOp::I {
                quality_segment.push_str(&String::from_utf8_lossy(&substr_with_len(
                    qq_bytes,
                    begin + m_len,
                    indel_len,
                )));
            }
            // else: append "" (nothing) for deletion case — Java: ""
        }
    }

    /// Ported from: CigarParser.java:L2311-L2313 (adddisccnt)
    /// Increments .disc on SV Sclip.
    fn add_disc_cnt(svref: &mut Sclip) {
        svref.disc += 1;
    }

    /// Ported from: CigarParser.java:L2315-L2317 (getLastSVStructure)
    /// Returns last element of SV list.
    fn get_last_sv_structure(sv_structure: &mut Vec<Sclip>) -> &mut Sclip {
        sv_structure.last_mut().expect("SV structure list is empty")
    }
}

// ─── adjInsPos standalone function ────────────────────────────────────────────

/// Ported from: VariationRealigner.java adjInsPos()
/// Left-aligns an insertion position by rotating the insertion sequence leftward
/// while the reference base matches the insertion's trailing base.
///
/// Algorithm: Walk bi left while reference[bi] == ins_bytes[ins.len() - n]
/// with wraparound of n. Rotate the insertion string. Return BaseInsertion.
///
/// Trap T14: reference.get(&bi) returning None terminates the loop
/// (Java compares null Character with autoboxed char — returns false).
pub fn adj_ins_pos(bi: i32, ins: &str, reference: &ReferenceSequenceMap) -> BaseInsertion {
    let ins_bytes = ins.as_bytes();
    let ins_len = ins_bytes.len();
    if ins_len == 0 {
        return BaseInsertion::new(bi, ins.to_string(), bi);
    }

    let mut bi = bi;
    let mut n = 0usize;
    loop {
        // Reference.get(&bi) returning None terminates the loop
        let Some(&ref_base) = reference.get(&bi) else {
            break;
        };
        // Compare with ins_bytes wrapping from end
        let ins_idx = if n == 0 {
            ins_len - 1
        } else {
            ins_len - 1 - n % ins_len
        };
        if ref_base != ins_bytes[ins_idx] {
            break;
        }
        bi -= 1;
        n += 1;
    }

    // Rotate the insertion string if we moved
    let rotated = if n > 0 && ins_len > 0 {
        let rot = n % ins_len;
        if rot == 0 {
            ins.to_string()
        } else {
            let split_pos = ins_len - rot;
            format!("{}{}", &ins[split_pos..], &ins[..split_pos])
        }
    } else {
        ins.to_string()
    };

    BaseInsertion::new(bi, rotated, bi)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cigar_string_basic() {
        let cigar = parse_cigar_string("10M5I3D");
        assert_eq!(cigar.num_cigar_elements(), 3);
        assert_eq!(cigar.get_cigar_element(0).length, 10);
        assert_eq!(cigar.get_cigar_element(0).operator, CigarOp::M);
        assert_eq!(cigar.get_cigar_element(1).length, 5);
        assert_eq!(cigar.get_cigar_element(1).operator, CigarOp::I);
        assert_eq!(cigar.get_cigar_element(2).length, 3);
        assert_eq!(cigar.get_cigar_element(2).operator, CigarOp::D);
    }

    #[test]
    fn test_parse_cigar_string_all_ops() {
        let cigar = parse_cigar_string("3S10M2I4D1N5H2P3=2X");
        assert_eq!(cigar.num_cigar_elements(), 9);
        assert_eq!(cigar.get_cigar_element(0).operator, CigarOp::S);
        assert_eq!(cigar.get_cigar_element(1).operator, CigarOp::M);
        assert_eq!(cigar.get_cigar_element(2).operator, CigarOp::I);
        assert_eq!(cigar.get_cigar_element(3).operator, CigarOp::D);
        assert_eq!(cigar.get_cigar_element(4).operator, CigarOp::N);
        assert_eq!(cigar.get_cigar_element(5).operator, CigarOp::H);
        assert_eq!(cigar.get_cigar_element(6).operator, CigarOp::P);
        assert_eq!(cigar.get_cigar_element(7).operator, CigarOp::Eq);
        assert_eq!(cigar.get_cigar_element(8).operator, CigarOp::X);
    }

    #[test]
    fn test_cigar_display() {
        let cigar = parse_cigar_string("10M5I3D");
        assert_eq!(cigar.to_string(), "10M5I3D");
    }

    #[test]
    fn test_cleanup_cigar_removes_hard_clips() {
        let mut parser = CigarParser::new(false);
        parser.cigar = parse_cigar_string("5H10M3H");
        parser.cleanup_cigar();
        assert_eq!(parser.cigar.to_string(), "10M");
    }

    #[test]
    fn test_cleanup_cigar_converts_edge_insertions_to_soft_clips() {
        let mut parser = CigarParser::new(false);
        parser.cigar = parse_cigar_string("3I10M5I");
        parser.cleanup_cigar();
        assert_eq!(parser.cigar.to_string(), "3S10M5S");
    }

    #[test]
    fn test_cleanup_cigar_combined() {
        let mut parser = CigarParser::new(false);
        parser.cigar = parse_cigar_string("2H3I10M5I4H");
        parser.cleanup_cigar();
        assert_eq!(parser.cigar.to_string(), "3S10M5S");
    }

    #[test]
    fn test_cleanup_cigar_no_change() {
        let mut parser = CigarParser::new(false);
        parser.cigar = parse_cigar_string("5S10M3S");
        parser.cleanup_cigar();
        assert_eq!(parser.cigar.to_string(), "5S10M3S");
    }

    #[test]
    fn test_cleanup_cigar_would_change_minmatch_length_for_edge_insertions() {
        let raw = parse_cigar_string("3I10M5I");

        let mut parser = CigarParser::new(false);
        parser.cigar = raw.clone();
        parser.cleanup_cigar();

        assert_eq!(CigarParser::get_match_insertion_length(&raw), 18);
        assert_eq!(CigarParser::get_match_insertion_length(&parser.cigar), 10);
    }

    #[test]
    fn test_round_duplicate_rate_matches_java_half_up_examples() {
        assert_eq!(CigarParser::round_duplicate_rate(1, 8), 125.0 / 1_000.0);
        assert_eq!(
            CigarParser::round_duplicate_rate(889, 2_000),
            445.0 / 1_000.0
        );
        assert_eq!(
            CigarParser::round_duplicate_rate(1_111, 2_000),
            556.0 / 1_000.0
        );
        assert_eq!(CigarParser::round_duplicate_rate(3_998, 4_000), 1.0);
    }

    #[test]
    fn test_get_aligned_length() {
        let cigar = parse_cigar_string("10M5I3D2S");
        assert_eq!(CigarParser::get_aligned_length(&cigar), 13); // 10M + 3D
    }

    #[test]
    fn test_get_soft_clipped_length() {
        let cigar = parse_cigar_string("3S10M5I2S");
        assert_eq!(CigarParser::get_soft_clipped_length(&cigar), 20); // 3S + 10M + 5I + 2S
    }

    #[test]
    fn test_get_match_insertion_length() {
        let cigar = parse_cigar_string("10M5I3D2S");
        assert_eq!(CigarParser::get_match_insertion_length(&cigar), 15); // 10M + 5I
    }

    #[test]
    fn test_get_insertion_deletion_length() {
        let cigar = parse_cigar_string("10M5I3D2S");
        assert_eq!(CigarParser::get_insertion_deletion_length(&cigar), 8); // 5I + 3D
    }

    #[test]
    fn test_is_begin_atgc_amp_atgcs_end() {
        assert!(CigarParser::is_begin_atgc_amp_atgcs_end("A&TG"));
        assert!(CigarParser::is_begin_atgc_amp_atgcs_end("C&A"));
        assert!(CigarParser::is_begin_atgc_amp_atgcs_end("G&ATGC"));
        assert!(!CigarParser::is_begin_atgc_amp_atgcs_end("A&")); // no chars after &
        assert!(!CigarParser::is_begin_atgc_amp_atgcs_end("AA")); // no &
        assert!(!CigarParser::is_begin_atgc_amp_atgcs_end("A&N")); // N is not ATGC
        assert!(!CigarParser::is_begin_atgc_amp_atgcs_end("N&A")); // N at start
        assert!(!CigarParser::is_begin_atgc_amp_atgcs_end("A")); // too short
        assert!(!CigarParser::is_begin_atgc_amp_atgcs_end("")); // empty
    }

    #[test]
    fn test_is_atgc() {
        assert!(CigarParser::is_atgc(b'A'));
        assert!(CigarParser::is_atgc(b'T'));
        assert!(CigarParser::is_atgc(b'G'));
        assert!(CigarParser::is_atgc(b'C'));
        assert!(!CigarParser::is_atgc(b'N'));
        assert!(!CigarParser::is_atgc(b'a'));
        assert!(!CigarParser::is_atgc(b'X'));
    }

    #[test]
    fn test_get_cigar_operator_edge_insertion() {
        let mut parser = CigarParser::new(false);
        // Leading insertion should be treated as soft clip
        parser.cigar = parse_cigar_string("3I10M5I");
        assert_eq!(parser.get_cigar_operator(0), CigarOp::S);
        assert_eq!(parser.get_cigar_operator(1), CigarOp::M);
        assert_eq!(parser.get_cigar_operator(2), CigarOp::S);
    }

    #[test]
    fn test_get_cigar_operator_middle_insertion() {
        let mut parser = CigarParser::new(false);
        parser.cigar = parse_cigar_string("10M3I10M");
        assert_eq!(parser.get_cigar_operator(0), CigarOp::M);
        assert_eq!(parser.get_cigar_operator(1), CigarOp::I); // not edge → stays I
        assert_eq!(parser.get_cigar_operator(2), CigarOp::M);
    }

    #[test]
    fn test_process_not_matched() {
        let mut parser = CigarParser::new(false);
        parser.region = Region::new("chr1", 100, 200, "gene");
        parser.start = 150;
        parser.cigar_element_length = 1000;
        parser.process_not_matched();
        assert!(parser.splice.contains("149-1149"));
        assert_eq!(parser.splice_count.get("149-1149").unwrap()[0], 1);
        assert_eq!(parser.start, 1150);
        assert_eq!(parser.offset, 0);
    }

    #[test]
    fn test_adj_ins_pos_no_movement() {
        let mut ref_map = ReferenceSequenceMap::default();
        ref_map.insert(100, b'A');
        ref_map.insert(99, b'C');
        // insertion "TG" at position 100 — ref[100]=A != T (last of "TG"), no movement
        let result = adj_ins_pos(100, "TG", &ref_map);
        assert_eq!(result.base_insert, Some(100));
        assert_eq!(result.insertion_sequence, "TG");
    }

    mod pbt {
        use super::*;
        use proptest::prelude::*;
        use proptest::sample::select;
        use proptest::test_runner::Config as ProptestConfig;

        fn arb_cigar_string() -> impl Strategy<Value = String> {
            proptest::collection::vec(
                (
                    prop_oneof![1..1000i32, Just(1), Just(999)],
                    select(&['M', 'I', 'D', 'N', 'S', 'H', 'P', '=', 'X']),
                ),
                1..10,
            )
            .prop_map(|elements: Vec<(i32, char)>| {
                elements
                    .into_iter()
                    .map(|(length, operator)| format!("{length}{operator}"))
                    .collect::<String>()
            })
        }

        fn arb_cigar_op() -> impl Strategy<Value = CigarOp> {
            prop_oneof![
                Just(CigarOp::M),
                Just(CigarOp::I),
                Just(CigarOp::D),
                Just(CigarOp::N),
                Just(CigarOp::S),
                Just(CigarOp::H),
                Just(CigarOp::P),
                Just(CigarOp::Eq),
                Just(CigarOp::X),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 256,
                ..ProptestConfig::default()
            })]

            #[test]
            fn pbt_parse_cigar_roundtrip(cigar_string in arb_cigar_string()) {
                let parsed = parse_cigar_string(&cigar_string);

                prop_assert_eq!(parsed.to_string(), cigar_string);
            }

            #[test]
            fn pbt_get_reference_length_equals_ref_consuming_sum(cigar_string in arb_cigar_string()) {
                let parsed = parse_cigar_string(&cigar_string);
                let expected: i32 = parsed
                    .elements
                    .iter()
                    .filter(|element| element.operator.consumes_reference_bases())
                    .map(|element| element.length)
                    .sum();

                prop_assert_eq!(parsed.get_reference_length(), expected);
            }

            #[test]
            fn pbt_consumes_both_iff_read_and_ref(operator in arb_cigar_op()) {
                prop_assert_eq!(
                    operator.consumes_both(),
                    operator.consumes_read_bases() && operator.consumes_reference_bases()
                );
            }

            #[test]
            fn pbt_get_reference_length_non_negative(cigar_string in arb_cigar_string()) {
                let parsed = parse_cigar_string(&cigar_string);

                prop_assert!(parsed.get_reference_length() >= 0);
            }
        }
    }
}
