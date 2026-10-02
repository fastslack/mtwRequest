package main

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"
)

type fakeLinker struct {
	mu           sync.Mutex
	session      bool
	connects     int
	disconns     int
	qr           chan qrItem // tests inject simulated server events here
	curOut       chan qrItem // channel returned by the most recent QRChannel call
	qrChanErr    error
	qrChanClosed bool // next QRChannel call returns an already-closed channel
	connectErr   error
	pairCode     string
	pairErr      error
	pairPhones   []string
	pairHook     func() // if set, called synchronously inside PairPhone before it returns
	cancelFn     func() // test-only hook pairHook can call (e.g. s.linkCancel)
	onQRChannel  func() // if set, called once (then cleared) right after the next QRChannel call sets curOut
	own          string
	ownLID       string
	chats        []chatInfo
}

func newFake() *fakeLinker {
	f := &fakeLinker{qr: make(chan qrItem, 8), own: "5491123456789:12@s.whatsapp.net"}
	go f.forward()
	return f
}

// forward relays every item a test sends on f.qr to whichever channel the
// most recent QRChannel call returned. Each attempt gets its own fresh
// channel (see QRChannel below), so two attempts never compete to receive
// off the same one — that competition, not anything about session.go, was
// what made some tests occasionally take the full 10s fallback.
func (f *fakeLinker) forward() {
	for it := range f.qr {
		f.mu.Lock()
		out := f.curOut
		f.mu.Unlock()
		if out != nil {
			out <- it
		}
	}
}

func (f *fakeLinker) HasSession() bool { f.mu.Lock(); defer f.mu.Unlock(); return f.session }
func (f *fakeLinker) Connect() error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.connectErr != nil {
		return f.connectErr
	}
	f.connects++
	return nil
}
func (f *fakeLinker) Disconnect() { f.mu.Lock(); f.disconns++; f.mu.Unlock() }
func (f *fakeLinker) QRChannel(ctx context.Context) (<-chan qrItem, error) {
	f.mu.Lock()
	if f.qrChanErr != nil {
		err := f.qrChanErr
		f.mu.Unlock()
		return nil, err
	}
	out := make(chan qrItem, 8)
	if f.qrChanClosed {
		f.mu.Unlock()
		close(out)
		return out, nil
	}
	f.curOut = out
	hook := f.onQRChannel
	f.onQRChannel = nil
	f.mu.Unlock()
	if hook != nil {
		hook()
	}
	return out, nil
}

// armQRChannel registers a hook that fires the next time QRChannel sets
// curOut, and returns a sender that waits for that hook before putting item
// on f.qr — removing the race (session_test.go's
// TestNewLinkCancelsPreviousAndCancelGoesIdle used to hit occasionally)
// between a test's injected item and the fake's own curOut bookkeeping.
//
// Call armQRChannel synchronously, in the test's main goroutine, BEFORE the
// call expected to trigger that QRChannel (e.g. s.linkPhone) — arming it
// inside the same goroutine that will later do the sending would just move
// the race rather than remove it, since that goroutine's own registration
// could still lose the race against QRChannel being called first. Only the
// returned sender (the wait-then-send part) goes in a goroutine:
//
//	send := f.armQRChannel()
//	go send(qrItem{...})
//	s.linkPhone(...)
func (f *fakeLinker) armQRChannel() func(item qrItem) {
	ready := make(chan struct{})
	f.mu.Lock()
	f.onQRChannel = func() { close(ready) }
	f.mu.Unlock()
	return func(item qrItem) {
		<-ready
		f.qr <- item
	}
}
func (f *fakeLinker) PairPhone(ctx context.Context, phone string) (string, error) {
	f.mu.Lock()
	hook := f.pairHook
	f.mu.Unlock()
	if hook != nil {
		hook()
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	f.pairPhones = append(f.pairPhones, phone)
	return f.pairCode, f.pairErr
}
func (f *fakeLinker) Logout(ctx context.Context) error              { return nil }
func (f *fakeLinker) OwnJID() string                                { return f.own }
func (f *fakeLinker) OwnLID() string                                { return f.ownLID }
func (f *fakeLinker) Chats(ctx context.Context) ([]chatInfo, error) { return f.chats, nil }

type recorder struct {
	mu  sync.Mutex
	evs []outMsg
}

func (r *recorder) emit(m outMsg) { r.mu.Lock(); r.evs = append(r.evs, m); r.mu.Unlock() }
func (r *recorder) last(typ string) outMsg {
	r.mu.Lock()
	defer r.mu.Unlock()
	for i := len(r.evs) - 1; i >= 0; i-- {
		if r.evs[i]["type"] == typ {
			return r.evs[i]
		}
	}
	return nil
}
func (r *recorder) waitFor(t *testing.T, typ, key, want string) outMsg {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if m := r.last(typ); m != nil && (key == "" || m[key] == want) {
			return m
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("no %s event with %s=%s; got %v", typ, key, want, r.evs)
	return nil
}

func TestBootWithoutSessionStaysIdle(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	r.waitFor(t, "status", "state", "idle")
	if f.connects != 0 {
		t.Fatalf("must not connect without a session, connects=%d", f.connects)
	}
}

func TestBootWithSessionConnects(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	s := newSession(f, r.emit)
	s.boot()
	if f.connects != 1 {
		t.Fatalf("connects=%d", f.connects)
	}
	s.onConnected()
	m := r.waitFor(t, "status", "state", "connected")
	if m["jid"] != f.own {
		t.Fatalf("jid=%v", m["jid"])
	}
	if _, present := m["lid"]; present {
		t.Fatalf("lid must be omitted when none is stored yet, got %v", m["lid"])
	}
}

// TestConnectedCarriesOwnLID: the self-chat can arrive addressed by LID
// instead of phone JID, so the kernel needs the LID alongside the JID on
// `connected` to recognise it (coordinator addendum to C3).
func TestConnectedCarriesOwnLID(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	f.ownLID = "123456789@lid"
	s := newSession(f, r.emit)
	s.boot()
	s.onConnected()
	m := r.waitFor(t, "status", "state", "connected")
	if m["lid"] != "123456789@lid" {
		t.Fatalf("lid=%v", m["lid"])
	}
}

func TestDisconnectAndLoggedOut(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	s := newSession(f, r.emit)
	s.boot()
	s.onConnected()
	s.onDisconnected("stream_replaced")
	r.waitFor(t, "status", "reason", "stream_replaced")
	s.onLoggedOut()
	r.waitFor(t, "status", "state", "logged_out")
}

var _ = errors.New

func TestLinkQRStreamsCodesAndTimesOut(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err != nil {
		t.Fatal(err)
	}
	r.waitFor(t, "status", "mode", "qr")
	f.qr <- qrItem{Event: "code", Code: "2@abc"}
	r.waitFor(t, "qr", "code", "2@abc")
	f.qr <- qrItem{Event: "timeout"}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "expired" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

func TestLinkPhoneEmitsCodeWithExpiry(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.pairCode = "K3M9QX2P"
	s := newSession(f, r.emit)
	s.boot()
	send := f.armQRChannel()
	go send(qrItem{Event: "code", Code: "2@first"}) // websocket ready
	if err := s.linkPhone("5491123456789"); err != nil {
		t.Fatal(err)
	}
	m := r.waitFor(t, "pairing_code", "code", "K3M9QX2P")
	exp, _ := m["expires_at"].(int64)
	if d := time.Until(time.Unix(exp, 0)); d < 150*time.Second || d > 170*time.Second {
		t.Fatalf("expires_at off: %v", d)
	}
	if len(f.pairPhones) != 1 || f.pairPhones[0] != "5491123456789" {
		t.Fatalf("phones=%v", f.pairPhones)
	}
	if r.last("qr") != nil {
		t.Fatalf("phone linking must not emit qr events")
	}
}

func TestLinkPhoneRejectsInvalidNumbers(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	for _, p := range []string{"", "+5491123456789", "549 11 2345", "0123456789", "12345", "1234567890123456", "54911abc6789"} {
		if err := s.linkPhone(p); err != errInvalidPhone {
			t.Fatalf("%q: err=%v", p, err)
		}
	}
	if f.connects != 0 {
		t.Fatalf("invalid numbers must not connect")
	}
}

func TestNewLinkCancelsPreviousAndCancelGoesIdle(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.pairCode = "AAAA1111"
	s := newSession(f, r.emit)
	s.boot()
	_ = s.linkQR()
	send := f.armQRChannel()
	go send(qrItem{Event: "code", Code: "2@x"})
	_ = s.linkPhone("5491123456789")
	if f.disconns < 1 {
		t.Fatalf("starting a new link must disconnect the previous attempt")
	}
	s.linkCancel()
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "cancelled" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

func TestLinkWhenAlreadyConnectedFails(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	s := newSession(f, r.emit)
	s.boot()
	s.onConnected()
	if err := s.linkQR(); err != errAlreadyLinked {
		t.Fatalf("err=%v", err)
	}
}

func TestPhoneCodeExpiresToIdle(t *testing.T) { // Review Focus 2
	f, r := newFake(), &recorder{}
	f.pairCode = "BBBB2222"
	s := newSession(f, r.emit)
	s.boot()
	send := f.armQRChannel()
	go send(qrItem{Event: "code", Code: "2@y"})
	_ = s.linkPhone("5491123456789")
	f.qr <- qrItem{Event: "timeout"}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "expired" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

// --- Fix round 1 regression tests (see task-2-report.md) ---

func TestStartLockedConnectFailureAfterReplacingAttemptGoesIdle(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err != nil {
		t.Fatal(err)
	}
	r.waitFor(t, "status", "mode", "qr")
	f.connectErr = errors.New("boom")
	if err := s.linkQR(); err == nil {
		t.Fatal("expected an error")
	}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "error" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

// TestStartLockedFirstAttemptConnectFailureGoesIdle: I6.2. The normal first
// click (nothing was linking before, e.g. no internet) must still report
// `status` `idle`/`reason:"error"` — Rust already answered the request `ok`,
// so silence here would leave the card at "Sin vincular" with no error box.
func TestStartLockedFirstAttemptConnectFailureGoesIdle(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.connectErr = errors.New("boom")
	s := newSession(f, r.emit)
	s.boot() // no stored session: boot leaves it at idle, does not call Connect
	if err := s.linkQR(); err == nil {
		t.Fatal("expected an error")
	}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "error" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

// TestStartLockedFirstAttemptQRChannelFailureGoesIdle: same as above, for the
// QRChannel() failure path.
func TestStartLockedFirstAttemptQRChannelFailureGoesIdle(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.qrChanErr = errors.New("boom")
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err == nil {
		t.Fatal("expected an error")
	}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "error" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

func TestStartLockedQRChannelFailureAfterReplacingAttemptGoesIdle(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err != nil {
		t.Fatal(err)
	}
	r.waitFor(t, "status", "mode", "qr")
	f.qrChanErr = errors.New("boom")
	if err := s.linkPhone("5491123456789"); err == nil {
		t.Fatal("expected an error")
	}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "error" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

func TestLinkPhoneCancelledDuringPairPhoneSkipsPairingCode(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.pairCode = "CCCC3333"
	f.pairHook = func() { f.cancelFn() }
	s := newSession(f, r.emit)
	s.boot()
	f.cancelFn = s.linkCancel
	send := f.armQRChannel()
	go send(qrItem{Event: "code", Code: "2@z"})
	if err := s.linkPhone("5491123456789"); err != nil {
		t.Fatal(err)
	}
	if r.last("pairing_code") != nil {
		t.Fatalf("pairing_code must not be emitted for an attempt cancelled mid-PairPhone")
	}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "cancelled" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

func TestLinkPhonePairPhoneErrorAfterCancelDoesNotOverwriteStatus(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.pairErr = errors.New("pair failed")
	s := newSession(f, r.emit)
	s.boot()
	f.cancelFn = s.linkCancel
	f.pairHook = func() { f.cancelFn() }
	send := f.armQRChannel()
	go send(qrItem{Event: "code", Code: "2@w"})
	_ = s.linkPhone("5491123456789")
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "cancelled" {
		t.Fatalf("reason=%v, want cancelled (a stale PairPhone error must not overwrite it)", m["reason"])
	}
}

func TestLinkQRChannelClosedWhileCurrentGoesIdleWithError(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.qrChanClosed = true
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err != nil {
		t.Fatal(err)
	}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "error" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

func TestWatchMapsUnmappedTerminalItemsToErrorIdle(t *testing.T) {
	for _, evt := range []string{"err-unexpected-state", "err-scanned-without-multidevice"} {
		f, r := newFake(), &recorder{}
		s := newSession(f, r.emit)
		s.boot()
		if err := s.linkQR(); err != nil {
			t.Fatal(err)
		}
		r.waitFor(t, "status", "mode", "qr")
		f.qr <- qrItem{Event: evt}
		m := r.waitFor(t, "status", "state", "idle")
		if m["reason"] != "error" {
			t.Fatalf("%s: reason=%v", evt, m["reason"])
		}
	}
}

func TestLinkPhoneFirstItemTerminalSkipsPairPhone(t *testing.T) {
	cases := map[string]string{"timeout": "expired", "err-client-outdated": "outdated"}
	for evt, reason := range cases {
		f, r := newFake(), &recorder{}
		s := newSession(f, r.emit)
		s.boot()
		send := f.armQRChannel()
		go send(qrItem{Event: evt})
		if err := s.linkPhone("5491123456789"); err != nil {
			t.Fatal(err)
		}
		m := r.waitFor(t, "status", "state", "idle")
		if m["reason"] != reason {
			t.Fatalf("%s: reason=%v", evt, m["reason"])
		}
		if len(f.pairPhones) != 0 {
			t.Fatalf("%s: PairPhone must not be called, phones=%v", evt, f.pairPhones)
		}
	}
}

// --- Fix round 2 regression tests (see task-2-report.md) ---

// TestLinkQRFirstItemTimeoutGoesIdleExpired documents that, at the session
// layer, "the socket disconnected before any events.QR" and "the linker's
// own no-events.QR bound elapsed" are indistinguishable from — and handled
// identically to — any other QR rotation timeout: all three surface as a
// single qrItem{Event:"timeout"}, which maps to idle/"expired" (not
// "error") via terminalReasons. The linker-internal mapping for the first
// two (events.Disconnected, and waitForFirstQR's bound) is covered directly
// in linker_test.go; this test is the session-layer half of that contract.
func TestLinkQRFirstItemTimeoutGoesIdleExpired(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err != nil {
		t.Fatal(err)
	}
	r.waitFor(t, "status", "mode", "qr")
	f.qr <- qrItem{Event: "timeout"}
	m := r.waitFor(t, "status", "state", "idle")
	if m["reason"] != "expired" {
		t.Fatalf("reason=%v", m["reason"])
	}
}

// TestEndAttemptDeferredDisconnectSkippedIfNewerAttemptStarted exercises
// stillCurrentGen directly: linkCancel/endAttempt call s.lk.Disconnect()
// after releasing s.mu, and a newer attempt can start in that gap. The gen
// counter is what keeps that deferred Disconnect() from reaching into
// whatever attempt replaced the one that queued it (see session.go).
func TestEndAttemptDeferredDisconnectSkippedIfNewerAttemptStarted(t *testing.T) {
	f, r := newFake(), &recorder{}
	s := newSession(f, r.emit)
	s.boot()
	if err := s.linkQR(); err != nil {
		t.Fatal(err)
	}
	r.waitFor(t, "status", "mode", "qr")
	gen := s.gen
	if !s.stillCurrentGen(gen) {
		t.Fatal("gen should still be current right after starting the attempt")
	}
	if err := s.linkQR(); err != nil { // a newer attempt replaces it
		t.Fatal(err)
	}
	if s.stillCurrentGen(gen) {
		t.Fatal("gen must have advanced once a newer attempt started")
	}
}
