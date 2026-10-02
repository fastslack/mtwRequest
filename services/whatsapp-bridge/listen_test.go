package main

import (
	"bufio"
	"net"
	"path/filepath"
	"runtime"
	"strconv"
	"testing"
	"time"
)

func testEndpoint(t *testing.T) string {
	if runtime.GOOS == "windows" {
		return `\\.\pipe\mtw-bridge-test-` + strconv.FormatInt(time.Now().UnixNano(), 10)
	}
	return filepath.Join(t.TempDir(), "bridge.sock")
}

// A peer that reconnects must find the endpoint accepting again (Review Focus 1).
func TestListenAcceptsSequentialClients(t *testing.T) {
	ep := testEndpoint(t)
	l, err := listen(ep)
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	defer l.Close()
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			go func(c net.Conn) {
				defer c.Close()
				line, _ := bufio.NewReader(c).ReadString('\n')
				_, _ = c.Write([]byte("echo:" + line))
			}(c)
		}
	}()
	for i := 0; i < 3; i++ {
		c, err := dialEndpoint(ep)
		if err != nil {
			t.Fatalf("dial %d: %v", i, err)
		}
		_, _ = c.Write([]byte("ping\n"))
		got, _ := bufio.NewReader(c).ReadString('\n')
		c.Close()
		if got != "echo:ping\n" {
			t.Fatalf("client %d got %q", i, got)
		}
	}
}
