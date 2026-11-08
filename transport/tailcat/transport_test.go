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
	}
}
