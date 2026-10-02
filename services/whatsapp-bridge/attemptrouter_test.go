package main

import "testing"

func TestAttemptRouterStaleAttemptNeverReceivesNewerEvents(t *testing.T) {
	r := &attemptRouter{}
	old := r.start()
	cur := r.start() // replaces old, without any explicit clear(old)

	if !r.routeQR([]string{"2@new"}) {
		t.Fatal("routeQR reported no current attempt")
	}
	select {
	case <-old.codes:
		t.Fatal("stale attempt received codes meant for the newer one")
	default:
	}
	select {
	case got := <-cur.codes:
		if len(got) != 1 || got[0] != "2@new" {
			t.Fatalf("current attempt got %v", got)
		}
	default:
		t.Fatal("current attempt did not receive its codes")
	}

	if !r.routeTerminal(qrItem{Event: "success"}) {
		t.Fatal("routeTerminal reported no current attempt")
	}
	select {
	case <-old.pairDone:
		t.Fatal("stale attempt received a terminal item meant for the newer one")
	default:
	}
	select {
	case got := <-cur.pairDone:
		if got.Event != "success" {
			t.Fatalf("current attempt got %+v", got)
		}
	default:
		t.Fatal("current attempt did not receive its terminal item")
	}

	if !r.routeRotate(advRotation{Old: "o", New: "n"}) {
		t.Fatal("routeRotate reported no current attempt")
	}
	select {
	case <-old.rotate:
		t.Fatal("stale attempt received a rotation meant for the newer one")
	default:
	}
	select {
	case got := <-cur.rotate:
		if got.Old != "o" || got.New != "n" {
			t.Fatalf("current attempt got %+v", got)
		}
	default:
		t.Fatal("current attempt did not receive its rotation")
	}
}

func TestAttemptRouterClearOnlyIfStillCurrent(t *testing.T) {
	r := &attemptRouter{}
	a := r.start()
	b := r.start() // b replaces a; a is now stale

	r.clear(a) // a's own cleanup must not clobber b
	if !r.routeQR([]string{"x"}) {
		t.Fatal("stale clear(a) wrongly removed the current attempt")
	}
	select {
	case got := <-b.codes:
		if len(got) != 1 || got[0] != "x" {
			t.Fatalf("b got %v", got)
		}
	default:
		t.Fatal("b should have received the codes")
	}

	r.clear(b)
	if r.routeQR([]string{"y"}) {
		t.Fatal("routeQR reported a current attempt after the real one cleared")
	}
}

func TestAttemptRouterNoCurrentAttemptDropsEvents(t *testing.T) {
	r := &attemptRouter{}
	if r.routeQR([]string{"x"}) {
		t.Fatal("routeQR reported a current attempt when there was none")
	}
	if r.routeTerminal(qrItem{Event: "error"}) {
		t.Fatal("routeTerminal reported a current attempt when there was none")
	}
}
