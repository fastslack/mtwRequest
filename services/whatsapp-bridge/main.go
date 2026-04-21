// whatsapp-bridge is the Go sidecar that speaks the WhatsApp Multi-Device
// protocol via whatsmeow and exposes a newline-delimited JSON protocol over a
// Unix domain socket. The Rust crate `mtw-whatsapp` is its client.
//
// Socket path is configurable via MTW_WHATSAPP_SOCKET (default
// /var/run/mtw-whatsapp/whatsapp.sock). Sessions persist in MTW_WHATSAPP_DB
// (default /var/lib/mtw-whatsapp/session.db).
//
// The protocol is specified in protocol.md in this directory.
package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
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
	Type       string `json:"type"`
	ID         string `json:"id,omitempty"`
	To         string `json:"to,omitempty"`
	Text       string `json:"text,omitempty"`
	Kind       string `json:"kind,omitempty"`
	Mime       string `json:"mime,omitempty"`
	Caption    string `json:"caption,omitempty"`
	Filename   string `json:"filename,omitempty"`
	DataB64    string `json:"data_b64,omitempty"`
	MessageID  string `json:"message_id,omitempty"`
	Emoji      string `json:"emoji,omitempty"`
	ForEveryone bool  `json:"for_everyone,omitempty"`
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
// The driver may connect AFTER whatsmeow has already emitted ready / qr /
// connected events. We cache the last sticky event of each kind so a
// reconnecting driver gets the current state immediately instead of
// waiting for WhatsApp's next QR rotation (~20s).

type driver struct {
	mu       sync.Mutex
	conn     net.Conn
	lastQr   *outMsg
	lastStat *outMsg
}

func (d *driver) set(c net.Conn) {
	d.mu.Lock()
	d.conn = c
	replay := []outMsg{}
	if d.lastQr != nil {
		replay = append(replay, *d.lastQr)
	}
	if d.lastStat != nil {
		replay = append(replay, *d.lastStat)
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
	case "qr":
		cp := m; d.lastQr = &cp
	case "ready", "connected", "disconnected", "pairing_success":
		cp := m; d.lastStat = &cp
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
	client *whatsmeow.Client
	drv    *driver
}

func (b *bridge) emit(m outMsg) { b.drv.send(m) }

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
			b.emit(outMsg{"type": "pairing_success", "jid": v.ID.String()})
		case *events.Connected:
			jid := ""
			if b.client.Store != nil && b.client.Store.ID != nil {
				jid = b.client.Store.ID.String()
			}
			b.emit(outMsg{"type": "connected", "jid": jid})
		case *events.Disconnected:
			b.emit(outMsg{"type": "disconnected", "reason": "disconnected"})
		case *events.StreamReplaced:
			b.emit(outMsg{"type": "disconnected", "reason": "stream_replaced"})
		case *events.LoggedOut:
			b.emit(outMsg{"type": "disconnected", "reason": "logged_out"})
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

	b.emit(outMsg{
		"type":        "message",
		"id":          e.Info.ID,
		"from":        e.Info.Sender.String(),
		"chat":        e.Info.Chat.String(),
		"is_group":    e.Info.IsGroup,
		"group_name":  groupName,
		"author":      e.Info.Sender.String(),
		"push_name":   e.Info.PushName,
		"timestamp":   e.Info.Timestamp.Unix(),
		"text":        text,
		"reply_to":    replyTo,
		"attachments": attachments,
	})
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

func (b *bridge) handle(msg inMsg) {
	if b.client == nil {
		b.emitErr(msg.ID, "not_ready", "client not initialised")
		return
	}

	switch msg.Type {
	case "request_qr":
		// Disconnecting triggers a new login flow on next Connect().
		b.client.Disconnect()
		go func() {
			time.Sleep(500 * time.Millisecond)
			if err := connectWithQR(b.client, b.drv); err != nil {
				b.emitErr("", "connect_failed", err.Error())
			}
		}()
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
	case "logout":
		if err := b.client.Logout(context.Background()); err != nil {
			b.emitErr(msg.ID, "logout_failed", err.Error())
			return
		}
		b.emit(outMsg{"type": "ack", "id": msg.ID})
	default:
		b.emitErr(msg.ID, "unknown_command", "type: "+msg.Type)
	}
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
	case "image":    mediaType = whatsmeow.MediaImage
	case "video":    mediaType = whatsmeow.MediaVideo
	case "audio", "voice":
		mediaType = whatsmeow.MediaAudio
	case "document": mediaType = whatsmeow.MediaDocument
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

func serve(socketPath string, drv *driver, handler func(inMsg)) error {
	if err := os.MkdirAll(filepath.Dir(socketPath), 0o755); err != nil {
		return fmt.Errorf("mkdir socket dir: %w", err)
	}
	_ = os.Remove(socketPath)

	l, err := net.Listen("unix", socketPath)
	if err != nil {
		return fmt.Errorf("listen: %w", err)
	}
	if err := os.Chmod(socketPath, 0o666); err != nil {
		log.Printf("chmod socket: %v", err)
	}
	log.Printf("listening on %s", socketPath)

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

func connectWithQR(client *whatsmeow.Client, drv *driver) error {
	if client.Store.ID != nil {
		log.Printf("connectWithQR: existing session (%s), skipping QR", client.Store.ID.String())
		return client.Connect()
	}
	log.Printf("connectWithQR: no session, requesting QR channel")
	qrCh, err := client.GetQRChannel(context.Background())
	if err != nil {
		return fmt.Errorf("qr channel: %w", err)
	}
	if err := client.Connect(); err != nil {
		return fmt.Errorf("connect: %w", err)
	}
	log.Printf("connectWithQR: client.Connect() returned; awaiting QR events")
	go func() {
		for evt := range qrCh {
			log.Printf("qr event: %q code=%q err=%v", evt.Event, truncate(evt.Code, 12), evt.Error)
			switch evt.Event {
			case "code":
				drv.send(outMsg{"type": "qr", "code": evt.Code})
			case "success":
				// PairSuccess is already emitted via the main event handler.
			case "timeout":
				drv.send(outMsg{"type": "error", "code": "qr_timeout", "message": "QR expired before scan"})
			case "err-client-outdated":
				drv.send(outMsg{"type": "error", "code": "client_outdated", "message": evt.Error.Error()})
			}
		}
		log.Printf("qr channel closed")
	}()
	return nil
}

func main() {
	socketPath := getenv("MTW_WHATSAPP_SOCKET", "/var/run/mtw-whatsapp/whatsapp.sock")
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

	clientLog := waLog.Stdout("Client", "DEBUG", true)
	client := whatsmeow.NewClient(device, clientLog)
	log.Printf("client initialised; Store.ID=%v", client.Store.ID)

	drv := &driver{}
	b := &bridge{client: client, drv: drv}
	b.wireEvents()

	// Start listening in background; emit `ready` only after socket is up.
	listenErr := make(chan error, 1)
	go func() {
		drv.send(outMsg{"type": "ready"})
		listenErr <- serve(socketPath, drv, b.handle)
	}()

	// Connect: if we already have a session, whatsmeow skips the QR stage.
	if err := connectWithQR(client, drv); err != nil {
		log.Printf("initial connect: %v", err)
	}

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt, syscall.SIGTERM)
	select {
	case <-stop:
		log.Printf("shutdown requested")
	case err := <-listenErr:
		log.Printf("socket server exited: %v", err)
	}
	client.Disconnect()
}

func truncate(s string, n int) string {
	if len(s) <= n { return s }
	return s[:n] + "…"
}

func getenv(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
