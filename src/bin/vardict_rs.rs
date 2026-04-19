use std::collections::HashMap;
use std::error::Error;
use std::io::{self, ErrorKind};

use clap::Parser;
use vardict_rs::config::{BamNames, Configuration};
use vardict_rs::data::Region;
use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::GlobalReadOnlyScope;

#[derive(Debug, Parser)]
#[command(name = "vardict_rs")]
struct Cli {
    #[arg(short = 'R')]
    region: String,

    #[arg(short = 'b')]
    bam: String,

    #[arg(short = 'G')]
    reference: String,

    #[arg(short = 'N')]
    sample_name: String,

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

    #[arg(long = "th")]
    threads: Option<i32>,
}

struct ScopeCleanup;

impl Drop for ScopeCleanup {
    fn drop(&mut self) {
        GlobalReadOnlyScope::clear()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    run(Cli::parse())
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let fai_path = format!("{}.fai", cli.reference);
    let chr_lengths = load_chr_lengths(&fai_path)?;

    let mut config = Configuration {
        bam: Some(BamNames::new(&cli.bam)),
        fasta: cli.reference.clone(),
        ..Configuration::default()
    };

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
    if let Some(threads) = cli.threads {
        config.threads = threads;
    }

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

    let mut region = parse_region(&cli.region)?;
    region.gene = region.chr.clone();

    let reference_resource = ReferenceResource::new(&cli.reference, 1200, 0, chr_lengths, false);
    let simple_mode = SimpleMode::new(vec![vec![region]], reference_resource);
    simple_mode.not_parallel();

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