use crate::data::{AlignedVarsData, Variant};
use crate::mods::output::SimpleOutputVariant;
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

            if variants_on_position.sv.is_empty()
                && (position < region.start || position > region.end)
            {
                return;
            }

            let mut vrefs: Vec<Variant> = Vec::new();
            if variants_on_position.variants.is_empty() {
                if !conf.do_pileup {
                    return;
                }
                if let Some(mut vref) = variants_on_position.reference_variant.clone() {
                    vref.vartype.clear();
                    vrefs.push(vref);
                } else {
                    let output_variant =
                        SimpleOutputVariant::new(None, &region, &variants_on_position.sv, position);
                    out.print_line(&output_variant.to_tsv_line(&conf));
                    return;
                }
            } else {
                let only_variant = variants_on_position.variants.len() == 1;
                for vref in &variants_on_position.variants {
                    if vref.refallele.contains('N') {
                        continue;
                    }
                    if vref.refallele == vref.varallele && !conf.do_pileup {
                        continue;
                    }
                    if vref.start_position != position && conf.do_pileup && only_variant {
                        if let Some(mut ref_var) = variants_on_position.reference_variant.clone() {
                            ref_var.vartype.clear();
                            vrefs.push(ref_var);
                        } else {
                            let output_variant = SimpleOutputVariant::new(
                                None,
                                &region,
                                &variants_on_position.sv,
                                position,
                            );
                            out.print_line(&output_variant.to_tsv_line(&conf));
                            let mut ref_var = Variant::default();
                            ref_var.vartype.clear();
                            vrefs.push(ref_var);
                        }
                    }
                    let vartype = vref.var_type();
                    let is_good = vref.is_good_var(
                        variants_on_position.reference_variant.as_ref(),
                        Some(&vartype),
                        &splice,
                        &conf,
                    );
                    if !is_good && !conf.do_pileup {
                        continue;
                    }

                    let mut owned_vref = vref.clone();
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
                let output_variant = SimpleOutputVariant::new(
                    Some(&vref),
                    &region,
                    &variants_on_position.sv,
                    position,
                );
                out.print_line(&output_variant.to_tsv_line(&conf));
            }
        }));
    }
}
