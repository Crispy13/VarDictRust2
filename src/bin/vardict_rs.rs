#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use vardict_rs::prelude::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::io::{self, ErrorKind};
use std::path::Path;

use clap::Parser;
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::modes::{AmpliconMode, ParallelMode, SimpleMode, SomaticMode};
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::GlobalReadOnlyScope;
use vardict_rs::variations::{clear_variation_utils_scope, configure_variation_utils_scope};

// VarDict-rs CLI. Mirrors VarDictJava's `CmdParser` option set (release parity).
//
// Behavioral parity, not bit-perfect parsing: clap (derive) cannot replicate Apache commons-cli
// token-for-token. Multi-char single-dash Java flags (`-UN`, `-DP`, `-VS`) are exposed as clap long
// flags and bridged from the single-dash form in `normalize_java_flags`. Help is reconfigured so
// `-h`/`--header` sets the header row (as in Java) and `-H`/`-?`/`--help` prints help.
#[derive(Debug, Default, Parser)]
#[command(name = "vardict_rs", disable_help_flag = true)]
struct Cli {
    #[arg(short = 'H', long = "help", visible_short_alias = '?', action = clap::ArgAction::Help)]
    help: Option<bool>,

    #[arg(short = 'R')]
    region: Option<String>,

    #[arg(short = 'b')]
    bam: String,

    #[arg(short = 'G')]
    reference: String,

    #[arg(short = 'N')]
    sample_name: String,

    /// -n: sample-name regexp (leading/trailing `/` stripped)
    #[arg(short = 'n')]
    sample_name_regexp: Option<String>,

    #[arg(short = 'p')]
    pileup: bool,

    #[arg(short = 'f')]
    freq: Option<f64>,

    #[arg(short = 'r')]
    minr: Option<i32>,

    #[arg(short = 'q')]
    goodq: Option<f64>,

    #[arg(short = 'm')]
    mismatch: Option<i32>,

    #[arg(short = 'X')]
    vext: Option<i32>,

    #[arg(short = 'B')]
    min_bias_reads: Option<i32>,

    /// -k: local realignment (optional value, default 1 when present)
    #[arg(short = 'k', num_args = 0..=1, default_missing_value = "1")]
    realign: Option<i32>,

    #[arg(long = "fisher")]
    fisher: bool,

    #[arg(long = "th")]
    threads: Option<i32>,

    // --- BED column selection ---
    #[arg(short = 'c', default_value_t = 1)]
    chromosome_column: usize,
    #[arg(short = 'S', default_value_t = 2)]
    start_column: usize,
    #[arg(short = 'E', default_value_t = 3)]
    end_column: usize,
    #[arg(short = 'g')]
    gene_column: Option<usize>,
    #[arg(short = 's')]
    insert_start_column: Option<usize>,
    #[arg(short = 'e')]
    insert_end_column: Option<usize>,

    // --- coordinates / regions ---
    /// -z: zero-based BED coordinates (optional value, default 1 when present)
    #[arg(short = 'z', num_args = 0..=1, default_missing_value = "1")]
    zero_based: Option<i32>,
    /// -x: bp to extend each region by
    #[arg(short = 'x')]
    number_nucleotide_to_extend: Option<i32>,
    /// -a/--amplicon: amplicon-based calling, "int:float" (default 10:0.95)
    #[arg(short = 'a', long = "amplicon", num_args = 0..=1, default_missing_value = "10:0.95")]
    amplicon: Option<String>,

    // --- thresholds / filters ---
    #[arg(short = 'Q')]
    mapping_quality: Option<i32>,
    #[arg(short = 'F')]
    samfilter: Option<String>,
    #[arg(short = 'Z', long = "downsample")]
    downsampling: Option<f64>,
    #[arg(short = 'T', long = "trim")]
    trim_bases_after: Option<i32>,
    #[arg(short = 'P')]
    read_pos_filter: Option<i32>,
    #[arg(short = 'o')]
    qratio: Option<f64>,
    #[arg(short = 'O')]
    mapq: Option<f64>,
    #[arg(short = 'V', long = "verbose")]
    lofreq: Option<f64>,
    #[arg(short = 'I')]
    indelsize: Option<i32>,
    #[arg(short = 'M')]
    minmatch: Option<i32>,

    // --- boolean modes ---
    /// -3: move indels to 3-prime
    #[arg(short = '3')]
    move_indels_to_3: bool,
    #[arg(short = 'D', long = "debug")]
    debug: bool,
    #[arg(short = 'y')]
    y: bool,
    /// -C: chromosome names are plain numbers (deprecated)
    #[arg(short = 'C')]
    chromosome_name_is_number: bool,
    #[arg(short = 't', long = "dedup")]
    remove_duplicated_reads: bool,
    #[arg(short = 'u')]
    unique_mode_alignment: bool,
    /// -UN: unique mode, second-in-pair (bridged from -UN)
    #[arg(long = "UN")]
    unique_mode_second_in_pair: bool,
    #[arg(short = 'K')]
    include_n_in_total_depth: bool,
    #[arg(short = 'i', long = "splice")]
    output_splicing: bool,
    /// -h/--header: print a header row
    #[arg(short = 'h', long = "header")]
    print_header: bool,
    #[arg(long = "chimeric")]
    chimeric: bool,
    /// -U/--nosv: turn off structural-variant calling
    #[arg(short = 'U', long = "nosv")]
    disable_sv: bool,
    #[arg(long = "deldupvar")]
    delete_duplicate_variants: bool,
    /// -v: accepted for VarDictJava compatibility; ignored (output is always var-TSV)
    #[arg(short = 'v')]
    vcf: bool,

    // --- structural-variant / insert params ---
    #[arg(short = 'w', long = "insert-size")]
    inssize: Option<i32>,
    #[arg(short = 'W', long = "insert-std")]
    insstd: Option<i32>,
    #[arg(short = 'A')]
    insstdamt: Option<i32>,
    #[arg(short = 'L')]
    svminlen: Option<i32>,
    #[arg(short = 'Y', long = "ref-extension")]
    reference_extension: Option<i32>,

    // --- CRISPR / MSI ---
    #[arg(short = 'J', long = "crispr")]
    crispr_cutting_site: Option<i32>,
    #[arg(short = 'j')]
    crispr_filtering_bp: Option<i32>,
    #[arg(long = "mfreq")]
    mfreq: Option<f64>,
    #[arg(long = "nmfreq")]
    nmfreq: Option<f64>,

    // --- output / misc ---
    /// -d: column delimiter (default tab)
    #[arg(short = 'd')]
    delimiter: Option<String>,
    /// -DP: default printer, OUT or ERR (bridged from -DP)
    #[arg(long = "DP")]
    default_printer: Option<String>,
    /// -VS: validation stringency: STRICT, LENIENT or SILENT (bridged from -VS)
    #[arg(long = "VS")]
    validation_stringency: Option<String>,
    /// --adaptor: comma-separated adaptor sequences to trim
    #[arg(long = "adaptor")]
    adaptor: Option<String>,

    #[arg(value_name = "BED")]
    bed_path: Option<String>,
}

struct ScopeCleanup;

impl Drop for ScopeCleanup {
    fn drop(&mut self) {
        GlobalReadOnlyScope::clear();
        clear_variation_utils_scope();
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    run(Cli::parse_from(normalize_java_flags(std::env::args_os())))
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let requested_threads = cli
        .threads
        .and_then(|threads| usize::try_from(threads).ok())
        .unwrap_or(1);
    let fai_path = format!("{}.fai", cli.reference);
    let chr_lengths = load_chr_lengths(&fai_path)?;

    let config = build_config(&cli)?;

    // Resolve regions / segments and the effective amplicon param (CmdParser + VarDictLauncher).
    let RegionPlan {
        segments,
        amplicon,
    } = resolve_regions(&cli, &config, &chr_lengths)?;

    // Sample names: VarDictJava splits "-N tumor|normal" into (sample, samplem).
    let (sample, samplem) = split_sample_name(&cli.sample_name);

    // Adaptor seed maps (VarDictLauncher.java:105-119).
    let (adaptor_forward, adaptor_reverse) = build_adaptor_maps(config.adaptor.as_slice());

    let reference_extension = config.reference_extension;
    let number_nucleotide_to_extend = config.number_nucleotide_to_extend;

    configure_variation_utils_scope(
        config.clone(),
        adaptor_forward.clone(),
        adaptor_reverse.clone(),
    );

    GlobalReadOnlyScope::init(
        config,
        chr_lengths.clone(),
        sample,
        samplem,
        amplicon.clone(),
        adaptor_forward,
        adaptor_reverse,
    );
    let _cleanup = ScopeCleanup;

    let reference_resource = ReferenceResource::new(
        &cli.reference,
        reference_extension,
        number_nucleotide_to_extend,
        chr_lengths,
        false,
    );

    // Mode selection mirrors VarDictLauncher.start(): AmpliconMode only when an amplicon param is
    // set and -R is absent; otherwise Somatic (paired BAM) or Simple.
    let is_amplicon = amplicon.is_some() && cli.region.is_none();
    let has_bam2 = GlobalReadOnlyScope::instance()
        .conf
        .bam
        .as_ref()
        .map_or(false, |b| b.has_bam2());

    if is_amplicon {
        let amplicon_mode = AmpliconMode::new(segments, reference_resource);
        if requested_threads > 1 {
            amplicon_mode.parallel(requested_threads);
        } else {
            amplicon_mode.not_parallel();
        }
    } else if has_bam2 {
        let somatic_mode = SomaticMode::new(segments, reference_resource);
        if requested_threads > 1 {
            somatic_mode.parallel(requested_threads);
        } else {
            somatic_mode.not_parallel();
        }
    } else {
        let simple_mode = SimpleMode::new(segments, reference_resource);
        if requested_threads > 1 {
            simple_mode.parallel(requested_threads);
        } else {
            simple_mode.not_parallel();
        }
    }

    Ok(())
}

/// Build a `Configuration` from the CLI, mirroring `CmdParser.parseCmd`. Every option overrides its
/// field only when present, so omitting a flag leaves the (Java-matching) `Configuration::default()`.
fn build_config(cli: &Cli) -> Result<Configuration, Box<dyn Error>> {
    let mut config = Configuration {
        bam: Some(BamNames::new(&cli.bam)),
        fasta: cli.reference.clone(),
        ..Configuration::default()
    };

    // thresholds / numeric options
    if let Some(freq) = cli.freq {
        config.freq = freq;
    }
    if let Some(minr) = cli.minr {
        config.minr = minr;
    }
    if let Some(goodq) = cli.goodq {
        config.goodq = goodq;
    }
    if let Some(mismatch) = cli.mismatch {
        config.mismatch = mismatch;
    }
    if let Some(vext) = cli.vext {
        config.vext = vext;
    }
    if let Some(min_bias_reads) = cli.min_bias_reads {
        config.min_bias_reads = min_bias_reads;
    }
    if let Some(realign) = cli.realign {
        config.perform_local_realignment = realign != 0;
    }
    if let Some(mapping_quality) = cli.mapping_quality {
        config.mapping_quality = Some(mapping_quality);
    }
    if let Some(samfilter) = &cli.samfilter {
        config.samfilter = samfilter.clone();
    }
    if let Some(downsampling) = cli.downsampling {
        config.downsampling = Some(downsampling);
    }
    if let Some(trim) = cli.trim_bases_after {
        config.trim_bases_after = trim;
    }
    if let Some(read_pos_filter) = cli.read_pos_filter {
        config.read_pos_filter = read_pos_filter;
    }
    if let Some(qratio) = cli.qratio {
        config.qratio = qratio;
    }
    if let Some(mapq) = cli.mapq {
        config.mapq = mapq;
    }
    if let Some(lofreq) = cli.lofreq {
        config.lofreq = lofreq;
    }
    if let Some(indelsize) = cli.indelsize {
        config.indelsize = indelsize;
    }
    if let Some(minmatch) = cli.minmatch {
        config.minmatch = minmatch;
    }
    if let Some(extend) = cli.number_nucleotide_to_extend {
        config.number_nucleotide_to_extend = extend;
    }
    if let Some(inssize) = cli.inssize {
        config.inssize = inssize;
    }
    if let Some(insstd) = cli.insstd {
        config.insstd = insstd;
    }
    if let Some(insstdamt) = cli.insstdamt {
        config.insstdamt = insstdamt;
    }
    if let Some(svminlen) = cli.svminlen {
        config.svminlen = svminlen;
    }
    if let Some(reference_extension) = cli.reference_extension {
        config.reference_extension = reference_extension;
    }
    if let Some(site) = cli.crispr_cutting_site {
        config.crispr_cutting_site = site;
    }
    if let Some(bp) = cli.crispr_filtering_bp {
        config.crispr_filtering_bp = bp;
    }
    if let Some(mfreq) = cli.mfreq {
        config.monomer_msi_frequency = mfreq;
    }
    if let Some(nmfreq) = cli.nmfreq {
        config.non_monomer_msi_frequency = nmfreq;
    }

    // coordinate handling
    config.zero_based = cli.zero_based.map(|value| value == 1);
    config.amplicon_based_calling = cli.amplicon.clone();
    config.region_of_interest = cli.region.clone();

    // boolean modes (default false, matching Configuration::default())
    config.move_indels_to_3 = cli.move_indels_to_3;
    config.debug = cli.debug;
    config.y = cli.y;
    config.chromosome_name_is_number = cli.chromosome_name_is_number;
    config.remove_duplicated_reads = cli.remove_duplicated_reads;
    config.unique_mode_alignment_enabled = cli.unique_mode_alignment;
    config.unique_mode_second_in_pair_enabled = cli.unique_mode_second_in_pair;
    config.include_n_in_total_depth = cli.include_n_in_total_depth;
    config.output_splicing = cli.output_splicing;
    config.print_header = cli.print_header;
    config.chimeric = cli.chimeric;
    config.disable_sv = cli.disable_sv;
    config.delete_duplicate_variants = cli.delete_duplicate_variants;
    config.fisher = cli.fisher;

    if let Some(delimiter) = &cli.delimiter {
        config.delimiter = delimiter.clone();
    }
    if let Some(regexp) = &cli.sample_name_regexp {
        config.sample_name_regexp = Some(strip_regexp_slashes(regexp));
    }
    if let Some(printer) = &cli.default_printer {
        config.printer_type = match printer.to_ascii_uppercase().as_str() {
            "ERR" => vardict_rs::config::PrinterType::Err,
            _ => vardict_rs::config::PrinterType::Out,
        };
    }
    if let Some(stringency) = &cli.validation_stringency {
        config.validation_stringency = match stringency.to_ascii_uppercase().as_str() {
            "STRICT" => vardict_rs::config::ValidationStringency::Strict,
            "LENIENT" => vardict_rs::config::ValidationStringency::Lenient,
            "SILENT" => vardict_rs::config::ValidationStringency::Silent,
            other => {
                return Err(Box::new(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("Invalid -VS validation stringency: {other}"),
                )));
            }
        };
    }
    if let Some(adaptor) = &cli.adaptor {
        config.adaptor = adaptor.split(',').map(|s| s.to_string()).collect();
    }

    if cli.threads.is_some() {
        config.threads = cli.threads.unwrap_or(1).max(1);
    }

    // -p pileup preset overrides freq/minr (CmdParser: doPileup=true, freq=-1, minr=0)
    if cli.pileup {
        apply_pileup_preset(&mut config);
    }

    Ok(config)
}

/// Split a VarDictJava sample argument: "tumor|normal" -> (tumor, Some(normal)).
fn split_sample_name(raw: &str) -> (String, Option<String>) {
    match raw.split_once('|') {
        Some((first, second)) => (first.to_string(), Some(second.to_string())),
        None => (raw.to_string(), None),
    }
}

/// Strip a single leading and trailing '/' from a -n regexp (CmdParser behavior).
fn strip_regexp_slashes(regexp: &str) -> String {
    let trimmed = regexp.strip_prefix('/').unwrap_or(regexp);
    trimmed.strip_suffix('/').unwrap_or(trimmed).to_string()
}

/// Build adaptor forward/reverse seed maps (VarDictLauncher.java:105-119): for each sequence, up to
/// 7 offsets i in [0,6] with i+ADSEED < len; forward = seq[i..i+ADSEED], reverse = complement(reverse).
fn build_adaptor_maps(adaptor: &[String]) -> (HashMap<String, i32>, HashMap<String, i32>) {
    use vardict_rs::config::ADSEED;
    use vardict_rs::utils::{complement_sequence, reverse_sequence};

    let mut forward: HashMap<String, i32> = HashMap::default();
    let mut reverse: HashMap<String, i32> = HashMap::default();
    let seed = ADSEED as usize;
    for sequence in adaptor {
        let bytes = sequence.as_bytes();
        let mut i = 0usize;
        while i <= 6 && i + seed < bytes.len() {
            let forward_seed = &bytes[i..i + seed];
            let reverse_seed = complement_sequence(&reverse_sequence(forward_seed));
            forward.insert(
                String::from_utf8_lossy(forward_seed).into_owned(),
                (i + 1) as i32,
            );
            reverse.insert(
                String::from_utf8_lossy(&reverse_seed).into_owned(),
                (i + 1) as i32,
            );
            i += 1;
        }
    }
    (forward, reverse)
}

fn load_chr_lengths(fai_path: &str) -> Result<HashMap<String, i32>, io::Error> {
    let content = std::fs::read_to_string(fai_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Failed to read FAI file {fai_path}: {error}"),
        )
    })?;

    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 2 {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Malformed FAI line in {fai_path}: {line}"),
                ));
            }

            let len = fields[1].parse::<i32>().map_err(|error| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "Invalid chromosome length '{}' in {fai_path}: {error}",
                        fields[1]
                    ),
                )
            })?;

            Ok((fields[0].to_string(), len))
        })
        .collect()
}

/// Result of region resolution: the segments to process plus the effective amplicon parameter
/// (`None` unless `-a` was given or an 8-column amplicon BED was auto-detected).
#[derive(Debug)]
struct RegionPlan {
    segments: Vec<Vec<Region>>,
    amplicon: Option<String>,
}

/// Parse a `-R chr:start-end[:gene]` region (RegionBuilder.buildRegionFromConfiguration).
fn parse_region(region_str: &str, extend: i32, zero_based: bool) -> Result<Region, io::Error> {
    let parts: Vec<&str> = region_str.split(':').collect();
    if parts.len() < 2 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid region format (expected chr:start-end): {region_str}"),
        ));
    }
    let chr = parts[0];
    let gene = if parts.len() < 3 { chr } else { parts[2] };
    let range: Vec<&str> = parts[1].split('-').collect();
    let parse_coord = |s: &str| -> Result<i32, io::Error> {
        s.replace(',', "").parse::<i32>().map_err(|error| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("Invalid coordinate in region {region_str}: {error}"),
            )
        })
    };
    let mut start = parse_coord(range[0])?;
    let mut end = if range.len() < 2 {
        start
    } else {
        parse_coord(range[1])?
    };
    start -= extend;
    end += extend;
    if zero_based && start < end {
        start += 1;
    }
    if start > end {
        start = end;
    }
    Ok(Region::new(chr, start, end, gene))
}

fn apply_pileup_preset(config: &mut Configuration) {
    config.do_pileup = true;
    config.freq = -1.0;
    config.minr = 0;
}

fn resolve_regions(
    cli: &Cli,
    config: &Configuration,
    chr_lengths: &HashMap<String, i32>,
) -> Result<RegionPlan, io::Error> {
    match (cli.region.as_deref(), cli.bed_path.as_deref()) {
        (Some(_), Some(_)) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Specify either -R <region> or a trailing BED path, not both",
        )),
        (None, None) => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "Specify either -R <region> or a trailing BED path",
        )),
        (Some(region_str), None) => {
            // -R path: ampliconBasedCalling stays null (VarDictLauncher); zero-based default false.
            let zero_based = config.zero_based.unwrap_or(false);
            let region = parse_region(region_str, config.number_nucleotide_to_extend, zero_based)?;
            Ok(RegionPlan {
                segments: vec![vec![region]],
                amplicon: None,
            })
        }
        (None, Some(bed_path)) => {
            let lines = read_bed_lines(Path::new(bed_path))?;
            let (amplicon, zero_based) = detect_amplicon(config, &lines);
            if amplicon.is_some() {
                let segments = build_amp_regions(&lines, config, chr_lengths, zero_based)?;
                Ok(RegionPlan { segments, amplicon })
            } else {
                let columns = BedColumns::from_cli(cli)?;
                let regions = build_regions(&lines, &columns, config, chr_lengths, zero_based)?;
                Ok(RegionPlan {
                    segments: vec![regions],
                    amplicon: None,
                })
            }
        }
    }
}

/// Read non-comment BED lines (skips `#`, `browser`, `track`), keeping 1-based line numbers.
fn read_bed_lines(bed_path: &Path) -> Result<Vec<(usize, String)>, io::Error> {
    let content = std::fs::read_to_string(bed_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Failed to read BED file {}: {error}", bed_path.display()),
        )
    })?;
    let mut lines = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("browser")
            || trimmed.starts_with("track")
        {
            continue;
        }
        lines.push((idx + 1, line.to_string()));
    }
    if lines.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "BED file {} did not contain any regions",
                bed_path.display()
            ),
        ));
    }
    Ok(lines)
}

/// Determine amplicon param and effective zero-based (VarDictLauncher.readBedFile): explicit `-a`
/// wins; otherwise auto-detect an 8-column amplicon BED (cols 7,8 numeric and inside the region).
fn detect_amplicon(config: &Configuration, lines: &[(usize, String)]) -> (Option<String>, bool) {
    if let Some(param) = &config.amplicon_based_calling {
        return (Some(param.clone()), config.zero_based.unwrap_or(false));
    }
    for (_, line) in lines {
        let cols: Vec<&str> = line.split(&config.delimiter).collect();
        if cols.len() == 8 {
            if let (Ok(a1), Ok(a2), Ok(a6), Ok(a7)) = (
                cols[1].parse::<i32>(),
                cols[2].parse::<i32>(),
                cols[6].parse::<i32>(),
                cols[7].parse::<i32>(),
            ) {
                if a6 >= a1 && a7 <= a2 {
                    // amplicon BED: default params, zero-based true unless explicitly set
                    return (
                        Some(vardict_rs::config::DEFAULT_AMPLICON_PARAMETERS.to_string()),
                        config.zero_based.unwrap_or(true),
                    );
                }
            }
        }
    }
    (None, config.zero_based.unwrap_or(false))
}

/// BED columns (0-based indices). `thick_start/thick_end` carry the `-s`/`-e` coupling: when `-S`/`-E`
/// are given without `-s`/`-e`, the region uses the start/end columns (CmdParser ~103-106).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BedColumns {
    chromosome: usize,
    thick_start: usize,
    thick_end: usize,
    gene: Option<usize>,
}

impl BedColumns {
    fn from_cli(cli: &Cli) -> Result<Self, io::Error> {
        let start = column_index(cli.start_column, "-S")?;
        let end = column_index(cli.end_column, "-E")?;
        let thick_start = match column_index_from_option(cli.insert_start_column, "-s")? {
            Some(value) => value,
            None => start,
        };
        let thick_end = match column_index_from_option(cli.insert_end_column, "-e")? {
            Some(value) => value,
            None => end,
        };
        Ok(Self {
            chromosome: column_index(cli.chromosome_column, "-c")?,
            thick_start,
            thick_end,
            gene: column_index_from_option(cli.gene_column, "-g")?,
        })
    }
}

/// Flat (Simple/Somatic) region build (RegionBuilder.buildRegions, single-value thick columns).
/// One region per BED line; segment grouping is flat (verified output-equivalent for these modes).
fn build_regions(
    lines: &[(usize, String)],
    columns: &BedColumns,
    config: &Configuration,
    chr_lengths: &HashMap<String, i32>,
    zero_based: bool,
) -> Result<Vec<Region>, io::Error> {
    let extend = config.number_nucleotide_to_extend;
    let mut regions = Vec::with_capacity(lines.len());
    for (line_number, line) in lines {
        let fields: Vec<&str> = line.split(&config.delimiter).collect();
        let chromosome = correct_chromosome(
            chr_lengths,
            required_bed_field(&fields, columns.chromosome, "chromosome", *line_number)?,
        );
        let raw_start = parse_bed_i32(
            required_bed_field(&fields, columns.thick_start, "start", *line_number)?,
            "start",
            *line_number,
        )?;
        let raw_end = parse_bed_i32(
            required_bed_field(&fields, columns.thick_end, "end", *line_number)?,
            "end",
            *line_number,
        )?;
        let mut start = raw_start - extend;
        let end = raw_end + extend;
        if zero_based && start < end {
            start += 1;
        }
        let gene = columns
            .gene
            .and_then(|gene_idx| fields.get(gene_idx).map(|field| field.trim()))
            .filter(|field| !field.is_empty())
            .unwrap_or(&chromosome)
            .to_string();
        regions.push(Region::new(chromosome, start, end, gene));
    }
    Ok(regions)
}

/// Amplicon region build with overlap grouping (RegionBuilder.buildAmpRegions). Columns are the fixed
/// AMP_BED_ROW_FORMAT (chr=0,start=1,end=2,gene=3,insertStart=6,insertEnd=7).
fn build_amp_regions(
    lines: &[(usize, String)],
    config: &Configuration,
    chr_lengths: &HashMap<String, i32>,
    zero_based: bool,
) -> Result<Vec<Vec<Region>>, io::Error> {
    // group per chromosome, preserving first-seen order
    let mut order: Vec<String> = Vec::new();
    let mut by_chr: HashMap<String, Vec<Region>> = HashMap::default();
    for (line_number, line) in lines {
        let f: Vec<&str> = line.split(&config.delimiter).collect();
        let chr = correct_chromosome(
            chr_lengths,
            required_bed_field(&f, 0, "chromosome", *line_number)?,
        );
        let mut start = parse_bed_i32(required_bed_field(&f, 1, "start", *line_number)?, "start", *line_number)?;
        let end = parse_bed_i32(required_bed_field(&f, 2, "end", *line_number)?, "end", *line_number)?;
        let gene = required_bed_field(&f, 3, "gene", *line_number)?.to_string();
        let mut insert_start =
            parse_bed_i32(required_bed_field(&f, 6, "insertStart", *line_number)?, "insertStart", *line_number)?;
        let insert_end =
            parse_bed_i32(required_bed_field(&f, 7, "insertEnd", *line_number)?, "insertEnd", *line_number)?;
        if zero_based && start < end {
            start += 1;
            insert_start += 1;
        }
        if !by_chr.contains_key(&chr) {
            order.push(chr.clone());
        }
        by_chr
            .entry(chr.clone())
            .or_default()
            .push(Region::new_with_insert_range(chr, start, end, gene, insert_start, insert_end));
    }

    let mut segs: Vec<Vec<Region>> = vec![Vec::new()];
    let mut previous_end = -1i32;
    let mut previous_chr: Option<String> = None;
    for chr in &order {
        let chr_regions = by_chr.get_mut(chr).unwrap();
        chr_regions.sort_by_key(|r| r.insert_start);
        for region in chr_regions.drain(..) {
            if previous_end != -1
                && (previous_chr.as_deref() != Some(region.chr.as_str())
                    || region.insert_start > previous_end)
            {
                segs.push(Vec::new());
            }
            previous_end = region.insert_end;
            previous_chr = Some(region.chr.clone());
            segs.last_mut().unwrap().push(region);
        }
    }
    Ok(segs)
}

fn required_bed_field<'a>(
    fields: &'a [&str],
    index: usize,
    field_name: &str,
    line_number: usize,
) -> Result<&'a str, io::Error> {
    fields.get(index).copied().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("BED line {line_number} is missing {field_name} column {}", index + 1),
        )
    })
}

fn parse_bed_i32(value: &str, field_name: &str, line_number: usize) -> Result<i32, io::Error> {
    value.parse::<i32>().map_err(|error| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("BED line {line_number} has invalid {field_name} value '{value}': {error}"),
        )
    })
}

fn correct_chromosome(chr_lengths: &HashMap<String, i32>, chromosome: &str) -> String {
    if chr_lengths.contains_key(chromosome) {
        chromosome.to_string()
    } else if let Some(stripped) = chromosome.strip_prefix("chr") {
        stripped.to_string()
    } else {
        format!("chr{chromosome}")
    }
}

fn column_index(column_number: usize, flag: &str) -> Result<usize, io::Error> {
    column_number.checked_sub(1).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{flag} column numbers are 1-based and must be >= 1"),
        )
    })
}

fn column_index_from_option(
    column_number: Option<usize>,
    flag: &str,
) -> Result<Option<usize>, io::Error> {
    column_number
        .map(|value| column_index(value, flag))
        .transpose()
}

/// Bridge VarDictJava's multi-char single-dash flags to clap long flags. clap cannot express
/// `-th`/`-UN`/`-DP`/`-VS` as shorts (it would split them into combined single-char flags), so we
/// rewrite the exact tokens to their `--` form before parsing. Values that follow are untouched.
fn normalize_java_flags<I>(iter: I) -> Vec<OsString>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    iter.into_iter()
        .map(Into::into)
        .map(|arg| match arg.as_os_str().to_str() {
            Some("-th") => OsString::from("--th"),
            Some("-UN") => OsString::from("--UN"),
            Some("-DP") => OsString::from("--DP"),
            Some("-VS") => OsString::from("--VS"),
            _ => arg,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cli() -> Cli {
        Cli {
            bam: String::from("reads.bam"),
            reference: String::from("ref.fa"),
            sample_name: String::from("sample"),
            chromosome_column: 1,
            start_column: 2,
            end_column: 3,
            ..Cli::default()
        }
    }

    fn cols_3() -> BedColumns {
        BedColumns {
            chromosome: 0,
            thick_start: 1,
            thick_end: 2,
            gene: Some(3),
        }
    }

    #[test]
    fn cli_parses_full_vdj_option_set() {
        // sanity: clap accepts the full set incl. bridged -UN/-DP/-VS and -h=header
        let cli = Cli::parse_from(normalize_java_flags(
            [
                "vardict_rs", "-G", "ref.fa", "-b", "t.bam", "-N", "s", "-U", "-Q", "30", "-F", "0",
                "-z", "0", "-a", "10:0.95", "--adaptor", "ACGTACGT", "-DP", "ERR", "-VS", "LENIENT",
                "-UN", "-h", "-3", "-x", "5", "regions.bed",
            ]
            .map(String::from),
        ));
        assert!(cli.disable_sv);
        assert_eq!(cli.mapping_quality, Some(30));
        assert_eq!(cli.samfilter.as_deref(), Some("0"));
        assert_eq!(cli.zero_based, Some(0));
        assert_eq!(cli.amplicon.as_deref(), Some("10:0.95"));
        assert_eq!(cli.default_printer.as_deref(), Some("ERR"));
        assert_eq!(cli.validation_stringency.as_deref(), Some("LENIENT"));
        assert!(cli.unique_mode_second_in_pair);
        assert!(cli.print_header);
        assert!(cli.move_indels_to_3);
        assert_eq!(cli.number_nucleotide_to_extend, Some(5));
    }

    #[test]
    fn build_config_disable_sv_and_mapping_quality() {
        let cli = Cli {
            disable_sv: true,
            mapping_quality: Some(30),
            ..sample_cli()
        };
        let config = build_config(&cli).unwrap();
        assert!(config.disable_sv);
        assert_eq!(config.mapping_quality, Some(30));
    }

    #[test]
    fn build_config_defaults_match_configuration_default() {
        // omitting every optional flag must leave Java-matching defaults
        let config = build_config(&sample_cli()).unwrap();
        let default = Configuration::default();
        assert_eq!(config.samfilter, default.samfilter);
        assert_eq!(config.qratio, default.qratio);
        assert_eq!(config.lofreq, default.lofreq);
        assert_eq!(config.reference_extension, default.reference_extension);
        assert!(!config.disable_sv);
        assert_eq!(config.zero_based, None);
    }

    #[test]
    fn pileup_flag_applies_cm_pileup_thresholds() {
        let mut config = Configuration {
            freq: 0.25,
            minr: 7,
            ..Configuration::default()
        };
        apply_pileup_preset(&mut config);
        assert!(config.do_pileup);
        assert_eq!(config.freq, -1.0);
        assert_eq!(config.minr, 0);
    }

    #[test]
    fn resolve_regions_rejects_missing_region_and_bed() {
        let cli = sample_cli();
        let config = Configuration::default();
        let error = resolve_regions(&cli, &config, &HashMap::default()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("Specify either -R <region> or a trailing BED path")
        );
    }

    #[test]
    fn build_regions_zero_based_toggles_start_increment() {
        let cols = cols_3();
        let lines = vec![(7usize, String::from("19\t60432\t61132\t"))];
        // zero-based (default for bedtools): start += 1
        let zb = build_regions(&lines, &cols, &Configuration::default(), &HashMap::default(), true)
            .unwrap();
        assert_eq!(zb[0].start, 60433);
        assert_eq!(zb[0].chr, "chr19");
        assert_eq!(zb[0].gene, "chr19");
        // one-based (VarDictJava default for explicit -S/-E BED): no increment
        let ob = build_regions(&lines, &cols, &Configuration::default(), &HashMap::default(), false)
            .unwrap();
        assert_eq!(ob[0].start, 60432);
    }

    #[test]
    fn build_regions_applies_extend() {
        let cols = cols_3();
        let lines = vec![(1usize, String::from("19\t100\t200\t"))];
        let config = Configuration {
            number_nucleotide_to_extend: 10,
            ..Configuration::default()
        };
        let regions = build_regions(&lines, &cols, &config, &HashMap::default(), false).unwrap();
        assert_eq!(regions[0].start, 90);
        assert_eq!(regions[0].end, 210);
    }

    #[test]
    fn build_regions_reports_invalid_integers() {
        let cols = cols_3();
        let lines = vec![(3usize, String::from("19\tnot-a-number\t61132"))];
        let error = build_regions(&lines, &cols, &Configuration::default(), &HashMap::default(), false)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("invalid start value 'not-a-number'"));
    }

    #[test]
    fn detect_amplicon_picks_up_8col_bed() {
        let config = Configuration::default();
        let lines = vec![(1usize, String::from("chr1\t100\t300\tg\t0\t+\t120\t280"))];
        let (amp, zero_based) = detect_amplicon(&config, &lines);
        assert_eq!(amp.as_deref(), Some("10:0.95"));
        assert!(zero_based); // amplicon BED defaults to zero-based when -z unset
    }

    #[test]
    fn build_adaptor_maps_builds_seeds() {
        let (fwd, rev) = build_adaptor_maps(&[String::from("ACGTACGTAC")]);
        // seq len 10, ADSEED 6 -> offsets i in [0,3] (i+6 < 10)
        assert_eq!(fwd.len(), 4);
        assert_eq!(rev.len(), 4);
        assert_eq!(fwd.get("ACGTAC").copied(), Some(1));
    }

    #[test]
    fn split_sample_name_handles_somatic_pair() {
        assert_eq!(
            split_sample_name("tumor|normal"),
            (String::from("tumor"), Some(String::from("normal")))
        );
        assert_eq!(split_sample_name("only"), (String::from("only"), None));
    }

    #[test]
    fn strip_regexp_slashes_strips_one_pair() {
        assert_eq!(strip_regexp_slashes("/foo/"), "foo");
        assert_eq!(strip_regexp_slashes("bar"), "bar");
    }
}
