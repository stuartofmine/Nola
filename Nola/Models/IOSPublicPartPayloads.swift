// SPDX-License-Identifier: Apache-2.0
// Typed, read-only public Part facts. No payload here is an execution grant.
import Foundation

extension IOSConversationProjectionV1 {
    typealias TextReview = AgentCoreTextReview

    nonisolated struct TextPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let markdown: String
        let isFinal: Bool
        let review: TextReview?
        var isDisclosed: Bool { review == nil || review == .approved }
        private enum CodingKeys: String, CodingKey {
            case revision, markdown, review
            case partID = "part_id", isFinal = "is_final"
        }
    }

    nonisolated struct ReasoningPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let state: ActivityState
        let title: String
        let summary: String?
        private enum CodingKeys: String, CodingKey { case revision, state, title, summary; case partID = "part_id" }
    }

    /// Exact owner tuple, never a private host path or a read capability.
    nonisolated struct FullOutput: Decodable, Equatable, Sendable {
        let messageID: String
        let partID: String
        let toolCallID: String
        private enum CodingKeys: String, CodingKey, CaseIterable {
            case messageID = "message_id", partID = "part_id", toolCallID = "tool_call_id"
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
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Full output contains only an exact public owner"))
            }
            let values = try decoder.container(keyedBy: CodingKeys.self)
            messageID = try values.decode(String.self, forKey: .messageID)
            partID = try values.decode(String.self, forKey: .partID)
            toolCallID = try values.decode(String.self, forKey: .toolCallID)
            guard [messageID, partID, toolCallID].allSatisfy({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Empty full output owner"))
            }
        }
    }

    nonisolated struct ToolExecutionSource: Decodable, Equatable, Sendable {
        enum Kind: String, Decodable, Sendable { case model, codeCell = "code_cell" }
        let kind: Kind
        let cell: CodeCellIdentity?
        private enum CodingKeys: String, CodingKey { case kind, cell }
        private struct SourceKey: CodingKey {
            let stringValue: String
            let intValue: Int? = nil
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { return nil }
        }
        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            kind = try values.decode(Kind.self, forKey: .kind)
            let keys = try decoder.container(keyedBy: SourceKey.self)
            let expected: Set<String> = kind == .model ? ["kind"] : ["kind", "cell"]
            guard Set(keys.allKeys.map(\.stringValue)) == expected else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Unexpected tool execution source fields"))
            }
            cell = try values.decodeIfPresent(CodeCellIdentity.self, forKey: .cell)
            guard (kind == .codeCell) == (cell != nil) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Tool execution source and cell identity disagree"))
            }
        }
    }

    nonisolated struct CodeCellIdentity: Decodable, Equatable, Sendable {
        let hostID: String
        let conversationID: String
        let turnID: String
        let parentCallID: String
        let cellID: String
        let generation: String
        private enum CodingKeys: String, CodingKey, CaseIterable {
            case hostID = "host_id", conversationID = "conversation_id", turnID = "turn_id"
            case parentCallID = "parent_call_id", cellID = "cell_id", generation
        }
        private struct SourceKey: CodingKey {
            let stringValue: String
            let intValue: Int? = nil
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { return nil }
        }
        init(from decoder: Decoder) throws {
            let keys = try decoder.container(keyedBy: SourceKey.self)
            guard Set(keys.allKeys.map(\.stringValue)) == Set(CodingKeys.allCases.map(\.rawValue)) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Code cell identity requires exactly six owner fields"))
            }
            let values = try decoder.container(keyedBy: CodingKeys.self)
            hostID = try values.decode(String.self, forKey: .hostID)
            conversationID = try values.decode(String.self, forKey: .conversationID)
            turnID = try values.decode(String.self, forKey: .turnID)
            parentCallID = try values.decode(String.self, forKey: .parentCallID)
            cellID = try values.decode(String.self, forKey: .cellID)
            generation = try values.decode(String.self, forKey: .generation)
            guard [hostID, conversationID, turnID, parentCallID, cellID, generation].allSatisfy({
                !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && $0.utf8.count <= 512
                    && $0.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
            }) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                    debugDescription: "Invalid code cell provenance identity"))
            }
        }
    }

    nonisolated struct ToolPart: Decodable, Equatable, Sendable {
        let executionSource: ToolExecutionSource
        let partID: String
        let revision: UInt64
        let toolCallID: String
        let capabilityID: String
        let state: ActivityState
        let displayKey: String?
        let title: String
        let summary: String?
        let origin: AgentCoreCapabilityOrigin
        let permission: AgentCoreToolPermission?
        let input: StructuredValue
        let output: StructuredValue?
        let outputPreview: String?
        let fullOutput: FullOutput?
        let artifacts: [AgentCorePartArtifact]
        let affectedResources: [AffectedResource]
        let startedAtMs: UInt64?
        let finishedAtMs: UInt64?
        private enum CodingKeys: String, CodingKey {
            case revision, state, title, summary, origin, permission, input, output, artifacts
            case partID = "part_id", toolCallID = "tool_call_id", capabilityID = "capability_id"
            case executionSource = "execution_source"
            case displayKey = "display_key", outputPreview = "output_preview", fullOutput = "full_output"
            case affectedResources = "affected_resources", startedAtMs = "started_at_ms", finishedAtMs = "finished_at_ms"
        }
        init(from decoder: Decoder) throws {
            let v = try decoder.container(keyedBy: CodingKeys.self)
            partID = try v.decode(String.self, forKey: .partID)
            revision = try v.decode(UInt64.self, forKey: .revision)
            toolCallID = try v.decode(String.self, forKey: .toolCallID)
            capabilityID = try v.decode(String.self, forKey: .capabilityID)
            executionSource = try v.decode(ToolExecutionSource.self, forKey: .executionSource)
            guard !capabilityID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw DecodingError.dataCorruptedError(forKey: .capabilityID, in: v, debugDescription: "Tool requires capability identity")
            }
            state = try v.decode(ActivityState.self, forKey: .state)
            displayKey = try v.decodeIfPresent(String.self, forKey: .displayKey)
            title = try v.decode(String.self, forKey: .title)
            summary = try v.decodeIfPresent(String.self, forKey: .summary)
            origin = try v.decode(AgentCoreCapabilityOrigin.self, forKey: .origin)
            permission = try v.decodeIfPresent(AgentCoreToolPermission.self, forKey: .permission)
            input = try v.decode(StructuredValue.self, forKey: .input)
            output = v.contains(.output) ? try v.decode(StructuredValue.self, forKey: .output) : nil
            outputPreview = try v.decodeIfPresent(String.self, forKey: .outputPreview)
            fullOutput = try v.decodeIfPresent(FullOutput.self, forKey: .fullOutput)
            if let fullOutput, fullOutput.partID != partID || fullOutput.toolCallID != toolCallID {
                throw DecodingError.dataCorruptedError(forKey: .fullOutput, in: v, debugDescription: "Foreign full output owner")
            }
            artifacts = try v.decode([AgentCorePartArtifact].self, forKey: .artifacts)
            affectedResources = try v.decode([AffectedResource].self, forKey: .affectedResources)
            startedAtMs = try v.decodeIfPresent(UInt64.self, forKey: .startedAtMs)
            finishedAtMs = try v.decodeIfPresent(UInt64.self, forKey: .finishedAtMs)
        }
    }

    nonisolated struct SourcePart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let sourceID: String
        let title: String
        let uri: String
        let mimeType: String?
        let snippet: String?
        let evidence: AgentCoreSourceEvidence?
        private enum CodingKeys: String, CodingKey {
            case revision, title, uri, snippet, evidence
            case partID = "part_id", sourceID = "source_id", mimeType = "mime_type"
        }
    }

    nonisolated struct CitationPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let citationID: String
        let sourceID: String
        let title: String
        let uri: String
        let mimeType: String?
        let originToolCallID: String
        let location: ChatSourceLocation?
        let rangeStart: UInt64?
        let rangeEnd: UInt64?
        private enum CodingKeys: String, CodingKey {
            case revision, title, uri, location, page, section
            case partID = "part_id", citationID = "citation_id", sourceID = "source_id", mimeType = "mime_type"
            case originToolCallID = "origin_tool_call_id", lineStart = "line_start", lineEnd = "line_end"
            case rangeStart = "range_start", rangeEnd = "range_end"
        }
        init(from decoder: Decoder) throws {
            let v = try decoder.container(keyedBy: CodingKeys.self)
            partID = try v.decode(String.self, forKey: .partID)
            revision = try v.decode(UInt64.self, forKey: .revision)
            citationID = try v.decode(String.self, forKey: .citationID)
            sourceID = try v.decode(String.self, forKey: .sourceID)
            title = try v.decode(String.self, forKey: .title)
            uri = try v.decode(String.self, forKey: .uri)
            mimeType = try v.decodeIfPresent(String.self, forKey: .mimeType)
            originToolCallID = try v.decode(String.self, forKey: .originToolCallID)
            let page = try v.decodeIfPresent(UInt64.self, forKey: .page)
            let lineStart = try v.decodeIfPresent(UInt64.self, forKey: .lineStart)
            let lineEnd = try v.decodeIfPresent(UInt64.self, forKey: .lineEnd)
            let section = try v.decodeIfPresent(String.self, forKey: .section)
            location = try v.decodeIfPresent(ChatSourceLocation.self, forKey: .location)
            guard location?.page == page, location?.lineStart == lineStart,
                  location?.lineEnd == lineEnd, location?.section == section else {
                throw DecodingError.dataCorruptedError(forKey: .location, in: v,
                    debugDescription: "Citation convenience fields differ from its exact location")
            }
            rangeStart = try v.decodeIfPresent(UInt64.self, forKey: .rangeStart)
            rangeEnd = try v.decodeIfPresent(UInt64.self, forKey: .rangeEnd)
            guard (rangeStart == nil) == (rangeEnd == nil) else {
                throw DecodingError.dataCorruptedError(forKey: .rangeStart, in: v,
                    debugDescription: "Citation range must have both bounds or neither")
            }
        }
    }

    nonisolated struct AttachmentPart: Decodable, Equatable, Sendable {
        let partID: String
        let revision: UInt64
        let attachmentID: String
        let name: String
        let mimeType: String?
        let uri: String
        let operation: String?
        private enum CodingKeys: String, CodingKey {
            case revision, name, uri, operation
            case partID = "part_id", attachmentID = "attachment_id", mimeType = "mime_type"
        }
    }
}
