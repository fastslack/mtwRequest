//! Load-robust in-process ratios.
//!
//! Absolute nanosecond timings drift wildly when the box is busy, but the
//! *ratio* between two variants measured back-to-back in the same process under
//! the same contention is stable: both pay the same scheduling tax. We use this
//! to compare wire formats, the cached-fanout path, and allocator throughput.
//!
//! Run: `cargo run -p mtw-benches --bin ratio --release`

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::time::Instant;

use mtw_benches::sample_message;
use mtw_codec::json::JsonCodec;
use mtw_codec::MtwCodec;
use mtw_protocol::SharedEnvelope;

/// Time `n` runs of `f`, returning total nanoseconds.
fn time<F: FnMut()>(n: u64, mut f: F) -> u128 {
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    start.elapsed().as_nanos()
}

/// Median ratio of a/b over `rounds`, interleaving a and b each round so
/// transient load hits both equally.
fn ratio<FA: FnMut(), FB: FnMut()>(label: &str, n: u64, rounds: usize, mut fa: FA, mut fb: FB) {
    let mut ratios: Vec<f64> = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let ta = time(n, &mut fa) as f64;
        let tb = time(n, &mut fb) as f64;
        ratios.push(ta / tb);
    }
    ratios.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let med = ratios[ratios.len() / 2];
    let lo = ratios[0];
    let hi = ratios[ratios.len() - 1];
    println!("{label:<50} median {med:>5.2}×  (range {lo:.2}–{hi:.2}×)");
}

fn main() {
    const N: u64 = 200_000;
    const ROUNDS: usize = 11;
    let codec = JsonCodec;

    println!("# Load-robust in-process ratios ({ROUNDS} rounds, {N} iters each)\n");
    println!("Higher than 1.00× means the FIRST named variant is that much SLOWER.\n");

    // 1) JSON encode vs MsgPack encode (same message).
    for &size in &[16usize, 256, 4096] {
        let msg = sample_message(size);
        ratio(
            &format!("json/encode ÷ msgpack/encode (size={size})"),
            N,
            ROUNDS,
            || {
                let b = codec.encode(black_box(&msg)).unwrap();
                black_box(&b);
            },
            || {
                let b = rmp_serde::to_vec_named(black_box(&msg)).unwrap();
                black_box(&b);
            },
        );
    }

    println!();

    // 2) Naive re-encode-per-subscriber vs cached SharedEnvelope fanout.
    const SUBS: u64 = 100;
    for &size in &[256usize, 4096] {
        let msg = sample_message(size);
        ratio(
            &format!("re-encode×{SUBS} ÷ cached-envelope×{SUBS} (size={size})"),
            2_000,
            ROUNDS,
            || {
                for _ in 0..SUBS {
                    let b = codec.encode(black_box(&msg)).unwrap();
                    black_box(&b);
                }
            },
            || {
                let env = SharedEnvelope::new(sample_message(size));
                let _ = env.text_bytes();
                for _ in 0..SUBS {
                    let b = env.text_bytes();
                    black_box(&b);
                }
            },
        );
    }

    println!();

    // 3) Allocator throughput: system malloc ÷ mimalloc, for the small-object
    //    alloc/free pattern a JSON decode produces (~15 allocs of 16–256 B).
    //    Both allocators are exercised in the SAME process, interleaved, so the
    //    ratio is robust to CPU load. This is the honest, load-independent way
    //    to quantify what `#[global_allocator] mimalloc` buys on this workload —
    //    a global allocator can't be A/B'd in one process, but its raw
    //    alloc/free throughput can, via the GlobalAlloc trait directly.
    let mi = mimalloc::MiMalloc;
    for &(count, sz) in &[(15usize, 64usize), (15, 256), (64, 64)] {
        ratio(
            &format!("system-malloc ÷ mimalloc ({count}×{sz}B alloc/free)"),
            20_000,
            ROUNDS,
            || {
                // System allocator: alloc then free `count` small blocks.
                let layout = Layout::from_size_align(sz, 8).unwrap();
                let mut ptrs = [std::ptr::null_mut::<u8>(); 64];
                for p in ptrs.iter_mut().take(count) {
                    *p = unsafe { System.alloc(layout) };
                    // touch first byte so the alloc isn't optimized away
                    if !p.is_null() {
                        unsafe { p.write(1) };
                    }
                }
                for p in ptrs.iter().take(count) {
                    if !p.is_null() {
                        unsafe { System.dealloc(*p, layout) };
                    }
                }
                black_box(&ptrs);
            },
            || {
                // mimalloc: same pattern via the GlobalAlloc trait directly.
                let layout = Layout::from_size_align(sz, 8).unwrap();
                let mut ptrs = [std::ptr::null_mut::<u8>(); 64];
                for p in ptrs.iter_mut().take(count) {
                    *p = unsafe { mi.alloc(layout) };
                    if !p.is_null() {
                        unsafe { p.write(1) };
                    }
                }
                for p in ptrs.iter().take(count) {
                    if !p.is_null() {
                        unsafe { mi.dealloc(*p, layout) };
                    }
                }
                black_box(&ptrs);
            },
        );
    }
}
