// whatsapp-bridge is the Go sidecar that speaks the WhatsApp Multi-Device
// protocol via whatsmeow and exposes a newline-delimited JSON protocol over a
// Unix domain socket (named pipe on Windows). The Rust crate `mtw-whatsapp`
// is its client.
//
// The endpoint is configurable via MTW_WHATSAPP_SOCKET (default a Unix
// socket path on Linux/macOS, a named pipe on Windows — see protocol.md).
// Sessions persist in MTW_WHATSAPP_DB (default
// /var/lib/mtw-whatsapp/session.db).
//
// The protocol is specified in protocol.md in this directory.
package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	_ "modernc.org/sqlite" // registers the "sqlite" driver used by whatsmeow

	"go.mau.fi/whatsmeow"
	waProto "go.mau.fi/whatsmeow/binary/proto"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
)

// ── wire types ────────────────────────────────────────────────────────

type inMsg struct {
	Type        string `json:"type"`
	ID          string `json:"id,omitempty"`
	To          string `json:"to,omitempty"`
	Text        string `json:"text,omitempty"`
	Kind        string `json:"kind,omitempty"`
	Mime        string `json:"mime,omitempty"`
	Caption     string `json:"caption,omitempty"`
	Filename    string `json:"filename,omitempty"`
	DataB64     string `json:"data_b64,omitempty"`
	MessageID   string `json:"message_id,omitempty"`
	Emoji       string `json:"emoji,omitempty"`
	ForEveryone bool   `json:"for_everyone,omitempty"`
	Phone       string `json:"phone,omitempty"`
	Limit       int    `json:"limit,omitempty"`
}

type outMsg map[string]any

type attachment struct {
	Kind     string `json:"kind"`
	Mime     string `json:"mime"`
	Filename any    `json:"filename"`
	DataB64  string `json:"data_b64"`
	Caption  any    `json:"caption"`
}

// ── driver (single-connection client of the socket) ──────────────────
//
// The driver may connect AFTER the bridge has already emitted its status or
// a pairing artefact. We cache the last `status` event and the last pairing
// artefact (`qr` / `pairing_code`) so a reconnecting driver gets the current
// state immediately instead of waiting for WhatsApp's next QR rotation (~20s).

type driver struct {
	mu       sync.Mutex
	conn     net.Conn
	lastStat *outMsg
	lastPair *outMsg
}

func (d *driver) set(c net.Conn) {
	d.mu.Lock()
	d.conn = c
	replay := []outMsg{}
	if d.lastStat != nil {
		replay = append(replay, *d.lastStat)
	}
	if d.lastPair != nil {
		replay = append(replay, *d.lastPair)
	}
	d.mu.Unlock()
	// Replay outside the lock so the write path can reacquire it if needed.
	for _, m := range replay {
		d.send(m)
	}
}

func (d *driver) send(m outMsg) {
	d.mu.Lock()
	// Update sticky cache for event types that represent "current state".
	switch m["type"] {
	case "status":
		cp := m
		d.lastStat = &cp
		if m["state"] != "linking" {
			d.lastPair = nil // a pairing artefact only matters while linking
		}
	case "qr", "pairing_code":
		cp := m
		d.lastPair = &cp
	}
	c := d.conn
	d.mu.Unlock()

	if c == nil {
		return
	}
	payload, err := json.Marshal(m)
	if err != nil {
		log.Printf("marshal event: %v", err)
		return
	}
	payload = append(payload, '\n')
	if _, err := c.Write(payload); err != nil {
		log.Printf("driver write (dropping event): %v", err)
	}
}

// ── bridge ───────────────────────────────────────────────────────────

type bridge struct {
	client *whatsmeow.Client // send/media paths
	lk     linker
	sess   *session
	drv    *driver
	emitFn func(outMsg) // overrides b.emit's destination in tests; nil uses b.drv.send
}

// emit sends an event to the driver. Tests set emitFn to a recorder so bridge
// methods are exercisable without a socket; main() leaves it nil so emit
// falls back to the real driver connection.
func (b *bridge) emit(m outMsg) {
	if b.emitFn != nil {
		b.emitFn(m)
		return
	}
	b.drv.send(m)
}

func (b *bridge) emitErr(id, code, msg string) {
	out := outMsg{"type": "error", "message": msg}
	if id != "" {
		out["id"] = id
	}
	if code != "" {
		out["code"] = code
	}
	b.emit(out)
}

// Register the single event handler that translates whatsmeow events into
// newline-JSON events on the socket.
func (b *bridge) wireEvents() {
	b.client.AddEventHandler(func(evt any) {
		switch v := evt.(type) {
		case *events.PairSuccess:
			// No event: *events.Connected follows and reports the new state.
		case *events.Connected:
			b.sess.onConnected()
		case *events.Disconnected:
			b.sess.onDisconnected("disconnected")
		case *events.StreamReplaced:
			b.sess.onDisconnected("stream_replaced")
		case *events.LoggedOut:
			b.sess.onLoggedOut()
		case *events.Message:
			b.forwardMessage(v)
		}
	})
}

func (b *bridge) forwardMessage(e *events.Message) {
	msg := e.Message
	if msg == nil {
		return
	}

	text := ""
	switch {
	case msg.GetConversation() != "":
		text = msg.GetConversation()
	case msg.GetExtendedTextMessage() != nil:
		text = msg.GetExtendedTextMessage().GetText()
	case msg.GetImageMessage() != nil:
		text = msg.GetImageMessage().GetCaption()
	case msg.GetVideoMessage() != nil:
		text = msg.GetVideoMessage().GetCaption()
	case msg.GetDocumentMessage() != nil:
		text = msg.GetDocumentMessage().GetCaption()
	}

	attachments := b.collectAttachments(msg, text)

	var replyTo any
	if ctxInfo := msg.GetExtendedTextMessage().GetContextInfo(); ctxInfo != nil && ctxInfo.GetStanzaID() != "" {
		replyTo = ctxInfo.GetStanzaID()
	}

	var groupName any
	if e.Info.IsGroup {
		if meta, err := b.client.GetGroupInfo(context.Background(), e.Info.Chat); err == nil {
			groupName = meta.Name
		}
	}

	b.emit(messageEvent(e.Info, text, replyTo, groupName, attachments))
}

// messageEvent builds the `message` wire event. Split out from
// forwardMessage so the field mapping — in particular from_me/sender_alt
// (C3) — is unit-testable without a live whatsmeow client.
func messageEvent(info types.MessageInfo, text string, replyTo, groupName any, attachments []attachment) outMsg {
	// sender_alt: the sender's OTHER address — its LID when `author`/`from`
	// is a phone JID, or its phone JID when `author`/`from` is a LID. May be
	// absent (empty) if whatsmeow has no mapping for it. Omitted when zero,
	// never logged (C3).
	var senderAlt any
	if alt := info.SenderAlt.String(); alt != "" {
		senderAlt = alt
	}

	return outMsg{
		"type":        "message",
		"id":          info.ID,
		"from":        info.Sender.String(),
		"chat":        info.Chat.String(),
		"is_group":    info.IsGroup,
		"group_name":  groupName,
		"author":      info.Sender.String(),
		"push_name":   info.PushName,
		"timestamp":   info.Timestamp.Unix(),
		"text":        text,
		"reply_to":    replyTo,
		"from_me":     info.IsFromMe,
		"sender_alt":  senderAlt,
		"attachments": attachments,
	}
}

func (b *bridge) collectAttachments(msg *waProto.Message, caption string) []attachment {
	var out []attachment
	capAny := any(nil)
	if caption != "" {
		capAny = caption
	}

	if m := msg.GetImageMessage(); m != nil {
		if data, err := b.client.Download(context.Background(), m); err == nil {
			out = append(out, attachment{
				Kind: "image", Mime: m.GetMimetype(),
				DataB64: base64.StdEncoding.EncodeToString(data),
				Caption: capAny,
			})
		}
	}
	if m := msg.GetVideoMessage(); m != nil {
		if data, err := b.client.Download(context.Background(), m); err == nil {
			out = append(out, attachment{
				Kind: "video", Mime: m.GetMimetype(),
				DataB64: base64.StdEncoding.EncodeToString(data),
				Caption: capAny,
			})
		}
	}
	if m := msg.GetAudioMessage(); m != nil {
		if data, err := b.client.Download(context.Background(), m); err == nil {
			kind := "audio"
			if m.GetPTT() {
				kind = "voice"
			}
			out = append(out, attachment{
				Kind: kind, Mime: m.GetMimetype(),
				DataB64: base64.StdEncoding.EncodeToString(data),
			})
		}
	}
	if m := msg.GetDocumentMessage(); m != nil {
		if data, err := b.client.Download(context.Background(), m); err == nil {
			out = append(out, attachment{
				Kind: "document", Mime: m.GetMimetype(),
				Filename: m.GetFileName(),
				DataB64:  base64.StdEncoding.EncodeToString(data),
				Caption:  capAny,
			})
		}
	}
	return out
}

// ── outbound handlers ────────────────────────────────────────────────

// startLinkPhone runs session.linkPhone off the socket's read loop (M7):
// it blocks up to the first-QR wait (30s, see qrFirstEventTimeout) plus the
// PairPhone round-trip, and send_text/link_cancel/etc. on the same
// connection must not queue behind it. id/phone are passed by value, so
// this is race-safe regardless of what readLoop's shared `msg` variable
// does on its next iteration — each call to handle (and so to this
// function) already has its own copy.
func (b *bridge) startLinkPhone(id, phone string) {
	go func() {
		if err := b.sess.linkPhone(phone); err != nil {
			b.emitErr(id, codeFor(err), err.Error())
		}
	}()
}

func (b *bridge) handle(msg inMsg) {
	if b.client == nil {
		b.emitErr(msg.ID, "not_ready", "client not initialised")
		return
	}

	switch msg.Type {
	case "link_qr", "request_qr":
		// request_qr is the legacy alias of link_qr.
		if err := b.sess.linkQR(); err != nil {
			b.emitErr(msg.ID, codeFor(err), err.Error())
		}
	case "link_phone":
		b.startLinkPhone(msg.ID, msg.Phone)
	case "link_cancel":
		b.sess.linkCancel()
	case "send_text":
		b.sendText(msg)
	case "send_media":
		b.sendMedia(msg)
	case "react":
		b.sendReact(msg)
	case "delete":
		b.sendDelete(msg)
	case "typing":
		b.sendTyping(msg)
	case "list_chats":
		go b.listChats(msg.ID, msg.Limit)
	case "logout":
		if err := b.lk.Logout(context.Background()); err != nil {
			b.emitErr(msg.ID, "logout_failed", err.Error())
			return
		}
		// whatsmeow does not emit LoggedOut for a self-initiated logout.
		b.sess.onLoggedOut()
		b.emit(outMsg{"type": "ack", "id": msg.ID})
	default:
		b.emitErr(msg.ID, "unknown_command", "type: "+msg.Type)
	}
}

func codeFor(err error) string {
	switch err {
	case errInvalidPhone:
		return "invalid_phone"
	case errAlreadyLinked:
		return "already_linked"
	}
	return "link_failed"
}

func (b *bridge) resolve(to string) (types.JID, error) {
	if to == "" {
		return types.JID{}, errors.New("recipient required")
	}
	if !strings.Contains(to, "@") {
		to = to + "@s.whatsapp.net"
	}
	return types.ParseJID(to)
}

func (b *bridge) sendText(msg inMsg) {
	jid, err := b.resolve(msg.To)
	if err != nil {
		b.emitErr(msg.ID, "unknown_recipient", err.Error())
		return
	}
	m := &waProto.Message{Conversation: proto.String(msg.Text)}
	resp, err := b.client.SendMessage(context.Background(), jid, m)
	if err != nil {
		b.emitErr(msg.ID, "send_failed", err.Error())
		return
	}
	b.emit(outMsg{"type": "ack", "id": msg.ID, "message_id": resp.ID})
}

func (b *bridge) sendMedia(msg inMsg) {
	jid, err := b.resolve(msg.To)
	if err != nil {
		b.emitErr(msg.ID, "unknown_recipient", err.Error())
		return
	}
	data, err := base64.StdEncoding.DecodeString(msg.DataB64)
	if err != nil {
		b.emitErr(msg.ID, "bad_payload", err.Error())
		return
	}

	mediaType := whatsmeow.MediaImage
	switch msg.Kind {
	case "image":
		mediaType = whatsmeow.MediaImage
	case "video":
		mediaType = whatsmeow.MediaVideo
	case "audio", "voice":
		mediaType = whatsmeow.MediaAudio
	case "document":
		mediaType = whatsmeow.MediaDocument
	default:
		b.emitErr(msg.ID, "bad_kind", "unsupported media kind")
		return
	}

	uploaded, err := b.client.Upload(context.Background(), data, mediaType)
	if err != nil {
		b.emitErr(msg.ID, "upload_failed", err.Error())
		return
	}

	mime := msg.Mime
	if mime == "" {
		mime = "application/octet-stream"
	}

	var waMsg *waProto.Message
	switch msg.Kind {
	case "image":
		waMsg = &waProto.Message{ImageMessage: &waProto.ImageMessage{
			Caption:       proto.String(msg.Caption),
			Mimetype:      proto.String(mime),
			URL:           proto.String(uploaded.URL),
			DirectPath:    proto.String(uploaded.DirectPath),
			MediaKey:      uploaded.MediaKey,
			FileEncSHA256: uploaded.FileEncSHA256,
			FileSHA256:    uploaded.FileSHA256,
			FileLength:    proto.Uint64(uint64(len(data))),
		}}
	case "video":
		waMsg = &waProto.Message{VideoMessage: &waProto.VideoMessage{
			Caption:       proto.String(msg.Caption),
			Mimetype:      proto.String(mime),
			URL:           proto.String(uploaded.URL),
			DirectPath:    proto.String(uploaded.DirectPath),
			MediaKey:      uploaded.MediaKey,
			FileEncSHA256: uploaded.FileEncSHA256,
			FileSHA256:    uploaded.FileSHA256,
			FileLength:    proto.Uint64(uint64(len(data))),
		}}
	case "audio", "voice":
		waMsg = &waProto.Message{AudioMessage: &waProto.AudioMessage{
			Mimetype:      proto.String(mime),
			URL:           proto.String(uploaded.URL),
			DirectPath:    proto.String(uploaded.DirectPath),
			MediaKey:      uploaded.MediaKey,
			FileEncSHA256: uploaded.FileEncSHA256,
			FileSHA256:    uploaded.FileSHA256,
			FileLength:    proto.Uint64(uint64(len(data))),
			PTT:           proto.Bool(msg.Kind == "voice"),
		}}
	case "document":
		filename := msg.Filename
		if filename == "" {
			filename = "document"
		}
		waMsg = &waProto.Message{DocumentMessage: &waProto.DocumentMessage{
			Caption:       proto.String(msg.Caption),
			Mimetype:      proto.String(mime),
			FileName:      proto.String(filename),
			URL:           proto.String(uploaded.URL),
			DirectPath:    proto.String(uploaded.DirectPath),
			MediaKey:      uploaded.MediaKey,
			FileEncSHA256: uploaded.FileEncSHA256,
			FileSHA256:    uploaded.FileSHA256,
			FileLength:    proto.Uint64(uint64(len(data))),
		}}
	}

	resp, err := b.client.SendMessage(context.Background(), jid, waMsg)
	if err != nil {
		b.emitErr(msg.ID, "send_failed", err.Error())
		return
	}
	b.emit(outMsg{"type": "ack", "id": msg.ID, "message_id": resp.ID})
}

func (b *bridge) sendReact(msg inMsg) {
	jid, err := b.resolve(msg.To)
	if err != nil {
		b.emitErr(msg.ID, "unknown_recipient", err.Error())
		return
	}
	react := &waProto.Message{ReactionMessage: &waProto.ReactionMessage{
		Key: &waProto.MessageKey{
			RemoteJID: proto.String(jid.String()),
			FromMe:    proto.Bool(false),
			ID:        proto.String(msg.MessageID),
		},
		Text:              proto.String(msg.Emoji),
		SenderTimestampMS: proto.Int64(time.Now().UnixMilli()),
	}}
	if _, err := b.client.SendMessage(context.Background(), jid, react); err != nil {
		b.emitErr(msg.ID, "send_failed", err.Error())
		return
	}
	b.emit(outMsg{"type": "ack", "id": msg.ID})
}

func (b *bridge) sendDelete(msg inMsg) {
	jid, err := b.resolve(msg.To)
	if err != nil {
		b.emitErr(msg.ID, "unknown_recipient", err.Error())
		return
	}
	waMsg := b.client.BuildRevoke(jid, types.EmptyJID, msg.MessageID)
	if _, err := b.client.SendMessage(context.Background(), jid, waMsg); err != nil {
		b.emitErr(msg.ID, "send_failed", err.Error())
		return
	}
	b.emit(outMsg{"type": "ack", "id": msg.ID})
}

func (b *bridge) sendTyping(msg inMsg) {
	jid, err := b.resolve(msg.To)
	if err != nil {
		return
	}
	_ = b.client.SendChatPresence(context.Background(), jid, types.ChatPresenceComposing, types.ChatPresenceMediaText)
}

// ── socket server ────────────────────────────────────────────────────

// serve accepts driver connections; listening is closed once the socket is up.
func serve(socketPath string, drv *driver, handler func(inMsg), listening chan<- struct{}) error {
	l, err := listen(socketPath)
	if err != nil {
		return err
	}
	log.Printf("listening on %s", socketPath)
	close(listening)

	for {
		conn, err := l.Accept()
		if err != nil {
			return err
		}
		drv.set(conn)
		go readLoop(conn, handler)
	}
}

func readLoop(conn net.Conn, handler func(inMsg)) {
	defer conn.Close()
	decoder := json.NewDecoder(conn)
	for {
		var msg inMsg
		if err := decoder.Decode(&msg); err != nil {
			if errors.Is(err, io.EOF) {
				log.Printf("driver disconnected")
				return
			}
			log.Printf("decode: %v", err)
			return
		}
		handler(msg)
	}
}

// ── whatsmeow bootstrap ──────────────────────────────────────────────

func main() {
	socketPath := getenv("MTW_WHATSAPP_SOCKET", defaultSocket)
	dbPath := getenv("MTW_WHATSAPP_DB", "/var/lib/mtw-whatsapp/session.db")

	if err := os.MkdirAll(filepath.Dir(dbPath), 0o755); err != nil {
		log.Fatalf("mkdir db: %v", err)
	}

	storeLog := waLog.Stdout("Store", "INFO", true)
	bootCtx := context.Background()
	// modernc.org/sqlite expects `_pragma=foreign_keys(1)` rather than the
	// mattn/go-sqlite3 `_foreign_keys=on` DSN shorthand.
	container, err := sqlstore.New(
		bootCtx, "sqlite",
		"file:"+dbPath+"?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)",
		storeLog,
	)
	if err != nil {
		log.Fatalf("open store: %v", err)
	}
	device, err := container.GetFirstDevice(bootCtx)
	if err != nil {
		log.Fatalf("get device: %v", err)
	}

	// WARN, not DEBUG: whatsmeow logs QR codes ("Emitting QR code ...") and
	// the PairPhone IQ (which carries the phone number) at Debug level.
	// Neither may ever reach the logs.
	clientLog := waLog.Stdout("Client", "WARN", true)
	client := whatsmeow.NewClient(device, clientLog)
	log.Printf("client initialised; stored session=%v", client.Store.ID != nil)

	drv := &driver{}
	lk := newWaLinker(client)
	b := &bridge{client: client, lk: lk, drv: drv}
	b.emitFn = drv.send
	b.sess = newSession(lk, b.emit)
	b.wireEvents()

	// Start listening in background; boot only once the socket is up. With
	// no stored session the bridge stays idle until the driver asks to link;
	// the sticky cache replays the status to a driver that connects later.
	listenErr := make(chan error, 1)
	listening := make(chan struct{})
	go func() {
		listenErr <- serve(socketPath, drv, b.handle, listening)
	}()

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt, syscall.SIGTERM)

	if os.Getenv("MTW_EXIT_ON_STDIN_EOF") == "1" {
		watchStdinEOF(os.Stdin, stop)
	}

	select {
	case <-listening:
		drv.send(outMsg{"type": "ready"})
		b.sess.boot()
	case err := <-listenErr:
		log.Fatalf("socket server exited: %v", err)
	}

	select {
	case <-stop:
		log.Printf("shutdown requested")
	case err := <-listenErr:
		log.Printf("socket server exited: %v", err)
	}
	shutdown(client, container)
}

// waClient and sessionStore are the slivers of *whatsmeow.Client and
// *sqlstore.Container that shutdown needs — narrow enough to fake in a test
// without a live WhatsApp connection or a real SQLite file.
type waClient interface{ Disconnect() }
type sessionStore interface{ Close() error }

// shutdown runs on every exit path (Ctrl+C, SIGTERM, or stdin EOF — see
// watchStdinEOF): it disconnects the WhatsApp client first, then closes the
// session store so the on-disk SQLite file isn't left open mid-write. The
// close error (if any) is logged without any session data — sqlstore.Close
// never returns anything derived from message content, phone numbers, or
// pairing material.
func shutdown(c waClient, store sessionStore) {
	c.Disconnect()
	if err := store.Close(); err != nil {
		log.Printf("close session store: %v", err)
	}
}

func getenv(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
