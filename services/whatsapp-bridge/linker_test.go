package main

import (
	"context"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types/events"
)

func drainTerminal(t *testing.T, ch <-chan qrItem) qrItem {
	t.Helper()
	select {
	case it := <-ch:
		return it
	case <-time.After(2 * time.Second):
		t.Fatal("no terminal item routed")
		return qrItem{}
	}
}

// These tests call waLinker.handleEvent directly on a zero-value waLinker —
// none of the event types exercised here ever touch w.c (that only happens
// inside runAttempt's Disconnect() calls, not in handleEvent's dispatch), so
// no real *whatsmeow.Client is needed.

func TestHandleEventRoutesEachEventTypeToTerminal(t *testing.T) {
	cases := []struct {
		name string
		evt  any
		want string
	}{
		{"PairSuccess", &events.PairSuccess{}, "success"},
		{"PairError", &events.PairError{}, "error"},
		{"ClientOutdated", &events.ClientOutdated{}, "err-client-outdated"},
		{"QRScannedWithoutMultidevice", &events.QRScannedWithoutMultidevice{}, "err-scanned-without-multidevice"},
		{"Disconnected", &events.Disconnected{}, "timeout"},
		{"ConnectFailure", &events.ConnectFailure{}, "err-unexpected-state"},
		{"LoggedOut", &events.LoggedOut{}, "err-unexpected-state"},
		{"TemporaryBan", &events.TemporaryBan{}, "err-unexpected-state"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			w := &waLinker{}
			a := w.router.start()
			w.handleEvent(c.evt)
			item := drainTerminal(t, a.pairDone)
			if item.Event != c.want {
				t.Fatalf("item.Event = %q, want %q", item.Event, c.want)
			}
		})
	}
}

func TestHandleEventRoutesQRCodesToCurrentAttempt(t *testing.T) {
	w := &waLinker{}
	a := w.router.start()
	w.handleEvent(&events.QR{Codes: []string{"2@a", "2@b"}})
	select {
	case codes := <-a.codes:
		if len(codes) != 2 || codes[0] != "2@a" || codes[1] != "2@b" {
			t.Fatalf("codes=%v", codes)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no codes routed")
	}
}

func TestHandleEventRoutesRotateADVSecretToCurrentAttempt(t *testing.T) {
	w := &waLinker{}
	a := w.router.start()
	w.handleEvent(&events.RotateADVSecret{OldSecret: "old", NewSecret: "new"})
	select {
	case rot := <-a.rotate:
		if rot.Old != "old" || rot.New != "new" {
			t.Fatalf("rot=%+v", rot)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no rotation routed")
	}
}

func TestHandleEventIgnoresUnmappedEventTypes(t *testing.T) {
	w := &waLinker{}
	a := w.router.start()
	w.handleEvent(&events.Connected{}) // not in the switch at all — must be a no-op
	select {
	case item := <-a.pairDone:
		t.Fatalf("unexpected terminal item %+v for an unmapped event", item)
	default:
	}
}

// waitForFirstQR is the pure decision logic behind runAttempt's initial
// wait for whatsmeow's first events.QR. These tests drive it directly,
// without any whatsmeow dependency.

func TestWaitForFirstQRReturnsCodesWhenTheyArriveFirst(t *testing.T) {
	codes := make(chan []string, 1)
	codes <- []string{"2@a"}
	res := waitForFirstQR(context.Background(), codes, make(chan time.Time)) // bound never fires
	if res.timedOut {
		t.Fatal("res.timedOut, want codes")
	}
	if len(res.codes) != 1 || res.codes[0] != "2@a" {
		t.Fatalf("res.codes=%v", res.codes)
	}
}

func TestWaitForFirstQRTimesOutWhenBoundElapses(t *testing.T) {
	bound := make(chan time.Time, 1)
	done := make(chan firstQRResult, 1)
	go func() { done <- waitForFirstQR(context.Background(), make(chan []string), bound) }()
	bound <- time.Now()
	res := <-done
	if !res.timedOut {
		t.Fatalf("res=%+v, want timedOut", res)
	}
	if res.codes != nil {
		t.Fatalf("res.codes=%v, want nil on a bound timeout", res.codes)
	}
}

func TestWaitForFirstQRReturnsEmptyOnCtxCancel(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	res := waitForFirstQR(ctx, make(chan []string), make(chan time.Time))
	if res.timedOut {
		t.Fatal("res.timedOut, want a plain cancel (not a bound timeout)")
	}
	if res.codes != nil {
		t.Fatalf("res.codes=%v, want nil", res.codes)
	}
}
