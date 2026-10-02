package main

import (
	"io"
	"os"
	"testing"
	"time"
)

func TestStdinEOFRequestsShutdown(t *testing.T) {
	r, w := io.Pipe()
	stop := make(chan os.Signal, 1)
	watchStdinEOF(r, stop)
	select {
	case <-stop:
		t.Fatal("stopped before EOF")
	case <-time.After(50 * time.Millisecond):
	}
	_, _ = w.Write([]byte("noise\n"))
	_ = w.Close()
	select {
	case <-stop:
	case <-time.After(2 * time.Second):
		t.Fatal("no shutdown after stdin EOF")
	}
}

func TestStdinEOFDoesNotBlockWhenStopIsFull(t *testing.T) {
	r, w := io.Pipe()
	stop := make(chan os.Signal, 1)
	stop <- os.Interrupt // already full: Ctrl+C came first
	watchStdinEOF(r, stop)
	_ = w.Close()
	time.Sleep(50 * time.Millisecond) // must not deadlock or panic
}
