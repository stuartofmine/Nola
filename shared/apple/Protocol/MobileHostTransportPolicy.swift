// SPDX-License-Identifier: Apache-2.0
import Foundation

/// The same versioned setup exported by the desktop app and `nola server`.
/// Pasting fills a draft; authentication and identity verification still follow.
nonisolated struct AppleHostConnectionSetup: Sendable {
    let origin: URL
    let hostID: String
    let accessToken: String

    static func decode(_ text: String) -> Self? {
        struct Payload: Decodable {
            let type: String
            let protocol_version: Int
            let origin: URL
            let host_id: String
            let access_token: String
        }
        guard text.utf8.count <= 32_768, let data = text.data(using: .utf8),
              let value = try? JSONDecoder().decode(Payload.self, from: data),
              ["nola.server.connection", "server.started"].contains(value.type),
              value.protocol_version == 1, MobileHostTransportPolicy.allows(value.origin),
              value.origin.query == nil, value.origin.fragment == nil,
              value.origin.path.isEmpty || value.origin.path == "/",
              !value.host_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !value.access_token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        return Self(origin: value.origin, hostID: value.host_id.trimmingCharacters(in: .whitespacesAndNewlines),
                    accessToken: value.access_token.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    static func sameOrigin(_ first: URL, _ second: URL) -> Bool {
        guard ["http", "https"].contains(first.scheme?.lowercased() ?? ""),
              first.host?.isEmpty == false, second.host?.isEmpty == false else { return false }
        func port(_ url: URL) -> Int? { url.port ?? (url.scheme?.lowercased() == "https" ? 443 : 80) }
        return first.scheme?.lowercased() == second.scheme?.lowercased()
            && first.host?.lowercased() == second.host?.lowercased() && port(first) == port(second)
    }
}

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
