package main

import (
	"context"
	"regexp"
	"sync"
	"time"
)

const pairingTTL = 160 * time.Second

var phoneRE = regexp.MustCompile(`^[1-9][0-9]{7,14}$`)

func validPhone(p string) bool { return phoneRE.MatchString(p) }

type session struct {
	mu     sync.Mutex
	lk     linker
	emit   func(outMsg)
	st     string
	mode   string
	cancel context.CancelFunc
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
	defer s.mu.Unlock()
	if !s.lk.HasSession() {
		s.setLocked("idle", nil)
		return
	}
	if err := s.lk.Connect(); err != nil {
		s.setLocked("disconnected", outMsg{"reason": "connect_failed"})
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
