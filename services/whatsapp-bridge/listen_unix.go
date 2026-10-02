//go:build !windows

package main

import (
	"fmt"
	"log"
	"net"
	"os"
	"path/filepath"
)

const defaultSocket = "/var/run/mtw-whatsapp/whatsapp.sock"

// listen opens the Unix socket mtw-server dials, replacing a stale file.
func listen(socketPath string) (net.Listener, error) {
	if err := os.MkdirAll(filepath.Dir(socketPath), 0o755); err != nil {
		return nil, fmt.Errorf("mkdir socket dir: %w", err)
	}
	_ = os.Remove(socketPath)
	l, err := net.Listen("unix", socketPath)
	if err != nil {
		return nil, fmt.Errorf("listen: %w", err)
	}
	if err := os.Chmod(socketPath, 0o666); err != nil {
		log.Printf("chmod socket: %v", err)
	}
	return l, nil
}

func dialEndpoint(ep string) (net.Conn, error) { return net.Dial("unix", ep) }
