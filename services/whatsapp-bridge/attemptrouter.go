package main

import "sync"

// linkAttempt is one QRChannel(ctx) call's state, as seen from the single
// permanent whatsmeow event handler in linker.go: the codes from one
// events.QR, the one terminal pairing result (success/error/outdated/...),
// and any ADV secret rotations the rotator needs to patch into its codes.
// codes/pairDone are buffered 1 (one events.QR, one terminal result per
// attempt); rotate is buffered 4, matching whatsmeow's own qrchan.go
// rotateAdv channel. All three are only ever written by routeQR/
// routeTerminal/routeRotate, called synchronously from whatsmeow's own event
// dispatch, so none of them must block.
type linkAttempt struct {
	codes    chan []string
	pairDone chan qrItem
	rotate   chan advRotation
}

func newLinkAttempt() *linkAttempt {
	return &linkAttempt{
		codes:    make(chan []string, 1),
		pairDone: make(chan qrItem, 1),
		rotate:   make(chan advRotation, 4),
	}
}

// attemptRouter tracks which linkAttempt is "current" so a stale attempt
// (one that's been replaced by a newer link_qr/link_phone call, or that has
// already finished) never receives an event meant for the one that replaced
// it. It has no dependency on whatsmeow — purely a routing/bookkeeping
// primitive, kept separate so it can be unit-tested directly.
//
// This exists because whatsmeow's own GetQRChannel (qrchan.go) ties a QR
// channel's lifetime to a per-attempt event handler that must be registered
// and removed with each attempt; cancelling an attempt doesn't reliably
// remove its handler (see linker.go's doc comment on waLinker), so a leftover
// handler can still react to the NEXT attempt's events.QR. Routing through a
// single long-lived handler and one `cur` pointer here sidesteps that: there
// is only ever one handler, and only the current attempt is reachable from it.
type attemptRouter struct {
	mu  sync.Mutex
	cur *linkAttempt
}

// start installs a new current attempt, replacing (without otherwise
// touching) whatever was current before.
func (r *attemptRouter) start() *linkAttempt {
	a := newLinkAttempt()
	r.mu.Lock()
	r.cur = a
	r.mu.Unlock()
	return a
}

// clear removes a as the current attempt, but only if it still is one — a
// stale attempt's own cleanup must never clobber whatever replaced it.
func (r *attemptRouter) clear(a *linkAttempt) {
	r.mu.Lock()
	if r.cur == a {
		r.cur = nil
	}
	r.mu.Unlock()
}

// routeQR delivers codes to the current attempt, if there is one, and
// reports whether there was one to deliver to.
func (r *attemptRouter) routeQR(codes []string) bool {
	a := r.current()
	if a == nil {
		return false
	}
	select {
	case a.codes <- codes:
	default:
	}
	return true
}

// routeTerminal delivers a terminal pairing result to the current attempt,
// if there is one, and reports whether there was one to deliver to.
func (r *attemptRouter) routeTerminal(item qrItem) bool {
	a := r.current()
	if a == nil {
		return false
	}
	select {
	case a.pairDone <- item:
	default:
	}
	return true
}

// routeRotate delivers an ADV secret rotation to the current attempt, if
// there is one, and reports whether there was one to deliver to.
func (r *attemptRouter) routeRotate(rot advRotation) bool {
	a := r.current()
	if a == nil {
		return false
	}
	select {
	case a.rotate <- rot:
	default:
	}
	return true
}

func (r *attemptRouter) current() *linkAttempt {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.cur
}
