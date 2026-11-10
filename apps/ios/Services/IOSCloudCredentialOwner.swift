import Foundation
import Security

nonisolated public enum IOSCloudCredentialBinding: Codable, Hashable, Sendable {
    case anonymous
    case keychain(account: String)

    var account: String? {
        if case .keychain(let account) = self { return account }
        return nil
    }
}

nonisolated enum IOSCloudCredentialError: LocalizedError {
    case unavailable, invalidReference, storageFailed, publicationRejected, publicationUnconfirmed
    var errorDescription: String? {
        switch self {
        case .unavailable: return "The saved cloud credential is unavailable. Reconfigure this model."
        case .invalidReference: return "The cloud model has no valid credential reference. Reconfigure this model."
        case .storageFailed: return "Could not save the API key securely."
        case .publicationRejected: return "The model profile changed before its credential could be saved."
        case .publicationUnconfirmed: return "The credential publication could not be confirmed. Reload the model profile before retrying."
        }
    }
}

/// New secrets get unique slots. Only an acknowledged rejection permits cleanup;
/// an uncertain response may already have published the reference durably.
nonisolated struct IOSCloudCredentialOwner: Sendable {
    private struct AccountSecret: Codable {
        let connectionID: String
        let endpoint: String
        let protocols: [String]
        let secret: String
    }

    func storeAccountSecret(_ secret: String, connectionID: String, endpoint: String, protocols: [String]) throws -> String {
        let reference = "account:" + UUID().uuidString.lowercased()
        let payload = AccountSecret(connectionID: connectionID, endpoint: endpoint, protocols: protocols, secret: secret)
        guard !secret.isEmpty, save(reference, try JSONEncoder().encode(payload)) else { throw IOSCloudCredentialError.storageFailed }
        return reference
    }

    func resolveAccountSecret(_ reference: String, connectionID: String, endpoint: String, wireProtocol: String) throws -> String {
        guard reference.hasPrefix("account:"), let data = read(reference),
              let bound = try? JSONDecoder().decode(AccountSecret.self, from: data),
              bound.connectionID == connectionID, bound.endpoint == endpoint,
              bound.protocols.contains(wireProtocol), !bound.secret.isEmpty else { throw IOSCloudCredentialError.unavailable }
        return bound.secret
    }
    private struct BoundSecret: Codable {
        let profileID: String
        let endpoint: String
        let wireProtocol: ModelWireProtocol
        let model: String
        let secret: String
    }
    enum Publication { case committed(revision: Int64), rejected, unconfirmed }
    static let service = "nola.cloud-model"
    static let shared = Self(
        save: { account, data in
            let item: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: account,
                kSecValueData as String: data,
                kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            ]
            // Never update an existing version, even in the event of a collision.
            return SecItemAdd(item as CFDictionary, nil) == errSecSuccess
        },
        read: { KeychainService.read(service: service, account: $0) },
        delete: { _ = KeychainService.delete(service: service, account: $0) })

    let save: @Sendable (String, Data) -> Bool
    let read: @Sendable (String) -> Data?
    let delete: @Sendable (String) -> Void

    func resolve(model: ModelInfo) throws -> String? {
        guard let binding = model.cloudCredential else { throw IOSCloudCredentialError.invalidReference }
        switch binding {
        case .anonymous: return nil
        case .keychain(let account):
            guard validAccount(account) else { throw IOSCloudCredentialError.invalidReference }
            guard let data = read(account), let value = try? JSONDecoder().decode(BoundSecret.self, from: data),
                  value.profileID == model.id, value.endpoint == (try endpoint(model)),
                  value.wireProtocol == (model.wireProtocol ?? .chatCompletions),
                  value.model == model.effectiveUpstreamModelId, !value.secret.isEmpty else {
                throw IOSCloudCredentialError.unavailable
            }
            return value.secret
        }
    }

    func publish(model: ModelInfo, secret: String, commit: (IOSCloudCredentialBinding) throws -> Publication) throws -> (IOSCloudCredentialBinding, Int64) {
        let endpoint = try endpoint(model)
        let account = secret.isEmpty ? nil : "version:\(UUID().uuidString.lowercased())"
        let binding = account.map { IOSCloudCredentialBinding.keychain(account: $0) } ?? .anonymous
        if let account {
            let payload = BoundSecret(profileID: model.id, endpoint: endpoint,
                wireProtocol: model.wireProtocol ?? .chatCompletions, model: model.effectiveUpstreamModelId, secret: secret)
            guard save(account, try JSONEncoder().encode(payload)) else { throw IOSCloudCredentialError.storageFailed }
        }
        // A thrown transport error is uncertain: retain the unique slot for reconciliation.
        switch try commit(binding) {
        case .committed(let revision):
            guard revision > 0 else { throw IOSCloudCredentialError.publicationUnconfirmed }
            return (binding, revision)
        case .rejected:
            if let account { delete(account) }
            throw IOSCloudCredentialError.publicationRejected
        case .unconfirmed: throw IOSCloudCredentialError.publicationUnconfirmed
        }
    }

    private func validAccount(_ account: String) -> Bool {
        guard account.hasPrefix("version:"), let id = UUID(uuidString: String(account.dropFirst(8))) else { return false }
        return account == "version:\(id.uuidString.lowercased())"
    }

    private func endpoint(_ model: ModelInfo) throws -> String {
        guard let raw = model.baseURL, RemoteModelEndpointURL.isValid(raw),
              var components = URLComponents(string: raw.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            throw IOSCloudCredentialError.invalidReference
        }
        components.scheme = components.scheme?.lowercased()
        components.host = components.host?.lowercased()
        if (components.scheme == "https" && components.port == 443) ||
            (components.scheme == "http" && components.port == 80) { components.port = nil }
        guard let endpoint = components.string else { throw IOSCloudCredentialError.invalidReference }
        return endpoint
    }
}
