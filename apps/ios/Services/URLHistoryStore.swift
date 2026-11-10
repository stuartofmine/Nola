// SPDX-License-Identifier: Apache-2.0

import Foundation

final class URLHistoryStore: @unchecked Sendable {
    static let shared = URLHistoryStore()

    private let keyPrefix = "url.history."
    private let applicationRuntime: IOSApplicationRuntime
    private let decoder: JSONDecoder
    private let encoder: JSONEncoder

    private convenience init() {
        guard let store = IOSApplicationRuntime.shared else {
            preconditionFailure("Nola Application runtime is unavailable")
        }
        self.init(applicationRuntime: store)
    }

    init(applicationRuntime: IOSApplicationRuntime) {
        self.applicationRuntime = applicationRuntime
        decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
    }

    func append(sourceID: String, entryData: Data) -> Bool {
        guard let entry = try? decoder.decode(URLDataSource.HistoryEntry.self, from: entryData) else {
            return false
        }
        var entries = load(sourceID: sourceID)
        if !entries.contains(where: { $0.dedupeKey == entry.dedupeKey }) {
            entries.append(entry)
        }
        guard let data = try? encoder.encode(entries),
              let json = String(data: data, encoding: .utf8) else { return false }
        return (try? applicationRuntime.putProductSetting(
            key: keyPrefix + sourceID,
            valueJSON: json,
            updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000)
        )) != nil
    }

    func load(sourceID: String) -> [URLDataSource.HistoryEntry] {
        guard let setting = try? applicationRuntime.productSetting(key: keyPrefix + sourceID),
              let data = setting.valueJSON.data(using: .utf8),
              let entries = try? decoder.decode([URLDataSource.HistoryEntry].self, from: data) else {
            return []
        }
        return entries
    }
}
