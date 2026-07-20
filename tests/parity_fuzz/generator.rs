//! proptest strategies for the germline differential-parity fuzzer.
//!
//! ALL randomness that affects the synthesized genome flows through proptest
//! `Strategy` values here so that a failing case can be shrunk. Nothing in this
//! module (or downstream in `synth`/`oracle`) touches the `rand` crate.

use proptest::prelude::*;

/// Read length used for every synthesized read: the number of ref-matched
/// bases spanned by a plain `<READ_LEN>M` read, and the `a+b` matched span
/// either side of an indel. Matches the ~60bp scale used by the manual spike.
pub const READ_LEN: u32 = 60;

/// Upper bound on both deletion length and insertion length (bp). Small
/// enough to stay well inside the read-window margins below.
const MAX_INDEL_LEN: u32 = 6;

/// Minimum spacing (bp) enforced between consecutive loci so their read
/// windows never overlap (reads only ever span roughly
/// `pos - READ_LEN/2 .. pos + READ_LEN/2`, plus up to `MAX_INDEL_LEN` extra
/// ref bases consumed by a deletion).
const MIN_LOCUS_SPACING: u32 = 200;
const MAX_LOCUS_SPACING: u32 = 500;

/// Deterministic non-homopolymer 4-cycle: A,C,G,T,A,C,G,T,... Every base differs
/// from its predecessor by construction, so the whole contig is generated from
/// this pure function alone, with no randomness involved.
const BASES: [u8; 4] = [b'A', b'C', b'G', b'T'];

fn base_at_cycle(zero_based_index: usize) -> u8 {
    BASES[zero_based_index % BASES.len()]
}

fn base_cycle_index(base: u8) -> usize {
    BASES
        .iter()
        .position(|&b| b == base)
        .expect("base_at_cycle only ever produces bases from BASES")
}

/// The variant carried by a locus's alt reads.
#[derive(Debug, Clone)]
pub enum VariantKind {
    Snv { ref_base: u8, alt_base: u8 },
    /// Deletion of `len` ref bases starting at the locus's 1-based `pos`.
    Del { len: u32 },
    /// Insertion of `bases` immediately after the locus's 1-based `pos`.
    Ins { bases: Vec<u8> },
    /// Multi-nucleotide variant: `alt_offsets.len()` adjacent base
    /// substitutions starting at the locus's 1-based `pos`. Each
    /// `alt_offsets[j]` is the cycle offset (1..=3) applied to the ref base
    /// at `pos + j` to pick a distinct alt base, same trick as `Snv`.
    Mnv { alt_offsets: Vec<u32> },
}

/// A clip is a READ-LEVEL modifier orthogonal to `VariantKind`: it transforms
/// an already-built aligned read's leading (5') or trailing (3') `M` op into
/// a soft- or hard-clip, proven byte-identical across both tools by two
/// manual spikes (clip x SNV via `gen_clip_spike.py`, clip x deletion).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// Soft clip (`S`): bases stay present in SEQ but are unaligned.
    Soft,
    /// Hard clip (`H`): bases are absent from SEQ entirely.
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipSide {
    FivePrime,
    ThreePrime,
}

/// A clip to apply to a subset of one locus's reads. `len` (1..=15) is the
/// number of bases clipped; the read's leading/trailing `M` span is always
/// comfortably larger (~30bp), so the clip fits without exhausting it.
#[derive(Debug, Clone, Copy)]
pub struct ClipSpec {
    pub kind: ClipKind,
    pub side: ClipSide,
    pub len: u32,
}

/// SAM FLAG bits both VarDictJava and vardict_rs must skip a read for
/// entirely: duplicate (0x400), secondary (0x100), supplementary (0x800).
/// Proven byte-identical by a manual spike (`gen_flags_spike.py`): a clean
/// SNV locus plus 10 duplicate + 10 secondary + 10 supplementary alt reads
/// yields `totcov=60, varcov=40` in both tools -- the 30 flagged reads
/// dropped identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterFlag {
    Duplicate,
    Secondary,
    Supplementary,
}

impl FilterFlag {
    /// The SAM FLAG bit this variant OR's into a read's base strand flag.
    fn bit(self) -> u16 {
        match self {
            FilterFlag::Duplicate => 0x400,
            FilterFlag::Secondary => 0x100,
            FilterFlag::Supplementary => 0x800,
        }
    }
}

/// Straddle sets: values BELOW common thresholds plus one no-op high value, so
/// a quality-noise read may or may not be filtered by a given preset.
const STRADDLE_MAPQS: &[u8] = &[0, 20, 25, 29, 60]; // vs -Q / -O floors
const STRADDLE_BASE_QUALS: &[u8] = &[10, 14, 15, 40]; // vs -q floor (Phred)

/// One extra ALT-carrying read whose MAPQ and/or base quality may fall below a
/// preset's filter floor. Appended on top of the clean `depth` reads (like
/// `filtered_reads`), so the variant always still calls at the default preset.
#[derive(Debug, Clone, Copy)]
pub struct QualNoiseRead {
    pub mapq: u8,
    pub base_qual: u8,
}

fn qual_noise_read_strategy() -> impl Strategy<Value = QualNoiseRead> {
    (
        proptest::sample::select(STRADDLE_MAPQS.to_vec()),
        proptest::sample::select(STRADDLE_BASE_QUALS.to_vec()),
    )
        .prop_map(|(mapq, base_qual)| QualNoiseRead { mapq, base_qual })
}

/// 0..=6 quality-noise reads, biased 3:1 toward none (same weighting as
/// `optional_clip_strategy` / `filtered_reads_strategy`) so most loci look like
/// the uniform-quality baseline and quality-straddle loci stay a stressing minority.
fn quality_noise_strategy() -> impl Strategy<Value = Vec<QualNoiseRead>> {
    prop_oneof![
        3 => Just(Vec::new()),
        1 => prop::collection::vec(qual_noise_read_strategy(), 1..=6),
    ]
}

/// One locus: a reference position covered by `depth` reads, `alt_count` of
/// which carry `kind`'s variant instead of the plain reference.
#[derive(Debug, Clone)]
pub struct Locus {
    /// 1-based position of the variant within the contig.
    pub pos: u32,
    pub kind: VariantKind,
    pub depth: u32,
    pub alt_count: u32,
    /// If set, applied to a subset (every other read) of this locus's reads.
    pub clip: Option<ClipSpec>,
    /// Extra ALT-carrying reads, each flagged duplicate/secondary/
    /// supplementary, appended on top of `depth`. Both tools must skip these
    /// entirely, so they must not change any output column -- the clean
    /// `depth` reads alone guarantee the call (non-vacuity is preserved).
    pub filtered_reads: Vec<FilterFlag>,
    /// Extra ALT-carrying reads whose MAPQ / base quality straddle -Q/-q
    /// filter floors, appended on top of `depth`. See `QualNoiseRead`.
    pub quality_noise: Vec<QualNoiseRead>,
}

/// One synthesized read record, materialized from a `Locus`.
#[derive(Debug, Clone)]
pub struct ReadRecord {
    pub qname: String,
    /// SAM FLAG: 0 (forward) or 16 (reverse) strand, optionally OR'd with a
    /// `FilterFlag` bit for noise reads that both tools must skip.
    pub flag: u16,
    /// 1-based leftmost mapping position.
    pub pos: u32,
    pub cigar: String,
    pub seq: Vec<u8>,
    /// SAM MAPQ column. Normal reads keep the default 60.
    pub mapq: u8,
    /// Uniform Phred base quality for EVERY base of this read (SAM QUAL column
    /// char = b'!' + base_qual). Normal reads use 40 ('I'); quality-noise reads
    /// use a straddling value so -q filters engage.
    pub base_qual: u8,
}

/// A synthetic single-contig genome: reference sequence plus the reads covering it.
#[derive(Debug, Clone)]
pub struct Genome {
    pub contig: String,
    pub sequence: Vec<u8>,
    #[allow(dead_code)] // kept for reproducer context / future non-fixed READ_LEN
    pub read_len: u32,
    pub loci: Vec<Locus>,
    pub reads: Vec<ReadRecord>,
    /// 1-based inclusive scan region passed as `-R contig:scan_start-scan_end`.
    /// Usually the whole contig, but sometimes cropped to sit at/near loci so
    /// region-boundary inclusion is exercised (a variant exactly at the region
    /// edge must be included/excluded identically by both tools).
    pub scan_start: u32,
    pub scan_end: u32,
}

/// How the scan region relates to the loci.
#[derive(Debug, Clone)]
enum RegionCrop {
    /// Whole contig (`1..=contig_len`).
    Full,
    /// Region edges placed at `first_locus.pos + start_delta` and
    /// `last_locus.pos + end_delta` (deltas in -1..=1), so a boundary locus
    /// lands just inside / on / just outside the region. Only applied when
    /// there are >=3 loci, so >=1 middle locus is always strictly inside and
    /// the case stays non-vacuous.
    Crop { start_delta: i32, end_delta: i32 },
}

fn region_crop_strategy() -> impl Strategy<Value = RegionCrop> {
    prop_oneof![
        2 => Just(RegionCrop::Full),
        1 => (-1i32..=1, -1i32..=1)
            .prop_map(|(start_delta, end_delta)| RegionCrop::Crop { start_delta, end_delta }),
    ]
}

/// Raw proptest-generated parameters for one locus's variant, before layout
/// (position, concrete ref/alt bases) is resolved against the contig.
#[derive(Debug, Clone)]
enum VariantKindSpec {
    /// Offset (1..=3) applied to the ref base's cycle index (mod 4) to pick a
    /// distinct alt base.
    Snv { alt_offset: u32 },
    /// Number of ref bases to delete.
    Del { len: u32 },
    /// Bases to insert.
    Ins { bases: Vec<u8> },
    /// Per-position cycle offsets (1..=3), one per adjacent substituted base;
    /// `alt_offsets.len()` is the MNV length (2..=4).
    Mnv { alt_offsets: Vec<u32> },
}

fn acgt_byte_strategy() -> impl Strategy<Value = u8> {
    (0usize..BASES.len()).prop_map(|index| BASES[index])
}

fn variant_kind_spec_strategy() -> impl Strategy<Value = VariantKindSpec> {
    prop_oneof![
        (1u32..=3).prop_map(|alt_offset| VariantKindSpec::Snv { alt_offset }),
        (1u32..=MAX_INDEL_LEN).prop_map(|len| VariantKindSpec::Del { len }),
        prop::collection::vec(acgt_byte_strategy(), 1..=MAX_INDEL_LEN as usize)
            .prop_map(|bases| VariantKindSpec::Ins { bases }),
        prop::collection::vec(1u32..=3, 2..=4)
            .prop_map(|alt_offsets| VariantKindSpec::Mnv { alt_offsets }),
    ]
}

/// Upper bound on clip length (bp). Small relative to the ~30bp leading/
/// trailing `M` span every read carries, so the clipped `M` op never
/// exhausts (guarded anyway, see `apply_clip`).
const MAX_CLIP_LEN: u32 = 15;

fn clip_kind_strategy() -> impl Strategy<Value = ClipKind> {
    prop_oneof![Just(ClipKind::Soft), Just(ClipKind::Hard)]
}

fn clip_side_strategy() -> impl Strategy<Value = ClipSide> {
    prop_oneof![Just(ClipSide::FivePrime), Just(ClipSide::ThreePrime)]
}

fn clip_spec_strategy() -> impl Strategy<Value = ClipSpec> {
    (clip_kind_strategy(), clip_side_strategy(), 1u32..=MAX_CLIP_LEN)
        .prop_map(|(kind, side, len)| ClipSpec { kind, side, len })
}

/// Bias toward `None` (3:1) so unclipped loci still dominate the corpus;
/// clipped loci are the minority stressing the read-level transform.
fn optional_clip_strategy() -> impl Strategy<Value = Option<ClipSpec>> {
    prop_oneof![
        3 => Just(None),
        1 => clip_spec_strategy().prop_map(Some),
    ]
}

/// Raw proptest-generated parameters for one locus, before layout (position,
/// concrete ref/alt bases) is resolved against its predecessor and the contig.
#[derive(Debug, Clone)]
struct LocusSpec {
    /// Distance (bp) from the previous locus. Ignored for the first locus, which
    /// instead uses a fixed margin sized to fit a full read upstream.
    spacing_from_previous: u32,
    /// Total read depth covering the locus. 30..=60 is safe VarDict-calling range.
    depth: u32,
    /// Percent of reads carrying the alt allele. 30..=70 is a safe calling range.
    alt_pct: u32,
    kind_spec: VariantKindSpec,
    clip: Option<ClipSpec>,
    filtered_reads: Vec<FilterFlag>,
    quality_noise: Vec<QualNoiseRead>,
}

fn filter_flag_strategy() -> impl Strategy<Value = FilterFlag> {
    prop_oneof![
        Just(FilterFlag::Duplicate),
        Just(FilterFlag::Secondary),
        Just(FilterFlag::Supplementary),
    ]
}

/// 0..=6 extra ALT-carrying reads that both tools must skip entirely
/// (duplicate/secondary/supplementary). Biased toward none (3:1, same weight
/// as `optional_clip_strategy`) so most loci look like the unfiltered
/// baseline, keeping filtered-noise loci a stressing minority.
fn filtered_reads_strategy() -> impl Strategy<Value = Vec<FilterFlag>> {
    prop_oneof![
        3 => Just(Vec::new()),
        1 => prop::collection::vec(filter_flag_strategy(), 1..=6),
    ]
}

fn locus_spec_strategy() -> impl Strategy<Value = LocusSpec> {
    (
        MIN_LOCUS_SPACING..=MAX_LOCUS_SPACING,
        30u32..=60,
        30u32..=70,
        variant_kind_spec_strategy(),
        optional_clip_strategy(),
        filtered_reads_strategy(),
        quality_noise_strategy(),
    )
        .prop_map(
            |(spacing_from_previous, depth, alt_pct, kind_spec, clip, filtered_reads, quality_noise)| LocusSpec {
                spacing_from_previous,
                depth,
                alt_pct,
                kind_spec,
                clip,
                filtered_reads,
                quality_noise,
            },
        )
}

/// A curated germline config preset: its name plus the CLI flag tokens it adds
/// to BOTH tools. Same tokens the e2e sweep feeds via scripts/config_presets.tsv
/// (resolved through `crate::common::config_preset_java_flags`).
#[derive(Debug, Clone)]
pub struct Preset {
    pub name: String,
    pub flags: Vec<String>,
}

/// Curated germline preset names for the preset-parity pass. Deliberately
/// EXCLUDES: CM-UNIQUN (--UN makes VarDictJava exit 1 on unpaired reads),
/// CM-TH4 (thread count only, redundant with pinned -th 1), CM-EXTEND
/// (deferred negative-coord region gap).
const CURATED_GERMLINE_PRESETS: &[&str] = &[
    "T1-02", "T1-03", "T1-06", "T1-08", "T1-09", "T1-10",
    "CM-MAPQ30", "CM-MEANMAPQ", "CM-QRATIO", "CM-TRIM", "CM-MINMATCH",
    "CM-SAMFILT", "CM-FISHER", "CM-PILEUP", "CM-NOSV", "CM-NOREAL",
    "CM-3PRIME", "CM-CHIMERIC", "CM-DEBUG",
];

/// Strategy selecting one curated preset (shrinks toward the first entry).
pub fn arb_germline_preset() -> impl Strategy<Value = Preset> {
    let presets: Vec<Preset> = CURATED_GERMLINE_PRESETS
        .iter()
        .map(|name| Preset {
            name: (*name).to_string(),
            flags: crate::common::config_preset_java_flags(name),
        })
        .collect();
    proptest::sample::select(presets)
}

/// Strategy producing a `Genome` with 1..=8 loci (each independently SNV,
/// deletion, or insertion) spaced >=200bp apart on one synthetic contig.
pub fn arb_genome() -> impl Strategy<Value = Genome> {
    (
        prop::collection::vec(locus_spec_strategy(), 1..=8),
        region_crop_strategy(),
    )
        .prop_map(|(specs, crop)| build_genome(specs, crop))
}

fn build_genome(specs: Vec<LocusSpec>, crop: RegionCrop) -> Genome {
    let read_len = READ_LEN;
    // Enough room upstream of the first locus for a full read to fit.
    let leading_margin = read_len + 50;

    let mut loci = Vec::with_capacity(specs.len());
    let mut pos: u32 = leading_margin;
    for (index, spec) in specs.iter().enumerate() {
        if index > 0 {
            pos += spec.spacing_from_previous;
        }

        let kind = match &spec.kind_spec {
            VariantKindSpec::Snv { alt_offset } => {
                let ref_base = base_at_cycle(pos as usize - 1);
                let ref_index = base_cycle_index(ref_base);
                let alt_base = BASES[(ref_index + *alt_offset as usize) % BASES.len()];
                VariantKind::Snv { ref_base, alt_base }
            }
            VariantKindSpec::Del { len } => VariantKind::Del { len: *len },
            VariantKindSpec::Ins { bases } => VariantKind::Ins {
                bases: bases.clone(),
            },
            VariantKindSpec::Mnv { alt_offsets } => VariantKind::Mnv {
                alt_offsets: alt_offsets.clone(),
            },
        };

        let alt_count =
            ((spec.depth * spec.alt_pct) / 100).clamp(1, spec.depth.saturating_sub(1).max(1));

        loci.push(Locus {
            pos,
            kind,
            depth: spec.depth,
            alt_count,
            clip: spec.clip,
            filtered_reads: spec.filtered_reads.clone(),
            quality_noise: spec.quality_noise.clone(),
        });
    }

    // Trailing margin mirrors the leading one so the last locus's reads fit
    // too, plus `MAX_INDEL_LEN` slack: a deletion's alt read consumes ref
    // bases up to `pos - 1 + len + b` (see `synthesize_reads`), which reaches
    // `MAX_INDEL_LEN` bases further than a plain `<READ_LEN>M` read would.
    let contig_len =
        loci.last().map_or(leading_margin, |l| l.pos) + read_len + 50 + MAX_INDEL_LEN;
    let sequence: Vec<u8> = (0..contig_len as usize).map(base_at_cycle).collect();

    let reads = synthesize_reads(&loci, &sequence, read_len, contig_len);

    // Scan region: whole contig, unless cropped to the loci edges. Cropping is
    // only honored with >=3 loci so >=1 middle locus is always strictly inside
    // (keeps the case non-vacuous); otherwise fall back to the full contig.
    let (scan_start, scan_end) = match crop {
        RegionCrop::Crop { start_delta, end_delta } if loci.len() >= 3 => {
            let first = loci.first().map_or(1, |l| l.pos) as i32;
            let last = loci.last().map_or(contig_len, |l| l.pos) as i32;
            let start = (first + start_delta).clamp(1, contig_len as i32) as u32;
            let end = (last + end_delta).clamp(start as i32, contig_len as i32) as u32;
            (start, end)
        }
        _ => (1, contig_len),
    };

    Genome {
        contig: "chrS".to_string(),
        sequence,
        read_len,
        loci,
        reads,
        scan_start,
        scan_end,
    }
}

/// A synthetic paired tumor/normal genome for the somatic lane. Tumor and
/// normal share one reference `sequence`; they differ only in per-locus
/// alt-read counts. For S0 every locus is StrongSomatic: the variant is
/// present in the tumor read set and absent from the normal read set.
#[derive(Debug, Clone)]
pub struct SomaticGenome {
    pub contig: String,
    pub sequence: Vec<u8>,
    pub loci: Vec<Locus>, // tumor loci (carry the variant); kept for reproducer context
    pub tumor_reads: Vec<ReadRecord>,
    pub normal_reads: Vec<ReadRecord>,
    pub scan_start: u32,
    pub scan_end: u32,
}

/// Raw params for one StrongSomatic SNV locus.
#[derive(Debug, Clone)]
struct SomaticLocusSpec {
    /// Distance (bp) from the previous locus; reuses the germline spacing
    /// range so read windows never overlap.
    spacing_from_previous: u32,
    /// Total read depth covering the locus in both tumor and normal. 30..=60
    /// is safe VarDict-calling range.
    depth: u32,
    /// Percent of TUMOR reads carrying the alt allele. 30..=70 is a safe
    /// calling range. The normal sample carries none (StrongSomatic).
    tumor_alt_pct: u32,
    /// Offset (1..=3) applied to the ref base's cycle index (mod 4) to pick a
    /// distinct alt base, same trick as the germline SNV.
    snv_alt_offset: u32,
}

fn somatic_locus_spec_strategy() -> impl Strategy<Value = SomaticLocusSpec> {
    (
        MIN_LOCUS_SPACING..=MAX_LOCUS_SPACING,
        30u32..=60,
        30u32..=70,
        1u32..=3,
    )
        .prop_map(
            |(spacing_from_previous, depth, tumor_alt_pct, snv_alt_offset)| SomaticLocusSpec {
                spacing_from_previous,
                depth,
                tumor_alt_pct,
                snv_alt_offset,
            },
        )
}

/// Strategy producing a `SomaticGenome` with 1..=6 StrongSomatic SNV loci
/// spaced >=200bp apart on one synthetic contig.
pub fn arb_somatic_genome() -> impl Strategy<Value = SomaticGenome> {
    prop::collection::vec(somatic_locus_spec_strategy(), 1..=6).prop_map(build_somatic_genome)
}

fn build_somatic_genome(specs: Vec<SomaticLocusSpec>) -> SomaticGenome {
    let read_len = READ_LEN;
    // Enough room upstream of the first locus for a full read to fit.
    let leading_margin = read_len + 50;

    let mut tumor_loci = Vec::with_capacity(specs.len());
    let mut normal_loci = Vec::with_capacity(specs.len());
    let mut pos: u32 = leading_margin;
    for (index, spec) in specs.iter().enumerate() {
        if index > 0 {
            pos += spec.spacing_from_previous;
        }

        let ref_base = base_at_cycle(pos as usize - 1);
        let ref_index = base_cycle_index(ref_base);
        let alt_base = BASES[(ref_index + spec.snv_alt_offset as usize) % BASES.len()];
        let kind = VariantKind::Snv { ref_base, alt_base };

        let tumor_alt_count = ((spec.depth * spec.tumor_alt_pct) / 100)
            .clamp(1, spec.depth.saturating_sub(1).max(1));

        tumor_loci.push(Locus {
            pos,
            kind: kind.clone(),
            depth: spec.depth,
            alt_count: tumor_alt_count,
            clip: None,
            filtered_reads: vec![],
            quality_noise: vec![],
        });
        normal_loci.push(Locus {
            pos,
            kind,
            depth: spec.depth,
            alt_count: 0,
            clip: None,
            filtered_reads: vec![],
            quality_noise: vec![],
        });
    }

    // Trailing margin mirrors the germline `build_genome` layout.
    let contig_len = tumor_loci.last().map_or(leading_margin, |l| l.pos) + read_len + 50 + MAX_INDEL_LEN;
    let sequence: Vec<u8> = (0..contig_len as usize).map(base_at_cycle).collect();

    let tumor_reads = synthesize_reads(&tumor_loci, &sequence, read_len, contig_len);
    let normal_reads = synthesize_reads(&normal_loci, &sequence, read_len, contig_len);

    SomaticGenome {
        contig: "chrS".to_string(),
        sequence,
        loci: tumor_loci,
        tumor_reads,
        normal_reads,
        scan_start: 1,
        scan_end: contig_len,
    }
}

/// Build the CIGAR + SEQ for one read at a locus. `start` is the read's
/// 1-based leftmost mapping position; `is_alt` selects whether the read
/// carries the locus's variant or plain reference bases.
///
/// Encoding proven by the manual indel spike (see task brief /
/// `gen_indel_spike.py`): for an indel, `a` is the matched span before the
/// event and `b` the matched span after, with `a + b == READ_LEN`.
fn build_read(kind: &VariantKind, sequence: &[u8], pos: u32, start: u32, is_alt: bool) -> (String, Vec<u8>) {
    let read_len = READ_LEN;
    let ref_window = |from_1based: u32, len: usize| -> &[u8] {
        let start_index = from_1based as usize - 1;
        &sequence[start_index..start_index + len]
    };

    match kind {
        VariantKind::Snv { ref_base, alt_base } => {
            let offset = (pos - start) as usize;
            let mut seq = ref_window(start, read_len as usize).to_vec();
            seq[offset] = if is_alt { *alt_base } else { *ref_base };
            (format!("{read_len}M"), seq)
        }
        VariantKind::Del { len } => {
            if !is_alt {
                return (
                    format!("{read_len}M"),
                    ref_window(start, read_len as usize).to_vec(),
                );
            }
            let len = *len;
            let a = (pos - start) as usize;
            let b = read_len as usize - a;
            let mut seq = ref_window(start, a).to_vec();
            seq.extend_from_slice(ref_window(pos + len, b));
            (format!("{a}M{len}D{b}M"), seq)
        }
        VariantKind::Ins { bases } => {
            if !is_alt {
                return (
                    format!("{read_len}M"),
                    ref_window(start, read_len as usize).to_vec(),
                );
            }
            let a = (pos - start + 1) as usize;
            let b = read_len as usize - a;
            let m = bases.len();
            let mut seq = ref_window(start, a).to_vec();
            seq.extend_from_slice(bases);
            seq.extend_from_slice(ref_window(pos + 1, b));
            (format!("{a}M{m}I{b}M"), seq)
        }
        VariantKind::Mnv { alt_offsets } => {
            let mut seq = ref_window(start, read_len as usize).to_vec();
            if is_alt {
                for (j, alt_offset) in alt_offsets.iter().enumerate() {
                    let offset = (pos - start) as usize + j;
                    let ref_base_j = seq[offset];
                    let ref_index = base_cycle_index(ref_base_j);
                    seq[offset] = BASES[(ref_index + *alt_offset as usize) % BASES.len()];
                }
            }
            (format!("{read_len}M"), seq)
        }
    }
}

/// Parse a CIGAR string into `(length, op)` tokens, e.g. `"30M6D30M"` ->
/// `[(30, 'M'), (6, 'D'), (30, 'M')]`. Every CIGAR produced by `build_read`
/// starts and ends with an `M` op (plain `<n>M`, or `<a>M<len>D<b>M` /
/// `<a>M<len>I<b>M`), which `apply_clip` relies on.
fn parse_cigar(cigar: &str) -> Vec<(u32, char)> {
    let mut tokens = Vec::new();
    let mut number = String::new();
    for ch in cigar.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
        } else {
            let len: u32 = number
                .parse()
                .unwrap_or_else(|error| panic!("invalid CIGAR op length in `{cigar}`: {error}"));
            tokens.push((len, ch));
            number.clear();
        }
    }
    assert!(
        number.is_empty(),
        "CIGAR `{cigar}` ends with a dangling length (missing op letter)"
    );
    tokens
}

fn format_cigar(tokens: &[(u32, char)]) -> String {
    tokens
        .iter()
        .map(|(len, op)| format!("{len}{op}"))
        .collect()
}

/// Apply `clip` to one already-built aligned read `(cigar, seq, pos)`,
/// editing the leading (5') or trailing (3') `M` op per the encoding proven
/// by the manual clip spike (`gen_clip_spike.py`):
///
/// - 5' Soft:  POS += c; CIGAR = `{c}S{m-c}M` + rest; SEQ unchanged.
/// - 5' Hard:  POS += c; CIGAR = `{c}H{m-c}M` + rest; SEQ = seq[c..].
/// - 3' Soft:  trailing `{m}M` -> `{m-c}M{c}S`; POS unchanged; SEQ unchanged.
/// - 3' Hard:  trailing `{m}M` -> `{m-c}M{c}H`; SEQ = seq[..len-c].
///
/// Guard: if the target `M` op is too small to clip (`m <= 1`), the read is
/// returned unmodified rather than emitting an invalid CIGAR or a 0-length
/// `M` op. Otherwise `c` is clamped to `min(clip.len, m - 1)`, which is a
/// no-op in practice: every read's leading/trailing `M` span is ~30bp
/// (`READ_LEN` for SNV/MNV, `a`/`b` for indels) against a clip length capped
/// at `MAX_CLIP_LEN` (15).
fn apply_clip(cigar: &str, seq: Vec<u8>, pos: u32, clip: &ClipSpec) -> (String, Vec<u8>, u32) {
    let mut tokens = parse_cigar(cigar);
    let clip_char = match clip.kind {
        ClipKind::Soft => 'S',
        ClipKind::Hard => 'H',
    };

    match clip.side {
        ClipSide::FivePrime => {
            let (m_len, op) = tokens[0];
            assert_eq!(op, 'M', "leading CIGAR op must be M, got `{cigar}`");
            if m_len <= 1 {
                return (cigar.to_string(), seq, pos);
            }
            let c = clip.len.min(m_len - 1);
            tokens.splice(0..1, [(c, clip_char), (m_len - c, 'M')]);
            let new_seq = match clip.kind {
                ClipKind::Soft => seq,
                ClipKind::Hard => seq[c as usize..].to_vec(),
            };
            (format_cigar(&tokens), new_seq, pos + c)
        }
        ClipSide::ThreePrime => {
            let last = tokens.len() - 1;
            let (m_len, op) = tokens[last];
            assert_eq!(op, 'M', "trailing CIGAR op must be M, got `{cigar}`");
            if m_len <= 1 {
                return (cigar.to_string(), seq, pos);
            }
            let c = clip.len.min(m_len - 1);
            tokens.splice(last..=last, [(m_len - c, 'M'), (c, clip_char)]);
            let seq_len = seq.len();
            let new_seq = match clip.kind {
                ClipKind::Soft => seq,
                ClipKind::Hard => seq[..seq_len - c as usize].to_vec(),
            };
            (format_cigar(&tokens), new_seq, pos)
        }
    }
}

fn synthesize_reads(
    loci: &[Locus],
    sequence: &[u8],
    read_len: u32,
    contig_len: u32,
) -> Vec<ReadRecord> {
    let half = read_len / 2;
    let max_start = contig_len - read_len + 1;

    let mut reads = Vec::new();
    for (locus_index, locus) in loci.iter().enumerate() {
        let start = locus.pos.saturating_sub(half).max(1).min(max_start);

        for read_index in 0..locus.depth {
            let is_alt = read_index < locus.alt_count;
            let (cigar, seq) = build_read(&locus.kind, sequence, locus.pos, start, is_alt);

            // Clip every other read (~50%) when this locus carries a clip,
            // leaving the rest unclipped so the variant still calls.
            let (cigar, seq, pos) = match &locus.clip {
                Some(clip_spec) if read_index % 2 == 0 => {
                    apply_clip(&cigar, seq, start, clip_spec)
                }
                _ => (cigar, seq, start),
            };

            let flag = if read_index % 2 == 0 { 0 } else { 16 };
            reads.push(ReadRecord {
                qname: format!("r{locus_index}_{read_index}"),
                flag,
                pos,
                cigar,
                seq,
                mapq: 60,
                base_qual: 40,
            });
        }

        // Extra ALT-carrying reads flagged duplicate/secondary/supplementary.
        // Both tools must skip them entirely, so they ride on the exact same
        // start position and ALT encoding as a normal alt read -- only the
        // FLAG differs.
        for (noise_index, filter_flag) in locus.filtered_reads.iter().enumerate() {
            let (cigar, seq) = build_read(&locus.kind, sequence, locus.pos, start, true);
            let base_flag = if noise_index % 2 == 0 { 0 } else { 16 };
            let flag = base_flag | filter_flag.bit();
            reads.push(ReadRecord {
                qname: format!("rf{locus_index}_{noise_index}"),
                flag,
                pos: start,
                cigar,
                seq,
                mapq: 60,
                base_qual: 40,
            });
        }

        // Extra ALT-carrying reads whose MAPQ / base quality straddle the preset
        // filter floors. On top of the clean `depth`, so the variant still calls
        // at default; under -Q/-q some of these are dropped and BOTH tools must
        // drop the same ones (proven parity-safe by a manual spike).
        for (qi, qn) in locus.quality_noise.iter().enumerate() {
            let (cigar, seq) = build_read(&locus.kind, sequence, locus.pos, start, true);
            let flag = if qi % 2 == 0 { 0 } else { 16 };
            reads.push(ReadRecord {
                qname: format!("rq{locus_index}_{qi}"),
                flag,
                pos: start,
                cigar,
                seq,
                mapq: qn.mapq,
                base_qual: qn.base_qual,
            });
        }
    }

    // Coordinate order (SAM requires sorted input before samtools sort/index).
    reads.sort_by_key(|r| r.pos);
    reads
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    fn seq_of(len: usize) -> Vec<u8> {
        (0..len).map(|i| base_at_cycle(i)).collect()
    }

    #[test]
    fn five_prime_soft_shifts_pos_and_keeps_seq() {
        let seq = seq_of(66); // 30M6D30M read -> seq len = a + b = 60
        let seq = seq[..60].to_vec();
        let clip = ClipSpec {
            kind: ClipKind::Soft,
            side: ClipSide::FivePrime,
            len: 10,
        };
        let (cigar, new_seq, pos) = apply_clip("30M6D30M", seq.clone(), 100, &clip);
        assert_eq!(cigar, "10S20M6D30M");
        assert_eq!(pos, 110);
        assert_eq!(new_seq, seq, "soft clip must not alter SEQ");
        assert_eq!(new_seq.len(), 60, "soft clip keeps all bases in SEQ");
    }

    #[test]
    fn five_prime_hard_shifts_pos_and_trims_seq() {
        let seq = seq_of(60);
        let clip = ClipSpec {
            kind: ClipKind::Hard,
            side: ClipSide::FivePrime,
            len: 10,
        };
        let (cigar, new_seq, pos) = apply_clip("30M6D30M", seq.clone(), 100, &clip);
        assert_eq!(cigar, "10H20M6D30M");
        assert_eq!(pos, 110);
        assert_eq!(new_seq, seq[10..].to_vec(), "hard clip removes leading bases from SEQ");
        assert_eq!(new_seq.len(), 50);
    }

    #[test]
    fn three_prime_soft_keeps_pos_and_seq() {
        let seq = seq_of(60);
        let clip = ClipSpec {
            kind: ClipKind::Soft,
            side: ClipSide::ThreePrime,
            len: 10,
        };
        let (cigar, new_seq, pos) = apply_clip("30M6D30M", seq.clone(), 100, &clip);
        assert_eq!(cigar, "30M6D20M10S");
        assert_eq!(pos, 100, "3' clip never moves POS");
        assert_eq!(new_seq, seq, "soft clip must not alter SEQ");
    }

    #[test]
    fn three_prime_hard_keeps_pos_and_trims_seq() {
        let seq = seq_of(60);
        let clip = ClipSpec {
            kind: ClipKind::Hard,
            side: ClipSide::ThreePrime,
            len: 10,
        };
        let (cigar, new_seq, pos) = apply_clip("30M6D30M", seq.clone(), 100, &clip);
        assert_eq!(cigar, "30M6D20M10H");
        assert_eq!(pos, 100, "3' clip never moves POS");
        assert_eq!(new_seq, seq[..50].to_vec(), "hard clip removes trailing bases from SEQ");
        assert_eq!(new_seq.len(), 50);
    }

    #[test]
    fn single_m_op_cigar_clips_cleanly() {
        // Plain SNV/MNV reads are a single `<READ_LEN>M` token -- the leading
        // and trailing op are the same token, exercised by both sides.
        let seq = seq_of(60);
        let clip5 = ClipSpec {
            kind: ClipKind::Soft,
            side: ClipSide::FivePrime,
            len: 12,
        };
        let (cigar, _seq, pos) = apply_clip("60M", seq.clone(), 100, &clip5);
        assert_eq!(cigar, "12S48M");
        assert_eq!(pos, 112);

        let clip3 = ClipSpec {
            kind: ClipKind::Hard,
            side: ClipSide::ThreePrime,
            len: 12,
        };
        let (cigar, seq3, pos3) = apply_clip("60M", seq.clone(), 100, &clip3);
        assert_eq!(cigar, "48M12H");
        assert_eq!(pos3, 100);
        assert_eq!(seq3.len(), 48);
    }

    #[test]
    fn guard_skips_clip_when_m_op_is_length_one() {
        // m_len == 1: clamping to `m - 1 == 0` would emit a 0-length clip
        // token, so the guard must skip the clip entirely instead.
        let seq = seq_of(1);
        let clip = ClipSpec {
            kind: ClipKind::Soft,
            side: ClipSide::FivePrime,
            len: 5,
        };
        let (cigar, new_seq, pos) = apply_clip("1M", seq.clone(), 100, &clip);
        assert_eq!(cigar, "1M", "unmodified: too small to clip");
        assert_eq!(pos, 100);
        assert_eq!(new_seq, seq);
    }

    #[test]
    fn guard_clamps_clip_len_to_m_minus_one() {
        // Requested clip (15) exceeds what a 10bp M op can give up (9),
        // never zeroing out the M op.
        let seq = seq_of(10);
        let clip = ClipSpec {
            kind: ClipKind::Soft,
            side: ClipSide::FivePrime,
            len: 15,
        };
        let (cigar, _seq, pos) = apply_clip("10M", seq, 100, &clip);
        assert_eq!(cigar, "9S1M", "clamped to M-1, never a 0-length M op");
        assert_eq!(pos, 109);
    }

    /// Sampling guard: `optional_clip_strategy` is biased 3:1 (None:Some), so
    /// clips should appear in roughly a quarter of sampled loci -- neither
    /// vacuously absent (bug in the `prop_oneof!` weights) nor dominant
    /// (would starve the unclipped majority the fuzzer relies on for calls).
    #[test]
    fn optional_clip_strategy_samples_roughly_quarter_clipped() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::TestRunner;

        let mut runner = TestRunner::deterministic();
        let strategy = optional_clip_strategy();
        let sample_count = 2000;
        let mut clipped = 0;
        for _ in 0..sample_count {
            let tree = strategy.new_tree(&mut runner).expect("generate clip sample");
            if tree.current().is_some() {
                clipped += 1;
            }
        }
        let fraction = clipped as f64 / sample_count as f64;
        assert!(
            (0.15..=0.35).contains(&fraction),
            "expected ~25% of sampled loci to carry a clip (3:1 None:Some bias), got {fraction:.3} ({clipped}/{sample_count})"
        );
    }

    /// Sampling guard: `filtered_reads_strategy` is biased 3:1 (empty:non-empty),
    /// same weighting as `optional_clip_strategy`, so filtered-noise reads
    /// should appear in roughly a quarter of sampled loci -- neither vacuously
    /// absent (bug in the `prop_oneof!` weights, silently never exercising the
    /// duplicate/secondary/supplementary skip path) nor dominant (would starve
    /// the unfiltered majority).
    #[test]
    fn filtered_reads_strategy_samples_roughly_quarter_nonempty() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::TestRunner;

        let mut runner = TestRunner::deterministic();
        let strategy = filtered_reads_strategy();
        let sample_count = 2000;
        let mut nonempty = 0;
        for _ in 0..sample_count {
            let tree = strategy.new_tree(&mut runner).expect("generate filtered_reads sample");
            if !tree.current().is_empty() {
                nonempty += 1;
            }
        }
        let fraction = nonempty as f64 / sample_count as f64;
        assert!(
            (0.15..=0.35).contains(&fraction),
            "expected ~25% of sampled loci to carry filtered noise reads (3:1 empty:non-empty bias), got {fraction:.3} ({nonempty}/{sample_count})"
        );
    }

    /// Sampling guard: `quality_noise_strategy` is biased 3:1 (empty:non-empty),
    /// same weighting as `optional_clip_strategy` / `filtered_reads_strategy`,
    /// so quality-noise reads should appear in roughly a quarter of sampled loci
    /// -- neither vacuously absent (bug in the `prop_oneof!` weights, silently
    /// never exercising the -Q/-q straddle path) nor dominant (would starve the
    /// uniform-quality majority).
    #[test]
    fn quality_noise_strategy_samples_roughly_quarter_nonempty() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::TestRunner;

        let mut runner = TestRunner::deterministic();
        let strategy = quality_noise_strategy();
        let sample_count = 2000;
        let mut nonempty = 0;
        for _ in 0..sample_count {
            let tree = strategy.new_tree(&mut runner).expect("generate quality_noise sample");
            if !tree.current().is_empty() {
                nonempty += 1;
            }
        }
        let fraction = nonempty as f64 / sample_count as f64;
        assert!(
            (0.15..=0.35).contains(&fraction),
            "expected ~25% of sampled loci to carry quality-noise reads (3:1 empty:non-empty bias), got {fraction:.3} ({nonempty}/{sample_count})"
        );
    }

    #[test]
    fn never_produces_zero_length_m_op_across_clip_lens() {
        // Non-vacuity / CIGAR-validity guard: for every clip length 1..=20
        // against every M span 1..=40, the resulting CIGAR must never
        // contain a `0M` token on either side.
        for m_len in 1u32..=40 {
            for clip_len in 1u32..=20 {
                let seq = seq_of(m_len as usize);
                for side in [ClipSide::FivePrime, ClipSide::ThreePrime] {
                    for kind in [ClipKind::Soft, ClipKind::Hard] {
                        let clip = ClipSpec {
                            kind,
                            side,
                            len: clip_len,
                        };
                        let cigar_in = format!("{m_len}M");
                        let (cigar_out, _seq, _pos) =
                            apply_clip(&cigar_in, seq.clone(), 100, &clip);
                        let tokens = parse_cigar(&cigar_out);
                        for (len, op) in &tokens {
                            assert!(
                                *len > 0,
                                "produced a 0-length {op} op for m_len={m_len} clip_len={clip_len} side={side:?} kind={kind:?}: {cigar_out}"
                            );
                        }
                        // Total M-consuming length must still be m_len minus
                        // whatever the clip peeled off (never negative/absurd).
                        let m_total: u32 = tokens
                            .iter()
                            .filter(|(_, op)| *op == 'M')
                            .map(|(len, _)| len)
                            .sum();
                        assert!(
                            m_total >= 1 && m_total <= m_len,
                            "M total {m_total} out of range for m_len={m_len}: {cigar_out}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod preset_tests {
    use super::CURATED_GERMLINE_PRESETS;

    /// Typo-guard: every curated preset name must resolve to a non-empty flag
    /// set via `config_preset_java_flags` (which panics on an unknown name).
    #[test]
    fn curated_presets_all_resolve() {
        assert!(!CURATED_GERMLINE_PRESETS.is_empty());
        for name in CURATED_GERMLINE_PRESETS {
            let flags = crate::common::config_preset_java_flags(name);
            assert!(!flags.is_empty(), "preset {name} resolved to no flags");
        }
    }
}
