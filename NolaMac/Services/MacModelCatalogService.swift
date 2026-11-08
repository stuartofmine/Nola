// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Discovers OpenAI-compatible models and describes the CLI's effective global configuration.

import Foundation
import Observation

@MainActor
@Observable
final class MacModelCatalogService {
    static let shared = MacModelCatalogService()
    private static let refreshTTL: TimeInterval = 3 * 60

    var models: [String] = []
    var isLoading = false
    var errorMessage: String?
    var lastCheckedAt: Date?

    func refresh(force: Bool = true) async {
        guard !isLoading else { return }
        if !force,
           !models.isEmpty,
           let lastCheckedAt,
           Date().timeIntervalSince(lastCheckedAt) < Self.refreshTTL {
            return
        }
        guard let configuration = MacAppleChatConfigurationStore.shared.load() else {
            errorMessage = String(
                localized: "Configure Chat Only before loading its model catalog.",
                table: "ModelCenter"
            )
            return
        }
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }

        do {
            var request = URLRequest(url: configuration.baseURL.appendingPathComponent("models"))
            request.httpMethod = "GET"
            request.timeoutInterval = 30
            request.setValue("application/json", forHTTPHeaderField: "Accept")
            if !configuration.apiKey.isEmpty {
                request.setValue("Bearer \(configuration.apiKey)", forHTTPHeaderField: "Authorization")
            }
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw URLError(.badServerResponse)
            }
            guard (200...299).contains(http.statusCode) else {
                let message = String(decoding: data.prefix(1_000), as: UTF8.self)
                throw NSError(
                    domain: "MacModelCatalogService",
                    code: http.statusCode,
                    userInfo: [
                        NSLocalizedDescriptionKey: message.isEmpty
                            ? String(
                                format: String(localized: "Model catalog returned HTTP %d.", table: "ModelCenter"),
                                http.statusCode
                            )
                            : message
                    ]
                )
            }
            guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let items = root["data"] as? [[String: Any]] else {
                throw NSError(
                    domain: "MacModelCatalogService",
                    code: 1,
                    userInfo: [
                        NSLocalizedDescriptionKey: String(
                            localized: "The provider returned an invalid model catalog.",
                            table: "ModelCenter"
                        )
                    ]
                )
            }
            models = Array(Set(items.compactMap { $0["id"] as? String }.filter { !$0.isEmpty })).sorted()
            lastCheckedAt = .now
        } catch {
            models = []
            errorMessage = error.localizedDescription
            lastCheckedAt = .now
        }
    }

}
