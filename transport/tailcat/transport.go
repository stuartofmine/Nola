// SPDX-License-Identifier: Apache-2.0
// Package tailcattransport carries the existing Nola protocol; it grants no
// application permissions, stores no keys, and never interprets Run completion.
package tailcattransport

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"syscall"

	"github.com/tailscale/tailcat"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
)

const ServicePort = 5001

// HostConfig contains no private identity. A host generates ephemeral transport
// keys only when explicitly started. Admission requires an explicit client key.
type HostConfig struct {
	Origin            string   `json:"origin"`
	ExpectedHostID    string   `json:"expectedHostId"`
	AllowedClientKeys []string `json:"allowedClientKeys"`
	DERPMapURL        string   `json:"derpMapUrl,omitempty"`
}

func localTarget(origin string) (string, error) {
	u, err := url.Parse(origin)
	if err != nil || u.Scheme != "http" || u.Hostname() != "127.0.0.1" || u.User != nil ||
		(u.Path != "" && u.Path != "/") || u.RawQuery != "" || u.Fragment != "" {
		return "", errors.New("Nola origin must be one exact http://127.0.0.1:port")
	}
	port, err := strconv.ParseUint(u.Port(), 10, 16)
	if err != nil || port == 0 {
		return "", errors.New("Nola origin requires a nonzero explicit port")
	}
	return net.JoinHostPort("127.0.0.1", strconv.Itoa(int(port))), nil
}

func validRelay(raw string) bool {
	if raw == "" {
		return true
	}
	u, err := url.Parse(raw)
	return err == nil && u.Scheme == "https" && u.Hostname() != "" && u.User == nil && u.RawQuery == "" && u.Fragment == ""
}

func checkHostStatus(body io.Reader, expected string) error {
	var status struct {
		Authenticated bool   `json:"authenticated"`
		LocalTrust    bool   `json:"localTrust"`
		Transport     string `json:"transportIngress"`
		HostID        string `json:"hostId"`
	}
	var raw json.RawMessage
	d := json.NewDecoder(io.LimitReader(body, 64*1024))
	if d.Decode(&raw) != nil || json.Unmarshal(raw, &status) != nil ||
		status.Transport != "tailcat" || status.LocalTrust || status.Authenticated || status.HostID != expected || expected == "" {
		return errors.New("Nola Host identity or authenticated ingress was not verified; tunnel not started")
	}
	// Missing booleans must not accidentally become a successful zero-value check.
	var fields map[string]json.RawMessage
	_ = json.Unmarshal(raw, &fields)
	if string(fields["localTrust"]) != "false" || string(fields["authenticated"]) != "false" {
		return errors.New("Nola Host did not explicitly refuse local trust")
	}
	return nil
}

// Host forwards only ServicePort to the verified Nola loopback endpoint.
// A tunnel address may contain PSK: no Stringer, logger or JSON serializer here.
type Host struct {
	server *tailcat.Server
	config HostConfig
	target string
	active *connections
}

func NewHost(config HostConfig) (*Host, error) {
	target, err := localTarget(config.Origin)
	if err != nil {
		return nil, err
	}
	if config.ExpectedHostID == "" || len(config.AllowedClientKeys) == 0 || !validRelay(config.DERPMapURL) {
		return nil, errors.New("explicit Host identity, allowed client keys and valid relay policy required")
	}
	allowed := make(map[key.NodePublic]bool)
	for _, raw := range config.AllowedClientKeys {
		var public key.NodePublic
		if public.UnmarshalText([]byte(raw)) != nil || public.IsZero() {
			return nil, errors.New("invalid allowed client public key")
		}
		allowed[public] = true
	}
	config.Origin = strings.TrimSuffix(config.Origin, "/")
	h := &Host{config: config, target: target, active: newConnections()}
	h.server = &tailcat.Server{Logf: logger.Discard, DERPMapURL: config.DERPMapURL,
		AllowClient: func(public key.NodePublic) bool { return allowed[public] }}
	h.server.OnTCP = func(port uint16) func(net.Conn) {
		if port != ServicePort {
			return nil
		}
		return func(conn net.Conn) {
			h.active.proxy(conn, func(ctx context.Context) (net.Conn, error) {
				return (&net.Dialer{}).DialContext(ctx, "tcp", h.target)
			})
		}
	}
	return h, nil
}

func (h *Host) Start(ctx context.Context) error {
	request, err := http.NewRequestWithContext(ctx, "GET", h.config.Origin+"/api/web/session", nil)
	if err != nil {
		return errors.New("invalid Nola ingress request")
	}
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	defer client.CloseIdleConnections()
	response, err := client.Do(request)
	if err != nil {
		return errors.New("Nola authenticated ingress is unavailable; tunnel not started")
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return errors.New("Nola ingress refused identity inspection")
	}
	if err := checkHostStatus(response.Body, h.config.ExpectedHostID); err != nil {
		return err
	}
	return h.server.StartContext(ctx)
}

// SecretAddress returns a credential for an explicit private IPC/share action.
// Call only after Start succeeds. Never include it in a normal URL or log.
func (h *Host) SecretAddress() string { return string(h.server.TailcatAddr()) }
func (h *Host) Close() error          { h.active.close(); return h.server.Close() }

// Client owns one ephemeral identity. PublicKey can be shared for Host admission;
// neither private key nor transport address is serialized by this type.
type Client struct {
	mu      sync.Mutex
	closed  bool
	private key.NodePrivate
	tunnel  *tailcat.Client
	active  *connections
}

func NewClient() *Client            { return &Client{private: key.NewNode(), active: newConnections()} }
func (c *Client) PublicKey() string { return c.private.Public().String() }
func (c *Client) Connect(address, relay string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed || c.tunnel != nil || !validRelay(relay) {
		return errors.New("invalid client lifecycle or relay policy")
	}
	info, err := tailcat.ParseAddr(tailcat.Addr(address))
	if err != nil || info.PresharedKey.IsZero() {
		return errors.New("invalid or non-PSK Tailcat address")
	}
	c.tunnel = &tailcat.Client{Server: tailcat.Addr(address), Key: c.private, Logf: logger.Discard, DERPMapURL: relay}
	return nil
}
func (c *Client) dial(ctx context.Context) (net.Conn, error) {
	if c.tunnel == nil {
		return nil, errors.New("client has no explicit server")
	}
	return c.tunnel.DialTCPPort(ctx, ServicePort)
}

// Classify only standard error types; raw URLs, peer addresses and provider
// messages can contain credentials and must not cross the native boundary.
func inspectionFailureCode(err error) string {
	var dns *net.DNSError
	var certificate x509.UnknownAuthorityError
	if errors.As(err, &dns) {
		return "dns"
	}
	if errors.As(err, &certificate) {
		return "certificate"
	}
	if errors.Is(err, syscall.EPERM) || errors.Is(err, syscall.EACCES) {
		return "permission"
	}
	var network net.Error
	if errors.Is(err, context.DeadlineExceeded) || errors.As(err, &network) && network.Timeout() {
		return "timeout"
	}
	return "network"
}

// VerifyHost completes the encrypted path and checks the application identity
// before a caller exposes a local endpoint as connected. It sends no credential.
func (c *Client) VerifyHost(ctx context.Context, expected string) error {
	if c.tunnel == nil || expected == "" {
		return errors.New("explicit expected Host identity required")
	}
	request, _ := http.NewRequestWithContext(ctx, "GET", "http://nola-transport.invalid/api/web/session", nil)
	transport := &http.Transport{Proxy: nil, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) { return c.dial(ctx) }}
	httpClient := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	defer httpClient.CloseIdleConnections()
	response, err := httpClient.Do(request)
	if err != nil {
		return errors.New("encrypted Nola Host inspection failed (" + inspectionFailureCode(err) + ")")
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return errors.New("remote Host inspection refused")
	}
	return checkHostStatus(response.Body, expected)
}

// ServeLocal is a thin desktop HTTP/SSE byte-stream bridge. It accepts only an
// already-created loopback listener, never a LAN/all-interface listener.
func (c *Client) ServeLocal(listener net.Listener) error {
	c.mu.Lock()
	ready := !c.closed && c.tunnel != nil
	c.mu.Unlock()
	address, ok := listener.Addr().(*net.TCPAddr)
	if !ok || !address.IP.IsLoopback() || !ready {
		return errors.New("client bridge requires an explicit loopback listener and server")
	}
	stop := context.AfterFunc(c.active.ctx, func() { _ = listener.Close() })
	defer stop()
	for {
		conn, err := listener.Accept()
		if err != nil {
			return err
		}
		go c.active.proxy(conn, c.dial)
	}
}
func (c *Client) Close() error {
	c.mu.Lock()
	c.closed = true
	tunnel := c.tunnel
	c.mu.Unlock()
	c.active.close()
	if tunnel != nil {
		return tunnel.Close()
	}
	return nil
}

// Tracks only this component's sockets. Closing transport does not cancel a Run.
type connections struct {
	mu     sync.Mutex
	closed bool
	ctx    context.Context
	cancel context.CancelFunc
	items  map[net.Conn]bool
	wg     sync.WaitGroup
}

func newConnections() *connections {
	ctx, cancel := context.WithCancel(context.Background())
	return &connections{ctx: ctx, cancel: cancel, items: make(map[net.Conn]bool)}
}
func (c *connections) add(conn net.Conn) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		_ = conn.Close()
		return false
	}
	c.items[conn] = true
	return true
}
func (c *connections) proxy(conn net.Conn, dial func(context.Context) (net.Conn, error)) {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		_ = conn.Close()
		return
	}
	c.items[conn] = true
	c.wg.Add(1)
	c.mu.Unlock()
	var remote net.Conn
	defer func() {
		_ = conn.Close()
		c.mu.Lock()
		delete(c.items, conn)
		if remote != nil {
			delete(c.items, remote)
		}
		c.mu.Unlock()
		c.wg.Done()
	}()
	var err error
	remote, err = dial(c.ctx)
	if err != nil || !c.add(remote) {
		return
	}
	tailcat.ProxyConns(conn, remote)
}
func (c *connections) close() {
	c.mu.Lock()
	c.closed = true
	c.cancel()
	for conn := range c.items {
		_ = conn.Close()
	}
	c.mu.Unlock()
	c.wg.Wait()
}
