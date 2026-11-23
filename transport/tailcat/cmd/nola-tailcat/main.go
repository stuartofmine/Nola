// SPDX-License-Identifier: Apache-2.0
// Private stdio adapter, not a daemon or a replacement Nola execution Host.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/signal"
	"strconv"
	"time"

	transport "nola.local/tailcattransport"
)

type control struct {
	ExpectedHostID string                `json:"expectedHostId,omitempty"`
	Version        int                   `json:"version"`
	Kind           string                `json:"kind"`
	Host           *transport.HostConfig `json:"host,omitempty"`
	Address        string                `json:"address,omitempty"`
	Relay          string                `json:"relay,omitempty"`
	ListenAddress  string                `json:"listenAddress,omitempty"`
}

func resolveListenAddress(raw string) (string, error) {
	if raw == "" {
		return "127.0.0.1:0", nil
	}
	host, portText, err := net.SplitHostPort(raw)
	if err != nil || host != "127.0.0.1" {
		return "", errors.New("invalid_listen_address")
	}
	port, err := strconv.ParseUint(portText, 10, 16)
	if err != nil {
		return "", errors.New("invalid_listen_address")
	}
	return net.JoinHostPort("127.0.0.1", strconv.FormatUint(port, 10)), nil
}

func main() { os.Exit(run()) }
func run() (code int) {
	for _, pipe := range []*os.File{os.Stdin, os.Stdout} {
		info, err := pipe.Stat()
		if err != nil || info.Mode()&os.ModeNamedPipe == 0 {
			fmt.Fprintln(os.Stderr, "Tailcat control requires private parent-owned stdin/stdout pipes; do not redirect credentials to a terminal or log")
			return 2
		}
	}
	// Neither addresses nor keys are command-line arguments or environment.
	if len(os.Args) != 1 {
		fmt.Fprintln(os.Stderr, "Use private stdio control, not command-line credentials")
		return 2
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	decoder := json.NewDecoder(io.LimitReader(os.Stdin, 64*1024))
	decoder.DisallowUnknownFields()
	encoder := json.NewEncoder(os.Stdout)
	frames := make(chan control)
	malformed := make(chan struct{}, 1)
	ownerExit := func() int {
		select {
		case <-malformed:
			return fail("control", "malformed_frame")
		default:
			return 0
		}
	}
	// Owner EOF cancels startup too; it must not leave a relay connection alive
	// until an unrelated startup timeout, or accept commands after owner exit.
	go func() {
		defer cancel()
		for {
			var frame control
			if err := decoder.Decode(&frame); err != nil {
				if !errors.Is(err, io.EOF) {
					malformed <- struct{}{}
				}
				return
			}
			select {
			case frames <- frame:
			case <-ctx.Done():
				return
			}
		}
	}()
	next := func() (control, bool) {
		select {
		case input := <-frames:
			return input, true
		case <-ctx.Done():
			return control{}, false
		}
	}
	asynchronousFailure := make(chan struct{}, 1)
	input, ok := next()
	if ok && input.Version != 1 {
		return fail("protocol_version", "unsupported_version")
	}
	if !ok {
		return ownerExit()
	}
	if input.Kind == "host" && input.Host != nil && input.Address == "" && input.Relay == "" && input.ExpectedHostID == "" && input.ListenAddress == "" {
		host, err := transport.NewHost(*input.Host)
		if err != nil {
			return fail("host_configuration", "invalid_host_config")
		}
		defer func() {
			if err := host.Close(); err != nil {
				code = fail("host_close", transport.ClassifyError(err))
			}
		}()
		startup, end := context.WithTimeout(ctx, 30*time.Second)
		err = host.Start(startup)
		end()
		if err != nil {
			if ctx.Err() != nil {
				return 0
			}
			return fail("host_start", transport.ClassifyError(err))
		}
		// This is credential-bearing private IPC. Never mirror to diagnostics.
		if encoder.Encode(map[string]any{"protocolVersion": 1, "event": "ready", "address": host.SecretAddress()}) != nil {
			return 1
		}
	} else if input.Kind == "client" && input.Host == nil && input.Address == "" && input.Relay == "" && input.ExpectedHostID == "" && input.ListenAddress == "" {
		client := transport.NewClient()
		defer func() {
			if err := client.Close(); err != nil {
				code = fail("client_close", transport.ClassifyError(err))
			}
		}()
		if encoder.Encode(map[string]any{"protocolVersion": 1, "event": "identity", "clientPublicKey": client.PublicKey()}) != nil {
			return 1
		}
		connect, ok := next()
		if !ok {
			return ownerExit()
		}
		if connect.Version != 1 || connect.Kind != "connect" || connect.Host != nil {
			return fail("client_control", "invalid_connect_frame")
		}
		listenAddr, err := resolveListenAddress(connect.ListenAddress)
		if err != nil {
			return fail("client_listener", "invalid_listen_address")
		}
		if client.Connect(connect.Address, connect.Relay) != nil {
			return fail("client_configuration", "invalid_client_config")
		}
		inspection, end := context.WithTimeout(ctx, 30*time.Second)
		err = client.VerifyHost(inspection, connect.ExpectedHostID)
		end()
		if err != nil {
			if ctx.Err() != nil {
				return 0
			}
			return fail("client_host_identity", transport.ClassifyError(err))
		}
		listener, err := net.Listen("tcp", listenAddr)
		if err != nil {
			return fail("client_listener", transport.ClassifyError(err))
		}
		defer listener.Close()
		go func() {
			if err := client.ServeLocal(listener); err != nil && ctx.Err() == nil {
				asynchronousFailure <- struct{}{}
				cancel()
			}
		}()
		if encoder.Encode(map[string]any{"protocolVersion": 1, "event": "ready", "origin": "http://" + listener.Addr().String()}) != nil {
			return 1
		}
	} else {
		return fail("mode", "invalid_mode")
	}
	command, ok := next()
	if !ok {
		select {
		case <-asynchronousFailure:
			return fail("client_listener", "listener_closed")
		default:
			return ownerExit()
		}
	}
	if command.Version != 1 || command.Kind != "stop" || command.Host != nil || command.Address != "" || command.Relay != "" || command.ExpectedHostID != "" || command.ListenAddress != "" {
		return fail("control", "invalid_stop_frame")
	}
	// Closing the tunnel never confirms cancellation of a Nola Run.
	cancel()
	return 0
}
func failEvent(stage, detail string) map[string]any {
	payload := map[string]any{"protocolVersion": 1, "event": "error", "stage": stage}
	if detail != "" {
		payload["detail"] = detail
	}
	return payload
}
func fail(stage, detail string) int {
	_ = json.NewEncoder(os.Stdout).Encode(failEvent(stage, detail))
	fmt.Fprintln(os.Stderr, "Tailcat transport failed; inspect the stage. No stop ACK is implied for a Nola Run.")
	return 1
}
