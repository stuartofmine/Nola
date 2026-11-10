// SPDX-License-Identifier: Apache-2.0
import Foundation
@testable import Nola

/// Strict read-only source for presentation tests. It emits journal metadata
/// and independent immutable field bodies, not the retired Part event protocol.
actor IOSJournalPresentationTestFixture {
    let cursor: IOSAssistantJournalCursor
    private let heads: [AgentCoreJSONValue]
    private let bodies: [String: [String: String]]
    private var opened = false
    private var released = false

    init(cursor: IOSAssistantJournalCursor, heads: [AgentCoreJSONValue], bodies: [String: [String: String]]) {
        self.cursor = cursor; self.heads = heads; self.bodies = bodies
    }

    func execute(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
        guard case .object(let fields) = command, case .string(let type)? = fields["type"] else { throw failure() }
        let snapshot = AgentCoreJSONValue.object(["snapshot_id": .string("snapshot"), "at": try Self.wire(cursor)])
        if type == "open_assistant_journal_snapshot" {
            guard !opened, fields["at"] == (try Self.wire(cursor)) else { throw failure() }
            opened = true; return snapshot
        }
        guard opened, !released, fields["snapshot"] == snapshot else { throw failure() }
        switch type {
        case "read_assistant_journal_parts":
            guard fields["after_part_id"] == .null else { throw failure() }
            return .object(["at": try Self.wire(cursor), "parts": .array(heads), "next_part_id": .null])
        case "read_assistant_journal_range":
            guard case .string(let id)? = fields["part_id"], case .object(let reference)? = fields["content"],
                  case .string(let name)? = reference["field"], let text = bodies[id]?[name],
                  case .string(let rawOffset)? = fields["offset_bytes"], let offset = Int(rawOffset),
                  fields["limit_bytes"] == .unsignedInteger(32_768) else { throw failure() }
            let bytes = Array(text.utf8)
            guard offset >= 0, offset < bytes.count else { throw failure() }
            var end = min(offset + 32_768, bytes.count)
            while end < bytes.count && bytes[end] & 0xC0 == 0x80 { end -= 1 }
            guard let page = String(bytes: bytes[offset..<end], encoding: .utf8) else { throw failure() }
            return .object(["offset_bytes": .string(rawOffset), "next_offset_bytes": .string(String(end)),
                "total_utf8_bytes": .string(String(bytes.count)), "text": .string(page), "end_of_content": .bool(end == bytes.count)])
        case "release_assistant_journal_snapshot": released = true; return .null
        default: throw failure()
        }
    }
    func didRelease() -> Bool { released }
    private func failure() -> Error { IOSApplicationRuntimeError.rejected("Unexpected journal test read") }
    nonisolated static func wire<T: Encodable>(_ value: T) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(value))
    }
}

@MainActor
func readJournalPresentation(_ publicParts: [[String: Any]], conversation: String = "conversation", run: String = "run",
                             assistant: String = "assistant", sequence: Int = 1) async throws -> IOSJournalMessageContent {
    let identity = IOSAssistantJournalIdentity(source_store_epoch: "test-source", conversation_id: conversation, run_id: run, assistant_message_id: assistant)
    let cursor = IOSAssistantJournalCursor(identity: identity, sequence: String(sequence), record_digest: String(repeating: "a", count: 64))
    var heads: [AgentCoreJSONValue] = [], bodies: [String: [String: String]] = [:]
    for (position, source) in publicParts.enumerated() {
        var metadata = source
        let id = source["part_id"] as! String, kind = source["kind"] as! String
        metadata.removeValue(forKey: "part_id"); metadata.removeValue(forKey: "kind"); metadata.removeValue(forKey: "revision")
        var content: [String: String] = [:]
        func field(_ key: String, _ name: String, structured: Bool = false) throws {
            guard let value = metadata.removeValue(forKey: key) else { return }
            if structured {
                let bytes = try JSONSerialization.data(withJSONObject: ["protocol_version": 1, "value": value], options: [.sortedKeys])
                content[name] = String(decoding: bytes, as: UTF8.self)
            } else { content[name] = value as? String }
        }
        if kind == "text" {
            try field("markdown", "markdown")
            metadata["visibility"] = ["pending", "rejected", "interrupted"].contains(metadata["review"] as? String ?? "") ? "withheld" : "public"
        }
        if kind == "reasoning" { try field("summary", "reasoning_summary") }
        if kind == "tool" {
            try field("input", "tool_input", structured: true); try field("output", "tool_output", structured: true)
            try field("output_preview", "tool_output_text")
            let artifacts = metadata["artifacts"] ?? []
            content["tool_artifacts"] = String(decoding: try JSONSerialization.data(withJSONObject: ["protocol_version": 1, "artifacts": artifacts]), as: UTF8.self)
            metadata["artifacts"] = "tool_artifacts"
            metadata["affected_resources"] = ["kind": "inline", "value": metadata["affected_resources"] ?? []]
            for group in ["origin", "permission"] {
                if var value = metadata[group] as? [String: Any] {
                    for key in ["provider_id", "provider_title", "namespace", "permission_domain", "rationale", "decision"] {
                        if let text = value[key] as? String { value[key] = ["kind": "inline", "value": text] }
                    }
                    metadata[group] = value
                }
            }
        }
        if kind == "source" {
            try field("snippet", "source_snippet")
            if let evidence = metadata["evidence"], !(evidence is NSNull) {
                content["source_evidence"] = String(decoding: try JSONSerialization.data(withJSONObject: ["protocol_version": 1, "evidence": evidence]), as: UTF8.self)
                metadata["evidence"] = "source_evidence"
            }
        }
        for key in ["title", "display_name", "uri", "mime_type", "name", "operation", "summary"] {
            if let text = metadata[key] as? String { metadata[key] = ["kind": "inline", "value": text] }
        }
        if var location = metadata["location"] as? [String: Any] {
            for key in ["page", "line_start", "line_end"] { if let value = location[key] { location[key] = String(describing: value) } }
            if let section = location["section"] as? String { location["section"] = ["kind": "inline", "value": section] }
            metadata["location"] = location
        }
        if kind == "citation" {
            let start = metadata.removeValue(forKey: "range_start")
            let end = metadata.removeValue(forKey: "range_end")
            guard (start == nil) == (end == nil) else {
                throw IOSApplicationRuntimeError.rejected("Incomplete journal citation byte range fixture")
            }
            if let start, let end {
                metadata["byte_range"] = ["start": String(describing: start), "end": String(describing: end)]
            }
        }
        for key in ["started_at_ms", "finished_at_ms"] { if let value = metadata[key], !(value is NSNull) { metadata[key] = String(describing: value) } }
        let references: [[String: Any]] = content.sorted(by: { $0.key < $1.key }).map { name, text in
            ["field": name, "content_generation": String(sequence), "utf8_bytes": String(text.utf8.count),
             "chunk_count": text.isEmpty ? "0" : "1", "content_root": String(repeating: "b", count: 64)]
        }
        let raw: [String: Any] = ["identity": ["source_store_epoch": identity.source_store_epoch, "conversation_id": conversation, "run_id": run, "assistant_message_id": assistant],
            "part_id": id, "generation": String(sequence), "revision": String(sequence), "metadata_revision": String(sequence),
            "order_key": ["section": "transcript", "position": String(position)], "metadata": ["kind": kind, "data": metadata], "bodies": references]
        heads.append(try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: raw)))
        bodies[id] = content
    }
    let source = IOSJournalPresentationTestFixture(cursor: cursor, heads: heads, bodies: bodies)
    let reader = IOSAssistantJournalReader { try await source.execute($0) }
    return try await reader.read(at: cursor)
}
