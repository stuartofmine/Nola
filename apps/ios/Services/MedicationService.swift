// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MedicationService logic in the iOS service layer.
// - Interfaces:
//   - MedicationInfo
//   - MedicationService
// - APIs:
//   - none
//
//
//  MedicationService.swift
//  Nola
//
//  Simple medication lookup service. Uses a bundled local JSON as primary
//  source and exposes an async lookup API. Network fallback hooks are
//  provided but not enabled by default.
//

import Foundation

public struct MedicationInfo: Codable, Hashable {
    public let id: String
    public let name: String
    public let description: String
    public let commonDosage: String?
    public let warnings: String?
}

public final class MedicationService {
    private var localIndex: [String: MedicationInfo] = [:]
    private let logger: Logger?

    public init(logger: Logger? = nil) {
        self.logger = logger
        loadLocalIndex()
    }

    private func loadLocalIndex() {
        logger?.log(.info, "LoadLocalIndexService", NSLocalizedString("autogen.MedicationService.a563fd56", comment: "AUTOGEN"))
//        guard let url = Bundle.main.url(forResource: "medications", withExtension: "json") else {
//            logger?.log(.warn, "MedicationService", NSLocalizedString("autogen.MedicationService.9bb13bb1", comment: "AUTOGEN"))
//            return
//        }
//
//        do {
//            let data = try Data(contentsOf: url)
//            let arr = try JSONDecoder().decode([MedicationInfo].self, from: data)
//            for m in arr { localIndex[m.name.lowercased()] = m }
//            logger?.log(.info, "MedicationService", NSLocalizedString("autogen.MedicationService.80acdf56", comment: "AUTOGEN"), metadata: ["count": "\(arr.count)"])
//        } catch {
//            logger?.log(.error, "MedicationService", NSLocalizedString("autogen.MedicationService.096d64a7", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
//        }
    }

    /// Lookup medication by name (case-insensitive). First checks local index.
    public func lookupMedication(named name: String) async throws -> MedicationInfo? {
        let key = name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let found = localIndex[key] {
            return found
        }

        // Network fallback: attempt to lookup via OpenFDA label endpoint
        // Example: https://api.fda.gov/drug/label.json?search=openfda.brand_name:"aspirin"&limit=1
        let escaped = name.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? name
        let urlString = "https://api.fda.gov/drug/label.json?search=openfda.brand_name:\"\(escaped)\"&limit=1"
        guard let url = URL(string: urlString) else { return nil }

        let request = URLRequest(url: url, timeoutInterval: 10)
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let httpResponse = response as? HTTPURLResponse,
                  (200...299).contains(httpResponse.statusCode) else {
                logger?.log(.warn, "MedicationService", "openFDA lookup HTTP error", metadata: ["name": name])
                return nil
            }
            if let parsed = try parseOpenFDALabel(data: data, for: name) {
                // cache locally for future lookups
                localIndex[parsed.name.lowercased()] = parsed
                return parsed
            }
            return nil
        } catch {
            logger?.log(.warn, "MedicationService", "openFDA lookup failed", metadata: ["error": error.localizedDescription])
            return nil
        }
    }

    private func parseOpenFDALabel(data: Data, for name: String) throws -> MedicationInfo? {
        // Minimal parsing; OpenFDA returns `results` array where `openfda.brand_name` exists
        guard let json = try? JSONSerialization.jsonObject(with: data, options: []) as? [String: Any] else {
            return nil
        }
        guard let results = json["results"] as? [[String: Any]], let first = results.first else { return nil }
        // Extract brand name
        var brandName = name
        if let openfda = first["openfda"] as? [String: Any], let brands = openfda["brand_name"] as? [String], let b = brands.first {
            brandName = b
        }
        // Try to get description or purpose from `purpose` or `description` field
        var desc: String = ""
        if let purpose = first["purpose"] as? [String] { desc = purpose.joined(separator: " \n") }
        if desc.isEmpty, let descriptionField = first["description"] as? String { desc = descriptionField }

        let mi = MedicationInfo(id: UUID().uuidString, name: brandName, description: desc, commonDosage: nil, warnings: nil)
        return mi
    }
}
