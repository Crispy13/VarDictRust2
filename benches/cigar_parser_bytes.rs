use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

const READ_LENGTHS: [usize; 4] = [76, 101, 151, 250];
const MAX_PHRED: u8 = 93;

fn sequence_string_current(sequence: &[u8]) -> String {
    String::from_utf8(sequence.to_vec()).unwrap_or_default()
}

fn sequence_bytes_proposed(sequence: &[u8]) -> Vec<u8> {
    sequence.to_vec()
}

fn quality_string_current(qualities: &[u8]) -> String {
    let mut result = Vec::with_capacity(qualities.len());
    for &quality in qualities {
        result.push(quality.min(MAX_PHRED) + 33);
    }
    String::from_utf8(result).expect("base qualities are capped to printable ASCII")
}

fn quality_bytes_proposed(qualities: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(qualities.len());
    for &quality in qualities {
        result.push(quality.min(MAX_PHRED) + 33);
    }
    result
}

fn sequence_for_len(len: usize) -> Vec<u8> {
    const BASES: &[u8] = b"ACGTN";
    (0..len).map(|index| BASES[index % BASES.len()]).collect()
}

fn qualities_for_len(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 120) as u8).collect()
}

fn bench_sequence_materialization(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("cigar_parser_sequence_materialization");
    for len in READ_LENGTHS {
        let sequence = sequence_for_len(len);
        group.throughput(Throughput::Bytes(len as u64));
        group.bench_with_input(
            BenchmarkId::new("current_string_from_utf8", len),
            &sequence,
            |b, sequence| {
                b.iter(|| black_box(sequence_string_current(black_box(sequence))));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("proposed_bytes", len),
            &sequence,
            |b, sequence| {
                b.iter(|| black_box(sequence_bytes_proposed(black_box(sequence))));
            },
        );
    }
    group.finish();
}

fn bench_quality_materialization(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("cigar_parser_quality_materialization");
    for len in READ_LENGTHS {
        let qualities = qualities_for_len(len);
        group.throughput(Throughput::Bytes(len as u64));
        group.bench_with_input(
            BenchmarkId::new("current_cap_then_string_from_utf8", len),
            &qualities,
            |b, qualities| {
                b.iter(|| black_box(quality_string_current(black_box(qualities))));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("proposed_cap_bytes", len),
            &qualities,
            |b, qualities| {
                b.iter(|| black_box(quality_bytes_proposed(black_box(qualities))));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_sequence_materialization,
    bench_quality_materialization
);
criterion_main!(benches);
