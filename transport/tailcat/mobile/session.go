// SPDX-License-Identifier: Apache-2.0
// Package nolatailcat is a primitive-only binding over the shared transport.
// It persists neither keys nor endpoints; it owns only one foreground connection.
package nolatailcat

import (
	"context"
	"errors"
	"net"
	transport "nola.local/tailcattransport"
	"sync"
	"time"
)

type Session struct {
	mu         sync.Mutex
	client     *transport.Client
	cancel     context.CancelFunc
	connecting chan struct{}
	listener   net.Listener
	origin     string
	closed     bool
}

func NewSession() *Session           { return &Session{client: transport.NewClient()} }
func (s *Session) PublicKey() string { return s.client.PublicKey() }
func (s *Session) Origin() string    { s.mu.Lock(); defer s.mu.Unlock(); return s.origin }

// Connect blocks; native adapters must call on their I/O executor. The secret
// address and expected logical Host identity come from an explicit private grant.
func (s *Session) Connect(address, expectedHostID, relay string, timeoutMillis int64) (string, error) {
	if timeoutMillis <= 0 || timeoutMillis > int64((1<<63-1)/int64(time.Millisecond)) {
		return "", errors.New("invalid transport startup timeout")
	}
	s.mu.Lock()
	if s.closed || s.connecting != nil || s.origin != "" {
		s.mu.Unlock()
		return "", errors.New("transport session cannot be retargeted")
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMillis)*time.Millisecond)
	done := make(chan struct{})
	s.connecting = done
	s.cancel = cancel
	s.mu.Unlock()
	ready := false
	defer func() {
		cancel()
		if !ready {
			_ = s.client.Close()
		}
		close(done)
	}()
	if err := s.client.Connect(address, relay); err != nil {
		return "", err
	}
	if err := s.client.VerifyHost(ctx, expectedHostID); err != nil {
		return "", err
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return "", errors.New("local transport endpoint unavailable")
	}
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		_ = listener.Close()
		return "", errors.New("transport startup cancelled")
	}
	s.listener = listener
	s.origin = "http://" + listener.Addr().String()
	origin := s.origin
	s.mu.Unlock()
	ready = true
	go func() {
		_ = s.client.ServeLocal(listener)
		s.mu.Lock()
		s.origin = ""
		s.closed = true
		s.mu.Unlock()
		_ = s.client.Close()
	}()
	return origin, nil
}

// Close cancels startup and closes the local connection. It never sends a Nola
// cancellation or deletes a saved credential. Call on an I/O executor.
func (s *Session) Close() error {
	s.mu.Lock()
	s.closed = true
	s.origin = ""
	cancel := s.cancel
	done := s.connecting
	listener := s.listener
	s.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if listener != nil {
		_ = listener.Close()
	}
	if done != nil {
		<-done
	}
	return s.client.Close()
}
