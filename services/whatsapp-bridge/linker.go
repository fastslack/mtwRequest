package main

import (
	"context"
	"errors"
	"slices"
	"sort"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types/events"
)

// qrFirstEventTimeout bounds how long an attempt waits for whatsmeow's first
// events.QR after Connect() before giving up. Without this, a login socket
// that connects but never gets a pair-device stanza (or that silently stalls)
// would leave the attempt — and the session's `status` — stuck at "linking"
// forever, since nothing else would ever signal it's over.
const qrFirstEventTimeout = 30 * time.Second

type qrItem struct {
	Event string
	Code  string
}

type chatInfo struct {
	JID     string `json:"jid"`
	Name    string `json:"name"`
	IsGroup bool   `json:"is_group"`
	LastTS  int64  `json:"last_ts"`
}

// linker is the slice of whatsmeow the linking flow needs; tests fake it.
type linker interface {
	HasSession() bool
	Connect() error
	Disconnect()
	QRChannel(ctx context.Context) (<-chan qrItem, error)
	PairPhone(ctx context.Context, phone string) (string, error)
	Logout(ctx context.Context) error
	OwnJID() string
	Chats(ctx context.Context) ([]chatInfo, error)
}

var errSessionExists = errors.New("a stored session already exists")

// waLinker implements linker on top of whatsmeow, but deliberately does NOT
// use whatsmeow's own Client.GetQRChannel (see qrchan.go in the pinned
// whatsmeow version). That type ties a QR attempt's lifetime to a per-call
// event handler it registers with AddEventHandler and expects to remove with
// RemoveEventHandler once the attempt ends — but its own emitQRs goroutine
// only does that cleanly on ctx.Done() or on running out of codes. Cancelling
// an attempt the way session.go does (cancel() then Disconnect()) takes the
// *other* branch of emitQRs' select roughly half the time (the one gated on
// cli.expectedDisconnect, set by Disconnect()), which just returns without
// closing the channel or removing the handler. The leaked handler then fires
// on the NEXT attempt's events.QR, pushes into its own now-abandoned channel,
// sees its context already done and calls cli.Disconnect() itself — killing
// the new attempt too, whose own (correctly-exiting) emitter then returns
// without ever sending "timeout", so session.go's watch() never hears back
// and state sticks at "linking" forever. Repeat attempts cascade until the
// process restarts.
//
// Instead, waLinker registers exactly ONE permanent event handler at
// construction and owns QR rotation itself (qrRotator, attemptrouter.go/
// qrrotator.go — both unit-tested independent of whatsmeow). There is never
// more than one whatsmeow event handler for the whole process lifetime, so
// there is nothing to leak.
type waLinker struct {
	c      *whatsmeow.Client
	router attemptRouter
}

func newWaLinker(c *whatsmeow.Client) *waLinker {
	w := &waLinker{c: c}
	c.AddEventHandler(w.handleEvent)
	return w
}

func (w *waLinker) HasSession() bool { return w.c.Store.ID != nil }
func (w *waLinker) Connect() error   { return w.c.Connect() }
func (w *waLinker) Disconnect()      { w.c.Disconnect() }
func (w *waLinker) OwnJID() string {
	if w.c.Store.ID == nil {
		return ""
	}
	return w.c.Store.ID.String()
}
func (w *waLinker) Logout(ctx context.Context) error { return w.c.Logout(ctx) }

// QRChannel starts a new linking attempt and returns its item channel,
// replacing whatever attempt was current (the old one's context is already
// cancelled by session.go before this is called again). It refuses to start
// one when a session is already stored, matching whatsmeow's own
// GetQRChannel contract (ErrQRStoreContainsID) — session.go's startLocked
// relies on that guarantee; since we no longer call GetQRChannel at all, we
// have to keep making it ourselves.
func (w *waLinker) QRChannel(ctx context.Context) (<-chan qrItem, error) {
	if w.c.Store.ID != nil {
		return nil, errSessionExists
	}
	a := w.router.start()
	out := make(chan qrItem, 8)
	go w.runAttempt(a, ctx, out)
	return out, nil
}

// firstQRResult is waitForFirstQR's pure outcome: either the codes from the
// first events.QR, or a report that nothing arrived in time.
type firstQRResult struct {
	codes    []string
	timedOut bool
}

// waitForFirstQR waits for a.codes (the first events.QR), the bound elapsing,
// or ctx being cancelled — deciding the outcome without doing any I/O itself,
// so it's unit-testable without a whatsmeow client. bound is a channel
// rather than a duration so tests can control it directly (same pattern as
// qrRotator's injectable `after`).
func waitForFirstQR(ctx context.Context, codes <-chan []string, bound <-chan time.Time) firstQRResult {
	select {
	case c := <-codes:
		return firstQRResult{codes: c}
	case <-bound:
		return firstQRResult{timedOut: true}
	case <-ctx.Done():
		return firstQRResult{}
	}
}

// runAttempt is the sole sender into out for this attempt — qrRotator.run
// only runs here, under this goroutine's control, so there is never a
// concurrent send to out from two different goroutines, and only this
// goroutine ever closes it.
func (w *waLinker) runAttempt(a *linkAttempt, ctx context.Context, out chan qrItem) {
	defer func() {
		w.router.clear(a)
		close(out)
	}()

	rotCtx, rotCancel := context.WithCancel(ctx)
	defer rotCancel()
	rotDone := make(chan struct{})
	go func() {
		defer close(rotDone)
		res := waitForFirstQR(rotCtx, a.codes, time.After(qrFirstEventTimeout))
		switch {
		case res.timedOut:
			// No events.QR within the bound: report it the same way running
			// out of codes does, rather than leaving the attempt (and
			// `status`) stuck at "linking" forever.
			select {
			case out <- qrItem{Event: "timeout"}:
			case <-rotCtx.Done():
			}
		case res.codes != nil:
			newQRRotator().run(rotCtx, res.codes, out, a.rotate)
		default:
			// rotCtx was cancelled before anything arrived; nothing to do.
		}
	}()

	select {
	case <-ctx.Done():
		return
	case item := <-a.pairDone:
		// Stop any in-progress rotation (without it emitting "timeout") and
		// wait for it to actually stop before this goroutine sends anything
		// itself — out must never have two concurrent senders.
		rotCancel()
		<-rotDone
		select {
		case out <- item:
		case <-ctx.Done():
			return
		}
		// Guard with ctx.Err(): a genuine whatsmeow event for this attempt
		// can still be routed to it in the brief window between session.go
		// cancelling ctx and this goroutine noticing (see attemptrouter.go).
		// Without the guard, this Disconnect() would hit whatever attempt
		// replaced this one — possibly already connected — instead of a
		// no-op on an attempt nobody cares about anymore.
		if item.Event != "success" && ctx.Err() == nil {
			w.c.Disconnect()
		}
	case <-rotDone:
		// Codes ran out naturally and qrRotator already sent "timeout" (or
		// the no-QR bound elapsed and the goroutine above sent it directly);
		// mirror whatsmeow's own emitQRs, which disconnects at that point.
		if ctx.Err() == nil {
			w.c.Disconnect()
		}
	}
}

// handleEvent is the one permanent handler for every login-flow event,
// registered once in newWaLinker. It only ever reaches the attempt the
// router considers current.
func (w *waLinker) handleEvent(evt any) {
	switch v := evt.(type) {
	case *events.QR:
		w.router.routeQR(slices.Clone(v.Codes))
	case *events.RotateADVSecret:
		// The server rotated the ADV secret mid-attempt (pair.go's
		// rotateADVSecret); every not-yet-used QR code embeds the old one
		// and would fail pairing with hmac-mismatch. See advRotation's doc
		// comment in qrrotator.go.
		w.router.routeRotate(advRotation{Old: v.OldSecret, New: v.NewSecret})
	case *events.PairSuccess:
		w.router.routeTerminal(qrItem{Event: "success"})
	case *events.PairError:
		w.router.routeTerminal(qrItem{Event: "error"})
	case *events.ClientOutdated:
		w.router.routeTerminal(qrItem{Event: "err-client-outdated"})
	case *events.QRScannedWithoutMultidevice:
		w.router.routeTerminal(qrItem{Event: "err-scanned-without-multidevice"})
	case *events.Disconnected:
		// Mirrors whatsmeow's own qrchan.go (handleEvent's `case
		// *events.Disconnected: outputType = QRChannelTimeout`): the socket
		// dropped — by the server or a network error — before pairing
		// finished, with nothing else telling this attempt it's over.
		w.router.routeTerminal(qrItem{Event: "timeout"})
	case *events.ConnectFailure, *events.LoggedOut, *events.TemporaryBan:
		// Mirrors whatsmeow's own QRChannelErrUnexpectedEvent (qrchan.go):
		// reaching one of these while an attempt is current means pairing
		// already happened, or failed, through some other path.
		w.router.routeTerminal(qrItem{Event: "err-unexpected-state"})
	}
}

func (w *waLinker) PairPhone(ctx context.Context, phone string) (string, error) {
	return w.c.PairPhone(ctx, phone, true, whatsmeow.PairClientChrome, "Chrome (Linux)")
}

func (w *waLinker) Chats(ctx context.Context) ([]chatInfo, error) {
	var out []chatInfo
	contacts, err := w.c.Store.Contacts.GetAllContacts(ctx)
	if err != nil {
		return nil, err
	}
	for jid, ci := range contacts {
		name := ci.FullName
		if name == "" {
			name = ci.PushName
		}
		if name == "" {
			name = ci.BusinessName
		}
		out = append(out, chatInfo{JID: jid.String(), Name: name})
	}
	groups, err := w.c.GetJoinedGroups(ctx)
	if err != nil {
		return nil, err
	}
	for _, g := range groups {
		out = append(out, chatInfo{JID: g.JID.String(), Name: g.GroupName.Name, IsGroup: true})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}
