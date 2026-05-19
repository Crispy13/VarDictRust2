//! Ported from: SAMFileParser.java, RecordPreprocessor.java, SamView.java
//!
//! SAMFileParser is the BAM ingestion entry gate: it splits the BAM path string,
//! opens overlapping-query iterators, and streams filtered reads through
//! RecordPreprocessor's filter cascade to CigarParser.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use rust_htslib::bam::{self, HeaderView, Read as BamRead};

use crate::data::{InitialData, Region};
use crate::scope::{GlobalReadOnlyScope, Scope};

thread_local! {
    static SAM_VIEW_READERS: RefCell<HashMap<String, bam::IndexedReader>> =
        RefCell::new(HashMap::new());
}

const _: fn() = {
    fn check() {
        fn assert_send<T: Send>() {}

        assert_send::<bam::IndexedReader>();
    }

    check
};

// ─── Integer.decode() equivalent ──────────────────────────────────────────────
// Java: SamView constructor calls Integer.decode(samfilter) which handles
// hex (0x/0X/#), octal (leading 0), and decimal.
// Ported from: SamView.java:L23

/// Parse a SAM filter string using Java `Integer.decode()` semantics:
/// - `"0x..."` or `"0X..."` → hex
/// - `"#..."` → hex
/// - `"-0x..."` / `"-0X..."` / `"-#..."` → negative hex
/// - leading `"0"` (after optional sign) → octal
/// - otherwise → decimal
pub fn parse_samfilter(s: &str) -> u16 {
    // Java Integer.decode handles: optional minus, then 0x/0X/# prefix, then digits
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    let (negative, rest) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else if let Some(stripped) = s.strip_prefix('+') {
        (false, stripped)
    } else {
        (false, s)
    };

    let (radix, digits) =
        if let Some(stripped) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            (16, stripped)
        } else if let Some(stripped) = rest.strip_prefix('#') {
            (16, stripped)
        } else if rest.starts_with('0') && rest.len() > 1 {
            (8, &rest[1..])
        } else {
            (10, rest)
        };

    let value = i32::from_str_radix(digits, radix)
        .unwrap_or_else(|_| panic!("Cannot decode samfilter string: {}", s));

    let value = if negative { -value } else { value };
    value as u16
}

// ─── getMateReferenceName ─────────────────────────────────────────────────────
// Ported from: CigarParser.java:L1653-L1662
// Shared between RecordPreprocessor and CigarParser. Placed here as a common
// utility so both modules can use it.

/// Returns the mate reference name for a BAM record, applying Java VarDict
/// normalization:
/// - mate unmapped (mtid == -1) → `"*"`
/// - same chromosome (tid == mtid) → `"="`
/// - otherwise → actual mate reference name
pub fn get_mate_reference_name(record: &bam::Record, header: &HeaderView) -> String {
    let mtid = record.mtid();
    // Java: record.getMateReferenceName() == null → "*"
    // rust-htslib: mtid == -1 means mate has no reference
    if mtid < 0 {
        return "*".to_string();
    }

    let tid = record.tid();
    // Java: record.getReferenceName().equals(record.getMateReferenceName()) → "="
    if tid == mtid {
        return "=".to_string();
    }

    // Otherwise return actual mate reference name
    std::str::from_utf8(header.tid2name(mtid as u32))
        .unwrap_or("*")
        .to_string()
}

// ─── getChrName ───────────────────────────────────────────────────────────────
// Ported from: RecordPreprocessor.java:L219-L228

/// Fixes chromosome name. If option -C set, it will remove prefix "chr".
pub fn get_chr_name(region: &Region, conf: &crate::config::Configuration) -> String {
    // Java: if (instance().conf.chromosomeNameIsNumber && region.chr.startsWith("chr"))
    if conf.chromosome_name_is_number && region.chr.starts_with("chr") {
        region.chr["chr".len()..].to_string()
    } else {
        region.chr.clone()
    }
}

// ─── SAMFileParser.process() ─────────────────────────────────────────────────
// Ported from: SAMFileParser.java:L10-L16

/// Pipeline entry point — splits BAM path by `:`, creates RecordPreprocessor,
/// wraps in Scope.
pub fn sam_file_parser_process(scope: Scope<InitialData>) -> Scope<RecordPreprocessor> {
    let bams: Vec<String> = scope.bam.split(':').map(|s| s.to_string()).collect();
    let region = scope.region.clone();
    // Destructure scope to avoid partial-move issues
    let Scope {
        bam,
        region: scope_region,
        region_ref,
        reference_resource,
        max_read_length,
        splice,
        out,
        data,
    } = scope;
    let preprocessor = RecordPreprocessor::new(bams, region, data);
    Scope {
        bam,
        region: scope_region,
        region_ref,
        reference_resource,
        max_read_length,
        splice,
        out,
        data: preprocessor,
    }
}

// ─── RecordPreprocessor ──────────────────────────────────────────────────────
// Ported from: RecordPreprocessor.java:L20-L228

pub struct RecordPreprocessor {
    pub initial_data: InitialData,

    /// BAM paths — processed from back (pop) to preserve original array order.
    /// Java: ArrayDeque with push (reverses), then pollLast (restores order).
    /// Rust: Vec with push (appends), then pop (takes from back) — SAME reversal
    /// as Java push+pollLast when we reverse the input first.
    bams: Vec<String>,
    region: Region,
    pub total_reads: i32,
    pub duplicate_reads: i32,

    current_reader: Option<bam::IndexedReader>,
    current_bam_path: Option<String>,
    current_record: bam::Record,
    filter: u16,
    duplicates: HashSet<String>,
    first_matching_position: i32,
}

impl RecordPreprocessor {
    // Ported from: RecordPreprocessor.java:L44-L68

    pub fn new(bams: Vec<String>, region: Region, data: InitialData) -> Self {
        let conf = &GlobalReadOnlyScope::instance().conf;
        let filter = parse_samfilter(&conf.samfilter);

        // Java: ArrayDeque push() adds to head, pollLast() removes from tail.
        // Effect: BAMs are processed in original array order.
        // Rust: We reverse the list and use pop() to get original order.
        let mut bam_stack: Vec<String> = Vec::with_capacity(bams.len());
        for bam in &bams {
            // Java push() adds to front — we collect in reverse to pop from back
            bam_stack.insert(0, bam.clone());
        }

        let mut preprocessor = RecordPreprocessor {
            initial_data: data,
            bams: bam_stack,
            region,
            total_reads: 0,
            duplicate_reads: 0,
            current_reader: None,
            current_record: bam::Record::new(),
            filter,
            duplicates: HashSet::new(),
            first_matching_position: -1,
            current_bam_path: None,
        };

        // Java constructor calls nextReader() at end
        preprocessor.next_reader();
        preprocessor
    }

    // Ported from: RecordPreprocessor.java:L73-L79

    /// Public iterator API — returns next filtered SAMRecord across all BAM files.
    /// Returns `None` when all BAMs are exhausted.
    pub fn next_record(&mut self) -> Option<bam::Record> {
        if self.advance_on_current_reader() {
            return Some(self.current_record.clone());
        }
        // Current BAM exhausted — try next BAM
        self.next_reader();
        if self.advance_on_current_reader() {
            Some(self.current_record.clone())
        } else {
            None
        }
    }

    /// Calls `f` for each filtered record without cloning the reusable BAM record buffer.
    pub fn for_each_record(&mut self, mut f: impl FnMut(&bam::Record)) {
        loop {
            if self.advance_on_current_reader() {
                f(&self.current_record);
                continue;
            }
            self.next_reader();
            if !self.advance_on_current_reader() {
                break;
            }
            f(&self.current_record);
        }
    }

    // Ported from: RecordPreprocessor.java:L84-L97

    /// Opens next BAM from the deque, resets duplicate detection state.
    fn next_reader(&mut self) {
        // Java: if (bams.isEmpty()) return;
        if self.bams.is_empty() {
            return;
        }

        // Java closes the current SamView before opening the next one. Reusing the reader
        // preserves fetch/read semantics while avoiding repeated BAM index reloads.
        self.release_current_reader();

        // Java: bams.pollLast() — we use pop() since our Vec is reversed
        let bam_path = self.bams.pop().unwrap();

        let instance = GlobalReadOnlyScope::instance();
        let conf = &instance.conf;

        // Open IndexedReader — equivalent to SamView constructor
        // Ported from: SamView.java:L20-L24
        let mut reader = SAM_VIEW_READERS
            .with(|cached_readers| cached_readers.borrow_mut().remove(&bam_path))
            .unwrap_or_else(|| {
                bam::IndexedReader::from_path(&bam_path)
                    .unwrap_or_else(|e| panic!("Failed to open BAM file {}: {}", bam_path, e))
            });

        // Set thread count for decompression (performance, not parity-critical)
        reader.set_threads(1).ok();

        // Java: queryOverlapping(region.chr, region.start, region.end) — 1-based inclusive
        // rust-htslib fetch() with a &str region string uses samtools-style "chr:start-end"
        // which is 1-based inclusive, matching htsjdk's queryOverlapping() semantics.
        let region_str = format!(
            "{}:{}-{}",
            self.region.chr, self.region.start, self.region.end
        );
        reader.fetch(region_str.as_str()).unwrap_or_else(|e| {
            panic!(
                "Failed to fetch region {} from {}: {}",
                self.region.print_region(),
                bam_path,
                e
            )
        });

        // Validation stringency: rust-htslib doesn't have an exact equivalent.
        // Java uses LENIENT by default. rust-htslib is already lenient by default.
        // No action needed for parity.
        let _ = conf.validation_stringency; // Acknowledge the field exists

        self.current_reader = Some(reader);
        self.current_bam_path = Some(bam_path);

        // Java: duplicates = new HashSet<>()
        self.duplicates = HashSet::new();
        // Java: firstMatchingPosition = -1
        self.first_matching_position = -1;
    }

    fn release_current_reader(&mut self) {
        if let (Some(bam_path), Some(reader)) =
            (self.current_bam_path.take(), self.current_reader.take())
        {
            SAM_VIEW_READERS.with(|cached_readers| {
                cached_readers.borrow_mut().insert(bam_path, reader);
            });
        }
    }

    // Ported from: RecordPreprocessor.java:L103-L115

    /// Read records from current reader until one passes the filter cascade, or EOF.
    fn advance_on_current_reader(&mut self) -> bool {
        if self.current_reader.is_none() {
            return false;
        }

        // Reuse the record buffer for efficiency
        loop {
            // Read next record from the BAM iterator
            let reader = self.current_reader.as_mut().unwrap();
            match reader.read(&mut self.current_record) {
                Some(Ok(())) => {}
                Some(Err(_)) => continue, // skip malformed records (LENIENT behavior)
                None => return false,     // EOF
            }

            // Layer 1: SamView.read() flag filter
            // Ported from: SamView.java:L30-L39
            if self.filter != 0 && (self.current_record.flags() & self.filter) != 0 {
                continue;
            }

            // Layer 2: preprocessRecord() filter cascade
            if self.preprocess_record() {
                return true;
            }
        }
    }

    // Ported from: RecordPreprocessor.java:L131-L182

    /// Core read filter cascade. References `self.current_record`.
    /// Returns `true` if the record should be processed by CigarParser.
    fn preprocess_record(&mut self) -> bool {
        let (downsampling, mapping_quality_filter, samfilter_is_nonzero, remove_duplicated_reads) =
            GlobalReadOnlyScope::with_instance(|scope| {
                (
                    scope.conf.downsampling,
                    scope.conf.mapping_quality,
                    scope.conf.samfilter != "0",
                    scope.conf.remove_duplicated_reads,
                )
            });

        // Step 1 — Downsampling (Java: L132-L134)
        if let Some(threshold) = downsampling {
            if rand::random::<f64>() <= threshold {
                return false;
            }
        }

        // Step 2 — Extract read attributes (Java: L135-L136)
        // querySequence: Java getReadString() returns "*" for no-seq.
        // rust-htslib: seq().len() == 0 for no-seq records.
        let seq_len = self.current_record.seq().len();
        let mapping_quality = self.current_record.mapq();

        // Step 3 — Mapping quality filter (Java: L139-L141)
        if let Some(min_mapping_quality) = mapping_quality_filter {
            if (mapping_quality as i32) < min_mapping_quality {
                return false;
            }
        }

        // Step 4 — Secondary alignment filter (Java: L144-L146)
        // Java: record.isSecondaryAlignment() = (flags & 0x100) != 0
        // Java: !instance().conf.samfilter.equals("0") — STRING equality, not numeric
        let is_secondary = (self.current_record.flags() & 0x100) != 0;
        if is_secondary && samfilter_is_nonzero {
            return false;
        }

        // Step 5 — No-sequence filter (Java: L148-L150)
        // Java checks: querySequence.length() == 1 && querySequence.charAt(0) == '*'
        // rust-htslib: no-seq records have seq().len() == 0
        if seq_len == 0 {
            return false;
        }

        // Step 6 — Increment totalReads (Java: L151)
        // This MUST be after steps 1-5 and BEFORE dup detection.
        // Duplicates ARE counted in totalReads.
        self.total_reads += 1;

        // Step 7 — Get mate reference name (Java: L153)
        let mate_reference_name = {
            let header = self.current_reader.as_ref().unwrap().header();
            get_mate_reference_name(&self.current_record, header)
        };

        // Step 8 — Duplicate detection (Java: L156-L181)
        if remove_duplicated_reads {
            // 1-based alignment start (Java: getAlignmentStart() is 1-based)
            let alignment_start = self.current_record.pos() as i32 + 1;
            // 1-based mate alignment start (Java: getMateAlignmentStart() is 1-based)
            let mate_alignment_start = self.current_record.mpos() as i32 + 1;

            // Step 8a — Position change clears dup set (Java: L157-L159)
            if alignment_start != self.first_matching_position {
                self.duplicates.clear();
            }

            // Step 8b — Branch 1: mateAlignmentStart < 10 (Java: L160-L169)
            if mate_alignment_start < 10 {
                // POS-RNEXT-PNEXT
                let dup_key = format!(
                    "{}-{}-{}",
                    alignment_start, mate_reference_name, mate_alignment_start
                );
                if self.duplicates.contains(&dup_key) {
                    self.duplicate_reads += 1;
                    return false;
                }
                self.duplicates.insert(dup_key);
                self.first_matching_position = alignment_start;
            } else {
                let flags = self.current_record.flags();
                let is_paired = (flags & 0x1) != 0;
                let is_mate_unmapped = (flags & 0x8) != 0;

                // Step 8c — Branch 2: paired + mate unmapped (Java: L170-L179)
                if is_paired && is_mate_unmapped {
                    // POS-CIGAR
                    let cigar_string = self.current_record.cigar().to_string();
                    let dup_key = format!("{}-{}", alignment_start, cigar_string);
                    if self.duplicates.contains(&dup_key) {
                        self.duplicate_reads += 1;
                        return false;
                    }
                    self.duplicates.insert(dup_key);
                    self.first_matching_position = alignment_start;
                }
                // Step 8d — Branch 3 (implicit): no dup check, read passes through
            }
        }

        // Step 9 — Return true (Java: L182)
        true
    }

    // Ported from: RecordPreprocessor.java:L117-L125

    /// Close BAM reader. Keep the underlying htslib reader available for the
    /// next region on this thread so the BAM index is not reloaded per tile.
    pub fn close(&mut self) {
        self.release_current_reader();
    }

    /// Returns a clone of the current BAM header for downstream CIGAR parsing.
    pub fn header_view(&self) -> Option<HeaderView> {
        self.current_reader
            .as_ref()
            .map(|reader| reader.header().clone())
    }

    /// Returns the chromosome name, optionally stripping "chr" prefix if -C flag is set.
    /// Ported from: RecordPreprocessor.getChrName() (Java: L219-L228)
    pub fn get_chr_name(&self) -> String {
        let conf = &GlobalReadOnlyScope::instance().conf;
        get_chr_name(&self.region, conf)
    }
}

impl Drop for RecordPreprocessor {
    fn drop(&mut self) {
        self.release_current_reader();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_samfilter_hex() {
        assert_eq!(parse_samfilter("0x504"), 0x504);
    }

    #[test]
    fn test_parse_samfilter_hex_uppercase() {
        assert_eq!(parse_samfilter("0X504"), 0x504);
    }

    #[test]
    fn test_parse_samfilter_decimal() {
        assert_eq!(parse_samfilter("1284"), 0x504);
    }

    #[test]
    fn test_parse_samfilter_octal() {
        // Octal 02404 = 0x504 = 1284
        assert_eq!(parse_samfilter("02404"), 0x504);
    }

    #[test]
    fn test_parse_samfilter_zero() {
        assert_eq!(parse_samfilter("0"), 0);
    }

    #[test]
    fn test_parse_samfilter_hash_hex() {
        assert_eq!(parse_samfilter("#504"), 0x504);
    }

    #[test]
    fn test_parse_samfilter_plain_zero_string() {
        // "0" as a single character is not treated as octal prefix
        // (needs len > 1 for octal), so it's decimal 0
        assert_eq!(parse_samfilter("0"), 0);
    }

    #[test]
    fn test_parse_samfilter_negative() {
        // Java Integer.decode("-1") = -1, which as u16 wraps to 65535
        assert_eq!(parse_samfilter("-1"), (-1i32) as u16);
    }

    #[test]
    fn test_get_chr_name_with_chr_prefix_and_flag() {
        let region = Region::new("chr1", 100, 200, "gene1");
        let mut conf = crate::config::Configuration::default();
        conf.chromosome_name_is_number = true;
        assert_eq!(get_chr_name(&region, &conf), "1");
    }

    #[test]
    fn test_get_chr_name_without_chr_prefix() {
        let region = Region::new("1", 100, 200, "gene1");
        let mut conf = crate::config::Configuration::default();
        conf.chromosome_name_is_number = true;
        assert_eq!(get_chr_name(&region, &conf), "1");
    }

    #[test]
    fn test_get_chr_name_flag_not_set() {
        let region = Region::new("chr1", 100, 200, "gene1");
        let conf = crate::config::Configuration::default();
        assert_eq!(get_chr_name(&region, &conf), "chr1");
    }
}
