# whatsapp-bridge — socket protocol

Transport: a Unix domain socket on Linux/macOS, a named pipe on Windows
(there is no Unix socket there). Framing: **newline-delimited JSON**. One
JSON object per line, `\n`-terminated. Every object has a `"type"`
discriminator.

Peers:
- **Driver** (the Rust crate `mtw-whatsapp`) — client of the endpoint.
- **Bridge** (this Go service, using `whatsmeow`) — server of the endpoint.

The driver MAY issue commands at any time. The bridge emits events whenever
something happens on the WhatsApp side. The connection is full-duplex: both
sides read and write concurrently. A single driver is expected per endpoint.

## Transport endpoint

`MTW_WHATSAPP_SOCKET` selects the endpoint:
- On Linux/macOS: a filesystem path to a Unix socket. Default
  `/var/run/mtw-whatsapp/whatsapp.sock`. A stale file at that path is
  removed before listening; the socket is created with mode `0666`.
- On Windows: a named pipe path, which **must** start with `\\.\pipe\`.
  Default `\\.\pipe\mtw-whatsapp`. The pipe's security descriptor grants
  full access to its owner and to SYSTEM only.

## Clean shutdown on stdin EOF

When the environment variable `MTW_EXIT_ON_STDIN_EOF=1` is set, the bridge
also treats EOF on its stdin as a shutdown request, in addition to
`SIGINT`/`SIGTERM`. The process supervising the bridge closes its stdin to
stop it — this matters on Windows, which has no `SIGTERM`, and avoids a hard
kill that could leave the session database half-written. The variable is
unset (feature off) by default.

## Driver → Bridge (commands)

### `link_qr`
Start (or restart) the login flow using a scanned QR code. Any linking
attempt already in progress (QR or phone) is aborted first.
```json
{"type":"link_qr"}
```
`request_qr` is accepted as a legacy alias of `link_qr`.

### `link_phone`
Start (or restart) the login flow using whatsmeow's phone-pairing code
instead of a QR scan. Any linking attempt already in progress is aborted
first.
```json
{"type":"link_phone", "phone":"5491123456789"}
```
- `phone`: digits only, no `+` or separators, 8–15 digits, not leading `0`.
  An invalid number never opens a connection; see `invalid_phone` below.

### `link_cancel`
Abort the linking attempt in progress (QR or phone) and go back to `idle`.
A no-op if nothing is linking.
```json
{"type":"link_cancel"}
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
will require a new QR scan or phone pairing.
```json
{"type":"logout"}
```

### `list_chats`
List contacts and joined groups — e.g. so the driver can let the user pick
who may talk to their office. Requires `status` `connected`; otherwise
rejected with `error` `code: "not_connected"`.
```json
{"type":"list_chats", "id":"req-5", "limit":50}
```
- `limit`: optional. `<= 0` or `> 200` falls back to `50`. Replies with the
  `chats` event.

## Bridge → Driver (events)

### `ready`
Sidecar has booted and opened its database. Emitted once per process.
```json
{"type":"ready"}
```

### `status`
The session's state machine. This is the single source of truth for session
state — `pairing_success`, `connected` and `disconnected` as standalone event
types are **not emitted**; everything they used to convey is now reported
through `status`.

```json
{"type":"status", "state":"idle"}
{"type":"status", "state":"linking", "mode":"qr"}
{"type":"status", "state":"linking", "mode":"phone"}
{"type":"status", "state":"connected", "jid":"5491123456789:1@s.whatsapp.net", "lid":"123456789@lid"}
{"type":"status", "state":"disconnected", "reason":"stream_replaced"}
{"type":"status", "state":"logged_out"}
```

| `state` | meaning | extra fields |
|---|---|---|
| `idle` | no stored session, nothing linking | `reason` (see below), only on a transition out of `linking` |
| `linking` | a `link_qr`/`link_phone` attempt is in progress | `mode` ∈ `"qr"` \| `"phone"` |
| `connected` | authenticated; outbound sends are safe | `jid`, `lid` (own LID; omitted if none is stored yet) |
| `disconnected` | socket dropped unexpectedly after being `connected` | `reason` |
| `logged_out` | credentials were revoked (remote logout, or a successful `logout` command) | — |

`lid` is the user's own LID (`<lid>@lid`). The self-chat ("Mensajes a mí
mismo") can arrive addressed by LID instead of phone JID (see `message`'s
`sender_alt` below); the driver needs `lid` to recognise it.

`disconnected` also covers a failed `Connect()` on boot with a stored
session (`reason: "connect_failed"`) — the one case where `disconnected` can
happen without ever having been `connected`. In every other case, whatsmeow
itself auto-reconnects in the background (`EnableAutoReconnect` defaults to
`true`); the bridge does not drive that retry, it only reports the
`disconnected`/eventual-reconnect `status` transitions whatsmeow's own events
produce. A `disconnected` session does **not** automatically fall back to
`idle` or restart linking — the driver decides whether/when to send
`link_qr`/`link_phone` again.

`idle`'s `reason`, when present, explains why a linking attempt ended without
reaching `connected`:

| `reason` | meaning |
|---|---|
| `cancelled` | the driver sent `link_cancel` |
| `expired` | WhatsApp's QR/phone code timed out before it was used |
| `outdated` | this whatsmeow/client version is no longer accepted by WhatsApp |
| `error` | the pairing attempt failed for another reason |

For a command-scoped failure (`link_phone`'s `PairPhone` call failing, for
example), the `status` `idle`/`reason:"error"` event is emitted **before**
the command's `error` event — the state transition happens first, the
command's own rejection is reported after, once the handler returns.

### `qr`
A QR code string was issued. WhatsApp rotates QRs roughly every 20s until the
user scans or the code expires; the bridge forwards every code it receives.
Only emitted during a `link_qr` attempt (never during `link_phone`).
```json
{"type":"qr", "code":"2@abc123..."}
```

### `pairing_code`
The 8-character code to enter on the phone, emitted once per `link_phone`
attempt. `expires_at` is a Unix timestamp (seconds); the code is no longer
valid after it, and the session falls back to `status` `idle` with
`reason: "expired"`.
```json
{"type":"pairing_code", "code":"K3M9QX2P", "expires_at":1713634960}
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
  "from_me":false,
  "sender_alt":null,
  "attachments":[
    {"kind":"image","mime":"image/jpeg","filename":null,"data_b64":"...","caption":null}
  ]
}
```
- `from_me`: true when the user's own device sent this message, to anyone
  (not just themselves) — every chat the user types into on their phone is
  forwarded here, `IsFromMe` included. The driver needs this to avoid
  treating its own outgoing traffic as inbound to auto-reply to.
- `sender_alt`: the sender's **other** address — its LID when `author`/`from`
  is a phone JID, or its phone JID when `author`/`from` is a `@lid` address.
  May be absent/empty if whatsmeow has no mapping for it. In particular, the
  self-chat can be addressed by LID, in which case `author` is
  `<own-lid>@lid` and `sender_alt` carries the phone JID back — compare
  `author`/`sender_alt` against `status`'s `jid` and `lid` (either can match)
  to recognise it.

### `chats`
Reply to `list_chats`: every contact plus every joined group, capped at
`limit` (sorted by name; groups are kept, not filtered out).
```json
{
  "type":"chats",
  "id":"req-5",
  "items":[
    {"jid":"5491123456789@s.whatsapp.net", "name":"Ana", "is_group":false, "last_ts":0},
    {"jid":"120363...@g.us", "name":"Familia", "is_group":true, "last_ts":0}
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
{"type":"error", "id":"req-1", "code":"unknown_recipient", "message":"recipient unknown"}
```

## Lifecycle

1. Sidecar boots, emits `ready`, then a `status` reflecting any stored
   session: `idle` (none) or straight to `connected` (valid stored session;
   no linking step).
2. The driver sends `link_qr` or `link_phone`. The bridge replies with
   `status` `linking` (`mode` set), then streams `qr` events (QR) or a single
   `pairing_code` event (phone).
3. After the user scans/enters the code: `status` `connected` (no
   intermediate "paired" event — the old `pairing_success` type is gone).
4. The driver may send `link_cancel` at any point while `linking`: `status`
   goes to `idle` with `reason: "cancelled"`.
5. A `link_qr`/`link_phone` that times out or is rejected by WhatsApp also
   lands on `status` `idle`, with `reason` ∈ `expired` | `outdated` | `error`.
6. Starting a new `link_qr`/`link_phone` while one is already in progress
   aborts the previous attempt first (no error — it simply replaces it).
7. With a stored session, boot goes straight to `connected` (no QR/pairing).
   If that initial `Connect()` itself fails, boot instead reports `status`
   `disconnected` with `reason: "connect_failed"`.
8. `link_qr`/`link_phone` while a session is already stored (but not yet
   `connected` — e.g. after a `connect_failed` boot) is rejected with
   `error` `code: "link_failed"`: re-pairing isn't the right recovery for a
   transient connect failure. `link_qr`/`link_phone` is only rejected with
   `already_linked` once `status` has actually reached `connected`.
9. Cancelling the readiness wait inside `link_phone` (via `link_cancel`,
   between sending the command and whatsmeow's login websocket coming up)
   surfaces as `error` `code: "link_failed"`, `message: "context canceled"` —
   the `status` `idle`/`reason:"cancelled"` event from the `link_cancel` is
   what actually matters; this `error` is just that command's own return value.
10. On network drops after `connected`, `status` `disconnected` is emitted;
    whatsmeow auto-reconnects on its own (see the `disconnected` state note
    above) and a subsequent `status` `connected` follows if it succeeds. The
    bridge does not itself restart linking on a drop.
11. A reconnecting driver that missed events gets the last `status` and, if
    still `linking`, the last `qr`/`pairing_code` replayed immediately
    instead of waiting for WhatsApp's next rotation.

## Error codes (non-exhaustive)

| code | meaning |
|---|---|
| `invalid_phone` | `link_phone`'s `phone` failed validation; no connection was attempted |
| `already_linked` | `link_qr`/`link_phone` requested while `status` is already `connected` |
| `link_failed` | `link_qr`/`link_phone` failed for another reason: a session is already stored (re-pairing isn't the recovery — see lifecycle note 8), the attempt was cancelled mid-setup (`message: "context canceled"`), or `Connect`/`PairPhone` itself failed |
| `not_connected` | Outbound requested before `connected` |
| `unknown_recipient` | The JID couldn't be resolved |
| `media_too_large` | Attachment exceeded WhatsApp's 16 MB limit |
| `rate_limited` | WhatsApp throttled us; retry with backoff |
| `auth_expired` | Session no longer valid; expect a new `link_qr`/`link_phone` round |
