// SPDX-License-Identifier: Apache-2.0
// In-memory launch authority; never serialized into a Run or transport frame.
import Foundation
import CryptoKit

nonisolated struct MacSiwcProfileSelection: Equatable, Sendable {
    let profileID: String
    let revision: Int64
}

nonisolated struct MacRunModelCredential: Equatable, Sendable {
    let profileID: String
    let endpointID: String
    let account: String
    let endpoint: String
    let model: String
    let wireProtocol: MacModelWireProtocol
    let secretDigest: String

    static func digest(_ secret: String) -> String {
        SHA256.hash(data: Data(secret.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}

nonisolated struct MacLocalAgentCredentialSelection: Sendable {
    let credentials: [MacRunModelCredential]
    let recoverable: Bool
    let nativeEndpoint: MacMLXOpenAIService.Endpoint?
    let applicationPort: MacApplicationPort?
    let siwcProfile: MacSiwcProfileSelection?
    init(credentials: [MacRunModelCredential], recoverable: Bool,
         nativeEndpoint: MacMLXOpenAIService.Endpoint? = nil, applicationPort: MacApplicationPort? = nil,
         siwcProfile: MacSiwcProfileSelection? = nil) {
        self.credentials = credentials
        self.recoverable = recoverable
        self.nativeEndpoint = nativeEndpoint
        self.applicationPort = applicationPort
        self.siwcProfile = siwcProfile
    }
    static let unavailable = Self(credentials: [], recoverable: false)
}

nonisolated enum MacLocalAgentCredentialGate {
    static func capture(_ request: MacLocalAgentRunRequest,
                        localEndpoint: MacMLXOpenAIService.Endpoint? = nil,
                        store: MacAppleChatConfigurationStore) throws -> MacLocalAgentCredentialSelection {
        if !MacExecutionBackendPolicy.requiresNolaModel(request.executionBackend) {
            return .init(credentials: [], recoverable: true)
        }
        if request.wireProtocol == .siwcResponses {
            guard let selection = request.siwcProfile, selection.profileID == request.modelProfileID,
                  request.apiKey?.isEmpty != false, request.modelFallbacks.isEmpty, localEndpoint == nil else {
                throw MacLocalAgentResumeError.credentialUnavailable
            }
            try store.validateSiwcProfile(selection)
            return .init(credentials: [], recoverable: true, siwcProfile: selection)
        }
        var credentials: [MacRunModelCredential] = []
        var recoverable = false
        if let localEndpoint {
            guard let port = request.applicationPort else { throw MacLocalAgentResumeError.credentialUnavailable }
            try MacMLXOpenAIService.shared.validate(localEndpoint, applicationPort: port)
            recoverable = true
        } else if let profileID = request.modelProfileID {
            credentials.append(try store.captureRunCredential(profileID: profileID,
                endpoint: request.endpoint ?? "", model: request.modelID ?? "", apiKey: request.apiKey ?? "",
                wireProtocol: request.wireProtocol ?? .chatCompletions))
            recoverable = true
        }
        for fallback in request.modelFallbacks {
            // Conservative admission: validate every frozen configured route,
            // matching the current Host binding. Shared executable-candidate
            // admission will narrow this set; do not duplicate it in Swift.
            credentials.append(try store.captureRunCredential(profileID: fallback.profileID,
                endpoint: fallback.endpoint, model: fallback.model, apiKey: fallback.apiKey,
                wireProtocol: request.wireProtocol ?? .chatCompletions))
        }
        return .init(credentials: credentials, recoverable: recoverable,
            nativeEndpoint: localEndpoint, applicationPort: localEndpoint == nil ? nil : request.applicationPort)
    }

    /// Called synchronously immediately before Process launch, after all async
    /// workspace preparation. A rejected credential never reaches dispatch.
    static func launch(selection: MacLocalAgentCredentialSelection, isResume: Bool,
                       store: MacAppleChatConfigurationStore, operation: () throws -> Void) throws {
        guard !isResume || selection.recoverable else { throw MacLocalAgentResumeError.credentialUnavailable }
        if let endpoint = selection.nativeEndpoint {
            guard let port = selection.applicationPort else { throw MacLocalAgentResumeError.credentialUnavailable }
            try MacMLXOpenAIService.shared.validate(endpoint, applicationPort: port)
        }
        for credential in selection.credentials { try store.validateRunCredential(credential) }
        if let siwc = selection.siwcProfile { try store.validateSiwcProfile(siwc) }
        try operation()
    }
}
