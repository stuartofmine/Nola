// SPDX-License-Identifier: Apache-2.0
// Classifies OS transport evidence without treating ordinary offline as permission denial.
import Foundation
import Network

nonisolated enum MacNetworkFailure: Equatable {
    case localNetworkDenied
    case offline

    static func classify(_ error: Error) -> Self? {
        var pending = [error as NSError]
        var visited = Set<ObjectIdentifier>()
        var offline = false
        while let error = pending.popLast(), visited.count < 16 {
            guard visited.insert(ObjectIdentifier(error)).inserted else { continue }
            if error.domain == NSURLErrorDomain || error.domain == "kCFErrorDomainCFNetwork" {
                // URLSession's -1009 alone also means offline. Only an explicit
                // OS path policy reason establishes a local-network restriction.
                if let path = error.userInfo["_NSURLErrorNWPathKey"] as? NWPath,
                   path.status == .unsatisfied, path.unsatisfiedReason == .localNetworkDenied {
                    return .localNetworkDenied
                }
                // CFNetwork sometimes bridges its internal path as an opaque
                // Objective-C object rather than NWPath. Inspect only this OS
                // diagnostic field, never error prose, URLs, or provider text.
                if let path = error.userInfo["_NSURLErrorNWPathKey"],
                   String(describing: path).hasPrefix("unsatisfied (Local network prohibited)") {
                    return .localNetworkDenied
                }
                offline = offline || error.code == URLError.notConnectedToInternet.rawValue
            }
            if let underlying = error.userInfo[NSUnderlyingErrorKey] as? NSError { pending.append(underlying) }
        }
        return offline ? .offline : nil
    }

    static func message(for error: Error) -> String {
        switch classify(error) {
        case .localNetworkDenied:
            String(localized: "mac.network.local_denied", defaultValue: "macOS blocked access to the local network. Allow Nola in System Settings → Privacy & Security → Local Network, then retry.", table: "ModelCenter")
        case .offline:
            String(localized: "mac.network.offline", defaultValue: "The network is unavailable. Check your connection and the model server, then retry. This error alone does not indicate a local-network permission denial.", table: "ModelCenter")
        case nil:
            error.localizedDescription
        }
    }
}

/// The actual model discovery transport, injectable for offline URLProtocol tests.
nonisolated struct MacModelCatalogHTTPError: LocalizedError, Equatable {
    let statusCode: Int

    var errorDescription: String? {
        switch statusCode {
        case 401:
            String(localized: "Authentication failed (HTTP 401). Check the API key for this endpoint, then retry.", table: "ModelCenter")
        case 403:
            String(localized: "Access denied (HTTP 403). This credential cannot access the model catalog.", table: "ModelCenter")
        case 404, 405:
            String(localized: "Model discovery is unavailable at this URL. Check the base URL or enter a model ID manually.", table: "ModelCenter")
        default:
            String(format: String(localized: "Model catalog returned HTTP %d.", table: "ModelCenter"), statusCode)
        }
    }
}

nonisolated enum MacModelCatalogClient {
    static func catalogURL(_ baseURL: URL) -> URL {
        var value = baseURL
        if value.path.hasSuffix("/chat/completions") {
            value.deleteLastPathComponent(); value.deleteLastPathComponent()
        } else if value.path.hasSuffix("/responses") { value.deleteLastPathComponent() }
        return value.appendingPathComponent("models")
    }

    static func fetch(baseURL: URL, apiKey: String, session: URLSession = .shared) async throws -> [String] {
        var request = URLRequest(url: catalogURL(baseURL))
        request.timeoutInterval = 30
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if !apiKey.isEmpty { request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization") }
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }
        guard (200...299).contains(http.statusCode) else {
            // Do not echo arbitrary response bodies; gateways can include secrets.
            throw MacModelCatalogHTTPError(statusCode: http.statusCode)
        }
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let items = root["data"] as? [[String: Any]] else {
            throw NSError(domain: "MacModelCatalogClient", code: 1, userInfo: [NSLocalizedDescriptionKey:
                String(localized: "The provider returned an invalid model catalog.", table: "ModelCenter")])
        }
        return Array(Set(items.compactMap { $0["id"] as? String })).sorted()
    }
}
