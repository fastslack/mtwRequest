package main

import (
	"errors"
	"testing"
)

type fakeWaClient struct{ disconnected bool }

func (f *fakeWaClient) Disconnect() { f.disconnected = true }

type fakeSessionStore struct {
	closed  bool
	closeAt func() // if set, called to observe ordering before Close returns
	err     error
}

func (f *fakeSessionStore) Close() error {
	f.closed = true
	if f.closeAt != nil {
		f.closeAt()
	}
	return f.err
}

func TestShutdownDisconnectsThenClosesStore(t *testing.T) {
	c := &fakeWaClient{}
	var disconnectedBeforeClose bool
	s := &fakeSessionStore{closeAt: func() { disconnectedBeforeClose = c.disconnected }}

	shutdown(c, s)

	if !c.disconnected {
		t.Fatal("client was not disconnected")
	}
	if !s.closed {
		t.Fatal("session store was not closed")
	}
	if !disconnectedBeforeClose {
		t.Fatal("store closed before client disconnected")
	}
}

func TestShutdownSurvivesStoreCloseError(t *testing.T) {
	c := &fakeWaClient{}
	s := &fakeSessionStore{err: errors.New("disk gone")}

	// Must not panic even though Close returns an error — shutdown only
	// logs it.
	shutdown(c, s)

	if !c.disconnected || !s.closed {
		t.Fatal("shutdown did not run both steps despite the store error")
	}
}
