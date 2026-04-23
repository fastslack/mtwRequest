# bench-suite

Comparative pub/sub benchmark for **mtwRequest** against three common
open-source alternatives:

| System | Protocol | Transport |
| --- | --- | --- |
| **mtwRequest** | `MtwMessage` JSON | WebSocket |
| **Centrifugo v5** | Centrifuge JSON | WebSocket |
| **Socket.IO v4** | engine.io v4 | WebSocket / HTTP longpoll |
| **NATS 2.x** | NATS protocol | TCP |

## Quick start

**One command does everything:**

```bash
cd bench-suite
./bench
```

That will:
1. `docker compose up -d --wait` — start centrifugo, nats, socket.io (with healthchecks).
2. Build + launch `bench-mtw-server` on a free port (avoiding the usual 7741 squatters).
3. Run every scenario against every system.
4. Write JSON results to `results/<timestamp>/`.
5. Print and save a markdown summary.
6. Generate PNG/SVG charts if `matplotlib` is installed.

### Just check that everything is reachable

```bash
./bench check
```

### Stop everything

```bash
./bench clean
```

### Running a subset

```bash
SYSTEMS="mtw nats"    ./bench        # only these two
SCENARIOS="fanout"    ./bench        # only fanout
SUBS="100 1000 5000"  ./bench        # fanout tiers
MESSAGES=10000        ./bench        # messages per fanout
CONNECT_COUNT="2000"  ./bench        # connect storm size
```

## Scenarios

| Name | Shape | Primary metric |
| --- | --- | --- |
| **fanout** | 1 publisher, N subscribers on the same channel | one-way p50 / p99 latency, deliveries/sec |
| **echo** | 1 requester ↔ 1 echo-bot through two channels | round-trip p50 / p99, RTTs/sec |
| **connect** | Open N connections with bounded concurrency | connects/sec, p99 connect latency |

## Method

- Every message carries a 16-byte little-endian `u128` nanosecond timestamp in
  the leading bytes. Subscribers compute `now - sent` on each receive; the
  header survives the wire unchanged across all four systems.
- Publishers **never subscribe** to the channel they publish on, so no
  self-delivery ever gets counted.
- Histograms are HdrHistograms (3 significant figures, 1 ns → 60 s range).
- Warmup messages are sent but their latencies are dropped.
- Competitor servers are capped at **2 CPU / 2 GB** (see `docker-compose.yml`).

The bench is **same-host** — clients and servers share the kernel. Numbers are
useful for *relative* comparison on a given machine; don't extrapolate to
distributed deployments.

## Layout

```
bench-suite/
├── bench                            # ← one-shot entry point (the only command you need)
├── Cargo.toml                       # workspace (separate from main repo)
├── crates/
│   ├── bench-metrics/               # HdrHistogram + BenchResult JSON schema
│   ├── bench-clients/               # BenchClient trait + 4 impls
│   ├── bench-runner/                # CLI: fanout / echo / connect
│   └── bench-mtw-server/            # minimal mtw server for the mtw runs
├── servers/
│   ├── centrifugo/config.json       # v5 namespace-style config
│   ├── nats/nats.conf
│   └── socketio/{Dockerfile,package.json,server.js}
├── docker-compose.yml               # all 3 competitors + healthchecks
├── scripts/
│   ├── summary.py                   # stdlib-only → markdown table
│   └── plot.py                      # matplotlib → PNG/SVG (optional)
└── results/<timestamp>/
    ├── *.json                       # one per (system, scenario, tag)
    ├── summary.md                   # human-readable table
    └── {fanout,echo,connect}.{png,svg}   # if matplotlib present
```

## Manual one-off

If you need to run a single scenario by hand after `./bench check`:

```bash
./target/release/bench-runner --system nats \
    fanout --subs 100 --messages 1000
```

`--url` overrides the default connection URL for each system.

## Caveats

- **"Why does curl show 400?"** Hitting `http://127.0.0.1:13000/socket.io/` in
  a browser or with curl returns 400 by design — Socket.IO expects engine.io
  query params. The 400 means the server is alive; `./bench check` does the
  proper probe via `/healthz`.
- **Centrifugo v5 namespace config**: channel policies (`allow_publish_for_client`,
  etc.) live in `channel.without_namespace`, not at the config root. `./bench`
  handles this; if you edit the config, reload with `docker compose restart
  centrifugo`.
- **Port conflicts**: Socket.IO maps to host port **13000** by default (not
  3000, which is commonly taken). Override with `SOCKETIO_HOST_PORT=4000`.
- **ulimit**: scenarios open thousands of sockets. If you hit "Too many open
  files": `ulimit -n 65535`.
