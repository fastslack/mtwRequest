//! Channel publish fan-out benchmark. Measures how quickly a single published
//! message is dispatched to N subscribers via `Channel::publish`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mtw_benches::sample_message;
use mtw_router::ChannelManager;
use tokio::runtime::Runtime;

fn bench_publish(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("channel/publish");
    for &subs in &[1usize, 16, 256, 4096] {
        group.throughput(Throughput::Elements(subs as u64));
        group.bench_with_input(BenchmarkId::from_parameter(subs), &subs, |b, &subs| {
            b.to_async(&rt).iter_batched(
                || {
                    let mut manager = ChannelManager::new();
                    // Drain the outgoing mpsc so the channel doesn't block on a
                    // full receiver. The receiver is consumed here but we don't
                    // need it for this benchmark.
                    let _ = manager.take_message_receiver();
                    let channel = manager.get_or_create("bench");
                    for i in 0..subs {
                        let id = format!("conn-{i}");
                        channel.subscribe(&id).unwrap();
                    }
                    (channel, sample_message(64))
                },
                |(channel, msg)| async move {
                    let sent = channel.publish(black_box(msg), None).await.unwrap();
                    debug_assert!(sent > 0);
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_publish);
criterion_main!(benches);
