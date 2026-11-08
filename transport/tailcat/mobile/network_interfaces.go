// SPDX-License-Identifier: Apache-2.0
package nolatailcat

import (
	"encoding/json"
	"errors"
	"net"
	"sync"

	"tailscale.com/net/netmon"
)

// Android supplies an actual platform snapshot; this is not a synthetic route
// or a permission bypass. A missing/failed snapshot must fail initialization.
var androidInterfaces struct {
	sync.RWMutex
	values []netmon.Interface
}

type platformInterface struct {
	Name         string   `json:"name"`
	Index        int      `json:"index"`
	MTU          int      `json:"mtu"`
	Up           bool     `json:"up"`
	Loopback     bool     `json:"loopback"`
	Multicast    bool     `json:"multicast"`
	PointToPoint bool     `json:"pointToPoint"`
	Addresses    []string `json:"addresses"`
}

func parsePlatformInterfaces(snapshot string) ([]netmon.Interface, error) {
	var records []platformInterface
	if json.Unmarshal([]byte(snapshot), &records) != nil || len(records) == 0 {
		return nil, errors.New("Android network interfaces unavailable")
	}
	result := make([]netmon.Interface, 0, len(records))
	addressed := false
	indexes := map[int]bool{}
	for _, record := range records {
		if record.Name == "" || record.Index <= 0 || record.MTU < 0 || indexes[record.Index] {
			return nil, errors.New("invalid Android network interface")
		}
		indexes[record.Index] = true
		var flags net.Flags
		if record.Up {
			flags |= net.FlagUp | net.FlagRunning
		}
		if record.Loopback {
			flags |= net.FlagLoopback
		}
		if record.Multicast {
			flags |= net.FlagMulticast
		}
		if record.PointToPoint {
			flags |= net.FlagPointToPoint
		}
		addresses := make([]net.Addr, 0, len(record.Addresses))
		for _, address := range record.Addresses {
			ip, prefix, err := net.ParseCIDR(address)
			if err != nil {
				return nil, errors.New("invalid Android interface address")
			}
			// Preserve the assigned host address, not only its network prefix.
			prefix.IP = ip
			addresses = append(addresses, prefix)
			if record.Up && !record.Loopback && !ip.IsUnspecified() {
				addressed = true
			}
		}
		result = append(result, netmon.Interface{
			Interface: &net.Interface{Name: record.Name, Index: record.Index, MTU: record.MTU, Flags: flags},
			AltAddrs:  addresses,
		})
	}
	if !addressed {
		return nil, errors.New("Android network has no active interface address")
	}
	return result, nil
}

// SetAndroidNetworkInterfaces replaces the process-local snapshot. Invalid input
// clears the previous snapshot so a stale interface cannot masquerade as ready.
// Native callers refresh before admission and on platform network callbacks.
func SetAndroidNetworkInterfaces(snapshot string) error {
	values, err := parsePlatformInterfaces(snapshot)
	androidInterfaces.Lock()
	androidInterfaces.values = values
	androidInterfaces.Unlock()
	return err
}

func platformNetworkInterfaces() ([]netmon.Interface, error) {
	androidInterfaces.RLock()
	defer androidInterfaces.RUnlock()
	if len(androidInterfaces.values) == 0 {
		return nil, errors.New("Android network interfaces unavailable")
	}
	return append([]netmon.Interface(nil), androidInterfaces.values...), nil
}
