//! Materialize a synthetic `Genome` to an indexed reference FASTA and a
//! coordinate-sorted, indexed BAM, by shelling out to `samtools`.
//!
//! Per the design brief, BAM synthesis goes through `samtools` (not
//! `rust-htslib::Writer`) so the fixture-building path matches the manual spike
//! that proved the whole loop.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::generator::Genome;

/// Paths (plus the backing tempdir, kept alive) for one materialized genome.
pub struct SynthPaths {
    /// Keeps the temp directory alive for the lifetime of this struct; the
    /// directory (and everything under it) is deleted on drop.
    _tempdir: tempfile::TempDir,
    pub ref_fasta: PathBuf,
    pub reads_bam: PathBuf,
    /// `<contig>:1-<len>`, ready to pass as `-R` to either binary.
    pub region: String,
    /// The exact SAM text used to build `reads_bam`, kept around so a failing
    /// proptest case can print a reproducer.
    pub reads_sam_text: String,
}

fn samtools_bin() -> PathBuf {
    std::env::var_os("SAMTOOLS_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("samtools"))
}

fn run_samtools(args: &[&str], cwd: &Path) {
    let samtools = samtools_bin();
    let output = Command::new(&samtools)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| {
            panic!("failed to spawn `{}` {args:?}: {error}", samtools.display())
        });

    // Only surface stderr on failure: samtools routinely prints harmless
    // htslib version banners on success that would otherwise spam every case.
    assert!(
        output.status.success(),
        "`{}` {args:?} failed with {}\nSTDERR:\n{}\nSTDOUT:\n{}",
        samtools.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
}

fn write_fasta(path: &Path, contig: &str, sequence: &[u8]) {
    let mut file = std::fs::File::create(path)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", path.display()));
    writeln!(file, ">{contig}").expect("write fasta header");
    for line in sequence.chunks(60) {
        file.write_all(line).expect("write fasta sequence line");
        file.write_all(b"\n").expect("write fasta newline");
    }
}

fn build_sam_text(genome: &Genome) -> String {
    let mut sam = String::new();
    sam.push_str("@HD\tVN:1.6\tSO:coordinate\n");
    sam.push_str(&format!(
        "@SQ\tSN:{}\tLN:{}\n",
        genome.contig,
        genome.sequence.len()
    ));

    for read in &genome.reads {
        let seq = std::str::from_utf8(&read.seq).expect("synthetic read seq is ASCII");
        let qual: String = std::iter::repeat((b'!' + read.base_qual) as char)
            .take(read.seq.len())
            .collect();
        sam.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t*\t0\t0\t{}\t{}\n",
            read.qname,
            read.flag,
            genome.contig,
            read.pos,
            read.mapq,
            read.cigar,
            seq,
            qual,
        ));
    }

    sam
}

/// Write `ref.fa`(+`.fai`) and a sorted+indexed `reads.bam` for `genome` into a
/// fresh temp directory, returning the paths needed to run either tool.
pub fn materialize(genome: &Genome) -> SynthPaths {
    let tempdir = tempfile::tempdir().expect("create tempdir for synthetic fixture");
    let dir = tempdir.path();

    let ref_fasta = dir.join("ref.fa");
    write_fasta(&ref_fasta, &genome.contig, &genome.sequence);
    run_samtools(&["faidx", "ref.fa"], dir);

    let reads_sam_text = build_sam_text(genome);
    let reads_sam = dir.join("reads.sam");
    std::fs::write(&reads_sam, &reads_sam_text).expect("write reads.sam");

    run_samtools(
        &["view", "-b", "-o", "reads.unsorted.bam", "reads.sam"],
        dir,
    );
    run_samtools(&["sort", "-o", "reads.bam", "reads.unsorted.bam"], dir);
    run_samtools(&["index", "reads.bam"], dir);

    let region = format!("{}:{}-{}", genome.contig, genome.scan_start, genome.scan_end);
    let reads_bam = dir.join("reads.bam");

    SynthPaths {
        _tempdir: tempdir,
        ref_fasta,
        reads_bam,
        region,
        reads_sam_text,
    }
}
