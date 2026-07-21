//! Differential parity fuzzer -- germline (SNV / indel / MNV / clips /
//! flag-filtered reads / region boundaries).
//!
//! A manual spike proved the loop: a synthetic reference FASTA + a tiny
//! sorted/indexed BAM (one contig, one variant) run through both VarDictJava
//! (VDJ) and vardict_rs (VDR) produce byte-identical output after
//! sort-normalizing stdout. A second manual spike (`gen_indel_spike.py`)
//! proved the CIGAR/SEQ/POS encoding for deletions (`<a>M<len>D<b>M`) and
//! insertions (`<a>M<len>I<b>M`). A third manual spike (`gen_mnv_spike.py`)
//! proved the encoding for multi-nucleotide variants: `k` adjacent base
//! substitutions at the locus's 1-based `pos`, CIGAR staying `<READ_LEN>M`
//! (VarDict reports these as `Complex`). A fourth manual spike
//! (`gen_clip_spike.py`) proved read-level soft/hard clips (5' `<c>S<m>M` /
//! `<c>H<m>M`, 3' `<m>M<c>S` / `<m>M<c>H`) stay byte-identical layered on top
//! of an SNV or deletion locus -- a clip is a per-read modifier orthogonal to
//! `VariantKind`, applied by `generator::apply_clip` to a subset of a locus's
//! reads. Further spikes proved read-flag filtering (duplicate/secondary/
//! supplementary noise reads are skipped identically) and region-boundary
//! inclusion (a variant at the exact `-R` edge is included/excluded the same
//! way by both tools). This test drives the loop with proptest so it can
//! generate (and shrink) many synthetic cases automatically, each locus
//! independently an SNV, deletion, insertion, or MNV, optionally with a
//! clipped read subset and/or skipped flag-filtered noise reads, over a scan
//! region that is sometimes cropped to the loci edges. Each locus may also
//! carry a minority of quality-noise reads whose MAPQ and base quality
//! straddle common `-Q`/`-q` filter floors, so those preset filters actually
//! engage and both tools are checked to drop exactly the same reads. A second test,
//! `pbt_germline_preset_parity`, runs every generated genome again under a
//! curated germline config preset (via `generator::arb_germline_preset`),
//! passing the preset's CLI flags through to both tools. A third test,
//! `pbt_somatic_parity`, exercises the paired tumor/normal somatic lane: two
//! synthetic BAMs (tumor + normal) built from one shared reference, run
//! through both tools via VarDict's paired `-b "tumor.bam|normal.bam"` mode.
//! Each somatic locus independently an SNV, deletion, insertion, or MNV, with
//! the normal sample's own alt fraction spanning the full somatic status
//! spectrum (StrongSomatic/LikelySomatic/AFDiff/Germline/LikelyLOH). A fourth
//! test, `pbt_somatic_preset_parity`, runs the paired somatic lane again under
//! a curated somatic config preset (via `generator::arb_somatic_preset`, the
//! germline curated set plus somatic-only `-V`/`-I` presets).
//!
//!   generate `Vec<Locus>` (generator.rs)
//!     -> materialize ref.fa + sorted/indexed reads.bam via samtools (synth.rs)
//!     -> run both binaries over one `-R chrS:1-LEN` region, threads pinned to 1
//!     -> normalize + compare stdout (oracle.rs)
//!
//! Requires the `vdr` conda env active (provides `samtools`) and the VDR binary
//! built ahead of time:
//!   cargo build --profile debug-release --bin vardict_rs
//!
//! Run with a small case count first to keep wall time down (each case shells
//! out to the JVM, ~1-2s):
//!   PARITY_FUZZ_CASES=8 cargo test --profile debug-release --test parity_fuzz
//!
//! On failure, proptest auto-writes `tests/parity_fuzz.proptest-regressions`
//! (checked-in corpus) and the panic message includes the failing loci and the
//! exact `reads.sam` text needed to reproduce.
//!
//! VDR binary path defaults to `<repo>/target/debug-release/vardict_rs`;
//! override with `VARDICT_RS_BIN`.

#[path = "common/mod.rs"]
mod common;

#[path = "parity_fuzz/generator.rs"]
mod generator;
#[path = "parity_fuzz/oracle.rs"]
mod oracle;
#[path = "parity_fuzz/synth.rs"]
mod synth;

use std::path::PathBuf;

use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn vdr_binary_path() -> PathBuf {
    std::env::var_os("VARDICT_RS_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_root().join("target/debug-release/vardict_rs"))
}

fn fuzz_cases() -> u32 {
    std::env::var("PARITY_FUZZ_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: fuzz_cases(),
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct("tests/parity_fuzz.proptest-regressions"),
        )),
        ..ProptestConfig::default()
    })]

    #[ignore = "requires vdr conda env (samtools) + built target/debug-release/vardict_rs + VarDictJava@4e362c0; run: PARITY_FUZZ_CASES=64 cargo test --profile debug-release --test parity_fuzz -- --include-ignored"]
    #[test]
    fn pbt_germline_snv_parity(genome in generator::arb_genome()) {
        let vdr_bin = vdr_binary_path();
        assert!(
            vdr_bin.is_file(),
            "VDR binary not found at {}. Build with: cargo build --profile debug-release --bin vardict_rs (or set VARDICT_RS_BIN)",
            vdr_bin.display(),
        );
        let java_bin = common::java_binary_path();

        let synth = synth::materialize(&genome);

        let vdj_out = oracle::run_vdj(&java_bin, &synth.ref_fasta, &synth.reads_bam, &synth.region, &[]);
        let vdr_out = oracle::run_vdr(&vdr_bin, &synth.ref_fasta, &synth.reads_bam, &synth.region, &[]);

        // Guard against vacuous parity: if VDJ (the reference oracle) called no
        // variants, an empty == empty comparison would pass silently and give
        // false confidence. Every generated locus is built to be callable
        // (depth 30..=60, alt fraction 30..=70%, >=1 alt read), so an empty VDJ
        // table means the generator drifted into an uncallable regime -- surface
        // it rather than let it masquerade as parity.
        prop_assert!(
            !oracle::normalize(&vdj_out).is_empty(),
            "vacuous case: VDJ called no variants for region {}\nloci: {:#?}\n\nreads.sam:\n{}",
            synth.region,
            genome.loci,
            synth.reads_sam_text,
        );

        if let Err(diff) = oracle::compare(&vdj_out, &vdr_out) {
            prop_assert!(
                false,
                "Parity mismatch for region {}\nloci: {:#?}\n\nreads.sam:\n{}\n\n{}\n\nVDJ stdout:\n{}\n\nVDR stdout:\n{}",
                synth.region,
                genome.loci,
                synth.reads_sam_text,
                diff,
                vdj_out,
                vdr_out,
            );
        }
    }

    #[ignore = "requires vdr conda env (samtools) + built target/debug-release/vardict_rs + VarDictJava@4e362c0; run: PARITY_FUZZ_CASES=64 cargo test --profile debug-release --test parity_fuzz -- --include-ignored"]
    #[test]
    fn pbt_germline_preset_parity(
        genome in generator::arb_genome(),
        preset in generator::arb_germline_preset(),
    ) {
        let vdr_bin = vdr_binary_path();
        assert!(
            vdr_bin.is_file(),
            "VDR binary not found at {}. Build with: cargo build --profile debug-release --bin vardict_rs (or set VARDICT_RS_BIN)",
            vdr_bin.display(),
        );
        let java_bin = common::java_binary_path();

        let synth = synth::materialize(&genome);

        let vdj_out = oracle::run_vdj(&java_bin, &synth.ref_fasta, &synth.reads_bam, &synth.region, &preset.flags);
        let vdr_out = oracle::run_vdr(&vdr_bin, &synth.ref_fasta, &synth.reads_bam, &synth.region, &preset.flags);

        // Real parity check FIRST — always catches a true divergence, including the
        // one-empty-one-not case, before any vacuity handling.
        if let Err(diff) = oracle::compare(&vdj_out, &vdr_out) {
            prop_assert!(
                false,
                "Parity mismatch under preset {} ({:?}) for region {}\nloci: {:#?}\n\nreads.sam:\n{}\n\n{}\n\nVDJ stdout:\n{}\n\nVDR stdout:\n{}",
                preset.name, preset.flags, synth.region, genome.loci, synth.reads_sam_text, diff, vdj_out, vdr_out,
            );
        }

        // Discard (do NOT fail) cases the preset legitimately suppressed to nothing:
        // both-empty is genuine parity but uninformative, and some presets (e.g. -T
        // trim near the variant) can zero out a uniform-quality case. prop_assume
        // keeps the case budget spent on informative, non-vacuous parity checks.
        prop_assume!(!oracle::normalize(&vdj_out).is_empty());
    }

    #[ignore = "requires vdr conda env (samtools) + built target/debug-release/vardict_rs + VarDictJava@4e362c0; run: PARITY_FUZZ_CASES=64 cargo test --profile debug-release --test parity_fuzz -- --include-ignored"]
    #[test]
    fn pbt_unique_mode_paired_parity(
        genome in generator::arb_paired_genome(),
        preset in generator::arb_unique_mode_preset(),
    ) {
        let vdr_bin = vdr_binary_path();
        assert!(
            vdr_bin.is_file(),
            "VDR binary not found at {}. Build with: cargo build --profile debug-release --bin vardict_rs (or set VARDICT_RS_BIN)",
            vdr_bin.display(),
        );
        let java_bin = common::java_binary_path();

        let synth = synth::materialize(&genome);

        let vdj_out = oracle::run_vdj(&java_bin, &synth.ref_fasta, &synth.reads_bam, &synth.region, &preset.flags);
        let vdr_out = oracle::run_vdr(&vdr_bin, &synth.ref_fasta, &synth.reads_bam, &synth.region, &preset.flags);

        // Real parity check FIRST — catches any true divergence in the overlap-dedup
        // (skipOverlappingReads) path exercised only by `-u`/`--UN` on paired reads.
        if let Err(diff) = oracle::compare(&vdj_out, &vdr_out) {
            prop_assert!(
                false,
                "Parity mismatch under unique-mode preset {} ({:?}) for region {}\nloci: {:#?}\n\nreads.sam:\n{}\n\n{}\n\nVDJ stdout:\n{}\n\nVDR stdout:\n{}",
                preset.name, preset.flags, synth.region, genome.loci, synth.reads_sam_text, diff, vdj_out, vdr_out,
            );
        }

        // Discard cases the preset suppressed to nothing (e.g. aggressive dedup on a
        // tiny genome). Default preset always calls, so the lane stays non-vacuous.
        prop_assume!(!oracle::normalize(&vdj_out).is_empty());
    }

    #[ignore = "requires vdr conda env (samtools) + built target/debug-release/vardict_rs + VarDictJava@4e362c0; run: PARITY_FUZZ_CASES=64 cargo test --profile debug-release --test parity_fuzz -- --include-ignored"]
    #[test]
    fn pbt_somatic_parity(sgenome in generator::arb_somatic_genome()) {
        let vdr_bin = vdr_binary_path();
        assert!(
            vdr_bin.is_file(),
            "VDR binary not found at {}. Build with: cargo build --profile debug-release --bin vardict_rs (or set VARDICT_RS_BIN)",
            vdr_bin.display(),
        );
        let java_bin = common::java_binary_path();

        let synth = synth::materialize_paired(&sgenome);

        let vdj_out = oracle::run_vdj_paired(&java_bin, &synth.ref_fasta, &synth.tumor_bam, &synth.normal_bam, &synth.region, &[]);
        let vdr_out = oracle::run_vdr_paired(&vdr_bin, &synth.ref_fasta, &synth.tumor_bam, &synth.normal_bam, &synth.region, &[]);

        // Non-vacuity: every StrongSomatic locus has a clear tumor variant, so VDJ
        // (the oracle) must emit at least one somatic row. (The NORMAL bam is
        // legitimately variant-free, so the germline "both bams non-empty" guard
        // does not apply -- guard on the joint paired stdout instead.)
        prop_assert!(
            !oracle::normalize(&vdj_out).is_empty(),
            "vacuous somatic case: VDJ called no variants for region {}\ntumor.sam:\n{}\nnormal.sam:\n{}",
            synth.region, synth.tumor_sam_text, synth.normal_sam_text,
        );

        if let Err(diff) = oracle::compare(&vdj_out, &vdr_out) {
            prop_assert!(false,
                "Somatic parity mismatch for region {}\nloci: {:#?}\n\ntumor.sam:\n{}\nnormal.sam:\n{}\n\n{}\n\nVDJ:\n{}\n\nVDR:\n{}",
                synth.region, sgenome.loci, synth.tumor_sam_text, synth.normal_sam_text, diff, vdj_out, vdr_out);
        }
    }

    #[ignore = "requires vdr conda env (samtools) + built target/debug-release/vardict_rs + VarDictJava@4e362c0; run: PARITY_FUZZ_CASES=64 cargo test --profile debug-release --test parity_fuzz -- --include-ignored"]
    #[test]
    fn pbt_somatic_preset_parity(
        sgenome in generator::arb_somatic_genome(),
        preset in generator::arb_somatic_preset(),
    ) {
        let vdr_bin = vdr_binary_path();
        assert!(vdr_bin.is_file(), "VDR binary not found at {}. Build with: cargo build --profile debug-release --bin vardict_rs (or set VARDICT_RS_BIN)", vdr_bin.display());
        let java_bin = common::java_binary_path();

        let synth = synth::materialize_paired(&sgenome);

        let vdj_out = oracle::run_vdj_paired(&java_bin, &synth.ref_fasta, &synth.tumor_bam, &synth.normal_bam, &synth.region, &preset.flags);
        let vdr_out = oracle::run_vdr_paired(&vdr_bin, &synth.ref_fasta, &synth.tumor_bam, &synth.normal_bam, &synth.region, &preset.flags);

        // Real parity check FIRST (catches one-empty-one-not), then discard cases a
        // preset legitimately suppressed to nothing.
        if let Err(diff) = oracle::compare(&vdj_out, &vdr_out) {
            prop_assert!(false,
                "Somatic preset parity mismatch under {} ({:?}) for region {}\nloci: {:#?}\n\ntumor.sam:\n{}\nnormal.sam:\n{}\n\n{}\n\nVDJ:\n{}\n\nVDR:\n{}",
                preset.name, preset.flags, synth.region, sgenome.loci, synth.tumor_sam_text, synth.normal_sam_text, diff, vdj_out, vdr_out);
        }
        prop_assume!(!oracle::normalize(&vdj_out).is_empty());
    }
}
