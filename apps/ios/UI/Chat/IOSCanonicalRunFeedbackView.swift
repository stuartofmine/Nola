// SPDX-License-Identifier: Apache-2.0
// Read-only result feedback remains attached to its canonical assistant on reopen.
import SwiftUI

nonisolated enum IOSCanonicalRunFeedback {
    static func read(cursor: IOSAssistantJournalCursor, store: any IOSAgentApplicationCommandStore) throws -> String? {
        struct Terminal: Decodable {
            let conversation_id: String
            let run_id: String
            let assistant_message_id: String
            let status: String
            let result: IOSRunResult
        }
        let value = try store.executeAgentCommand(.object(["type": .string("canonical_run_terminal"),
            "conversation_id": .string(cursor.identity.conversation_id), "run_id": .string(cursor.identity.run_id)]))
        guard value != .null else { return nil }
        let terminal = try JSONDecoder().decode(Terminal.self, from: JSONEncoder().encode(value))
        guard terminal.status == "sealed", terminal.conversation_id == cursor.identity.conversation_id,
              terminal.run_id == cursor.identity.run_id,
              terminal.assistant_message_id == cursor.identity.assistant_message_id,
              terminal.result.conversation_id == terminal.conversation_id,
              terminal.result.run_id == terminal.run_id else { throw IOSApplicationRuntimeError.invalidResponse }
        return terminal.result.value.blockedFeedback
    }
}

struct IOSCanonicalRunFeedbackView: View {
    let cursor: IOSAssistantJournalCursor
    let store: IOSApplicationRuntime
    let isRunning: Bool
    @State private var feedback: (owner: IOSAssistantJournalIdentity, text: String)?
    var body: some View {
        Group {
            if feedback?.owner == cursor.identity, let text = feedback?.text {
                Label(text, systemImage: "pause.circle")
                    .font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .accessibilityIdentifier("chat.canonical.blockedFeedback")
            }
        }
        .task(id: "\(cursor.identity)|\(cursor.sequence)|\(isRunning)") {
            guard !isRunning else { return }
            let captured = cursor
            let read = Task.detached(priority: .utility) { try IOSCanonicalRunFeedback.read(cursor: captured, store: store) }
            do {
                let text = try await withTaskCancellationHandler { try await read.value } onCancel: { read.cancel() }
                try Task.checkCancellation()
                feedback = text.map { (captured.identity, $0) }
            } catch { /* Never infer a successful result from an unavailable seal. */ }
        }
    }
}
