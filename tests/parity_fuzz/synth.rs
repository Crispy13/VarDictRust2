//! Materialize a synthetic `Genome` to an indexed reference FASTA and a
//! coordinate-sorted, indexed BAM, by shelling out to `samtools`.
//!
//! Per the design brief, BAM synthesis goes through `samtools` (not
//! `rust-htslib::Writer`) so the fixture-building path matches the manual spike
//! that proved the whole loop.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::generator::{Genome, ReadRecord};

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

fn build_sam_text(contig: &str, seq_len: usize, reads: &[ReadRecord]) -> String {
    let mut sam = String::new();
    sam.push_str("@HD\tVN:1.6\tSO:coordinate\n");
    sam.push_str(&format!("@SQ\tSN:{contig}\tLN:{seq_len}\n"));

    for read in reads {
        let seq = std::str::from_utf8(&read.seq).expect("synthetic read seq is ASCII");
        let qual: String = std::iter::repeat((b'!' + read.base_qual) as char)
            .take(read.seq.len())
            .collect();
        sam.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t*\t0\t0\t{}\t{}\n",
            read.qname,
            read.flag,
            contig,
            read.pos,
            read.mapq,
            read.cigar,
            seq,
            qual,
        ));
    }

    sam
}

/// Write `<name>.sam` from `reads`, then samtools view|sort|index into `<name>.bam`.
/// Returns (bam_path, sam_text). `contig`/`seq_len` supply the @SQ header.
fn build_indexed_bam(
    dir: &Path,
    contig: &str,
    seq_len: usize,
    reads: &[ReadRecord],
    name: &str,
) -> (PathBuf, String) {
    let sam_text = build_sam_text(contig, seq_len, reads);
    let sam_path = dir.join(format!("{name}.sam"));
    std::fs::write(&sam_path, &sam_text).unwrap_or_else(|error| {
        panic!("failed to write {}: {error}", sam_path.display())
    });

    let unsorted_bam = format!("{name}.unsorted.bam");
    let sorted_bam = format!("{name}.bam");
    run_samtools(
        &["view", "-b", "-o", &unsorted_bam, &format!("{name}.sam")],
        dir,
    );
    run_samtools(&["sort", "-o", &sorted_bam, &unsorted_bam], dir);
    run_samtools(&["index", &sorted_bam], dir);

    (dir.join(sorted_bam), sam_text)
}

/// Write `ref.fa`(+`.fai`) and a sorted+indexed `reads.bam` for `genome` into a
/// fresh temp directory, returning the paths needed to run either tool.
pub fn materialize(genome: &Genome) -> SynthPaths {
    let tempdir = tempfile::tempdir().expect("create tempdir for synthetic fixture");
    let dir = tempdir.path();

    let ref_fasta = dir.join("ref.fa");
    write_fasta(&ref_fasta, &genome.contig, &genome.sequence);
    run_samtools(&["faidx", "ref.fa"], dir);

    let (reads_bam, reads_sam_text) = build_indexed_bam(
        dir,
        &genome.contig,
        genome.sequence.len(),
        &genome.reads,
        "reads",
    );

    let region = format!("{}:{}-{}", genome.contig, genome.scan_start, genome.scan_end);

    SynthPaths {
        _tempdir: tempdir,
        ref_fasta,
        reads_bam,
        region,
        reads_sam_text,
    }
}

/// Paths (plus the backing tempdir, kept alive) for one materialized paired
/// tumor/normal somatic genome.
pub struct SomaticSynthPaths {
    /// Keeps the temp directory alive for the lifetime of this struct; the
    /// directory (and everything under it) is deleted on drop.
    _tempdir: tempfile::TempDir,
    pub ref_fasta: PathBuf,
    pub tumor_bam: PathBuf,
    pub normal_bam: PathBuf,
    /// `<contig>:<scan_start>-<scan_end>`, ready to pass as `-R` to either binary.
    pub region: String,
    /// The exact SAM text used to build `tumor_bam`, kept around so a failing
    /// proptest case can print a reproducer.
    pub tumor_sam_text: String,
    /// The exact SAM text used to build `normal_bam`, kept around so a failing
    /// proptest case can print a reproducer.
    pub normal_sam_text: String,
}

/// Write `ref.fa`(+`.fai`) and sorted+indexed `tumor.bam`/`normal.bam` for
/// `genome` into a fresh temp directory, returning the paths needed to run
/// either tool in paired somatic mode.
pub fn materialize_paired(genome: &super::generator::SomaticGenome) -> SomaticSynthPaths {
    let tempdir = tempfile::tempdir().expect("create tempdir for synthetic fixture");
    let dir = tempdir.path();

    let ref_fasta = dir.join("ref.fa");
    write_fasta(&ref_fasta, &genome.contig, &genome.sequence);
    run_samtools(&["faidx", "ref.fa"], dir);

    let (tumor_bam, tumor_sam_text) = build_indexed_bam(
        dir,
        &genome.contig,
        genome.sequence.len(),
        &genome.tumor_reads,
        "tumor",
    );
    let (normal_bam, normal_sam_text) = build_indexed_bam(
        dir,
        &genome.contig,
        genome.sequence.len(),
        &genome.normal_reads,
        "normal",
    );

    let region = format!("{}:{}-{}", genome.contig, genome.scan_start, genome.scan_end);

    SomaticSynthPaths {
        _tempdir: tempdir,
        ref_fasta,
        tumor_bam,
        normal_bam,
        region,
        tumor_sam_text,
        normal_sam_text,
    }
}
