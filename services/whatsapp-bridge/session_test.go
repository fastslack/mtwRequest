package main

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"
)

type fakeLinker struct {
	mu         sync.Mutex
	session    bool
	connects   int
	disconns   int
	qr         chan qrItem
	pairCode   string
	pairErr    error
	pairPhones []string
	own        string
	chats      []chatInfo
}

func newFake() *fakeLinker { return &fakeLinker{qr: make(chan qrItem, 8), own: "5491123456789:12@s.whatsapp.net"} }

func (f *fakeLinker) HasSession() bool { f.mu.Lock(); defer f.mu.Unlock(); return f.session }
func (f *fakeLinker) Connect() error  { f.mu.Lock(); f.connects++; f.mu.Unlock(); return nil }
func (f *fakeLinker) Disconnect()     { f.mu.Lock(); f.disconns++; f.mu.Unlock() }
func (f *fakeLinker) QRChannel(ctx context.Context) (<-chan qrItem, error) { return f.qr, nil }
func (f *fakeLinker) PairPhone(ctx context.Context, phone string) (string, error) {
	f.mu.Lock(); defer f.mu.Unlock()
	f.pairPhones = append(f.pairPhones, phone)
	return f.pairCode, f.pairErr
}
func (f *fakeLinker) Logout(ctx context.Context) error            { return nil }
func (f *fakeLinker) OwnJID() string                              { return f.own }
func (f *fakeLinker) Chats(ctx context.Context) ([]chatInfo, error) { return f.chats, nil }

type recorder struct {
	mu  sync.Mutex
	evs []outMsg
}

func (r *recorder) emit(m outMsg) { r.mu.Lock(); r.evs = append(r.evs, m); r.mu.Unlock() }
func (r *recorder) last(typ string) outMsg {
	r.mu.Lock(); defer r.mu.Unlock()
	for i := len(r.evs) - 1; i >= 0; i-- {
		if r.evs[i]["type"] == typ { return r.evs[i] }
	}
	return nil
}
func (r *recorder) waitFor(t *testing.T, typ, key, want string) outMsg {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if m := r.last(typ); m != nil && (key == "" || m[key] == want) { return m }
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
	if f.connects != 0 { t.Fatalf("must not connect without a session, connects=%d", f.connects) }
}

func TestBootWithSessionConnects(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	s := newSession(f, r.emit)
	s.boot()
	if f.connects != 1 { t.Fatalf("connects=%d", f.connects) }
	s.onConnected()
	m := r.waitFor(t, "status", "state", "connected")
	if m["jid"] != f.own { t.Fatalf("jid=%v", m["jid"]) }
}

func TestDisconnectAndLoggedOut(t *testing.T) {
	f, r := newFake(), &recorder{}
	f.session = true
	s := newSession(f, r.emit)
	s.boot(); s.onConnected()
	s.onDisconnected("stream_replaced")
	r.waitFor(t, "status", "reason", "stream_replaced")
	s.onLoggedOut()
	r.waitFor(t, "status", "state", "logged_out")
}

var _ = errors.New
