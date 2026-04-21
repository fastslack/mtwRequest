//! JSON codec encode/decode throughput benchmarks.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mtw_codec::json::JsonCodec;
use mtw_codec::MtwCodec;
use mtw_benches::sample_message;

fn bench_encode(c: &mut Criterion) {
    let codec = JsonCodec;
    let mut group = c.benchmark_group("codec/json/encode");
    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(size), &msg, |b, msg| {
            b.iter(|| codec.encode(black_box(msg)).unwrap());
        });
    }
    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let codec = JsonCodec;
    let mut group = c.benchmark_group("codec/json/decode");
    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        let bytes = codec.encode(&msg).unwrap();
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &bytes, |b, bytes| {
            b.iter(|| codec.decode(black_box(bytes)).unwrap());
        });
    }
    group.finish();
}

criterion_group!(benches, bench_encode, bench_decode);
criterion_main!(benches);
