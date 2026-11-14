// SPDX-License-Identifier: Apache-2.0
import AppKit
import XCTest
@testable import Nola

@MainActor
final class MacWorkspaceContentTabsTests: XCTestCase {
    func testPrimarySidebarContainsOnlyConversationsAndResources() {
        XCTAssertEqual(MacWorkspaceSidebarArea.allCases, [.conversations, .resources])
        XCTAssertNil(MacWorkspaceSidebarArea(rawValue: "activity"))
    }

    func testResourceNavigationUsesOneHomeForContainedFeatures() {
        XCTAssertEqual(MacWorkspaceSection.resourceNavigationSections, [.data, .models, .extensions, .capabilities])
        XCTAssertEqual(MacWorkspaceSection.memory.resourceNavigationOwner, .data)
        XCTAssertEqual(MacWorkspaceSection.automations.resourceNavigationOwner, .data)
        XCTAssertTrue(MacDataCenterSection.allCases.contains(.memory))
        XCTAssertTrue(MacDataCenterSection.allCases.contains(.plans))
    }

    func testFileIdentityDeduplicatesLineTargetsAndCloseReturnsLeft() {
        let url = URL(fileURLWithPath: "/tmp/example.swift")
        var state = MacConversationContentWorkspace()
        state.open(.section(.models))
        state.open(.file(MacCodeReference(url: url, line: 4, column: nil)))
        state.open(.file(MacCodeReference(url: url, line: 9, column: 2)))
        XCTAssertEqual(state.objects.count, 2)
        state.close(state.selection)
        XCTAssertEqual(state.selectedObject, .section(.models))
        state.close(state.selection)
        XCTAssertEqual(state.selection, MacConversationContentWorkspace.conversationID)
        state.close(MacConversationContentWorkspace.conversationID)
        XCTAssertTrue(state.objects.isEmpty)
    }
    func testConversationScopesAndBackgroundCloseDoNotReplaceSelection() {
        var first = MacConversationContentWorkspace()
        first.open(.section(.data)); first.open(.section(.models))
        let selected = first.selection
        first.close(MacWorkspaceContentObject.section(.data).id)
        XCTAssertEqual(first.selection, selected)
        var second = MacConversationContentWorkspace()
        second.select(selected)
        XCTAssertEqual(second.selection, MacConversationContentWorkspace.conversationID)
        first.select(MacConversationContentWorkspace.conversationID)
        XCTAssertEqual(first.objects.count, 1)
    }
    func testExpertAppearanceSaveIsScopedAndDoesNotMutateDraftBase() throws {
        let original = Data()
        let saved = try XCTUnwrap(AppleExpertAppearance.saving(.purple, agentID: "writer", in: original))
        XCTAssertEqual(AppleExpertAppearance.selected(saved, agentID: "writer"), .purple)
        XCTAssertEqual(AppleExpertAppearance.selected(saved, agentID: "other"), .blue)
        XCTAssertEqual(AppleExpertAppearance.selected(original, agentID: "writer"), .blue)
        XCTAssertEqual(AppleExpertBubbleColor.allCases.count, 8)
        XCTAssertNil(AppleExpertAppearance.saving(.red, agentID: "", in: original))
    }
    func testShortExpertProseShrinksAndRichContentKeepsItsAvailableWidth() {
        let short = MacExpertBubbleLayout.contentWidth(text: "Hello", maximumWidth: 660, requiresFullWidth: false)
        XCTAssertLessThan(short, 140)
        XCTAssertGreaterThan(short, 32)
        XCTAssertEqual(MacExpertBubbleLayout.contentWidth(text: "Hello", maximumWidth: 660, requiresFullWidth: true), 624)
        XCTAssertEqual(MacExpertBubbleLayout.contentWidth(text: String(repeating: "长文字", count: 100), maximumWidth: 300, requiresFullWidth: false), 264)
    }

    func testReadingViewportSurvivesConversationSwitchAndReflow() throws {
        final class Document: NSView { override var isFlipped: Bool { true } }
        let store = MacTranscriptViewportStore()
        let messageID = UUID()
        let first = MacTranscriptScrollController()
        first.reset(conversationID: "first", viewportStore: store)
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 500))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 4000))
        scroll.documentView = document
        let marker = NSView(frame: NSRect(x: 0, y: 1000, width: 600, height: 800))
        document.addSubview(marker)
        first.attach(scroll); first.register(messageID: messageID, view: marker)
        first.userBeganScrolling()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 1100))
        first.userEndedScrolling(distanceFromBottom: 2400)
        store.setVisibleLimit(60, for: "first")
        first.detach()
        XCTAssertEqual(try XCTUnwrap(store.snapshot(for: "first")).messageID, messageID)

        let other = MacTranscriptScrollController()
        other.reset(conversationID: "other", viewportStore: store)
        XCTAssertTrue(other.followsLatest)
        let restored = MacTranscriptScrollController()
        restored.reset(conversationID: "first", viewportStore: store)
        restored.attach(scroll); restored.register(messageID: messageID, view: marker)
        marker.setFrameOrigin(NSPoint(x: 0, y: 1400))
        restored.reconcileLayout()
        XCTAssertEqual(scroll.documentVisibleRect.minY - marker.frame.minY, 100, accuracy: 1)
        XCTAssertFalse(restored.followsLatest)
        XCTAssertEqual(store.snapshot(for: "first")?.visibleMessageLimit, 60)
        restored.detach()
    }

    func testContentIdentityIncludesTypeAndDetailsAreOwnerScoped() {
        let entry = AppleActivityEntry(run_id: UUID().uuidString, conversation_id: "owner",
            assistant_message_id: "message", source: "conversation", title: "Activity",
            created_at_ms: 0, completed_at_ms: nil, status: "running",
            needs_response: false, is_current: true)
        var state = MacConversationContentWorkspace()
        state.open(.activity(entry)); state.open(.activity(entry))
        let url = URL(fileURLWithPath: "/tmp/project")
        state.open(.review(url)); state.open(.file(.init(url: url, line: nil, column: nil)))
        XCTAssertEqual(state.objects.count, 3)
        let positions = MacDetailScrollPositions()
        positions.save(NSPoint(x: 2, y: 140), for: "first/file")
        positions.save(NSPoint(x: 5, y: 800), for: "second/file")
        XCTAssertEqual(positions.position(for: "first/file"), NSPoint(x: 2, y: 140))
        XCTAssertEqual(positions.position(for: "second/file"), NSPoint(x: 5, y: 800))
        XCTAssertNil(positions.position(for: "third/file"))
    }

    func testInspectorNavigationRetainsAllTools() {
        XCTAssertEqual(MacChatInspector.InspectorTab.allCases,
                       [.overview, .progress, .goal, .review, .artifacts, .context, .execution, .participants])
    }

    func testFileURIRequiresCanonicalLocalOwnershipRatherThanLocalEndpointSelection() {
        let localPath = URL(fileURLWithPath: "/tmp/same-path-on-every-host.txt")
        XCTAssertTrue(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .local, isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .remote(hostID: "source-host", storeEpoch: "epoch"), isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .unavailable, isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: nil, isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .local, isLocalExecutionHost: false, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.isLocalFileURL(URL(string: "file://other-host/tmp/data.txt")!))
        XCTAssertFalse(MacFilePreviewPolicy.isLocalFileURL(URL(string: "https://example.com/data.txt")!))
    }

}
