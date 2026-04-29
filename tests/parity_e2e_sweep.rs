use std::ffi::OsString;

use libtest_mimic::Arguments;

// parity_e2e_sweep: full-BAM e2e parity vs cached Java TSV.
// Phase 0a fixed sample naming to use the BAM basename, not `test_sample`.
// Regenerate goldens with: bash scripts/gen_e2e_sweep_golden.sh
// Shard runs with: VARDICT_E2E_SWEEP_SHARD=i/N cargo test --profile debug-release --test parity_e2e_sweep -- --include-ignored --test-threads=10
// Scope configs with: VARDICT_E2E_SWEEP_CONFIG=<config>
// Legacy exact selectors like hg002_sweep::parity_e2e_sweep_hg002 are rewritten
// to the corresponding chunk-trial prefix for backwards-compatible repro commands.
#[path = "common/mod.rs"]
mod common;

#[path = "parity_e2e_sweep/common.rs"]
mod sweep_common;

#[path = "parity_e2e_sweep/hg002_sweep.rs"]
mod hg002_sweep;

#[path = "parity_e2e_sweep/na12878_exome_sweep.rs"]
mod na12878_exome_sweep;

#[path = "parity_e2e_sweep/na12878_lowcov_sweep.rs"]
mod na12878_lowcov_sweep;

fn main() {
	sweep_common::reset_failure_count();

	let args = Arguments::from_iter(rewrite_legacy_exact_filter(std::env::args_os()));
	let mut trials = Vec::new();
	trials.extend(hg002_sweep::build_trials());
	trials.extend(na12878_exome_sweep::build_trials());
	trials.extend(na12878_lowcov_sweep::build_trials());
	trials.sort_by(|left, right| left.name().cmp(right.name()));

	libtest_mimic::run(&args, trials).exit();
}

fn rewrite_legacy_exact_filter<I>(iter: I) -> Vec<OsString>
where
	I: IntoIterator,
	I::Item: Into<OsString>,
{
	let mut args: Vec<OsString> = iter.into_iter().map(Into::into).collect();
	let Some(exact_index) = args.iter().position(|arg| arg == "--exact") else {
		return args;
	};

	let Some((filter_index, replacement)) = args
		.iter()
		.enumerate()
		.find_map(|(index, arg)| {
			arg.to_str()
				.and_then(sweep_common::legacy_selector_to_chunk_filter)
				.map(|replacement| (index, replacement))
		})
	else {
		return args;
	};

	args[filter_index] = replacement.into();
	args.remove(exact_index);
	args
}