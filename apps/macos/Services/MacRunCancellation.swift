import Foundation

nonisolated enum MacRunCancellationContractError: Error, Equatable {
    case invalidRequest
    case invalidReceipt
    case identityMismatch
}

/// Cancellation targets an existing Run; it carries no model, workspace grant,
/// prompt or replacement submission. A receipt is not a transport launch grant.
nonisolated struct MacRunCancellationRequest: Encodable, Equatable, Sendable {
    let conversationID: String
    let expectedRunID: String
    let assistantMessageID: String
    let cancellationID: String
    let reason: String?
    let requestedAtMs: Int64

    init(conversationID: String, expectedRunID: String, assistantMessageID: String,
         cancellationID: String, reason: String? = nil, requestedAtMs: Int64) throws {
        guard [conversationID, expectedRunID, assistantMessageID, cancellationID].allSatisfy(Self.validIdentity),
              requestedAtMs > 0, reason.map({ $0.utf8.count <= 4096 }) ?? true else {
            throw MacRunCancellationContractError.invalidRequest
        }
        self.conversationID = conversationID
        self.expectedRunID = expectedRunID
        self.assistantMessageID = assistantMessageID
        self.cancellationID = cancellationID
        self.reason = reason
        self.requestedAtMs = requestedAtMs
    }

    fileprivate static func validIdentity(_ value: String) -> Bool {
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && value.utf8.count <= 1024
    }

    private enum CodingKeys: String, CodingKey {
        case conversationID = "conversation_id", expectedRunID = "expected_run_id"
        case assistantMessageID = "assistant_message_id", cancellationID = "cancellation_id"
        case reason, requestedAtMs = "requested_at_ms"
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(conversationID, forKey: .conversationID)
        try values.encode(expectedRunID, forKey: .expectedRunID)
        try values.encode(assistantMessageID, forKey: .assistantMessageID)
        try values.encode(cancellationID, forKey: .cancellationID)
        try values.encode(reason, forKey: .reason) // explicit null is part of the wire shape
        try values.encode(requestedAtMs, forKey: .requestedAtMs)
    }
}

nonisolated struct MacRunCancellationReceipt: Decodable, Equatable, Sendable {
    enum Disposition: String, Decodable, Sendable {
        case cancellationCommitted = "cancellation_committed"
        case alreadyCancelling = "already_cancelling"
        case alreadyTerminal = "already_terminal"
    }
    let conversationID: String
    let runID: String
    let assistantMessageID: String
    let cancellationID: String
    let disposition: Disposition
    let terminalResult: MacRunResult?
    let resultDigest: String?

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case conversationID = "conversation_id", runID = "run_id"
        case assistantMessageID = "assistant_message_id", cancellationID = "cancellation_id"
        case disposition, terminalResult = "terminal_result", resultDigest = "result_digest"
    }
    private struct AnyKey: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: AnyKey.self)
        guard Set(keys.allKeys.map(\.stringValue)) == Set(CodingKeys.allCases.map(\.rawValue)) else {
            throw MacRunCancellationContractError.invalidReceipt
        }
        let values = try decoder.container(keyedBy: CodingKeys.self)
        conversationID = try values.decode(String.self, forKey: .conversationID)
        runID = try values.decode(String.self, forKey: .runID)
        assistantMessageID = try values.decode(String.self, forKey: .assistantMessageID)
        cancellationID = try values.decode(String.self, forKey: .cancellationID)
        disposition = try values.decode(Disposition.self, forKey: .disposition)
        terminalResult = try values.decodeIfPresent(MacRunResult.self, forKey: .terminalResult)
        resultDigest = try values.decodeIfPresent(String.self, forKey: .resultDigest)
        guard [conversationID, runID, assistantMessageID, cancellationID].allSatisfy(MacRunCancellationRequest.validIdentity),
              (terminalResult == nil) == (resultDigest == nil),
              terminalResult.map({ $0.run_id == runID && $0.conversation_id == conversationID }) ?? true,
              resultDigest.map({ $0.count == 64 && $0.allSatisfy { $0.isHexDigit && !$0.isUppercase } }) ?? true,
              disposition != .alreadyTerminal || terminalResult != nil,
              disposition != .alreadyCancelling || terminalResult == nil else {
            throw MacRunCancellationContractError.invalidReceipt
        }
    }

    func validated(for request: MacRunCancellationRequest) throws -> Self {
        guard conversationID == request.conversationID, runID == request.expectedRunID,
              assistantMessageID == request.assistantMessageID, cancellationID == request.cancellationID else {
            throw MacRunCancellationContractError.identityMismatch
        }
        return self
    }
}
