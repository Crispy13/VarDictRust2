//! M7 dual-run emitter.
//!
//! This test is driven by `scripts/dual_run.py` to produce Rust-side TSV
//! output for ad-hoc regions. It is ignored by default; invoke with
//!   DUAL_REGION=chr:start-end DUAL_BAM=... DUAL_REF=... DUAL_OUT=/tmp/out.tsv \
//!     cargo test --test dual_run_emit dual_run_emit -- --include-ignored --exact
//!
//! The test ALWAYS passes — any mismatch detection is the driver's job.

mod common;

use std::sync::{Arc, Mutex};

use vardict_rs::modes::SimpleMode;
use vardict_rs::reference::ReferenceResource;
use vardict_rs::scope::{GlobalReadOnlyScope, VariantPrinter};

#[test]
#[ignore = "Dual-run emit harness — driven by scripts/dual_run.py via DUAL_* env vars"]
fn dual_run_emit() {
    let region_str = std::env::var("DUAL_REGION").expect("DUAL_REGION env var required");
    let bam_path = std::env::var("DUAL_BAM").expect("DUAL_BAM env var required");
    let ref_path = std::env::var("DUAL_REF").expect("DUAL_REF env var required");
    let out_path = std::env::var("DUAL_OUT").expect("DUAL_OUT env var required");

    let fai_path = format!("{ref_path}.fai");
    let chr_lengths = common::load_chr_lengths(&fai_path);
    let _guard =
        common::init_test_scope_with_bam(&bam_path, &ref_path, chr_lengths.clone());

    let mut region = common::parse_region(&region_str);
    region.gene = region.chr.clone();

    let reference_resource = ReferenceResource::new(&ref_path, 1200, 0, chr_lengths, false);
    let simple_mode = SimpleMode::new(vec![vec![region]], reference_resource);
    let captured = Arc::new(Mutex::new(String::new()));
    GlobalReadOnlyScope::set_variant_printer(VariantPrinter::Buffer(captured.clone()));
    simple_mode.not_parallel();
    let output = {
        let mut guard = captured.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    GlobalReadOnlyScope::clear();

    std::fs::write(&out_path, output)
        .unwrap_or_else(|e| panic!("failed to write Rust output to {out_path}: {e}"));
    eprintln!("dual_run_emit wrote {out_path}");
}
