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
}

/// One synthesized read record, materialized from a `Locus`.
#[derive(Debug, Clone)]
pub struct ReadRecord {
    pub qname: String,
    /// SAM FLAG: 0 (forward) or 16 (reverse), alternating per read.
    pub flag: u16,
    /// 1-based leftmost mapping position.
    pub pos: u32,
    pub cigar: String,
    pub seq: Vec<u8>,
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
}

fn locus_spec_strategy() -> impl Strategy<Value = LocusSpec> {
    (
        MIN_LOCUS_SPACING..=MAX_LOCUS_SPACING,
        30u32..=60,
        30u32..=70,
        variant_kind_spec_strategy(),
    )
        .prop_map(
            |(spacing_from_previous, depth, alt_pct, kind_spec)| LocusSpec {
                spacing_from_previous,
                depth,
                alt_pct,
                kind_spec,
            },
        )
}

/// Strategy producing a `Genome` with 1..=8 loci (each independently SNV,
/// deletion, or insertion) spaced >=200bp apart on one synthetic contig.
pub fn arb_genome() -> impl Strategy<Value = Genome> {
    prop::collection::vec(locus_spec_strategy(), 1..=8).prop_map(build_genome)
}

fn build_genome(specs: Vec<LocusSpec>) -> Genome {
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
        };

        let alt_count =
            ((spec.depth * spec.alt_pct) / 100).clamp(1, spec.depth.saturating_sub(1).max(1));

        loci.push(Locus {
            pos,
            kind,
            depth: spec.depth,
            alt_count,
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

    Genome {
        contig: "chrS".to_string(),
        sequence,
        read_len,
        loci,
        reads,
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

            let flag = if read_index % 2 == 0 { 0 } else { 16 };
            reads.push(ReadRecord {
                qname: format!("r{locus_index}_{read_index}"),
                flag,
                pos: start,
                cigar,
                seq,
            });
        }
    }

    // Coordinate order (SAM requires sorted input before samtools sort/index).
    reads.sort_by_key(|r| r.pos);
    reads
}
