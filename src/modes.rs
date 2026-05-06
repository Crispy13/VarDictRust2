use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;

use crate::data::{
    AlignedVarsData, InitialData, RealignedVariationData, Region, Sclip, VariationMap,
};
use crate::mods::amplicon_post_process::amplicon_post_process;
use crate::mods::cigar_parser::CigarParser;
use crate::mods::sam_file_parser::sam_file_parser_process;
use crate::mods::simple_post_process::simple_post_process;
use crate::mods::somatic_post_process::somatic_post_process;
use crate::mods::{structural_variants_processor, to_vars_builder, variation_realigner};
use crate::parity::snapshot::maybe_write_module_snapshot;
use crate::reference::{Reference, ReferenceResource};
use crate::scope::{AbstractMode, GlobalReadOnlyScope, Scope, VariantPrinter};
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

fn write_mode_output(printer: &VariantPrinter, buffer: &[u8]) {
    match printer {
        VariantPrinter::Out => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle
                .write_all(buffer)
                .expect("failed to write mode output to stdout");
        }
        VariantPrinter::Err => {
            let stderr = std::io::stderr();
            let mut handle = stderr.lock();
            handle
                .write_all(buffer)
                .expect("failed to write mode output to stderr");
        }
        VariantPrinter::Buffer(captured) => {
            let rendered = std::str::from_utf8(buffer).expect("mode output must be valid UTF-8");
            let mut output = captured.lock().unwrap_or_else(|error| error.into_inner());
            output.push_str(rendered);
        }
    }
}

fn run_pipeline(scope: Scope<InitialData>) -> Scope<AlignedVarsData> {
    let parsed_scope = sam_file_parser_process(scope);
    maybe_write_module_snapshot(
        "SAM_FILE_PARSER",
        &parsed_scope.region,
        &parsed_scope.data.initial_data,
    );
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
    let variation_data = parser.process_preprocessor(&mut data, &header, &chr_name);
    maybe_write_module_snapshot("CIGAR_PARSER", &region, &variation_data);
    maybe_write_module_snapshot("CIGAR_MODIFIER", &region, &parser.cigar_modifier_snapshots);
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
    maybe_write_module_snapshot("REALIGNER", &realigned_scope.region, &realigned_scope.data);
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
    let mut prev_non_insertion_variants = HashMap::<i32, VariationMap>::new();
    let mut prev_ref_coverage = HashMap::<i32, i32>::new();
    let mut prev_soft_clips_3_end = HashMap::<i32, Sclip>::new();
    let mut prev_soft_clips_5_end = HashMap::<i32, Sclip>::new();
    let prev_reference_sequences = HashMap::<i32, u8>::new();
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
    maybe_write_module_snapshot("SV_PROCESSOR", &region, &data);

    let aligned_data = to_vars_builder::process(
        data.max_read_length.unwrap_or(max_read_length),
        &region,
        &reference.reference_sequences,
        &data.ref_coverage,
        &data.insertion_variants,
        &mut data.non_insertion_variants,
        data.duprate,
    );
    maybe_write_module_snapshot("TOVARS", &region, &aligned_data);

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

/// Ported from: AbstractMode.java:L100-L105 and AbstractMode.AbstractParallelMode.
pub trait ParallelMode: AbstractMode + Sync {
    fn regions(&self) -> &[Region];

    fn process_region_to_buffer(&self, region: &Region, out: &mut Vec<u8>);

    fn post_parallel_hook(&self) {}

    fn parallel(&self, threads: usize) {
        use crossbeam_channel::{Receiver, bounded};
        use rayon::ThreadPoolBuilder;

        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("failed to build simple-mode rayon pool");
        let (outer_tx, outer_rx) = bounded::<Receiver<Vec<u8>>>(threads.max(10));
        let printer = GlobalReadOnlyScope::instance().variant_printer;

        let consumer = std::thread::spawn(move || {
            while let Ok(inner_rx) = outer_rx.recv() {
                let buffer = inner_rx.recv().expect("worker dropped buffered output");
                write_mode_output(&printer, &buffer);
            }
        });

        pool.scope(|scope| {
            for region in self.regions() {
                let (inner_tx, inner_rx) = bounded::<Vec<u8>>(1);
                outer_tx
                    .send(inner_rx)
                    .expect("consumer dropped outer simple-mode queue");
                let self_ref = self;
                scope.spawn(move |_| {
                    let mut buffer = Vec::new();
                    self_ref.process_region_to_buffer(region, &mut buffer);
                    inner_tx
                        .send(buffer)
                        .expect("consumer dropped inner simple-mode queue");
                });
            }
        });

        drop(outer_tx);
        consumer.join().expect("parallel consumer thread panicked");
        self.post_parallel_hook();
    }

    fn not_parallel(&self) {
        let printer = GlobalReadOnlyScope::instance().variant_printer;
        for region in self.regions() {
            let mut buffer = Vec::new();
            self.process_region_to_buffer(region, &mut buffer);
            write_mode_output(&printer, &buffer);
        }
        self.post_parallel_hook();
    }
}

/// Ported from: SimpleMode.java:L25-L118
#[derive(Clone, Debug)]
pub struct SimpleMode {
    regions: Vec<Region>,
    reference_resource: ReferenceResource,
}

impl AbstractMode for SimpleMode {}

impl ParallelMode for SimpleMode {
    fn regions(&self) -> &[Region] {
        &self.regions
    }

    fn process_region_to_buffer(&self, region: &Region, out: &mut Vec<u8>) {
        SimpleMode::process_region_to_buffer(self, region, out);
    }
}

impl SimpleMode {
    pub fn new(segments: Vec<Vec<Region>>, reference_resource: ReferenceResource) -> Self {
        let mode = Self {
            regions: segments.into_iter().flatten().collect(),
            reference_resource,
        };
        mode.print_header();
        mode
    }

    pub fn not_parallel(&self) {
        <Self as ParallelMode>::not_parallel(self);
    }

    /// Ported from: SimpleMode.processBamInPipeline()
    /// Java source: SimpleMode.java:L90-L104
    pub fn process_region_to_buffer(&self, region: &Region, out: &mut Vec<u8>) {
        let buffer = Arc::new(Mutex::new(String::new()));
        let printer = VariantPrinter::Buffer(buffer.clone());
        let reference = try_to_get_reference(&self.reference_resource, region);
        let bam1 = GlobalReadOnlyScope::with_instance(|scope| {
            scope
                .conf
                .bam
                .as_ref()
                .expect("BAM names must be configured")
                .get_bam1()
                .to_string()
        });
        let initial_scope = Scope::new(
            bam1,
            region.clone(),
            Arc::new(reference),
            Arc::new(self.reference_resource.clone()),
            0,
            HashSet::new(),
            printer,
            InitialData::default(),
        );
        simple_post_process(run_pipeline(initial_scope));

        let mut rendered = buffer.lock().unwrap_or_else(|error| error.into_inner());
        let mut bytes = std::mem::take(&mut *rendered).into_bytes();
        out.append(&mut bytes);
    }

    pub fn parallel(&self, threads: usize) {
        <Self as ParallelMode>::parallel(self, threads);
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
    regions: Vec<Region>,
    reference_resource: ReferenceResource,
}

impl AbstractMode for SomaticMode {}

impl ParallelMode for SomaticMode {
    fn regions(&self) -> &[Region] {
        &self.regions
    }

    fn process_region_to_buffer(&self, region: &Region, out: &mut Vec<u8>) {
        SomaticMode::process_region_to_buffer(self, region, out);
    }
}

impl SomaticMode {
    pub fn new(segments: Vec<Vec<Region>>, reference_resource: ReferenceResource) -> Self {
        let mode = Self {
            regions: segments.into_iter().flatten().collect(),
            reference_resource,
        };
        mode.print_header();
        mode
    }

    pub fn not_parallel(&self) {
        <Self as ParallelMode>::not_parallel(self);
    }

    pub fn parallel(&self, threads: usize) {
        <Self as ParallelMode>::parallel(self, threads);
    }

    /// Ported from: SomaticMode.processBothBamsInPipeline()
    /// Java source: SomaticMode.java:L97-L130
    pub fn process_region_to_buffer(&self, region: &Region, out: &mut Vec<u8>) {
        let buffer = Arc::new(Mutex::new(String::new()));
        let printer = VariantPrinter::Buffer(buffer.clone());
        let bam_names = GlobalReadOnlyScope::instance()
            .conf
            .bam
            .as_ref()
            .expect("BAM names must be configured")
            .clone();
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
            printer,
            InitialData::default(),
        );
        let bam2_aligned = run_pipeline(bam2_scope);
        somatic_post_process(bam2_aligned, bam1_aligned, &self.reference_resource);

        let mut rendered = buffer.lock().unwrap_or_else(|error| error.into_inner());
        let mut bytes = std::mem::take(&mut *rendered).into_bytes();
        out.append(&mut bytes);
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

impl ParallelMode for AmpliconMode {
    fn regions(&self) -> &[Region] {
        &[]
    }

    fn process_region_to_buffer(&self, _region: &Region, _out: &mut Vec<u8>) {
        panic!("not yet implemented");
    }

    fn not_parallel(&self) {
        AmpliconMode::not_parallel(self);
    }
}

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
