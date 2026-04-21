# whatsapp-bridge — socket protocol

Unix domain socket: `/var/run/mtw-whatsapp/whatsapp.sock` (mounted via shared
Docker volume). Framing: **newline-delimited JSON**. One JSON object per line,
`\n`-terminated. Every object has a `"type"` discriminator.

Peers:
- **Driver** (the Rust crate `mtw-whatsapp`) — client of the socket.
- **Bridge** (this Go service, using `whatsmeow`) — server of the socket.

The driver MAY issue commands at any time. The bridge emits events whenever
something happens on the WhatsApp side. The socket is full-duplex: both sides
read and write concurrently. A single driver is expected per socket.

## Driver → Bridge (commands)

### `request_qr`
Force the bridge to restart the login flow. Useful when the session is stale.
```json
{"type":"request_qr"}
```

### `send_text`
```json
{"type":"send_text", "id":"req-1", "to":"5491123456789@s.whatsapp.net", "text":"hello"}
```
- `to`: JID. Plain numbers get auto-suffixed with `@s.whatsapp.net`.
- `id`: echoed back in the `ack` / `error` event so the driver can correlate.

### `send_media`
```json
{
  "type":"send_media",
  "id":"req-2",
  "to":"5491123456789@s.whatsapp.net",
  "kind":"image",
  "mime":"image/jpeg",
  "caption":"optional",
  "filename":"optional (documents only)",
  "data_b64":"<base64 payload>"
}
```
`kind ∈ {"image","video","audio","voice","document"}`.

### `react`
```json
{"type":"react", "id":"req-3", "to":"5491123456789@s.whatsapp.net", "message_id":"3EB0...", "emoji":"👍"}
```
Pass `"emoji": ""` to remove a reaction.

### `delete`
```json
{"type":"delete", "id":"req-4", "to":"...", "message_id":"3EB0...", "for_everyone":true}
```

### `typing`
```json
{"type":"typing", "to":"..."}
```

### `logout`
End the WhatsApp session and delete stored credentials. The next connection
will require a new QR scan.
```json
{"type":"logout"}
```

## Bridge → Driver (events)

### `ready`
Sidecar has booted and opened its database. Emitted once per process.
```json
{"type":"ready"}
```

### `qr`
A QR code string was issued. WhatsApp rotates QRs roughly every 20s until the
user scans or the code expires; the bridge forwards every code it receives.
```json
{"type":"qr", "code":"2@abc123..."}
```

### `pairing_success`
Device was paired (right after the user scans the QR). `connected` follows.
```json
{"type":"pairing_success", "jid":"5491123456789:1@s.whatsapp.net"}
```

### `connected`
Socket connected and authenticated. Outbound sends are safe after this.
```json
{"type":"connected", "jid":"5491123456789:1@s.whatsapp.net"}
```

### `disconnected`
```json
{"type":"disconnected", "reason":"stream_replaced"}
```

### `message`
Inbound message from any chat. Attachments are decoded and base64'd inline.
```json
{
  "type":"message",
  "id":"3EB0ABCDEF...",
  "from":"5491123456789@s.whatsapp.net",
  "chat":"5491123456789@s.whatsapp.net",
  "is_group":false,
  "group_name":null,
  "author":"5491123456789@s.whatsapp.net",
  "push_name":"Nombre",
  "timestamp":1713634800,
  "text":"hola",
  "reply_to":null,
  "attachments":[
    {"kind":"image","mime":"image/jpeg","filename":null,"data_b64":"...","caption":null}
  ]
}
```

### `ack`
Driver command `id` completed successfully.
```json
{"type":"ack", "id":"req-1", "message_id":"3EB0..."}
```

### `error`
Either command-scoped (`id` present) or fatal (no `id`, process keeps running).
```json
{"type":"error", "id":"req-1", "message":"recipient unknown"}
```

## Lifecycle

1. Sidecar boots, emits `ready`.
2. If no stored session, emits `qr` events until the driver's user scans.
3. After scan: `pairing_success` then `connected`.
4. With a stored session, it goes straight to `connected` (no QR).
5. On network drops, `disconnected` is emitted, the sidecar auto-reconnects in
   the background, and `connected` follows when the socket is back.

## Error codes (non-exhaustive)

| code | meaning |
|---|---|
| `not_connected` | Outbound requested before `connected` |
| `unknown_recipient` | The JID couldn't be resolved |
| `media_too_large` | Attachment exceeded WhatsApp's 16 MB limit |
| `rate_limited` | WhatsApp throttled us; retry with backoff |
| `auth_expired` | Session no longer valid; expect a new `qr` event |
