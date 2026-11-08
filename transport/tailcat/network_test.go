// SPDX-License-Identifier: Apache-2.0
package tailcattransport

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"
)

// Explicit opt-in only: uses ephemeral in-memory keys and public DERP, never
// a user's Host, accounts, device keys, model provider or filesystem workspace.
// The HTTP origin is a protocol fixture, not a real Nola execution Host.
func TestNetworkAuthenticatedBytesAndSSE(t *testing.T) {
	if os.Getenv("NOLA_TAILCAT_NETWORK_TEST") != "1" {
		t.Skip("explicit isolated network validation only")
	}
	release := make(chan struct{})
	defer closeIfOpen(release)
	fixture := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/web/session" {
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{"hostId": "isolated-host", "transportIngress": "tailcat", "authenticated": false, "localTrust": false})
			return
		}
		if r.Header.Get("Authorization") != "Bearer synthetic-fixture-token" {
			w.WriteHeader(401)
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "event: delta\ndata: first\n\n")
		w.(http.Flusher).Flush()
		select {
		case <-release:
		case <-r.Context().Done():
			return
		}
		_, _ = io.WriteString(w, "event: done\ndata: final\n\n")
		w.(http.Flusher).Flush()
	}))
	defer fixture.Close()
	client := NewClient()
	defer client.Close()
	host, err := NewHost(HostConfig{Origin: fixture.URL, ExpectedHostID: "isolated-host", AllowedClientKeys: []string{client.PublicKey()}})
	if err != nil {
		t.Fatal("fixture configuration refused")
	}
	defer host.Close()
	startup, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	if err = host.Start(startup); err != nil {
		t.Fatal("network host startup failed (address/credentials withheld)")
	}
	if client.Connect(host.SecretAddress(), "") != nil {
		t.Fatal("client address refused")
	}
	if client.VerifyHost(startup, "isolated-host") != nil {
		t.Fatal("encrypted Host identity was not verified")
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal("isolated client listener unavailable")
	}
	defer listener.Close()
	serving := make(chan error, 1)
	go func() { serving <- client.ServeLocal(listener) }()
	httpClient := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: 25 * time.Second}
	defer httpClient.CloseIdleConnections()
	endpoint := "http://" + listener.Addr().String() + "/api/fixture"
	denied, err := httpClient.Get(endpoint)
	if err != nil {
		t.Fatal("encrypted transport request failed")
	}
	_ = denied.Body.Close()
	if denied.StatusCode != 401 {
		t.Fatal("application authorization was bypassed")
	}
	request, _ := http.NewRequest("GET", endpoint, nil)
	request.Header.Set("Authorization", "Bearer synthetic-fixture-token")
	response, err := httpClient.Do(request)
	if err != nil {
		t.Fatal("authenticated stream failed")
	}
	reader := bufio.NewReader(response.Body)
	line, err := reader.ReadString('\n')
	if err != nil || line != "event: delta\n" {
		t.Fatal("first frame did not arrive before completion")
	}
	close(release)
	rest, err := io.ReadAll(reader)
	_ = response.Body.Close()
	if err != nil || string(rest) != "data: first\n\nevent: done\ndata: final\n\n" {
		t.Fatal("SSE bytes were changed or lost")
	}
	if client.Close() != nil {
		t.Fatal("client close failed")
	}
	select {
	case <-serving:
	case <-time.After(2 * time.Second):
		t.Fatal("client listener stayed alive")
	}
	if host.Close() != nil {
		t.Fatal("host close failed")
	}
	t.Log("real WireGuard/DERP transport: HTTP401 retained, authenticated SSE incremental bytes exact, client listener closed; protocol fixture, not production Nola Host")
}
func closeIfOpen(done chan struct{}) {
	select {
	case <-done:
	default:
		close(done)
	}
}
