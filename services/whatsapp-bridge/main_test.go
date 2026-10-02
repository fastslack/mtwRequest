package main

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// TestStartLinkPhoneDoesNotBlockCaller: M7. link_phone can block for the
// first-QR wait plus the PairPhone round-trip; startLinkPhone must return
// immediately regardless, so send_text/link_cancel on the same connection
// never queue behind it.
func TestStartLinkPhoneDoesNotBlockCaller(t *testing.T) {
	f, r := newFake(), &recorder{}
	// No injected QR item: linkPhone's first select blocks on
	// time.After(10s)/ctx.Done() until this goroutine's session is torn
	// down, which only happens via linkCancel below — so a blocking
	// startLinkPhone would make this test hang and time out.
	b := &bridge{lk: f, sess: newSession(f, r.emit)}
	b.emitFn = r.emit
	b.sess.boot()

	done := make(chan struct{})
	go func() {
		b.startLinkPhone("req-1", "5491123456789")
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("startLinkPhone must not block the caller")
	}

	r.waitFor(t, "status", "mode", "phone") // the attempt did start
	b.sess.linkCancel()                     // let the leaked goroutine finish
}

// TestStartLinkPhoneErrorStillReachesSocket: M7. linkPhone's error must
// still reach the socket via the existing emit path even though it now
// runs off the read loop.
func TestStartLinkPhoneErrorStillReachesSocket(t *testing.T) {
	f, r := newFake(), &recorder{}
	b := &bridge{lk: f, sess: newSession(f, r.emit)}
	b.emitFn = r.emit
	b.sess.boot()

	b.startLinkPhone("req-2", "abc") // invalid: fails synchronously inside linkPhone, before any I/O
	m := r.waitFor(t, "error", "id", "req-2")
	if m["code"] != "invalid_phone" {
		t.Fatalf("code=%v", m["code"])
	}
}

// TestMessageEventCarriesFromMeAndSenderAlt covers C3: the kernel needs
// from_me to drop its own outgoing traffic instead of replying to it, and
// sender_alt to still recognise the self-chat when WhatsApp addresses it by
// LID instead of phone JID.
func TestMessageEventCarriesFromMeAndSenderAlt(t *testing.T) {
	own := types.NewJID("5491123456789", types.DefaultUserServer)
	alt := types.NewJID("123456789", types.HiddenUserServer)

	info := types.MessageInfo{
		MessageSource: types.MessageSource{
			Chat:      own,
			Sender:    own,
			IsFromMe:  true,
			SenderAlt: alt,
		},
		ID:        "wamid-1",
		Timestamp: time.Unix(1700000000, 0),
	}

	m := messageEvent(info, "hola", nil, nil, nil)

	if m["from_me"] != true {
		t.Fatalf("from_me=%v, want true", m["from_me"])
	}
	if m["sender_alt"] != alt.String() {
		t.Fatalf("sender_alt=%v, want %q", m["sender_alt"], alt.String())
	}
}

// TestMessageEventOmitsSenderAltWhenZero: most messages are addressed by
// phone JID, not LID — sender_alt must come back empty/absent, not a
// stringified zero JID.
func TestMessageEventOmitsSenderAltWhenZero(t *testing.T) {
	other := types.NewJID("5491199999999", types.DefaultUserServer)
	info := types.MessageInfo{
		MessageSource: types.MessageSource{
			Chat:     other,
			Sender:   other,
			IsFromMe: false,
		},
		ID:        "wamid-2",
		Timestamp: time.Unix(1700000001, 0),
	}

	m := messageEvent(info, "hola", nil, nil, nil)

	if m["from_me"] != false {
		t.Fatalf("from_me=%v, want false", m["from_me"])
	}
	if m["sender_alt"] != nil {
		t.Fatalf("sender_alt=%v, want nil (omitted)", m["sender_alt"])
	}
}
