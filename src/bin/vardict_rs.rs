use std::collections::HashMap;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind};
use std::path::Path;

use clap::Parser;
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::GlobalReadOnlyScope;
use vardict_rs::variations::{clear_variation_utils_scope, configure_variation_utils_scope};

#[derive(Debug, Parser)]
#[command(name = "vardict_rs")]
struct Cli {
    #[arg(short = 'R')]
    region: Option<String>,

    #[arg(short = 'b')]
    bam: String,

    #[arg(short = 'G')]
    reference: String,

    #[arg(short = 'N')]
    sample_name: String,

    #[arg(short = 'p')]
    pileup: bool,

    #[arg(short = 'f')]
    freq: Option<f64>,

    #[arg(short = 'r')]
    minr: Option<i32>,

    #[arg(short = 'p')]
    pileup: bool,

    #[arg(short = 'q')]
    goodq: Option<f64>,

    #[arg(short = 'm')]
    mismatch: Option<i32>,

    #[arg(short = 'X')]
    vext: Option<i32>,

    #[arg(short = 'B')]
    min_bias_reads: Option<i32>,

    #[arg(short = 'k')]
    realign: Option<i32>,

    #[arg(long = "fisher")]
    fisher: bool,

    #[arg(long = "th")]
    threads: Option<i32>,

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
    run(Cli::parse_from(normalize_java_thread_flag(
        std::env::args_os(),
    )))
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let requested_threads = cli
        .threads
        .and_then(|threads| usize::try_from(threads).ok())
        .unwrap_or(1);
    let fai_path = format!("{}.fai", cli.reference);
    let chr_lengths = load_chr_lengths(&fai_path)?;
    let regions = resolve_regions(&cli, &chr_lengths)?;

    let mut config = Configuration {
        bam: Some(BamNames::new(&cli.bam)),
        fasta: cli.reference.clone(),
        ..Configuration::default()
    };

    if cli.pileup {
        apply_pileup_preset(&mut config);
    } else {
        if let Some(freq) = cli.freq {
            config.freq = freq;
        }
        if let Some(minr) = cli.minr {
            config.minr = minr;
        }
    }
    if cli.pileup {
        config.do_pileup = true;
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
    if cli.fisher {
        config.fisher = true;
    }
    if let Some(threads) = cli.threads {
        config.threads = threads;
    }

    configure_variation_utils_scope(config.clone(), HashMap::new(), HashMap::new());

    GlobalReadOnlyScope::init(
        config,
        chr_lengths.clone(),
        cli.sample_name,
        None,
        None,
        HashMap::new(),
        HashMap::new(),
    );
    let _cleanup = ScopeCleanup;

    let reference_resource = ReferenceResource::new(&cli.reference, 1200, 0, chr_lengths, false);
    let simple_mode = SimpleMode::new(regions, reference_resource);
    if requested_threads > 1 {
        simple_mode.parallel(requested_threads);
    } else {
        simple_mode.not_parallel();
    }

    Ok(())
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

fn parse_region(region_str: &str) -> Result<Region, io::Error> {
    let (chr, range) = region_str.split_once(':').ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid region format (expected chr:start-end): {region_str}"),
        )
    })?;
    let (start_str, end_str) = range.split_once('-').ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid region range (expected start-end): {region_str}"),
        )
    })?;
    let start = start_str.parse::<i32>().map_err(|error| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid start in region {region_str}: {error}"),
        )
    })?;
    let end = end_str.parse::<i32>().map_err(|error| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid end in region {region_str}: {error}"),
        )
    })?;

    Ok(Region::new(chr, start, end, ""))
}

fn apply_pileup_preset(config: &mut Configuration) {
    config.do_pileup = true;
    config.freq = -1.0;
    config.minr = 0;
}

fn resolve_regions(
    cli: &Cli,
    chr_lengths: &HashMap<String, i32>,
) -> Result<Vec<Vec<Region>>, io::Error> {
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
            let mut region = parse_region(region_str)?;
            region.gene = region.chr.clone();
            Ok(vec![vec![region]])
        }
        (None, Some(bed_path)) => {
            let columns = BedColumns::from_cli(cli)?;
            let regions = parse_bed_regions(Path::new(bed_path), &columns, chr_lengths)?;
            Ok(vec![regions])
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BedColumns {
    chromosome: usize,
    start: usize,
    end: usize,
    gene: Option<usize>,
}

impl BedColumns {
    fn from_cli(cli: &Cli) -> Result<Self, io::Error> {
        let _ = column_index_from_option(cli.insert_start_column, "-s")?;
        let _ = column_index_from_option(cli.insert_end_column, "-e")?;

        Ok(Self {
            chromosome: column_index(cli.chromosome_column, "-c")?,
            start: column_index(cli.start_column, "-S")?,
            end: column_index(cli.end_column, "-E")?,
            gene: column_index_from_option(cli.gene_column, "-g")?,
        })
    }
}

/// Ported from: RegionBuilder.java:L41-L95
/// Implements the narrow BED parsing needed for VarDictJava-style CLI profiling.
fn parse_bed_regions(
    bed_path: &Path,
    columns: &BedColumns,
    chr_lengths: &HashMap<String, i32>,
) -> Result<Vec<Region>, io::Error> {
    let content = std::fs::read_to_string(bed_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Failed to read BED file {}: {error}", bed_path.display()),
        )
    })?;
    let mut regions = Vec::new();

    for (line_number, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        regions.push(parse_bed_line(
            line,
            line_number + 1,
            bed_path,
            columns,
            chr_lengths,
        )?);
    }

    if regions.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "BED file {} did not contain any regions",
                bed_path.display()
            ),
        ));
    }

    Ok(regions)
}

fn parse_bed_line(
    line: &str,
    line_number: usize,
    bed_path: &Path,
    columns: &BedColumns,
    chr_lengths: &HashMap<String, i32>,
) -> Result<Region, io::Error> {
    let fields: Vec<&str> = line.split('\t').collect();
    let chromosome = correct_chromosome(
        chr_lengths,
        required_bed_field(
            &fields,
            columns.chromosome,
            "chromosome",
            line_number,
            bed_path,
        )?,
    );
    let bed_start = parse_bed_i32(
        required_bed_field(&fields, columns.start, "start", line_number, bed_path)?,
        "start",
        line_number,
        bed_path,
    )?;
    let start = bed_start.checked_add(1).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "BED file {} line {} start overflows after BED-to-region conversion",
                bed_path.display(),
                line_number
            ),
        )
    })?;
    let end = parse_bed_i32(
        required_bed_field(&fields, columns.end, "end", line_number, bed_path)?,
        "end",
        line_number,
        bed_path,
    )?;
    let gene = columns
        .gene
        .and_then(|gene_idx| fields.get(gene_idx).map(|field| field.trim()))
        .filter(|field| !field.is_empty())
        .unwrap_or(&chromosome)
        .to_string();

    Ok(Region::new(chromosome, start, end, gene))
}

fn required_bed_field<'a>(
    fields: &'a [&str],
    index: usize,
    field_name: &str,
    line_number: usize,
    bed_path: &Path,
) -> Result<&'a str, io::Error> {
    fields.get(index).copied().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "BED file {} line {} is missing {field_name} column {}",
                bed_path.display(),
                line_number,
                index + 1
            ),
        )
    })
}

fn parse_bed_i32(
    value: &str,
    field_name: &str,
    line_number: usize,
    bed_path: &Path,
) -> Result<i32, io::Error> {
    value.parse::<i32>().map_err(|error| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "BED file {} line {} has invalid {field_name} value '{}': {error}",
                bed_path.display(),
                line_number,
                value
            ),
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

fn normalize_java_thread_flag<I>(iter: I) -> Vec<OsString>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    iter.into_iter()
        .map(Into::into)
        .map(|arg| {
            if arg.as_os_str() == OsStr::new("-th") {
                OsString::from("--th")
            } else {
                arg
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cli() -> Cli {
        Cli {
            region: None,
            bam: String::from("reads.bam"),
            reference: String::from("ref.fa"),
            sample_name: String::from("sample"),
            pileup: false,
            freq: None,
            minr: None,
            goodq: None,
            mismatch: None,
            vext: None,
            min_bias_reads: None,
            realign: None,
            fisher: false,
            threads: None,
            chromosome_column: 1,
            start_column: 2,
            end_column: 3,
            gene_column: None,
            insert_start_column: None,
            insert_end_column: None,
            bed_path: None,
        }
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
        let error = resolve_regions(&cli, &HashMap::new()).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("Specify either -R <region> or a trailing BED path")
        );
    }

    #[test]
    fn parse_bed_line_uses_selected_columns_and_chr_fallback_gene() {
        let columns = BedColumns {
            chromosome: 0,
            start: 1,
            end: 2,
            gene: Some(3),
        };
        let region = parse_bed_line(
            "19\t60432\t61132\t",
            7,
            Path::new("regions.bed"),
            &columns,
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(region.chr, "chr19");
        assert_eq!(region.start, 60433);
        assert_eq!(region.end, 61132);
        assert_eq!(region.gene, "chr19");
    }

    #[test]
    fn parse_bed_line_reports_invalid_integers_with_path_and_line() {
        let columns = BedColumns {
            chromosome: 0,
            start: 1,
            end: 2,
            gene: None,
        };
        let error = parse_bed_line(
            "19\tnot-a-number\t61132",
            3,
            Path::new("broken.bed"),
            &columns,
            &HashMap::new(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("BED file broken.bed line 3 has invalid start value 'not-a-number'")
        );
    }
}
