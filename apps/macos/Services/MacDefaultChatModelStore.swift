// SPDX-License-Identifier: Apache-2.0
// Host-owned default chat preference. Resolution is performed by Rust Application.
import Foundation

nonisolated enum MacDefaultChatModelSelection: Equatable, Sendable, Codable {
    case automatic
    case profile(String)

    private enum CodingKeys: String, CodingKey { case kind; case profileID = "profile_id" }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "automatic": self = .automatic
        case "profile": self = .profile(try values.decode(String.self, forKey: .profileID))
        default: throw MacApplicationPortError.invalidResponse
        }
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .automatic: try values.encode("automatic", forKey: .kind)
        case let .profile(id):
            try values.encode("profile", forKey: .kind)
            try values.encode(id, forKey: .profileID)
        }
    }
}

nonisolated enum MacChatModelIntent: Equatable, Sendable, Encodable {
    case inherit
    case explicit(String)
    private enum CodingKeys: String, CodingKey { case kind; case profileID = "profile_id" }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .inherit: try values.encode("inherit", forKey: .kind)
        case let .explicit(id):
            try values.encode("explicit", forKey: .kind)
            try values.encode(id, forKey: .profileID)
        }
    }
}

nonisolated enum MacChatModelAvailability: String, Codable, Sendable {
    case available, disabled
    case notInstalled = "not_installed"
    case unsupportedRuntime = "unsupported_runtime"
}

nonisolated struct MacChatModelCandidate: Equatable, Sendable, Encodable {
    let profileID: String
    var allowedForChat: Bool = true
    var availability: MacChatModelAvailability = .available
    private enum CodingKeys: String, CodingKey {
        case profileID = "profile_id"
        case allowedForChat = "allowed_for_chat"
        case availability
    }
}

nonisolated enum MacChatModelOrigin: String, Codable, Sendable {
    case conversation, `default`, automatic
}

nonisolated enum MacChatModelUnavailableReason: String, Codable, Sendable {
    case missing, disabled
    case notInstalled = "not_installed"
    case unsupportedRuntime = "unsupported_runtime"
    case notAllowedForChat = "not_allowed_for_chat"
}

nonisolated enum MacChatModelResolution: Equatable, Sendable, Decodable {
    case resolved(profileID: String, origin: MacChatModelOrigin)
    case unavailable(profileID: String, origin: MacChatModelOrigin, reason: MacChatModelUnavailableReason)
    case noAutomaticCandidate
    case invalidConfiguration

    private enum CodingKeys: String, CodingKey { case kind, origin, reason; case profileID = "profile_id" }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "resolved":
            self = .resolved(profileID: try values.decode(String.self, forKey: .profileID),
                             origin: try values.decode(MacChatModelOrigin.self, forKey: .origin))
        case "unavailable":
            self = .unavailable(profileID: try values.decode(String.self, forKey: .profileID),
                                origin: try values.decode(MacChatModelOrigin.self, forKey: .origin),
                                reason: try values.decode(MacChatModelUnavailableReason.self, forKey: .reason))
        case "no_automatic_candidate": self = .noAutomaticCandidate
        case "invalid_configuration": self = .invalidConfiguration
        default: throw MacApplicationPortError.invalidResponse
        }
    }

    var profileID: String? {
        if case let .resolved(profileID, _) = self { return profileID }
        return nil
    }
}

nonisolated struct MacDefaultChatModelSnapshot: Equatable, Sendable {
    let selection: MacDefaultChatModelSelection
    let revision: Int64?
    let valueJSON: String?
}

/// Preserves the compare-and-swap token even when the stored configuration needs repair.
nonisolated struct MacDefaultChatModelRawSnapshot: Equatable, Sendable {
    let revision: Int64?
    let valueJSON: String?
}

extension Notification.Name {
    nonisolated static let macDefaultChatModelDidChange = Notification.Name("macDefaultChatModelDidChange")
}

nonisolated struct MacDefaultChatModelStore: Sendable {
    static let key = "chat.default_model"
    let application: MacApplicationPort

    init(application: MacApplicationPort = .shared) { self.application = application }

    private struct Configuration: Codable {
        let version: Int
        let selection: MacDefaultChatModelSelection
    }

    func rawSnapshot() throws -> MacDefaultChatModelRawSnapshot {
        let setting = try application.productSetting(key: Self.key)
        return .init(revision: setting?.revision, valueJSON: setting?.valueJSON)
    }

    func snapshot() throws -> MacDefaultChatModelSnapshot {
        let setting = try application.productSetting(key: Self.key)
        // Rust validates the version and strict tagged union before the UI decodes it.
        if try application.resolveChatModel(intent: .inherit, defaultValueJSON: setting?.valueJSON,
                                            candidates: []) == .invalidConfiguration {
            throw MacApplicationPortError.rejected("Invalid default chat model configuration.")
        }
        let selection: MacDefaultChatModelSelection
        if let setting {
            selection = try JSONDecoder().decode(Configuration.self, from: Data(setting.valueJSON.utf8)).selection
        } else { selection = .automatic }
        return .init(selection: selection, revision: setting?.revision, valueJSON: setting?.valueJSON)
    }

    @discardableResult
    func set(_ selection: MacDefaultChatModelSelection, expectedRevision: Int64?,
             candidates: [MacChatModelCandidate]) throws -> MacDefaultChatModelSnapshot {
        let data = try JSONEncoder().encode(Configuration(version: 1, selection: selection))
        let valueJSON = String(decoding: data, as: UTF8.self)
        let resolution = try application.resolveChatModel(intent: .inherit, defaultValueJSON: valueJSON, candidates: candidates)
        switch resolution {
        case .resolved, .noAutomaticCandidate: break
        case .unavailable: throw MacApplicationPortError.rejected("The selected default chat model is unavailable.")
        case .invalidConfiguration: throw MacApplicationPortError.rejected("Invalid default chat model configuration.")
        }
        _ = try application.putProductSettingChecked(key: Self.key, valueJSON: valueJSON,
            expectedRevision: expectedRevision, updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000))
        let saved = try snapshot()
        NotificationCenter.default.post(name: .macDefaultChatModelDidChange, object: application.databaseURL)
        return saved
    }

    func resolve(intent: MacChatModelIntent = .inherit,
                 candidates: [MacChatModelCandidate]) throws -> MacChatModelResolution {
        let setting = try application.productSetting(key: Self.key)
        return try application.resolveChatModel(intent: intent, defaultValueJSON: setting?.valueJSON, candidates: candidates)
    }
}
