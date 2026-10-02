package main

import (
	"io"
	"os"
)

// watchStdinEOF asks for shutdown once r reaches EOF. The kernel that
// supervises the bridge closes our stdin to stop us: Windows has no SIGTERM,
// and a hard kill could leave the session database half-written.
func watchStdinEOF(r io.Reader, stop chan<- os.Signal) {
	go func() {
		_, _ = io.Copy(io.Discard, r)
		select {
		case stop <- os.Interrupt:
		default: // a shutdown is already pending
		}
	}()
}
