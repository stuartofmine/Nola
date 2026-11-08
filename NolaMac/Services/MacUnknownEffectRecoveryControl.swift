// SPDX-License-Identifier: Apache-2.0
// Durable transport intent for explicitly confirmed, local user recovery only.
import Foundation
import CryptoKit
import Darwin

nonisolated struct MacUnknownEffectRecoveryTarget: Codable, Equatable, Sendable {
    let effectID: String
    let requestID: String
    let attemptID: String
    let workerGeneration: String
    let leaseOwner: String
    let leaseToken: String
    let expectedAttemptRevision: Int64
    enum CodingKeys: String, CodingKey {
        case effectID = "effect_id", requestID = "request_id", attemptID = "attempt_id"
        case workerGeneration = "worker_generation", leaseOwner = "lease_owner", leaseToken = "lease_token"
        case expectedAttemptRevision = "expected_attempt_revision"
    }
}

nonisolated struct MacUnknownEffectRecoveryInspection: Codable, Equatable, Sendable {
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let invocationConfigDigest: String
    let targets: [MacUnknownEffectRecoveryTarget]
    let retryAllowed: Bool
    let independentPendingEffects: UInt64
    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", runID = "run_id", assistantMessageID = "assistant_message_id"
        case invocationConfigDigest = "invocation_config_digest", targets
        case independentPendingEffects = "independent_pending_effects", retryAllowed = "retry_allowed"
    }
}

nonisolated struct MacUnknownEffectRecoveryDecision: Codable, Equatable, Sendable {
    struct Choice: Codable, Equatable, Sendable {
        enum Kind: String, Codable, Sendable { case authorizeRiskRetry = "authorize_risk_retry", keepUnknownBlocked = "keep_unknown_blocked" }
        let kind: Kind
        let reason: String
    }
    let conversationID: String
    let runID: String
    let invocationConfigDigest: String
    let resolutionID: String
    let targets: [MacUnknownEffectRecoveryTarget]
    let choice: Choice
    let resolvedAtMs: Int64
    enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", runID = "run_id", invocationConfigDigest = "invocation_config_digest"
        case resolutionID = "resolution_id", targets, choice, resolvedAtMs = "resolved_at_ms"
    }

    func validate() throws {
        let ids = [conversationID, runID, invocationConfigDigest, resolutionID]
        guard ids.allSatisfy(Self.validID), resolvedAtMs >= 0, (1...256).contains(targets.count),
              !choice.reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              choice.reason.utf8.count <= 4096 else { throw MacApplicationServiceError.invalidResponse }
        for target in targets {
            guard [target.effectID, target.requestID, target.attemptID, target.workerGeneration,
                   target.leaseOwner, target.leaseToken].allSatisfy(Self.validID),
                  target.expectedAttemptRevision > 0, target.expectedAttemptRevision < Int64.max else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
    }
    static func validID(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.count <= 512
            && value.trimmingCharacters(in: .whitespacesAndNewlines) == value
            && value.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
    }
}

nonisolated struct MacUnknownEffectRecoveryReply: Decodable, Equatable, Sendable {
    enum Outcome: String, Decodable, Sendable {
        case retryAuthorized = "retry_authorized", unknownPreservedBlocked = "unknown_preserved_blocked"
        case rejected, conflict
    }
    let outcome: Outcome
    let resolutionID: String?
    let runID: String?
    let code: String?
    enum CodingKeys: String, CodingKey {
        case outcome, resolutionID = "resolution_id", runID = "run_id", code
    }
    func validate(for decision: MacUnknownEffectRecoveryDecision) throws {
        switch outcome {
        case .retryAuthorized, .unknownPreservedBlocked:
            guard resolutionID == decision.resolutionID, runID == decision.runID, code == nil,
                  (outcome == .retryAuthorized) == (decision.choice.kind == .authorizeRiskRetry) else {
                throw MacApplicationServiceError.invalidResponse
            }
        case .rejected, .conflict:
            guard let code, !code.isEmpty, resolutionID == nil, runID == nil else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
    }
}

/// One private transport-intent file per exact Run. No execution state is
/// synthesized here; after a reply callers reload the canonical projection.
nonisolated final class MacUnknownEffectRecoveryControl: @unchecked Sendable {
    typealias Transport = (Data, String) throws -> Data
    private struct Command: Codable { let type: String; let input: MacUnknownEffectRecoveryDecision; var owner: AppleOwnedTaskRecoveryOwner? = nil }
    private struct Pending: Codable {
        let schema: Int
        let storeID: String
        let clientID: String
        let protocolVersion: Int
        let requestID: String
        let assistantMessageID: String
        let command: Data
    }
    // 256 targets × 6 identities × 512 UTF-8 bytes × max JSON escaping (6),
    // plus keys/reason/owner fields fits 8 MiB; base64 journal fits 12 MiB.
    private static let commandLimit = 8 * 1024 * 1024
    private static let journalLimit = 12 * 1024 * 1024
    private let directory: URL
    private let storeID: String
    private let clientID: String
    private let transport: Transport
    private let lock = NSLock()
    private var storageUncertain = false

    convenience init(client: MacApplicationServiceClient) {
        self.init(databaseURL: client.databaseURL, clientID: client.clientID) { data, requestID in
            try client.execute(commandData: data, requestID: requestID)
        }
    }
    init(databaseURL: URL, clientID: String, transport: @escaping Transport) {
        storeID = databaseURL.standardizedFileURL.path
        directory = databaseURL.appendingPathExtension("pending-recovery-decisions")
        self.clientID = clientID
        self.transport = transport
    }

    func inspect(conversationID: String, runID: String, assistantMessageID: String) throws -> MacUnknownEffectRecoveryInspection? {
        guard [conversationID, runID, assistantMessageID].allSatisfy(MacUnknownEffectRecoveryDecision.validID) else {
            throw MacApplicationServiceError.invalidResponse
        }
        let query = try JSONSerialization.data(withJSONObject: ["type": "inspect_unknown_effect_recovery",
            "conversation_id": conversationID, "expected_run_id": runID, "assistant_message_id": assistantMessageID])
        let result = try JSONDecoder().decode(MacUnknownEffectRecoveryInspection?.self,
            from: transport(query, UUID().uuidString.lowercased()))
        if let result {
            guard result.conversationID == conversationID, result.runID == runID,
                  result.assistantMessageID == assistantMessageID, result.targets.count <= 256 else {
                throw MacApplicationServiceError.invalidResponse
            }
        }
        return result
    }

    func inspectOwnedTask(_ owner: AppleOwnedTaskRecoveryOwner) throws -> MacUnknownEffectRecoveryInspection? {
        struct Query: Encodable { let type = "inspect_owned_task_recovery"; let owner: AppleOwnedTaskRecoveryOwner }
        return try JSONDecoder().decode(MacUnknownEffectRecoveryInspection?.self,
            from: transport(JSONEncoder().encode(Query(owner: owner)), UUID().uuidString.lowercased()))
    }

    func pendingOwnedDecision(_ owner: AppleOwnedTaskRecoveryOwner) throws -> MacUnknownEffectRecoveryDecision? {
        try withJournal(runID: owner.parent_run_id, taskOwner: owner) { url in
            guard let pending = try load(url, runID: nil) else { return nil }
            let command = try JSONDecoder().decode(Command.self, from: pending.command)
            guard command.owner == owner else { throw MacApplicationServiceError.invalidResponse }
            return command.input
        }
    }

    /// Read transport intent only; does not inspect or replace its frozen fences.
    func pendingDecision(runID: String, taskOwner: AppleOwnedTaskRecoveryOwner? = nil) throws -> MacUnknownEffectRecoveryDecision? {
        try withJournal(runID: runID, taskOwner: taskOwner) { url in
            guard let pending = try load(url, runID: runID) else { return nil }
            let command = try JSONDecoder().decode(Command.self, from: pending.command)
            guard command.owner == taskOwner else { throw MacApplicationServiceError.invalidResponse }
            return command.input
        }
    }

    /// Call only after explicit confirmation. Caller supplies the exact inspected
    /// assistant owner; time and IDs are frozen once and never refreshed on retry.
    func submitConfirmed(_ decision: MacUnknownEffectRecoveryDecision, assistantMessageID: String,
                         requestID: String = UUID().uuidString.lowercased(),
                         taskOwner: AppleOwnedTaskRecoveryOwner? = nil) throws -> MacUnknownEffectRecoveryReply {
        try decision.validate()
        guard MacUnknownEffectRecoveryDecision.validID(assistantMessageID),
              MacUnknownEffectRecoveryDecision.validID(requestID) else { throw MacApplicationServiceError.invalidResponse }
        return try withJournal(runID: decision.runID, taskOwner: taskOwner) { url in
            if let existing = try load(url, runID: decision.runID) {
                throw MacApplicationServiceError.outcomePending(requestID: existing.requestID)
            }
            guard taskOwner == nil || decision.choice.kind == .keepUnknownBlocked else { throw MacApplicationServiceError.invalidResponse }
            let command = try JSONEncoder().encode(Command(type: taskOwner == nil ? "resolve_unknown_effect_recovery" : "resolve_owned_task_recovery", input: decision, owner: taskOwner))
            guard command.count <= Self.commandLimit else { throw MacApplicationServiceError.invalidResponse }
            let pending = Pending(schema: 1, storeID: storeID, clientID: clientID,
                protocolVersion: MacApplicationServiceClient.protocolVersion, requestID: requestID,
                assistantMessageID: assistantMessageID, command: command)
            try save(pending, to: url)
            return try send(pending, decision: decision, url: url)
        }
    }

    /// Recovery always queries the original receipt first. A missing receipt
    /// permits only same-ID/same-payload replay, never a new risk authorization.
    func recoverPending(runID: String, replayIfMissing: Bool = true, taskOwner: AppleOwnedTaskRecoveryOwner? = nil) throws -> MacUnknownEffectRecoveryReply? {
        try withJournal(runID: runID, taskOwner: taskOwner) { url in
            guard let pending = try load(url, runID: runID) else { return nil }
            let command = try JSONDecoder().decode(Command.self, from: pending.command)
            guard command.owner == taskOwner else { throw MacApplicationServiceError.invalidResponse }
            let decision = command.input
            let query = try JSONSerialization.data(withJSONObject: ["type": "command_receipt", "request_id": pending.requestID])
            let raw = try transport(query, UUID().uuidString.lowercased())
            let value = try JSONSerialization.jsonObject(with: raw, options: [.fragmentsAllowed])
            if value is NSNull {
                // Terminal projection reconciliation must only observe the
                // original receipt. It never resubmits a recovery decision.
                guard replayIfMissing else {
                    throw MacApplicationServiceError.outcomePending(requestID: pending.requestID)
                }
                return try send(pending, decision: decision, url: url)
            }
            guard let receipt = value as? [String: Any], receipt["request_id"] as? String == pending.requestID,
                  receipt["client_id"] as? String == clientID,
                  receipt["protocol_version"] as? Int == pending.protocolVersion else { throw MacApplicationServiceError.invalidResponse }
            switch receipt["status"] as? String {
            case "committed":
                guard let result = receipt["result"] else { throw MacApplicationServiceError.invalidResponse }
                let reply = try JSONDecoder().decode(MacUnknownEffectRecoveryReply.self,
                    from: JSONSerialization.data(withJSONObject: result))
                try reply.validate(for: decision)
                try save(nil, to: url)
                return reply
            case "rejected":
                guard let error = receipt["error"] as? [String: Any], let message = error["message"] as? String else {
                    throw MacApplicationServiceError.invalidResponse
                }
                try save(nil, to: url)
                throw MacApplicationServiceError.rejected(message)
            case "conflict":
                guard let conflict = receipt["conflict"] as? [String: Any], let scope = conflict["scope_id"] as? String,
                      let expected = conflict["expected_revision"] as? NSNumber, let actual = conflict["actual_revision"] as? NSNumber else {
                    throw MacApplicationServiceError.invalidResponse
                }
                try save(nil, to: url)
                throw MacApplicationServiceError.revisionConflict(scope: scope, expected: expected.uint64Value, actual: actual.uint64Value)
            case "accepted": throw MacApplicationServiceError.outcomePending(requestID: pending.requestID)
            default: throw MacApplicationServiceError.invalidResponse
            }
        }
    }

    private func send(_ pending: Pending, decision: MacUnknownEffectRecoveryDecision, url: URL) throws -> MacUnknownEffectRecoveryReply {
        // Error names are not receipts: even .rejected can originate in a
        // local ABI gate. Only a validated response retires the intent.
        let data = try transport(pending.command, pending.requestID)
        let reply = try JSONDecoder().decode(MacUnknownEffectRecoveryReply.self, from: data)
        try reply.validate(for: decision)
        try save(nil, to: url)
        return reply
    }

    private func load(_ url: URL, runID: String?) throws -> Pending? {
        let fd = Darwin.open(url.path, O_RDONLY | O_NOFOLLOW)
        if fd < 0 { if errno == ENOENT { return nil }; throw failure() }
        let file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_uid == geteuid(), info.st_mode & 0o777 == 0o600,
              info.st_mode & S_IFMT == S_IFREG, info.st_size >= 0,
              info.st_size <= Int64(Self.journalLimit) else { throw failure() }
        // fstat is only a snapshot. Bound the read itself as well, so a file
        // growing after inspection cannot turn recovery into an unbounded read.
        var data = Data()
        while data.count <= Self.journalLimit {
            let remaining = Self.journalLimit + 1 - data.count
            guard let chunk = try file.read(upToCount: min(64 * 1024, remaining)), !chunk.isEmpty else { break }
            data.append(chunk)
        }
        guard data.count <= Self.journalLimit else { throw MacApplicationServiceError.invalidResponse }
        let pending = try JSONDecoder().decode(Pending?.self, from: data)
        guard let pending else { return nil }
        guard pending.schema == 1, pending.storeID == storeID, pending.clientID == clientID,
              pending.protocolVersion == MacApplicationServiceClient.protocolVersion,
              MacUnknownEffectRecoveryDecision.validID(pending.requestID),
              MacUnknownEffectRecoveryDecision.validID(pending.assistantMessageID),
              pending.command.count <= Self.commandLimit else { throw MacApplicationServiceError.invalidResponse }
        let command = try JSONDecoder().decode(Command.self, from: pending.command)
        guard (runID == nil || command.input.runID == runID),
              (command.type == "resolve_unknown_effect_recovery" && command.owner == nil)
                || (command.type == "resolve_owned_task_recovery" && command.owner != nil
                    && command.input.choice.kind == .keepUnknownBlocked) else {
            throw MacApplicationServiceError.invalidResponse
        }
        try command.input.validate()
        return pending
    }

    private func withJournal<T>(runID: String, taskOwner: AppleOwnedTaskRecoveryOwner? = nil, _ body: (URL) throws -> T) throws -> T {
        guard MacUnknownEffectRecoveryDecision.validID(runID) else { throw MacApplicationServiceError.invalidResponse }
        lock.lock(); defer { lock.unlock() }
        guard !storageUncertain else { throw failure() }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                               attributes: [.posixPermissions: 0o700])
        var info = stat()
        guard lstat(directory.path, &info) == 0, info.st_uid == geteuid(), info.st_mode & S_IFMT == S_IFDIR,
              info.st_mode & 0o777 == 0o700 else { throw failure() }
        let journalIdentity = try taskOwner.map { try JSONSerialization.data(withJSONObject: [$0.parent_conversation_id, $0.parent_run_id, $0.task_id]) } ?? Data(runID.utf8)
        let key = SHA256.hash(data: journalIdentity).map { String(format: "%02x", $0) }.joined()
        let url = directory.appendingPathComponent(key + ".json")
        let fd = Darwin.open(directory.appendingPathComponent(key + ".lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw failure() }
        defer { Darwin.close(fd) }
        guard fstat(fd, &info) == 0, info.st_uid == geteuid(), info.st_mode & S_IFMT == S_IFREG,
              info.st_mode & 0o777 == 0o600, flock(fd, LOCK_EX) == 0 else { throw failure() }
        defer { flock(fd, LOCK_UN) }
        return try body(url)
    }

    private func save(_ pending: Pending?, to url: URL) throws {
        do { try writeAtomically(pending, to: url) }
        catch {
            // A failed durability acknowledgement must never allow this owner
            // to overtake the intent with a fresh risk authorization.
            storageUncertain = true
            throw error
        }
    }

    private func writeAtomically(_ pending: Pending?, to url: URL) throws {
        let data = try JSONEncoder().encode(pending)
        guard data.count <= Self.journalLimit else { throw failure() }
        var template = Array(directory.appendingPathComponent(".decision.XXXXXX").path.utf8CString)
        let fd = mkstemp(&template)
        guard fd >= 0 else { throw failure() }
        let path = String(cString: template)
        defer { Darwin.close(fd); unlink(path) }
        guard fchmod(fd, 0o600) == 0 else { throw failure() }
        try FileHandle(fileDescriptor: fd, closeOnDealloc: false).write(contentsOf: data)
        guard fsync(fd) == 0, rename(path, url.path) == 0 else { throw failure() }
        let directoryFD = Darwin.open(directory.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard directoryFD >= 0 else { throw failure() }
        defer { Darwin.close(directoryFD) }
        guard fsync(directoryFD) == 0 else { throw failure() }
    }
    private func failure() -> NSError {
        NSError(domain: "NolaRecoveryIntent", code: Int(errno), userInfo: [NSLocalizedDescriptionKey: "Recovery intent storage could not be confirmed; no replacement decision is authorized."])
    }
}
