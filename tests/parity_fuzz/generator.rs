//! proptest strategies for the germline SNV differential-parity slice.
//!
//! ALL randomness that affects the synthesized genome flows through proptest
//! `Strategy` values here so that a failing case can be shrunk. Nothing in this
//! module (or downstream in `synth`/`oracle`) touches the `rand` crate.

use proptest::prelude::*;

/// Read length used for every synthesized read (perfect `<READ_LEN>M` match except
/// at the locus base). Matches the ~60bp scale used by the manual spike.
pub const READ_LEN: u32 = 60;

/// Minimum spacing (bp) enforced between consecutive loci so their read windows
/// never overlap (reads only ever span roughly `pos - READ_LEN/2 .. pos + READ_LEN/2`).
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

/// One SNV locus: a reference position covered by `depth` reads, `alt_count` of
/// which carry `alt_base` instead of `ref_base`.
#[derive(Debug, Clone)]
pub struct Locus {
    /// 1-based position of the SNV within the contig.
    pub pos: u32,
    pub ref_base: u8,
    pub alt_base: u8,
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
    /// Offset (1..=3) applied to the ref base's cycle index (mod 4) to pick a
    /// distinct alt base.
    alt_offset: u32,
}

fn locus_spec_strategy() -> impl Strategy<Value = LocusSpec> {
    (
        MIN_LOCUS_SPACING..=MAX_LOCUS_SPACING,
        30u32..=60,
        30u32..=70,
        1u32..=3,
    )
        .prop_map(
            |(spacing_from_previous, depth, alt_pct, alt_offset)| LocusSpec {
                spacing_from_previous,
                depth,
                alt_pct,
                alt_offset,
            },
        )
}

/// Strategy producing a `Genome` with 1..=8 loci spaced >=200bp apart on one
/// synthetic contig.
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

        let ref_base = base_at_cycle(pos as usize - 1);
        let ref_index = base_cycle_index(ref_base);
        let alt_base = BASES[(ref_index + spec.alt_offset as usize) % BASES.len()];

        let alt_count =
            ((spec.depth * spec.alt_pct) / 100).clamp(1, spec.depth.saturating_sub(1).max(1));

        loci.push(Locus {
            pos,
            ref_base,
            alt_base,
            depth: spec.depth,
            alt_count,
        });
    }

    // Trailing margin mirrors the leading one so the last locus's reads fit too.
    let contig_len = loci.last().map_or(leading_margin, |l| l.pos) + read_len + 50;
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
        let offset_in_read = (locus.pos - start) as usize;
        let template = &sequence[(start as usize - 1)..(start as usize - 1 + read_len as usize)];

        for read_index in 0..locus.depth {
            let mut seq = template.to_vec();
            seq[offset_in_read] = if read_index < locus.alt_count {
                locus.alt_base
            } else {
                locus.ref_base
            };

            let flag = if read_index % 2 == 0 { 0 } else { 16 };
            reads.push(ReadRecord {
                qname: format!("r{locus_index}_{read_index}"),
                flag,
                pos: start,
                seq,
            });
        }
    }

    // Coordinate order (SAM requires sorted input before samtools sort/index).
    reads.sort_by_key(|r| r.pos);
    reads
}
