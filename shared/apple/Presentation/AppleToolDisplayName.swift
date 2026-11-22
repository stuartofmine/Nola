// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated enum AppleToolDisplayName {
    private struct Catalog: Decodable {
        let aliases: [String: String]
        let labels: [String: [String: String]]
    }

    private static let catalog: Catalog? = {
        guard let url = Bundle.main.url(forResource: "tool-names", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(Catalog.self, from: data)
    }()

    static func localized(_ id: String, title: String? = nil, language: String? = nil) -> String? {
        let key = id.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().replacingOccurrences(of: "-", with: "_")
        guard let catalog else { return nil }
        let canonical = catalog.aliases[key] ?? key
        guard let labels = catalog.labels[canonical] else { return nil }
        if let title = title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty, title != "Tool" {
            let titleKey = title.lowercased().replacingOccurrences(of: "-", with: "_").replacingOccurrences(of: " ", with: "_")
            guard (catalog.aliases[titleKey] ?? titleKey) == canonical || labels.values.contains(title) else { return nil }
        }
        let preferred = language ?? Bundle.main.preferredLocalizations.first ?? "en"
        return labels[preferred] ?? labels["en"]
    }
}
