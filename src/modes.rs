use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::sync::Mutex;

use crate::data::{
    AlignedVarsData, CoverageMap, InitialData, PositionMap, RealignedVariationData, Region, Sclip,
    VariationMap,
};
use crate::mods::amplicon_post_process::amplicon_post_process;
use crate::mods::cigar_parser::CigarParser;
use crate::mods::sam_file_parser::sam_file_parser_process;
use crate::mods::simple_post_process::{simple_post_process, simple_post_process_position_lines};
use crate::mods::somatic_post_process::somatic_post_process;
use crate::mods::{structural_variants_processor, to_vars_builder, variation_realigner};
use crate::parity::snapshot::maybe_write_module_snapshot;
use crate::reference::{Reference, ReferenceResource, ReferenceSequenceMap};
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
        VariantPrinter::LineSink(sink) => {
            let rendered = std::str::from_utf8(buffer).expect("mode output must be valid UTF-8");
            for line in rendered.split_terminator('\n') {
                sink(line);
            }
        }
        VariantPrinter::OwnedLineSink(sink) => {
            let rendered = std::str::from_utf8(buffer).expect("mode output must be valid UTF-8");
            for line in rendered.split_terminator('\n') {
                sink(line.to_string());
            }
        }
    }
}

fn buffered_streaming_printer(printer: &VariantPrinter) -> Option<VariantPrinter> {
    match printer {
        VariantPrinter::Out => {
            let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
            Some(VariantPrinter::LineSink(Arc::new(move |line| {
                let mut writer = writer.lock().unwrap_or_else(|error| error.into_inner());
                writeln!(writer, "{line}").expect("failed to write mode output to stdout");
            })))
        }
        VariantPrinter::Err => {
            let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stderr())));
            Some(VariantPrinter::LineSink(Arc::new(move |line| {
                let mut writer = writer.lock().unwrap_or_else(|error| error.into_inner());
                writeln!(writer, "{line}").expect("failed to write mode output to stderr");
            })))
        }
        _ => None,
    }
}

struct SimpleLineBuffer {
    start: i32,
    end: i32,
    next_in_region_to_print: i32,
    in_region: VecDeque<Option<Vec<String>>>,
    extra: BTreeMap<i32, Vec<String>>,
}

impl SimpleLineBuffer {
    fn new(region: &Region) -> Self {
        Self {
            start: region.start,
            end: region.end,
            next_in_region_to_print: region.start,
            in_region: VecDeque::new(),
            extra: BTreeMap::new(),
        }
    }

    fn push(&mut self, position: i32, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        if position >= self.start && position <= self.end {
            let offset = usize::try_from(position - self.next_in_region_to_print)
                .expect("in-region position offset should fit usize");
            if offset >= self.in_region.len() {
                self.in_region.resize_with(offset + 1, || None);
            }
            match &mut self.in_region[offset] {
                Some(existing) => existing.extend(lines),
                slot @ None => *slot = Some(lines),
            }
        } else {
            self.extra.entry(position).or_default().extend(lines);
        }
    }

    fn flush_extra_before(&mut self, limit: i32, printer: &VariantPrinter) {
        let remaining = self.extra.split_off(&limit);
        let ready = std::mem::replace(&mut self.extra, remaining);
        for (_position, lines) in ready {
            for line in lines {
                printer.print_owned_line(line);
            }
        }
    }

    fn flush_in_region_before(&mut self, limit: i32, printer: &VariantPrinter) {
        while self.next_in_region_to_print < limit && self.next_in_region_to_print <= self.end {
            if let Some(Some(lines)) = self.in_region.pop_front() {
                for line in lines {
                    printer.print_owned_line(line);
                }
            }
            self.next_in_region_to_print += 1;
        }
    }

    fn flush_before(&mut self, limit: i32, printer: &VariantPrinter) {
        let pre_region_limit = limit.min(self.start);
        if pre_region_limit > i32::MIN {
            self.flush_extra_before(pre_region_limit, printer);
        }

        if limit > self.end {
            self.flush_in_region_before(limit, printer);
            self.flush_extra_before(limit, printer);
        } else {
            self.flush_in_region_before(limit, printer);
        }
    }

    fn print(mut self, printer: &VariantPrinter) {
        self.flush_before(i32::MAX, printer);
        for lines in self.in_region.into_iter().flatten() {
            for line in lines {
                printer.print_owned_line(line);
            }
        }
        for (_position, lines) in self.extra {
            for line in lines {
                printer.print_owned_line(line);
            }
        }
    }
}

fn run_realigned_pipeline(scope: Scope<InitialData>) -> Scope<RealignedVariationData> {
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
    drop(parser);
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
    realigned_scope
}

pub(crate) fn run_pipeline(scope: Scope<InitialData>) -> Scope<AlignedVarsData> {
    finalize_pipeline(run_realigned_pipeline(scope))
}

fn process_structural_variants(
    scope: Scope<RealignedVariationData>,
) -> Scope<RealignedVariationData> {
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

    let mut reference =
        Arc::try_unwrap(region_ref).unwrap_or_else(|region_ref| (*region_ref).clone());
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
    let mut prev_non_insertion_variants = PositionMap::<VariationMap>::default();
    let mut prev_ref_coverage = CoverageMap::default();
    let mut prev_soft_clips_3_end = HashMap::<i32, Sclip>::new();
    let mut prev_soft_clips_5_end = HashMap::<i32, Sclip>::new();
    let prev_reference_sequences = ReferenceSequenceMap::default();
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

    Scope {
        bam,
        region,
        region_ref: Arc::new(reference),
        reference_resource,
        max_read_length,
        splice,
        out,
        data,
    }
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
    } = process_structural_variants(scope);

    let aligned_data = to_vars_builder::process(
        data.max_read_length.unwrap_or(max_read_length),
        &region,
        &region_ref.reference_sequences,
        &data.ref_coverage,
        &data.insertion_variants,
        &mut data.non_insertion_variants,
        data.duprate,
    );
    maybe_write_module_snapshot("TOVARS", &region, &aligned_data);

    Scope {
        bam,
        region,
        region_ref,
        reference_resource,
        max_read_length: aligned_data.max_read_length,
        splice,
        out,
        data: aligned_data,
    }
}

fn run_simple_pipeline(scope: Scope<InitialData>) {
    let Scope {
        region,
        region_ref,
        max_read_length,
        splice,
        out,
        data:
            RealignedVariationData {
                mut non_insertion_variants,
                mut insertion_variants,
                mut ref_coverage,
                max_read_length: realigned_max_read_length,
                duprate,
                ..
            },
        ..
    } = process_structural_variants(run_realigned_pipeline(scope));

    let conf = GlobalReadOnlyScope::instance().conf.clone();
    let mut lines = SimpleLineBuffer::new(&region);
    let max_read_length = to_vars_builder::process_incremental(
        realigned_max_read_length.unwrap_or(max_read_length),
        &region,
        &region_ref.reference_sequences,
        &mut ref_coverage,
        &mut insertion_variants,
        &mut non_insertion_variants,
        duprate,
        |event| match event {
            to_vars_builder::IncrementalProcessEvent::Position { position, vars } => {
                let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    simple_post_process_position_lines(position, &vars, &region, &splice, &conf)
                }));
                if let Ok(position_lines) = rendered {
                    lines.push(position, position_lines);
                }
            }
            to_vars_builder::IncrementalProcessEvent::ReadyBefore(limit) => {
                lines.flush_before(limit, &out);
            }
        },
    );

    drop(non_insertion_variants);
    drop(insertion_variants);
    drop(ref_coverage);
    let _ = max_read_length;
    lines.print(&out);
}

/// Ported from: AbstractMode.java:L100-L105 and AbstractMode.AbstractParallelMode.
pub trait ParallelMode: AbstractMode + Sync {
    fn regions(&self) -> &[Region];

    fn process_region_to_buffer(&self, region: &Region, out: &mut Vec<u8>);

    fn process_region_to_printer(&self, region: &Region, printer: VariantPrinter) {
        let mut buffer = Vec::new();
        self.process_region_to_buffer(region, &mut buffer);
        write_mode_output(&printer, &buffer);
    }

    fn post_parallel_hook(&self) {}

    fn parallel(&self, threads: usize) {
        use crossbeam_channel::{Receiver, bounded};
        use rayon::ThreadPoolBuilder;

        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("failed to build simple-mode rayon pool");
        let (outer_tx, outer_rx) = bounded::<Receiver<Vec<u8>>>(threads.max(10));
        let printer = GlobalReadOnlyScope::instance().variant_printer.clone();
        let worker_scope = GlobalReadOnlyScope::try_thread_local_instance();
        let worker_mode = GlobalReadOnlyScope::try_thread_local_mode();

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
                let worker_scope = worker_scope.clone();
                let worker_mode = worker_mode.clone();
                scope.spawn(move |_| {
                    let _context_guard =
                        GlobalReadOnlyScope::enter_thread_local_context(worker_scope, worker_mode);
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
        let printer = GlobalReadOnlyScope::instance().variant_printer.clone();
        for region in self.regions() {
            match buffered_streaming_printer(&printer) {
                Some(streaming_printer) => {
                    self.process_region_to_printer(region, streaming_printer);
                }
                None if matches!(
                    printer,
                    VariantPrinter::LineSink(_) | VariantPrinter::OwnedLineSink(_)
                ) =>
                {
                    self.process_region_to_printer(region, printer.clone());
                }
                None => {
                    let mut buffer = Vec::new();
                    self.process_region_to_buffer(region, &mut buffer);
                    write_mode_output(&printer, &buffer);
                }
            }
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

    fn process_region_to_printer(&self, region: &Region, printer: VariantPrinter) {
        SimpleMode::process_region_to_printer(self, region, printer);
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
        self.process_region_to_printer(region, printer);

        let mut rendered = buffer.lock().unwrap_or_else(|error| error.into_inner());
        let mut bytes = std::mem::take(&mut *rendered).into_bytes();
        out.append(&mut bytes);
    }

    fn process_region_to_printer(&self, region: &Region, printer: VariantPrinter) {
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
        if std::env::var_os("VARDICT_PARITY_TOVARS").is_some() {
            simple_post_process(run_pipeline(initial_scope));
        } else {
            run_simple_pipeline(initial_scope);
        }
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
        let instance = GlobalReadOnlyScope::instance();
        let bam_names = instance
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
            Arc::new(reference),
            Arc::new(self.reference_resource.clone()),
            0,
            splice,
            printer.clone(),
            InitialData::default(),
        );
        let bam1_aligned = run_pipeline(bam1_scope);
        // Java SomaticMode.processBothBamsInPipeline passes the SAME Reference and splice
        // object to both pipelines (initialScope1 and initialScope2). The tumor pass's
        // SV-breakpoint reference extensions and splice sites are therefore visible to the
        // normal pass, which lets it skip the partial-pipeline re-parse at already-loaded
        // breakpoints. Share the tumor-extended reference + splice with the normal pipeline.
        let bam2_scope = Scope::new(
            bam_names.get_bam2().expect("Somatic mode requires BAM2"),
            region.clone(),
            bam1_aligned.region_ref.clone(),
            Arc::new(self.reference_resource.clone()),
            bam1_aligned.max_read_length,
            (*bam1_aligned.splice).clone(),
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
        let instance = GlobalReadOnlyScope::instance();
        let printer = instance.variant_printer.clone();
        let bam1 = instance
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
