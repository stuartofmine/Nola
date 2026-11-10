// SPDX-License-Identifier: Apache-2.0
import Foundation
import CoreFoundation

/// Host-resolved identities only. This layer never chooses among model candidates.
nonisolated enum AppleRemoteHostModelCatalog {
    enum Failure: LocalizedError {
        case invalidCatalog, unavailableDefault, changedDefault
        var errorDescription: String? {
            switch self {
            case .invalidCatalog: "Invalid model catalog from the paired Host."
            case .unavailableDefault: "The paired Host has no available default chat model. Configure it on the Host before sending."
            case .changedDefault: "The paired Host's default model changed while preparing this request. Try again."
            }
        }
    }
    struct Profile: Equatable, Sendable {
        let hostID: String
        let profileID: String
        let revision: UInt64
        let displayName: String
        let modelName: String
    }
    struct DefaultIntent: Equatable, Sendable {
        let profile: Profile
        let defaultRevision: Int64?
        let origin: String
        func payload(tier: String) throws -> [String: Any] {
            guard ["concise", "balanced", "detailed"].contains(tier) else { throw Failure.invalidCatalog }
            return ["profile_id": profile.profileID, "expected_revision": profile.revision,
                    "tier": tier, "overrides": [String: Any]()]
        }
    }
    static func profiles(settings: [String: Any], expectedHostID: String?) throws -> [Profile] {
        struct Record: Decodable {
            let profile_id: String
            let revision: UInt64
            let display_name: String
            let model_name: String
        }
        guard let hostID = (settings["host_identity"] as? [String: Any])?["id"] as? String,
              !hostID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              expectedHostID == nil || expectedHostID == hostID,
              let records = settings["model_profiles"] as? [[String: Any]] else { throw Failure.invalidCatalog }
        let decoded = try JSONDecoder().decode([Record].self, from: JSONSerialization.data(withJSONObject: records))
        guard Set(decoded.map(\.profile_id)).count == decoded.count,
              decoded.allSatisfy({ !$0.profile_id.isEmpty && $0.revision > 0 && !$0.model_name.isEmpty }) else {
            throw Failure.invalidCatalog
        }
        return decoded.map { Profile(hostID: hostID, profileID: $0.profile_id, revision: $0.revision,
                                     displayName: $0.display_name, modelName: $0.model_name) }
    }
    static func freezeDefault(settings: [String: Any], expectedHostID: String) throws -> DefaultIntent {
        let profiles = try profiles(settings: settings, expectedHostID: expectedHostID)
        guard let resolution = settings["default_chat_model_resolution"] as? [String: Any],
              resolution["kind"] as? String == "resolved",
              let id = resolution["profile_id"] as? String,
              let origin = resolution["origin"] as? String, ["default", "automatic"].contains(origin),
              let profile = profiles.first(where: { $0.profileID == id }) else { throw Failure.unavailableDefault }
        let revision: Int64?
        if let value = settings["default_chat_model_revision"], !(value is NSNull) {
            guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
                  number.int64Value > 0, number.doubleValue == Double(number.int64Value) else { throw Failure.invalidCatalog }
            revision = number.int64Value
        } else { revision = nil }
        return DefaultIntent(profile: profile, defaultRevision: revision, origin: origin)
    }
    static func validateUnchanged(_ frozen: DefaultIntent, settings: [String: Any]) throws {
        let current = try freezeDefault(settings: settings, expectedHostID: frozen.profile.hostID)
        guard current.profile.profileID == frozen.profile.profileID,
              current.profile.revision == frozen.profile.revision,
              current.defaultRevision == frozen.defaultRevision, current.origin == frozen.origin else {
            throw Failure.changedDefault
        }
    }
}
