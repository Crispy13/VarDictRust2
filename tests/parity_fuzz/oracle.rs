//! Run VDJ/VDR over a synthetic region and compare their output modulo line
//! order and comment/blank lines — the same normalization `scripts/dual_run.py`
//! applies before comparing (see `scripts/dual_run.py:420-462`).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// Run VarDictJava (`-th 1`) over `region` and return its raw (un-normalized) stdout.
pub fn run_vdj(java_bin: &Path, ref_fasta: &Path, bam: &Path, region: &str, extra_flags: &[String]) -> String {
    run_tool(java_bin, "-th", ref_fasta, bam.as_os_str(), region, extra_flags)
}

/// Run vardict_rs (`--th 1`) over `region` and return its raw (un-normalized) stdout.
pub fn run_vdr(vdr_bin: &Path, ref_fasta: &Path, bam: &Path, region: &str, extra_flags: &[String]) -> String {
    run_tool(vdr_bin, "--th", ref_fasta, bam.as_os_str(), region, extra_flags)
}

/// Run VarDictJava in paired somatic mode over `region`: -b "tumor|normal".
pub fn run_vdj_paired(java_bin: &Path, ref_fasta: &Path, tumor_bam: &Path, normal_bam: &Path, region: &str, extra_flags: &[String]) -> String {
    run_tool(java_bin, "-th", ref_fasta, &paired_bam_arg(tumor_bam, normal_bam), region, extra_flags)
}

/// Run vardict_rs in paired somatic mode over `region`: -b "tumor|normal".
pub fn run_vdr_paired(vdr_bin: &Path, ref_fasta: &Path, tumor_bam: &Path, normal_bam: &Path, region: &str, extra_flags: &[String]) -> String {
    run_tool(vdr_bin, "--th", ref_fasta, &paired_bam_arg(tumor_bam, normal_bam), region, extra_flags)
}

/// Join two BAM paths into VarDict's pipe-separated paired -b argument.
fn paired_bam_arg(tumor_bam: &Path, normal_bam: &Path) -> std::ffi::OsString {
    let mut arg = tumor_bam.as_os_str().to_os_string();
    arg.push("|");
    arg.push(normal_bam.as_os_str());
    arg
}

fn run_tool(
    bin: &Path,
    threads_flag: &str,
    ref_fasta: &Path,
    bam_arg: &std::ffi::OsStr,
    region: &str,
    extra_flags: &[String],
) -> String {
    let mut cmd = Command::new(bin);
    cmd.arg("-G")
        .arg(ref_fasta)
        .arg("-b")
        .arg(bam_arg)
        .arg("-N")
        .arg("test_sample")
        .arg(threads_flag)
        .arg("1");
    for f in extra_flags {
        cmd.arg(f);
    }
    cmd.arg("-R").arg(region);
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("failed to spawn `{}`: {error}", bin.display()));

    assert!(
        output.status.success(),
        "`{}` failed for region {region} with {}\nSTDERR:\n{}\nSTDOUT:\n{}",
        bin.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Drop comment (`#`-prefixed) and blank lines, then sort — this absorbs
/// thread-order nondeterminism, matching `scripts/dual_run.py`'s normalization.
pub fn normalize(raw_stdout: &str) -> Vec<String> {
    let mut lines: Vec<String> = raw_stdout
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

/// Compare two tools' raw stdout after normalization. `Ok(())` if identical;
/// otherwise `Err` with a readable diff (lines unique to each side).
pub fn compare(vdj_raw_stdout: &str, vdr_raw_stdout: &str) -> Result<(), String> {
    let vdj_lines = normalize(vdj_raw_stdout);
    let vdr_lines = normalize(vdr_raw_stdout);

    if vdj_lines == vdr_lines {
        return Ok(());
    }

    let vdj_set: BTreeSet<&String> = vdj_lines.iter().collect();
    let vdr_set: BTreeSet<&String> = vdr_lines.iter().collect();

    let only_vdj: Vec<&&String> = vdj_set.difference(&vdr_set).collect();
    let only_vdr: Vec<&&String> = vdr_set.difference(&vdj_set).collect();

    Err(format!(
        "normalized output mismatch\nlines only in VDJ ({}):\n{}\nlines only in VDR ({}):\n{}",
        only_vdj.len(),
        only_vdj
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        only_vdr.len(),
        only_vdr
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

#[cfg(test)]
mod loop_proof {
    use super::compare;

    /// Same locus, one numeric field (var-cov) changed 30 -> 31 — mirrors the
    /// spike's known-divergent case. `compare` MUST report this as a mismatch,
    /// otherwise the fuzzer can never catch a real bug.
    #[test]
    fn compare_detects_known_divergence() {
        let vdj = "chrS\ttest_sample\tchrS:100-100\t100\t100\tA\tG\t30\t30\t15\t15\t14\t16\tSNV\t0.500\t0\t60\t0\t0\n";
        let vdr = "chrS\ttest_sample\tchrS:100-100\t100\t100\tA\tG\t30\t31\t15\t15\t14\t16\tSNV\t0.500\t0\t60\t0\t0\n";

        let result = compare(vdj, vdr);
        assert!(
            result.is_err(),
            "expected compare() to detect a numeric-field divergence, got Ok"
        );
    }

    /// Same three data lines, different order, different comment lines and
    /// blank-line placement. `compare` MUST treat these as equal.
    #[test]
    fn compare_absorbs_line_order_and_comments() {
        let vdj = "# VDJ header\nline_b\n\nline_a\nline_c\n";
        let vdr = "line_a\n# VDR header (different text)\nline_c\nline_b\n\n";

        let result = compare(vdj, vdr);
        assert!(
            result.is_ok(),
            "expected compare() to absorb line order/comment/blank differences, got {result:?}"
        );
    }
}
