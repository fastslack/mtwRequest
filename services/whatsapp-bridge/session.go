package main

import (
	"context"
	"errors"
	"regexp"
	"sync"
	"time"
)

const pairingTTL = 160 * time.Second

var phoneRE = regexp.MustCompile(`^[1-9][0-9]{7,14}$`)

var (
	errInvalidPhone  = errors.New("invalid_phone")
	errAlreadyLinked = errors.New("already_linked")
)

func validPhone(p string) bool { return phoneRE.MatchString(p) }

type session struct {
	mu     sync.Mutex
	lk     linker
	emit   func(outMsg)
	st     string
	mode   string
	cancel context.CancelFunc
	gen    int // bumped each time a new linking attempt starts (startLocked);
	// guards a Disconnect() deferred until after s.mu is released (linkCancel,
	// endAttempt) from hitting whatever attempt replaced the one that queued
	// it — see stillCurrentGen.
}

func newSession(lk linker, emit func(outMsg)) *session {
	return &session{lk: lk, emit: emit, st: "idle"}
}

func (s *session) state() string { s.mu.Lock(); defer s.mu.Unlock(); return s.st }

// setLocked must be called with s.mu held; extra keys go into the status event.
func (s *session) setLocked(st string, extra outMsg) {
	s.st = st
	m := outMsg{"type": "status", "state": st}
	for k, v := range extra {
		m[k] = v
	}
	s.emit(m)
}

func (s *session) boot() {
	s.mu.Lock()
	if !s.lk.HasSession() {
		s.setLocked("idle", nil)
		s.mu.Unlock()
		return
	}
	s.mu.Unlock()
	// Connect runs outside the lock so a slow handshake on boot doesn't hold
	// up a concurrent link_* call; the window where a link_* command could
	// race this Connect() (before onConnected/onDisconnected report back) is
	// accepted the same way startLocked's in-lock Connect() is below.
	if err := s.lk.Connect(); err != nil {
		s.mu.Lock()
		s.setLocked("disconnected", outMsg{"reason": "connect_failed"})
		s.mu.Unlock()
	}
}

func (s *session) onConnected() {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.cancel != nil {
		s.cancel()
		s.cancel = nil
	}
	s.mode = ""
	s.setLocked("connected", outMsg{"jid": s.lk.OwnJID()})
}

func (s *session) onDisconnected(reason string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.st == "linking" || s.st == "idle" {
		return // a linking websocket closing is not a "disconnected" session
	}
	s.setLocked("disconnected", outMsg{"reason": reason})
}

func (s *session) onLoggedOut() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.setLocked("logged_out", nil)
}

// terminalReasons maps every non-"code"/"success" qrItem.Event the linker can
// produce to the `status` `idle` reason it should report. "err-unexpected-state"
// and "err-scanned-without-multidevice" aren't named in whatsmeow's own QR-flow
// vocabulary as fatal, but we don't attempt to keep an attempt alive across
// them (see linker.go) so they end up here too, same as any other unmapped
// terminal item (closed channel included — see watch()).
var terminalReasons = map[string]string{
	"timeout":                         "expired",
	"err-client-outdated":             "outdated",
	"error":                           "error",
	"err-unexpected-state":            "error",
	"err-scanned-without-multidevice": "error",
}

// stopLocked cancels any linking attempt's context and reports whether the
// caller must also call s.lk.Disconnect() — true only if a linking attempt
// was actually in progress. Disconnect() itself is deliberately NOT called
// here: it can block for a while draining whatsmeow's handler queue, and
// most callers (linkCancel, endAttempt, linkPhone's PairPhone-error path)
// have nothing that depends on it finishing before they release s.mu, so
// they call it themselves after unlocking. startLocked is the one exception
// that keeps it inside the lock — see the comment there.
func (s *session) stopLocked() (needsDisconnect bool) {
	if s.cancel != nil {
		s.cancel()
		s.cancel = nil
	}
	return s.st == "linking"
}

// startLocked begins a new linking attempt, replacing any in progress. Caller
// holds s.mu for the duration of this call, including the Connect() below:
// whatsmeow dispatches events.Connected from its own goroutine only after
// Connect() returns, so onConnected can never observe s.mu mid-transition here.
//
// Unlike linkCancel/endAttempt, the Disconnect() of a replaced attempt (if
// any) is kept inside the lock here: it must complete before QRChannel/Connect
// is called for the new attempt, or the new Connect() could race the old
// socket's teardown. That's the accepted cost of replacing an attempt — a
// comparatively rare operation — against the stall Disconnect() can cause.
func (s *session) startLocked(mode string) (context.Context, <-chan qrItem, error) {
	if s.st == "connected" {
		return nil, nil, errAlreadyLinked
	}
	wasLinking := s.stopLocked()
	if wasLinking {
		s.lk.Disconnect()
	}
	ctx, cancel := context.WithCancel(context.Background())
	ch, err := s.lk.QRChannel(ctx)
	if err != nil {
		cancel()
		if wasLinking {
			s.setLocked("idle", outMsg{"reason": "error"})
		}
		return nil, nil, err
	}
	if err := s.lk.Connect(); err != nil {
		cancel()
		if wasLinking {
			s.setLocked("idle", outMsg{"reason": "error"})
		}
		return nil, nil, err
	}
	s.cancel = cancel
	s.mode = mode
	s.gen++
	s.setLocked("linking", outMsg{"mode": mode})
	return ctx, ch, nil
}

// stillCurrentGen reports whether no newer linking attempt has started since
// gen was captured. linkCancel and endAttempt call s.lk.Disconnect() after
// releasing s.mu (see stopLocked's doc comment) — in the gap between
// unlocking and that call actually running, a *different* link_qr/link_phone
// command could already have started a new attempt and connected it. Gating
// the deferred Disconnect() on this check closes that window down to just
// the check-then-call gap, instead of the full unlock-to-call one.
func (s *session) stillCurrentGen(gen int) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.gen == gen
}

func (s *session) linkQR() error {
	s.mu.Lock()
	ctx, ch, err := s.startLocked("qr")
	s.mu.Unlock()
	if err != nil {
		return err
	}
	go s.watch(ctx, ch, true)
	return nil
}

func (s *session) linkPhone(phone string) error {
	if !validPhone(phone) {
		return errInvalidPhone
	}
	s.mu.Lock()
	ctx, ch, err := s.startLocked("phone")
	s.mu.Unlock()
	if err != nil {
		return err
	}
	// whatsmeow: wait for the first QR item (login websocket ready) before
	// PairPhone. That first item can itself be terminal (e.g. the server
	// rejected the connection outright) — report its reason instead of
	// blindly calling PairPhone on a dead attempt.
	select {
	case it, ok := <-ch:
		if !ok {
			s.endAttempt(ctx, "error")
			return nil
		}
		if reason, terminal := terminalReasons[it.Event]; terminal {
			s.endAttempt(ctx, reason)
			return nil
		}
	case <-time.After(10 * time.Second):
	case <-ctx.Done():
		return ctx.Err()
	}
	code, err := s.lk.PairPhone(ctx, phone)
	if err != nil {
		s.endAttempt(ctx, "error")
		return err
	}
	if ctx.Err() != nil {
		// Cancelled while PairPhone was in flight: don't report a pairing
		// code for an attempt that's already gone.
		return nil
	}
	s.emit(outMsg{"type": "pairing_code", "code": code, "expires_at": time.Now().Add(pairingTTL).Unix()})
	go s.watch(ctx, ch, false)
	return nil
}

func (s *session) linkCancel() {
	s.mu.Lock()
	if s.st != "linking" {
		s.mu.Unlock()
		return
	}
	gen := s.gen
	needsDisconnect := s.stopLocked()
	s.setLocked("idle", outMsg{"reason": "cancelled"})
	s.mu.Unlock()
	if needsDisconnect && s.stillCurrentGen(gen) {
		s.lk.Disconnect()
	}
}

// endAttempt aborts the attempt identified by ctx and reports idle/reason,
// but only if it is still the current one: s.st must still be "linking" and
// ctx must not already be done. Without that guard, a stale attempt's own
// goroutine (watch, or linkPhone's PairPhone-error path) could clobber the
// status of whatever attempt replaced it.
func (s *session) endAttempt(ctx context.Context, reason string) {
	s.mu.Lock()
	if s.st != "linking" || ctx.Err() != nil {
		s.mu.Unlock()
		return
	}
	gen := s.gen
	needsDisconnect := s.stopLocked()
	s.setLocked("idle", outMsg{"reason": reason})
	s.mu.Unlock()
	if needsDisconnect && s.stillCurrentGen(gen) {
		s.lk.Disconnect()
	}
}

// watch drains the QR channel of one linking attempt. emitCodes=false for phone pairing.
func (s *session) watch(ctx context.Context, ch <-chan qrItem, emitCodes bool) {
	for {
		select {
		case <-ctx.Done():
			return
		case it, ok := <-ch:
			if !ok {
				// Channel closed without an explicit terminal item (e.g. the
				// linker's own goroutine exited unexpectedly) — don't leave
				// state stuck at "linking" forever.
				s.endAttempt(ctx, "error")
				return
			}
			switch it.Event {
			case "code":
				// ctx may have just gone Done the same instant this item
				// became ready; select can pick either, so re-check before
				// reporting a code for an attempt that's already stale.
				if emitCodes && ctx.Err() == nil {
					s.emit(outMsg{"type": "qr", "code": it.Code})
				}
			case "success":
				return // events.Connected → onConnected
			default:
				reason, known := terminalReasons[it.Event]
				if !known {
					reason = "error"
				}
				s.endAttempt(ctx, reason)
				return
			}
		}
	}
}
