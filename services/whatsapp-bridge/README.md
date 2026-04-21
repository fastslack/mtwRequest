# whatsapp-bridge

Go sidecar that links a real WhatsApp account to mtwRequest via `whatsmeow`.
The Rust crate [`mtw-whatsapp`](../../crates/mtw-whatsapp) is its only client.
Spoken on a Unix domain socket; protocol: newline-delimited JSON, documented
in [`protocol.md`](./protocol.md).

## Why this exists

WhatsApp's Multi-Device protocol has no public spec. The two mature,
community-maintained implementations are `whatsmeow` (Go) and `Baileys`
(Node). We isolate the protocol concerns here so the rest of mtwRequest
stays in pure Rust.

## Pairing flow

1. First run, no session on disk → sidecar emits `qr` events.
2. User scans the code (displayed by the dashboard or any `mtw-whatsapp` consumer).
3. `pairing_success` → `connected`. Session is cached in `MTW_WHATSAPP_DB`.
4. Subsequent runs skip the QR and jump straight to `connected`.

## Environment

| Variable | Default | Purpose |
|---|---|---|
| `MTW_WHATSAPP_SOCKET` | `/var/run/mtw-whatsapp/whatsapp.sock` | Unix socket path |
| `MTW_WHATSAPP_DB`     | `/var/lib/mtw-whatsapp/session.db`   | whatsmeow session store |

## Build + run locally

```sh
# From services/whatsapp-bridge/
go mod tidy
go run .
```

Or as part of the full stack:

```sh
# From repo root
docker compose up whatsapp-bridge
```

## Resetting the session

Stop the sidecar, wipe the session file, start again — the next boot will
emit a fresh `qr` event.

```sh
docker compose stop whatsapp-bridge
docker volume rm mtwrequest_mtw-whatsapp-data
docker compose up whatsapp-bridge
```
