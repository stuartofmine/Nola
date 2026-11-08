// SPDX-License-Identifier: Apache-2.0
// Transport catchup only: Application owns authenticity, journals and terminal facts.
import Foundation

@MainActor final class IOSAssistantJournalConsumer {
    private let provider: WebAgentProvider
    private let store: any ConversationRemoteTurnStore
    private let conversationID: String
    private let runID: String
    private let assistantID: String
    private let sourceHostID: String
    private(set) var terminalStatus: String?
    private(set) var terminalResult: IOSRunResult?
    private(set) var admissionCursor: IOSRunMessageAdmissionCursor?
    private(set) var receivedAssistantMessageID: String?
    private(set) var receivedAssistantCursor: IOSAssistantJournalCursor?

    init(provider: WebAgentProvider, store: any ConversationRemoteTurnStore,
         conversationID: String, runID: String, assistantID: String) throws {
        guard let source = provider.journalSourceHostID, !source.isEmpty else {
            throw IOSApplicationRuntimeError.rejected("Journal requires an authenticated paired Host")
        }
        self.provider = provider; self.store = store; self.conversationID = conversationID
        self.runID = runID; self.assistantID = assistantID; sourceHostID = source
    }
    private func value<T: Encodable>(_ input: T) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(input))
    }
    private func command<T: Decodable>(_ type: String, _ fields: [String: AgentCoreJSONValue]) throws -> T {
        var fields = fields; fields["type"] = .string(type)
        let result = try store.executeRemoteJournalCommand(.object(fields))
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(result))
    }
    private func invalid() -> Error { IOSApplicationRuntimeError.rejected("Journal source, owner or cursor mismatch") }
    private func check(_ cursor: IOSAssistantJournalCursor) throws {
        guard cursor.identity.conversation_id == conversationID, cursor.identity.run_id == runID,
              let sequence = UInt64(cursor.sequence), String(sequence) == cursor.sequence else { throw invalid() }
    }
    func receiveHint(_ raw: AgentCoreJSONValue, terminal: Bool) async throws {
        struct Hint: Decodable { let session_id: String; let source_host_id: String; let run_id: String?; let seal_digest: String?; let through: IOSAssistantJournalCursor? }
        let hint = try JSONDecoder().decode(Hint.self, from: JSONEncoder().encode(raw))
        guard hint.session_id == conversationID, hint.source_host_id == sourceHostID else { throw invalid() }
        if terminal {
            guard hint.run_id == runID, let digest = hint.seal_digest else { throw invalid() }
            guard try await recoverTerminal(expectedDigest: digest) else { throw invalid() }
        } else {
            guard let through = hint.through else { throw invalid() }
            try check(through)
            let existing: IOSAssistantJournalReceptionState? = try? command("assistant_journal_reception_state",
                ["source_host_id": .string(sourceHostID), "through": try value(through)])
            if admissionCursor == nil || existing?.after.sequence == "0" || existing == nil { try await recoverAdmissions() }
            try await catchUp(through)
            receivedAssistantMessageID = through.identity.assistant_message_id
            receivedAssistantCursor = through
        }
    }

    func recover(contextUsage: IOSCommittedRunContextUsage? = nil) async throws {
        try await recoverAdmissions()
        if try await recoverTerminal(contextUsage: contextUsage) { return }
        var after: String?
        while true {
            try Task.checkCancellation()
            let heads: [IOSAssistantJournalCursor] = try await provider.journalRequest(
                .heads(conversationID: conversationID, runID: runID, afterMessageID: after))
            guard heads.count <= 64 else { throw invalid() }
            for head in heads {
                try check(head)
                guard after == nil || head.identity.assistant_message_id > after! else { throw invalid() }
                try await catchUp(head)
                after = head.identity.assistant_message_id
            }
            if heads.count < 64 { break }
        }
        _ = try await recoverContextUsage(contextUsage)
    }

    /// Both observation and live notifications settle through the same durable
    /// Core boundary. The notification's token counts are never authoritative.
    @discardableResult
    func recoverContextUsage(_ observed: IOSCommittedRunContextUsage? = nil) async throws -> IOSCommittedRunContextUsage? {
        if admissionCursor == nil { try await recoverAdmissions() }
        guard let owner = admissionCursor?.identity, owner.assistant_message_id == assistantID else { throw invalid() }
        let record: IOSCommittedRunContextUsage?
        if let observed { record = observed }
        else { record = try await provider.journalRequest(.contextUsage(owner)) }
        try Task.checkCancellation()
        guard let record else { return nil }
        try record.validate(owner: owner)
        try await recoverAdmissions(through: record.admission)
        try Task.checkCancellation()
        let committed: IOSCommittedRunContextUsage = try command("accept_remote_run_context_usage",
            ["source_host_id": .string(sourceHostID), "record": try value(record)])
        try committed.validate(owner: owner)
        return committed
    }
    private func recoverAdmissions(through required: IOSRunMessageAdmissionCursor? = nil) async throws {
        let through: IOSRunMessageAdmissionCursor
        if let required { through = required }
        else {
            through = try await provider.journalRequest(.admissionHead(conversationID: conversationID, runID: runID))
        }
        guard through.identity.conversation_id == conversationID, through.identity.run_id == runID,
              through.identity.assistant_message_id == assistantID,
              let limit = UInt64(through.sequence), String(limit) == through.sequence else { throw invalid() }
        var after: IOSRunMessageAdmissionCursor = try command("run_message_admission_reception_state",
            ["source_host_id": .string(sourceHostID), "through": try value(through)])
        guard after.identity == through.identity, let start = UInt64(after.sequence) else { throw invalid() }
        admissionCursor = after
        if start >= limit { return } // Core validates even an already covered historical cursor.
        while after != through {
            try Task.checkCancellation()
            let page: IOSRunMessageAdmissionPage = try await provider.journalRequest(.admissions(after: after, through: through))
            guard page.after == after, page.through == through, page.next.identity == through.identity,
                  let next = UInt64(page.next.sequence), let previous = UInt64(after.sequence),
                  next > previous, next <= limit, !page.records.isEmpty, page.records.count <= 64,
                  UInt64(page.records.count) == next-previous,
                  page.complete == (page.next == through) else { throw invalid() }
            let received: IOSRunMessageAdmissionCursor = try command("receive_run_message_admissions",
                ["source_host_id": .string(sourceHostID), "page": try value(page)])
            guard received == page.next else { throw invalid() }
            after = received
            admissionCursor = received
        }
    }
    private func catchUp(_ through: IOSAssistantJournalCursor) async throws {
        try check(through)
        func reception() throws -> IOSAssistantJournalReceptionState {
            try command("assistant_journal_reception_state", ["source_host_id": .string(sourceHostID), "through": try value(through)])
        }
        let initial: IOSAssistantJournalReceptionState
        do { initial = try reception() }
        catch {
            // A new segment can be published between admission-head and heads
            // reads. Refresh the immutable Run admissions, then let Core prove
            // the exact message owner; no arbitrary same-Run message is trusted.
            try await recoverAdmissions()
            initial = try reception()
        }
        var state = initial
        while !state.complete {
            try Task.checkCancellation()
            let page: IOSAssistantJournalPage = try await provider.journalRequest(.records(after: state.after, through: through))
            guard case .records(let records) = page else { throw IOSAssistantJournalCatchUpError.snapshotRequired }
            guard records.after == state.after, records.through == through, !records.records.isEmpty,
                  records.records.count <= 32, records.next == records.records.last?.cursor,
                  let start = UInt64(state.after.sequence), let next = UInt64(records.next.sequence),
                  let limit = UInt64(through.sequence), next > start, next <= limit,
                  records.complete == (records.next == through),
                  records.records.allSatisfy({ record in
                      guard record.header.identity == through.identity,
                            let sequence = UInt64(record.header.sequence) else { return false }
                      return sequence > start && sequence <= limit
                  }) else { throw invalid() }
            for record in records.records {
                try check(record.cursor)
                _ = try await ingest(record)
            }
            let previous = state.after
            state = try reception()
            guard state.complete || state.after != previous else { throw invalid() }
        }
    }
    private func ingest(_ record: IOSAssistantJournalRecord) async throws -> IOSAssistantJournalReceipt {
        let recordValue = try value(record)
        let fields: [String: AgentCoreJSONValue] = ["source_host_id": .string(sourceHostID), "record": recordValue]
        var need: IOSAssistantJournalContentNeed? = try command("next_assistant_journal_content", fields)
        var snapshot: IOSAssistantJournalSnapshot?
        do {
            if need != nil {
                let opened: IOSAssistantJournalSnapshot = try await provider.journalRequest(.open(record.cursor))
                guard opened.at == record.cursor else { throw invalid() }; snapshot = opened
            }
            while let current = need {
                try Task.checkCancellation()
                guard let snapshot else { throw invalid() }
                let chunk: IOSAssistantJournalChunk = try await provider.journalRequest(.chunk(snapshot, current))
                let _: IOSAssistantJournalContentReference = try command("stage_assistant_journal_content", ["input": .object([
                    "source_host_id": .string(sourceHostID), "record": recordValue, "part_id": .string(current.part_id),
                    "content": try value(current.content), "expected_prefix": try value(current.expected_prefix),
                    "chunks": .array([try value(chunk)]), "restart": .bool(false),
                ])])
                need = try command("next_assistant_journal_content", fields)
                guard need != current else { throw invalid() }
            }
            var receiveFields = fields; receiveFields["chunks"] = .array([])
            let receipt: IOSAssistantJournalReceipt = try command("receive_assistant_journal_record", receiveFields)
            guard receipt.cursor == record.cursor else { throw invalid() }
            if let snapshot { let _: AgentCoreJSONValue? = try? await provider.journalRequest(.release(snapshot)) }
            return receipt
        } catch {
            if let snapshot { let _: AgentCoreJSONValue? = try? await provider.journalRequest(.release(snapshot)) }
            throw error
        }
    }
    /// EOF and reconnect re-read the source seal; EOF itself never completes a Run.
    @discardableResult func recoverTerminal(expectedDigest: String? = nil,
                                            contextUsage: IOSCommittedRunContextUsage? = nil) async throws -> Bool {
        let seal: IOSAssistantRunTerminalSeal? = try await provider.journalRequest(.terminalSeal(conversationID: conversationID, runID: runID))
        guard let seal else { return false }
        guard seal.protocol_version == 1,
              seal.identity.conversation_id == conversationID, seal.identity.run_id == runID,
              seal.result.conversation_id == conversationID, seal.result.run_id == runID,
              seal.identity.assistant_message_id == assistantID,
              expectedDigest == nil || expectedDigest == seal.seal_digest,
              let total = UInt64(seal.entry_count) else { throw invalid() }
        try await recoverAdmissions(through: seal.admission)
        let fields: [String: AgentCoreJSONValue] = ["source_host_id": .string(sourceHostID), "seal": try value(seal)]
        var state: IOSAssistantRunTerminalReception = try command("begin_receive_assistant_run_terminal", fields)
        guard state.seal == seal else { throw invalid() }
        while !state.manifest_complete {
            try Task.checkCancellation()
            let page: IOSAssistantRunTerminalManifestPage = try await provider.journalRequest(.terminalManifest(seal, afterEntry: state.next_entry))
            guard page.seal == seal, page.after_entry == state.next_entry,
                  let next = UInt64(page.next_entry), let start = UInt64(page.after_entry),
                  next > start, next <= total, page.entries.count <= 64,
                  UInt64(page.entries.count) == next-start else { throw invalid() }
            state = try command("stage_assistant_run_terminal_manifest", ["source_host_id": .string(sourceHostID), "page": try value(page)])
            guard state.seal == seal, state.next_entry == page.next_entry else { throw invalid() }
        }
        var after: UInt64 = 0
        while after < total {
            let page: IOSAssistantRunTerminalManifestPage = try await provider.journalRequest(.terminalManifest(seal, afterEntry: String(after)))
            guard page.seal == seal, page.after_entry == String(after), let next = UInt64(page.next_entry),
                  next > after, next <= total, UInt64(page.entries.count) == next-after else { throw invalid() }
            for entry in page.entries {
                if case .assistant(let through, _, _, _, _) = entry { try await catchUp(through) }
            }
            after = next
        }
        // A lost usage notification must not leave a cold receiver without the
        // source's committed statistics once terminal recovery has completed.
        _ = try await recoverContextUsage(contextUsage)
        try Task.checkCancellation()
        let committed: IOSAssistantRunTerminalReception = try command("commit_received_assistant_run_terminal", fields)
        guard committed.seal == seal, committed.committed, committed.manifest_complete,
              committed.next_entry == seal.entry_count else { throw invalid() }
        terminalResult = seal.result
        terminalStatus = "sealed"
        return true
    }
}
