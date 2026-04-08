use std::sync::atomic::{AtomicI32, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const HG19: &str = "/ngs/reference_data/genomes/Hsapiens/hg19/seq/hg19.fa";
pub const HG38: &str = "/ngs/reference_data/genomes/Hsapiens/hg38/seq/hg38.fa";
pub const MM10: &str = "/ngs/reference_data/genomes/Mmusculus/mm10/seq/mm10.fa";
pub const LOWQUAL: i32 = 10;
pub const SEED_1: i32 = 17;
pub const SEED_2: i32 = 12;
pub const ADSEED: i32 = 6;
pub const MINSVCDIST: f64 = 1.5;
pub const MINMAPBASE: i32 = 15;
pub const MINSVPOS: i32 = 25;
pub const SVMAXLEN: i32 = 150_000;
pub const SVFLANK: i32 = 50;
pub const DISCPAIRQUAL: i32 = 35;
pub const EXTENSION: i32 = 5_000;
pub const DEFAULT_AMPLICON_PARAMETERS: &str = "10:0.95";
pub static MAX_EXCEPTION_COUNT: AtomicI32 = AtomicI32::new(10);

/// Ported from: Configuration.java enum field usage + CmdParser.java:L147-L147
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationStringency {
    Strict,
    Lenient,
    Silent,
}

impl Default for ValidationStringency {
    fn default() -> Self {
        Self::Lenient
    }
}

/// Ported from: printers/PrinterType.java:L7-L15
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrinterType {
    Out,
    Err,
}

impl Default for PrinterType {
    fn default() -> Self {
        Self::Out
    }
}

/// Ported from: RegionBuilder.BedRowFormat()
/// Java source: RegionBuilder.java:L189-L204
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BedRowFormat {
    pub chr_column: i32,
    pub start_column: i32,
    pub end_column: i32,
    pub thick_start_column: i32,
    pub thick_end_column: i32,
    pub gene_column: i32,
}

impl BedRowFormat {
    /// Ported from: RegionBuilder.BedRowFormat.BedRowFormat()
    /// Java source: RegionBuilder.java:L197-L204
    pub const fn new(
        chr_column: i32,
        start_column: i32,
        end_column: i32,
        thick_start_column: i32,
        thick_end_column: i32,
        gene_column: i32,
    ) -> Self {
        Self {
            chr_column,
            start_column,
            end_column,
            thick_start_column,
            thick_end_column,
            gene_column,
        }
    }
}

impl Default for BedRowFormat {
    fn default() -> Self {
        DEFAULT_BED_ROW_FORMAT
    }
}

pub const DEFAULT_BED_ROW_FORMAT: BedRowFormat = BedRowFormat::new(2, 6, 7, 9, 10, 12);
pub const CUSTOM_BED_ROW_FORMAT: BedRowFormat = BedRowFormat::new(0, 1, 2, 3, 1, 2);
pub const AMP_BED_ROW_FORMAT: BedRowFormat = BedRowFormat::new(0, 1, 2, 6, 7, 3);

/// Ported from: Configuration.BamNames
/// Java source: Configuration.java:L341-L370
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BamNames {
    pub bam1: String,
    pub bam2: Option<String>,
    pub bam_x: String,
    pub bam_raw: String,
}

impl BamNames {
    /// Ported from: Configuration.BamNames.BamNames(String)
    /// Java source: Configuration.java:L346-L349
    pub fn new(value: impl Into<String>) -> Self {
        let bam_raw = value.into();
        let mut bam_names = bam_raw.split('|');
        let bam1 = bam_names.next().unwrap_or_default().to_string();
        let bam2 = bam_names.next().map(str::to_string);
        let bam_x = bam1.split(':').next().unwrap_or_default().to_string();

        Self {
            bam1,
            bam2,
            bam_x,
            bam_raw,
        }
    }

    pub fn get_bam1(&self) -> &str {
        &self.bam1
    }

    pub fn get_bam2(&self) -> Option<&str> {
        self.bam2.as_deref()
    }

    pub fn get_bam_x(&self) -> &str {
        &self.bam_x
    }

    pub fn has_bam2(&self) -> bool {
        self.bam2.is_some()
    }

    pub fn get_bam_raw(&self) -> &str {
        &self.bam_raw
    }
}

mod atomic_i32_serde {
    use super::*;

    pub fn serialize<S>(value: &AtomicI32, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(value.load(Ordering::Relaxed))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<AtomicI32, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(AtomicI32::new(i32::deserialize(deserializer)?))
    }
}

/// Ported from: Configuration.java field inventory + CmdParser.parseCmd()
/// Java source: Configuration.java:L10-L323, CmdParser.java:L53-L182
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub print_header: bool,
    pub delimiter: String,
    pub bed: Option<String>,
    pub number_nucleotide_to_extend: i32,
    pub zero_based: Option<bool>,
    pub amplicon_based_calling: Option<String>,
    pub column_for_chromosome: i32,
    pub bed_row_format: BedRowFormat,
    pub sample_name_regexp: Option<String>,
    pub sample_name: Option<String>,
    pub fasta: String,
    pub bam: Option<BamNames>,
    pub downsampling: Option<f64>,
    pub chromosome_name_is_number: bool,
    pub mapping_quality: Option<i32>,
    pub remove_duplicated_reads: bool,
    pub mismatch: i32,
    pub y: bool,
    pub goodq: f64,
    pub vext: i32,
    pub trim_bases_after: i32,
    pub perform_local_realignment: bool,
    pub indelsize: i32,
    pub bias: f64,
    pub min_bias_reads: i32,
    pub minr: i32,
    pub debug: bool,
    pub freq: f64,
    pub move_indels_to_3: bool,
    pub samfilter: String,
    pub region_of_interest: Option<String>,
    pub read_pos_filter: i32,
    pub qratio: f64,
    pub mapq: f64,
    pub do_pileup: bool,
    pub lofreq: f64,
    pub minmatch: i32,
    pub output_splicing: bool,
    pub validation_stringency: ValidationStringency,
    pub include_n_in_total_depth: bool,
    pub unique_mode_alignment_enabled: bool,
    pub unique_mode_second_in_pair_enabled: bool,
    pub threads: i32,
    pub chimeric: bool,
    pub disable_sv: bool,
    pub delete_duplicate_variants: bool,
    pub fisher: bool,
    #[serde(rename = "INSSIZE")]
    pub inssize: i32,
    #[serde(rename = "INSSTD")]
    pub insstd: i32,
    #[serde(rename = "INSSTDAMT")]
    pub insstdamt: i32,
    #[serde(rename = "SVMINLEN")]
    pub svminlen: i32,
    pub reference_extension: i32,
    pub printer_type: PrinterType,
    #[serde(with = "atomic_i32_serde")]
    pub exception_counter: AtomicI32,
    pub adaptor: Vec<String>,
    pub crispr_filtering_bp: i32,
    pub crispr_cutting_site: i32,
    pub monomer_msi_frequency: f64,
    pub non_monomer_msi_frequency: f64,
}

impl Clone for Configuration {
    fn clone(&self) -> Self {
        Self {
            print_header: self.print_header,
            delimiter: self.delimiter.clone(),
            bed: self.bed.clone(),
            number_nucleotide_to_extend: self.number_nucleotide_to_extend,
            zero_based: self.zero_based,
            amplicon_based_calling: self.amplicon_based_calling.clone(),
            column_for_chromosome: self.column_for_chromosome,
            bed_row_format: self.bed_row_format,
            sample_name_regexp: self.sample_name_regexp.clone(),
            sample_name: self.sample_name.clone(),
            fasta: self.fasta.clone(),
            bam: self.bam.clone(),
            downsampling: self.downsampling,
            chromosome_name_is_number: self.chromosome_name_is_number,
            mapping_quality: self.mapping_quality,
            remove_duplicated_reads: self.remove_duplicated_reads,
            mismatch: self.mismatch,
            y: self.y,
            goodq: self.goodq,
            vext: self.vext,
            trim_bases_after: self.trim_bases_after,
            perform_local_realignment: self.perform_local_realignment,
            indelsize: self.indelsize,
            bias: self.bias,
            min_bias_reads: self.min_bias_reads,
            minr: self.minr,
            debug: self.debug,
            freq: self.freq,
            move_indels_to_3: self.move_indels_to_3,
            samfilter: self.samfilter.clone(),
            region_of_interest: self.region_of_interest.clone(),
            read_pos_filter: self.read_pos_filter,
            qratio: self.qratio,
            mapq: self.mapq,
            do_pileup: self.do_pileup,
            lofreq: self.lofreq,
            minmatch: self.minmatch,
            output_splicing: self.output_splicing,
            validation_stringency: self.validation_stringency,
            include_n_in_total_depth: self.include_n_in_total_depth,
            unique_mode_alignment_enabled: self.unique_mode_alignment_enabled,
            unique_mode_second_in_pair_enabled: self.unique_mode_second_in_pair_enabled,
            threads: self.threads,
            chimeric: self.chimeric,
            disable_sv: self.disable_sv,
            delete_duplicate_variants: self.delete_duplicate_variants,
            fisher: self.fisher,
            inssize: self.inssize,
            insstd: self.insstd,
            insstdamt: self.insstdamt,
            svminlen: self.svminlen,
            reference_extension: self.reference_extension,
            printer_type: self.printer_type,
            exception_counter: AtomicI32::new(self.exception_counter.load(Ordering::Relaxed)),
            adaptor: self.adaptor.clone(),
            crispr_filtering_bp: self.crispr_filtering_bp,
            crispr_cutting_site: self.crispr_cutting_site,
            monomer_msi_frequency: self.monomer_msi_frequency,
            non_monomer_msi_frequency: self.non_monomer_msi_frequency,
        }
    }
}

impl Default for Configuration {
    /// Ported from: CmdParser.parseCmd()
    /// Java source: CmdParser.java:L62-L182
    fn default() -> Self {
        Self {
            print_header: false,
            delimiter: "\t".to_string(),
            bed: None,
            number_nucleotide_to_extend: 0,
            zero_based: None,
            amplicon_based_calling: None,
            column_for_chromosome: -1,
            bed_row_format: DEFAULT_BED_ROW_FORMAT,
            sample_name_regexp: None,
            sample_name: None,
            fasta: HG19.to_string(),
            bam: None,
            downsampling: None,
            chromosome_name_is_number: false,
            mapping_quality: None,
            remove_duplicated_reads: false,
            mismatch: 8,
            y: false,
            goodq: 22.5,
            vext: 2,
            trim_bases_after: 0,
            perform_local_realignment: true,
            indelsize: 50,
            bias: 0.05,
            min_bias_reads: 2,
            minr: 2,
            debug: false,
            freq: 0.01,
            move_indels_to_3: false,
            samfilter: "0x504".to_string(),
            region_of_interest: None,
            read_pos_filter: 5,
            qratio: 1.5,
            mapq: 0.0,
            do_pileup: false,
            lofreq: 0.05,
            minmatch: 0,
            output_splicing: false,
            validation_stringency: ValidationStringency::Lenient,
            include_n_in_total_depth: false,
            unique_mode_alignment_enabled: false,
            unique_mode_second_in_pair_enabled: false,
            threads: 1,
            chimeric: false,
            disable_sv: false,
            delete_duplicate_variants: false,
            fisher: false,
            inssize: 300,
            insstd: 100,
            insstdamt: 4,
            svminlen: 1_000,
            reference_extension: 1_200,
            printer_type: PrinterType::Out,
            exception_counter: AtomicI32::new(0),
            adaptor: Vec::new(),
            crispr_filtering_bp: 0,
            crispr_cutting_site: 0,
            monomer_msi_frequency: 0.25,
            non_monomer_msi_frequency: 0.1,
        }
    }
}

impl Configuration {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ported from: Configuration.isColumnForChromosomeSet()
    /// Java source: Configuration.java:L325-L327
    pub fn is_column_for_chromosome_set(&self) -> bool {
        self.column_for_chromosome >= 0
    }

    /// Ported from: Configuration.isDownsampling()
    /// Java source: Configuration.java:L329-L331
    pub fn is_downsampling(&self) -> bool {
        self.downsampling.is_some()
    }

    /// Ported from: Configuration.hasMappingQuality()
    /// Java source: Configuration.java:L333-L335
    pub fn has_mapping_quality(&self) -> bool {
        self.mapping_quality.is_some()
    }

    /// Ported from: Configuration.isZeroBasedDefined()
    /// Java source: Configuration.java:L337-L339
    pub fn is_zero_based_defined(&self) -> bool {
        self.zero_based.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_float_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }

    #[test]
    fn default_configuration_matches_cmd_parser_defaults() {
        let config = Configuration::default();

        assert!(!config.print_header);
        assert_eq!(config.delimiter, "\t");
        assert_eq!(config.bed, None);
        assert_eq!(config.number_nucleotide_to_extend, 0);
        assert_eq!(config.zero_based, None);
        assert_eq!(config.amplicon_based_calling, None);
        assert_eq!(config.column_for_chromosome, -1);
        assert_eq!(config.bed_row_format, DEFAULT_BED_ROW_FORMAT);
        assert_eq!(config.sample_name_regexp, None);
        assert_eq!(config.sample_name, None);
        assert_eq!(config.fasta, HG19);
        assert_eq!(config.bam, None);
        assert_eq!(config.downsampling, None);
        assert!(!config.chromosome_name_is_number);
        assert_eq!(config.mapping_quality, None);
        assert!(!config.remove_duplicated_reads);
        assert_eq!(config.mismatch, 8);
        assert!(!config.y);
        assert_float_eq(config.goodq, 22.5);
        assert_eq!(config.vext, 2);
        assert_eq!(config.trim_bases_after, 0);
        assert!(config.perform_local_realignment);
        assert_eq!(config.indelsize, 50);
        assert_float_eq(config.bias, 0.05);
        assert_eq!(config.min_bias_reads, 2);
        assert_eq!(config.minr, 2);
        assert!(!config.debug);
        assert_float_eq(config.freq, 0.01);
        assert!(!config.move_indels_to_3);
        assert_eq!(config.samfilter, "0x504");
        assert_eq!(config.region_of_interest, None);
        assert_eq!(config.read_pos_filter, 5);
        assert_float_eq(config.qratio, 1.5);
        assert_float_eq(config.mapq, 0.0);
        assert!(!config.do_pileup);
        assert_float_eq(config.lofreq, 0.05);
        assert_eq!(config.minmatch, 0);
        assert!(!config.output_splicing);
        assert_eq!(config.validation_stringency, ValidationStringency::Lenient);
        assert!(!config.include_n_in_total_depth);
        assert!(!config.unique_mode_alignment_enabled);
        assert!(!config.unique_mode_second_in_pair_enabled);
        assert_eq!(config.threads, 1);
        assert!(!config.chimeric);
        assert!(!config.disable_sv);
        assert!(!config.delete_duplicate_variants);
        assert!(!config.fisher);
        assert_eq!(config.inssize, 300);
        assert_eq!(config.insstd, 100);
        assert_eq!(config.insstdamt, 4);
        assert_eq!(config.svminlen, 1_000);
        assert_eq!(config.reference_extension, 1_200);
        assert_eq!(config.printer_type, PrinterType::Out);
        assert_eq!(config.exception_counter.load(Ordering::Relaxed), 0);
        assert!(config.adaptor.is_empty());
        assert_eq!(config.crispr_filtering_bp, 0);
        assert_eq!(config.crispr_cutting_site, 0);
        assert_float_eq(config.monomer_msi_frequency, 0.25);
        assert_float_eq(config.non_monomer_msi_frequency, 0.1);
    }

    #[test]
    fn bam_names_parse_expected_tumor_normal_layout() {
        let bam_names = BamNames::new("tumor1.bam:tumor2.bam|normal.bam");

        assert_eq!(bam_names.get_bam1(), "tumor1.bam:tumor2.bam");
        assert_eq!(bam_names.get_bam2(), Some("normal.bam"));
        assert_eq!(bam_names.get_bam_x(), "tumor1.bam");
        assert!(bam_names.has_bam2());
        assert_eq!(bam_names.get_bam_raw(), "tumor1.bam:tumor2.bam|normal.bam");
    }

    #[test]
    fn bed_row_format_constants_match_java_defaults() {
        assert_eq!(DEFAULT_BED_ROW_FORMAT, BedRowFormat::new(2, 6, 7, 9, 10, 12));
        assert_eq!(CUSTOM_BED_ROW_FORMAT, BedRowFormat::new(0, 1, 2, 3, 1, 2));
        assert_eq!(AMP_BED_ROW_FORMAT, BedRowFormat::new(0, 1, 2, 6, 7, 3));
        assert_eq!(MAX_EXCEPTION_COUNT.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn configuration_serializes_and_deserializes_exception_counter() {
        let config = Configuration {
            exception_counter: AtomicI32::new(3),
            ..Configuration::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let round_trip: Configuration = serde_json::from_str(&json).unwrap();

        assert!(json.contains("\"exceptionCounter\":3"));
        assert_eq!(round_trip.exception_counter.load(Ordering::Relaxed), 3);
        assert_eq!(round_trip.fasta, HG19);
        assert!(round_trip.perform_local_realignment);
    }
}