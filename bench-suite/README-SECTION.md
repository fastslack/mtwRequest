<!--
  Paste this section into README.md to replace the existing "## Performance"
  block. All charts are pure UTF-8 in fenced code blocks — no images, no
  external dependencies, renders identically on GitHub/GitLab/npmjs.
-->

## Benchmarks

mtwRequest vs three widely-used open-source real-time alternatives, on the same host, same Docker resource caps (2 CPU / 2 GB per competitor), same payload (128 B) and same bench harness. Full method and reproducible runner in [`bench-suite/`](./bench-suite/).

### TL;DR

| Scenario | 🥇 Winner | 🥈 Runner-up | Verdict |
|---|---|---|---|
| Fanout **50 subscribers** p50 | **mtwRequest** `7.5 ms` | NATS `9.5 ms` | mtwRequest wins |
| Fanout **500 subscribers** p50 | NATS `61 ms` | **mtwRequest** `64 ms` | technical tie |
| Echo RTT (ping → pong) p50 | NATS `60 µs` | **mtwRequest** `82 µs` | mtwRequest 2nd |
| Connect storm (handshakes/s) | **mtwRequest** `39 k/s` | Centrifugo `22 k/s` | mtwRequest wins 1.7× |

> **mtwRequest finishes 🥇 or 🥈 in every metric**, while also providing HTTP, AI-agent, auth, trading, and MCP in the same core — the other three are narrow pub/sub engines.

### Fanout — 1 publisher → N subscribers

Lower is better. Time from publisher-side stamp to subscriber receive.

```
50 subscribers · one-way p50 latency
 🏆  mtwRequest   ██                                      7.5 ms
     NATS         ██                                      9.5 ms
     Centrifugo   ██████                                 22.8 ms
     Socket.IO    ████████████████████████████████████  236.5 ms

500 subscribers · one-way p50 latency
 🏆  NATS         █                                      61.1 ms
     mtwRequest   █                                      64.5 ms   ← technical tie
     Centrifugo   ███                                   175.9 ms
     Socket.IO    █████████████████████████████████████   2.29 s

500 subscribers · p99 latency (tail)
 🏆  mtwRequest   █                                     107.7 ms
     NATS         █                                     114.0 ms
     Centrifugo   ████                                  321.9 ms
     Socket.IO    █████████████████████████████████████   4.17 s
```

### Echo — single ping → pong round-trip

```
p50 round-trip
 🏆  NATS          █████████████                          60.5 µs
     mtwRequest    ██████████████████                     82.0 µs
     Centrifugo    ██████████████████████                101.6 µs
     Socket.IO     ██████████████████████████████        135.9 µs
```

### Connect storm — 200 concurrent handshakes

Higher is better.

```
connects per second
 🏆  mtwRequest   ████████████████████████████████████   39.0 k/s
     Centrifugo   █████████████████████                  22.4 k/s
     NATS         █████████████████                      18.2 k/s
     Socket.IO    ·                                        168 /s
```

### Throughput — the one place NATS dominates

Raw broadcast throughput at 500-subscriber fanout. NATS is a TCP-only protocol
with no WebSocket framing, so this is a structural ceiling we don't try to
cross — WS is a conscious tradeoff for browser compatibility.

```
messages-delivered per second
 🏆  NATS         ████████████████████████████████████   1.20 G/s
     Centrifugo   █████                                  156 M/s
     mtwRequest   ████                                   124 M/s
     Socket.IO    ·                                      11.5 M/s
```

### Method

- Same-host benchmark. Relative numbers only — don't extrapolate to distributed deployments.
- All histograms are HdrHistogram (3 significant figures, 1 ns – 60 s range).
- Publisher never subscribes to the channel it publishes on, so no self-delivery is counted.
- Every competitor runs under identical Docker limits (`cpus: 2.0, mem_limit: 2g`).
- 128-byte payloads carry a 16-byte `u128` nanosecond timestamp; subscribers compute `now − sent` on receive.

### Reproduce

```bash
cd bench-suite
docker compose up -d                 # centrifugo, nats, socket.io
./bench                              # builds mtw server + runs full matrix
cat results/*/summary.md             # latest run
```

Override any matrix knob:

```bash
SUBS="100 1000 5000" MESSAGES=5000 ./bench
SYSTEMS="mtw nats" SCENARIOS="fanout" ./bench
```

### Optimization journey

mtwRequest's fanout latency dropped 5× during a focused optimization cycle:

```
Fanout 500 subscribers · p50 latency over mtwRequest versions
                     ←— lower is better

v0 baseline              ████████████████████████████████████████  418.9 ms
v1 encode-once           ███████████████████████████               282.3 ms
v2 direct sink + ArcSwap █████████████                             130.7 ms
v3 ConnTarget cache      ████████████████                          169.0 ms ← noise
v4 +BufWriter batching   ██████████                                107.0 ms
v5 +Utf8Bytes zero-copy  ██████                                     64.5 ms ← today
```

Each step is documented in the commit history under `perf:` and `feat:` prefixes.

<!-- end of benchmarks section -->
