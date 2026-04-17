use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::data::{
    AlignedVarsData, InitialData, RealignedVariationData, Region, Sclip, VariationMap,
};
use crate::mods::amplicon_post_process::amplicon_post_process;
use crate::mods::cigar_parser::CigarParser;
use crate::mods::sam_file_parser::sam_file_parser_process;
use crate::mods::simple_post_process::simple_post_process;
use crate::mods::somatic_post_process::somatic_post_process;
use crate::mods::{structural_variants_processor, to_vars_builder, variation_realigner};
use crate::reference::{Reference, ReferenceResource};
use crate::scope::{AbstractMode, GlobalReadOnlyScope, Scope};
use crate::utils::tsv_join;

fn try_to_get_reference(reference_resource: &ReferenceResource, region: &Region) -> Reference {
    reference_resource
        .get_reference(region)
        .unwrap_or_else(|error| {
            panic!(
                "Failed to fetch reference for {}: {}",
                region.print_region(),
                error
            )
        })
}

fn run_pipeline(scope: Scope<InitialData>) -> Scope<AlignedVarsData> {
    let parsed_scope = sam_file_parser_process(scope);
    let Scope {
        bam,
        region,
        region_ref,
        reference_resource,
        max_read_length,
        splice,
        out,
        mut data,
    } = parsed_scope;

    let chr_name = data.get_chr_name();
    let header = data
        .header_view()
        .expect("current BAM header must exist before CIGAR parsing");
    let InitialData {
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_5_end,
        soft_clips_3_end,
    } = std::mem::take(&mut data.initial_data);
    let total_reads = data.total_reads;
    let duplicate_reads = data.duplicate_reads;
    let mut parser = CigarParser::new(false);
    parser.init_from_scope(
        &region,
        &region_ref,
        &splice,
        max_read_length,
        non_insertion_variants,
        insertion_variants,
        ref_coverage,
        soft_clips_3_end,
        soft_clips_5_end,
        total_reads,
        duplicate_reads,
    );
    let mut records = std::iter::from_fn(|| data.next_record());
    let variation_data = parser.process(&mut records, &header, &chr_name);
    data.close();

    let variation_scope = Scope {
        bam,
        region,
        region_ref,
        reference_resource,
        max_read_length: variation_data.max_read_length.unwrap_or(max_read_length),
        splice,
        out,
        data: variation_data,
    };
    let realigned_scope = variation_realigner::process(variation_scope);
    finalize_pipeline(realigned_scope)
}

fn finalize_pipeline(scope: Scope<RealignedVariationData>) -> Scope<AlignedVarsData> {
    let Scope {
        bam,
        region,
        region_ref,
        reference_resource,
        max_read_length,
        splice,
        out,
        mut data,
    } = scope;

    let mut reference = (*region_ref).clone();
    let bams = if bam.is_empty() {
        None
    } else {
        Some(bam.split(':').map(ToString::to_string).collect::<Vec<_>>())
    };
    let splice_btree = if splice.is_empty() {
        None
    } else {
        Some(splice.iter().cloned().collect::<BTreeSet<_>>())
    };
    let mut prev_non_insertion_variants: HashMap<i32, VariationMap> = HashMap::new();
    let mut prev_ref_coverage: HashMap<i32, i32> = HashMap::new();
    let mut prev_soft_clips_3_end: HashMap<i32, Sclip> = HashMap::new();
    let mut prev_soft_clips_5_end: HashMap<i32, Sclip> = HashMap::new();
    let prev_reference_sequences: HashMap<i32, u8> = HashMap::new();
    structural_variants_processor::process(
        &mut data,
        &mut reference,
        &reference_resource,
        &region,
        &bams,
        &splice_btree,
        &mut prev_non_insertion_variants,
        &mut prev_ref_coverage,
        &mut prev_soft_clips_3_end,
        &mut prev_soft_clips_5_end,
        &prev_reference_sequences,
        "",
        0,
    );

    let aligned_data = to_vars_builder::process(
        data.max_read_length.unwrap_or(max_read_length),
        &region,
        &reference.reference_sequences,
        &data.ref_coverage,
        &data.insertion_variants,
        &mut data.non_insertion_variants,
        data.duprate,
    );

    Scope {
        bam,
        region,
        region_ref: Arc::new(reference),
        reference_resource,
        max_read_length: aligned_data.max_read_length,
        splice,
        out,
        data: aligned_data,
    }
}

/// Ported from: SimpleMode.java:L25-L118
#[derive(Clone, Debug)]
pub struct SimpleMode {
    segments: Vec<Vec<Region>>,
    reference_resource: ReferenceResource,
}

impl AbstractMode for SimpleMode {}

impl SimpleMode {
    pub fn new(segments: Vec<Vec<Region>>, reference_resource: ReferenceResource) -> Self {
        let mode = Self {
            segments,
            reference_resource,
        };
        mode.print_header();
        mode
    }

    pub fn not_parallel(&self) {
        let printer = GlobalReadOnlyScope::instance().variant_printer;
        for regions in &self.segments {
            for region in regions {
                let reference = try_to_get_reference(&self.reference_resource, region);
                let initial_scope = Scope::new(
                    GlobalReadOnlyScope::instance()
                        .conf
                        .bam
                        .as_ref()
                        .expect("BAM names must be configured")
                        .get_bam1(),
                    region.clone(),
                    Arc::new(reference),
                    Arc::new(self.reference_resource.clone()),
                    0,
                    HashSet::new(),
                    printer.clone(),
                    InitialData::default(),
                );
                simple_post_process(run_pipeline(initial_scope));
            }
        }
    }

    pub fn print_header(&self) {
        let instance = GlobalReadOnlyScope::instance();
        if !instance.conf.print_header {
            return;
        }

        let mut header = tsv_join!(
            "\t",
            "Sample",
            "Gene",
            "Chr",
            "Start",
            "End",
            "Ref",
            "Alt",
            "Depth",
            "AltDepth",
            "RefFwdReads",
            "RefRevReads",
            "AltFwdReads",
            "AltRevReads",
            "Genotype",
            "AF",
            "Bias",
            "PMean",
            "PStd",
            "QMean",
            "QStd",
            "MQ",
            "Sig_Noise",
            "HiAF",
            "ExtraAF",
            "shift3",
            "MSI",
            "MSI_NT",
            "NM",
            "HiCnt",
            "HiCov",
            "5pFlankSeq",
            "3pFlankSeq",
            "Seg",
            "VarType",
            "Duprate",
            "SV_info",
        );
        if instance.conf.crispr_cutting_site != 0 {
            header = tsv_join!("\t", header, "CRISPR");
        }
        println!("{header}");
    }
}

/// Ported from: SomaticMode.java:L27-L140
#[derive(Clone, Debug)]
pub struct SomaticMode {
    segments: Vec<Vec<Region>>,
    reference_resource: ReferenceResource,
}

impl AbstractMode for SomaticMode {}

impl SomaticMode {
    pub fn new(segments: Vec<Vec<Region>>, reference_resource: ReferenceResource) -> Self {
        let mode = Self {
            segments,
            reference_resource,
        };
        mode.print_header();
        mode
    }

    pub fn not_parallel(&self) {
        let printer = GlobalReadOnlyScope::instance().variant_printer;
        let bam_names = GlobalReadOnlyScope::instance()
            .conf
            .bam
            .as_ref()
            .expect("BAM names must be configured")
            .clone();

        for regions in &self.segments {
            for region in regions {
                let splice = HashSet::new();
                let reference = try_to_get_reference(&self.reference_resource, region);
                let bam1_scope = Scope::new(
                    bam_names.get_bam1(),
                    region.clone(),
                    Arc::new(reference.clone()),
                    Arc::new(self.reference_resource.clone()),
                    0,
                    splice.clone(),
                    printer.clone(),
                    InitialData::default(),
                );
                let bam1_aligned = run_pipeline(bam1_scope);
                let bam2_scope = Scope::new(
                    bam_names.get_bam2().expect("Somatic mode requires BAM2"),
                    region.clone(),
                    Arc::new(reference),
                    Arc::new(self.reference_resource.clone()),
                    bam1_aligned.max_read_length,
                    splice,
                    printer.clone(),
                    InitialData::default(),
                );
                let bam2_aligned = run_pipeline(bam2_scope);
                somatic_post_process(bam2_aligned, bam1_aligned, &self.reference_resource);
            }
        }
    }

    pub fn print_header(&self) {
        if !GlobalReadOnlyScope::instance().conf.print_header {
            return;
        }

        let header = tsv_join!(
            "\t",
            "Sample",
            "Gene",
            "Chr",
            "Start",
            "End",
            "Ref",
            "Alt",
            "Depth",
            "AltDepth",
            "RefFwdReads",
            "RefRevReads",
            "AltFwdReads",
            "AltRevReads",
            "Genotype",
            "AF",
            "Bias",
            "PMean",
            "PStd",
            "QMean",
            "QStd",
            "MQ",
            "Sig_Noise",
            "HiAF",
            "ExtraAF",
            "NM",
            "Depth",
            "AltDepth",
            "RefFwdReads",
            "RefRevReads",
            "AltFwdReads",
            "AltRevReads",
            "Genotype",
            "AF",
            "Bias",
            "PMean",
            "PStd",
            "QMean",
            "QStd",
            "MQ",
            "Sig_Noise",
            "HiAF",
            "ExtraAF",
            "NM",
            "shift3",
            "MSI",
            "MSI_NT",
            "5pFlankSeq",
            "3pFlankSeq",
            "Seg",
            "VarLabel",
            "VarType",
            "Duprate1",
            "SV_info1",
            "Duprate2",
            "SV_info2",
        );
        println!("{header}");
    }
}

/// Ported from: AmpliconMode.java:L28-L142
#[derive(Clone, Debug)]
pub struct AmpliconMode {
    segments: Vec<Vec<Region>>,
    reference_resource: ReferenceResource,
}

impl AbstractMode for AmpliconMode {}

impl AmpliconMode {
    pub fn new(segments: Vec<Vec<Region>>, reference_resource: ReferenceResource) -> Self {
        let mode = Self {
            segments,
            reference_resource,
        };
        mode.print_header();
        mode
    }

    pub fn not_parallel(&self) {
        let printer = GlobalReadOnlyScope::instance().variant_printer;
        let bam1 = GlobalReadOnlyScope::instance()
            .conf
            .bam
            .as_ref()
            .expect("BAM names must be configured")
            .get_bam1()
            .to_string();

        for regions in &self.segments {
            let mut pos: HashMap<i32, Vec<(i32, Region)>> = HashMap::new();
            let mut current_region = regions
                .first()
                .cloned()
                .unwrap_or_else(|| Region::new("", 0, 0, ""));
            let splice = HashSet::new();
            let mut vars = Vec::new();

            for (amplicon_number, region) in regions.iter().enumerate() {
                current_region = region.clone();
                for position in region.insert_start..=region.insert_end {
                    pos.entry(position)
                        .or_default()
                        .push((i32::try_from(amplicon_number).unwrap_or(0), region.clone()));
                }
                let reference = try_to_get_reference(&self.reference_resource, region);
                let initial_scope = Scope::new(
                    bam1.clone(),
                    region.clone(),
                    Arc::new(reference),
                    Arc::new(self.reference_resource.clone()),
                    0,
                    splice.clone(),
                    printer.clone(),
                    InitialData::default(),
                );
                vars.push(run_pipeline(initial_scope).data.aligned_variants);
            }

            amplicon_post_process(&current_region, &vars, &pos, &splice, &printer);
        }
    }

    pub fn print_header(&self) {
        if !GlobalReadOnlyScope::instance().conf.print_header {
            return;
        }

        let header = tsv_join!(
            "\t",
            "Sample",
            "Gene",
            "Chr",
            "Start",
            "End",
            "Ref",
            "Alt",
            "Depth",
            "AltDepth",
            "RefFwdReads",
            "RefRevReads",
            "AltFwdReads",
            "AltRevReads",
            "Genotype",
            "AF",
            "Bias",
            "PMean",
            "PStd",
            "QMean",
            "QStd",
            "MQ",
            "Sig_Noise",
            "HiAF",
            "ExtraAF",
            "shift3",
            "MSI",
            "MSI_NT",
            "NM",
            "HiCnt",
            "HiCov",
            "5pFlankSeq",
            "3pFlankSeq",
            "Seg",
            "VarType",
            "GoodVarCount",
            "TotalVarCount",
            "Nocov",
            "Ampflag",
        );
        println!("{header}");
    }
}
