// SPDX-License-Identifier: Apache-2.0
package tailcattransport

import (
	"context"
	"crypto/x509"
	"github.com/tailscale/tailcat"
	"io"
	"net"
	"net/url"
	"strings"
	"syscall"
	"tailscale.com/types/key"
	"testing"
	"time"
)

func TestAdmissionAndIdentityWithoutNetwork(t *testing.T) {
	client := NewClient()
	defer client.Close()
	config := HostConfig{Origin: "http://127.0.0.1:5001/", ExpectedHostID: "fixture-host", AllowedClientKeys: []string{client.PublicKey()}}
	host, err := NewHost(config)
	if err != nil {
		t.Fatal(err)
	}
	defer host.Close()
	if !host.server.AllowClient(client.private.Public()) || host.server.AllowClient(key.NewNode().Public()) {
		t.Fatal("client admission widened")
	}
	if host.server.OnTCP(ServicePort+1) != nil {
		t.Fatal("unrequested service exposed")
	}
	for _, origin := range []string{"http://localhost:5001", "http://127.0.0.1", "http://127.0.0.1:5001/path", "https://127.0.0.1:5001", "http://user@127.0.0.1:5001"} {
		config.Origin = origin
		if _, err := NewHost(config); err == nil {
			t.Fatal("invalid origin admitted")
		}
	}
	config.Origin = "http://127.0.0.1:5001"
	config.AllowedClientKeys = nil
	if _, err := NewHost(config); err == nil {
		t.Fatal("empty allowlist became allow-all")
	}
	valid := `{"hostId":"fixture-host","transportIngress":"tailcat","authenticated":false,"localTrust":false}`
	if err := checkHostStatus(strings.NewReader(valid), "fixture-host"); err != nil {
		t.Fatal(err)
	}
	for _, status := range []string{strings.Replace(valid, `"localTrust":false`, `"localTrust":true`, 1), strings.Replace(valid, `"authenticated":false,`, "", 1), valid} {
		expected := "fixture-host"
		if status == valid {
			expected = "different-host"
		}
		if checkHostStatus(strings.NewReader(status), expected) == nil {
			t.Fatal("unverified ingress admitted")
		}
	}
	info := tailcat.ConnInfo{ServerPublic: tailcat.NodePublic{NodePublic: key.NewNode().Public()}, ServerDiscoPublic: tailcat.DiscoPublic{DiscoPublic: key.NewDisco().Public()}, PresharedKey: tailcat.NewPresharedKey(), RegionID: 303}
	if err := client.Connect(string(info.Addr()), ""); err != nil {
		t.Fatal(err)
	}
	if err := client.Connect(string(info.Addr()), ""); err == nil {
		t.Fatal("client silently retargeted")
	}
}

func TestStreamAndCloseUseOnlyOwnedInMemoryConnections(t *testing.T) {
	state := newConnections()
	a, b := net.Pipe()
	c, d := net.Pipe()
	done := make(chan struct{})
	go func() { state.proxy(b, func(context.Context) (net.Conn, error) { return c, nil }); close(done) }()
	expected := "event: text\ndata: delta\n\n"
	go func() { _, _ = io.WriteString(a, expected) }()
	got := make([]byte, len(expected))
	if _, err := io.ReadFull(d, got); err != nil || string(got) != expected {
		t.Fatal("stream bytes changed")
	}
	// No TCP/UDP or third-party call occurs in this cancellation check.
	state.close()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("owned stream did not close")
	}
	_ = a.Close()
	_ = d.Close()
}

func TestInspectionFailureCodesNeverIncludeCredentialBearingErrorText(t *testing.T) {
	cases := []struct {
		err  error
		code string
	}{
		{&url.Error{Op: "Get", URL: "https://fixture-secret-address", Err: &net.DNSError{Name: "private-peer", Err: "not found"}}, "dns"},
		{x509.UnknownAuthorityError{}, "certificate"},
		{&net.OpError{Op: "dial", Net: "tcp", Err: syscall.EPERM}, "permission"},
		{context.DeadlineExceeded, "timeout"},
	}
	for _, example := range cases {
		if got := inspectionFailureCode(example.err); got != example.code {
			t.Fatalf("unexpected bounded failure code %q", got)
		}
		if got := ClassifyError(example.err); got != example.code {
			t.Fatalf("unexpected classified failure code %q", got)
		}
	}
	if got := ClassifyError(checkHostStatus(strings.NewReader(`{"hostId":"other","transportIngress":"tailcat","authenticated":false,"localTrust":false}`), "expected")); got != "host_identity_mismatch" {
		t.Fatalf("expected host_identity_mismatch, got %q", got)
	}
	if got := ClassifyError(checkHostStatus(strings.NewReader(`{"hostId":"expected","transportIngress":"tailcat"}`), "expected")); got != "local_trust_enabled" {
		t.Fatalf("expected local_trust_enabled, got %q", got)
	}
}

func TestHostAndClientCloseDoNotDeadlockWithActiveConnections(t *testing.T) {
	client := NewClient()
	config := HostConfig{
		Origin:            "http://127.0.0.1:5001",
		ExpectedHostID:    "fixture-host",
		AllowedClientKeys: []string{client.PublicKey()},
	}
	host, err := NewHost(config)
	if err != nil {
		t.Fatal(err)
	}

	// Simulate an active proxied connection on Host.
	a1, b1 := net.Pipe()
	c1, d1 := net.Pipe()
	defer a1.Close()
	defer d1.Close()
	go host.active.proxy(b1, func(context.Context) (net.Conn, error) { return c1, nil })
	time.Sleep(20 * time.Millisecond)

	hostDone := make(chan error, 1)
	go func() { hostDone <- host.Close() }()
	select {
	case <-hostDone:
	case <-time.After(3 * time.Second):
		t.Fatal("Host.Close() deadlocked with active connection")
	}

	// Simulate an active proxied connection on Client.
	a2, b2 := net.Pipe()
	c2, d2 := net.Pipe()
	defer a2.Close()
	defer d2.Close()
	go client.active.proxy(b2, func(context.Context) (net.Conn, error) { return c2, nil })
	time.Sleep(20 * time.Millisecond)

	clientDone := make(chan error, 1)
	go func() { clientDone <- client.Close() }()
	select {
	case <-clientDone:
	case <-time.After(3 * time.Second):
		t.Fatal("Client.Close() deadlocked with active connection")
	}
}

func TestConnectionsCloseWaitIsBoundedOnStalledDial(t *testing.T) {
	state := newConnections()
	a, b := net.Pipe()
	defer a.Close()
	unblock := make(chan struct{})
	defer close(unblock)

	// Dial ignores ctx cancellation to simulate a wedged socket/syscall.
	go state.proxy(b, func(context.Context) (net.Conn, error) {
		<-unblock
		return nil, io.EOF
	})
	time.Sleep(20 * time.Millisecond)

	start := time.Now()
	state.beginClose()
	if state.wait(80 * time.Millisecond) {
		t.Fatal("expected bounded wait to time out on wedged dial")
	}
	if time.Since(start) > time.Second {
		t.Fatal("bounded close wait took too long")
	}
}

func TestBidirectionalIdlePairKeepsUnidirectionalSSEAlive(t *testing.T) {
	c1, s1 := net.Pipe()
	c2, s2 := net.Pipe()
	defer c1.Close()
	defer s2.Close()

	wrappedA, wrappedB, stop := newBidirectionalIdlePair(s1, c2, 80*time.Millisecond)
	defer stop()
	go tailcat.ProxyConns(wrappedA, wrappedB)

	// Push server->client SSE heartbeats every 25ms for 150ms (> 80ms timeout)
	// while the client sends ZERO bytes; bidirectional activity must keep it open!
	done := make(chan error, 1)
	go func() {
		buf := make([]byte, 5)
		for i := 0; i < 6; i++ {
			if _, err := io.ReadFull(c1, buf); err != nil {
				done <- err
				return
			}
		}
		done <- nil
	}()
	for i := 0; i < 6; i++ {
		if _, err := io.WriteString(s2, "ping\n"); err != nil {
			t.Fatalf("SSE write %d failed prematurely: %v", i, err)
		}
		time.Sleep(25 * time.Millisecond)
	}
	if err := <-done; err != nil {
		t.Fatalf("SSE read failed prematurely: %v", err)
	}
}
