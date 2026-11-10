// SPDX-License-Identifier: Apache-2.0
// Transport values only. Core owns record validation, content roots and recovery.
import Foundation

typealias IOSAssistantJournalIdentity = AppleAssistantJournalIdentity

typealias IOSAssistantJournalCursor = AppleAssistantJournalCursor

typealias IOSAssistantJournalNotification = AppleAssistantJournalNotification

// This is source completion evidence, not a local completion flag. Application
// must admit its immutable manifest after all required journal heads are durable.
typealias IOSRunMessageAdmissionCursor = AppleRunMessageAdmissionCursor

typealias IOSRunMessageAdmissionPage = AppleRunMessageAdmissionPage<AgentCoreJSONValue>

// Transport retains the complete result evidence. Rust validates source ownership,
// required variant fields and digest before accepting any terminal publication.
public typealias IOSRunResult = AppleRunResult<AgentCoreJSONValue>

typealias IOSAssistantRunTerminalSeal = AppleAssistantRunTerminalSeal<AgentCoreJSONValue>

typealias IOSAssistantRunTerminalManifestEntry = AppleAssistantRunTerminalManifestEntry

typealias IOSAssistantRunTerminalManifestPage = AppleAssistantRunTerminalManifestPage<AgentCoreJSONValue>

typealias IOSAssistantRunTerminalReception = AppleAssistantRunTerminalReception<AgentCoreJSONValue>

typealias IOSAssistantJournalRecord = AppleAssistantJournalRecord<AgentCoreJSONValue>

typealias IOSAssistantJournalContentReference = AppleAssistantJournalContentReference

typealias IOSAssistantJournalContentNeed = AppleAssistantJournalContentNeed

typealias IOSAssistantJournalChunk = AppleAssistantJournalChunk

typealias IOSAssistantJournalSnapshot = AppleAssistantJournalSnapshot

typealias IOSAssistantJournalReceipt = AppleAssistantJournalReceipt

typealias IOSAssistantJournalReceptionState = AppleAssistantJournalReceptionState

typealias IOSAssistantJournalCatchUpError = AppleAssistantJournalCatchUpError

typealias IOSAssistantJournalPage = AppleAssistantJournalPage<AgentCoreJSONValue>

typealias IOSAssistantJournalReadRequest = AppleAssistantJournalReadRequest<AgentCoreJSONValue>

// JSON adapters retain each platform transport; protocol validation is shared.
extension AgentCoreJSONValue: JournalJSONValue {
    nonisolated public var journalObject: [String: Self]? { if case .object(let value) = self { return value }; return nil }
    nonisolated public var journalArray: [Self]? { if case .array(let value) = self { return value }; return nil }
    nonisolated public var journalString: String? { if case .string(let value) = self { return value }; return nil }
    nonisolated public var journalIsNull: Bool { self == .null }
}
