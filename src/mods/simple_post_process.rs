use crate::config::Configuration;
use crate::data::{AlignedVarsData, Region, Variant, Vars};
use crate::mods::output::SimpleOutputVariant;
use crate::prelude::HashSet;
use crate::scope::{GlobalReadOnlyScope, Scope};
use std::panic;

/// Ported from: SimplePostProcessModule.accept()
/// Java source: SimplePostProcessModule.java:L33-L107
pub fn simple_post_process(scope: Scope<AlignedVarsData>) {
    let scope_instance = GlobalReadOnlyScope::instance();
    let conf = &scope_instance.conf;
    let Scope {
        region,
        splice,
        out,
        data,
        ..
    } = scope;

    let mut positions: Vec<i32> = data.aligned_variants.keys().copied().collect();
    positions.sort();

    for position in positions {
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let Some(variants_on_position) = data.aligned_variants.get(&position) else {
                return;
            };

            for line in simple_post_process_position_lines(
                position,
                variants_on_position,
                &region,
                &splice,
                conf,
            ) {
                out.print_owned_line(line);
            }
        }));
    }
}

/// Render one aligned position using SimplePostProcessModule.accept() rules.
/// Java source: SimplePostProcessModule.java:L43-L103
pub fn simple_post_process_position_lines(
    position: i32,
    variants_on_position: &Vars,
    region: &Region,
    splice: &HashSet<String>,
    conf: &Configuration,
) -> Vec<String> {
    if variants_on_position.sv.is_empty() && (position < region.start || position > region.end) {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut vrefs: Vec<Variant> = Vec::new();
    if variants_on_position.variants.is_empty() {
        if !conf.do_pileup {
            return lines;
        }
        if let Some(ref rv) = variants_on_position.reference_variant {
            let mut vref = rv.clone();
            vref.vartype.clear();
            vrefs.push(vref);
        } else {
            let output_variant =
                SimpleOutputVariant::new(None, region, &variants_on_position.sv, position);
            lines.push(output_variant.to_tsv_line(conf));
            return lines;
        }
    } else {
        let only_variant = variants_on_position.variants.len() == 1;
        let ref_var_owned = variants_on_position.reference_variant.clone();
        for &idx in &variants_on_position.variants {
            let vref = variants_on_position.arena[idx].clone();
            if vref.refallele.contains('N') {
                continue;
            }
            if vref.refallele == vref.varallele && !conf.do_pileup {
                continue;
            }
            if vref.start_position != position && conf.do_pileup && only_variant {
                if let Some(ref_var) = ref_var_owned.clone() {
                    let mut rv = ref_var;
                    rv.vartype.clear();
                    vrefs.push(rv);
                } else {
                    let output_variant =
                        SimpleOutputVariant::new(None, region, &variants_on_position.sv, position);
                    lines.push(output_variant.to_tsv_line(conf));
                    let mut rv = Variant::default();
                    rv.vartype.clear();
                    vrefs.push(rv);
                }
            }
            let vartype = vref.var_type();
            let is_good = vref.is_good_var(ref_var_owned.as_ref(), Some(&vartype), splice, conf);
            if !is_good && !conf.do_pileup {
                continue;
            }

            let mut owned_vref = vref;
            owned_vref.vartype = vartype;
            vrefs.push(owned_vref);
        }
    }

    for mut vref in vrefs {
        if vref.vartype == "Complex" {
            vref.adj_complex();
        }
        if conf.crispr_cutting_site == 0 {
            vref.crispr = 0;
        }
        let output_variant =
            SimpleOutputVariant::new(Some(&vref), region, &variants_on_position.sv, position);
        lines.push(output_variant.to_tsv_line(conf));
    }
    lines
}
