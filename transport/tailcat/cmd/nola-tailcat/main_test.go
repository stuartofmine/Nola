// SPDX-License-Identifier: Apache-2.0
package main

import "testing"

func TestResolveListenAddressLoopbackOnly(t *testing.T) {
	valid := map[string]string{
		"":                "127.0.0.1:0",
		"127.0.0.1:0":     "127.0.0.1:0",
		"127.0.0.1:54321": "127.0.0.1:54321",
	}
	for input, want := range valid {
		got, err := resolveListenAddress(input)
		if err != nil || got != want {
			t.Fatalf("resolveListenAddress(%q) = (%q, %v), want (%q, nil)", input, got, err, want)
		}
	}
	for _, invalid := range []string{
		"0.0.0.0:54321",
		"localhost:54321",
		"[::1]:54321",
		"127.0.0.1:99999",
		"127.0.0.1",
		"192.168.1.2:5001",
	} {
		if got, err := resolveListenAddress(invalid); err == nil {
			t.Fatalf("resolveListenAddress(%q) unexpectedly succeeded with %q", invalid, got)
		}
	}
}

func TestFailEventIncludesSafeStageAndOptionalDetail(t *testing.T) {
	withDetail := failEvent("client_host_identity", "timeout")
	if withDetail["protocolVersion"] != 1 || withDetail["event"] != "error" ||
		withDetail["stage"] != "client_host_identity" || withDetail["detail"] != "timeout" {
		t.Fatalf("unexpected fail event with detail: %#v", withDetail)
	}
	withoutDetail := failEvent("host_start", "")
	if _, ok := withoutDetail["detail"]; ok {
		t.Fatalf("empty detail should be omitted: %#v", withoutDetail)
	}
}
