# Security & Privacy

This doc is **honest**. There is no toggle that gives you "privacy 100%".
Different threats need different mitigations, and many of the things people
mean when they say "encrypt everything" cannot be achieved by *encrypting*
alone — they require redesigning who sees what data.

## Threat model breakdown

Read this before configuring. The right setup depends on which of these
adversaries you actually care about.

| Adversary | What they can see today | What it takes to defeat |
|---|---|---|
| **ISP / passive AS** (sees your traffic in transit) | TLS already hides content. They see destination IPs, packet sizes, timing. | VPN or Tor for **all egress** + DoH for DNS. |
| **Local network attacker** (hostile WiFi, captive portal) | Same as ISP, plus may be able to inject TLS errors / try MITM. | Same: VPN/Tor + don't ignore TLS warnings. |
| **The remote service you talk to** (Anthropic, OpenAI, Bitvavo, archive.org, every BitTorrent peer) | **Plaintext content of what you send.** Encryption to them does not help — they decrypt to process. | Switch to a service you trust, or self-host (e.g. Ollama instead of Anthropic). For BT: I2P swarms instead of clearnet. |
| **Hosting provider / cloud admin** (where your kernel runs) | Disk content, RAM, all decrypted in-process state. | Self-host on hardware you control; encrypt at rest; never put privacy-sensitive data through a cloud you don't trust. |
| **Government with subpoena power** | Anything any of the above adversaries see, plus future logs. | Combination of all of the above + jurisdictional choices. No software-only fix. |
| **State actor (active, well-resourced)** | Likely beats most defences except Tor (sometimes) and air-gapped systems. | Out of scope for this software. |

## What v0.4.0 ships

### Single-flag egress profile

Every outbound `reqwest::Client` in the workspace (mtw-ai cloud
providers, mtw-exchange, mtw-integrations OAuth2 + cloud AI,
mtw-registry, mtw-federation HTTP fallback, mtw-http, mtw-torrent
webseeds) goes through a process-wide `mtw-net::NetFactory` installed
at boot from the `[net]` block of `mtw.toml`.

```toml
[net]
default_profile = "tor-trackers"
https_only = true
min_tls_version = "1.3"

[net.profiles.tor-trackers]
type = "proxy"
url = "socks5h://127.0.0.1:9050"
no_proxy = ["127.0.0.1", "localhost", "::1"]
```

Profiles in this release:

- `direct` — straight egress (default).
- `proxy` — HTTP / HTTPS / SOCKS5 / SOCKS5h. Use this with a local
  Tor or arti-proxy daemon for HTTP traffic.
- `wireguard` — declared in the schema, **not implemented yet**
  (boringtun integration). Will return a config error if selected.
- `tor` — declared, **not implemented yet** (arti embedded).

### Local providers bypass the profile on purpose

`mtw_ai::providers::ollama` and `mtw_ai::providers::lmstudio` (and
their counterparts in `mtw-integrations`) use a vanilla
`reqwest::Client::builder()` and **do not** route through
`mtw-net::default_client_builder()`. Reasoning:

- They default to `http://localhost:11434` / `http://localhost:1234`.
- Sending localhost requests through a SOCKS5 proxy breaks them.
- There's zero privacy upside — the request never leaves the host.

If you point Ollama/LMStudio at a *remote* host (e.g.
`http://gpu-server:11434`), be aware those requests still bypass the
profile. Either patch the provider or set `no_proxy` on the egress
profile.

### Bridge socket auto-permissioning

The Unix-socket bridge (`/tmp/mtw-rust.sock` by default) is
`chmod 0666` after bind so non-root clients (e.g. mtwKernel container
running as uid 1000) can connect without a manual chmod each restart.
Override with `MTW_BRIDGE_SOCKET_MODE=0660` in production deployments
where both ends share a gid.

### iroh federation E2E

`mtw-federation` peer-to-peer sync over QUIC + Ed25519 is end-to-end
encrypted between peers, with the same key serving as both transport
identity (NodeId) and message-signing identity (`mtw_identity`).
Peers see each other's IPs (NAT holepunching is on by default — set
relay-only in your iroh config to use only relay servers).

## What v0.4.0 explicitly does **not** ship

- **WireGuard userspace** — the `[net.profiles.<name>]` schema accepts
  `type = "wireguard"`, but trying to use it returns a config error.
  Realistic implementation needs `boringtun` + a TUN device or
  `smoltcp` userspace TCP/IP. Roadmap.
- **Native arti Tor** — same. The `proxy` profile pointing at
  `socks5h://127.0.0.1:9050` to a system-installed `tor` daemon is the
  current way to use Tor.
- **At-rest encryption** for SQLite (`mtw-store`). Roadmap (SQLCipher
  via `rusqlite/bundled-sqlcipher`).
- **End-to-end encrypted "shared" content** (forum posts, comms with
  another user where the server cannot read the body). Roadmap
  (`mtw-share` with X25519 + ChaCha20-Poly1305 to recipient pubkeys).
- **BitTorrent peer-IP hiding** in clearnet swarms — *physically
  impossible*. Every peer in a swarm sees every other peer's IP, that's
  how the protocol works. The mitigations are: (a) run inside a VPN so
  peers see the VPN's IP, (b) participate in I2P swarms only.

## Recommended setups, ranked by realism

### Tier 1 — "ISP can't see what I do, but Anthropic still can"

Easiest. Covers passive-network adversary, doesn't change services.

1. Install a system VPN (WireGuard via `wg-quick`, Mullvad/IVPN/etc.).
2. Run mtwRequest inside that namespace (`ip netns exec wg-vpn …`) or
   on a host configured to send everything through it.
3. Leave `[net]` empty — direct egress is fine when the OS already
   tunnels everything.

ISP sees only encrypted traffic to your VPN endpoint. Anthropic still
sees your prompts. archive.org still sees your downloads.

### Tier 2 — "Tier 1 + announce traffic anonymised"

Adds Tor for **HTTP traffic only** (BT trackers via HTTP, RSS
fetches, certain integrations). Peer wires and DHT still go through
the VPN.

1. Run a system `tor` daemon (port 9050).
2. `[net]` config:
   ```toml
   [net]
   default_profile = "tor-http"

   [net.profiles.tor-http]
   type = "proxy"
   url = "socks5h://127.0.0.1:9050"
   no_proxy = ["127.0.0.1", "localhost", "::1", "192.168.0.0/16"]
   ```
3. Use `Ollama` instead of cloud LLMs.

Caveats:
- Tor over Anthropic *still* lets Anthropic see prompts. They just
  don't see your IP — they see a Tor exit IP.
- Trade order placement via Bitvavo: same — exchange sees orders.
- BT peer connections **do not** go through Tor. They go through
  whatever the OS routes (the VPN if you have Tier 1).

### Tier 3 — "I'm running a private node"

Self-hosted, single user, hostile network.

1. Hardware you control (no VPS).
2. `[net]` set as Tier 2.
3. Ollama only — no cloud LLM keys configured.
4. SQLCipher for `mtw-store` (when v0.5+).
5. Tor onion service for the kernel UI (operational, not in code).
6. Federation peers only over iroh (QUIC E2E).

### Tier 4 — "Maximum paranoia"

Out of scope for this software:
- I2P-only BitTorrent.
- Air-gapped key material (offline signing of federation messages).
- Verified reproducible builds, hardware attestation.
- Cover traffic against traffic-analysis.

These are research projects, not toggles.

## Protocol-specific notes

### BitTorrent and Tor: **don't**

It's tempting to set `default_profile = "tor-socks"` and call it
done, but BT over Tor is well-documented as harmful:

- DHT (UDP) doesn't work over Tor at all.
- uTP (UDP) doesn't work over Tor.
- TCP peer wires *do* work but are slow and saturate exit relays.
- Tor exit operators report BT abuse complaints constantly — the Tor
  Project asks users not to do this.

Solutions ranked:
1. **VPN** for the whole engine (peers + announces + DHT). Realistic
   today via system-level WireGuard. v0.5+ will ship userspace WG.
2. **I2P swarms** (separate content space, completely anonymised).
   Roadmap.
3. **Tor for trackers only**, peers via VPN — ad-hoc but possible.
   Configure with two `[net.profiles.*]` entries and select per-call
   site. Not exposed yet.

### What `https_only = true` does and doesn't

- ✅ Rejects `http://` URLs at request time.
- ✅ Forces TLS validation everywhere.
- ❌ Doesn't validate certificate pinning. A nation-state with CA
  collusion can MITM you. Cert pinning is on the roadmap for known
  AI/exchange providers.

### What about the bridge socket?

The Unix socket between mtwRequest (Rust) and mtwKernel (TS) is a
local IPC channel. **It is not on the network.** Anyone with shell
access to the host has equivalent power; encrypting the socket would
add zero privacy and a real bug surface. We don't.

If you ever expose the bridge as TCP cross-machine, that's a
different threat model — TCP bridge needs auth + Noise/TLS. Not in
v0.4.0.

## Where to file feedback

Concrete privacy bugs (e.g. "I configured `tor-socks` and call X is
still going direct"): open an issue with the call-site path. Vague
threat-model questions: read this doc again, then ask.

Roadmap items (WireGuard userspace, arti embedded, SQLCipher at-rest,
mtw-share E2E) are tracked in `ROADMAP.md`.
