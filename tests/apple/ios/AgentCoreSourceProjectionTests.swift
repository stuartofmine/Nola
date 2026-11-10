// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

@MainActor final class AgentCoreSourceProjectionTests: XCTestCase {
    func testCitationLocationUsesExactSnakeCaseWireAndRoundTripsLargeCounts() throws {
        let wire = Data(#"{"page":3,"line_start":18446744073709551615,"line_end":18446744073709551615,"section":"方法 α"}"#.utf8)
        let location = try JSONDecoder().decode(ChatSourceLocation.self, from: wire)
        XCTAssertEqual(location.lineStart, UInt64.max)
        XCTAssertEqual(location.lineEnd, UInt64.max)
        let encoded = try JSONEncoder().encode(location)
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertNotNil(fields["line_start"]); XCTAssertNotNil(fields["line_end"])
        XCTAssertNil(fields["lineStart"]); XCTAssertNil(fields["lineEnd"])
        XCTAssertEqual(try JSONDecoder().decode(ChatSourceLocation.self, from: encoded), location)
        XCTAssertEqual(try JSONDecoder().decode(ChatSourceLocation.self, from: Data("{}".utf8)), ChatSourceLocation())
    }
    func testSourceAndCitationPartsProjectStructuredReference() async throws {
        var parts: [[String: Any]] = []
        parts.append(try sourcePart(part: """
        {"kind":"source","id":"part-source","message_id":"message-1","turn_id":"turn-1","source_id":"source-1","title":"Core design","uri":"file:///tmp/SOFTWARE_DESIGN.md","mime_type":"text/markdown"}
        """, partID: "part-source", cursor: 1))
        parts.append(try sourcePart(part: """
        {"kind":"citation","id":"part-citation","message_id":"message-1","turn_id":"turn-1","citation_id":"citation-1","source_id":"source-1","title":"Core design","uri":"file:///tmp/SOFTWARE_DESIGN.md","mime_type":"text/markdown","location":{"line_start":40,"line_end":48,"section":"Context"},"origin_tool_call_id":"tool-1"}
        """, partID: "part-citation", cursor: 2))

        let content = try await readJournalPresentation(parts)
        let references = IOSConversationProjectionPresenter.sourceReferences(from: content.parts)
        XCTAssertEqual(references.count, 1)
        let reference = try XCTUnwrap(references.first)
        XCTAssertEqual(reference.citationID, "citation-1")
        XCTAssertEqual(reference.sourceID, "source-1")
        XCTAssertEqual(reference.location?.lineStart, 40)
        XCTAssertEqual(reference.location?.lineEnd, 48)
        XCTAssertEqual(reference.location?.section, "Context")
        XCTAssertEqual(reference.originToolCallID, "tool-1")
        XCTAssertEqual(reference.fileURL?.lastPathComponent, "SOFTWARE_DESIGN.md")
    }

    func testCitationInheritsTypedMemoryEvidenceFromSource() async throws {
        var parts: [[String: Any]] = []
        parts.append(try sourcePart(part: """
        {"kind":"source","id":"part-memory","message_id":"message-1","turn_id":"turn-1","source_id":"memory-2","title":"Workspace preference","uri":"memory://memory-2","evidence":{"kind":"memory","usage":{"record_id":"memory-2","component_id":"context-memory-2","scope":"global","namespace":{"kind":"workspace"},"provenance_kind":"explicit_user","source":"conversation://session-1/message-9","observed_at":"2026-08-06T00:00:00Z","requires_current_state_verification":true,"truncated":false,"supersedes":["memory-1"],"usage_count":4,"last_used_at_epoch_ms":1786000000000}}}
        """, partID: "part-memory", cursor: 1))
        parts.append(try sourcePart(part: """
        {"kind":"citation","id":"part-citation","message_id":"message-1","turn_id":"turn-1","citation_id":"citation-1","source_id":"memory-2","title":"Workspace preference","uri":"memory://memory-2","origin_tool_call_id":"memory-search-1"}
        """, partID: "part-citation", cursor: 2))

        let content = try await readJournalPresentation(parts)
        let references = IOSConversationProjectionPresenter.sourceReferences(from: content.parts)
        let evidence = try XCTUnwrap(references.single?.memoryUsage)
        XCTAssertEqual(evidence.recordID, "memory-2")
        XCTAssertEqual(evidence.namespace?.kind, "workspace")
        XCTAssertEqual(evidence.supersedes, ["memory-1"])
        XCTAssertEqual(evidence.usageCount, 4)
        XCTAssertTrue(evidence.requiresCurrentStateVerification)
    }

    func testWebReferenceExposesDomainAndRejectsUnsafeScheme() {
        let web = ChatSourceReference(id: "web", sourceID: "web", title: "Report", uri: "https://www.example.com/report")
        let unsafe = ChatSourceReference(id: "unsafe", sourceID: "unsafe", title: "Unsafe", uri: "javascript:alert(1)")

        XCTAssertEqual(web.displayDomain, "example.com")
        XCTAssertEqual(web.webURL?.absoluteString, "https://www.example.com/report")
        XCTAssertTrue(web.canOpen)
        XCTAssertNil(unsafe.webURL)
        XCTAssertNil(unsafe.fileURL)
        XCTAssertFalse(unsafe.canOpen)
    }

    func testConversationMetadataRoundTripRestoresReferences() throws {
        let references = [ChatSourceReference(
            id: "citation-1",
            sourceID: "source-1",
            citationID: "citation-1",
            title: "Release notes",
            uri: "https://example.com/releases",
            location: ChatSourceLocation(page: 3, section: "Changes")
        )]
        let encoded = try XCTUnwrap(ChatSourceReference.encodeForConversationMeta(references))
        let restored = ChatSourceReference.decodeFromConversationMeta([ChatSourceReference.conversationMetaKey: encoded])
        XCTAssertEqual(restored, references)
    }

    private func sourcePart(part: String, partID: String, cursor: UInt64) throws -> [String: Any] {
        var value = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(part.utf8)) as? [String: Any])
        value.removeValue(forKey:"id"); value.removeValue(forKey:"message_id"); value.removeValue(forKey:"turn_id")
        value["part_id"] = partID; value["revision"] = cursor
        if let location = value["location"] as? [String: Any] {
            for key in ["page","line_start","line_end","section"] { value[key] = location[key] }
        }
        return value
    }
}

private extension Collection {
    var single: Element? { count == 1 ? first : nil }
}
