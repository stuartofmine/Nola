// SPDX-License-Identifier: Apache-2.0
// Stable projection identity shared by iOS chat UI and canonical Runs.

import Foundation

nonisolated enum IOSCanonicalMessageIdentityError: LocalizedError, Equatable {
    case visibleAssistantMismatch(expected: UUID, actual: UUID)
    case invalidRecoveryIdentity

    var errorDescription: String? {
        switch self {
        case .invalidRecoveryIdentity: return "Invalid canonical Run recovery identity."
        case .visibleAssistantMismatch(let expected, let actual):
            return "Assistant message identity mismatch: expected \(expected), received \(actual)."
        }
    }
}

nonisolated struct IOSCanonicalAgentTurnIdentity: Equatable, Sendable {
    let submissionID: String
    let userMessageID: String
    let assistantMessageID: String
    let visibleUserMessageID: UUID
    let visibleAssistantMessageID: UUID

    /// Recovery references an admitted assistant, and never invents user input.
    init(recoveringSubmissionID: String, assistantMessageID: String) throws {
        for id in [recoveringSubmissionID, assistantMessageID] {
            guard !id.isEmpty, id.utf8.count <= 512,
                  id == id.trimmingCharacters(in: .whitespacesAndNewlines),
                  id.rangeOfCharacter(from: .controlCharacters) == nil else {
                throw IOSCanonicalMessageIdentityError.invalidRecoveryIdentity
            }
        }
        submissionID = recoveringSubmissionID
        userMessageID = ""
        self.assistantMessageID = assistantMessageID
        visibleUserMessageID = IOSCanonicalMessageIdentity.stableUUID(for: "")
        visibleAssistantMessageID = IOSCanonicalMessageIdentity.stableUUID(for: assistantMessageID)
    }

    init(queuedSubmissionID: String, userMessageID: String) {
        submissionID = queuedSubmissionID
        self.userMessageID = userMessageID
        assistantMessageID = "\(queuedSubmissionID):assistant"
        visibleUserMessageID = IOSCanonicalMessageIdentity.stableUUID(for: userMessageID)
        visibleAssistantMessageID = IOSCanonicalMessageIdentity.stableUUID(for: assistantMessageID)
    }

    init(automation: IOSAutomationExecutionIdentity) {
        submissionID = automation.submissionID
        userMessageID = automation.userMessageID
        assistantMessageID = automation.assistantMessageID
        visibleUserMessageID = IOSCanonicalMessageIdentity.stableUUID(for: automation.userMessageID)
        visibleAssistantMessageID = IOSCanonicalMessageIdentity.stableUUID(for: automation.assistantMessageID)
    }

    init(editedResend prepared: ApplePreparedEditedResend<AgentCoreJSONValue>) throws {
        guard prepared.run_id == prepared.input.operation_id,
              prepared.user_message_id == "\(prepared.run_id):user",
              prepared.assistant_message_id == "\(prepared.run_id):assistant",
              UUID(uuidString: prepared.run_id) != nil else {
            throw IOSCanonicalMessageIdentityError.invalidRecoveryIdentity
        }
        submissionID = prepared.run_id
        userMessageID = prepared.user_message_id
        assistantMessageID = prepared.assistant_message_id
        visibleUserMessageID = IOSCanonicalMessageIdentity.stableUUID(for: prepared.user_message_id)
        visibleAssistantMessageID = IOSCanonicalMessageIdentity.stableUUID(for: prepared.assistant_message_id)
    }

    init(userMessageID: UUID) {
        let submissionID = userMessageID.uuidString.lowercased()
        let assistantMessageID = "\(submissionID):assistant"
        let expectedVisibleAssistantID = IOSCanonicalMessageIdentity.stableUUID(
            for: assistantMessageID
        )
        self.submissionID = submissionID
        self.userMessageID = userMessageID.uuidString.lowercased()
        self.assistantMessageID = assistantMessageID
        self.visibleUserMessageID = userMessageID
        self.visibleAssistantMessageID = expectedVisibleAssistantID
    }

    init(
        userMessageID: UUID,
        validatingVisibleAssistantMessageID visibleAssistantMessageID: UUID
    ) throws {
        let identity = Self(userMessageID: userMessageID)
        if visibleAssistantMessageID != identity.visibleAssistantMessageID {
            throw IOSCanonicalMessageIdentityError.visibleAssistantMismatch(
                expected: identity.visibleAssistantMessageID,
                actual: visibleAssistantMessageID
            )
        }
        self = identity
    }
}

nonisolated enum IOSCanonicalMessageIdentity {
    static func stableUUID(for canonicalID: String) -> UUID {
        let bytes = Array(canonicalID.utf8)
        let first = fnv1a64(bytes, basis: 0xcbf29ce484222325)
        let second = fnv1a64(bytes.reversed(), basis: 0x84222325cbf29ce4)
        var output = withUnsafeBytes(of: first.bigEndian, Array.init)
            + withUnsafeBytes(of: second.bigEndian, Array.init)
        output[6] = (output[6] & 0x0f) | 0x50
        output[8] = (output[8] & 0x3f) | 0x80
        return UUID(uuid: (
            output[0], output[1], output[2], output[3],
            output[4], output[5], output[6], output[7],
            output[8], output[9], output[10], output[11],
            output[12], output[13], output[14], output[15]
        ))
    }

    private static func fnv1a64<S: Sequence>(_ bytes: S, basis: UInt64) -> UInt64
    where S.Element == UInt8 {
        bytes.reduce(basis) { hash, byte in
            (hash ^ UInt64(byte)) &* 0x100000001b3
        }
    }
}
