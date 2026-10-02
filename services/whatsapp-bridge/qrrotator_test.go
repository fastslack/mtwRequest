package main

import (
	"context"
	"strings"
	"sync"
	"testing"
	"time"
)

// fakeAfter lets a test drive qrRotator's timing without real waits, while
// still recording what durations were requested and in what order.
type fakeAfter struct {
	mu   sync.Mutex
	reqs []time.Duration
}

func (f *fakeAfter) after(d time.Duration) <-chan time.Time {
	f.mu.Lock()
	f.reqs = append(f.reqs, d)
	f.mu.Unlock()
	ch := make(chan time.Time, 1)
	ch <- time.Now()
	return ch
}

func (f *fakeAfter) durations() []time.Duration {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]time.Duration(nil), f.reqs...)
}

func drain(t *testing.T, out <-chan qrItem, n int) []qrItem {
	t.Helper()
	items := make([]qrItem, 0, n)
	for i := 0; i < n; i++ {
		select {
		case it := <-out:
			items = append(items, it)
		case <-time.After(2 * time.Second):
			t.Fatalf("timed out waiting for item %d/%d; got %v", i+1, n, items)
		}
	}
	return items
}

func TestQRRotatorOrderAndTimings(t *testing.T) {
	fa := &fakeAfter{}
	r := &qrRotator{after: fa.after}
	codes := []string{"c1", "c2", "c3", "c4", "c5", "c6"}
	out := make(chan qrItem, len(codes)+1)
	r.run(context.Background(), codes, out, nil)

	items := drain(t, out, len(codes)+1)
	for i, code := range codes {
		if items[i].Event != "code" || items[i].Code != code {
			t.Fatalf("item %d = %+v, want code %q", i, items[i], code)
		}
	}
	if items[len(codes)].Event != "timeout" {
		t.Fatalf("last item = %+v, want timeout", items[len(codes)])
	}

	durs := fa.durations()
	if len(durs) != len(codes) {
		t.Fatalf("durations=%v, want %d entries", durs, len(codes))
	}
	if durs[0] != qrFirstTimeout {
		t.Fatalf("first timeout=%v, want %v", durs[0], qrFirstTimeout)
	}
	for i := 1; i < len(durs); i++ {
		if durs[i] != qrRestTimeout {
			t.Fatalf("timeout[%d]=%v, want %v", i, durs[i], qrRestTimeout)
		}
	}
}

func TestQRRotatorCtxCancelClosesWithoutTimeout(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	r := &qrRotator{after: func(time.Duration) <-chan time.Time {
		// Never fires: the rotator must notice ctx.Done() instead, not this.
		return make(chan time.Time)
	}}
	out := make(chan qrItem, 8)
	cancel() // cancelled before run() ever starts waiting
	done := make(chan struct{})
	go func() {
		r.run(ctx, []string{"c1", "c2"}, out, nil)
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("run() did not return after ctx cancel")
	}
	select {
	case it := <-out:
		if it.Event == "timeout" {
			t.Fatalf("ctx-cancelled run must not emit timeout, got %+v", it)
		}
		// A single buffered "code" send landing before cancel was observed is
		// fine; just make sure "timeout" specifically never appears.
	default:
	}
}

func TestQRRotatorNoCodesEmitsTimeoutImmediately(t *testing.T) {
	fa := &fakeAfter{}
	r := &qrRotator{after: fa.after}
	out := make(chan qrItem, 1)
	r.run(context.Background(), nil, out, nil)
	items := drain(t, out, 1)
	if items[0].Event != "timeout" {
		t.Fatalf("item = %+v, want timeout", items[0])
	}
	if len(fa.durations()) != 0 {
		t.Fatalf("no codes means no waits, got %v", fa.durations())
	}
}

// seqAfter is a controllable `after` fake: each call appends a fresh channel
// to a queue, and fire(n) sends on the n-th one requested — letting a test
// advance qrRotator's wait exactly when it wants to, one request at a time.
// It polls briefly for the n-th request to exist rather than assuming it
// already does, since the rotator goroutine requests it asynchronously.
type seqAfter struct {
	mu    sync.Mutex
	chans []chan time.Time
}

func (s *seqAfter) after(time.Duration) <-chan time.Time {
	ch := make(chan time.Time, 1)
	s.mu.Lock()
	s.chans = append(s.chans, ch)
	s.mu.Unlock()
	return ch
}

func (s *seqAfter) fire(t *testing.T, n int) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for {
		s.mu.Lock()
		if len(s.chans) > n {
			ch := s.chans[n]
			s.mu.Unlock()
			ch <- time.Now()
			return
		}
		s.mu.Unlock()
		if time.Now().After(deadline) {
			t.Fatalf("after() was never called a %dth time", n+1)
		}
		time.Sleep(time.Millisecond)
	}
}

func TestQRRotatorRotatesADVSecretAndReemitsCurrentCode(t *testing.T) {
	sa := &seqAfter{}
	r := &qrRotator{after: sa.after}
	codes := []string{
		"https://wa.me/settings/linked_devices#ref1,noise,identity,OLDSECRET,1",
		"https://wa.me/settings/linked_devices#ref2,noise,identity,OLDSECRET,1",
	}
	out := make(chan qrItem, 8)
	rotate := make(chan advRotation, 1)
	done := make(chan struct{})
	go func() {
		r.run(context.Background(), codes, out, rotate)
		close(done)
	}()

	first := drain(t, out, 1)[0]
	if first.Code != codes[0] {
		t.Fatalf("first code = %q, want %q", first.Code, codes[0])
	}

	rotate <- advRotation{Old: "OLDSECRET", New: "NEWSECRET"}

	wantReemitted := strings.Replace(codes[0], "OLDSECRET", "NEWSECRET", 1)
	reemitted := drain(t, out, 1)[0]
	if reemitted.Code != wantReemitted {
		t.Fatalf("re-emitted code = %q, want %q", reemitted.Code, wantReemitted)
	}

	// Let the re-emitted code's (fresh) wait elapse so the rotator advances
	// to the second code — which must carry the new secret too, since the
	// rotation patches every remaining code, not just the one in flight.
	sa.fire(t, 1)
	wantSecond := strings.Replace(codes[1], "OLDSECRET", "NEWSECRET", 1)
	second := drain(t, out, 1)[0]
	if second.Code != wantSecond {
		t.Fatalf("second code = %q, want %q (later codes must be patched too)", second.Code, wantSecond)
	}

	sa.fire(t, 2) // second code's wait: codes exhausted, rotator emits timeout
	last := drain(t, out, 1)[0]
	if last.Event != "timeout" {
		t.Fatalf("last item = %+v, want timeout", last)
	}
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("run() did not return after emitting timeout")
	}
}
