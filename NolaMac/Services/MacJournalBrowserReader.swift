// SPDX-License-Identifier: Apache-2.0
import Foundation

/// Browser chrome is derived from committed tool results, never trace or a
/// second Part mirror. Large bodies remain available in the journal inspector.
actor MacJournalBrowserReader {
    struct Update: Sendable, Equatable {
        let toolName: String
        let state: String
        let arguments: MacAgentTraceJSONValue?
        let output: MacAgentTraceJSONValue?
        let finishedAt: Int64
        let key: String
    }
    private struct State {
        let reference: MacJournalMessageReference
        let parts: [MacAssistantJournalPart]
    }
    private struct ToolKey: Hashable {
        let owner: MacJournalOwnerKey
        let partID: String
    }
    private var states: [MacJournalOwnerKey: State] = [:]
    private var bodies: [MacJournalFieldReadKey: MacAgentTraceJSONValue] = [:]
    private var updates: [ToolKey: (MacJournalOwnerKey, Update)] = [:]

    func read(_ reference: MacJournalMessageReference) async throws -> [Update] {
        let identity = reference.cursor.identity
        let owner = reference.ownerKey
        let prior = states[owner]
        if let prior, prior.reference.cursor == reference.cursor { return current(owner) }
        let parts: [MacAssistantJournalPart]
        if let prior {
            parts = try await MacJournalTextRead.updatedParts(reference, from: prior.reference, existing: prior.parts)
        } else { parts = try await MacJournalTextRead.parts(reference) }
        var next: [ToolKey: (MacJournalOwnerKey, Update)] = [:]
        for part in parts {
            guard case .tool(let tool) = part.metadata,
                  tool.capability_id.hasPrefix("browser_") || tool.capability_id.hasPrefix("chrome_native_"),
                  tool.state == "completed" || tool.state == "failed" else { continue }
            let key = identity.assistant_message_id + ":" + part.part_id
            let input = try await body("tool_input", part: part, reference: reference)
            var output = try await body("tool_output", part: part, reference: reference)
            if output == nil { output = try await body("tool_output_text", part: part, reference: reference) }
            next[ToolKey(owner: owner, partID: part.part_id)] = (owner, Update(toolName: tool.capability_id, state: tool.state,
                arguments: input, output: output,
                finishedAt: tool.finished_at_ms.flatMap(Int64.init) ?? 0, key: key))
        }
        try Task.checkCancellation()
        if let newer = states[owner], (UInt64(newer.reference.cursor.sequence) ?? 0) > (UInt64(reference.cursor.sequence) ?? 0) {
            return current(owner)
        }
        states[owner] = State(reference: reference, parts: parts)
        updates = updates.filter { $0.value.0 != owner }
        updates.merge(next, uniquingKeysWith: { _, latest in latest })
        return current(owner)
    }

    private func current(_ owner: MacJournalOwnerKey) -> [Update] {
        updates.values.filter { $0.0.databaseURL == owner.databaseURL
            && $0.0.identity.source_store_epoch == owner.identity.source_store_epoch
            && $0.0.identity.conversation_id == owner.identity.conversation_id
            && $0.0.identity.run_id == owner.identity.run_id }
            .map(\.1).sorted { ($0.finishedAt, $0.key) < ($1.finishedAt, $1.key) }
    }

    private func body(_ field: String, part: MacAssistantJournalPart,
                      reference: MacJournalMessageReference) async throws -> MacAgentTraceJSONValue? {
        guard let content = part.bodies.first(where: { $0.field == field }) else { return nil }
        let key = MacJournalFieldReadKey(databaseURL: reference.databaseURL, identity: part.identity,
            partID: part.part_id, content: content)
        if let cached = bodies[key] { return cached }
        guard let length = UInt64(content.utf8_bytes), length <= 262_144 else { return nil }
        var text = "", offset = "0"
        repeat {
            let range = try await MacJournalTextRead.range(reference, partID: part.part_id, content: content, offset: offset)
            text += range.text
            if range.end_of_content { break }
            offset = range.next_offset_bytes
        } while true
        let display = try MacJournalBodyPresentation.toolField(field).display(text, complete: true)
        let value: MacAgentTraceJSONValue
        if field == "tool_output_text" { value = .string(display) }
        else { value = try JSONDecoder().decode(MacAgentTraceJSONValue.self, from: Data(display.utf8)) }
        bodies[key] = value
        return value
    }
}
