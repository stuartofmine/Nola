// SPDX-License-Identifier: Apache-2.0
// Read-only presentation of committed journal content; never an execution grant.
import Foundation

/// Metadata admission is read-only presentation work. A temporarily unavailable
/// window must not tear down the independently owned execution event stream.
@MainActor
enum IOSLocalJournalWindowAdmission {
    static func read<Value: Sendable>(isCurrent: () -> Bool, operation: @MainActor () async throws -> Value,
        sleep: (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) async throws -> Value? {
        for attempt in 0..<3 {
            try Task.checkCancellation()
            guard isCurrent() else { return nil }
            do {
                let value = try await operation()
                try Task.checkCancellation()
                guard isCurrent() else { return nil }
                return value
            } catch {
                guard isTransient(error) else { throw error }
                guard attempt < 2 else { return nil }
                try await sleep(UInt64(attempt + 1) * 50_000_000)
            }
        }
        return nil
    }

    static func load(cursor: IOSAssistantJournalCursor,
        isCurrent: () -> Bool,
        readWindow: (Int64?) throws -> IOSConversationProjectionWindowV1,
        sleep: (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) async throws -> IOSConversationProjectionWindowV1? {
        var before: Int64?
        // Bound both retry work and historical lookup. Missing metadata remains
        // unadmitted, allowing a later publication/observation to retry.
        for _ in 0..<4 {
            var loaded: IOSConversationProjectionWindowV1?
            for attempt in 0..<3 {
                try Task.checkCancellation()
                guard isCurrent() else { return nil }
                do { loaded = try readWindow(before); break }
                catch {
                    guard isTransient(error) else { throw error }
                    guard attempt < 2 else { return nil }
                    try await sleep(UInt64(attempt + 1) * 50_000_000)
                }
            }
            try Task.checkCancellation()
            guard isCurrent() else { return nil }
            guard let window = loaded,
                  window.projection.conversationID == cursor.identity.conversation_id else {
                throw IOSApplicationRuntimeError.rejected("Journal metadata has a foreign conversation owner")
            }
            if let message = window.projection.messages.first(where: { $0.messageID == cursor.identity.assistant_message_id }) {
                guard message.role == .assistant,
                      window.assistantJournalHeads.contains(where: { $0.identity == cursor.identity }) else {
                    throw IOSApplicationRuntimeError.rejected("Journal metadata has a foreign message or Run owner")
                }
                return window
            }
            guard window.hasMore else { return nil }
            guard let next = window.beforeOrdinal, before == nil || next < before! else {
                throw IOSApplicationRuntimeError.rejected("Journal metadata window cursor did not advance")
            }
            before = next
        }
        return nil
    }

    private static func isTransient(_ error: Error) -> Bool {
        guard let error = error as? IOSApplicationRuntimeError else { return false }
        switch error {
        case .applicationUnavailable, .openFailed, .outcomePending: return true
        default: return false // Never retry protocol, authorization or identity rejection.
        }
    }
}

/// One local execution owner, shared by hot execution and permission recovery.
/// Only first admission of a Message needs a metadata window; body deltas do not.
nonisolated struct IOSLocalJournalPublicationScope: Sendable {
    let conversationID: String
    let runID: String
    private var epoch: String?
    private var admittedMessages = Set<String>()

    init(conversationID: String, runID: String) {
        self.conversationID = conversationID
        self.runID = runID
    }

    func needsAdmission(_ cursor: IOSAssistantJournalCursor) throws -> Bool {
        guard cursor.identity.conversation_id == conversationID,
              cursor.identity.run_id == runID,
              !cursor.identity.source_store_epoch.isEmpty,
              epoch == nil || epoch == cursor.identity.source_store_epoch else {
            throw IOSApplicationRuntimeError.rejected("Local journal publication owner mismatch")
        }
        return !admittedMessages.contains(cursor.identity.assistant_message_id)
    }

    mutating func didAdmit(_ cursor: IOSAssistantJournalCursor) throws {
        _ = try needsAdmission(cursor)
        epoch = cursor.identity.source_store_epoch
        admittedMessages.insert(cursor.identity.assistant_message_id)
    }
}

nonisolated struct IOSJournalMessageContent: Equatable, Sendable {
    let cursor: IOSAssistantJournalCursor
    let parts: [IOSConversationProjectionV1.Part]
    var text: String {
        parts.compactMap { if case .text(let part) = $0, part.isDisclosed { return part.markdown }; return nil }.joined()
    }
    var reasoning: String {
        parts.compactMap { if case .reasoning(let part) = $0 { return part.summary }; return nil }.joined()
    }
}

/// One reader can follow local Application or authenticated remote read commands.
/// It caches cursors, immutable content references and already-read UTF-8 prefixes.
actor IOSAssistantJournalReader {
    typealias Transport = @Sendable (AgentCoreJSONValue) async throws -> AgentCoreJSONValue
    private let transport: Transport
    private var messages: [IOSAssistantJournalIdentity: State] = [:]
    private struct Body: Sendable { let reference: Content; let text: String }
    private struct CachedPart: Sendable { let head: Head; let bodies: [String: Body] }
    private struct State: Sendable { let cursor: IOSAssistantJournalCursor; let parts: [String: CachedPart] }
    private struct Snapshot: Codable, Equatable, Sendable { let snapshot_id: String; let at: IOSAssistantJournalCursor }
    private struct Content: Codable, Equatable, Sendable {
        let field: String; let content_generation: String; let utf8_bytes: String
        let chunk_count: String; let content_root: String
    }
    private struct Order: Codable, Sendable { let section: String; let position: String }
    private struct Metadata: Codable, Sendable { let kind: String; let data: [String: AgentCoreJSONValue] }
    private struct Head: Codable, Sendable {
        let identity: IOSAssistantJournalIdentity; let part_id: String
        let generation: String; let revision: String; let metadata_revision: String
        let order_key: Order; let metadata: Metadata; let bodies: [Content]
    }
    private struct Change: Decodable, Sendable { let part_id: String; let head: Head? }
    private struct Page: Decodable, Sendable {
        let base: IOSAssistantJournalCursor; let at: IOSAssistantJournalCursor
        let changes: [Change]; let next_part_id: String?
    }
    private struct PartsPage: Decodable, Sendable {
        let at: IOSAssistantJournalCursor; let parts: [Head]; let next_part_id: String?
    }
    private struct Range: Decodable, Sendable {
        let offset_bytes: String; let next_offset_bytes: String; let total_utf8_bytes: String
        let text: String; let end_of_content: Bool
    }

    init(transport: @escaping Transport) { self.transport = transport }
    init(store: any IOSAgentApplicationCommandStore) {
        transport = { try store.executeAgentCommand($0) }
    }
    private func invalid() -> Error { IOSApplicationRuntimeError.rejected("Invalid committed journal presentation") }
    private func number(_ value: String) throws -> UInt64 {
        guard let value64 = UInt64(value), String(value64) == value else { throw invalid() }
        return value64
    }
    private func wire<T: Encodable>(_ value: T) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value))
    }
    private func command<T: Decodable>(_ type: String, _ fields: [String: AgentCoreJSONValue]) async throws -> T {
        var body = fields; body["type"] = .string(type)
        let value = try await transport(.object(body))
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    private func release(_ snapshot: Snapshot) async {
        if let value = try? wire(snapshot) {
            _ = try? await transport(.object(["type": .string("release_assistant_journal_snapshot"), "snapshot": value]))
        }
    }

    func read(at cursor: IOSAssistantJournalCursor) async throws -> IOSJournalMessageContent {
        let sequence = try number(cursor.sequence)
        guard !cursor.identity.source_store_epoch.isEmpty, !cursor.identity.conversation_id.isEmpty,
              !cursor.identity.run_id.isEmpty, !cursor.identity.assistant_message_id.isEmpty,
              cursor.record_digest.count == 64,
              cursor.record_digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { throw invalid() }
        if let cached = messages[cursor.identity], try number(cached.cursor.sequence) >= sequence {
            if cached.cursor.sequence == cursor.sequence, cached.cursor != cursor { throw invalid() }
            return try presentation(cached)
        }
        let prior = messages[cursor.identity]
        let snapshot: Snapshot = try await command("open_assistant_journal_snapshot", ["at": try wire(cursor)])
        guard snapshot.at == cursor else { await release(snapshot); throw invalid() }
        do {
            var parts = prior?.parts ?? [:]
            var after: String?
            var directoryOrder: (Int, UInt64, String)?
            var directoryParts = Set<String>()
            var continuations = Set<String>()
            repeat {
                try Task.checkCancellation()
                var request: [String: AgentCoreJSONValue] = [
                    "snapshot": try wire(snapshot),
                    "after_part_id": after.map(AgentCoreJSONValue.string) ?? .null,
                    "max_records": .unsignedInteger(64), "max_encoded_bytes": .unsignedInteger(262_144)]
                let changes: [Change], nextPartID: String?
                if let prior {
                    request["base"] = try wire(prior.cursor)
                    let page: Page = try await command("read_assistant_journal_part_changes", request)
                    guard page.at == cursor, page.base == prior.cursor else { throw invalid() }
                    changes = page.changes; nextPartID = page.next_part_id
                } else {
                    let page: PartsPage = try await command("read_assistant_journal_parts", request)
                    guard page.at == cursor else { throw invalid() }
                    changes = page.parts.map { Change(part_id: $0.part_id, head: $0) }; nextPartID = page.next_part_id
                }
                guard changes.count <= 64 else { throw invalid() }
                var last = after
                for change in changes {
                    guard !change.part_id.isEmpty else { throw invalid() }
                    if prior != nil {
                        guard last.map({ $0.utf8.lexicographicallyPrecedes(change.part_id.utf8) }) ?? true else { throw invalid() }
                    } else {
                        guard directoryParts.insert(change.part_id).inserted else { throw invalid() }
                    }
                    last = change.part_id
                    guard let head = change.head else { parts.removeValue(forKey: change.part_id); continue }
                    guard head.identity == cursor.identity, head.part_id == change.part_id, head.bodies.count <= 16,
                          Set(head.bodies.map(\.field)).count == head.bodies.count else { throw invalid() }
                    _ = try number(head.revision); _ = try number(head.generation); _ = try number(head.metadata_revision); _ = try number(head.order_key.position)
                    guard let section = ["transcript", "tasks", "skills", "model_attempts"].firstIndex(of: head.order_key.section) else { throw invalid() }
                    if prior == nil {
                        // A cold directory follows display order, not Part ID
                        // order. Only the incremental changes endpoint is keyed
                        // lexically. Keep this frontier across cold pages.
                        let order = (section, try number(head.order_key.position), head.part_id)
                        guard directoryOrder.map({ previous in
                            if previous.0 != order.0 { return previous.0 < order.0 }
                            if previous.1 != order.1 { return previous.1 < order.1 }
                            return previous.2.utf8.lexicographicallyPrecedes(order.2.utf8)
                        }) ?? true else { throw invalid() }
                        directoryOrder = order
                    }
                    let old = parts[head.part_id]
                    var bodies: [String: Body] = [:]
                    // Withheld text is never fetched, cached or displayed.
                    if head.metadata.kind != "text" || head.metadata.data["visibility"] == .string("public") {
                        for content in head.bodies {
                            let cached = old?.head.generation == head.generation ? old?.bodies[content.field] : nil
                            bodies[content.field] = try await readBody(content, cached: cached,
                                snapshot: snapshot, partID: head.part_id)
                        }
                    }
                    parts[head.part_id] = CachedPart(head: head, bodies: bodies)
                }
                if let next = nextPartID {
                    guard !next.isEmpty, next != after, continuations.insert(next).inserted else { throw invalid() }
                    if prior != nil {
                        guard last == next, after.map({ $0.utf8.lexicographicallyPrecedes(next.utf8) }) ?? true else { throw invalid() }
                    }
                    // Cold continuation is the last *scanned* directory key.
                    // Retired/post-snapshot entries may yield an empty page or
                    // a continuation absent from the returned visible heads.
                }
                after = nextPartID
            } while after != nil
            let next = State(cursor: cursor, parts: parts)
            // Actor transport suspension may let a newer snapshot finish first.
            if let current = messages[cursor.identity], current.cursor.sequence == cursor.sequence,
               current.cursor != cursor { throw invalid() }
            if let newer = messages[cursor.identity], try number(newer.cursor.sequence) > sequence {
                await release(snapshot)
                return try presentation(newer)
            }
            let result = try presentation(next)
            messages[cursor.identity] = next
            await release(snapshot)
            return result
        } catch { await release(snapshot); throw error }
    }

    private func readBody(_ content: Content, cached: Body?, snapshot: Snapshot, partID: String) async throws -> Body {
        let total = try number(content.utf8_bytes)
        _ = try number(content.content_generation); _ = try number(content.chunk_count)
        var text = ""
        if let cached, cached.reference.content_generation == content.content_generation {
            let previous = try number(cached.reference.utf8_bytes)
            guard previous <= total, previous == UInt64(cached.text.utf8.count),
                  previous != total || cached.reference.content_root == content.content_root else { throw invalid() }
            text = cached.text
        }
        var offset = UInt64(text.utf8.count)
        while offset < total {
            try Task.checkCancellation()
            let range: Range = try await command("read_assistant_journal_range", [
                "snapshot": try wire(snapshot), "part_id": .string(partID), "content": try wire(content),
                "offset_bytes": .string(String(offset)), "limit_bytes": .unsignedInteger(32_768)])
            let next = try number(range.next_offset_bytes)
            guard try number(range.offset_bytes) == offset, try number(range.total_utf8_bytes) == total,
                  next > offset, next <= total, next - offset == UInt64(range.text.utf8.count),
                  range.text.utf8.count <= 32_768, range.end_of_content == (next == total) else { throw invalid() }
            text.append(range.text); offset = next
        }
        return Body(reference: content, text: text)
    }

    private func presentation(_ state: State) throws -> IOSJournalMessageContent {
        let sections = ["transcript", "tasks", "skills", "model_attempts"]
        let ordered = state.parts.values.sorted { left, right in
            let lhs = sections.firstIndex(of: left.head.order_key.section) ?? 4
            let rhs = sections.firstIndex(of: right.head.order_key.section) ?? 4
            if lhs != rhs { return lhs < rhs }
            let lpos = UInt64(left.head.order_key.position) ?? 0, rpos = UInt64(right.head.order_key.position) ?? 0
            return lpos == rpos ? left.head.part_id.utf8.lexicographicallyPrecedes(right.head.part_id.utf8) : lpos < rpos
        }
        return IOSJournalMessageContent(cursor: state.cursor, parts: try ordered.compactMap(present))
    }

    private func present(_ part: CachedPart) throws -> IOSConversationProjectionV1.Part? {
        let head = part.head
        let body: (String) -> String = { part.bodies[$0]?.text ?? "" }
        func text(_ value: AgentCoreJSONValue?) throws -> AgentCoreJSONValue {
            guard let value, value != .null else { return .null }
            guard case .object(let reference) = value, case .string(let kind)? = reference["kind"],
                  case .string(let content)? = reference["value"] else { throw invalid() }
            switch kind {
            case "inline": return .string(content)
            case "field":
                guard let value = part.bodies[content] else { throw invalid() }
                return .string(value.text)
            default: throw invalid()
            }
        }
        func structured(_ field: String, _ key: String = "value") throws -> AgentCoreJSONValue {
            let content = body(field)
            guard !content.isEmpty else { return .null }
            let value = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(content.utf8))
            guard case .object(let envelope) = value, envelope.count == 2,
                  envelope["protocol_version"] == .number(1) || envelope["protocol_version"] == .unsignedInteger(1)
                    || envelope["protocol_version"] == .signedInteger(1), let result = envelope[key] else { throw invalid() }
            return result
        }
        var fields = head.metadata.data
        var kind = head.metadata.kind
        if kind == "task_control", case .object(let task)? = fields["task"] {
            let guidance = fields["supports_guidance"]
            let cancellation = fields["supports_cancellation"]
            fields = task
            fields["supports_guidance"] = guidance
            fields["supports_cancellation"] = cancellation
            fields["parent_run_id"] = .string(head.identity.run_id)
        }
        fields["kind"] = .string(kind); fields["part_id"] = .string(head.part_id)
        fields["revision"] = .unsignedInteger(try number(head.revision))
        for name in ["title", "display_name", "uri", "mime_type", "name", "operation", "summary"] {
            if kind == "ui_surface", name == "summary" { continue }
            if let value = fields[name], value != .null { fields[name] = try text(value) }
        }
        switch kind {
        case "text":
            fields["markdown"] = .string(fields["visibility"] == .string("public") ? body("markdown") : "")
        case "reasoning": fields["summary"] = .string(body("reasoning_summary"))
        case "tool":
            fields["input"] = try structured("tool_input")
            fields["output"] = part.bodies["tool_output"] == nil ? nil : try structured("tool_output")
            fields["output_preview"] = part.bodies["tool_output_text"].map { .string($0.text) }
            fields["artifacts"] = try structured("tool_artifacts", "artifacts")
            if fields["artifacts"] == .null { fields["artifacts"] = .array([]) }
            if case .object(let resources)? = fields["affected_resources"] {
                fields["affected_resources"] = resources["kind"] == .string("inline") ? resources["value"] : try structured("tool_resources", "resources")
            }
            for (name, keys) in [("permission", ["permission_domain", "rationale", "decision"]),
                                  ("origin", ["provider_id", "provider_title", "namespace"])] {
                if case .object(var object)? = fields[name] {
                    for key in keys { if object[key] != nil { object[key] = try text(object[key]) } }
                    fields[name] = .object(object)
                }
            }
        case "source": fields["snippet"] = .string(body("source_snippet")); fields["evidence"] = try structured("source_evidence", "evidence")
        case "citation":
            if case .object(var location)? = fields["location"] {
                if location["section"] != nil { location["section"] = try text(location["section"]) }
                for key in ["page", "line_start", "line_end"] {
                    if case .string(let count)? = location[key] { location[key] = .unsignedInteger(try number(count)) }
                }
                fields["location"] = .object(location)
                for key in ["page", "line_start", "line_end", "section"] { fields[key] = location[key] }
            }
            if case .object(let range)? = fields["byte_range"] {
                for (source, target) in [("start", "range_start"), ("end", "range_end")] {
                    if case .string(let count)? = range[source] { fields[target] = .unsignedInteger(try number(count)) }
                }
            }
        case "task", "task_control", "skill":
            fields["current_step"] = .string(body("task_current_step")); fields["result_ref"] = .string(body("result_reference"))
            if kind == "skill" { fields["failure"] = .string(body("failure_message")) }
            if case .object(let progress)? = fields["progress"], case .string(let bits)? = progress["fraction_bits"] {
                let value = Double(bitPattern: try number(bits))
                guard value.isFinite, (0...1).contains(value) else { throw invalid() }
                fields["progress"] = .number(value)
            }
        case "model_attempt":
            guard let code = fields["failure_code"], let retryable = fields["retryable"] else { throw invalid() }
            fields["failure"] = .object(["code": code, "retryable": retryable, "message": .string(body("failure_message"))])
        case "attachment", "error", "ui_surface": break
        case "step", "historical":
            // Kept as explicitly typed journal-only presentation, not forged
            // as a tool, permission challenge or canonical text message.
            return .journalDetail(id: head.part_id, revision: try number(head.revision), kind: kind,
                title: head.metadata.data["title"] == nil ? "" : try text(head.metadata.data["title"]).displayText,
                content: body(kind == "historical" ? "historical_preview" : "reasoning_summary"))
        default: throw invalid()
        }
        for key in ["started_at_ms", "finished_at_ms", "model_turns", "tool_calls", "elapsed_millis"] {
            if case .string(let count)? = fields[key] { fields[key] = .unsignedInteger(try number(count)) }
        }
        let result = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: JSONEncoder().encode(AgentCoreJSONValue.object(fields)))
        if case .tool(let tool) = result, let full = tool.fullOutput,
           full.messageID != head.identity.assistant_message_id { throw invalid() }
        if case .uiSurface(let value) = result {
            guard value.surface.revision > 0,
                  value.surface.identity.owner.message_id == head.identity.assistant_message_id,
                  value.surface.identity.owner.conversation_id == head.identity.conversation_id else { throw invalid() }
        }
        return result
    }
}
