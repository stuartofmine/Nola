// SPDX-License-Identifier: Apache-2.0
package nolatailcat

import "tailscale.com/net/netmon"

func init() {
	// Supported Tailscale app integration: Android Java APIs own enumeration.
	// Register once, before any Tailcat monitor starts; snapshots are synchronized.
	netmon.RegisterInterfaceGetter(platformNetworkInterfaces)
}
