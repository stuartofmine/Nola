// SPDX-License-Identifier: Apache-2.0
package nolatailcat

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	transport "nola.local/tailcattransport"
)

func TestSessionRefusesInvalidGrantAndUseAfterClose(t *testing.T) {
	session := NewSession()
	if session.PublicKey() == "" || session.Origin() != "" {
		t.Fatal("unconnected session advertised an endpoint")
	}
	if _, err := session.Connect("not-a-grant", "synthetic-host", "", 100); err == nil {
		t.Fatal("invalid grant admitted")
	}
	if session.Origin() != "" {
		t.Fatal("failed connection advertised readiness")
	}
	if err := session.Close(); err != nil {
		t.Fatal("unused transport did not close")
	}
	if _, err := session.Connect("not-a-grant", "synthetic-host", "", 100); err == nil {
		t.Fatal("closed session could be reused")
	}
}

// Explicit isolated transport check, never an app/real Nola Host assertion.
func TestNetworkMobileSessionOwnsEndpoint(t *testing.T) {
	if os.Getenv("NOLA_TAILCAT_NETWORK_TEST") != "1" {
		t.Skip("explicit isolated network validation only")
	}
	fixture := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/web/session" {
			_, _ = io.WriteString(w, `{"hostId":"mobile-fixture","transportIngress":"tailcat","authenticated":false,"localTrust":false}`)
			return
		}
		w.WriteHeader(http.StatusUnauthorized)
	}))
	defer fixture.Close()
	session := NewSession()
	defer session.Close()
	host, err := transport.NewHost(transport.HostConfig{Origin: fixture.URL, ExpectedHostID: "mobile-fixture", AllowedClientKeys: []string{session.PublicKey()}})
	if err != nil {
		t.Fatal("fixture configuration refused")
	}
	defer host.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 25*time.Second)
	defer cancel()
	if host.Start(ctx) != nil {
		t.Fatal("fixture transport unavailable")
	}
	origin, err := session.Connect(host.SecretAddress(), "mobile-fixture", "", 25000)
	if err != nil || origin == "" || session.Origin() != origin {
		t.Fatal("mobile transport did not verify Host and publish its endpoint")
	}
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 5 * time.Second}
	defer client.CloseIdleConnections()
	response, err := client.Get(origin + "/api/mobile/tasks")
	if err != nil {
		t.Fatal("mobile transport request failed")
	}
	_ = response.Body.Close()
	if response.StatusCode != http.StatusUnauthorized {
		t.Fatal("application authentication was lost")
	}
	if session.Close() != nil || session.Origin() != "" {
		t.Fatal("transport endpoint remained ready after close")
	}
	if response, err = client.Get(origin + "/api/mobile/tasks"); err == nil {
		_ = response.Body.Close()
		t.Fatal("closed endpoint still accepted requests")
	}
}
