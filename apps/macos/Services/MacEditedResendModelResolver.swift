// SPDX-License-Identifier: Apache-2.0
// Reconstruct only model credentials from current trusted profiles, never old execution policy.
import Foundation

struct MacEditedResendResolvedModel {
    let profileID: String
    let nativeRevision: Int64?
    let fallbacks: [MacRemoteModelFallback]
    let frozen: AppleFrozenResendModel<MacCanonicalJSONValue>
    var siwcProfile: MacSiwcProfileSelection? = nil
}

nonisolated enum MacEditedResendError: LocalizedError {
    case sourceUnavailable, modelUnavailable, ambiguousModel, unsupportedModel, admissionUnconfirmed, intentChanged
    var errorDescription: String? {
        switch self {
        case .sourceUnavailable: String(localized: "The original message and sending snapshot could not be verified.")
        case .modelUnavailable: String(localized: "The original model credential or configuration has changed. Resend was not admitted.")
        case .ambiguousModel: String(localized: "More than one authorized profile matches the original model. An exact choice cannot be proved.")
        case .unsupportedModel: String(localized: "This original model cannot yet be rebuilt through the trusted edited-resend adapter.")
        case .admissionUnconfirmed: String(localized: "Resend admission is not confirmed. The prepared operation was retained; retry checks it before sending.")
        case .intentChanged: String(localized: "This prepared resend contains a different edit. Cancel and open a new edit.")
        }
    }
}

@MainActor
enum MacEditedResendModelResolver {
    static func resolve(_ model: AppleFrozenResendModel<MacCanonicalJSONValue>, application: MacApplicationPort) throws -> MacEditedResendResolvedModel {
        guard case .object(let primary) = model.primary else { throw MacEditedResendError.unsupportedModel }
        guard model.version == 1, case .object = model.generation else {
            throw MacEditedResendError.unsupportedModel
        }
        // Cache material is recovered and digest-checked by the private Host
        // factory. Swift neither reconstructs a key from its hash nor drops it.
        if let account = primary["siwc_account"], account != .null {
            return try resolveSiwc(model, application: application)
        }

        if primary["kind"] == .string("native") {
            guard primary["adapter"] == .string("mlx"), primary["adapter_contract_revision"] == .number(1),
                  case .string(let id) = primary["model_id"], case .object(let reference) = primary["profile_ref"],
                  reference["profile_id"] == .string(id),
                  let wire = try JSONSerialization.jsonObject(with: JSONEncoder().encode(reference["admitted_revision"]), options: [.fragmentsAllowed]) as? NSNumber,
                  wire.int64Value > 0, Double(wire.int64Value) == wire.doubleValue,
                  let current = try application.modelProfile(id), current.revision == wire.int64Value,
                  current.enabled, current.deletedAtMs == nil, MacLocalModelProfile.resolve(id) != nil,
                  model.fallbacks == .array([]) else { throw MacEditedResendError.modelUnavailable }
            return .init(profileID: id, nativeRevision: wire.int64Value, fallbacks: [], frozen: model)
        }
        guard primary["kind"] == .string("http"), primary["siwc_account"] == .null else { throw MacEditedResendError.unsupportedModel }
        let store = MacAppleChatConfigurationStore(applicationPort: application)
        let catalog = try MacChatModelCatalog.load(application: application)
        let candidates = catalog.entries.filter { $0.candidate.allowedForChat && $0.candidate.availability == .available && !$0.option.isLocal }
        let credentials: [(MacRunModelCredential, String)] = candidates.compactMap { entry in
            guard let id = entry.option.configurationID else { return nil }
            return try? store.resolveColdRunCredential(profileID: id)
        }
        func resolveRoute(_ route: MacCanonicalJSONValue) throws -> (MacRunModelCredential, String) {
            let index = try uniqueHTTPMatch(route, credentials: credentials.map(\.0))
            return credentials[index]
        }
        let resolvedPrimary = try resolveRoute(model.primary)
        guard let configuration = try store.loadThrowing(model: resolvedPrimary.0.profileID),
              configuration.profileID == resolvedPrimary.0.profileID else { throw MacEditedResendError.modelUnavailable }
        guard case .array(let routes) = model.fallbacks else { throw MacEditedResendError.unsupportedModel }
        let fallbacks = try routes.map { route -> MacRemoteModelFallback in
            let (credential, secret) = try resolveRoute(route)
            guard credential.wireProtocol == resolvedPrimary.0.wireProtocol else { throw MacEditedResendError.unsupportedModel }
            return .init(profileID: credential.profileID, endpointName: "", endpoint: credential.endpoint, apiKey: secret, model: credential.model)
        }
        return .init(profileID: resolvedPrimary.0.profileID, nativeRevision: nil, fallbacks: fallbacks, frozen: model)
    }

    private struct SiwcBinding: Decodable {
        let account_id: String
        let account_revision: Int64
        let credential_ref: String
        let client_id: String
        let subject: String
    }

    private static func resolveSiwc(_ model: AppleFrozenResendModel<MacCanonicalJSONValue>,
                                    application: MacApplicationPort) throws -> MacEditedResendResolvedModel {
        guard case .object(let primary) = model.primary,
              primary["kind"] == .string("http"), primary["protocol"] == .string("siwc_responses"),
              primary["endpoint"] == .string("https://api.openai.com/v1"),
              primary["endpoint_query_sha256"] == .null,
              primary["credential_sha256"] == .string(MacRunModelCredential.digest("")),
              model.fallbacks == .array([]), let raw = primary["siwc_account"] else {
            throw MacEditedResendError.unsupportedModel
        }
        let binding = try JSONDecoder().decode(SiwcBinding.self, from: JSONEncoder().encode(raw))
        let account: MacModelAccountSnapshot? = try MacModelConnectionService(application: application).command([
            "type": .string("get_account"), "id": .string(binding.account_id)])
        guard let account, matchesSiwcAccount(raw, account: account) else { throw MacEditedResendError.modelUnavailable }
        let candidates = try application.listModelProfiles().filter { profile in
            guard profile.enabled, profile.deletedAtMs == nil, profile.runtime == "remote", profile.provider == "nola-chatgpt",
                  primary["model"] == .string(profile.modelName),
                  let source = try? JSONDecoder().decode([String: MacCanonicalJSONValue].self, from: Data(profile.sourceJSON.utf8)) else { return false }
            return source["accountID"] == .string(binding.account_id) && source["wireProtocol"] == .string("siwc_responses")
                && source["baseURL"] == primary["endpoint"] && source["endpointEnabled"] == .bool(true)
                && source["modelEnabled"] == .bool(true)
        }
        guard candidates.count == 1, let profile = candidates.first else {
            throw candidates.isEmpty ? MacEditedResendError.modelUnavailable : .ambiguousModel
        }
        let selection = MacSiwcProfileSelection(profileID: profile.id, revision: profile.revision)
        try MacAppleChatConfigurationStore(applicationPort: application).validateSiwcProfile(selection)
        // Host execution_binding rechecks the original client/subject, account
        // revision and private session immediately before dispatch. No bearer
        // or API-key representation is requested by this adapter.
        return .init(profileID: profile.id, nativeRevision: nil, fallbacks: [], frozen: model, siwcProfile: selection)
    }

    nonisolated static func matchesSiwcAccount(_ raw: MacCanonicalJSONValue, account: MacModelAccountSnapshot) -> Bool {
        guard let binding = try? JSONDecoder().decode(SiwcBinding.self, from: JSONEncoder().encode(raw)) else { return false }
        return binding.account_revision > 0 && binding.account_revision == account.revision
            && binding.account_id == account.id && account.account.source == "nola_chatgpt"
            && account.account.state == "ready" && binding.credential_ref == "siwc:" + account.id
            && binding.credential_ref == account.account.credential_ref
            && !binding.client_id.isEmpty && !binding.subject.isEmpty
    }

    nonisolated static func uniqueHTTPMatch(_ route: MacCanonicalJSONValue, credentials: [MacRunModelCredential]) throws -> Int {
        let matching = credentials.indices.filter { matchesHTTP(route, credential: credentials[$0]) }
        guard matching.count == 1 else { throw matching.isEmpty ? MacEditedResendError.modelUnavailable : .ambiguousModel }
        return matching[0]
    }

    nonisolated static func matchesHTTP(_ route: MacCanonicalJSONValue, credential: MacRunModelCredential) -> Bool {
        guard case .object(let value) = route,
              value["kind"] == .string("http"), value["siwc_account"] == .null,
              value["model"] == .string(credential.model), value["protocol"] == .string(credential.wireProtocol.rawValue),
              value["credential_sha256"] == .string(credential.secretDigest),
              var endpoint = URLComponents(string: credential.endpoint), endpoint.user == nil, endpoint.password == nil,
              endpoint.fragment == nil else { return false }
        let query = endpoint.percentEncodedQuery
        endpoint.query = nil
        if endpoint.path.isEmpty { endpoint.path = "/" }
        guard value["endpoint"] == endpoint.url.map({ MacCanonicalJSONValue.string($0.absoluteString) }),
              value["endpoint_query_sha256"] == (query.map { .string(MacRunModelCredential.digest($0)) } ?? .null) else { return false }
        return true
    }
}
