// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated enum MobileHostTransportPolicy {
    static func allows(_ url: URL) -> Bool {
        guard url.user == nil,
              let scheme = url.scheme?.lowercased(),
              let host = url.host?.trimmingCharacters(in: CharacterSet(charactersIn: "[]")).lowercased(),
              !host.isEmpty else {
            return false
        }
        if scheme == "https" { return true }
        guard scheme == "http" else { return false }
        return isLocalHost(host)
    }

    private static func isLocalHost(_ host: String) -> Bool {
        if host == "localhost" || host == "localhost.localdomain" || host.hasSuffix(".localhost") {
            return true
        }
        if host == "::1" || host == "0:0:0:0:0:0:0:1" || host.hasSuffix(".local") {
            return true
        }
        if host.contains(":"), isPrivateIPv6(host) {
            return true
        }
        let labels = host.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count == 4 else { return false }
        let parts = labels.compactMap { Int($0) }
        guard parts.count == 4, parts.allSatisfy({ (0...255).contains($0) }) else {
            return false
        }
        return parts[0] == 10
            || parts[0] == 127
            || (parts[0] == 172 && (16...31).contains(parts[1]))
            || (parts[0] == 192 && parts[1] == 168)
            || (parts[0] == 169 && parts[1] == 254)
    }

    private static func isPrivateIPv6(_ host: String) -> Bool {
        let first = host.split(separator: ":", omittingEmptySubsequences: false).first.map(String.init) ?? ""
        guard let firstGroup = Int(first, radix: 16) else { return false }
        return (0xfc00...0xfdff).contains(firstGroup)
            || (0xfe80...0xfebf).contains(firstGroup)
    }
}
