//go:build windows

package main

import (
	"fmt"
	"net"
	"strings"

	"github.com/Microsoft/go-winio"
)

const defaultSocket = `\\.\pipe\mtw-whatsapp`

// Full access for the pipe's owner (the user running the bridge) and SYSTEM;
// nobody else can open it.
const pipeSDDL = "D:P(A;;GA;;;OW)(A;;GA;;;SY)"

// listen opens the named pipe mtw-server dials.
func listen(endpoint string) (net.Listener, error) {
	if !strings.HasPrefix(endpoint, `\\.\pipe\`) {
		return nil, fmt.Errorf(`on Windows MTW_WHATSAPP_SOCKET must be a named pipe (\\.\pipe\...)`)
	}
	l, err := winio.ListenPipe(endpoint, &winio.PipeConfig{SecurityDescriptor: pipeSDDL})
	if err != nil {
		return nil, fmt.Errorf("listen: %w", err)
	}
	return l, nil
}

func dialEndpoint(ep string) (net.Conn, error) { return winio.DialPipe(ep, nil) }
