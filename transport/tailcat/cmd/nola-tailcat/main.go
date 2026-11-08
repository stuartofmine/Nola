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
			return fail("control")
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
		return fail("protocol_version")
	}
	if !ok {
		return ownerExit()
	}
	if input.Kind == "host" && input.Host != nil && input.Address == "" && input.Relay == "" && input.ExpectedHostID == "" {
		host, err := transport.NewHost(*input.Host)
		if err != nil {
			return fail("host_configuration")
		}
		defer func() {
			if err := host.Close(); err != nil {
				code = fail("host_close")
			}
		}()
		startup, end := context.WithTimeout(ctx, 30*time.Second)
		err = host.Start(startup)
		end()
		if err != nil {
			if ctx.Err() != nil {
				return 0
			}
			return fail("host_start")
		}
		// This is credential-bearing private IPC. Never mirror to diagnostics.
		if encoder.Encode(map[string]any{"protocolVersion": 1, "event": "ready", "address": host.SecretAddress()}) != nil {
			return 1
		}
	} else if input.Kind == "client" && input.Host == nil && input.Address == "" && input.Relay == "" && input.ExpectedHostID == "" {
		client := transport.NewClient()
		defer func() {
			if err := client.Close(); err != nil {
				code = fail("client_close")
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
			return fail("client_control")
		}
		if client.Connect(connect.Address, connect.Relay) != nil {
			return fail("client_configuration")
		}
		inspection, end := context.WithTimeout(ctx, 30*time.Second)
		err := client.VerifyHost(inspection, connect.ExpectedHostID)
		end()
		if err != nil {
			if ctx.Err() != nil {
				return 0
			}
			return fail("client_host_identity")
		}
		listener, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			return fail("client_listener")
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
		return fail("mode")
	}
	command, ok := next()
	if !ok {
		select {
		case <-asynchronousFailure:
			return fail("client_listener")
		default:
			return ownerExit()
		}
	}
	if command.Version != 1 || command.Kind != "stop" || command.Host != nil || command.Address != "" || command.Relay != "" || command.ExpectedHostID != "" {
		return fail("control")
	}
	// Closing the tunnel never confirms cancellation of a Nola Run.
	cancel()
	return 0
}
func fail(stage string) int {
	_ = json.NewEncoder(os.Stdout).Encode(map[string]any{"protocolVersion": 1, "event": "error", "stage": stage})
	fmt.Fprintln(os.Stderr, "Tailcat transport failed; inspect the stage. No stop ACK is implied for a Nola Run.")
	return 1
}
