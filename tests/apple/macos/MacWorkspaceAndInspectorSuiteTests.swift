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
        for text in ["搜索 current", "please /stop", "https://example.org", "模型权限设计", "/opt/tools/notes.md", "// comment"] {
            XCTAssertNil(ComposerCommand.parse(text))
        }
        XCTAssertEqual(ComposerCommand.query("/mo"), "mo")
        XCTAssertNil(ComposerCommand.query("/model "))
        XCTAssertEqual(ComposerCommand.parse("/search 最近变化")?.argument, "最近变化")
        XCTAssertEqual(ComposerCommand.parse("/goal Ship v1.0")?.command, .goal)
        XCTAssertEqual(ComposerCommand.parse("/goal Ship v1.0")?.argument, "Ship v1.0")
        XCTAssertNil(ComposerCommand.parse("/unknown")?.command)
        XCTAssertNil(ComposerCommand.parse("/stop extra")?.command)
        XCTAssertEqual(ComposerCommand.parse("/stop ")?.command, .stop)

        XCTAssertEqual(MacWorkspaceSection.sidebarSections, [.chat, .extensions, .automations, .memory])
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
            if let artifactDir = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"], !artifactDir.isEmpty {
                let outputDir = URL(fileURLWithPath: artifactDir, isDirectory: true)
                try FileManager.default.createDirectory(at: outputDir, withIntermediateDirectories: true)
                let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
                try png.write(to: outputDir.appendingPathComponent("nola-progress-populated-\(scheme == .light ? "light" : "dark").png"))
            }
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
    func testReviewUsesFolderChosenOnFirstInvocationAndKeepsItForTheNextReview() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let defaults = try XCTUnwrap(UserDefaults(suiteName: root.lastPathComponent))
        defer { defaults.removePersistentDomain(forName: root.lastPathComponent) }
        let io = SubmissionBookmarkIO()
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(),
            modelApplication: port, bookmarkStore: store)
        model.newConversation()
        defer { model.presentationDidDetach() }

        let cancelled = await model.workspaceForReview()
        XCTAssertNil(cancelled)
        let folder = root.appendingPathComponent("review")
        io.selected = folder
        let chosen = await model.workspaceForReview()
        XCTAssertEqual(chosen, folder)
        io.selected = nil
        let reused = await model.workspaceForReview()
        XCTAssertEqual(reused, folder)
    }

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
                    if (text.isEmpty || text == "你好 Nola Ag"),
                       let artifactDir = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"],
                       !artifactDir.isEmpty {
                        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                        host.cacheDisplay(in: host.bounds, to: bitmap)
                        let output = URL(fileURLWithPath: artifactDir, isDirectory: true)
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
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
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

    func testConversationSearchDoesNotSurviveCloseOrNewConversation() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("workspace-search-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let preferencesName = "workspace-search-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: preferencesName))
        defer { preferences.removePersistentDomain(forName: preferencesName) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let repository = MacCanonicalConversationRepository(store: port)
        let runtime = PersistenceTestLocalRuntime()
        let remote = PersistenceTestRemoteHost()
        let model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: runtime,
            remoteHost: remote, modelApplication: port)
        model.newConversation()
        let original = try XCTUnwrap(model.selectedConversationID)
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let dependencies = MacInspectorDependencies(dataRegistry: registry,
            platformData: MacPlatformDataService(registry: registry, startsPeriodicRefresh: false, preferences: preferences),
            webSources: MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry),
            chromeMessaging: LayoutTestChromeMessaging())
        let size = CGSize(width: 1280, height: 780)
        let host = NSHostingView(rootView: MacChatWorkspaceView(initialConversationID: original,
            canonicalRepository: repository, localAgentRuntime: runtime, remoteHost: remote,
            inspectorDependencies: dependencies, model: model)
            .defaultAppStorage(preferences).environment(\.accessibilityEnabled, true)
            .frame(width: size.width, height: size.height))
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        func press(_ id: String) async throws {
            let deadline = ContinuousClock.now + .seconds(4)
            while ContinuousClock.now < deadline {
                host.layoutSubtreeIfNeeded()
                if let button = objects(host).first(where: { $0.accessibilityIdentifier?() == id }) {
                    XCTAssertTrue(button.accessibilityPerformPress?() == true, id)
                    try await Task.sleep(for: .milliseconds(180))
                    return
                }
                try await Task.sleep(for: .milliseconds(30))
            }
            XCTFail("Missing visible control: " + id)
            throw MacApplicationPortError.invalidResponse
        }
        func capture(_ name: String) throws {
            let image = try MacVisualPixelAssertions.captureCGImage(of: host)
            let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
            let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        try await press("workspace.searchConversation")
        XCTAssertTrue(model.isTranscriptSearchVisible)
        let searchEditor = try XCTUnwrap(window.firstResponder as? NSTextInputClient)
        searchEditor.insertText("Only this conversation", replacementRange: NSRange(location: NSNotFound, length: 0))
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(model.transcriptSearchText, "Only this conversation")
        XCTAssertEqual(model.searchText, "")
        try await press("workspace.searchConversation")
        XCTAssertFalse(model.isTranscriptSearchVisible)
        XCTAssertEqual(model.transcriptSearchText, "")
        XCTAssertTrue(window.firstResponder is MacAttachmentPasteTextView, "Closing search must restore keyboard editing")
        try await press("workspace.searchConversation")
        model.transcriptSearchText = "Previous conversation"
        model.searchText = "No matching historical title"
        try await press("workspace.newConversation")
        let created = try XCTUnwrap(model.selectedConversationID)
        XCTAssertNotEqual(created, original)
        XCTAssertFalse(model.isTranscriptSearchVisible)
        XCTAssertEqual(model.transcriptSearchText, "")
        XCTAssertEqual(model.searchText, "")
        XCTAssertTrue(window.firstResponder is MacAttachmentPasteTextView, "New conversations must be ready for typing")
        let createdTitle = try XCTUnwrap(model.selectedConversation?.title)
        model.searchText = "  " + createdTitle + "  "
        XCTAssertTrue(model.filteredConversations.contains { $0.id == created })
        model.searchText = ""
        try capture("mac-new-conversation-after-filter")
        try await press("workspace.conversation." + original)
        XCTAssertEqual(model.selectedConversationID, original)
        for area in ["task", "artifacts", "context"] {
            try await press("mac-inspector-group-" + area)
            XCTAssertTrue(objects(host).contains { $0.accessibilityIdentifier?() == "mac-inspector-region-" + area })
            try capture("mac-workspace-" + area)
        }
        for section in ["extensions", "automations", "memory", "chat"] {
            try await press("workspace.nav." + section)
            try capture("mac-destination-" + section)
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
            if let artifactDir = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"], !artifactDir.isEmpty {
                let output = URL(fileURLWithPath: artifactDir, isDirectory: true)
                try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: bitmap)
                try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                    output.appendingPathComponent("sidebar-\(Int(width))-\(dark ? "dark" : "light").png"))
            }
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
            let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.appearance = hosting.appearance
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await Task.sleep(for: .milliseconds(100))
            hosting.layoutSubtreeIfNeeded()
            let size = hosting.fittingSize
            XCTAssertLessThan(size.height, 1400)
            window.setContentSize(NSSize(width: 320, height: size.height))
            hosting.layoutSubtreeIfNeeded()
            if let artifactDir = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"], !artifactDir.isEmpty {
                let outputDir = URL(fileURLWithPath: artifactDir, isDirectory: true)
                try FileManager.default.createDirectory(at: outputDir, withIntermediateDirectories: true)
                let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
                hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
                let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                try png.write(to: outputDir.appendingPathComponent("nola-inspector-populated-\(scheme == .dark ? "dark" : "light").png"))
            }
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
                    let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
                    window.appearance = hosting.appearance
                    window.isReleasedWhenClosed = false
                    window.contentView = hosting
                    window.orderFrontRegardless()
                    defer { window.orderOut(nil); window.contentView = nil; window.close() }
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
                    let target = window.contentView?.superview ?? hosting
                    let cgImage = try MacVisualPixelAssertions.captureCGImage(of: target)
                    let metrics = try MacVisualPixelAssertions.analyzePixels(
                        cgImage: cgImage,
                        contentRect: CGRect(x: 0, y: 0, width: cgImage.width, height: cgImage.height),
                        isDark: scheme == .dark
                    )
                    XCTAssertGreaterThanOrEqual(metrics.warmSurfaceFraction, 0.35, "\(page)-\(Int(width))-\(scheme)")
                    XCTAssertGreaterThanOrEqual(max(metrics.headerContrastDelta, metrics.bodyContrastDelta), 40.0, "\(page)-\(Int(width))-\(scheme)")
                    if let artifactDir = ProcessInfo.processInfo.environment["NOLA_VISUAL_ARTIFACT_DIR"], !artifactDir.isEmpty {
                        let output = URL(fileURLWithPath: artifactDir, isDirectory: true)
                        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
                        let png = try XCTUnwrap(NSBitmapImageRep(cgImage: cgImage).representation(using: .png, properties: [:]))
                        try png.write(to: output.appendingPathComponent("\(page)-\(Int(width))-\(scheme == .dark ? "dark" : "light").png"))
                    }
                }
            }
        }
    }

    func testWarmInkColorsetsAndTopBarSingleHeaderPlacement() async throws {
        try validateAll15MacColorsetsAndPaletteAgainstTokensJSON()

        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("workspace-header-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let repo = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(canonicalRepository: repo,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        defer { model.presentationDidDetach() }
        let deps = MacInspectorDependencies(dataRegistry: registry,
            platformData: MacPlatformDataService(registry: registry, startsPeriodicRefresh: false),
            webSources: MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry),
            chromeMessaging: InspectorTestChromeMessaging())

        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            let suffix = appearance == .darkAqua ? "dark" : "light"
            for (layoutLabel, size, expectsSidebar) in [
                ("threeColumn", CGSize(width: 1_280, height: 820), true),
                ("inspectorOverlay", CGSize(width: 920, height: 720), true),
                ("compactOverlay", CGSize(width: 680, height: 720), false),
            ] {
                let workspaceView = MacChatWorkspaceView(
                    canonicalRepository: repo,
                    localAgentRuntime: PersistenceTestLocalRuntime(),
                    remoteHost: PersistenceTestRemoteHost(),
                    inspectorDependencies: deps,
                    model: model
                ).environment(\.accessibilityEnabled, true)

                try await MacVisualPixelAssertions.assertView(
                    workspaceView,
                    name: "workspace-\(layoutLabel)-\(suffix)",
                    size: size,
                    appearance: appearance,
                    styleMask: [.titled, .closable, .miniaturizable, .resizable],
                    assertWindow: { window, hosting in
                        XCTAssertTrue(window.styleMask.contains(.fullSizeContentView), "\(layoutLabel) must apply fullSizeContentView")
                        XCTAssertEqual(window.titleVisibility, .hidden, "\(layoutLabel) must hide titleVisibility")
                        XCTAssertEqual(window.titlebarSeparatorStyle, .none, "\(layoutLabel) must remove titlebar separator")
                        XCTAssertTrue(window.titlebarAppearsTransparent, "\(layoutLabel) must keep transparent titlebar")

                        let button = try await self.waitForAccessibleElement(identifier: "workspace.toggleSidebar", in: hosting)
                        let frame = try XCTUnwrap(button.accessibilityFrame?())
                        let bottomOffsetFromWindowTop = window.frame.maxY - frame.minY
                        XCTAssertGreaterThanOrEqual(bottomOffsetFromWindowTop, 0.0)
                        XCTAssertLessThanOrEqual(bottomOffsetFromWindowTop, 48.0, "Top header must sit at y=0..48 inside fullSizeContentView")
                    }
                )

                if expectsSidebar && layoutLabel == "threeColumn" {
                    for section in MacWorkspaceSection.sidebarSections {
                        let sectionView = MacChatWorkspaceView(
                            canonicalRepository: repo,
                            localAgentRuntime: PersistenceTestLocalRuntime(),
                            remoteHost: PersistenceTestRemoteHost(),
                            inspectorDependencies: deps,
                            model: model
                        ).environment(\.accessibilityEnabled, true)

                        try await MacVisualPixelAssertions.assertView(
                            sectionView,
                            name: "workspace-section-\(section.rawValue)-\(suffix)",
                            size: size,
                            appearance: appearance,
                            styleMask: [.titled, .closable, .miniaturizable, .resizable],
                            beforeCapture: { _, hosting in
                                let navButton = try await self.waitForAccessibleElement(
                                    identifier: "workspace.nav." + section.rawValue,
                                    in: hosting
                                )
                                XCTAssertTrue(navButton.accessibilityPerformPress?() ?? false)
                                try await Task.sleep(for: .milliseconds(200))
                            }
                        )
                    }
                }
            }

            try await MacVisualPixelAssertions.assertView(
                MacOnboardingView(onComplete: {}).environment(\.accessibilityEnabled, true),
                name: "workspace-onboarding-chrome-\(suffix)",
                size: CGSize(width: 980, height: 740),
                appearance: appearance,
                styleMask: [.titled, .closable, .miniaturizable, .resizable],
                assertWindow: { window, _ in
                    XCTAssertTrue(window.styleMask.contains(.fullSizeContentView))
                    XCTAssertEqual(window.titleVisibility, .hidden)
                    XCTAssertEqual(window.titlebarSeparatorStyle, .none)
                }
            )
        }
    }

    func testWorkspaceSheetsRenderAndPassPixelAssertionsViaRealNSWindowSheets() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("workspace-sheets-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let repo = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(
            canonicalRepository: repo,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost(),
            modelApplication: port
        )
        model.newConversation()
        defer { model.presentationDidDetach() }

        let roomExecution = MacRoomExecutionService(
            repository: repo,
            localRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let roomWorkspace = MacAgentRoomWorkspace(
            service: repo.makeAgentRoomService(),
            executeOwner: { try await roomExecution.execute($0, intent: $1) }
        )
        let sampleAgent = MacPersistentAgent(
            id: "research-expert",
            name: "Research Expert",
            description: "Reviews citations and source documents.",
            revision: 1,
            availability: "active",
            defaults: .object([:])
        )
        let recoveryPresentation = MacUnknownEffectRecoveryPresentation(
            owner: MacLocalRunRecovery(
                conversationID: "conv-recovery",
                expectedRunID: "run-recovery",
                canonicalAssistantMessageID: "msg-assistant",
                assistantMessageID: UUID(),
                generationID: 1
            ),
            inspection: MacUnknownEffectRecoveryInspection(
                conversationID: "conv-recovery",
                runID: "run-recovery",
                assistantMessageID: "msg-assistant",
                invocationConfigDigest: "digest-1",
                targets: [
                    MacUnknownEffectRecoveryTarget(
                        effectID: "effect-1",
                        requestID: "req-1",
                        attemptID: "attempt-1",
                        workerGeneration: "gen-1",
                        leaseOwner: "worker-1",
                        leaseToken: "token-1",
                        expectedAttemptRevision: 1
                    )
                ],
                retryAllowed: true,
                independentPendingEffects: 0
            ),
            pendingDecision: nil
        )

        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            let suffix = appearance == .darkAqua ? "dark" : "light"
            try await MacVisualPixelAssertions.assertSheet(
                MacCommandPaletteView(
                    model: model,
                    newConversation: {},
                    selectConversation: { _ in },
                    focusComposer: {},
                    toggleSidebar: {},
                    toggleInspector: {},
                    dismiss: {}
                ),
                name: "sheet-command-palette-\(suffix)",
                sheetSize: CGSize(width: 640, height: 480),
                appearance: appearance
            )
            try await MacVisualPixelAssertions.assertSheet(
                MacMessageEditSheet(
                    draft: AppleMessageEditDraft(
                        sourceConversationID: "conv-edit",
                        sourceMessageID: "msg-edit",
                        expectedSourceRevision: 1,
                        expectedMessageRevision: 1,
                        text: "Review the revised release checklist."
                    ),
                    submit: { _ in },
                    cancel: {}
                ),
                name: "sheet-message-edit-\(suffix)",
                sheetSize: CGSize(width: 620, height: 430),
                appearance: appearance
            )
            try await MacVisualPixelAssertions.assertSheet(
                MacChatExpertPicker(agents: [sampleAgent], selection: .constant([sampleAgent.id]), manage: {}),
                name: "sheet-expert-picker-\(suffix)",
                sheetSize: CGSize(width: 460, height: 420),
                appearance: appearance
            )
            try await MacVisualPixelAssertions.assertSheet(
                MacExpertDirectoryManagementView(workspace: roomWorkspace),
                name: "sheet-expert-management-\(suffix)",
                sheetSize: CGSize(width: 780, height: 540),
                appearance: appearance
            )
            try await MacVisualPixelAssertions.assertSheet(
                MacConversationModelSettingsView(model: model),
                name: "sheet-conversation-model-settings-\(suffix)",
                sheetSize: CGSize(width: 580, height: 560),
                appearance: appearance
            )
            try await MacVisualPixelAssertions.assertSheet(
                MacUnknownEffectRecoveryView(
                    presentation: recoveryPresentation,
                    isBusy: false,
                    submit: { _ in },
                    recoverPending: {}
                ),
                name: "sheet-unknown-effect-recovery-\(suffix)",
                sheetSize: CGSize(width: 520, height: 340),
                appearance: appearance
            )
        }
    }

    func testSlashGoalCommandPrefillsGoalInspectorAgainstIsolatedStore() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("workspace-goal-command-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let repo = MacCanonicalConversationRepository(store: port)
        let host = PersistenceTestRemoteHost()
        let model = MacWorkspaceModel(
            canonicalRepository: repo,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: host,
            modelApplication: port,
            readinessIssue: { _ in nil }
        )
        model.presentationDidAttach()
        defer { model.presentationDidDetach() }
        model.newConversation()
        let conversationID = try XCTUnwrap(model.selectedConversationID)

        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let deps = MacInspectorDependencies(
            dataRegistry: registry,
            platformData: MacPlatformDataService(registry: registry, startsPeriodicRefresh: false),
            webSources: MacWebSourceStore(storeURL: root.appendingPathComponent("sources.json"), registry: registry),
            chromeMessaging: InspectorTestChromeMessaging()
        )
        let workspaceView = MacChatWorkspaceView(
            canonicalRepository: repo,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: host,
            inspectorDependencies: deps,
            model: model
        )
        .frame(width: 1_280, height: 820)
        .environment(\.accessibilityEnabled, true)

        let hosting = NSHostingView(rootView: workspaceView)
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: CGSize(width: 1_280, height: 820)),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil; window.close() }

        try await MacVisualPixelAssertions.settleLayout(window: window, hostingView: hosting)
        let editor = try XCTUnwrap(findNSViews(in: hosting).compactMap { $0 as? MacAttachmentPasteTextView }.first)
        model.setDraft("/goal Ship v1.0 release", conversationID: conversationID)
        try await Task.sleep(for: .milliseconds(80))

        let press = try XCTUnwrap(NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: [],
            timestamp: 0,
            windowNumber: window.windowNumber,
            context: nil,
            characters: "\r",
            charactersIgnoringModifiers: "\r",
            isARepeat: false,
            keyCode: 36
        ))
        editor.keyDown(with: press)
        try await Task.sleep(for: .milliseconds(250))
        hosting.layoutSubtreeIfNeeded()

        XCTAssertEqual(model.draft, "", "/goal command must be intercepted instead of remaining in draft")
        XCTAssertTrue(
            (model.messagesByConversation[conversationID] ?? []).isEmpty,
            "/goal prefill must not dispatch a chat turn directly"
        )

        let goalEditor = try await waitForAccessibleElement(identifier: "mac-goal-objective", in: hosting)
        let deadline = ContinuousClock.now + .seconds(3)
        var capturedValue = ""
        while ContinuousClock.now < deadline {
            hosting.layoutSubtreeIfNeeded()
            let accessibilityText = ((goalEditor as? NSView)?.accessibilityValue() as? String)
                ?? ((goalEditor as? NSAccessibilityElement)?.accessibilityValue() as? String)
                ?? (goalEditor as? NSControl)?.stringValue
                ?? (goalEditor as? NSTextView)?.string
            if let text = accessibilityText
                ?? findNSViews(in: hosting).compactMap({ ($0 as? NSTextView)?.string }).first(where: { $0.contains("Ship v1.0 release") }) {
                capturedValue = text
                if capturedValue.contains("Ship v1.0 release") { break }
            }
            try await Task.sleep(for: .milliseconds(30))
        }
        XCTAssertTrue(capturedValue.contains("Ship v1.0 release"), "Expected goal objective prefill in inspector, got '\(capturedValue)'")
    }

    func testModelConnectionDisconnectAndReconnectPostsConfigurationNotification() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("mac-connection-lifecycle-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let service = MacModelConnectionService(application: port)
        let created = try service.create(
            name: "Lifecycle Provider",
            endpoint: "https://lifecycle.example.test/v1",
            wireProtocol: "responses",
            secret: "sk-lifecycle-secret"
        )
        XCTAssertTrue(created.document.connection.enabled)

        var notificationCount = 0
        let observer = NotificationCenter.default.addObserver(
            forName: .nolaMacConfigurationDidChange,
            object: nil,
            queue: nil
        ) { _ in
            notificationCount += 1
        }
        defer { NotificationCenter.default.removeObserver(observer) }

        let disconnected = try service.disconnect(created)
        XCTAssertFalse(disconnected.document.connection.enabled)
        XCTAssertEqual(notificationCount, 1)
        XCTAssertThrowsError(try service.credential(
            connectionID: created.id,
            endpoint: "https://lifecycle.example.test/v1",
            wireProtocol: "responses"
        ))

        let reconnected = try service.reconnect(disconnected)
        XCTAssertTrue(reconnected.document.connection.enabled)
        XCTAssertEqual(notificationCount, 2)
        let resolved = try service.credential(
            connectionID: created.id,
            endpoint: "https://lifecycle.example.test/v1",
            wireProtocol: "responses"
        )
        XCTAssertEqual(resolved.secret, "sk-lifecycle-secret")
    }

    private func validateAll15MacColorsetsAndPaletteAgainstTokensJSON() throws {
        let tokensURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("shared/design-tokens/tokens.json")
        let data = try Data(contentsOf: tokensURL)
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let colorSection = try XCTUnwrap(root["color"] as? [String: Any])
        let lightTokens = try XCTUnwrap(colorSection["light"] as? [String: String])
        let darkTokens = try XCTUnwrap(colorSection["dark"] as? [String: String])

        func parseHex(_ hex: String) throws -> (Double, Double, Double) {
            let trimmed = hex.trimmingCharacters(in: .whitespacesAndNewlines).replacingOccurrences(of: "#", with: "")
            let value = try XCTUnwrap(UInt32(trimmed, radix: 16), "Invalid hex: \(hex)")
            return (
                Double((value >> 16) & 0xFF) / 255.0,
                Double((value >> 8) & 0xFF) / 255.0,
                Double(value & 0xFF) / 255.0
            )
        }

        func luminance(_ rgb: (Double, Double, Double)) -> Double {
            let channels = [rgb.0, rgb.1, rgb.2].map { c in
                c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
        }

        func contrast(_ a: (Double, Double, Double), _ b: (Double, Double, Double)) -> Double {
            let l1 = luminance(a)
            let l2 = luminance(b)
            return (max(l1, l2) + 0.05) / (min(l1, l2) + 0.05)
        }

        func resolveNSColor(_ name: String, appearance: NSAppearance.Name) throws -> (r: Double, g: Double, b: Double, a: Double) {
            let color = try XCTUnwrap(NSColor(named: name), "Missing macOS colorset: \(name)")
            let nsAppearance = try XCTUnwrap(NSAppearance(named: appearance))
            var resolved: NSColor?
            nsAppearance.performAsCurrentDrawingAppearance {
                resolved = color.usingColorSpace(.sRGB)
            }
            let srgb = try XCTUnwrap(resolved, "Could not resolve \(name) in sRGB")
            return (Double(srgb.redComponent), Double(srgb.greenComponent), Double(srgb.blueComponent), Double(srgb.alphaComponent))
        }

        let colorsetMapping: [(String, String)] = [
            ("NolaCanvas", "canvas"),
            ("NolaSurface", "surface"),
            ("NolaSurfaceRaised", "surfaceSubtle"),
            ("NolaAccentSubtle", "accentSubtle"),
            ("NolaDivider", "border"),
            ("NolaControlOutline", "outline"),
            ("NolaTextPrimary", "ink"),
            ("AccentColor", "accent"),
            ("NolaTextSecondary", "inkSecondary"),
            ("NolaOnAccent", "onAccent"),
            ("NolaSuccess", "success"),
            ("NolaWarning", "warning"),
            ("NolaDanger", "danger"),
        ]

        for (colorsetName, tokenRole) in colorsetMapping {
            let expectedLight = try parseHex(try XCTUnwrap(lightTokens[tokenRole]))
            let actualLight = try resolveNSColor(colorsetName, appearance: .aqua)
            XCTAssertEqual(actualLight.r, expectedLight.0, accuracy: 0.004, "\(colorsetName) light red")
            XCTAssertEqual(actualLight.g, expectedLight.1, accuracy: 0.004, "\(colorsetName) light green")
            XCTAssertEqual(actualLight.b, expectedLight.2, accuracy: 0.004, "\(colorsetName) light blue")

            let expectedDark = try parseHex(try XCTUnwrap(darkTokens[tokenRole]))
            let actualDark = try resolveNSColor(colorsetName, appearance: .darkAqua)
            XCTAssertEqual(actualDark.r, expectedDark.0, accuracy: 0.004, "\(colorsetName) dark red")
            XCTAssertEqual(actualDark.g, expectedDark.1, accuracy: 0.004, "\(colorsetName) dark green")
            XCTAssertEqual(actualDark.b, expectedDark.2, accuracy: 0.004, "\(colorsetName) dark blue")
        }

        for (name, lightAlpha, darkAlpha) in [
            ("NolaControlHover", 0.055, 0.120),
            ("NolaControlPressed", 0.090, 0.190),
        ] {
            let lightResolved = try resolveNSColor(name, appearance: .aqua)
            XCTAssertEqual(lightResolved.r, 0.0, accuracy: 0.004, "\(name) light")
            XCTAssertEqual(lightResolved.a, lightAlpha, accuracy: 0.01, "\(name) light alpha")
            let darkResolved = try resolveNSColor(name, appearance: .darkAqua)
            XCTAssertEqual(darkResolved.r, 1.0, accuracy: 0.004, "\(name) dark")
            XCTAssertEqual(darkResolved.a, darkAlpha, accuracy: 0.01, "\(name) dark alpha")
        }

        let paletteChecks: [(String, UInt32, UInt32)] = [
            ("canvas", MacDesignTokens.Palette.Light.canvas, MacDesignTokens.Palette.Dark.canvas),
            ("surface", MacDesignTokens.Palette.Light.surface, MacDesignTokens.Palette.Dark.surface),
            ("surfaceSubtle", MacDesignTokens.Palette.Light.surfaceSubtle, MacDesignTokens.Palette.Dark.surfaceSubtle),
            ("surfaceElevated", MacDesignTokens.Palette.Light.surfaceElevated, MacDesignTokens.Palette.Dark.surfaceElevated),
            ("ink", MacDesignTokens.Palette.Light.ink, MacDesignTokens.Palette.Dark.ink),
            ("inkSecondary", MacDesignTokens.Palette.Light.inkSecondary, MacDesignTokens.Palette.Dark.inkSecondary),
            ("inkMuted", MacDesignTokens.Palette.Light.inkMuted, MacDesignTokens.Palette.Dark.inkMuted),
            ("border", MacDesignTokens.Palette.Light.border, MacDesignTokens.Palette.Dark.border),
            ("outline", MacDesignTokens.Palette.Light.outline, MacDesignTokens.Palette.Dark.outline),
            ("accent", MacDesignTokens.Palette.Light.accent, MacDesignTokens.Palette.Dark.accent),
            ("onAccent", MacDesignTokens.Palette.Light.onAccent, MacDesignTokens.Palette.Dark.onAccent),
            ("accentSubtle", MacDesignTokens.Palette.Light.accentSubtle, MacDesignTokens.Palette.Dark.accentSubtle),
            ("success", MacDesignTokens.Palette.Light.success, MacDesignTokens.Palette.Dark.success),
            ("warning", MacDesignTokens.Palette.Light.warning, MacDesignTokens.Palette.Dark.warning),
            ("danger", MacDesignTokens.Palette.Light.danger, MacDesignTokens.Palette.Dark.danger),
            ("info", MacDesignTokens.Palette.Light.info, MacDesignTokens.Palette.Dark.info),
        ]
        for (role, actualLightHex, actualDarkHex) in paletteChecks {
            let expLight = try parseHex(try XCTUnwrap(lightTokens[role]))
            let actLight = (
                Double((actualLightHex >> 16) & 0xFF) / 255.0,
                Double((actualLightHex >> 8) & 0xFF) / 255.0,
                Double(actualLightHex & 0xFF) / 255.0
            )
            XCTAssertEqual(actLight.0, expLight.0, accuracy: 0.001, "Palette.Light.\(role)")
            XCTAssertEqual(actLight.1, expLight.1, accuracy: 0.001, "Palette.Light.\(role)")
            XCTAssertEqual(actLight.2, expLight.2, accuracy: 0.001, "Palette.Light.\(role)")

            let expDark = try parseHex(try XCTUnwrap(darkTokens[role]))
            let actDark = (
                Double((actualDarkHex >> 16) & 0xFF) / 255.0,
                Double((actualDarkHex >> 8) & 0xFF) / 255.0,
                Double(actualDarkHex & 0xFF) / 255.0
            )
            XCTAssertEqual(actDark.0, expDark.0, accuracy: 0.001, "Palette.Dark.\(role)")
            XCTAssertEqual(actDark.1, expDark.1, accuracy: 0.001, "Palette.Dark.\(role)")
            XCTAssertEqual(actDark.2, expDark.2, accuracy: 0.001, "Palette.Dark.\(role)")
        }

        for (label, tokens) in [("light", lightTokens), ("dark", darkTokens)] {
            for fg in ["ink", "inkSecondary", "inkMuted", "accent", "success", "warning", "danger", "info"] {
                let fgRGB = try parseHex(try XCTUnwrap(tokens[fg]))
                for bg in ["canvas", "surface", "surfaceSubtle", "surfaceElevated"] {
                    let bgRGB = try parseHex(try XCTUnwrap(tokens[bg]))
                    XCTAssertGreaterThanOrEqual(contrast(fgRGB, bgRGB), 4.5, "WCAG AA text \(label) \(fg)/\(bg)")
                }
            }
            XCTAssertGreaterThanOrEqual(
                contrast(try parseHex(try XCTUnwrap(tokens["onAccent"])), try parseHex(try XCTUnwrap(tokens["accent"]))),
                4.5,
                "WCAG AA \(label) onAccent/accent"
            )
            for bg in ["canvas", "surface"] {
                XCTAssertGreaterThanOrEqual(
                    contrast(try parseHex(try XCTUnwrap(tokens["outline"])), try parseHex(try XCTUnwrap(tokens[bg]))),
                    3.0,
                    "WCAG AA outline \(label) outline/\(bg)"
                )
            }
        }
    }

    private func waitForAccessibleElement(identifier: String, in root: AnyObject) async throws -> AnyObject {
        let deadline = ContinuousClock.now + .seconds(3)
        while ContinuousClock.now < deadline {
            if let nsView = root as? NSView {
                nsView.layoutSubtreeIfNeeded()
            }
            var queue: [AnyObject] = [root], seen = Set<ObjectIdentifier>()
            while let item = queue.popLast() {
                guard seen.insert(ObjectIdentifier(item)).inserted else { continue }
                if (item.accessibilityIdentifier?() ?? nil) == identifier {
                    return item
                }
                let children = item.accessibilityChildren?() ?? []
                queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
                if let nsView = item as? NSView { queue += nsView.subviews }
            }
            try await Task.sleep(for: .milliseconds(25))
        }
        XCTFail("Missing accessible element with identifier \(identifier)")
        throw NSError(domain: "NolaTest", code: 1)
    }

    private func findNSViews(in root: NSView) -> [NSView] {
        [root] + root.subviews.flatMap(findNSViews)
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

    func testExtensionCardHasIndependentDetailsAndEnableActions() async throws {
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
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let identifier = "mac.extensions.details." + item.id
        let deadline = ContinuousClock.now + .seconds(4)
        while !objects(host).contains(where: { ($0.accessibilityIdentifier?() ?? nil) == identifier }),
              ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        let controls = objects(host)
        let details = controls.filter { ($0.accessibilityIdentifier?() ?? nil) == identifier }
        XCTAssertEqual(details.count, 1)
        let control = try XCTUnwrap(details.first)
        let toggle = try XCTUnwrap(controls.first { ($0.accessibilityIdentifier?() ?? nil) == "mac.extensions.toggle." + item.id })
        XCTAssertNotEqual(toggle.accessibilityLabel?(), control.accessibilityLabel?())
        let frame = try XCTUnwrap(control.accessibilityFrame?())
        XCTAssertGreaterThan(frame.width, 240, "The source and trailing whitespace must open details, replacing the tiny arrow target")
        XCTAssertGreaterThanOrEqual(frame.height, 36)
        XCTAssertTrue(control.accessibilityPerformPress?() ?? false)
        XCTAssertEqual(navigation.selectedID, item.id)
    }

    func testRemoteExtensionScopeDoesNotOfferLocalMCPConfiguration() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let navigation = MacExtensionsNavigationState()
        let catalog = MacExtensionsCatalogLoader(local: { _ in [] }, host: { [] }, removedSkills: { false })
        let host = NSHostingView(rootView: MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(),
            kinds: [.mcp], navigation: navigation, catalog: catalog)
            .environment(\.accessibilityEnabled, true).frame(width: 860, height: 620))
        let window = NSWindow(contentViewController: NSViewController())
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.setContentSize(NSSize(width: 860, height: 620))
        window.orderFrontRegardless()
        defer { window.close() }
        for source in [MacExtensionsCenterView.SourceFilter.thisMac, .connectedHost, .all] {
            navigation.sourceFilter = source
            let expected = source != .connectedHost
            let deadline = ContinuousClock.now + .seconds(3)
            func hasLocalCreate() -> Bool {
                host.layoutSubtreeIfNeeded()
                return objects(host).contains { ($0.accessibilityIdentifier?() ?? nil) == "mac.extensions.action.add_mcp" }
            }
            while hasLocalCreate() != expected && ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(30))
            }
            XCTAssertEqual(hasLocalCreate(), expected)
        }
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
