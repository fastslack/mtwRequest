//! Middleware chain throughput: N passthrough middlewares each adding a
//! metadata key.

use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mtw_benches::sample_message;
use mtw_core::MtwError;
use mtw_protocol::MtwMessage;
use mtw_router::{MiddlewareAction, MiddlewareChain, MiddlewareContext, MtwMiddleware};
use std::sync::Arc;
use tokio::runtime::Runtime;

struct AddTag {
    name: String,
    priority: i32,
}

#[async_trait]
impl MtwMiddleware for AddTag {
    fn name(&self) -> &str {
        &self.name
    }
    fn priority(&self) -> i32 {
        self.priority
    }
    async fn on_inbound(
        &self,
        msg: MtwMessage,
        _ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction, MtwError> {
        Ok(MiddlewareAction::Continue(
            msg.with_metadata(self.name.clone(), serde_json::json!(self.priority)),
        ))
    }
}

fn bench_chain(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("middleware/process_inbound");
    for &n in &[1usize, 4, 16, 64] {
        let mut chain = MiddlewareChain::new();
        for i in 0..n {
            chain.add(Arc::new(AddTag {
                name: format!("mw-{i}"),
                priority: i as i32,
            }));
        }
        let ctx = MiddlewareContext {
            conn_id: "bench".to_string(),
            channel: None,
        };

        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(n), &chain, |b, chain| {
            b.to_async(&rt).iter_batched(
                || sample_message(64),
                |msg| async {
                    let out = chain.process_inbound(black_box(msg), &ctx).await.unwrap();
                    debug_assert!(out.is_some());
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_chain);
criterion_main!(benches);
