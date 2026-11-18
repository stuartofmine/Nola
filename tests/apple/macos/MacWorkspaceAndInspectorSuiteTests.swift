// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS workspace content tabs, layout contracts, presentation lifecycle, steering journal, submission selection, composer/inspector layout & interaction, row hit regions, and composer commands suite.

import AppKit
import Combine
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacWorkspaceTabsLayoutAndComposerContractTests: XCTestCase {
    func testWorkspaceSidebarContentTabsExpertBubbleViewportAndFilePreviewContracts() throws {
        for text in ["搜索 current", "please /stop", "https://example.org", "模型权限设计"] {
            XCTAssertNil(ComposerCommand.parse(text))
        }
        XCTAssertEqual(ComposerCommand.query("/mo"), "mo")
        XCTAssertNil(ComposerCommand.query("/model "))
        XCTAssertEqual(ComposerCommand.parse("/search 最近变化")?.argument, "最近变化")
        XCTAssertNil(ComposerCommand.parse("/unknown")?.command)
        XCTAssertNil(ComposerCommand.parse("/stop extra")?.command)
        XCTAssertEqual(ComposerCommand.parse("/stop ")?.command, .stop)

        XCTAssertEqual(MacWorkspaceSection.allCases, [.chat, .extensions, .automations, .memory])
        XCTAssertTrue(MacDataCenterSection.allCases.contains(.memory))
        XCTAssertFalse(MacDataCenterSection.allCases.contains { $0.rawValue == "plans" })
        XCTAssertEqual(MacChatInspector.InspectorTab.allCases, [.task, .artifacts, .context])

        let positions = MacDetailScrollPositions()
        positions.save(NSPoint(x: 2, y: 140), for: "first/file")
        positions.save(NSPoint(x: 5, y: 800), for: "second/file")
        XCTAssertEqual(positions.position(for: "first/file"), NSPoint(x: 2, y: 140))
        XCTAssertEqual(positions.position(for: "second/file"), NSPoint(x: 5, y: 800))
        XCTAssertNil(positions.position(for: "third/file"))

        let originalAppearance = Data()
        let savedAppearance = try XCTUnwrap(AppleExpertAppearance.saving(.purple, agentID: "writer", in: originalAppearance))
        XCTAssertEqual(AppleExpertAppearance.selected(savedAppearance, agentID: "writer"), .purple)
        XCTAssertEqual(AppleExpertAppearance.selected(savedAppearance, agentID: "other"), .blue)
        XCTAssertEqual(AppleExpertAppearance.selected(originalAppearance, agentID: "writer"), .blue)
        XCTAssertEqual(AppleExpertBubbleColor.allCases.count, 8)
        XCTAssertNil(AppleExpertAppearance.saving(.red, agentID: "", in: originalAppearance))

        let shortWidth = MacExpertBubbleLayout.contentWidth(text: "Hello", maximumWidth: 660, requiresFullWidth: false)
        XCTAssertLessThan(shortWidth, 140)
        XCTAssertGreaterThan(shortWidth, 32)
        XCTAssertEqual(MacExpertBubbleLayout.contentWidth(text: "Hello", maximumWidth: 660, requiresFullWidth: true), 624)
        XCTAssertEqual(MacExpertBubbleLayout.contentWidth(text: String(repeating: "长文字", count: 100), maximumWidth: 300, requiresFullWidth: false), 264)

        let localPath = URL(fileURLWithPath: "/tmp/same-path-on-every-host.txt")
        XCTAssertTrue(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .local, isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .remote(hostID: "source-host", storeEpoch: "epoch"), isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .unavailable, isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: nil, isLocalExecutionHost: true, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.allowsLocalRead(sourceOwner: .local, isLocalExecutionHost: false, url: localPath))
        XCTAssertFalse(MacFilePreviewPolicy.isLocalFileURL(URL(string: "file://other-host/tmp/data.txt")!))
        XCTAssertFalse(MacFilePreviewPolicy.isLocalFileURL(URL(string: "https://example.com/data.txt")!))

        final class Document: NSView { override var isFlipped: Bool { true } }
        let store = MacTranscriptViewportStore()
        let messageID = UUID()
        let firstCtrl = MacTranscriptScrollController()
        firstCtrl.reset(conversationID: "first", viewportStore: store)
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 500))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 4000))
        scroll.documentView = document
        let marker = NSView(frame: NSRect(x: 0, y: 1000, width: 600, height: 800))
        document.addSubview(marker)
        firstCtrl.attach(scroll); firstCtrl.register(messageID: messageID, view: marker)
        firstCtrl.userBeganScrolling()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 1100))
        firstCtrl.userEndedScrolling(distanceFromBottom: 2400)
        store.setVisibleLimit(60, for: "first")
        firstCtrl.detach()
        XCTAssertEqual(try XCTUnwrap(store.snapshot(for: "first")).messageID, messageID)
        let otherCtrl = MacTranscriptScrollController()
        otherCtrl.reset(conversationID: "other", viewportStore: store)
        XCTAssertTrue(otherCtrl.followsLatest)
        let restoredCtrl = MacTranscriptScrollController()
        restoredCtrl.reset(conversationID: "first", viewportStore: store)
        restoredCtrl.attach(scroll); restoredCtrl.register(messageID: messageID, view: marker)
        marker.setFrameOrigin(NSPoint(x: 0, y: 1400))
        restoredCtrl.reconcileLayout()
        XCTAssertEqual(scroll.documentVisibleRect.minY - marker.frame.minY, 100, accuracy: 1)
        XCTAssertFalse(restoredCtrl.followsLatest)
        XCTAssertEqual(store.snapshot(for: "first")?.visibleMessageLimit, 60)
        restoredCtrl.detach()
    }

    func testThreeColumnWorkspaceLayoutNavigationIsolationAndComposerRecovery() throws {
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_101, sidebarWidth: 360), .inspectorOverlay)
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_000, sidebarWidth: 360), .inspectorOverlay)
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_102, sidebarWidth: 360), .threeColumn)
        for sidebar in [CGFloat(220), 280, 360] {
            for width in stride(from: CGFloat(1_200), through: 1_440, by: 1) {
                guard MacWorkspaceLayoutMode.resolve(width: width, sidebarWidth: sidebar) == .threeColumn else { continue }
                for requestedInspector in [CGFloat(340), 420, 480] {
                    let inspector = MacWorkspaceColumnMetrics.inspectorWidth(availableWidth: width, sidebarWidth: sidebar, storedWidth: requestedInspector)
                    XCTAssertGreaterThanOrEqual(width - sidebar - inspector - 2, MacWorkspaceColumnMetrics.minimumConversationWidth)
                }
            }
        }
        XCTAssertEqual(MacWorkspaceLayoutMode.resolve(width: 1_280, sidebarWidth: 0), .threeColumn)
        XCTAssertEqual(MacWorkspaceColumnMetrics.inspectorWidth(availableWidth: 1_280, sidebarWidth: 0, storedWidth: 480), 480)
        XCTAssertEqual(MacWorkspaceColumnMetrics.preferredInspectorWidth, MacDesignTokens.Layout.inspectorWidth)
        XCTAssertGreaterThanOrEqual(MacDesignTokens.Control.iconButton, 36)
        XCTAssertGreaterThanOrEqual(MacDesignTokens.Control.compactRow, 36)
        XCTAssertGreaterThan(MacDesignTokens.Radius.panel, MacDesignTokens.Radius.control)
        XCTAssertGreaterThan(MacDesignTokens.Radius.floating, MacDesignTokens.Radius.panel)

        let minWindowWidth: CGFloat = 1_280
        let inspectorWidth = MacWorkspaceColumnMetrics.inspectorWidth(availableWidth: minWindowWidth, sidebarWidth: MacDesignTokens.Layout.sidebarWidth, storedWidth: 900)
        let conversationWidth = minWindowWidth - MacDesignTokens.Layout.sidebarWidth - inspectorWidth
        XCTAssertGreaterThanOrEqual(conversationWidth, MacWorkspaceColumnMetrics.minimumConversationWidth)
        XCTAssertGreaterThan(conversationWidth, inspectorWidth)
        XCTAssertLessThanOrEqual(inspectorWidth, MacWorkspaceColumnMetrics.maximumInspectorWidth)
        let clampedWidth = MacWorkspaceColumnMetrics.inspectorWidth(availableWidth: 1_440, sidebarWidth: MacDesignTokens.Layout.sidebarWidth, storedWidth: 900)
        XCTAssertEqual(clampedWidth, MacWorkspaceColumnMetrics.maximumInspectorWidth)
        XCTAssertGreaterThan(clampedWidth, MacWorkspaceColumnMetrics.minimumInspectorWidth)

        let data = MacDataCenterNavigationState()
        data.section = .usage; data.showWebSources()
        let otherData = MacDataCenterNavigationState()
        XCTAssertEqual(data.section, .usage); XCTAssertEqual(otherData.section, .memory); XCTAssertEqual(otherData.route, .overview)
        let automations = MacAutomationCenterNavigationState()
        automations.showRunHistory(); automations.searchText = "daily"; automations.selectedFilter = .paused
        let otherAutomations = MacAutomationCenterNavigationState()
        XCTAssertFalse(otherAutomations.showsRunHistory); XCTAssertEqual(otherAutomations.searchText, ""); XCTAssertEqual(otherAutomations.selectedFilter, .all)
        let extensions = MacExtensionsNavigationState()
        extensions.selectedID = "fixture-extension"; extensions.query = "fixture"; extensions.sourceFilter = .connectedHost
        let otherExtensions = MacExtensionsNavigationState()
        XCTAssertNil(otherExtensions.selectedID); XCTAssertEqual(otherExtensions.query, ""); XCTAssertEqual(otherExtensions.sourceFilter, .thisMac)

        let editor = MacAttachmentPasteTextView()
        var sends = 0, accepts = 0
        editor.submitHandler = { sends += 1 }
        editor.mentionHandler = { _ in accepts += 1; return true }
        for code in [UInt16(36), UInt16(76)] {
            let repeated = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: true, keyCode: code))
            editor.keyDown(with: repeated)
        }
        XCTAssertEqual(sends, 0); XCTAssertEqual(accepts, 0)
        let press = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
        editor.keyDown(with: press)
        XCTAssertEqual(accepts, 1)
        editor.mentionHandler = { _ in false }
        editor.keyDown(with: press)
        XCTAssertEqual(sends, 1)

        var blockedComposer = MacComposerDisplayState(conversationID: "conversation", draft: "next question", attachments: [], isRunning: false, isVisiblyRunning: false, isSubmissionBlocked: true)
        XCTAssertFalse(blockedComposer.canSend)
        XCTAssertTrue(blockedComposer.isPrimaryActionDisabled)
        XCTAssertEqual(blockedComposer.draft, "next question")
        blockedComposer.isSubmissionBlocked = false
        XCTAssertTrue(blockedComposer.canSend)
        XCTAssertFalse(blockedComposer.isPrimaryActionDisabled)

        let runningBlockedComposer = MacComposerDisplayState(conversationID: "conversation", draft: "", attachments: [], isRunning: true, isVisiblyRunning: true, isSubmissionBlocked: true)
        XCTAssertFalse(runningBlockedComposer.canSend)
        XCTAssertFalse(runningBlockedComposer.isPrimaryActionDisabled)
    }

    func testInspectorProgressRefreshStateSnapshotDecodingAndGoalControlPolicies() throws {
        XCTAssertEqual(MacInspectorProgressContent.symbol("reported_done"), "circle")
        XCTAssertEqual(MacInspectorProgressContent.symbol("cancelled"), "minus.circle")
        XCTAssertEqual(MacInspectorProgressContent.symbol("completed"), "checkmark.circle")
        XCTAssertEqual(MacInspectorProgressContent.symbol("blocked"), "exclamationmark.circle")

        var refresh = MacInspectorRefreshState()
        XCTAssertTrue(refresh.begin(owner: "a"))
        refresh.finish(succeeded: true)
        XCTAssertFalse(refresh.begin(owner: "a"))
        XCTAssertTrue(refresh.loaded)
        refresh.finish(succeeded: false)
        XCTAssertTrue(refresh.loaded); XCTAssertTrue(refresh.failed)
        XCTAssertFalse(refresh.begin(owner: "a"))
        XCTAssertTrue(refresh.loaded); XCTAssertFalse(refresh.failed)
        XCTAssertFalse(refresh.controlsConfirmed)
        refresh.finish(succeeded: true)
        XCTAssertTrue(refresh.controlsConfirmed)
        XCTAssertTrue(refresh.begin(owner: "b"))
        XCTAssertFalse(refresh.loaded)
        refresh.finish(succeeded: false)
        XCTAssertFalse(refresh.loaded); XCTAssertTrue(refresh.failed)

        let reqJSON = #"{"can_pause":true,"can_resume":false,"can_cancel":true,"active":{"id":"goal-fixture","objective":"Ship","status":"running","requirements":{"r":{"id":"r","description":"Verify output","status":"blocked"}},"work":{"entries":[{"id":"old","content":"First plan","progress":"planned","supersedes":null},{"id":"new","content":"Revised plan","progress":"reported_done","supersedes":"old"}]}}}"#
        let reqSnapshot = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(reqJSON.utf8))
        XCTAssertEqual(reqSnapshot.active?.requirements["r"]?.description, "Verify output")
        XCTAssertEqual(reqSnapshot.active?.requirements["r"]?.status, "blocked")
        XCTAssertEqual(reqSnapshot.active?.work.current.map(\.id), ["new"])
        XCTAssertEqual(reqSnapshot.active?.work.current.first?.progress, "reported_done")

        let emptyGoal = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(#"{"can_pause":false,"can_resume":false,"can_cancel":false,"active":null}"#.utf8))
        XCTAssertNil(emptyGoal.active)
        let todos = try JSONDecoder().decode([MacInspectorProgressSnapshot.Todo].self, from: Data(#"[{"id":"s","content":"Review","status":"in_progress"}]"#.utf8))
        XCTAssertEqual(todos.first?.content, "Review"); XCTAssertEqual(todos.first?.status, "in_progress")

        let needsUserPayload = #"{"can_pause":true,"can_resume":true,"can_cancel":true,"active":{"id":"goal","objective":"Verify","status":"needs_user","requirements":{},"work":{"entries":[]}}}"#
        let needsUser = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(needsUserPayload.utf8))
        XCTAssertEqual(needsUser.active?.status, "needs_user")
        XCTAssertTrue(needsUser.can_pause && needsUser.can_resume && needsUser.can_cancel)
        for key in ["can_pause", "can_resume", "can_cancel"] {
            var obj = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(needsUserPayload.utf8)) as? [String: Any])
            obj.removeValue(forKey: key)
            XCTAssertThrowsError(try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: JSONSerialization.data(withJSONObject: obj)))
        }

        let noTokenActive = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(#"{"can_submit":true,"can_pause":false,"can_resume":false,"can_cancel":false,"active":{"id":"g","objective":"Keep working","status":"running","requirements":{},"work":{"entries":[]}}}"#.utf8))
        XCTAssertFalse(MacInspectorGoalControlPolicy.canStart(noTokenActive))
        XCTAssertFalse(MacInspectorGoalControlPolicy.canControl(noTokenActive))

        func tokenSnapshot(active: String, token: String, admitted: Bool) throws -> MacInspectorProgressSnapshot {
            let json = "{\"can_submit\":\(admitted),\"control_token\":\"\(token)\",\"can_pause\":false,\"can_resume\":false,\"can_cancel\":false,\"active\":\(active)}"
            return try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(json.utf8))
        }
        XCTAssertFalse(MacInspectorGoalControlPolicy.canStart(try tokenSnapshot(active: "null", token: "", admitted: false)))
        XCTAssertTrue(MacInspectorGoalControlPolicy.canStart(try tokenSnapshot(active: "null", token: "", admitted: true)))
        let activeStr = #"{"id":"g","objective":"Work","status":"running","requirements":{},"work":{"entries":[]}}"#
        XCTAssertFalse(MacInspectorGoalControlPolicy.canControl(try tokenSnapshot(active: activeStr, token: "", admitted: true)))
        XCTAssertTrue(MacInspectorGoalControlPolicy.canControl(try tokenSnapshot(active: activeStr, token: "current-source-token", admitted: false)))

        let navigation = MacInspectorNavigationState()
        let firstDraft = navigation.goalDrafts.draft(for: "first")
        firstDraft.objective = "Research with sources"; firstDraft.constraints = "Keep the original files"; firstDraft.isSubmitting = true
        navigation.selectedTab = .artifacts; navigation.selectedTab = .task
        XCTAssertTrue(firstDraft === navigation.goalDrafts.draft(for: "first"))
        let secondDraft = navigation.goalDrafts.draft(for: "second")
        secondDraft.objective = "Another task"
        firstDraft.error = "The first request failed"; firstDraft.isSubmitting = false
        XCTAssertEqual(navigation.goalDrafts.draft(for: "first").objective, "Research with sources")
        XCTAssertEqual(secondDraft.objective, "Another task")
        XCTAssertNil(secondDraft.error); XCTAssertFalse(secondDraft.isSubmitting)

        XCTAssertThrowsError(try JSONDecoder().decode(MacInspectorProgressSnapshot.self,
            from: Data(#"{"can_pause":true,"can_resume":false,"can_cancel":true,"active":{"id":"goal-fixture","objective":"Missing required state"}}"#.utf8)))
    }

    func testPopulatedProgressRendersAtNarrowWidthInLightAndDark() throws {
        let snapshot = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(#"{"can_pause":true,"can_resume":false,"can_cancel":true,"active":{"id":"goal-fixture","objective":"整理研究资料，完成方案与验证","status":"running","requirements":{"r":{"id":"r","description":"保留来源，并确认引用可以追溯到原文","status":"blocked"}},"work":{"entries":[{"id":"w","content":"资料已整理，等待校验引用","progress":"reported_done","supersedes":null}]}}}"#.utf8))
        let todos = try JSONDecoder().decode([MacInspectorProgressSnapshot.Todo].self, from: Data(#"[{"id":"a","content":"读取资料","status":"completed"},{"id":"b","content":"核对较长的跨语言引用与上下文，确认所有结论都有来源依据","status":"in_progress"},{"id":"c","content":"交付总结","status":"pending"}]"#.utf8))
        for scheme in [ColorScheme.light, .dark] {
            let renderer = ImageRenderer(content: MacInspectorProgressContent(snapshot: snapshot, todos: todos, tasks: [])
                .font(.system(size: 13)).padding(16).frame(width: 320)
                .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, scheme))
            let image = try XCTUnwrap(renderer.cgImage)
            XCTAssertEqual(image.width, 320)
            XCTAssertGreaterThan(image.height, 200)
            let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
            try png.write(to: URL(fileURLWithPath: "/tmp/nola-progress-populated-\(scheme == .light ? "light" : "dark").png"))
        }
        let duplicate = try JSONDecoder().decode([MacInspectorProgressSnapshot.Todo].self, from: JSONEncoder().encode(todos.map { ["id": $0.id, "content": $0.content, "status": $0.status] }))
        XCTAssertEqual(todos, duplicate)
    }
}

@MainActor
final class MacWorkspacePresentationLifecycleTests: XCTestCase {
    private func waitUntil(_ predicate: () -> Bool,
                           file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(3)
        while !predicate() && Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(predicate(), "Expected lifecycle transition", file: file, line: line)
    }

    private func seed(_ repository: MacCanonicalConversationRepository, count: Int = 1) throws {
        try repository.restoreBackupSnapshot([MacStoredConversation(
            conversation: MacConversation(id: "lifecycle-conversation", title: "Lifecycle", preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true),
            messages: (1...count).map {
                MacChatMessage(role: .user, text: "Stored message \($0)",
                    createdAt: Date(timeIntervalSince1970: Double($0)))
            }
        )])
    }

    func testDetachedIdleWorkspaceCanBeReleasedAfterItsPersistentSubscriptionStarted() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try seed(repository)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        var model: MacWorkspaceModel? = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        weak var weakModel: MacWorkspaceModel?
        weakModel = model
        model?.selectedConversationID = "lifecycle-conversation"
        model?.presentationDidAttach()
        try await waitUntil { model?.selectedMessages.count == 1 }
        try repository.rename(conversationID: "lifecycle-conversation", title: "Subscription is live")
        try await waitUntil { model?.conversations.first?.title == "Subscription is live" }
        // Do not nil the selection: the presentation lifecycle itself must
        // release the polling task's reference to this Workspace.
        model?.presentationDidDetach()
        model = nil
        try await waitUntil { weakModel == nil }
        XCTAssertTrue(host.cancellationRequests.isEmpty)
    }

    func testDetachDoesNotCancelAnActiveRemoteRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        let repository = MacCanonicalConversationRepository(store: port)
        weak var releasedModel: MacWorkspaceModel?
        do {
            let model = MacWorkspaceModel(canonicalRepository: repository,
                localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
            releasedModel = model
            model.presentationDidAttach()
            let conversation = try XCTUnwrap(model.selectedConversationID)
            model.executionHost = .remote
            model.updateRemoteHostConfigurationState(true)
            model.setFixtureRemoteModels([host.fixtureProfile])
            model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
            try await model.waitForModelConfigurationCommit()
            model.draft = "The presentation lifetime must not own execution cancellation"
            model.send()
            try await waitUntil { host.runContinuation != nil }
            let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
            model.presentationDidDetach()
            XCTAssertTrue(host.cancellationRequests.isEmpty)
            XCTAssertTrue(model.isRunning)
            XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, runID)
            model.presentationDidAttach()
            XCTAssertEqual(host.runRequests, 1, "Reattaching presentation cannot resubmit the Run")
            XCTAssertTrue(host.cancellationRequests.isEmpty)
            model.presentationDidDetach()
            host.runContinuation?.finish()
            try await waitUntil { !model.isRunning }
            XCTAssertNotNil(model.remoteRunRecovery)
            let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
                runID: runID, assistantMessageID: "\(runID):assistant")
            _ = try source.finish("The Run completed independently of presentation attachment.")
            try source.commitTerminal(into: port)
            try await waitUntil { model.remoteRunRecovery == nil && model.selectedTerminalTranscriptRevision > 0 }
            model.presentationDidDetach()
            XCTAssertTrue(host.cancellationRequests.isEmpty)
        }
        try await waitUntil { releasedModel == nil }
    }

    func testReattachDuringCommittedTerminalRevealRestoresWholeAnswerWithoutDuplicateNodes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        host.allowsRun = true
        weak var releasedModel: MacWorkspaceModel?
        do {
            let model = MacWorkspaceModel(canonicalRepository: repository,
                localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
            releasedModel = model
            model.presentationDidAttach()
            model.executionHost = .remote
            model.updateRemoteHostConfigurationState(true)
            model.setFixtureRemoteModels([host.fixtureProfile])
            model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
            try await model.waitForModelConfigurationCommit()
            let conversation = try XCTUnwrap(model.selectedConversationID)
            model.draft = "Reveal this committed answer across presentation detachment"
            model.send()
            try await waitUntil { host.runContinuation != nil }
            let runID = try XCTUnwrap(try port.activeRunIdentity(conversationID: conversation)?.runID)
            let markers = (1...10).map { "REVEAL-SECTION-\($0)-END" }
            let answer = markers.map {
                "\($0): The canonical answer must survive presentation cancellation without losing or duplicating its text."
            }.joined(separator: "\n\n")
            // Author actual source records and receive the fixed terminal frontier
            // before sending its bounded presentation notification.
            let source = try MacAssistantJournalTestFixture.forReceiver(receiver: port, conversationID: conversation,
                runID: runID, assistantMessageID: "\(runID):assistant")
            let seal = try source.finish(answer)
            try source.commitTerminal(into: port)
            host.runContinuation?.yield(source.terminalEvent(seal: seal))
            host.runContinuation?.finish()
            // Detach while the committed frontier is being delivered. Journal
            // bodies are reader-owned, not incrementally copied into message.text.
            model.presentationDidDetach()
            await Task.yield()
            model.presentationDidAttach()
            try await waitUntil {
                model.selectedMessages.last(where: { $0.role == .assistant })?.journalReference != nil && !model.isRunning
            }
            let assistant = try XCTUnwrap(model.selectedMessages.last(where: { $0.role == .assistant }))
            let reference = try XCTUnwrap(assistant.journalReference)
            let parts = try await MacJournalTextRead.parts(reference)
            XCTAssertFalse(parts.isEmpty)
            XCTAssertEqual(Set(parts.map(\.part_id)).count, parts.count)
            let rendered = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
            XCTAssertEqual(rendered, answer)
            for marker in markers {
                XCTAssertEqual(rendered.components(separatedBy: marker).count - 1, 1,
                    "Reattachment must preserve every original journal segment exactly once")
            }
            XCTAssertEqual(host.runRequests, 1)
            XCTAssertTrue(host.cancellationRequests.isEmpty)
            XCTAssertNil(model.remoteRunRecovery)
            XCTAssertNil(try port.activeRunIdentity(conversationID: conversation))
            model.presentationDidDetach()
        }
        try await waitUntil { releasedModel == nil }
    }

    func testReattachRefreshesCanonicalProjectionWithoutReplacingTheHistoryWindow() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try seed(repository, count: 130)
        let host = PermissionEnvelopeRemoteHost()
        host.applicationPort = port
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host)
        model.selectedConversationID = "lifecycle-conversation"
        model.presentationDidAttach()
        try await waitUntil { model.hasEarlierStoredMessages && !model.selectedMessages.isEmpty }
        await model.loadEarlierStoredMessages()
        XCTAssertTrue(model.isViewingStoredHistory)
        let windowIdentity = model.transcriptWindowIdentity
        let messageIDs = model.selectedMessages.map(\.id)
        XCTAssertFalse(messageIDs.isEmpty)
        model.presentationDidDetach()
        try repository.rename(conversationID: "lifecycle-conversation", title: "Changed while detached")
        model.presentationDidAttach()
        try await waitUntil { model.conversations.first?.title == "Changed while detached" }
        XCTAssertTrue(model.isViewingStoredHistory)
        XCTAssertEqual(model.transcriptWindowIdentity, windowIdentity)
        XCTAssertEqual(model.selectedMessages.map(\.id), messageIDs)
        try repository.rename(conversationID: "lifecycle-conversation", title: "Continues observing")
        try await waitUntil { model.conversations.first?.title == "Continues observing" }
        XCTAssertEqual(model.transcriptWindowIdentity, windowIdentity)
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        model.presentationDidDetach()
    }
}

@MainActor
final class MacWorkspaceSteeringJournalTests: XCTestCase {
    private func waitUntil(_ predicate: () -> Bool, file: StaticString = #filePath, line: UInt = #line) async throws {
        let deadline = Date().addingTimeInterval(4)
        while !predicate() && Date() < deadline { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(predicate(), "Expected canonical steering publication", file: file, line: line)
    }

    func testAttachedWindowReceivesPromotedRunThroughCanonicalProjectionObservation() async throws {
        let conversation = "observed-steering-\(UUID())"
        let run = "initial-\(UUID())"
        let source = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant", userText: "Initial request before steering")
        let first = try source.append("Initial answer")
        let port = try MacApplicationPort(databaseURL: source.databaseURL)
        let host = PermissionEnvelopeRemoteHost()
        let model = MacWorkspaceModel(
            canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        model.selectedConversationID = conversation
        model.presentationDidAttach()
        defer { model.presentationDidDetach() }
        try await waitUntil { model.selectedMessages.contains { $0.journalReference?.cursor == first } }
        XCTAssertEqual(model.selectedMessages.count, 2)

        // Publish through Application's actual admission/Host-response commands.
        // No reload, synthetic UI event, remote receipt or parent Run rebinding:
        // the attached window's production projection subscription must wake.
        let next = try source.publishSteeringSuccessor(
            userText: "Queued follow-up", answer: "Live promoted answer")
        try await waitUntil {
            model.selectedMessages.count == 4 && model.selectedMessages.contains {
                $0.journalReference?.cursor.identity.run_id == next
            }
        }
        XCTAssertNotEqual(next, run)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: conversation, runID: run))
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, next)
        XCTAssertEqual(model.selectedMessages.map { $0.role == .user }, [true, false, true, false])
        XCTAssertEqual(model.selectedMessages.filter { $0.role == .user }.map(\.text),
            ["Initial request before steering", "Queued follow-up"])
        let successor = try XCTUnwrap(model.selectedMessages.last?.journalReference)
        XCTAssertEqual(successor.cursor.identity.run_id, next)
        XCTAssertNotEqual(successor.cursor.identity.assistant_message_id, source.assistantMessageID)
        let answer = try await MacJournalTextRead.markdown(successor, historicalFallback: "")
        XCTAssertEqual(answer, "Live promoted answer")
        XCTAssertEqual(Set(model.selectedMessages.map(\.id)).count, 4)
        XCTAssertEqual(host.runRequests, 0)
        XCTAssertTrue(host.cancellationRequests.isEmpty)
    }

    func testActiveWindowShowsQueuedUserOnlyAfterSealedRunPromotion() async throws {
        let conversation = "steering-\(UUID())"
        let run = "initial-\(UUID())"
        let source = try MacAssistantJournalTestFixture(conversationID: conversation, runID: run,
            assistantMessageID: "\(run):assistant", userText: "Initial request before steering")
        // The local projection reads the same canonical database. A locally
        // queued turn must not acquire the predecessor's remote client binding.
        let port = try MacApplicationPort(databaseURL: source.databaseURL)
        let repository = MacCanonicalConversationRepository(store: port)
        let host = PermissionEnvelopeRemoteHost()
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: host, readinessIssue: { _ in nil })
        defer { model.presentationDidDetach() }
        model.presentationDidAttach()
        model.selectedConversationID = conversation
        let firstHead = try source.append("Initial answer")
        model.reloadPersistentData()
        await model.loadSelectedConversationIfNeeded()
        try await waitUntil { model.selectedMessages.contains { $0.journalReference?.cursor == firstHead } }
        XCTAssertEqual(model.selectedMessages.count, 2)

        let successorRun = try source.publishSteeringSuccessor(
            userText: "Steering request after initial answer", answer: "Successor answer in the promoted Run")
        XCTAssertNotEqual(successorRun, run)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: conversation, runID: run))
        model.reloadPersistentData()
        await model.loadSelectedConversationIfNeeded()

        try await waitUntil {
            model.selectedMessages.contains { $0.role == .user && $0.text == "Steering request after initial answer" } &&
            model.selectedMessages.contains { $0.journalReference?.cursor.identity.run_id == successorRun }
        }
        var visibleText: [String] = []
        for message in model.selectedMessages {
            if let reference = message.journalReference {
                visibleText.append(try await MacJournalTextRead.markdown(reference, historicalFallback: ""))
            } else { visibleText.append(message.text) }
        }
        XCTAssertEqual(visibleText, [
            "Initial request before steering", "Initial answer",
            "Steering request after initial answer", "Successor answer in the promoted Run",
        ])
        XCTAssertEqual(model.selectedMessages.map { $0.role == .user }, [true, false, true, false])
        XCTAssertEqual(try port.activeRunIdentity(conversationID: conversation)?.runID, successorRun)
        XCTAssertEqual(host.runRequests, 0, "Projection must not dispatch a new remote execution")
        XCTAssertTrue(host.cancellationRequests.isEmpty)
        XCTAssertEqual(Set(model.selectedMessages.map(\.id)).count, 4)
    }
}

// These helpers use only typed Application operations and the actual delivery
// lease. No checkpoint, aggregate, assistant owner or terminal fact is fabricated.
private extension MacAssistantJournalTestFixture {
    func steeringResult(_ command: [String: Any]) throws -> Any {
        try JSONSerialization.jsonObject(with: source.execute(commandData: JSONSerialization.data(withJSONObject: command)), options: [.fragmentsAllowed])
    }
    func steeringObject(_ command: [String: Any]) throws -> [String: Any] {
        try XCTUnwrap(steeringResult(command) as? [String: Any])
    }
    func publishSteeringSuccessor(userText: String, answer: String) throws -> String {
        let invocation = try steeringObject(["type": "load_exact_run_invocation", "conversation_id": conversationID,
            "expected_run_id": runID, "assistant_message_id": assistantMessageID])
        var digest = try XCTUnwrap(invocation["invocation_config_digest"] as? String)
        var executingRun = runID
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let loaded = try steeringObject(["type": "load_committed_host_requests", "run_id": runID])
        var first = try XCTUnwrap((loaded["requests"] as? [[String: Any]])?.first {
            ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
        })
        let leases = try XCTUnwrap(steeringResult(["type": "load_run_delivery_ownership", "run_id": runID]) as? [[String: Any]])
        XCTAssertEqual(leases.count, 1)
        first["delivery"] = try XCTUnwrap(leases.first)
        _ = try steeringObject(["type": "reduce_and_commit_active_submission", "input": [
            "conversation_id": conversationID, "run_id": runID, "transition_id": "fixture-queue-steering",
            "invocation_config_digest": digest, "committed_at_ms": now,
            "submission": ["type": "user_input", "submission_id": "fixture-steering-submission", "delivery": "next_turn",
                "submitted_at_epoch_ms": now, "items": [["type": "message", "id": "fixture-steering-user", "role": "user", "content": userText]]],
        ]])
        func accept(_ request: [String: Any], _ sequence: Int, _ payload: [String: Any]) throws {
            let keys: Set<String> = ["request_id", "attempt_id", "delegated_invocation_id", "effect_id", "host_id",
                "lease_id", "session_id", "turn_id", "catalog_revision", "availability_epoch", "delivery"]
            var response = request.filter { keys.contains($0.key) }
            response["sequence"] = sequence; response["payload"] = payload
            _ = try steeringObject(["type": "reduce_and_accept_host_response", "input": [
                "conversation_id": conversationID, "run_id": executingRun, "invocation_config_digest": digest,
                "response": response, "received_at_ms": now,
            ]])
        }
        let firstPart = try XCTUnwrap((first["payload"] as? [String: Any])?["text_part_id"] as? String)
        let previousSequence = try XCTUnwrap(first["resume_after_sequence"] as? NSNumber).intValue
        try accept(first, previousSequence + 1, ["type": "model_completed", "items": [["type": "message",
            "id": firstPart, "role": "assistant", "content": "Initial answer"]], "tool_calls": [], "usage": [:]])
        let ended = try steeringObject(["type": "load_committed_host_requests", "run_id": runID])
        XCTAssertEqual((ended["requests"] as? [[String: Any]])?.count, 0)
        let port = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: conversationID, runID: runID))
        let promoted = try steeringObject(["type": "promote_queued_assistant_turn", "input": [
            "conversation_id": conversationID, "created_at_ms": now]])
        executingRun = try XCTUnwrap((promoted["begin"] as? [String: Any])?["run_id"] as? String)
        digest = try XCTUnwrap(promoted["invocation_config_digest"] as? String)
        XCTAssertNotEqual(executingRun, runID)
        let claimed = try steeringObject(["type": "claim_committed_host_requests", "input": [
            "run_id": executingRun, "worker_generation": "steering-fixture-generation", "lease_owner": "steering-fixture-worker",
            "claimed_at_ms": now, "lease_expires_at_ms": now + 240_000, "limit": 64,
        ]])
        var successor = try XCTUnwrap((claimed["requests"] as? [[String: Any]])?.first {
            ($0["payload"] as? [String: Any])?["type"] as? String == "model_stream"
        })
        let lease = successor["delivery"]
        try accept(successor, 1, ["type": "execution_started"])
        try accept(successor, 2, ["type": "model_attempt_started", "provider": "fixture", "model": "fixture-model", "reason": "initial"])
        let prepared = try steeringObject(["type": "load_committed_host_requests", "run_id": executingRun])
        let effectID = try XCTUnwrap(successor["effect_id"] as? String)
        successor = try XCTUnwrap((prepared["requests"] as? [[String: Any]])?.first { $0["effect_id"] as? String == effectID })
        successor["delivery"] = lease
        let part = try XCTUnwrap((successor["payload"] as? [String: Any])?["text_part_id"] as? String)
        try accept(successor, 3, ["type": "model_committed_delta", "part_id": part, "delta": answer])
        return executingRun
    }
}

@MainActor
private final class SubmissionBookmarkIO: MacWorkspaceBookmarkIO {
    var selected: URL?
    func chooseFolder(suggested: URL?) -> URL? { selected }
    func bookmark(_ url: URL) throws -> Data { Data(url.absoluteString.utf8) }
    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        guard let value = String(data: data, encoding: .utf8), let url = URL(string: value) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return (url, false)
    }
    func start(_ url: URL) -> Bool { true }
    func stop(_ url: URL) {}
}

@MainActor
final class MacWorkspaceSubmissionSelectionTests: XCTestCase {
    func testLaterPrimaryAndGrantsDoNotChangeCapturedSubmissionAuthority() async throws {
        let suite = "Nola.SubmissionSelection.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = SubmissionBookmarkIO()
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let root = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
            .appendingPathComponent(UUID().uuidString)
        let original = root.appendingPathComponent("original")
        let added = root.appendingPathComponent("new-primary")
        let confirmed = root.appendingPathComponent("new-confirmed")
        io.selected = original
        let originalSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertEqual(originalSelection, original)
        let implicit = store.captureSubmissionSelection(conversationID: "conversation", selectedURL: nil)
        let explicit = store.captureSubmissionSelection(conversationID: "conversation", selectedURL: original)
        io.selected = added
        let addedSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertEqual(addedSelection, added)
        try store.confirmFolderAccess(confirmed, conversationID: "conversation")
        for snapshot in [implicit, explicit] {
            let resolved = try await store.resolveSubmissionSelection(snapshot)
            XCTAssertEqual(resolved.projectURL, original)
            XCTAssertEqual(Set(resolved.authorizedURLs), [original])
            XCTAssertFalse(resolved.authorizedURLs.contains(added))
            XCTAssertFalse(resolved.authorizedURLs.contains(confirmed))
        }
    }

    func testRevokedCapturedWorkspaceFailsInsteadOfChangingToSandboxOrNewPrimary() async throws {
        let suite = "Nola.SubmissionSelection.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = SubmissionBookmarkIO()
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let root = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
            .appendingPathComponent(UUID().uuidString)
        let original = root.appendingPathComponent("original")
        io.selected = original
        let originalSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertNotNil(originalSelection)
        let snapshots = [
            store.captureSubmissionSelection(conversationID: "conversation", selectedURL: nil),
            store.captureSubmissionSelection(conversationID: "conversation", selectedURL: original)
        ]
        store.removeWorkspace(conversationID: "conversation")
        io.selected = root.appendingPathComponent("replacement")
        let replacementSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertNotNil(replacementSelection)
        for snapshot in snapshots {
            do {
                _ = try await store.resolveSubmissionSelection(snapshot)
                XCTFail("Revocation must reject this submission, not silently select a different directory")
            } catch {
                XCTAssertEqual((error as NSError).domain, NSCocoaErrorDomain)
                XCTAssertEqual((error as NSError).code, CocoaError.fileReadNoPermission.rawValue)
            }
        }
    }
}

@MainActor
final class MacComposerAndInspectorLayoutTests: XCTestCase {
    func testNativeComposerCentersSingleLineAndGrowsWithoutClipping() async throws {
        for dark in [false, true] {
            for width: CGFloat in [360, 700] {
                let host = NSHostingView(rootView: ComposerAlignmentProbe()
                    .environment(\.colorScheme, dark ? .dark : .light)
                    .frame(width: width))
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 260),
                    styleMask: [.titled], backing: .buffered, defer: false)
                window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                window.isReleasedWhenClosed = false
                window.contentView = host
                window.orderFrontRegardless()
                defer { window.orderOut(nil); window.contentView = nil; window.close() }
                try await Task.sleep(for: .milliseconds(150))
                let editor = try XCTUnwrap(views(host).compactMap { $0 as? MacAttachmentPasteTextView }.first)
                for text in ["你好 Nola Ag", "第一行\n第二行", String(repeating: "长内容 wrapping text ", count: 60), ""] {
                    editor.insertText(text, replacementRange: NSRange(location: 0, length: (editor.string as NSString).length))
                    try await Task.sleep(for: .milliseconds(100))
                    host.layoutSubtreeIfNeeded()
                    let scroll = try XCTUnwrap(editor.enclosingScrollView)
                    let manager = try XCTUnwrap(editor.layoutManager)
                    let container = try XCTUnwrap(editor.textContainer)
                    manager.ensureLayout(for: container)
                    XCTAssertGreaterThanOrEqual(scroll.bounds.height, 35)
                    XCTAssertLessThanOrEqual(scroll.bounds.height, 133)
                    if text == "你好 Nola Ag" {
                        XCTAssertEqual(scroll.bounds.height, 36, accuracy: 1, "A single line must not expand to the multiline height limit")
                        let font = try XCTUnwrap(editor.font)
                        let baseline = manager.location(forGlyphAt: 0).y + editor.textContainerOrigin.y
                        let inkCenter = baseline - (font.ascender + font.descender) / 2
                        XCTAssertEqual(inkCenter, scroll.contentView.bounds.height / 2, accuracy: 2,
                            "Single-line input must be vertically centered using native font metrics")
                    }
                    if text.isEmpty || text == "你好 Nola Ag" {
                        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                        host.cacheDisplay(in: host.bounds, to: bitmap)
                        let output = URL(fileURLWithPath: "/tmp/nola-composer-layout")
                        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
                        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                            output.appendingPathComponent("\(Int(width))-\(dark ? "dark" : "light")-\(text.isEmpty ? "hint" : "typed").png"))
                    }
                    if text.contains("\n") {
                        XCTAssertGreaterThan(scroll.bounds.height, 45)
                        XCTAssertGreaterThanOrEqual(editor.bounds.height, manager.usedRect(for: container).maxY + editor.textContainerOrigin.y)
                    }
                }
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: bitmap)
                let attachment = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])), uniformTypeIdentifier: "public.png")
                attachment.name = "composer-centered-\(Int(width))-\(dark ? "dark" : "light")"
                attachment.lifetime = .keepAlways
                add(attachment)
            }
        }
    }

    func testInspectorManualExpansionKeepsRevealPositionAndCurrentSelection() {
        XCTAssertEqual(MacInspectorNavigationState(captureTab: "task").selectedTab, .task)
        XCTAssertEqual(MacInspectorNavigationState(captureTab: "artifacts").selectedTab, .artifacts)
        XCTAssertEqual(MacInspectorNavigationState(captureTab: "context").selectedTab, .context)
        XCTAssertEqual(MacInspectorNavigationState(captureTab: "unknown").selectedTab, .task)

        let navigation = MacInspectorNavigationState()
        let revision = navigation.revealRevision
        navigation.setKnowledgeExpanded(.wiki, true)
        XCTAssertEqual(navigation.selectedTab, .context)
        XCTAssertEqual(navigation.revealRevision, revision, "Local knowledge disclosure must not request an automatic scroll")
        XCTAssertTrue(navigation.select(.artifacts, isVisible: true))
        XCTAssertGreaterThan(navigation.revealRevision, revision)
        XCTAssertEqual(navigation.revealID, "artifacts")
        XCTAssertFalse(navigation.select(.artifacts, isVisible: true))
        XCTAssertEqual(MacChatInspector.InspectorTab.allCases, [.task, .artifacts, .context])
    }

    func testMountedInspectorHeadersFollowVisualOrderAndToggleOnlyTheirOwnSection() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("inspector-clicks-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.newConversation()
        defer { model.presentationDidDetach() }
        let preferencesName = "inspector-clicks-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: preferencesName))
        defer { preferences.removePersistentDomain(forName: preferencesName) }
        let deps = MacInspectorDependencies(dataRegistry: registry,
            platformData: MacPlatformDataService(registry: registry, startsPeriodicRefresh: false, preferences: preferences),
            webSources: MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry),
            chromeMessaging: LayoutTestChromeMessaging())
        let navigation = MacInspectorNavigationState()
        let host = NSHostingView(rootView: MacChatInspector(model: model, hostDisplayName: "Fixture",
            dependencies: deps, navigation: navigation).environment(\.accessibilityEnabled, true)
            .frame(width: 360, height: 900))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 360, height: 900),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(200))
        for area in MacChatInspector.InspectorArea.allCases {
            navigation.selectedTab = area.sections[0]
            try await Task.sleep(for: .milliseconds(100))
            let visible = objects(host).compactMap { $0.accessibilityIdentifier?() ?? nil }
            XCTAssertTrue(visible.contains("mac-inspector-region-" + area.rawValue))
            if area == .context {
                XCTAssertTrue(visible.contains("mac-inspector-context-summary"))
            } else {
                XCTAssertFalse(visible.contains("mac-inspector-context-summary"),
                    "mac-inspector-context-summary belongs only in .context, not in \(area)")
            }
            for dark in [false, true] {
                window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                try await Task.sleep(for: .milliseconds(100))
                host.layoutSubtreeIfNeeded()
                let title = try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == "mac-inspector-area-title" })
                let titleFrame = try XCTUnwrap(title.accessibilityFrame?())
                let bounds = window.convertToScreen(host.convert(host.bounds, to: nil))
                XCTAssertLessThanOrEqual(titleFrame.maxY, bounds.maxY + 0.5, "The area title must never be pushed above the viewport")
                XCTAssertGreaterThanOrEqual(titleFrame.minY, bounds.maxY - 44.5, "The fixed header must remain above scrolling content")
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: bitmap)
                let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                attachment.name = "inspector-\(area.rawValue)-\(dark ? "dark" : "light")"
                attachment.lifetime = .keepAlways; add(attachment)
            }
        }
    }

    func testLeftSidebarDestinationsAndSettingsStayUniqueInBothAppearances() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("sidebar-audit-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.newConversation()
        defer { model.presentationDidDetach() }
        let output = URL(fileURLWithPath: "/tmp/nola-mac-sidebar-pages")
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        for dark in [false, true] { for width: CGFloat in [240, 340] {
            var selectedSection = MacWorkspaceSection.chat
            var openedSettingsCount = 0
            let sectionBinding = Binding<MacWorkspaceSection>(
                get: { selectedSection },
                set: { selectedSection = $0 }
            )
            let content = MacConversationSidebar(
                model: model,
                selectedSection: sectionBinding,
                selectConversation: { _ in },
                loadParticipantNames: { _ in [] },
                onOpenSettings: { openedSettingsCount += 1 }
            ).frame(width: width, height: 650).background(MacDesignColor.sidebar)
                .environment(\.colorScheme, dark ? .dark : .light).environment(\.accessibilityEnabled, true)
            let host = NSHostingView(rootView: content)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 650),
                styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await Task.sleep(for: .milliseconds(180))
            host.layoutSubtreeIfNeeded()
            let controls = objects(host)
            let settings = controls.filter { ($0.accessibilityIdentifier?() ?? nil) == "workspace.settings" }
            XCTAssertEqual(settings.count, 1, "Sidebar has exactly one settings entry")
            let navDestinations = controls.filter { ($0.accessibilityIdentifier?() ?? nil)?.hasPrefix("workspace.nav.") == true }
            XCTAssertEqual(navDestinations.count, 4)
            XCTAssertFalse(controls.contains { ($0.accessibilityIdentifier?() ?? nil)?.hasPrefix("expert.children.toggle.") == true },
                "The conversation list must not contain expert history subtrees")
            let extensionsNav = try XCTUnwrap(navDestinations.first { ($0.accessibilityIdentifier?() ?? nil) == "workspace.nav.extensions" })
            XCTAssertTrue(extensionsNav.accessibilityPerformPress?() ?? false)
            XCTAssertEqual(selectedSection, .extensions)
            XCTAssertTrue(try XCTUnwrap(settings.first).accessibilityPerformPress?() ?? false)
            XCTAssertEqual(openedSettingsCount, 1)
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                output.appendingPathComponent("sidebar-\(Int(width))-\(dark ? "dark" : "light").png"))
        } }
    }

    private func objects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], result: [AnyObject] = [], seen = Set<ObjectIdentifier>()
        var index = 0
        while index < queue.count, result.count < 4000 {
            let object = queue[index]; index += 1
            guard seen.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = object as? NSView { queue += view.subviews }
        }
        return result
    }

    private func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(views) }
}

private struct ComposerAlignmentProbe: View {
    @State private var draft = ""
    @FocusState private var focused: Bool
    var body: some View {
        MacComposerView(state: .init(conversationID: "alignment-fixture", draft: draft,
            attachments: [], isRunning: false, isVisiblyRunning: false),
            actions: MacComposerActionSink { action in
                if case .updateDraft(_, let value) = action { draft = value; return true }
                return false
            }, composerFocused: $focused)
            .padding(16)
    }
}

private final class LayoutTestChromeMessaging: MacInspectorChromeMessaging, @unchecked Sendable {
    var installCount = 0
    var openCount = 0
    var installFails = false
    var openSucceeds = true
    func snapshot(now: Date) -> MacChromeBridgeSnapshot {
        .init(isChromeInstalled: true, isServiceRunning: true, isNativeHostInstalled: false,
            isExtensionPrepared: false, isExtensionConnected: false, extensionVersion: nil,
            capabilities: [], lastSeen: nil, errorMessage: nil)
    }
    func reinstall() throws {
        installCount += 1
        if installFails { throw CocoaError(.fileWriteNoPermission) }
    }
    func openChromeExtensions() -> Bool { openCount += 1; return openSucceeds }
}

@MainActor
final class MacInspectorInteractionTests: XCTestCase {
    func testInspectorAreasPartitionEveryToolAndRetainOtherAreaDisclosureState() {
        let state = MacInspectorNavigationState()
        let areas = MacChatInspector.InspectorArea.allCases
        XCTAssertEqual(areas.flatMap(\.sections), MacChatInspector.InspectorTab.allCases)
        XCTAssertEqual(Set(areas.flatMap(\.sections)).count, MacChatInspector.InspectorTab.allCases.count)
        state.setKnowledgeExpanded(.wiki, true)
        XCTAssertEqual(state.selectedTab.area, .context)
        XCTAssertTrue(state.select(.artifacts, isVisible: true))
        XCTAssertEqual(state.selectedTab.area, .artifacts)
        XCTAssertTrue(state.expandedKnowledge.contains(.wiki))
        XCTAssertTrue(state.select(.context, isVisible: true))
        XCTAssertEqual(state.selectedTab.area, .context)
        XCTAssertTrue(state.expandedKnowledge.contains(.wiki))
    }

    func testOpeningSectionsKeepsOtherSectionsExpanded() {
        let state = MacInspectorNavigationState()
        state.setKnowledgeExpanded(.points, true)
        state.selectedTab = .artifacts
        let revision = state.revealRevision
        state.selectedTab = .context
        XCTAssertTrue(state.expandedKnowledge.contains(.points))
        XCTAssertGreaterThan(state.revealRevision, revision)
    }

    func testOpeningContextDoesNotOpenAnUnrequestedEditor() {
        let state = MacInspectorNavigationState(captureTab: "context")
        XCTAssertTrue(state.expandedKnowledge.isEmpty)
        state.knowledgePane = .points
        XCTAssertEqual(state.expandedKnowledge, [.points])
    }

    func testDirectKnowledgeEntryExpandsTheRequestedEditor() {
        let state = MacInspectorNavigationState(captureTab: "wiki")
        XCTAssertEqual(state.selectedTab, .context)
        XCTAssertTrue(state.expandedKnowledge.contains(.wiki))
        state.knowledgePane = .recall
        XCTAssertTrue(state.expandedKnowledge.contains(.wiki))
        XCTAssertTrue(state.expandedKnowledge.contains(.recall))
    }
    func testFailureRecoveryUsesTypedCodesAndNeverRetriesAnEffect() {
        XCTAssertTrue(MacJournalFailurePresentation.canOpenModelSettings(for: "chatgpt_reauthorization_required"))
        XCTAssertTrue(MacJournalFailurePresentation.canOpenModelSettings(for: "invalid_model_request"))
        XCTAssertFalse(MacJournalFailurePresentation.canOpenModelSettings(for: "unknown_effect_outcome"))
        XCTAssertFalse(MacJournalFailurePresentation.canOpenModelSettings(for: "quoted authentication_failed"))
    }

    func testKnowledgeNavigationRevealsTheExactEditorFromAnotherSection() {
        let state = MacInspectorNavigationState(captureTab: "wiki")
        XCTAssertEqual(state.revealID, "knowledge-wiki")
        state.selectedTab = .artifacts
        XCTAssertEqual(state.revealID, "artifacts")
        state.knowledgePane = .recall
        XCTAssertEqual(state.selectedTab, .context)
        XCTAssertEqual(state.revealID, "knowledge-recall")
    }

    func testToolbarReopensCollapsedSelectedSectionInsteadOfHidingInspector() {
        let state = MacInspectorNavigationState()
        XCTAssertTrue(state.select(.task, isVisible: false))
        XCTAssertFalse(state.select(.task, isVisible: true))
        XCTAssertTrue(state.select(.context, isVisible: true))
    }

    func testChromeInstallOpensOnceAndReportsFailuresWithoutOpening() {
        let service = InspectorTestChromeMessaging()
        let model = MacInspectorChromeModel(service: service)
        model.installAndOpenExtensions()
        XCTAssertEqual(service.installCount, 1)
        XCTAssertEqual(service.openCount, 1)
        service.installFails = true
        model.installAndOpenExtensions()
        XCTAssertEqual(service.openCount, 1)
        XCTAssertNotNil(model.actionError)
        service.openSucceeds = false
        model.openExtensions()
        XCTAssertNotNil(model.actionError)
        service.openSucceeds = true
        model.openExtensions()
        XCTAssertNil(model.actionError)
    }

    func testArtifactNavigationSeparatesWebLinksFromSourceHostFiles() throws {
        let web = try XCTUnwrap(URL(string: "https://example.com/reports/final.pdf"))
        XCTAssertEqual(MacInspectorArtifactRow.destination(url: web, canReadLocalFiles: false, isLocalHost: false), .web(web))
        let local = URL(fileURLWithPath: "/tmp/inspector-report.pdf")
        XCTAssertNil(MacInspectorArtifactRow.destination(url: local, canReadLocalFiles: false, isLocalHost: true))
        XCTAssertNil(MacInspectorArtifactRow.destination(url: local, canReadLocalFiles: true, isLocalHost: false))
        XCTAssertEqual(MacInspectorArtifactRow.destination(url: local, canReadLocalFiles: true, isLocalHost: true), .localFile(local))
        for raw in ["file://foreign-host/tmp/private.pdf", "javascript:alert(1)", "custom-app://execute"] {
            let url = try XCTUnwrap(URL(string: raw))
            XCTAssertNil(MacInspectorArtifactRow.destination(url: url, canReadLocalFiles: true, isLocalHost: true))
        }
    }

    func testLongDiffPreviewIsBoundedAndRetainsBlankLines() {
        let lines = MacInspectorDiffPreview.lines("first\n\nthird")
        XCTAssertEqual(lines.map(String.init), ["first", "", "third"])
        let large = MacInspectorDiffPreview.lines(String(repeating: "changed line\n", count: 100_000))
        XCTAssertEqual(large.count, MacInspectorDiffPreview.lineLimit + 1)
    }

    func testPopulatedGitAndArtifactsRenderWithoutClippingAtNarrowWidths() async throws {
        let changes = [
            MacGitChange(indexStatus: " ", workTreeStatus: "M", path: "Sources/Research/跨语言研究资料与引用校验实现.swift", originalPath: nil),
            MacGitChange(indexStatus: "?", workTreeStatus: "?", path: "Documentation/Results/another-result-with-a-long-file-name.md", originalPath: nil)
        ]
        let url = URL(fileURLWithPath: "/tmp/inspector-visual-fixture")
        let snapshot = MacGitWorkspaceSnapshot(requestedURL: url, repositoryURL: url, branch: "feature/research",
            changes: changes, hasCommits: true, hasOriginRemote: false, upstreamName: nil, aheadCount: 0, behindCount: 0)
        for scheme in [ColorScheme.light, .dark] {
            let view = VStack(alignment: .leading, spacing: 20) {
                MacInspectorGitChangesView(snapshot: snapshot, selected: changes[0].id,
                    diff: "@@ -1,2 +1,2 @@\n-previous result\n+" + String(repeating: "引用来源验证 ", count: 12),
                    diffError: nil, isLoadingDiff: false, select: { _ in }, openReview: { _ in })
                MacInspectorArtifactRow(name: "最终研究报告与跨语言引用核验结果（完整交付版本）.pdf",
                    url: URL(string: "https://example.com/report.pdf")!, source: Date(timeIntervalSince1970: 1700000000),
                    type: "application/pdf", canReadLocalFiles: false, isLocalHost: false, inspectFile: { _ in })
                MacInspectorParticipantsView(participants: [.init(id: "expert", name: "跨语言研究、来源核验与交付质量审阅专家")])
            }.padding(16).frame(width: 320).fixedSize(horizontal: false, vertical: true)
                .background(MacDesignColor.inspector).environment(\.colorScheme, scheme)
            let hosting = NSHostingView(rootView: view)
            hosting.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            hosting.frame = NSRect(x: 0, y: 0, width: 320, height: 1000)
            let window = NSWindow(contentRect: hosting.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            window.contentView = hosting
            defer { window.orderOut(nil); window.contentView = nil }
            try await Task.sleep(for: .milliseconds(100))
            hosting.layoutSubtreeIfNeeded()
            let size = hosting.fittingSize
            XCTAssertLessThan(size.height, 1400)
            window.setContentSize(NSSize(width: 320, height: size.height))
            hosting.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
            hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try png.write(to: URL(fileURLWithPath: "/tmp/nola-inspector-populated-\(scheme == .dark ? "dark" : "light").png"))
        }
    }

    func testEveryInspectorSectionRendersAtNarrowAndWideWidthsInBothAppearances() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("inspector-pages-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        model.newConversation()
        defer { model.presentationDidDetach() }
        let deps = MacInspectorDependencies(dataRegistry: registry,
            platformData: MacPlatformDataService(registry: registry, startsPeriodicRefresh: false),
            webSources: MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry),
            chromeMessaging: InspectorTestChromeMessaging())
        let output = URL(fileURLWithPath: "/tmp/nola-mac-inspector-pages")
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        for scheme in [ColorScheme.light, .dark] {
            for width in [CGFloat(320), 440] {
                let pages = MacChatInspector.InspectorTab.allCases.map { ($0.rawValue, $0) }
                    + MacInspectorKnowledgePane.allCases.map { ($0.rawValue, MacChatInspector.InspectorTab.context) }
                for (page, section) in pages {
                    let navigation = MacInspectorNavigationState(captureTab: page)
                    let inspector = MacChatInspector(model: model, hostDisplayName: "Local test Host", dependencies: deps, navigation: navigation)
                    let view = VStack(alignment: .leading, spacing: 12) {
                        Text(section.title).font(.headline)
                        inspector.inspectorContent(section)
                    }.padding(16).frame(width: width, alignment: .topLeading)
                        .fixedSize(horizontal: false, vertical: true)
                        .background(MacDesignColor.inspector).environment(\.colorScheme, scheme)
                        .environment(\.accessibilityEnabled, true)
                    let hosting = NSHostingView(rootView: view)
                    hosting.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                    hosting.frame = NSRect(x: 0, y: 0, width: width, height: 1800)
                    let window = NSWindow(contentRect: hosting.frame, styleMask: [.borderless], backing: .buffered, defer: false)
                    window.contentView = hosting
                    window.orderFrontRegardless()
                    defer { window.orderOut(nil); window.contentView = nil }
                    // These are real Application reads against an isolated Store.
                    // A yield alone snapshots loading rather than the page itself.
                    try await Task.sleep(for: .milliseconds(250))
                    for _ in 0..<30 {
                        guard hasLoadingIndicator(hosting) else { break }
                        try await Task.sleep(for: .milliseconds(100))
                    }
                    XCTAssertFalse(hasLoadingIndicator(hosting), "\(page) must finish its isolated reads before visual acceptance")
                    hosting.layoutSubtreeIfNeeded()
                    let size = hosting.fittingSize
                    XCTAssertTrue(size.height.isFinite && size.height > 0)
                    window.setContentSize(NSSize(width: width, height: max(160, size.height)))
                    hosting.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
                    hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
                    let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                    XCTAssertGreaterThan(png.count, 1000, "\(section.rawValue) must render visible content")
                    try png.write(to: output.appendingPathComponent("\(page)-\(Int(width))-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }

    private func hasLoadingIndicator(_ root: AnyObject) -> Bool {
        var queue = [root], seen = Set<ObjectIdentifier>()
        while let item = queue.popLast() {
            guard seen.insert(ObjectIdentifier(item)).inserted else { continue }
            if (item.accessibilityIdentifier?() ?? nil) == "mac-inspector-progress-loading" { return true }
            let children = item.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = item as? NSView { queue += view.subviews }
        }
        return false
    }

}

// Test-only adapter: never installs an extension or opens the user's browser.
private final class InspectorTestChromeMessaging: MacInspectorChromeMessaging, @unchecked Sendable {
    var installCount = 0
    var openCount = 0
    var installFails = false
    var openSucceeds = true
    func snapshot(now: Date) -> MacChromeBridgeSnapshot {
        .init(isChromeInstalled: true, isServiceRunning: true, isNativeHostInstalled: false,
            isExtensionPrepared: false, isExtensionConnected: false, extensionVersion: nil,
            capabilities: [], lastSeen: nil, errorMessage: nil)
    }
    func reinstall() throws {
        installCount += 1
        if installFails { throw CocoaError(.fileWriteNoPermission) }
    }
    func openChromeExtensions() -> Bool { openCount += 1; return openSucceeds }
}

@MainActor
final class MacRowHitRegionTests: XCTestCase {
    func testDisclosureExposesOneFullWidthTargetIncludingTrailingWhitespace() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let state = RowState()
        let host = NSHostingView(rootView: RowProbe(state: state).environment(\.accessibilityEnabled, true).frame(width: 360, height: 140))
        host.frame = NSRect(x: 0, y: 0, width: 360, height: 140)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let deadline = ContinuousClock.now + .seconds(2)
        while objects(host).filter { let role = $0.accessibilityRole?() ?? nil; return role == .button || role == .disclosureTriangle }.isEmpty && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        host.layoutSubtreeIfNeeded()
        let actions = objects(host).filter { let role = $0.accessibilityRole?() ?? nil; return role == .button || role == .disclosureTriangle }
        XCTAssertEqual(actions.count, 1)
        let action = try XCTUnwrap(actions.first)
        let frame = try XCTUnwrap(action.accessibilityFrame?())
        XCTAssertGreaterThanOrEqual(frame.width, 330, "The empty trailing area belongs to the row, not only its label or arrow")
        XCTAssertGreaterThanOrEqual(frame.height, 36)
        XCTAssertTrue(action.accessibilityPerformPress?() ?? false)
        XCTAssertTrue(state.expanded)
    }

    func testExtensionFooterOpensDetailsFromAFullWidthControl() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let item = MacExtensionItem(id: "hit-region-fixture", kind: .skill, source: .global,
            title: "Fixture skill", detail: "Reads a selected document", path: nil, isEnabled: true)
        let navigation = MacExtensionsNavigationState()
        let catalog = MacExtensionsCatalogLoader(local: { _ in [item] }, host: { [] }, removedSkills: { false })
        let host = NSHostingView(rootView: MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(),
            kinds: [.skill], navigation: navigation, catalog: catalog)
            .environment(\.accessibilityEnabled, true).frame(width: 520, height: 720))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 720),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let identifier = "mac.extensions.details." + item.id
        let deadline = ContinuousClock.now + .seconds(4)
        while !objects(host).contains(where: { ($0.accessibilityIdentifier?() ?? nil) == identifier }),
              ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        let control = try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == identifier })
        let frame = try XCTUnwrap(control.accessibilityFrame?())
        XCTAssertGreaterThan(frame.width, 240, "The source and trailing whitespace must open details, replacing the tiny arrow target")
        XCTAssertGreaterThanOrEqual(frame.height, 36)
        XCTAssertTrue(control.accessibilityPerformPress?() ?? false)
        XCTAssertEqual(navigation.selectedID, item.id)
    }

    private func objects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], visited = Set<ObjectIdentifier>(), result: [AnyObject] = []
        var index = 0
        while index < queue.count, result.count < 4000 {
            let object = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }

}

@MainActor
private final class RowState: ObservableObject { @Published var expanded = false }
private struct RowProbe: View {
    @ObservedObject var state: RowState
    var body: some View {
        VStack {
            DisclosureGroup("Tools", isExpanded: $state.expanded) { Text("Visible content") }
                .disclosureGroupStyle(MacRowDisclosureStyle())
            Spacer()
        }.padding(12)
    }
}
