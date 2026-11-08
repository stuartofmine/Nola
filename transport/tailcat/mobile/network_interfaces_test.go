// SPDX-License-Identifier: Apache-2.0
package nolatailcat

import "testing"

func TestAndroidInterfacesPreserveAddressesAndFailClosed(t *testing.T) {
	snapshot := `[{"name":"wlan0","index":3,"mtu":1500,"up":true,"multicast":true,"addresses":["192.0.2.7/24","2001:db8::7/64"]}]`
	if err := SetAndroidNetworkInterfaces(snapshot); err != nil {
		t.Fatal(err)
	}
	interfaces, err := platformNetworkInterfaces()
	if err != nil || len(interfaces) != 1 || !interfaces[0].IsUp() {
		t.Fatal("platform interface not available")
	}
	addresses, err := interfaces[0].Addrs()
	if err != nil || len(addresses) != 2 || addresses[0].String() != "192.0.2.7/24" || addresses[1].String() != "2001:db8::7/64" {
		t.Fatal("platform addresses changed or fell through to OS enumeration")
	}
	changed := `[{"name":"eth0","index":4,"mtu":1500,"up":true,"addresses":["198.51.100.9/24"]}]`
	if SetAndroidNetworkInterfaces(changed) != nil {
		t.Fatal("changed snapshot refused")
	}
	interfaces, err = platformNetworkInterfaces()
	if err != nil || len(interfaces) != 1 || interfaces[0].Name != "eth0" {
		t.Fatal("stale snapshot retained")
	}
	for _, invalid := range []string{`[]`, `null`, `invalid`, `[{"name":"wlan0","index":3,"up":true,"addresses":["not-an-address"]}]`, `[{"name":"lo","index":1,"up":true,"loopback":true,"addresses":["127.0.0.1/8"]}]`} {
		if SetAndroidNetworkInterfaces(invalid) == nil {
			t.Fatal("invalid or offline snapshot admitted")
		}
		if _, err := platformNetworkInterfaces(); err == nil {
			t.Fatal("previous snapshot survived failed refresh")
		}
	}
}
