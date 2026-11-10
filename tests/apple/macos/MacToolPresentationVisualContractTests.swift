import XCTest
@testable import Nola

@MainActor
final class MacToolPresentationVisualContractTests: XCTestCase {
    func testProtocolStateIsNotRenderedAsToolHeaderDetail() {
        for state in ["running", "completed", "succeeded", "failed", "waiting", "cancelled"] {
            let trace = MacAssistantTrace(
                kind: .tool,
                title: "web_search",
                detail: state,
                state: state == "running" ? .running : .completed
            )

            XCTAssertNil(
                MacToolTraceDisplayPolicy.headerDetail(for: trace),
                "Protocol state \(state) must be represented by icon/state, not repeated as raw text"
            )
        }
    }

    private func projectedTool(title: String, capability: String, origin: [String: String] = ["kind": "builtin"]) throws -> MacAssistantTrace {
        let message: [String: Any] = ["message_id": "assistant", "role": "assistant", "state": "running",
            "revision": 1, "created_at_ms": 1, "parts": [["kind": "tool", "execution_source": ["kind": "model"], "part_id": "tool", "revision": 1,
            "tool_call_id": "call", "capability_id": capability, "title": title, "state": "running",
            "input": [:], "summary": "", "origin": origin, "artifacts": [], "affected_resources": []]]]
        let decoded = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self,
            from: JSONSerialization.data(withJSONObject: message))
        return try XCTUnwrap(MacCanonicalConversationRepository.projectMessage(decoded).presentation?.traceItems.first)
    }

    func testCanonicalToolTitleAndCapabilityIdentityRemainIndependent() throws {
        let trace = try projectedTool(title: "Tool", capability: "write_file")
        XCTAssertEqual(trace.title, "Tool")
        XCTAssertEqual(trace.capabilityID, "write_file")
        XCTAssertEqual(MacToolTraceDisplayPolicy.operationTitle(for: trace), "Tool")
        XCTAssertNil(MacToolTraceDisplayPolicy.headerDetail(for: trace))
        XCTAssertEqual(MacToolPresentationPolicy.displayTitle(for: trace),
                       MacToolPresentationPolicy.displayTitle(for: "write_file"))
    }

    func testCanonicalProtocolNamesUseLocalizedDisplayTitlesWithoutChangingIdentity() throws {
        // Discovery is intentionally hidden by canonical trace projection;
        // permission/detail contexts still need its localized display title.
        XCTAssertNotEqual(MacToolPresentationPolicy.displayTitle(for: "tool_search"), "tool_search")
        for name in ["write_file", "read_file", "append_file", "bash", "web_search"] {
            let trace = try projectedTool(title: name, capability: name)
            let shown = MacToolPresentationPolicy.displayTitle(for: trace)
            XCTAssertEqual(shown, MacToolPresentationPolicy.displayTitle(for: name))
            XCTAssertNotEqual(shown, name)
            XCTAssertEqual(trace.title, name)
            XCTAssertEqual(trace.capabilityID, name)
        }
    }

    func testDeclaredExtensionTitleDoesNotBecomeAnInferredBuiltinAction() throws {
        let trace = try projectedTool(title: "Export my research", capability: "vendor_export",
                                     origin: ["kind": "mcp", "provider_id": "research"])
        XCTAssertEqual(MacToolPresentationPolicy.displayTitle(for: trace), "Export my research")
        XCTAssertEqual(trace.capabilityID, "vendor_export")
    }

    func testDeclaredSkillTitleWinsOverOpaqueCapabilityIdentifier() throws {
        let trace = try projectedTool(title: "Create Word document", capability: "word-document-authoring",
            origin: ["kind": "skill", "provider_id": "word-document-authoring", "provider_title": "Word"])
        XCTAssertEqual(trace.title, "Create Word document")
        XCTAssertEqual(MacToolPresentationPolicy.displayTitle(for: trace), "Create Word document")
    }

    func testTruncatedJSONNeverLeaksIntoCollapsedToolHeader() {
        let trace = MacAssistantTrace(
            kind: .tool,
            title: "web_search",
            detail: #"{"text":"large tool payload…"#,
            state: .completed
        )

        XCTAssertNil(MacToolTraceDisplayPolicy.headerDetail(for: trace))
    }

    func testBuiltInSkillIdentifierUsesRegistryDisplayTitle() {
        let title = MacToolPresentationPolicy.displayTitle(for: "word-document-authoring")

        XCTAssertEqual(
            title,
            MacBuiltInSkillRegistry.descriptors.first(where: {
                $0.id == .word
            })?.title
        )
        XCTAssertFalse(title.contains("Unknown"))
        XCTAssertFalse(title.contains("未知"))
    }
}
