// SPDX-License-Identifier: Apache-2.0
import Foundation
import Darwin

nonisolated enum MacSiwcBridgeFailure: Error, LocalizedError {
    case unavailable, accountChanged, privateSessionRetained, privateSessionChanged, refreshUnknown, reauthorize, consent
    case authorizationDeclined, authorizationExpired, authorizationNetwork
    static func sanitized(_ code: String?) -> Self {
        switch code {
        case "native_siwc_authorization_declined": .authorizationDeclined
        case "native_siwc_authorization_expired": .authorizationExpired
        case "native_siwc_authorization_network": .authorizationNetwork
        case "native_siwc_account_changed": .accountChanged
        case "native_siwc_core_commit_conflict_private_session_retained": .privateSessionRetained
        case "native_siwc_private_session_changed": .privateSessionChanged
        case "native_siwc_refresh_result_unknown": .refreshUnknown
        case "native_siwc_reauthorization_required": .reauthorize
        case "native_siwc_plan_consent_required": .consent
        default: .unavailable
        }
    }
    var errorDescription: String? {
        switch self {
        case .authorizationDeclined: String(localized: "Sign-in was declined. Start a new sign-in only if you want to grant access.", table: "ModelCenter")
        case .authorizationExpired: String(localized: "Sign-in expired. Start a new sign-in; the old browser response cannot be reused.", table: "ModelCenter")
        case .authorizationNetwork: String(localized: "The authorization service could not be reached. Check your connection before starting a new sign-in.", table: "ModelCenter")
        case .accountChanged: String(localized: "This Mac’s account changed. Check status before choosing another action; no changes were retried.")
        case .privateSessionRetained: String(localized: "Authorization was verified, but the account changed before saving. Private authorization may remain. Check this Mac’s accounts; do not repeat the confirmation blindly.")
        case .privateSessionChanged: String(localized: "Private authorization changed. Check status again before continuing with this account.")
        case .refreshUnknown: String(localized: "The refresh result is unknown. Do not retry the old refresh. Check status and reauthorize this account if required.")
        case .reauthorize: String(localized: "This account requires reauthorization. Use Reauthorize for this same account; Nola will not switch accounts automatically.")
        case .consent: String(localized: "This authorization does not grant plan usage. Reauthorize with the required consent before requesting models.")
        case .unavailable: String(localized: "The account operation could not be confirmed. Check this Mac’s accounts before starting again. No inference was requested.")
        }
    }
    static func message(_ error: Error) -> String { ((error as? Self) ?? .unavailable).localizedDescription }
}

nonisolated struct MacSiwcFlow: Decodable, Sendable {
    let owner_id: String
    let flow_id: String
    let account_id: String
    let expected_account_revision: Int64?
    let phase: String
    let authorization_url: String?
    let error: String?
    let private_session_may_be_retained: Bool
    let account: MacModelAccountSnapshot?
    let inference_performed: Bool
}

nonisolated struct MacSiwcAccountResult: Decodable, Sendable {
    let owner_id: String
    let account_id: String
    let account: MacModelAccountSnapshot
    let session: Session?
    let connection: MacModelConnectionSnapshot?
    let inference_performed: Bool
    struct Session: Decodable, Sendable {
        let account_id: String
        let session_revision: Int64
        let signed_in: Bool
        let plan_usage_authorized: Bool
        let reauthorization_required: Bool
        let revocation_unconfirmed: Bool
    }
}

nonisolated protocol MacSiwcAccountTransport: Sendable {
    func start(accountID: String, revision: Int64?, name: String) async throws -> MacSiwcFlow
    func update(_ action: String) async throws -> MacSiwcFlow
    func close()
}

/// Same-database private IPC. OAuth materials never cross this boundary.
nonisolated final class MacSiwcAccountBridge: MacSiwcAccountTransport, @unchecked Sendable {
    private let process = Process()
    private let input: FileHandle
    private let output: FileHandle
    private let queue = DispatchQueue(label: "com.gaojc.Nola.siwc-account")
    private var buffer = Data()
    private var owner: String?
    private var current: MacSiwcFlow?
    private let maximumFrame = 64 * 1024

    init(application: MacApplicationPort) throws {
        guard let executable = MacAgentHostExecutableLocator.resolve(), executable.isFileURL,
              FileManager.default.isExecutableFile(atPath: executable.path),
              (try executable.resourceValues(forKeys: [.isRegularFileKey])).isRegularFile == true else {
            throw Self.failure()
        }
        let incoming = Pipe(), outgoing = Pipe()
        input = incoming.fileHandleForWriting; output = outgoing.fileHandleForReading
        process.executableURL = executable
        process.arguments = ["model-siwc-bridge", "--database", application.databaseURL.path]
        process.standardInput = incoming; process.standardOutput = outgoing
        process.standardError = FileHandle.nullDevice
        try process.run()
    }
    #if DEBUG
    init(fixtureInput: FileHandle, fixtureOutput: FileHandle) {
        input = fixtureInput; output = fixtureOutput
    }
    #endif
    deinit { close() }
    func close() {
        try? input.close()
        if process.isRunning { process.terminate() }
    }
    func start(accountID: String, revision: Int64?, name: String) async throws -> MacSiwcFlow {
        try await operation { [self] in
            guard current == nil else { throw Self.failure() }
            let hello = try exchange(type: "hello", request: nil)
            guard hello["type"] as? String == "hello", hello["protocol"] as? Int == 1,
                  let ownerID = hello["owner_id"] as? String, UUID(uuidString: ownerID) != nil else { throw Self.failure() }
            owner = ownerID
            let frame = try exchange(type: "start", request: ["owner_id": ownerID,
                "account_id": accountID, "expected_account_revision": revision.map { $0 as Any } ?? NSNull(),
                "display_name": name, "acknowledge_authorization": true])
            let flow = try decode(frame, accountID: accountID, revision: revision, flowID: nil)
            current = flow
            return flow
        }
    }
    func update(_ action: String) async throws -> MacSiwcFlow {
        try await operation { [self] in
            guard ["status", "cancel", "finish"].contains(action), let selected = current else { throw Self.failure() }
            let frame = try exchange(type: action, request: ["owner_id": selected.owner_id,
                "flow_id": selected.flow_id, "account_id": selected.account_id,
                "expected_account_revision": selected.expected_account_revision.map { $0 as Any } ?? NSNull()])
            let flow = try decode(frame, accountID: selected.account_id,
                revision: selected.expected_account_revision, flowID: selected.flow_id)
            current = flow
            return flow
        }
    }
    func manage(_ action: String, account: MacModelAccountSnapshot, sessionRevision: Int64?, acknowledgeSignOut: Bool = false) async throws -> MacSiwcAccountResult {
        try await operation { [self] in
            guard ["account_status", "models", "refresh", "sign_out"].contains(action) else { throw Self.failure() }
            if owner == nil {
                let hello = try exchange(type: "hello", request: nil)
                guard hello["type"] as? String == "hello", hello["protocol"] as? Int == 1,
                      let id = hello["owner_id"] as? String, UUID(uuidString: id) != nil else { throw Self.failure() }
                owner = id
            }
            guard let owner else { throw Self.failure() }
            let frame = try exchange(type: action, request: ["owner_id": owner, "account_id": account.id,
                "expected_account_revision": account.revision,
                "expected_session_revision": sessionRevision.map { $0 as Any } ?? NSNull(),
                "acknowledge_sign_out": acknowledgeSignOut])
            guard frame["type"] as? String == "account", let value = frame["result"] else { throw Self.failure() }
            let result = try JSONDecoder().decode(MacSiwcAccountResult.self, from: JSONSerialization.data(withJSONObject: value))
            guard result.owner_id == owner, result.account_id == account.id, result.account.id == account.id,
                  result.account.revision >= account.revision,
                  result.account.account.source == "nola_chatgpt", !result.inference_performed,
                  result.session == nil || result.session?.account_id == account.id,
                  result.connection == nil || result.connection?.document.connection.account_id == account.id else { throw Self.failure() }
            return result
        }
    }
    private func decode(_ frame: [String: Any], accountID: String, revision: Int64?, flowID: String?) throws -> MacSiwcFlow {
        guard frame["type"] as? String == "flow", let value = frame["flow"] else { throw Self.failure() }
        let flow = try JSONDecoder().decode(MacSiwcFlow.self, from: JSONSerialization.data(withJSONObject: value))
        guard flow.owner_id == owner, flow.account_id == accountID, flow.expected_account_revision == revision,
              UUID(uuidString: flow.flow_id) != nil, flowID == nil || flow.flow_id == flowID,
              !flow.inference_performed,
              ["preparing", "awaiting_authorization", "authorized", "failed", "cancelled", "committed"].contains(flow.phase),
              flow.account == nil || (flow.phase == "committed" && flow.account?.id == accountID),
              flow.phase != "committed" || flow.account != nil else { throw Self.failure() }
        if let url = flow.authorization_url {
            guard flow.phase == "awaiting_authorization", Self.authorizationURL(url) != nil else { throw Self.failure() }
        }
        return flow
    }
    static func authorizationURL(_ value: String) -> URL? {
        guard let url = URL(string: value), url.scheme == "https", url.host == "auth.openai.com",
              url.user == nil, url.password == nil, url.port == nil, url.fragment == nil else { return nil }
        return url
    }
    private func operation<T: Sendable>(_ action: @escaping @Sendable () throws -> T) async throws -> T {
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            queue.async { continuation.resume(with: Result { try action() }) }
        }
    }
    private func exchange(type: String, request: [String: Any]?) throws -> [String: Any] {
        let id = UUID().uuidString.lowercased()
        let deadline = DispatchWorkItem { [weak self] in self?.close() }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 15, execute: deadline)
        defer { deadline.cancel() }
        var frame: [String: Any] = ["type": type, "request_id": id]
        if let request { frame["request"] = request }
        var bytes = try JSONSerialization.data(withJSONObject: frame)
        guard bytes.count < maximumFrame else { throw Self.failure() }
        bytes.append(10)
        do {
            try input.write(contentsOf: bytes)
            let response = try read()
            guard response["request_id"] as? String == id else { throw Self.failure() }
            if response["type"] as? String == "error" { throw MacSiwcBridgeFailure.sanitized(response["error"] as? String) }
            return response
        } catch let error as MacSiwcBridgeFailure { close(); throw error }
        catch { close(); throw Self.failure() }
    }
    private func read() throws -> [String: Any] {
        while true {
            if let newline = buffer.firstIndex(of: 10) {
                let bytes = buffer.prefix(upTo: newline)
                buffer.removeSubrange(...newline)
                guard let frame = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else { throw Self.failure() }
                return frame
            }
            var bytes = [UInt8](repeating: 0, count: 4096)
            let count = Darwin.read(output.fileDescriptor, &bytes, bytes.count)
            if count < 0 && errno == EINTR { continue }
            guard count > 0 else { throw Self.failure() }
            buffer.append(contentsOf: bytes.prefix(count))
            guard buffer.count <= maximumFrame else { throw Self.failure() }
        }
    }
    private static func failure() -> Error {
        MacSiwcBridgeFailure.unavailable
    }
}
