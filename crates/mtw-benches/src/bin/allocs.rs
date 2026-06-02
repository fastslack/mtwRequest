//! Deterministic allocation / wire-size measurements for the hot paths.
//!
//! Unlike the Criterion timing benchmarks, these numbers do NOT depend on CPU
//! load — a counting global allocator records the exact number of heap
//! allocations and bytes requested per operation. The result is identical on an
//! idle box or one at load 20, which is why this is the trustworthy measurement
//! when the machine is busy.
//!
//! Run: `cargo run -p mtw-benches --bin allocs --release`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use mtw_benches::sample_message;
use mtw_codec::json::JsonCodec;
use mtw_codec::MtwCodec;
use mtw_protocol::SharedEnvelope;

/// System allocator wrapper that counts live allocation calls and total bytes.
struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) == 1 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) == 1 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Measure (allocs, bytes) attributable to `n` runs of `f`, averaged per run.
fn measure<F: FnMut()>(n: u64, mut f: F) -> (f64, f64) {
    // Warm any lazy statics first, outside the counted region.
    f();
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(1, Ordering::Relaxed);
    for _ in 0..n {
        f();
    }
    ENABLED.store(0, Ordering::Relaxed);
    let a = ALLOCS.load(Ordering::Relaxed) as f64 / n as f64;
    let b = BYTES.load(Ordering::Relaxed) as f64 / n as f64;
    (a, b)
}

fn main() {
    const N: u64 = 100_000;
    let codec = JsonCodec;

    println!("# Deterministic per-operation cost (load-independent)\n");
    println!(
        "{:<34} {:>12} {:>14}",
        "operation", "allocs/op", "bytes/op"
    );
    println!("{}", "-".repeat(62));

    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        let (a, b) = measure(N, || {
            let out = codec.encode(&msg).unwrap();
            std::hint::black_box(&out);
        });
        println!("{:<34} {:>12.2} {:>14.0}", format!("json/encode/{size}"), a, b);
    }

    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        let bytes = codec.encode(&msg).unwrap();
        let (a, b) = measure(N, || {
            let out: mtw_protocol::MtwMessage = codec.decode(&bytes).unwrap();
            std::hint::black_box(&out);
        });
        println!("{:<34} {:>12.2} {:>14.0}", format!("json/decode/{size}"), a, b);
    }

    // Binary MTW frame encode (the `binary_connections` wire path).
    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        let (a, b) = measure(N, || {
            let out = mtw_protocol::frame::Frame::encode_message(&msg).unwrap();
            std::hint::black_box(&out);
        });
        println!("{:<34} {:>12.2} {:>14.0}", format!("frame/encode/{size}"), a, b);
    }

    // SharedEnvelope: encode once, then N cheap refcount clones (the fanout path).
    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        let (a, b) = measure(N, || {
            let env = SharedEnvelope::new(sample_message(size));
            // first text_bytes() encodes; subsequent clones are refcount bumps
            let t1 = env.text_bytes();
            let t2 = env.text_bytes();
            let t3 = env.text_bytes();
            std::hint::black_box((&t1, &t2, &t3));
        });
        let _ = msg;
        println!(
            "{:<34} {:>12.2} {:>14.0}",
            format!("envelope/new+3×text/{size}"),
            a,
            b
        );
    }

    // Decode allocation breakdown: same message, progressively stripped down,
    // to attribute the 15 allocs to specific fields. Each row decodes a message
    // built from the JSON of a deliberately minimal MtwMessage.
    use mtw_protocol::{MsgType, MtwMessage, Payload};
    let cases: [(&str, MtwMessage); 5] = [
        (
            "decode/minimal(None payload)",
            MtwMessage::new(MsgType::Event, Payload::None),
        ),
        (
            "decode/+text payload",
            MtwMessage::new(MsgType::Event, Payload::Text("hello world".into())),
        ),
        (
            "decode/+channel",
            MtwMessage::new(MsgType::Event, Payload::Text("hello world".into()))
                .with_channel("ticker.btcusd"),
        ),
        (
            "decode/+1 metadata",
            MtwMessage::new(MsgType::Event, Payload::Text("hello world".into()))
                .with_channel("ticker.btcusd")
                .with_metadata("source", serde_json::json!("bench")),
        ),
        (
            "decode/+json payload(5 fields)",
            sample_message(16),
        ),
    ];
    for (label, msg) in &cases {
        let bytes = codec.encode(msg).unwrap();
        let (a, b) = measure(N, || {
            let out: MtwMessage = codec.decode(&bytes).unwrap();
            std::hint::black_box(&out);
        });
        println!("{label:<34} {a:>12.2} {b:>14.0}");
    }

    // Wire size: JSON vs MsgPack (deterministic, the documented size win).
    println!("\n# Wire size: JSON vs MsgPack (bytes on the wire)\n");
    println!("{:<20} {:>10} {:>10} {:>10}", "payload", "json", "msgpack", "saving");
    println!("{}", "-".repeat(52));
    for &size in &[16usize, 256, 4096] {
        let env = SharedEnvelope::new(sample_message(size));
        let j = env.text_bytes().len();
        let m = env.msgpack().len();
        let saving = 100.0 * (j as f64 - m as f64) / j as f64;
        println!("{:<20} {:>10} {:>10} {:>9.1}%", format!("size={size}"), j, m, saving);
    }
}
