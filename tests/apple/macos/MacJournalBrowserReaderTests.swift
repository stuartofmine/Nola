// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacJournalBrowserReaderTests: XCTestCase {
    func testActualCommittedToolDrivesBrowserChromeAndColdRead() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "browser-conversation",
            runID: "browser-run", assistantMessageID: "browser-assistant",
            tool: ScriptedCanonicalTool(callID: "navigate", capabilityID: "browser_navigate",
                title: "Open documentation", summary: "Page loaded",
                input: ["url": "https://example.com/requested"],
                output: #"{"url":"https://example.com/final","title":"Documentation"}"#,
                outputPreview: "Page loaded", resourceURI: "https://example.com/final"))
        _ = try fixture.finish("The page is ready.")
        let cursor = try XCTUnwrap(fixture.heads().first)
        let reference = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: cursor)
        let reader = MacJournalBrowserReader()
        let updates = try await reader.read(reference)
        let tool = try XCTUnwrap(updates.first)
        XCTAssertEqual(updates.count, 1)
        XCTAssertEqual(tool.toolName, "browser_navigate")
        XCTAssertEqual(tool.state, "completed")
        let snapshot = try XCTUnwrap(MacConversationBrowserSnapshot.reducingCoreTool(
            toolName: tool.toolName, state: tool.state, arguments: tool.arguments,
            structuredOutput: tool.output, outputPreview: nil, previous: nil))
        XCTAssertEqual(snapshot.currentURL, "https://example.com/final")
        XCTAssertEqual(snapshot.currentTitle, "Documentation")
        let cold = try await MacJournalBrowserReader().read(reference)
        XCTAssertEqual(cold.first?.arguments, tool.arguments)
        XCTAssertEqual(cold.first?.output, tool.output)
        let unchanged = try await reader.read(reference)
        XCTAssertEqual(unchanged.count, 1, "Repeated cursor does not duplicate browser operations")
    }

    func testNonBrowserToolsAndDifferentRunsCannotLeakIntoBrowserChrome() async throws {
        let reader = MacJournalBrowserReader()
        let browser = try MacAssistantJournalTestFixture(conversationID: "shared-conversation",
            runID: "old-run", assistantMessageID: "old-assistant",
            tool: ScriptedCanonicalTool(callID: "old-browser", capabilityID: "browser_navigate",
                title: "Open", summary: "Opened", input: ["url": "https://example.com/old"],
                output: #"{"url":"https://example.com/old"}"#, outputPreview: "Opened",
                resourceURI: "https://example.com/old"))
        _ = try browser.finish("Opened")
        let oldUpdates = try await reader.read(.init(databaseURL: browser.databaseURL,
            cursor: XCTUnwrap(browser.heads().first)))
        XCTAssertEqual(oldUpdates.count, 1)
        let next = try MacAssistantJournalTestFixture(conversationID: "shared-conversation",
            runID: "new-run", assistantMessageID: "new-assistant",
            tool: ScriptedCanonicalTool(callID: "read", capabilityID: "read_file", title: "Read",
                summary: "Read", input: ["path": "/tmp/read.txt"],
                output: #"{"url":"https://example.com/not-browser"}"#, outputPreview: "Read",
                resourceURI: "file:///tmp/read.txt"))
        _ = try next.finish("Read")
        let nextUpdates = try await reader.read(.init(databaseURL: next.databaseURL,
            cursor: XCTUnwrap(next.heads().first)))
        XCTAssertTrue(nextUpdates.isEmpty)
    }
}
