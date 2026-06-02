# Performance measurement log

Captured on a **loaded machine** (load average 7–19 on 12 cores during the
session). That load makes wall-clock timing unreliable, so the deltas here use
methods that are robust to CPU contention. Every table is real captured output.

## Why wall-clock timing was not used for deltas

The same already-compiled binary, benchmarked twice against the same baseline
minutes apart under this load:

| bench | run A | run B |
|---|---|---|
| codec/json/encode/16 | +50.9% | +8.2% |
| codec/json/decode/256 | −0.7% | +69.9% |
| codec/json/encode/4096 | +84.8% | +60.4% |

Identical code, deltas swinging 40–70 points → noise. So timing-based deltas
(and the build-level LTO/mimalloc effects, ~5–15% each) cannot be honestly
quantified on this box. They need an idle machine.

Two methods *are* load-robust and are used below.

---

## APPLIED CHANGE — presize encode buffers + frame zero-copy

Three edits, all targeting heap allocations on the encode/fanout hot paths:

1. `crates/mtw-codec/src/json.rs` — `JsonCodec::encode` now serializes into a
   presized `Vec::with_capacity(512)` via `serde_json::to_writer` instead of
   `serde_json::to_vec` (which starts at 128 B and reallocs as it grows).
2. `crates/mtw-protocol/src/frame.rs` — `Frame::encode_message` serializes JSON
   **directly into the framed `BytesMut`** and backfills the length prefix,
   instead of building an intermediate `Vec` and copying it into the frame.
3. `crates/mtw-protocol/src/envelope.rs` — `SharedEnvelope::text_bytes`/`text`
   use the same presized encode helper.

### Measured: allocations per operation (deterministic, load-independent)

A counting global allocator records exact heap (re)allocations per op — the
result is byte-identical at any CPU load (verified across 3 runs).
Source: `crates/mtw-benches/src/bin/allocs.rs`. Run:
`cargo run -p mtw-benches --bin allocs --release`

| operation | allocs BEFORE | allocs AFTER | reduction | bytes BEFORE→AFTER |
|---|---|---|---|---|
| json/encode/16 | 4.00 | **2.00** | −50% | 920 → 536 |
| json/encode/256 | 4.00 | **2.00** | −50% | 1271 → 536 |
| json/encode/4096 | 4.00 | 4.00 | — | 12791 → 13175 |
| frame/encode/16 | 4.00 | **2.00** | −50% | 1171 → 545 |
| frame/encode/256 | 4.00 | **2.00** | −50% | 1762 → 545 |
| frame/encode/4096 | 4.00 | 4.00 | — | 17122 → 13211 |
| envelope: new + 3×text / 16 | 20.00 | **18.00** | −10% | 1918 → 1534 |
| envelope: new + 3×text / 256 | 20.00 | **18.00** | −10% | 2749 → 2014 |
| envelope: new + 3×text / 4096 | 20.00 | 20.00 | — | 21949 → 22333 |
| json/decode/* (untouched) | 15.00 | 15.00 | — | unchanged |

Real result (matches captured output exactly):
- **Small/medium messages — the common case — now do 2 allocations instead of 4
  on both the JSON and binary-frame encode paths (−50%).** Bytes requested also
  dropped sharply (json/encode/256: 1271→536; frame/encode/256: 1762→545 — the
  frame path also lost its intermediate-`Vec`-then-copy).
- **4 KB+ messages: unchanged (4 allocs)** — they outgrow the 512 B presize, so
  the buffer still reallocs. A larger default would trade wasted memory on every
  small message for fewer reallocs on rare big ones; left at 512 deliberately.
- Envelope path: −10% (the `new` + first encode shed allocs; the 2 extra
  `text_bytes()` reads were already 0-alloc refcount bumps, as designed).

The remaining 2nd allocation on small messages is `Vec::with_capacity(512)` plus
serde_json's internal bookkeeping; getting to 1 would need a reusable
thread-local buffer, which adds complexity for one alloc — not done.

Correctness: `cargo test -p mtw-protocol -p mtw-codec` → all pass (frame and
codec roundtrip tests included); dependents (`mtw-transport`, `mtw-router`,
`mtw-core`) build clean.

---

## Fanout audit (this round) — no change needed

Investigated whether any code path re-encodes a message per recipient/channel.
Walked every encode/fanout site in the transport-layer crates:

| site | verdict |
|---|---|
| `ws.rs::broadcast` | encodes once, clones `WsMessage` (refcounted) per conn — OK |
| `ws.rs::send_envelope` / `ConnTarget` | cached envelope, refcount per conn — OK |
| `channel.rs::publish` | one `SharedEnvelope`, refcount per subscriber — OK |
| `router.rs` `MsgType::Publish` | resolves **one** exact-named channel, publishes once — OK |
| `bridge/server.rs` event fanout | encode-once then fan out — OK |
| `whatsapp.rs` `publish_event` | distinct message per channel by design — OK |

**Result: nothing to fix.** The fanout layer was already optimal — every real
path encodes a message at most once and hands out refcounted clones. (An earlier
speculative `publish_to`/`publish_envelope` change was reverted: there is no
glob-fanout caller in the codebase, so it would have been dead code measured
against a synthetic benchmark. Honest negative result.)

---

## Pre-existing characteristics (for context / next targets)

### Load-robust in-process ratios

Two variants timed back-to-back in one process; load taxes both equally so the
ratio is stable. 11 rounds × 200k iters, medians from two runs.
Source: `crates/mtw-benches/src/bin/ratio.rs`.

| comparison | run 1 | run 2 |
|---|---|---|
| json/encode ÷ msgpack/encode (16 B) | 1.17× | 1.15× |
| json/encode ÷ msgpack/encode (256 B) | 1.50× | 1.52× |
| json/encode ÷ msgpack/encode (4096 B) | 6.68× | 6.40× |
| re-encode×100 ÷ cached-envelope×100 (256 B) | 34.98× | 33.57× |
| re-encode×100 ÷ cached-envelope×100 (4096 B) | 79.49× | 78.67× |

- MsgPack encode beats JSON at every size; the gap grows with payload
  (1.15× → ~6.5×), because JSON must scan+escape every byte of a large string
  while msgpack length-prefixes raw bytes.
- Cached `SharedEnvelope` fanout is **34×–79× faster** than re-encoding per
  subscriber — any path calling `codec.encode()` in a per-recipient loop pays
  this.

### Wire size: JSON vs MsgPack (deterministic)

| payload | json | msgpack | saving |
|---|---|---|---|
| size=16 | 266 B | 215 B | 19.2% |
| size=256 | 506 B | 457 B | 9.7% |
| size=4096 | 4346 B | 4297 B | 1.1% |

Note: `envelope.rs` comments claim MsgPack is "40–50% smaller"; measured it is
19% at best and ~1% for large bodies. The 40–50% only holds for messages with
many small typed fields, not string-heavy payloads.

---

## Next targets (identified, not yet done)

1. **Decode: 15 allocs/op** (vs encode's 2 now) — the biggest remaining
   allocation cost. A borrowing/zero-copy decoder is the win. (Touches public
   `MtwMessage` API — more invasive.)
2. **Default hot traffic to MsgPack** — 1.2×–6.5× faster encode, smaller wire.
   (Changes default wire format — needs client coordination.)
3. **LTO + `codegen-units=1` and mimalloc** — needs an idle box to quantify.

## Reproduce

```
cargo run -p mtw-benches --bin allocs --release   # deterministic, any load
cargo run -p mtw-benches --bin ratio  --release   # load-robust ratios
cargo bench --bench codec --bench channel --bench middleware   # needs idle box
```
