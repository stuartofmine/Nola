// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Covers stable macOS Chat message ordering, title restoration, and streaming follow cadence.

import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications
import XCTest
@testable import Nola

@MainActor
private final class WorkspaceBookmarkTestIO: MacWorkspaceBookmarkIO {
    nonisolated let onResolve: @Sendable () -> Void
    var selections: [URL?] = []
    var started: [URL] = []
    var stopped: [URL] = []
    var failBookmark = false
    var failStart = false

    init(onResolve: @escaping @Sendable () -> Void = {}) { self.onResolve = onResolve }

    func chooseFolder(suggested: URL?) -> URL? { selections.removeFirst() }
    func bookmark(_ url: URL) throws -> Data {
        if failBookmark { throw CocoaError(.fileWriteUnknown) }
        return Data(url.path.utf8)
    }
    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        onResolve()
        return (URL(fileURLWithPath: String(decoding: data, as: UTF8.self)), false)
    }
    func start(_ url: URL) -> Bool {
        guard !failStart else { return false }
        started.append(url)
        return true
    }
    func stop(_ url: URL) { stopped.append(url) }
}
@MainActor
final class MacChatWorkspaceBehaviorTests: XCTestCase {
    func testMemoryDisplayDefaultReadsCurrentStoreWithoutChangingConversationOverride() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("memory-display-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(), modelApplication: port)
        let initial = await model.memoryDefaultForDisplay()
        XCTAssertFalse(initial)
        model.memoryEnabledOverride = false
        MacAssistantPreferences.setMemoryEnabled(true, in: port)
        let enabled = await model.memoryDefaultForDisplay()
        XCTAssertTrue(enabled)
        XCTAssertFalse(model.memoryEnabled, "Presentation default must not override a conversation's explicit choice")
        MacAssistantPreferences.setMemoryEnabled(false, in: port)
        let disabled = await model.memoryDefaultForDisplay()
        XCTAssertFalse(disabled)
    }

    func testLatestThinkingSummaryMatchesFullHistoryReference() {
        func reference(_ input: String, limit: Int) -> String {
            let text = input.replacingOccurrences(of: "<think>", with: "")
                .replacingOccurrences(of: "</think>", with: "")
            func normalize(_ value: String) -> String {
                value.components(separatedBy: .whitespacesAndNewlines)
                    .filter { !$0.isEmpty }.joined(separator: " ")
            }
            let segments = text.split { "\n\r。！？!?；;".contains($0) }
                .map { normalize(String($0)) }.filter { !$0.isEmpty }
            let current = normalize(segments.last ?? text)
            if current.isEmpty { return String(localized: "Working…") }
            return current.count > limit ? "…" + current.suffix(limit) : current
        }
        let tokens = ["分析", "e\u{301}", "👩🏽‍💻", "\n", "\r", "。", "！", "？", "!", "?", ";", "；", " ", "\t", "<think>", "</think>"]
        var inputs = ["", "。； ! ?", " \t\n", "a\r\nb", String(repeating: "旧步骤。 ", count: 10_000) + "最后一步。\n"]
        for left in tokens {
            for right in tokens { inputs.append(left + right + "。" + right) }
        }
        for input in inputs {
            for limit in [0, 1, 8, 140] {
                XCTAssertEqual(MacThinkingActivityFormatter.currentStep(from: input, maximumLength: limit),
                    reference(input, limit: limit))
            }
        }
    }

    func testCapabilityLabelUsesCatalogSnapshotUntilConfigurationRefresh() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("capability-label-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: port)
        var endpoint = MacRemoteEndpointConfiguration(id: "endpoint", name: "fixture",
            baseURL: "https://fixture.invalid/v1", models: [MacRemoteModelConfiguration(
                id: "model", modelName: "fixture", executionCapabilities: .init())])
        try configuration.replaceEndpoints([endpoint])
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(),
            modelApplication: port)
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        XCTAssertEqual(model.selectedModelCapabilityStatus, .textOnly)

        endpoint.models[0].executionCapabilities.toolCalls = true
        try configuration.replaceEndpoints([endpoint])
        // Rendering must use one coherent catalog revision, not independently
        // read storage again every time SwiftUI evaluates the header label.
        for _ in 0..<100 {
            XCTAssertEqual(model.selectedModelCapabilityStatus, .textOnly)
        }
        model.refreshModelConfiguration()
        XCTAssertEqual(model.selectedModelCapabilityStatus, .toolsEnabled)
        model.selectExecutionBackend(.init(id: "codex", displayName: "Codex"), available: true)
        XCTAssertNil(model.selectedModelCapabilityStatus)
    }

    func testSavedExternalModelSurvivesAbsentOrChangedDiscovery() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("backend-display-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let model = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: port),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost(),
            modelApplication: port)
        model.newConversation()
        model.selectExecutionBackend(.init(id: "codex", displayName: "Codex"), available: true)
        model.selectExternalModel(.init(id: "saved-model", displayName: "Saved", providerID: "provider"), backendID: "codex")
        XCTAssertEqual(model.selectedExternalModel(in: nil)?.id, "saved-model")
        XCTAssertEqual(model.selectedExternalModel(in: nil)?.providerID, "provider")
        let refreshed = MacRemoteExecutionBackend(id: "codex", displayName: "Codex", availability: "available",
            version: nil, diagnostic: nil, models: [.init(id: "other", displayName: "Other", providerID: nil)],
            defaultModelID: "other")
        XCTAssertEqual(model.selectedExternalModel(in: refreshed)?.id, "saved-model")
        XCTAssertEqual(model.selectedConversation?.externalExecutionModelIDs?["codex"], "saved-model")
    }

    private actor RejectingDeletionRuntime: MacLocalAgentRuntime {
        func run(_ request: MacLocalAgentRunRequest) -> AsyncThrowingStream<MacLocalAgentEvent, Error> {
            AsyncThrowingStream { $0.finish(throwing: CocoaError(.fileWriteNoPermission)) }
        }
        func cancel(conversationID: String) {}
        func resolvePermission(conversationID: String, request: MacRemotePermissionRequest,
                               allow: Bool, scope: MacPermissionGrantScope) throws {}
        func listSessions() async throws -> [MacLocalAgentSessionSummary] { [] }
        func deleteSession(id: String) async throws { throw CocoaError(.fileWriteNoPermission) }
    }
    func testFolderCardConsentNeverOpensPickerAndIsConversationScoped() throws {
        let suite = "workspace-card-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = WorkspaceBookmarkTestIO() // empty selections: any picker call fails the test
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        let folder = URL(fileURLWithPath: "/Users/example/new-project")
        XCTAssertFalse(store.isAuthorizedWorkspace(folder, conversationID: "one"))
        try store.confirmFolderAccess(folder, conversationID: "one")
        try store.confirmFolderAccess(folder, conversationID: "one")
        XCTAssertEqual(store.authorizedWorkspaceURLs(conversationID: "one"), [folder])
        XCTAssertTrue(io.started.isEmpty, "Application consent must not fabricate an OS scope")
        XCTAssertNil(store.restoreWorkspace(conversationID: "one"), "Extra access does not change cwd")
        let restored = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        XCTAssertTrue(restored.isAuthorizedWorkspace(folder, conversationID: "one"))
        XCTAssertFalse(restored.isAuthorizedWorkspace(folder, conversationID: "two"))
        restored.clearWorkspace(conversationID: "one")
        XCTAssertFalse(restored.isAuthorizedWorkspace(folder, conversationID: "one"))
    }

    func testMacIterationBudgetMatchesCoreAndPreservesExplicitLimits() throws {
        for budget in [0, 1, 8, 24, 100] {
            let request = try MacLocalAgentRunRequest(conversationID: "budget", prompt: "test",
                projectURL: FileManager.default.temporaryDirectory, attachmentURLs: [], maxModelIterations: budget)
            XCTAssertEqual(request.maxModelIterations, budget)
        }
        let request = try MacLocalAgentRunRequest(conversationID: "default-budget", prompt: "test",
            projectURL: FileManager.default.temporaryDirectory, attachmentURLs: [])
        XCTAssertEqual(request.maxModelIterations, 0)
    }

    func testModelParameterPresetsAreExplicitAndRecoverable() {
        var parameters = ModelParameters.webAlignedCloudDefaults

        MacModelParameterPreset.precise.apply(to: &parameters)
        XCTAssertEqual(parameters.temperature, 0.2, accuracy: 0.001)
        XCTAssertEqual(parameters.topP, 0.8, accuracy: 0.001)
        XCTAssertEqual(MacModelParameterPreset.matching(parameters), .precise)

        parameters.temperature = 0.43
        XCTAssertEqual(MacModelParameterPreset.matching(parameters), .custom)
        MacModelParameterPreset.creative.apply(to: &parameters)
        XCTAssertEqual(MacModelParameterPreset.matching(parameters), .creative)
    }

    func testAutomationValidationIgnoresHiddenPersonalDataAge() {
        let sharedArguments = (
            name: "Daily review",
            prompt: "Summarize the latest project changes",
            workspacePath: "/tmp/project",
            maximumDataAge: "not-a-number",
            timeoutMinutes: "30",
            retryCount: "0"
        )

        XCTAssertNil(MacAutomationEditorValidation.message(
            name: sharedArguments.name,
            prompt: sharedArguments.prompt,
            workspacePath: sharedArguments.workspacePath,
            validatesPersonalDataAge: false,
            maximumDataAge: sharedArguments.maximumDataAge,
            timeoutMinutes: sharedArguments.timeoutMinutes,
            retryCount: sharedArguments.retryCount
        ))
        XCTAssertNotNil(MacAutomationEditorValidation.message(
            name: sharedArguments.name,
            prompt: sharedArguments.prompt,
            workspacePath: sharedArguments.workspacePath,
            validatesPersonalDataAge: true,
            maximumDataAge: sharedArguments.maximumDataAge,
            timeoutMinutes: sharedArguments.timeoutMinutes,
            retryCount: sharedArguments.retryCount
        ))
    }

    func testModelParameterValidationRejectsValuesInsteadOfSilentlyClamping() {
        var parameters = ModelParameters.webAlignedCloudDefaults
        XCTAssertNil(MacModelParameterValidation.message(
            parameters: parameters,
            seedText: "",
            isLocal: false,
            contextRange: 512...2_000_000
        ))

        parameters.maxOutputTokens = 64_000
        parameters.maxContextLength = 4_000_000
        XCTAssertNil(MacAddRemoteModelValidation.message(
            modelID: "reasoning-large", capabilities: [.chat], parameters: parameters
        ), "Creation and editing must accept the same remote model limits")
        parameters.maxOutputTokens = ModelParameterLimits.outputTokens.upperBound + 1
        XCTAssertNotNil(MacModelParameterValidation.message(
            parameters: parameters,
            seedText: "",
            isLocal: false,
            contextRange: 512...2_000_000
        ))

        parameters.maxOutputTokens = 16_384
        XCTAssertNotNil(MacModelParameterValidation.message(
            parameters: parameters,
            seedText: "12.5",
            isLocal: false,
            contextRange: 512...2_000_000
        ))
    }

    func testModelTestUsesSinglePaneBeforeSplitViewWouldOverflow() {
        XCTAssertTrue(MacModelTestLayoutPolicy.usesCompactLayout(availableWidth: 700))
        XCTAssertTrue(MacModelTestLayoutPolicy.usesCompactLayout(availableWidth: 819))
        XCTAssertFalse(MacModelTestLayoutPolicy.usesCompactLayout(availableWidth: 820))
        XCTAssertFalse(MacModelTestLayoutPolicy.usesCompactLayout(availableWidth: 1_200))
    }

    func testAddRemoteModelValidationRequiresAUseAndRejectsOutOfRangeParameters() {
        var parameters = ModelParameters.webAlignedCloudDefaults

        XCTAssertNotNil(MacAddRemoteModelValidation.message(
            modelID: "reasoning-large",
            capabilities: [.sensitiveData],
            parameters: parameters
        ))
        XCTAssertNil(MacAddRemoteModelValidation.message(
            modelID: "reasoning-large",
            capabilities: [.chat],
            parameters: parameters
        ))

        parameters.maxOutputTokens = ModelParameterLimits.outputTokens.upperBound + 1
        XCTAssertNotNil(MacAddRemoteModelValidation.message(
            modelID: "reasoning-large",
            capabilities: [.chat],
            parameters: parameters
        ))
    }

    func testTerminalFailureRemainsVisibleOutsideCollapsedTraces() {
        let failedTrace = MacAssistantTrace(
            kind: .status,
            title: "Context compression failed",
            detail: "The original context was not replaced.",
            state: .failed
        )
        XCTAssertTrue(MacAssistantFailurePresentationPolicy.showsFallbackCard(
            deliveryState: .failed,
            traces: [failedTrace]
        ))
        XCTAssertTrue(MacAssistantFailurePresentationPolicy.showsFallbackCard(
            deliveryState: .failed,
            traces: []
        ))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.showsFallbackCard(
            deliveryState: .completed,
            traces: []
        ))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.showsFallbackCard(
            deliveryState: .completed,
            traces: [failedTrace]
        ))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.highlightsResponse(deliveryState: .completed))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.highlightsResponse(deliveryState: .running))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.highlightsResponse(deliveryState: .cancelled))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.highlightsResponse(deliveryState: nil))
        XCTAssertTrue(MacAssistantFailurePresentationPolicy.highlightsResponse(deliveryState: .failed))
    }

    func testPartialResultsRequireActualCompletedToolOutput() {
        let result = MacAssistantTrace(kind: .tool, title: "Search", detail: "", result: "Found a source", state: .completed)
        let thinking = MacAssistantTrace(kind: .thinking, title: "Thinking", detail: "", result: "Plan", state: .completed)
        XCTAssertTrue(MacAssistantFailurePresentationPolicy.hasPartialToolResults(deliveryState: .failed, traces: [result]))
        XCTAssertTrue(MacAssistantFailurePresentationPolicy.hasPartialToolResults(deliveryState: .cancelled, traces: [result]))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.hasPartialToolResults(deliveryState: .completed, traces: [result]))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.hasPartialToolResults(deliveryState: .failed, traces: [thinking]))
        XCTAssertFalse(MacAssistantFailurePresentationPolicy.hasPartialToolResults(deliveryState: .failed, traces: []))
    }

    func testNotificationAuthorizationPolicySeparatesAppPreferenceFromSystemPermission() {
        XCTAssertFalse(MacNotificationAuthorizationPolicy.isGranted(.notDetermined))
        XCTAssertFalse(MacNotificationAuthorizationPolicy.isGranted(.denied))
        XCTAssertTrue(MacNotificationAuthorizationPolicy.isGranted(.authorized))
        XCTAssertTrue(MacNotificationAuthorizationPolicy.isGranted(.provisional))

        XCTAssertFalse(MacNotificationAuthorizationPolicy.needsSystemSettings(.notDetermined))
        XCTAssertTrue(MacNotificationAuthorizationPolicy.needsSystemSettings(.denied))
        XCTAssertFalse(MacNotificationAuthorizationPolicy.needsSystemSettings(.authorized))
    }

    func testMasterDetailPagesUseSinglePaneNavigationAtCompactWidths() {
        for width: CGFloat in [620, 700, 759] {
            XCTAssertTrue(
                MacNotificationCenterLayoutPolicy.usesCompactNavigation(availableWidth: width)
            )
            XCTAssertTrue(
                MacPromptLibraryLayoutPolicy.usesCompactNavigation(availableWidth: width)
            )
        }
        XCTAssertFalse(
            MacNotificationCenterLayoutPolicy.usesCompactNavigation(availableWidth: 760)
        )
        XCTAssertFalse(
            MacPromptLibraryLayoutPolicy.usesCompactNavigation(availableWidth: 760)
        )
    }

    func testThinkingDisclosureRemainsCollapsedUntilExplicitlyOpened() {
        XCTAssertFalse(MacThinkingDisclosurePolicy.showsTimeline(
            isStreaming: true,
            hasFailedTrace: false,
            hasExplicitDisclosureChoice: false,
            isExpanded: false
        ))
        XCTAssertFalse(MacThinkingDisclosurePolicy.showsTimeline(
            isStreaming: false,
            hasFailedTrace: false,
            hasExplicitDisclosureChoice: false,
            isExpanded: false
        ))
        XCTAssertFalse(MacThinkingDisclosurePolicy.showsTimeline(
            isStreaming: false,
            hasFailedTrace: true,
            hasExplicitDisclosureChoice: false,
            isExpanded: false
        ))
        XCTAssertFalse(MacThinkingDisclosurePolicy.showsTimeline(
            isStreaming: false,
            hasFailedTrace: true,
            hasExplicitDisclosureChoice: true,
            isExpanded: false
        ))
    }

    func testContinuousStatusAnimationHonorsDynamicReduceMotionPolicy() {
        XCTAssertTrue(macContinuousStatusAnimationEnabled(isActive: true, reduceMotion: false))
        XCTAssertFalse(macContinuousStatusAnimationEnabled(isActive: true, reduceMotion: true))
        XCTAssertFalse(macContinuousStatusAnimationEnabled(isActive: false, reduceMotion: false))
        XCTAssertFalse(macContinuousStatusAnimationEnabled(isActive: true, reduceMotion: false, isVisible: false))
        // Re-entering a visible active scene resumes only when motion is permitted.
        XCTAssertTrue(macContinuousStatusAnimationEnabled(isActive: true, reduceMotion: false, isVisible: true))
        XCTAssertFalse(macContinuousStatusAnimationEnabled(isActive: true, reduceMotion: true, isVisible: true))
    }

    func testArchiveCountRejectsLateResultsAcrossConversationSwitches() {
        var state = MacWorkspaceModel.ContextArchiveCountState()
        let firstA = state.begin(conversationID: "a")
        let b = state.begin(conversationID: "b")
        XCTAssertNil(state.count)
        XCTAssertFalse(state.finish(conversationID: "a", request: firstA, count: 99))
        XCTAssertTrue(state.finish(conversationID: "b", request: b, count: 2))
        XCTAssertEqual(state.count, 2)
        let secondA = state.begin(conversationID: "a")
        XCTAssertNil(state.count)
        XCTAssertFalse(state.finish(conversationID: "a", request: firstA, count: 99))
        XCTAssertFalse(state.finish(conversationID: "b", request: b, count: 2))
        XCTAssertTrue(state.finish(conversationID: "a", request: secondA, count: 3))
        XCTAssertEqual(state.count, 3)
    }

    func testArchiveCountDistinguishesFailureFromEmptyAndKeepsValueDuringRefresh() {
        var state = MacWorkspaceModel.ContextArchiveCountState()
        let first = state.begin(conversationID: "a")
        XCTAssertTrue(state.finish(conversationID: "a", request: first, count: 0))
        XCTAssertEqual(state.count, 0)
        XCTAssertFalse(state.failed)
        let refresh = state.begin(conversationID: "a")
        XCTAssertEqual(state.count, 0)
        XCTAssertTrue(state.finish(conversationID: "a", request: refresh, count: nil))
        XCTAssertNil(state.count)
        XCTAssertTrue(state.failed)
        let retry = state.begin(conversationID: "a")
        XCTAssertTrue(state.finish(conversationID: "a", request: retry, count: 8))
        XCTAssertEqual(state.count, 8)
        XCTAssertFalse(state.failed)
    }


    func testBurstPhrasesHaveOrderedBoundedRevealStarts() throws {
        let ids = (0..<20).map { _ in UUID() }
        let now = Date()
        let starts = MacStreamingPresentationTiming.revealStarts(for: ids + [ids[0]], at: now)
        XCTAssertEqual(starts.count, ids.count)
        XCTAssertEqual(starts[ids[0]], now)
        XCTAssertGreaterThan(try XCTUnwrap(starts[ids[1]]), now)
        XCTAssertLessThanOrEqual(
            try XCTUnwrap(starts[ids.last!]).timeIntervalSince(now),
            MacStreamingPresentationTiming.maximumRevealBacklog + 0.001
        )
    }

    func testSharedChatMotionContractUsesRestrainedDurations() {
        XCTAssertEqual(MacDesignTokens.Motion.disclosure, 0.18, accuracy: 0.001)
        XCTAssertEqual(MacDesignTokens.Motion.contentReveal, 0.12, accuracy: 0.001)
        XCTAssertEqual(MacStreamingPresentationTiming.revealDuration, AppleStreamingRevealTiming.duration, accuracy: 0.001)
        XCTAssertEqual(AppleStreamingRevealTiming.remaining(elapsed: 0, position: 0), 1, accuracy: 0.001)
        XCTAssertEqual(AppleStreamingRevealTiming.remaining(elapsed: AppleStreamingRevealTiming.duration, position: 1), 0, accuracy: 0.001)
        XCTAssertGreaterThan(AppleStreamingRevealTiming.remaining(elapsed: AppleStreamingRevealTiming.duration / 2, position: 1),
                             AppleStreamingRevealTiming.remaining(elapsed: AppleStreamingRevealTiming.duration / 2, position: 0))
        XCTAssertEqual(MacStreamingPresentationTiming.maximumRevealBacklog, 0.32, accuracy: 0.001)
    }

    func testTranscriptEntrancePolicyOnlyMarksAppendedSuffix() {
        let first = UUID(), second = UUID(), third = UUID()
        XCTAssertEqual(
            MacTranscriptMessageMotionPolicy.appendedIDs(previousConversationID: "a", conversationID: "a",
                previous: [first, second], current: [first, second, third]),
            [third]
        )
        XCTAssertTrue(MacTranscriptMessageMotionPolicy.appendedIDs(previousConversationID: "a", conversationID: "a",
            previous: nil, current: [first]).isEmpty)
        XCTAssertTrue(MacTranscriptMessageMotionPolicy.appendedIDs(previousConversationID: "a", conversationID: "b",
            previous: [first], current: [first, second]).isEmpty)
        XCTAssertTrue(MacTranscriptMessageMotionPolicy.appendedIDs(previousConversationID: "a", conversationID: "a",
            previous: [second], current: [first, second]).isEmpty)
        XCTAssertTrue(MacTranscriptMessageMotionPolicy.appendedIDs(previousConversationID: "a", conversationID: "a",
            previous: [first, second], current: [third]).isEmpty)
    }





    private func brightTextBounds(_ bitmap: NSBitmapImageRep) throws -> CGRect {
        var minX = bitmap.pixelsWide
        var minY = bitmap.pixelsHigh
        var maxX = -1
        var maxY = -1
        for y in 0..<bitmap.pixelsHigh {
            for x in 0..<bitmap.pixelsWide {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
                      color.alphaComponent > 0.5,
                      min(color.redComponent, color.greenComponent, color.blueComponent) > 0.35 else { continue }
                minX = min(minX, x)
                maxX = max(maxX, x)
                minY = min(minY, y)
                maxY = max(maxY, y)
            }
        }
        XCTAssertGreaterThan(maxX, minX, "Expected visible text in the captured bubble.")
        return CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1)
    }

    func testPoetryProjectionReplayNeverReplacesCommittedNodes() {
        let deltas = [
            "《春江花月夜》是唐代诗人张若虚的代表作，被誉为\"孤篇盖全唐\"。",
            "以下是全文：\n\n---\n\n**春江花月夜**\n（唐·张若虚）\n",
            "\n春江潮水连海平，海上明月共潮生。\n滟滟随波千万里，何处春江无月明！",
            "\n江流宛转绕芳甸，月照花林皆似霰。\n空里流霜不觉飞，汀上白沙看不见。",
            "\n江天一色无纤尘，皎皎空中孤月轮。\n江畔何人初见月？",
            "江月何年初照人？\n人生代代无穷已，江月年年望相似。"
        ]
        var response = ""
        var presentation = MacAssistantPresentation.processing()
        var pacing = MacPacedStreamBuffer(minimumLength: 28, maximumLength: 72)
        for delta in deltas {
            var assembler = MacSemanticStreamAssembler(existingText: response, identitySeed: "poetry")
            for segment in pacing.append(delta) {
                response += segment
                presentation.append(assembler.append(segment))
                presentation.updateMutableTextNode(assembler.mutableTail)
                let nodes = presentation.renderedTextNodes
                XCTAssertEqual(Set(nodes.map(\.id)).count, nodes.count, "Duplicate node identities at \(response)")
                var replay = MacSemanticStreamAssembler(identitySeed: "poetry")
                let expected = replay.append(response)
                XCTAssertEqual(presentation.textNodes, expected, "Projection reload changed committed nodes at \(response)")
                let groups = MacAssistantRenderGroup.make(from: nodes)
                XCTAssertEqual(Set(groups.map(\.id)).count, groups.count)
            }
        }
    }





    func testTranscriptFollowTracksVisibleProseTailWithoutRestartingAtCommit() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: "follow-visible-tail")
        var presentation = MacAssistantPresentation.processing()
        let initial = presentation.lastTextStableIdentity
        presentation.append(assembler.append("尚未完成的短语"))
        presentation.updateMutableTextNode(assembler.mutableTail)
        let liveNode = try XCTUnwrap(presentation.renderedTextNodes.last)
        let liveIdentity = presentation.lastTextStableIdentity
        XCTAssertNotEqual(liveIdentity, initial)

        presentation.append(assembler.append("继续增长"))
        presentation.updateMutableTextNode(assembler.mutableTail)
        XCTAssertEqual(presentation.renderedTextNodes.last?.id, liveNode.id)
        let grownIdentity = presentation.lastTextStableIdentity
        XCTAssertNotEqual(grownIdentity, liveIdentity)

        presentation.append(assembler.finish())
        presentation.updateMutableTextNode(nil)
        XCTAssertEqual(presentation.lastTextStableIdentity, grownIdentity)
    }


    func testComposerTextViewKeepsDefaultEditableTextSystem() {
        let textView = MacAttachmentPasteTextView()
        XCTAssertNotNil(textView.textContainer)
        XCTAssertNotNil(textView.textStorage)
        XCTAssertTrue(textView.isEditable)

        textView.insertText("Hello Nola", replacementRange: NSRange(location: 0, length: 0))

        XCTAssertEqual(textView.string, "Hello Nola")
    }

    func testComposerFirstResponderConsumesAttachmentPasteBeforeTextInsertion() {
        let textView = MacAttachmentPasteTextView()
        textView.string = "Existing caption"
        var pasteRequests = 0
        textView.attachmentPasteHandler = {
            pasteRequests += 1
            return true
        }

        textView.paste(nil)

        XCTAssertEqual(pasteRequests, 1)
        XCTAssertEqual(textView.string, "Existing caption")
    }

    func testComposerTextViewRecognizesNativeImageDragPayload() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        pasteboard.clearContents()
        let image = NSImage(size: NSSize(width: 8, height: 8))
        image.lockFocus()
        NSColor.systemTeal.setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: 8, height: 8)).fill()
        image.unlockFocus()
        XCTAssertTrue(pasteboard.writeObjects([image]))

        let textView = MacAttachmentPasteTextView()
        XCTAssertTrue(textView.canImportAttachments(from: pasteboard))
        var receivedPasteboard: NSPasteboard?
        textView.attachmentDropHandler = {
            receivedPasteboard = $0
            return true
        }
        XCTAssertTrue(try XCTUnwrap(textView.attachmentDropHandler)(pasteboard))
        XCTAssertTrue(receivedPasteboard === pasteboard)
    }

    func testPastedBitmapBecomesAPngAttachmentInsteadOfPathText() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-paste-image-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let pasteboard = NSPasteboard.withUniqueName()
        pasteboard.clearContents()
        let image = NSImage(size: NSSize(width: 4, height: 4))
        image.lockFocus()
        NSColor.systemBlue.setFill()
        NSBezierPath(rect: NSRect(x: 0, y: 0, width: 4, height: 4)).fill()
        image.unlockFocus()
        XCTAssertTrue(pasteboard.writeObjects([image]))

        XCTAssertTrue(model.importPasteboardAttachments(pasteboard))
        let attachment = try XCTUnwrap(model.attachments.first)
        defer { try? FileManager.default.removeItem(at: attachment.url) }
        XCTAssertEqual(model.attachments.count, 1)
        XCTAssertEqual(attachment.url.pathExtension.lowercased(), "png")
        XCTAssertTrue(FileManager.default.fileExists(atPath: attachment.url.path))
    }

    func testComposerDraftAndAttachmentsAreIsolatedByConversation() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-composer-state-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let firstID = try XCTUnwrap(model.selectedConversationID)
        let firstFile = root.appendingPathComponent("first.txt")
        try "first".write(to: firstFile, atomically: true, encoding: .utf8)
        model.draft = "first draft"
        model.attachments = [MacAttachment(url: firstFile)]

        model.newConversation()
        let secondID = try XCTUnwrap(model.selectedConversationID)
        XCTAssertNotEqual(secondID, firstID)
        XCTAssertEqual(model.draft, "")
        XCTAssertTrue(model.attachments.isEmpty)

        let secondFile = root.appendingPathComponent("second.txt")
        try "second".write(to: secondFile, atomically: true, encoding: .utf8)
        model.draft = "second draft"
        model.attachments = [MacAttachment(url: secondFile)]

        model.selectedConversationID = firstID
        XCTAssertEqual(model.draft, "first draft")
        XCTAssertEqual(model.attachments.map(\.url.standardizedFileURL), [firstFile.standardizedFileURL])

        model.selectedConversationID = secondID
        XCTAssertEqual(model.draft, "second draft")
        XCTAssertEqual(model.attachments.map(\.url.standardizedFileURL), [secondFile.standardizedFileURL])

        model.setDraft("late first update", conversationID: firstID)
        XCTAssertEqual(model.draft, "second draft")
        model.selectedConversationID = firstID
        XCTAssertEqual(model.draft, "late first update")
    }

    func testComposerDepthDoesNotRewriteModelParameters() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-thinking-link-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(
            canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        var override = model.conversationModelOverride
        override.temperature = 0.25
        model.conversationModelOverride = override
        model.style = .detailed
        XCTAssertEqual(model.style, .detailed)
        XCTAssertEqual(model.conversationModelOverride.temperature, 0.25)
        let revision = model.modelConfigurationRevision
        model.refreshModelConfiguration()
        XCTAssertGreaterThan(model.modelConfigurationRevision, revision)
    }

    func testPastedFinderFileBecomesAttachmentWithoutChangingDraftText() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-paste-file-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("reference.txt")
        try "attachment body".write(to: file, atomically: true, encoding: .utf8)
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        model.draft = "Keep this caption"
        let pasteboard = NSPasteboard.withUniqueName()
        pasteboard.clearContents()
        XCTAssertTrue(pasteboard.writeObjects([file as NSURL]))

        XCTAssertTrue(model.importPasteboardAttachments(pasteboard))
        XCTAssertEqual(model.draft, "Keep this caption")
        XCTAssertEqual(model.attachments.map(\.url.standardizedFileURL), [file.standardizedFileURL])
    }

    func testPastedPlainExistingFilePathBecomesAttachment() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-paste-path-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("screenshot.png")
        try Data([0x89, 0x50, 0x4E, 0x47]).write(to: file)
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let pasteboard = NSPasteboard.withUniqueName()
        pasteboard.clearContents()
        XCTAssertTrue(pasteboard.setString(file.path, forType: .string))

        XCTAssertTrue(model.importPasteboardAttachments(pasteboard))
        XCTAssertEqual(model.attachments.map(\.url.standardizedFileURL), [file.standardizedFileURL])
    }

    func testDroppedFinderImageBecomesAttachment() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-drop-image-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("dragged.png")
        try Data([0x89, 0x50, 0x4E, 0x47]).write(to: file)
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let provider = try XCTUnwrap(NSItemProvider(contentsOf: file))

        XCTAssertTrue(model.importDroppedAttachments([provider]))
        for _ in 0..<50 where model.attachments.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTAssertEqual(model.attachments.map(\.url.standardizedFileURL), [file.standardizedFileURL])
    }

    func testDroppedImageDataBecomesPersistentAttachment() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-drop-image-data-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        let model = MacWorkspaceModel(canonicalRepository: repository, localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let provider = NSItemProvider()
        let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        provider.registerDataRepresentation(forTypeIdentifier: UTType.png.identifier, visibility: .all) { completion in
            completion(png, nil)
            return nil
        }

        XCTAssertTrue(model.importDroppedAttachments([provider]))
        for _ in 0..<50 where model.attachments.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
        }

        let attachment = try XCTUnwrap(model.attachments.first)
        defer { try? FileManager.default.removeItem(at: attachment.url) }
        XCTAssertEqual(attachment.url.pathExtension.lowercased(), "png")
        XCTAssertTrue(FileManager.default.fileExists(atPath: attachment.url.path))
    }

    func testExternalExecutionBackendDoesNotRequireSelectedNolaModel() {
        XCTAssertTrue(MacExecutionBackendPolicy.requiresNolaModel("nola"))
        XCTAssertFalse(MacExecutionBackendPolicy.requiresNolaModel("codex"))
        XCTAssertFalse(MacExecutionBackendPolicy.requiresNolaModel("claude_code"))
        XCTAssertFalse(MacExecutionBackendPolicy.requiresNolaModel("opencode"))
        XCTAssertEqual(
            MacExecutionBackendPolicy.effectiveModelID("deepseek-v4-flash", backendID: "nola"),
            "deepseek-v4-flash"
        )
        XCTAssertEqual(
            MacExecutionBackendPolicy.effectiveModelID("gpt-5-codex", backendID: "codex"),
            "gpt-5-codex"
        )
        XCTAssertEqual(
            MacExecutionBackendPolicy.effectiveModelID(
                "anthropic/claude-sonnet",
                backendID: "opencode"
            ),
            "anthropic/claude-sonnet"
        )
        XCTAssertEqual(MacExecutionBackendPolicy.externalContextPreparationTokens, 32_768)
    }

    func testExecutionBackendSelectionPersistsPerConversation() async throws {
        let conversationID = "executor-roundtrip-\(UUID().uuidString)"
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-executor-persistence-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        defer { try? FileManager.default.removeItem(at: baseDirectory) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectExecutionBackend(
            MacExecutionBackendOption(id: "claude_code", displayName: "Claude Code"),
            available: true
        )
        try await Task.sleep(for: .milliseconds(700))
        let persisted = try XCTUnwrap(archiveStore.load(conversationID: conversationID))
        XCTAssertEqual(persisted.conversation.executionBackendRawValue, "claude_code")

        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        try await Task.sleep(for: .milliseconds(700))
        XCTAssertEqual(restored.executionBackend.id, "claude_code")
    }
    func testConversationSandboxesAreIsolatedAndManaged() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-session-workspaces-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(
            applicationSupportRoot: root,
            cachesRoot: root.appendingPathComponent("Caches")
        )

        let first = environment.sessionWorkspaceURL(for: "conversation-a")
        let second = environment.sessionWorkspaceURL(for: "conversation-b")

        XCTAssertNotEqual(first, second)
        XCTAssertTrue(FileManager.default.fileExists(atPath: first.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: second.path))
        XCTAssertTrue(MacAppEnvironment.contains(first, in: environment.supportRoot))
        XCTAssertTrue(MacAppEnvironment.contains(second, in: environment.supportRoot))

        try environment.removeSessionWorkspace(for: "conversation-a")
        XCTAssertFalse(FileManager.default.fileExists(atPath: first.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: second.path))
    }

    func testProcessEnvironmentCanIsolateTheFinalMacSupportAndCacheRoots() {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-process-environment-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let support = root.appendingPathComponent("IsolatedSupport/Nola", isDirectory: true)
        let cache = root.appendingPathComponent("IsolatedCaches/Nola", isDirectory: true)

        let environment = MacAppEnvironment(
            environment: [
                "NOLA_MAC_APP_SUPPORT": support.path,
                "NOLA_MAC_CACHE_ROOT": cache.path,
            ]
        )

        XCTAssertEqual(environment.supportRoot.standardizedFileURL, support.standardizedFileURL)
        XCTAssertEqual(environment.cacheRoot.standardizedFileURL, cache.standardizedFileURL)
        XCTAssertEqual(
            environment.databaseURL.standardizedFileURL,
            support.appendingPathComponent("State/nola-v4.db").standardizedFileURL
        )
    }

    func testDefaultTestEnvironmentHasAStableRunIdentityBeyondTheReusedProcessID() throws {
        let first = MacAppEnvironment(environment: [:])
        let second = MacAppEnvironment(environment: [:])
        XCTAssertEqual(first.databaseURL, second.databaseURL)
        XCTAssertEqual(first.cacheRoot, second.cacheRoot)

        let runDirectory = first.supportRoot.deletingLastPathComponent()
            .deletingLastPathComponent().lastPathComponent
        let prefix = "Nola-XCTest-\(ProcessInfo.processInfo.processIdentifier)-"
        XCTAssertTrue(runDirectory.hasPrefix(prefix))
        XCTAssertNotNil(UUID(uuidString: String(runDirectory.dropFirst(prefix.count))))
    }

    func testAgentLaunchEnvironmentDoesNotOverrideConversationPermissionMode() {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-agent-environment-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(
            applicationSupportRoot: root,
            cachesRoot: root.appendingPathComponent("Caches")
        ).launchAgentEnvironment(projectURL: root)

        XCTAssertEqual(environment["NOLA_DESKTOP_HOST"], "1")
        XCTAssertEqual(environment["NOLA_CLIENT_SURFACE"], "mac")
        XCTAssertEqual(environment["NOLA_AGENT_HOST_PROTOCOL"], "1")
        XCTAssertEqual(environment["NOLA_NATIVE_PERMISSION_MODE"], "1")
        XCTAssertEqual(environment["CRASHREPORTER_DISABLE"], "1")
        XCTAssertEqual(environment["CRASH_REPORTER_DISABLE"], "1")
        XCTAssertNil(environment["NOLA_PERMISSION_PROFILE"])
    }

    func testConversationPermissionModeRoundTripsAndDefaultsToCautious() throws {
        XCTAssertEqual(MacPermissionMode.allCases.map(\.rawValue), ["cautious", "allow_all"])
        XCTAssertEqual(MacPermissionMode.cautious.permissionProfile, "cautious")
        XCTAssertTrue(MacPermissionMode.cautious.autoApprovesReversibleWork)
        XCTAssertEqual(MacPermissionMode.cautious.networkPolicy, "ask")
        XCTAssertEqual(try MacPermissionMode.configured(nil), .cautious)
        for invalid in ["safe", "recommended", "moderate", "allow-all", " AllowAll ", "unknown", ""] {
            XCTAssertThrowsError(try MacPermissionMode.configured(invalid))
        }
        for mode in MacPermissionMode.allCases {
            XCTAssertEqual(try MacPermissionMode.configured(mode.rawValue), mode)
        }
        let conversation = MacConversation(
            title: "Current",
            preview: "",
            updatedAt: .now,
            isPinned: false
        )
        XCTAssertNil(conversation.permissionModeRawValue)

        var configured = conversation
        configured.permissionModeRawValue = MacPermissionMode.cautious.rawValue
        let data = try JSONEncoder().encode(configured)
        let restored = try JSONDecoder().decode(MacConversation.self, from: data)
        XCTAssertEqual(restored.permissionModeRawValue, MacPermissionMode.cautious.rawValue)
    }

    func testWorkspacePermissionPromptUsesStructuredFolderArguments() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-folder-permission-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let arguments = try XCTUnwrap(String(
            data: JSONSerialization.data(withJSONObject: ["root": root.path]),
            encoding: .utf8
        ))

        let requested = MacWorkspacePermissionPromptPolicy.requestedFolderURL(
            toolName: "workspace_access",
            arguments: arguments,
            isAuthorized: { _ in false }
        )

        XCTAssertEqual(requested, root.standardizedFileURL)
    }

    func testWorkspaceGrantsSurviveStoreRecreationWithoutChangingDefaultDirectory() async throws {
        let suite = "workspace-grants-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = WorkspaceBookmarkTestIO()
        let first = URL(fileURLWithPath: "/tmp/nola-primary")
        let second = URL(fileURLWithPath: "/tmp/nola-additional")
        io.selections = [first, second, second]
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        let primarySelection = await store.chooseWorkspace(conversationID: "one")
        let additionalSelection = await store.authorizeFolder(suggestedURL: second, conversationID: "one")
        let repeatedSelection = await store.authorizeFolder(suggestedURL: second, conversationID: "one")
        XCTAssertEqual(primarySelection, first)
        XCTAssertEqual(additionalSelection, second)
        XCTAssertEqual(repeatedSelection, second)
        XCTAssertEqual(store.restoreWorkspace(conversationID: "one"), first)
        XCTAssertEqual(io.started, [first, second], "Repeated consent must not leak OS leases")
        store.close()
        XCTAssertEqual(Set(io.stopped), Set([first, second]))

        let restored = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { restored.close() }
        let primary = await restored.restoreWorkspaceAsync(conversationID: "one")
        XCTAssertEqual(primary, first)
        XCTAssertEqual(restored.authorizedWorkspaceURLs(conversationID: "one"), [first, second])
        XCTAssertEqual(restored.restoreWorkspace(conversationID: "one"), first)
        XCTAssertEqual(restored.authorizedWorkspaceURLs(conversationID: "two"), [])
        let request = try MacLocalAgentRunRequest(
            conversationID: "one", prompt: "只读检查", projectURL: first,
            attachmentURLs: [], authorizedWorkspaceURLs: restored.authorizedWorkspaceURLs(conversationID: "one")
        )
        XCTAssertEqual(request.projectURL, first)
        XCTAssertTrue(request.allowedDirectoryArguments.contains(second.path))
    }

    func testAdditionalWorkspaceGrantDoesNotCreatePrimaryAndRevocationIsIsolated() async throws {
        let suite = "workspace-revoke-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = WorkspaceBookmarkTestIO()
        let first = URL(fileURLWithPath: "/tmp/nola-grant-one")
        let second = URL(fileURLWithPath: "/tmp/nola-grant-two")
        io.selections = [first, second]
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let firstGrant = await store.authorizeFolder(suggestedURL: first, conversationID: "one")
        let secondGrant = await store.authorizeFolder(suggestedURL: second, conversationID: "two")
        XCTAssertNotNil(firstGrant)
        XCTAssertNotNil(secondGrant)
        XCTAssertNil(store.restoreWorkspace(conversationID: "one"))
        store.clearWorkspace(conversationID: "one")
        XCTAssertEqual(io.stopped, [first])
        XCTAssertEqual(store.authorizedWorkspaceURLs(conversationID: "one"), [])
        XCTAssertEqual(store.authorizedWorkspaceURLs(conversationID: "two"), [second])
    }

    func testWorkspacePickerCancellationWrongFolderAndBookmarkFailureDoNotGrantAccess() async throws {
        let suite = "workspace-rejection-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = WorkspaceBookmarkTestIO()
        let root = URL(fileURLWithPath: "/tmp/nola-exact-target")
        io.selections = [nil, root.deletingLastPathComponent(), root]
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let cancelled = await store.authorizeFolder(suggestedURL: root, conversationID: "one")
        let wrongFolder = await store.authorizeFolder(suggestedURL: root, conversationID: "one")
        XCTAssertNil(cancelled)
        XCTAssertNil(wrongFolder)
        XCTAssertTrue(io.started.isEmpty)
        io.failBookmark = true
        let failedBookmark = await store.authorizeFolder(suggestedURL: root, conversationID: "one")
        XCTAssertNil(failedBookmark)
        XCTAssertEqual(io.started, [root])
        XCTAssertEqual(io.stopped, [root])
        XCTAssertTrue(store.authorizedWorkspaceURLs(conversationID: "one").isEmpty)
    }

    func testWorkspaceRestoreDoesNotTreatFailedOSActivationAsAuthority() async throws {
        let suite = "workspace-unavailable-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = WorkspaceBookmarkTestIO()
        let root = URL(fileURLWithPath: "/tmp/nola-recoverable-grant")
        io.selections = [root]
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        let initialGrant = await store.authorizeFolder(suggestedURL: root, conversationID: "one")
        XCTAssertNotNil(initialGrant)
        store.close()
        let restored = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { restored.close() }
        io.failStart = true
        XCTAssertTrue(restored.authorizedWorkspaceURLs(conversationID: "one").isEmpty)
        io.failStart = false
        XCTAssertEqual(restored.authorizedWorkspaceURLs(conversationID: "one"), [root])
    }

    func testNativeWorkspacePickerCancellation() async throws {
        guard ProcessInfo.processInfo.environment["NOLA_RUN_NATIVE_WORKSPACE_PANEL_TESTS"] == "1" else {
            throw XCTSkip("Opt-in test opens and cancels a real macOS folder picker.")
        }
        // Exercise AppKit's real modal loop without touching a user's folder
        // or requiring Accessibility automation permissions.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            NSApp.windows.compactMap { $0 as? NSOpenPanel }.forEach { $0.cancel(nil) }
        }
        let selected = await MacSystemWorkspaceBookmarkIO().chooseFolder(suggested: FileManager.default.temporaryDirectory)
        XCTAssertNil(selected)
    }

    func testWorkspaceRevocationWinsOverInFlightBookmarkRestore() async throws {
        let suite = "workspace-restore-race-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let root = URL(fileURLWithPath: "/tmp/nola-revoked-during-restore")
        let initialIO = WorkspaceBookmarkTestIO()
        initialIO.selections = [root]
        let initial = MacSecurityScopedBookmarkStore(defaults: defaults, io: initialIO)
        let initialGrant = await initial.authorizeFolder(suggestedURL: root, conversationID: "one")
        XCTAssertNotNil(initialGrant)
        initial.close()
        let entered = DispatchSemaphore(value: 0)
        let resume = DispatchSemaphore(value: 0)
        let io = WorkspaceBookmarkTestIO(onResolve: {
            entered.signal()
            _ = resume.wait(timeout: .now() + 5)
        })
        let restored = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { restored.close() }
        let task = Task { await restored.restoreWorkspaceAsync(conversationID: "one") }
        let didEnter = await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(returning: entered.wait(timeout: .now() + 3) == .success)
            }
        }
        restored.clearWorkspace(conversationID: "one")
        resume.signal()
        let result = await task.value
        XCTAssertTrue(didEnter)
        XCTAssertNil(result)
        XCTAssertTrue(io.started.isEmpty, "A revoked snapshot must not reacquire OS leases")
        XCTAssertTrue(restored.authorizedWorkspaceURLs(conversationID: "one").isEmpty)
    }


    func testWorkspacePermissionPromptIgnoresProseAndAuthorizedFolders() {
        XCTAssertEqual(MacWorkspacePermissionPromptPolicy.coreGrantScope(
            toolName: "workspace_access", requested: .conversation, usesLocalRuntime: true
        ), .once)
        XCTAssertEqual(MacWorkspacePermissionPromptPolicy.coreGrantScope(
            toolName: "workspace_access", requested: .conversation, usesLocalRuntime: false
        ), .conversation)
        XCTAssertEqual(MacWorkspacePermissionPromptPolicy.coreGrantScope(
            toolName: "write_file", requested: .conversation, usesLocalRuntime: true
        ), .conversation)
        for name in ["bash", "write_file", "workspace_status", "unknown_tool"] {
            XCTAssertNil(MacWorkspacePermissionPromptPolicy.requestedFolderURL(
                toolName: name,
                arguments: #"{"root":"/Users/example/Documents","command":"ls ~/Desktop"}"#,
                isAuthorized: { _ in false }
            ))
        }
        for arguments in [#"{"root":"~/Desktop"}"#, #"{"root":"relative"}"#, #"{"roots":["/tmp"]}"#, #"{"nested":{"root":"/tmp"}}"#] {
            XCTAssertNil(MacWorkspacePermissionPromptPolicy.requestedFolderURL(
                toolName: "workspace_access", arguments: arguments, isAuthorized: { _ in false }
            ))
        }
        XCTAssertNil(MacWorkspacePermissionPromptPolicy.requestedFolderURL(
            toolName: "web_search",
            arguments: #"{"query":"search /Users/example/Documents"}"#,
            isAuthorized: { _ in false }
        ))
        XCTAssertNil(MacWorkspacePermissionPromptPolicy.requestedFolderURL(
            toolName: "workspace_access",
            arguments: #"{"root":"/Users/example/Documents"}"#,
            isAuthorized: { _ in true }
        ))
    }

    func testVoiceInputStopIsSafeBeforeRecordingStarts() {
        let service = MacVoiceInputService()

        service.stop()

        XCTAssertFalse(service.isRecording)
        XCTAssertNil(service.errorMessage)
    }

    func testAssistantArtifactURLsRoundTripForQuickLook() throws {
        let url = URL(fileURLWithPath: "/tmp/report.pdf")
        let message = MacChatMessage(
            role: .assistant,
            text: "Created the report.",
            createdAt: Date(timeIntervalSince1970: 1_700_000_000),
            attachmentNames: ["report.pdf"],
            attachmentURLs: [url]
        )

        let data = try JSONEncoder().encode(message)
        let decoded = try JSONDecoder().decode(MacChatMessage.self, from: data)

        XCTAssertEqual(decoded.attachmentNames, ["report.pdf"])
        XCTAssertEqual(decoded.attachmentURLs, [url])
    }

    func testAgentToolResultDecodesStructuredArtifactsWithoutParsingDisplayText() {
        let path = "/tmp/nola-e2e/screen.png"
        let event = MacAgentHostRuntime.decode(
            line: #"""
            {
              "type":"tool_result",
              "meta":{"screenshot_path":"\#(path)"},
              "trace":{
                "schema_version":1,
                "trace_id":"trace-screen",
                "run_id":"run-screen",
                "conversation_id":"local",
                "turn_id":"turn-screen",
                "event_id":"event-screen",
                "sequence":1,
                "emitted_at":"2026-07-19T00:00:00Z",
                "kind":"tool.completed",
                "span_id":"screen-1",
                "status":"succeeded",
                "visibility":"summary",
                "payload":{
                  "summary":"Screenshot complete",
                  "tool_name":"computer_screenshot",
                  "call_id":"screen-1"
                }
              }
            }
            """#
        )

        XCTAssertEqual(event?.kind, .toolFinished)
        XCTAssertEqual(event?.artifactURLs, [URL(fileURLWithPath: path).standardizedFileURL])
    }

    func testAgentHostDecodesCancelledTraceWithoutSecondTerminalEvent() throws {
        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: #"""
                {
                  "type":"trace",
                  "trace":{
                    "schema_version":1,
                    "trace_id":"trace-cancelled",
                    "run_id":"run-cancelled",
                    "conversation_id":"cancelled-conversation",
                    "turn_id":"turn-cancelled",
                    "event_id":"event-cancelled",
                    "sequence":3,
                    "emitted_at":"2026-07-24T08:00:00Z",
                    "kind":"run.cancelled",
                    "status":"cancelled",
                    "visibility":"summary",
                    "payload":{"summary":"Cancelled by user"}
                  }
                }
                """#,
                conversationID: "cancelled-conversation"
            )
        )

        XCTAssertEqual(event.kind, .status)
        XCTAssertEqual(event.trace?.kind, "run.cancelled")
        XCTAssertNil(event.coreTurnTerminal)
    }

    func testDiagnosticRunTracesDoNotEmitLifecycleTerminals() throws {
        for kind in ["run.completed", "run.failed", "run.cancelled"] {
            let object: [String: Any] = ["type": "trace", "trace": [
                "schema_version": 1, "trace_id": "trace", "run_id": "run",
                "conversation_id": "local", "turn_id": "turn", "event_id": kind,
                "sequence": 1, "emitted_at": "2026-10-12T00:00:00Z",
                "kind": kind, "status": kind == "run.completed" ? "succeeded" : String(kind.dropFirst(4)),
                "visibility": "summary", "payload": [String: String]()
            ]]
            let line = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
            let event = try XCTUnwrap(MacAgentHostRuntime.decode(line: line))
            XCTAssertEqual(event.kind, .status)
            XCTAssertEqual(event.trace?.kind, kind)
            XCTAssertNil(event.coreTurnTerminal)
        }
    }

    func testAgentHostRejectsNestedCoreLifecycleEnvelope() throws {
        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: #"{"protocol_version":2,"session_id":"core-parts","cursor":{"sequence":7},"revision":9,"event":{"type":"part_updated","turn_id":"turn-1","projection_revision":3,"update":{"type":"text_replaced","part_id":"text-1"},"part":{"kind":"text","id":"text-1","message_id":"assistant-1","turn_id":"turn-1","text":"authoritative","is_complete":false}}}"#,
                conversationID: "core-parts"
            )
        )

        XCTAssertEqual(event.kind, .failed)
        XCTAssertTrue(event.isError)
        XCTAssertNil(event.assistantJournalCursor)
    }

    func testAgentHostDecodesJournalCursorWithoutCopyingBodyOrTrace() throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "journal-host", runID: "run", assistantMessageID: "assistant")
        let cursor = try fixture.append("Committed body")
        let cursorObject = try JSONSerialization.jsonObject(with: JSONEncoder().encode(cursor))
        let wire = try JSONSerialization.data(withJSONObject: [
            "type": "assistant_journal_committed_v1", "session_id": "journal-host",
            "source_host_id": fixture.sourceHostID, "through": cursorObject
        ])
        let event = try XCTUnwrap(MacAgentHostRuntime.decode(
            line: String(decoding: wire, as: UTF8.self), conversationID: "journal-host"))
        XCTAssertEqual(event.kind, .status)
        XCTAssertNil(event.trace)
        XCTAssertEqual(event.assistantJournalCursor, cursor)
        XCTAssertEqual(event.text, "")
    }

    private func canonicalTestMessage(_ parts: [[String: Any]], id: String = "assistant",
                                      state: String = "running") throws -> MacChatMessage {
        let message = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self,
            from: JSONSerialization.data(withJSONObject: [
                "message_id": id, "role": "assistant", "state": state, "revision": 2,
                "created_at_ms": 1, "parts": parts
            ]))
        return try MacCanonicalConversationRepository.projectMessage(message)
    }

    func testCanonicalSourceProjectionPreservesMemoryEvidenceThroughCitation() throws {
        let usage: [String: Any] = ["record_id": "memory-2", "component_id": "context-memory-2",
            "scope": "global", "namespace": ["kind": "workspace"], "provenance_kind": "explicit_user",
            "source": "conversation://session-1/message-9", "observed_at": "2026-08-06T00:00:00Z",
            "requires_current_state_verification": true, "truncated": false,
            "supersedes": ["memory-1"], "usage_count": 4, "last_used_at_epoch_ms": 1786000000000]
        let message = try canonicalTestMessage([
            ["kind": "source", "part_id": "source-memory", "revision": 1, "source_id": "memory-2",
             "title": "Workspace preference", "uri": "memory://memory-2", "evidence": ["kind": "memory", "usage": usage]],
            ["kind": "citation", "part_id": "citation-part", "revision": 2, "citation_id": "citation-1",
             "source_id": "memory-2", "title": "Workspace preference", "uri": "memory://memory-2",
             "origin_tool_call_id": "memory-search-1"]
        ])
        let citation = try XCTUnwrap(message.presentation?.renderedSourceReferences.first { $0.citationID == "citation-1" })
        let memory = try XCTUnwrap(citation.memoryUsage)
        XCTAssertEqual(memory.recordID, "memory-2")
        XCTAssertEqual(memory.namespace?.kind, "workspace")
        XCTAssertEqual(memory.supersedes, ["memory-1"])
        XCTAssertEqual(memory.usageCount, 4)
        XCTAssertTrue(memory.requiresCurrentStateVerification)
    }

    func testJournalKeepsPartIdentityAndOldSnapshotsAcrossUnicodeAppend() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "stable-journal", runID: "run", assistantMessageID: "assistant")
        let first = try fixture.append("你好")
        let firstRef = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: first)
        let initialParts = try await MacJournalTextRead.parts(firstRef)
        let second = try fixture.append("🙂 world")
        let secondRef = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: second)
        let advanced = try await MacJournalTextRead.updatedParts(secondRef, from: firstRef, existing: initialParts)
        let complete = try await MacJournalTextRead.parts(secondRef)
        XCTAssertEqual(advanced.map(\.part_id), complete.map(\.part_id))
        XCTAssertEqual(initialParts.map(\.part_id), complete.map(\.part_id))
        let oldText = try await MacJournalTextRead.markdown(firstRef, historicalFallback: "must not render")
        let newText = try await MacJournalTextRead.markdown(secondRef, historicalFallback: "must not render")
        XCTAssertEqual(oldText, "你好")
        XCTAssertEqual(newText, "你好🙂 world")
        XCTAssertTrue(second.advances(first, conversationID: fixture.conversationID, observedRunID: fixture.runID))
        XCTAssertFalse(second.advances(second, conversationID: fixture.conversationID, observedRunID: fixture.runID))
    }



    func testJournalPublishesTextBeforeTerminalAndDoesNotReplayAtCompletion() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "live-journal", runID: "run", assistantMessageID: "assistant")
        let live = try fixture.append("## Verified\n\nFirst.")
        let before = try await MacJournalTextRead.markdown(.init(databaseURL: fixture.databaseURL, cursor: live), historicalFallback: "")
        XCTAssertEqual(before, "## Verified\n\nFirst.")
        _ = try fixture.append(" Second.")
        let seal = try fixture.finish()
        let head = try XCTUnwrap(fixture.heads().first { $0.identity.assistant_message_id == fixture.assistantMessageID })
        let final = try await MacJournalTextRead.markdown(.init(databaseURL: fixture.databaseURL, cursor: head), historicalFallback: "")
        XCTAssertEqual(final, "## Verified\n\nFirst. Second.")
        XCTAssertEqual(seal.identity.run_id, fixture.runID)
        XCTAssertEqual(try fixture.finish(), seal)
        XCTAssertEqual(try fixture.heads().filter { $0.identity.assistant_message_id == fixture.assistantMessageID }.count, 1)
    }

    func testCanonicalDisclosureReplacementClearsOnlyRetractedPart() throws {
        func text(_ id: String, _ value: String, review: String = "approved") -> [String: Any] {
            ["kind": "text", "part_id": id, "revision": 2, "markdown": value, "is_final": true, "review": review]
        }
        let before = try canonicalTestMessage([text("preview", "Provisional."), text("answer", "Verified.")])
        XCTAssertEqual(before.text, "Provisional.Verified.")
        for review in ["pending", "rejected", "interrupted"] {
            let after = try canonicalTestMessage([text("preview", "PRIVATE", review: review), text("answer", "Verified.")])
            XCTAssertEqual(after.text, "Verified.")
            XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: after.text, accepted: before.text), .replace("Verified."))
        }
        let cleared = try canonicalTestMessage([text("preview", ""), text("answer", "Verified.")])
        XCTAssertEqual(cleared.text, "Verified.")
    }

    func testCanonicalTextPartsComposeMarkdownWithinExactMessage() throws {
        let heading: [String: Any] = ["kind": "text", "part_id": "heading", "revision": 1,
            "markdown": "## Verified results\n\n", "is_final": true]
        let table = "| Item | Status |\n| --- | --- |\n| A | Open |\n| B | Open |"
        let message = try canonicalTestMessage([heading, ["kind": "text", "part_id": "table", "revision": 2,
            "markdown": table, "is_final": true]])
        XCTAssertEqual(message.text, "## Verified results\n\n" + table)
        XCTAssertEqual(message.id, MacCanonicalMessageIdentity.stableID("assistant"))
    }

    func testAuthoritativeTextReconciliationDistinguishesAppendFromReplacement() {
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "same", accepted: "same"), .unchanged)
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "你好🌍", accepted: "你好"), .append("🌍"))
        XCTAssertEqual(AuthoritativeTextReconciliation.reconcile(authoritative: "", accepted: "retracted"), .replace(""))
        XCTAssertEqual(
            AuthoritativeTextReconciliation.reconcile(
                authoritative: "A complete answer",
                accepted: "A complete"
            ),
            .append(" answer")
        )
        XCTAssertEqual(
            AuthoritativeTextReconciliation.reconcile(
                authoritative: "A complete",
                accepted: "A complete answer"
            ),
            .replace("A complete")
        )
        XCTAssertEqual(
            AuthoritativeTextReconciliation.reconcile(
                authoritative: "A corrected answer",
                accepted: "A complete answer"
            ),
            .replace("A corrected answer")
        )
    }

    func testJournalMessageOwnersNeverConcatenateAcrossRuns() async throws {
        let first = try MacAssistantJournalTestFixture(conversationID: "owners", runID: "first", assistantMessageID: "first:assistant")
        let second = try MacAssistantJournalTestFixture(conversationID: "owners", runID: "second", assistantMessageID: "second:assistant")
        let a = try first.append("First answer")
        let b = try second.append("Second answer")
        let left = try await MacJournalTextRead.markdown(.init(databaseURL: first.databaseURL, cursor: a), historicalFallback: "")
        let right = try await MacJournalTextRead.markdown(.init(databaseURL: second.databaseURL, cursor: b), historicalFallback: "")
        XCTAssertEqual(left, "First answer")
        XCTAssertEqual(right, "Second answer")
        XCTAssertNotEqual(a.identity.assistant_message_id, b.identity.assistant_message_id)
        XCTAssertFalse(a.advances(nil, conversationID: "owners", observedRunID: "second"))
        do {
            _ = try await MacJournalTextRead.parts(.init(databaseURL: second.databaseURL, cursor: a))
            XCTFail("A foreign database cannot satisfy an owner-bound journal read")
        } catch {}
    }

    func testOwnedChildProcessRegistryStopsRegisteredProcessSynchronously() throws {
        let registry = MacOwnedChildProcessRegistry()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sleep")
        process.arguments = ["30"]
        try process.run()
        registry.register(process)

        registry.stopAll(gracePeriod: 0.05)

        let deadline = Date().addingTimeInterval(1)
        while process.isRunning, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.01))
        }
        XCTAssertFalse(process.isRunning)
    }

    func testSeededSemanticAssemblerKeepsCanonicalNodeIdentityAcrossProjectionReloads() {
        let markdown = "## Result\n\nA stable streamed paragraph.\n\n```swift\nlet value = 1\n```"
        var live = MacSemanticStreamAssembler(identitySeed: "assistant-stable")
        var liveNodes: [MacAssistantTextNode] = []
        for fragment in ["## Result\n\n", "A stable streamed ", "paragraph.\n\n```swift\n", "let value = 1\n```"] {
            liveNodes.append(contentsOf: live.append(fragment))
        }
        liveNodes.append(contentsOf: live.finish())

        var restored = MacSemanticStreamAssembler(identitySeed: "assistant-stable")
        var restoredNodes = restored.append(markdown)
        restoredNodes.append(contentsOf: restored.finish())

        XCTAssertEqual(liveNodes, restoredNodes)
        XCTAssertEqual(liveNodes.map(\.id), restoredNodes.map(\.id))
        XCTAssertEqual(liveNodes.map(\.blockID), restoredNodes.map(\.blockID))
    }

    func testJournalAssistantBoundariesAppendOnceWithoutTerminalReplay() async throws {
        let fixture = try MacAssistantJournalTestFixture(
            conversationID: "append-only", runID: "run", assistantMessageID: "assistant")
        _ = try fixture.append("## Result\n\n")
        _ = try fixture.append("Stable body.")
        let seal = try fixture.finish()
        XCTAssertEqual(try fixture.finish(), seal)
        let heads = try fixture.heads()
        XCTAssertEqual(heads.filter { $0.identity.assistant_message_id == fixture.assistantMessageID }.count, 1)
        let head = try XCTUnwrap(heads.first { $0.identity.assistant_message_id == fixture.assistantMessageID })
        let text = try await MacJournalTextRead.markdown(
            .init(databaseURL: fixture.databaseURL, cursor: head), historicalFallback: "")
        XCTAssertEqual(text, "## Result\n\nStable body.")
    }

    func testCanonicalReasoningIsNotAnswerTextAndDoesNotHideFinalAnswer() throws {
        let message = try canonicalTestMessage([
            ["kind": "reasoning", "part_id": "reasoning", "revision": 1, "title": "Thinking",
             "state": "completed", "summary": "Checking sources"],
            ["kind": "text", "part_id": "answer", "revision": 2, "markdown": "Verified answer", "is_final": true]
        ])
        XCTAssertEqual(message.text, "Verified answer")
        XCTAssertEqual(message.presentation?.traceItems.first { $0.externalID == "reasoning" }?.detail, "Checking sources")
    }

    func testJournalRejectsStaleForeignNotificationsAndCannotWriteAfterSeal() throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "fenced", runID: "run", assistantMessageID: "assistant")
        let first = try fixture.append("Current")
        let second = try fixture.append(" answer")
        XCTAssertFalse(first.advances(second, conversationID: "fenced", observedRunID: "run"))
        XCTAssertFalse(second.advances(first, conversationID: "foreign", observedRunID: "run"))
        XCTAssertFalse(second.advances(first, conversationID: "fenced", observedRunID: "successor"))
        let seal = try fixture.finish()
        XCTAssertThrowsError(try fixture.append("Late answer"))
        XCTAssertEqual(try fixture.finish(), seal)
    }

    func testSealedAnswerHasOneOwnerAndStopsVisibleRun() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "goal-session", runID: "goal-turn", assistantMessageID: "goal-turn:assistant")
        let seal = try fixture.finish("Goal completed exactly once.")
        let heads = try fixture.heads()
        let head = try XCTUnwrap(heads.first { $0.identity.assistant_message_id == fixture.assistantMessageID })
        let answer = try await MacJournalTextRead.markdown(.init(databaseURL: fixture.databaseURL, cursor: head), historicalFallback: "")
        XCTAssertEqual(answer, "Goal completed exactly once.")
        XCTAssertEqual(heads.filter { $0.identity.assistant_message_id == fixture.assistantMessageID }.count, 1)
        XCTAssertEqual(seal.identity.run_id, fixture.runID)
        XCTAssertFalse(MacVisibleRunStatePolicy.isRunning(internalIsRunning: true,
            activeConversationID: fixture.conversationID, terminalObservedConversationIDs: [fixture.conversationID]))
    }

    func testCanonicalTraceRejectsObsoleteBodyChannel() {
        XCTAssertNil(MacAgentHostRuntime.decode(
            line: #"{"type":"trace","trace":{"schema_version":1,"trace_id":"trace-1","run_id":"run-1","conversation_id":"local","message_id":"assistant-1","turn_id":"turn-1","event_id":"event-1","sequence":1,"emitted_at":"2026-08-02T00:00:00Z","kind":"assistant.delta","status":"running","visibility":"summary","payload":{"delta":"current"}}}"#
        )?.trace)
    }

    func testCoreOwnedTraceStillProjectsTerminalFailureLifecycle() throws {
        let trace = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: #"{"type":"trace","trace":{"schema_version":1,"trace_id":"trace-1","run_id":"run-1","conversation_id":"local","message_id":"assistant-1","turn_id":"turn-1","event_id":"event-failed","sequence":2,"emitted_at":"2026-08-25T00:00:00Z","kind":"run.failed","status":"failed","visibility":"summary","payload":{"reason":"append_file is unavailable"}}}"#
            )?.trace
        )
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()

        let outcome = reducer.apply(trace, to: &presentation, projectsContent: false)

        XCTAssertTrue(outcome.isTerminal)
        XCTAssertEqual(presentation.traceItems.first?.state, .failed)
        XCTAssertEqual(presentation.traceItems.first?.detail, "append_file is unavailable")
    }

    func testEmptyJournalPlaceholderIsNotVisibleAnswerOrFallbackText() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "empty", runID: "run", assistantMessageID: "assistant")
        let cursor = try fixture.admittedHead()
        XCTAssertEqual(cursor.sequence, "0")
        XCTAssertTrue(try fixture.heads().isEmpty)
        let value = try await MacJournalTextRead.markdown(.init(databaseURL: fixture.databaseURL, cursor: cursor),
            historicalFallback: "obsolete provisional answer")
        XCTAssertEqual(value, "")
    }

    func testCanonicalReasoningAndToolKeepIdentityAcrossLifecycleProjection() throws {
        func project(_ terminal: Bool) throws -> MacChatMessage {
            try canonicalTestMessage([
                ["kind": "reasoning", "part_id": "reason-1", "revision": terminal ? 2 : 1,
                 "title": "Thinking", "state": terminal ? "completed" : "running",
                 "summary": terminal ? "Inspected" : "Inspecting"],
                ["kind": "tool", "execution_source": ["kind": "model"], "part_id": "tool-1", "revision": terminal ? 2 : 1,
                 "tool_call_id": "call-1", "capability_id": "read_file", "title": "Read file",
                 "state": terminal ? "completed" : "running", "input": ["path": "Doc/README.md"],
                 "origin": ["kind": "builtin"], "summary": terminal ? "Read Doc/README.md" : "",
                 "output_preview": "20 lines of technical output", "output": ["line_count": 20],
                 "artifacts": [], "affected_resources": [], "started_at_ms": 1]
            ])
        }
        let running = try XCTUnwrap(project(false).presentation)
        let completed = try XCTUnwrap(project(true).presentation)
        let prior = try XCTUnwrap(running.traceItems.first { $0.externalID == "tool-1" })
        let tool = try XCTUnwrap(completed.traceItems.first { $0.externalID == "tool-1" })
        XCTAssertEqual(tool.id, prior.id)
        XCTAssertEqual(tool.state, .completed)
        XCTAssertEqual(tool.tool?.toolCallID, "call-1")
        XCTAssertEqual(tool.tool?.capabilityID, "read_file")
        XCTAssertEqual(tool.tool?.summary, "Read Doc/README.md")
        XCTAssertEqual(tool.tool?.outputPreview, "20 lines of technical output")
        XCTAssertEqual(tool.tool?.output, .object(["line_count": .number(20)]))
        XCTAssertEqual(completed.traceItems.first { $0.externalID == "reason-1" }?.detail, "Inspected")
        XCTAssertEqual(completed.traceItems.first { $0.externalID == "reason-1" }?.state, .completed)
        XCTAssertEqual(completed.traceItems.filter { $0.externalID == "reason-1" }.count, 1)
        XCTAssertEqual(completed.traceItems.filter { $0.externalID == "tool-1" }.count, 1)
    }

    func testAgentHostDecodesVersionedTraceEnvelopeAndStructuredToolParameters() throws {
        let rawFixture = try sharedLongTaskTraceFixtureObjects()
        let traceObject = try XCTUnwrap(
            rawFixture.first { ($0["event_id"] as? String) == "event_007" }
        )
        let wrapper: [String: Any] = [
            "type": "tool_use",
            "tool_name": "read_file",
            "args": ["path": "Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md"],
            "trace": traceObject,
        ]
        let lineData = try JSONSerialization.data(withJSONObject: wrapper, options: [.sortedKeys])
        let line = String(decoding: lineData, as: UTF8.self)

        let event = try XCTUnwrap(
            MacAgentHostRuntime.decode(
                line: line,
                conversationID: "conversation_contract"
            )
        )

        XCTAssertEqual(event.kind, .toolStarted)
        XCTAssertEqual(event.eventID, "event_007")
        XCTAssertEqual(event.toolCallID, "tool_read_1")
        XCTAssertEqual(event.trace?.sequence, try XCTUnwrap(traceObject["sequence"] as? NSNumber).uint64Value)
        XCTAssertEqual(event.trace?.payload.toolName, "read_file")
        XCTAssertTrue(event.trace?.payload.arguments?.displayString.contains("SOFTWARE_DESIGN.md") == true)
        XCTAssertTrue(event.artifactURLs.isEmpty)
        XCTAssertEqual(event.trace?.affectedFiles, ["Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md"])
    }

    func testRestoringHistoryPreservesManualConversationTitle() {
        XCTAssertEqual(
            MacConversationTitlePolicy.restoredTitle(
                currentTitle: "Quarterly review",
                firstUserMessage: "Summarize this report"
            ),
            "Quarterly review"
        )
    }

    func testRestoringHistoryNamesOnlyDefaultConversation() {
        XCTAssertEqual(
            MacConversationTitlePolicy.restoredTitle(
                currentTitle: "New Conversation",
                firstUserMessage: "  Analyze\nthis   document  "
            ),
            "Analyze this document"
        )
    }

    func testCanonicalIdentifiersRemainReplaceableConversationTitles() {
        let identifier = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        XCTAssertTrue(MacConversationTitlePolicy.isDefault(identifier))
        XCTAssertTrue(MacConversationTitlePolicy.isDefault("Session 01DAF2BB"))
        XCTAssertEqual(
            MacConversationTitlePolicy.displayTitle(identifier),
            String(localized: "New Conversation")
        )
        XCTAssertEqual(
            MacConversationTitlePolicy.displayTitle("Session 01DAF2BB"),
            String(localized: "New Conversation")
        )
        XCTAssertEqual(
            MacConversationTitlePolicy.displayTitle("Architecture review"),
            "Architecture review"
        )
        XCTAssertEqual(
            MacConversationTitlePolicy.displayTitle(
                "opaque-core-session-id",
                conversationID: "opaque-core-session-id"
            ),
            String(localized: "New Conversation")
        )
        XCTAssertEqual(
            MacConversationTitlePolicy.restoredTitle(
                currentTitle: identifier,
                firstUserMessage: "Inspect the Mac title bar layout"
            ),
            "Inspect the Mac title bar layout"
        )
    }

    func testSessionPrefixedUserTitlesAreNotTreatedAsRuntimeIdentifiers() {
        XCTAssertFalse(MacConversationTitlePolicy.isDefault("Session planning"))
        XCTAssertFalse(MacConversationTitlePolicy.isDefault("Session 2026 retrospective"))
        XCTAssertEqual(
            MacConversationTitlePolicy.displayTitle(
                "Session planning",
                conversationID: "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
            ),
            "Session planning"
        )
    }

    func testGeneratedRuntimeFallbackTitleNeverFlashesItsConversationIdentifier() {
        let identifier = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        let fallback = String(
            format: String(localized: "Conversation %@"),
            String(identifier.prefix(8))
        )
        XCTAssertTrue(MacConversationTitlePolicy.isRuntimePlaceholder(
            fallback,
            conversationID: identifier
        ))
        XCTAssertEqual(
            MacConversationTitlePolicy.displayTitle(
                fallback,
                conversationID: identifier
            ),
            String(localized: "New Conversation")
        )
        XCTAssertFalse(MacConversationTitlePolicy.isRuntimePlaceholder(
            "Conversation design review",
            conversationID: identifier
        ))
        XCTAssertEqual(
            MacConversationTitlePolicy.automaticTitle(
                currentTitle: identifier,
                conversationID: identifier,
                userMessages: ["Inspect the Mac title bar layout"],
                latestSource: "Inspect the Mac title bar layout"
            ),
            "Inspect the Mac title bar layout"
        )
    }

    func testLowSignalGreetingTitleIsReplacedByFirstMeaningfulRequest() {
        XCTAssertEqual(
            MacConversationTitlePolicy.automaticTitle(
                currentTitle: "你好",
                userMessages: ["你好", "帮我用 C++ 写个神经网络算法"],
                latestSource: "帮我用 C++ 写个神经网络算法"
            ),
            "帮我用 C++ 写个神经网络算法"
        )
        XCTAssertNil(
            MacConversationTitlePolicy.automaticTitle(
                currentTitle: "季度报告",
                userMessages: ["请分析季度报告", "补充现金流风险"],
                latestSource: "补充现金流风险"
            )
        )
        XCTAssertEqual(
            MacConversationTitlePolicy.automaticTitle(
                currentTitle: "我好",
                userMessages: ["你好", "我好", "背诵赤壁赋"],
                latestSource: "背诵赤壁赋"
            ),
            "背诵赤壁赋"
        )
    }

    func testCoreContextComponentsReachHostAsSystemMessagesWithoutReplacingQuery() {
        let messages = MacContextCore.hostContextMessages(
            userPrompt: "分析这份报告",
            components: [
                MacCoreContextComponent(
                    kind: "prompt_template",
                    id: "template:analysis",
                    content: "Compare evidence before drawing conclusions."
                ),
                MacCoreContextComponent(
                    kind: "attachment",
                    id: "attachment:report",
                    content: "Quarterly revenue grew by 18 percent.",
                    source: .init(uri: "file:///tmp/report.pdf", title: "report.pdf")
                ),
            ],
            contextWindowTokens: 32_768
        )

        XCTAssertFalse(messages.isEmpty)
        XCTAssertTrue(messages.allSatisfy { $0.role == "system" })
        let combined = messages.map(\.content).joined(separator: "\n")
        XCTAssertTrue(combined.contains("Compare evidence"))
        XCTAssertTrue(combined.contains("Quarterly revenue"))
        XCTAssertFalse(combined.contains("分析这份报告"))
    }

    func testLocalAgentSessionAvailabilityPersistsThroughWorkspaceReload() async throws {
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-host-session-state-\(UUID().uuidString)", isDirectory: true)
        let archiveStore = MacCanonicalConversationRepository(store: try! MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite")))
        let conversationID = "host-session-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: baseDirectory) }

        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.markLocalAgentSessionAvailable(conversationID: conversationID)
        try await Task.sleep(for: .milliseconds(700))

        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        XCTAssertTrue(
            try XCTUnwrap(
                restored.conversations.first(where: { $0.id == conversationID })
            ).isPersisted
        )
    }

    func testPermissionTitleSummarizesCommandWithoutLeakingRuntimePrefix() throws {
        let data = try JSONSerialization.data(withJSONObject: [
                "command": "/usr/bin/python3 \"/Users/example/DerivedData/Nola.app/Contents/Resources/create_document.py\" --spec input.json --output report.docx",
                "cwd": "/Users/example/workspace",
            ])
        let arguments = try XCTUnwrap(String(data: data, encoding: .utf8))
        let title = MacToolPresentationPolicy.displayTitle(
            for: "bash",
            arguments: arguments
        )

        XCTAssertTrue(title.contains("report.docx"))
        XCTAssertFalse(title.contains("DerivedData"))
        XCTAssertFalse(title.contains("/usr/bin/python3"))
    }

    func testStreamingAssistantUpdateKeepsEarlierRepliesAndStableOrder() {
        let firstUser = MacChatMessage(role: .user, text: "First", createdAt: .now)
        let firstReply = MacChatMessage(role: .assistant, text: "Earlier reply", createdAt: .now)
        let secondUser = MacChatMessage(role: .user, text: "Second", createdAt: .now)
        let streamingID = UUID()
        var messages = [firstUser, firstReply, secondUser]

        XCTAssertTrue(
            MacChatTranscriptReducer.upsertAssistant(
                text: "New",
                messageID: streamingID,
                messages: &messages
            )
        )
        XCTAssertTrue(
            MacChatTranscriptReducer.upsertAssistant(
                text: "New reply",
                messageID: streamingID,
                messages: &messages
            )
        )

        XCTAssertEqual(messages.map(\.id), [firstUser.id, firstReply.id, secondUser.id, streamingID])
        XCTAssertEqual(messages[1].text, "Earlier reply")
        XCTAssertEqual(messages[3].text, "New reply")
        XCTAssertEqual(messages.count, 4)
    }

    func testIdenticalStreamingUpdateDoesNotPublishAnotherMutation() {
        let messageID = UUID()
        var messages = [
            MacChatMessage(id: messageID, role: .assistant, text: "Stable", createdAt: .now)
        ]

        XCTAssertFalse(
            MacChatTranscriptReducer.upsertAssistant(
                text: "Stable",
                messageID: messageID,
                messages: &messages
            )
        )
        XCTAssertEqual(messages.count, 1)
    }

    func testCanonicalCompletionRemovesBarePlaceholderButRetainsTraceOnlyMessage() throws {
        let bare = MacCanonicalConversationProjectionV1.Message(
            messageID: "bare", role: .assistant, state: .completed, revision: 1,
            createdAtMs: 1, parts: []
        )
        let traced = MacCanonicalConversationProjectionV1.Message(
            messageID: "traced", role: .assistant, state: .completed, revision: 1,
            createdAtMs: 2, parts: [.reasoning(.init(
                partID: "reason", revision: 1, state: .completed,
                title: "Inspect request",
                summary: "Inspecting the request."
            ))]
        )
        let projection = MacCanonicalConversationProjectionV1(
            contractVersion: 1, conversationID: "completion", revision: 1,
            title: "Completion", messages: [bare, traced], activeRun: nil,
            surface: .init(
                revision: 1, committedMessageIDs: ["bare", "traced"], liveMessageID: nil,
                currentActivityPartID: nil, archivedActivityPartIDs: ["reason"],
                showAssistantPlaceholder: false
            ),
            contextUsage: .init(
                usedTokens: 0, windowTokens: 65536, promptTokenLimit: 57344,
                estimated: true, measuredAtRevision: 1
            )
        )
        let messages = try MacCanonicalConversationRepository.projectMessages(projection)
        XCTAssertEqual(messages.count, 1)
        XCTAssertEqual(messages.first?.id, MacCanonicalMessageIdentity.stableID("traced"))
        XCTAssertTrue(messages.first?.text.isEmpty == true)
        XCTAssertTrue(messages.first?.presentation?.isComplete == true)
        XCTAssertEqual(messages.first?.presentation?.traceItems.count, 1)
        XCTAssertEqual(messages.first?.presentation?.traceItems.first?.externalID, "reason")
    }

    func testCanonicalProjectionKeepsSystemProtocolMessagesOutOfTimeline() throws {
        let system = MacCanonicalConversationProjectionV1.Message(
            messageID: "system-policy", role: .system, state: .completed, revision: 1,
            createdAtMs: 1, parts: []
        )
        let user = MacCanonicalConversationProjectionV1.Message(
            messageID: "user-visible", role: .user, state: .completed, revision: 1,
            createdAtMs: 2, parts: []
        )
        let projection = MacCanonicalConversationProjectionV1(
            contractVersion: 1, conversationID: "system-filter", revision: 1,
            title: "System filter", messages: [system, user], activeRun: nil,
            surface: .init(
                revision: 1, committedMessageIDs: ["system-policy", "user-visible"],
                liveMessageID: nil, currentActivityPartID: nil,
                archivedActivityPartIDs: [], showAssistantPlaceholder: false
            ),
            contextUsage: .init(
                usedTokens: 0, windowTokens: 65_536, promptTokenLimit: 57_344,
                estimated: true, measuredAtRevision: 1
            )
        )

        let messages = try MacCanonicalConversationRepository.projectMessages(projection)
        XCTAssertEqual(messages.map(\.id), [MacCanonicalMessageIdentity.stableID("user-visible")])
        XCTAssertEqual(messages.map(\.role), [.user])
    }

    func testCanonicalCompactionPreservesSourceIdentityThroughPresentation() throws {
        let part = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.Part.self,
            from: Data(#"{"kind":"compaction","part_id":"compact-1","revision":7,"summary":"Earlier work remains available.","source_item_ids":["message-origin","text-origin"],"source_fingerprint":"0123456789abcdef","original_token_count":9000,"compacted_token_count":900}"#.utf8)
        )
        guard case let .compaction(compaction) = part else {
            XCTFail("Expected a canonical compaction Part")
            return
        }
        XCTAssertEqual(compaction.sourceItemIDs, ["message-origin", "text-origin"])
        XCTAssertEqual(compaction.sourceFingerprint, "0123456789abcdef")

        let message = MacCanonicalConversationProjectionV1.Message(
            messageID: "message-summary", role: .assistant, state: .completed,
            revision: 7, createdAtMs: 1, parts: [part]
        )
        let projected = try MacCanonicalConversationRepository.projectMessage(message)
        let reference = try XCTUnwrap(projected.presentation?.renderedCompactionReferences.first)
        XCTAssertEqual(reference.id, "compact-1")
        XCTAssertEqual(reference.sourceItemIDs, ["message-origin", "text-origin"])
        XCTAssertEqual(reference.sourceFingerprint, "0123456789abcdef")
        XCTAssertEqual(projected.canonicalItemIDs, Set(["message-summary", "compact-1"]))
    }

    func testCompactionSourceRequestRejectsStaleOrForeignReferenceIdentity() throws {
        let part = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.Part.self,
            from: Data(#"{"kind":"compaction","part_id":"compact-current","revision":9,"summary":"Bounded summary","source_item_ids":["message-origin","text-origin"],"source_fingerprint":"abcdef0123456789","original_token_count":9000,"compacted_token_count":900}"#.utf8)
        )
        let canonical = MacCanonicalConversationProjectionV1.Message(
            messageID: "summary-message", role: .assistant, state: .completed,
            revision: 9, createdAtMs: 1, parts: [part]
        )
        let message = try MacCanonicalConversationRepository.projectMessage(canonical)
        let exact = MacCompactionSourceRequest(
            conversationID: "conversation-current",
            messageID: message.id,
            compactionPartID: "compact-current",
            sourceItemIDs: ["message-origin", "text-origin"],
            sourceFingerprint: "abcdef0123456789"
        )
        XCTAssertTrue(exact.matches(
            conversationID: "conversation-current", messages: [message]
        ))

        let variants = [
            MacCompactionSourceRequest(
                conversationID: "conversation-old", messageID: message.id,
                compactionPartID: exact.compactionPartID,
                sourceItemIDs: exact.sourceItemIDs, sourceFingerprint: exact.sourceFingerprint
            ),
            MacCompactionSourceRequest(
                conversationID: exact.conversationID, messageID: UUID(),
                compactionPartID: exact.compactionPartID,
                sourceItemIDs: exact.sourceItemIDs, sourceFingerprint: exact.sourceFingerprint
            ),
            MacCompactionSourceRequest(
                conversationID: exact.conversationID, messageID: message.id,
                compactionPartID: "compact-stale",
                sourceItemIDs: exact.sourceItemIDs, sourceFingerprint: exact.sourceFingerprint
            ),
            MacCompactionSourceRequest(
                conversationID: exact.conversationID, messageID: message.id,
                compactionPartID: exact.compactionPartID,
                sourceItemIDs: ["message-origin"], sourceFingerprint: exact.sourceFingerprint
            ),
            MacCompactionSourceRequest(
                conversationID: exact.conversationID, messageID: message.id,
                compactionPartID: exact.compactionPartID,
                sourceItemIDs: exact.sourceItemIDs, sourceFingerprint: "stale-fingerprint"
            ),
        ]
        for request in variants {
            XCTAssertFalse(request.matches(
                conversationID: "conversation-current", messages: [message]
            ))
        }
    }

    func testCompactionNavigatorUsesOnlyCanonicalMessageAndPartIdentity() {
        let originID = UUID()
        let messages = [
            MacChatMessage(
                id: originID,
                role: .user,
                text: "Summary words must not be used for matching.",
                createdAt: .now,
                canonicalItemIDs: ["message-origin", "text-origin"]
            ),
            MacChatMessage(
                role: .assistant,
                text: "message-origin text-origin",
                createdAt: .now,
                canonicalItemIDs: ["unrelated"]
            ),
        ]

        XCTAssertEqual(
            MacCompactionSourceNavigator.targetMessageID(
                sourceItemIDs: ["text-origin"], messages: messages
            ),
            originID
        )
        XCTAssertNil(MacCompactionSourceNavigator.targetMessageID(
            sourceItemIDs: ["Summary words"], messages: messages
        ))
    }

    func testCompactionNavigatorLoadsEarlierPagesUntilCanonicalIdentityMatches() async {
        let targetID = UUID()
        let initial = MacCompactionSourceNavigator.Page(
            requestedBeforeOrdinal: nil, beforeOrdinal: 100, hasMore: true,
            messages: [MacChatMessage(role: .assistant, text: "summary", createdAt: .now)]
        )
        var requested: [Int64] = []
        let outcome = await MacCompactionSourceNavigator.locate(
            sourceItemIDs: ["source-part"], initialPage: initial
        ) { cursor in
            requested.append(cursor)
            if cursor == 100 {
                return .init(
                    requestedBeforeOrdinal: cursor, beforeOrdinal: 80, hasMore: true,
                    messages: [MacChatMessage(
                        role: .user, text: "unrelated", createdAt: .now,
                        canonicalItemIDs: ["other-part"]
                    )]
                )
            }
            return .init(
                requestedBeforeOrdinal: cursor, beforeOrdinal: 60, hasMore: true,
                messages: [MacChatMessage(
                    id: targetID, role: .user, text: "original", createdAt: .now,
                    canonicalItemIDs: ["source-part"]
                )]
            )
        }

        XCTAssertEqual(requested, [100, 80])
        guard case let .found(messageID, page) = outcome else {
            XCTFail("Expected canonical source to be found")
            return
        }
        XCTAssertEqual(messageID, targetID)
        XCTAssertEqual(page.requestedBeforeOrdinal, 80)
    }

    func testCompactionNavigatorReportsEndStalledCursorAndReadError() async {
        let empty = MacCompactionSourceNavigator.Page(
            requestedBeforeOrdinal: nil, beforeOrdinal: nil, hasMore: false, messages: []
        )
        let end = await MacCompactionSourceNavigator.locate(
            sourceItemIDs: ["missing"], initialPage: empty,
            loadEarlier: { _ in XCTFail("End must not issue a read"); return empty }
        )
        XCTAssertEqual(end, .end)

        let initial = MacCompactionSourceNavigator.Page(
            requestedBeforeOrdinal: nil, beforeOrdinal: 100, hasMore: true, messages: []
        )
        let stalled = await MacCompactionSourceNavigator.locate(
            sourceItemIDs: ["missing"], initialPage: initial,
            loadEarlier: { cursor in .init(
                requestedBeforeOrdinal: cursor, beforeOrdinal: cursor,
                hasMore: true, messages: []
            ) }
        )
        XCTAssertEqual(stalled, .stalled)

        enum ReadFailure: Error { case failed }
        let readError = await MacCompactionSourceNavigator.locate(
            sourceItemIDs: ["missing"], initialPage: initial,
            loadEarlier: { _ in throw ReadFailure.failed }
        )
        XCTAssertEqual(readError, .readError)

        let snapshotChanged = await MacCompactionSourceNavigator.locate(
            sourceItemIDs: ["missing"], initialPage: initial,
            loadEarlier: { _ in throw MacCompactionSourceNavigator.LoadError.snapshotChanged }
        )
        XCTAssertEqual(snapshotChanged, .snapshotChanged)
    }

    func testCancelledAssistantPreservesCanonicalStateWithoutInventingAnAnswer() throws {
        let message = MacCanonicalConversationProjectionV1.Message(
            messageID: "cancelled", role: .assistant, state: .cancelled, revision: 1,
            createdAtMs: 1, parts: [.reasoning(.init(
                partID: "cancelled-reason", revision: 1, state: .cancelled,
                title: "Inspect request",
                summary: "Inspecting the request."
            ))]
        )
        let projected = try MacCanonicalConversationRepository.projectMessage(message)
        XCTAssertTrue(projected.text.isEmpty)
        XCTAssertTrue(projected.presentation?.isComplete == true)
        XCTAssertEqual(projected.presentation?.traceItems.first?.state, .cancelled)
        XCTAssertEqual(projected.presentation?.traceItems.first?.externalID, "cancelled-reason")
    }

    func testBufferedFinalPhrasePreventsFalseMissingAnswerFallback() {
        let effective = MacChatTranscriptReducer.effectiveAssistantText(
            committed: "",
            buffered: "你好！😊"
        )

        XCTAssertEqual(effective, "你好！😊")
    }

    func testRealAnswerReplacesFailurePlaceholderInsteadOfAppendingToIt() {
        let fallback = String(
            localized: "Nola could not produce a final answer. Please try again.",
            table: "Chat"
        )
        XCTAssertEqual(
            MacChatTranscriptReducer.replacingFailurePlaceholderIfNeeded(
                current: fallback,
                incoming: "你好！😊"
            ),
            "你好！😊"
        )
        XCTAssertEqual(
            MacChatTranscriptReducer.replacingFailurePlaceholderIfNeeded(
                current: "Hello",
                incoming: " world"
            ),
            "Hello world"
        )
        XCTAssertEqual(
            MacChatTranscriptReducer.replacingFailurePlaceholderIfNeeded(
                current: fallback + "\n",
                incoming: "Late authoritative answer"
            ),
            "Late authoritative answer"
        )
        XCTAssertTrue(
            MacChatTranscriptReducer.isReplaceableFailurePlaceholder(fallback)
        )
        XCTAssertFalse(
            MacChatTranscriptReducer.isReplaceableFailurePlaceholder("A valid answer")
        )
        XCTAssertEqual(
            MacChatTranscriptReducer.replacingFailurePlaceholderIfNeeded(
                current: "Nola 未能生成最终回答，请重试。",
                incoming: "你好！😊"
            ),
            "你好！😊"
        )
        XCTAssertTrue(
            MacChatTranscriptReducer.isReplaceableFailurePlaceholder(
                "Nola 未能生成最终回答，请重试。"
            )
        )
    }

    func testAuthoritativeCoreProjectionIsUsedWhenTransportIsEmpty() {
        let effective = MacChatTranscriptReducer.effectiveAssistantText(
            committed: "",
            buffered: "",
            projected: "你好！😊"
        )

        XCTAssertEqual(effective, "你好！😊")
    }

    func testAuthoritativeCoreReplacementWinsOverDivergentTransportAccumulator() {
        XCTAssertEqual(
            MacChatTranscriptReducer.effectiveAssistantText(
                committed: "stale transport text",
                buffered: "",
                projected: "authoritative Core answer"
            ),
            "authoritative Core answer"
        )
    }

    func testCoreTerminalIsRecognizedWithoutWaitingForProcessEOF() {
        let terminal = MacLocalAgentEvent(
            kind: .finished,
            text: "",
            title: nil,
            detail: "",
            isError: false
        )
        XCTAssertTrue(MacAgentHostEventDrainBuffer.isTerminal(terminal))
    }

    func testVisibleRunStateStopsAtTerminalBeforeInternalFinalizationCompletes() {
        let conversationID = "conversation-terminal"

        XCTAssertTrue(
            MacVisibleRunStatePolicy.isRunning(
                internalIsRunning: true,
                activeConversationID: conversationID,
                terminalObservedConversationIDs: []
            )
        )
        XCTAssertFalse(
            MacVisibleRunStatePolicy.isRunning(
                internalIsRunning: true,
                activeConversationID: conversationID,
                terminalObservedConversationIDs: [conversationID]
            )
        )
        XCTAssertFalse(
            MacVisibleRunStatePolicy.isRunning(
                internalIsRunning: false,
                activeConversationID: conversationID,
                terminalObservedConversationIDs: []
            )
        )
    }

    func testHostDrainTreatsCancellationAsTerminal() {
        let cancelled = MacLocalAgentEvent(
            kind: .cancelled,
            text: "",
            title: nil,
            detail: "cancelled",
            isError: false
        )

        XCTAssertTrue(MacAgentHostEventDrainBuffer.isTerminal(cancelled))
    }

    func testActivityUpdatePreservesIdentityAndStartTime() {
        let id = UUID()
        let startedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let activity = MacAgentActivity(
            id: id,
            title: "Thinking",
            detail: "Inspecting",
            state: .running,
            startedAt: startedAt
        )

        let updated = activity.updating(
            title: "Thinking",
            detail: "Reading the file",
            state: .running
        )

        XCTAssertEqual(updated.id, id)
        XCTAssertEqual(updated.startedAt, startedAt)
        XCTAssertNil(updated.finishedAt)
        XCTAssertEqual(updated.detail, "Reading the file")
    }


    func testToolAndThinkingRuntimeUpdatesDoNotRetriggerTextFollowing() {
        let blockID = UUID()
        var presentation = MacAssistantPresentation()
        presentation.append([
            MacAssistantTextNode(blockID: blockID, kind: .phrase, text: "Stable answer segment.")
        ])
        let textIdentity = presentation.lastTextStableIdentity

        presentation.appendThinking("Inspecting the current state.", replacing: true)
        let thinkingIdentity = presentation.traceItems.first(where: { $0.kind == .thinking })?.id
        presentation.upsertTool(
            externalID: "read-1",
            title: "Read file",
            detail: "path: /tmp/example.md",
            state: .running
        )
        presentation.upsertTool(
            externalID: "read-1",
            title: "Read file",
            detail: "Read 42 lines",
            state: .completed
        )

        XCTAssertEqual(presentation.lastTextStableIdentity, textIdentity)
        XCTAssertEqual(presentation.traceItems.first(where: { $0.kind == .thinking })?.id, thinkingIdentity)
        XCTAssertEqual(presentation.textNodes.count, 1)
        XCTAssertEqual(presentation.traceItems.count, 2)
    }

    func testActivityPresentationDoesNotInferProtocolMeaningFromDisplayText() {
        let activity = MacAgentActivity(
            title: "Host status",
            detail: "request_agent_decision",
            state: .running
        )

        XCTAssertEqual(activity.title, "Host status")
        XCTAssertEqual(activity.detail, "request_agent_decision")
    }

    func testMacChromeConfigurationInjectsOnlyMacNativeBridgeEnvironment() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: directory) }
        let fileURL = directory.appendingPathComponent("native_browser_host.json")
        try Data("""
        {
          "server_url": "http://127.0.0.1:5001",
          "host_id": "chrome-default",
          "token": "test-browser-token"
        }
        """.utf8).write(to: fileURL)

        let configuration = try XCTUnwrap(MacChromeMessagingConfiguration.load(from: fileURL))
        var environment: [String: String] = [:]
        configuration.apply(to: &environment)

        XCTAssertEqual(
            environment["NOLA_NATIVE_BROWSER_SERVER_URL"],
            "http://127.0.0.1:5001"
        )
        XCTAssertEqual(environment["NOLA_NATIVE_BROWSER_TOKEN"], "test-browser-token")
        XCTAssertEqual(environment["NOLA_NATIVE_BROWSER_LOCAL_USER_ID"], "mac-local-user")
        XCTAssertEqual(environment["NOLA_NATIVE_BROWSER_HOST_ID"], "chrome-default")
        XCTAssertNil(environment["NOLA_CLI_WEB_HOST_URL"])
        XCTAssertNil(environment["NOLA_CLI_LOCAL_USER_ID"])
    }

    func testChromeBrokerUsesEphemeralPortDuringXCTestUnlessExplicitlyOverridden() {
        XCTAssertEqual(
            MacChromeMessagingConfiguration.runtimePort(
                environment: [:],
                isRunningTests: true
            ),
            0
        )
        XCTAssertEqual(
            MacChromeMessagingConfiguration.runtimePort(
                environment: ["NOLA_NATIVE_BROWSER_PORT": "45123"],
                isRunningTests: true
            ),
            45_123
        )
        XCTAssertEqual(
            MacChromeMessagingConfiguration.runtimePort(
                environment: [:],
                isRunningTests: false
            ),
            MacChromeMessagingConfiguration.defaultPort
        )
    }

    func testAssistantStatusRemainsVisibleAfterStreamedTextCommits() {
        XCTAssertTrue(
            MacAssistantStatusVisibilityPolicy.shouldShow(
                hasTraces: false,
                isStreaming: true,
                hasCommittedText: true,
                hasLiveActivities: true
            )
        )
        XCTAssertFalse(
            MacAssistantStatusVisibilityPolicy.shouldShow(
                hasTraces: false,
                isStreaming: false,
                hasCommittedText: true,
                hasLiveActivities: true
            )
        )
    }

    func testStreamingProcessTitleDoesNotChangeWhenToolsChange() {
        let runningSearch = MacAssistantTrace(
            kind: .tool,
            title: "web_search",
            detail: "Searching",
            arguments: #"{"query":"current AI competitions"}"#,
            state: .running
        )
        let runningFile = MacAssistantTrace(
            kind: .tool,
            title: "read_file",
            detail: "Reading",
            arguments: #"{"path":"/tmp/result.md"}"#,
            state: .running
        )

        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: [runningSearch], isStreaming: true),
            String(localized: "Activity")
        )
        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: [runningSearch, runningFile], isStreaming: true),
            String(localized: "Activity")
        )
        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: [runningSearch, runningFile], isStreaming: false),
            String(localized: "Activity")
        )
    }

    func testTerminalFailureKeepsStableProcessTitle() {
        let failure = MacAssistantTrace(
            kind: .status,
            title: "Nola",
            detail: "LLM request failed (402 Payment Required)",
            state: .failed
        )

        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: [failure], isStreaming: false),
            String(localized: "Status")
        )
        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: [failure], isStreaming: true),
            String(localized: "Status")
        )
    }

    func testProcessTimelinePreservesReasoningAlongsideConcreteWork() {
        let reasoning = MacAssistantTrace(
            kind: .thinking,
            title: "",
            detail: "I need to rediscover the workspace and think through several options.",
            state: .completed
        )
        let tool = MacAssistantTrace(
            kind: .tool,
            title: "web_search",
            detail: "Searching current sources",
            arguments: #"{"query":"current AI competitions"}"#,
            state: .running
        )

        let visible = MacAssistantArtifactPresentationPolicy.visibleProcessTraces([
            reasoning,
            tool,
        ])
        XCTAssertEqual(visible.map(\.id), [reasoning.id, tool.id])
        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: visible, isStreaming: true),
            String(localized: "Thinking")
        )
        XCTAssertEqual(
            MacAssistantProcessTitlePolicy.title(traces: visible, isStreaming: false),
            String(localized: "Thinking Process")
        )
    }

    func testProcessTimelineShowsFinalArtifactEvenWhenArgumentsMentionScratchSpec() {
        let workspace = "/tmp/workspaces/sessions/run-1"
        let scratchWrite = MacAssistantTrace(
            kind: .tool,
            title: "write_file",
            detail: "Wrote specification",
            arguments: #"{"path":"ai_competitions_spec.json"}"#,
            result: "Wrote specification",
            affectedFiles: ["\(workspace)/ai_competitions_spec.json"],
            state: .completed
        )
        let validatedDocument = MacAssistantTrace(
            kind: .tool,
            title: "bash",
            detail: "validated: true",
            arguments: #"{"command":"create_document.py --spec ai_competitions_spec.json --output report.docx"}"#,
            result: #"{"validated":true}"#,
            affectedFiles: ["\(workspace)/report.docx"],
            state: .completed
        )

        let visible = MacAssistantArtifactPresentationPolicy.visibleProcessTraces([
            scratchWrite,
            validatedDocument,
        ])

        XCTAssertEqual(visible.map(\.id), [scratchWrite.id, validatedDocument.id])
    }

    func testEstimatedContextShowsItsApproximateUsageAndModelWindow() {
        let conversationID = "context-estimated-\(UUID().uuidString)"
        let (root, archiveStore) = makeIsolatedArchiveStore("context-estimated")
        defer { try? FileManager.default.removeItem(at: root) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = conversationID
        model.applyContextSnapshot(
            .init(
                usedTokens: 2_000,
                contextWindowTokens: 128_000,
                promptTokenLimit: 120_000,
                isEstimated: true
            ),
            conversationID: conversationID
        )

        XCTAssertEqual(
            model.contextUsageText,
            String(
                format: String(localized: "Context %@ / %@"),
                "~2k",
                "128k"
            )
        )
    }

    func testRenderGroupIdentityKeepsStablePrefixDuringIncrementalMarkdown() {
        let paragraphID = UUID()
        let heading = MacAssistantTextNode(blockID: UUID(), kind: .markdownBlock, text: "## Plan")
        let first = MacAssistantTextNode(blockID: paragraphID, kind: .phrase, text: "First **stable** phrase. ")
        let second = MacAssistantTextNode(blockID: paragraphID, kind: .phrase, text: "Second phrase.")

        let before = MacAssistantRenderGroup.make(from: [heading, first])
        let after = MacAssistantRenderGroup.make(from: [heading, first, second])

        XCTAssertEqual(before.map(\.id), after.prefix(before.count).map(\.id))
        XCTAssertEqual(before.first?.id, heading.id)
        XCTAssertEqual(after.last?.id, paragraphID)
    }

    func testCompletedMarkdownPartitionIsCachedWhileMutablePrefixesBypassCache() {
        MacMarkdownBlockCache.removeAll()
        var parseCount = 0
        let source = "## Result\n\n```swift\nlet value = 1\n```"
        let parser: @MainActor @Sendable (String) -> [MacMarkdownBlock] = { value in
            parseCount += 1
            return MacMarkdownParser.parse(value)
        }

        let first = MacMarkdownBlockCache.parse(source, cacheable: true, parser: parser)
        let second = MacMarkdownBlockCache.parse(source, cacheable: true, parser: parser)
        XCTAssertEqual(first, second)
        XCTAssertEqual(parseCount, 1)

        _ = MacMarkdownBlockCache.parse(source + "\n", cacheable: false, parser: parser)
        _ = MacMarkdownBlockCache.parse(source + "\n", cacheable: false, parser: parser)
        XCTAssertEqual(parseCount, 3)
    }

    func testCompletedMarkdownStructureIsCachedWhileMutableDocumentsBypassCache() {
        MacMarkdownTextBlockCache.removeAll()
        var parseCount = 0
        let source = "## Result\n\n- first\n- second\n\n| Item | State |\n| --- | --- |\n| Cache | Ready |"
        let parser: @MainActor @Sendable (String) -> [MacMarkdownTextBlock] = { value in
            parseCount += 1
            return MacMarkdownTextParser.parse(value)
        }

        let first = MacMarkdownTextBlockCache.parse(source, cacheable: true, parser: parser)
        let second = MacMarkdownTextBlockCache.parse(source, cacheable: true, parser: parser)
        XCTAssertEqual(first, second)
        XCTAssertEqual(parseCount, 1)

        _ = MacMarkdownTextBlockCache.parse(source + "\n", cacheable: false, parser: parser)
        _ = MacMarkdownTextBlockCache.parse(source + "\n", cacheable: false, parser: parser)
        XCTAssertEqual(parseCount, 3)
    }











    func testToolTraceProjectsNestedHostResultWithoutTransportJSON() throws {
        let nestedResult = [
            "display": "Find files architecture.md (1 matches)",
            "text": "# Find files: architecture.md\nfile architecture.md",
        ]
        let nestedData = try JSONSerialization.data(
            withJSONObject: nestedResult,
            options: [.sortedKeys]
        )
        let hostResult: [String: Any] = [
            "artifacts": [],
            "full_output_ref": NSNull(),
            "output_preview": String(decoding: nestedData, as: UTF8.self),
        ]
        let hostData = try JSONSerialization.data(
            withJSONObject: hostResult,
            options: [.sortedKeys]
        )
        let persisted = try XCTUnwrap(
            MacToolTraceDisplayPolicy.persistedResult(
                toolName: "file_search",
                raw: String(decoding: hostData, as: UTF8.self)
            )
        )
        let trace = MacAssistantTrace(
            kind: .tool,
            title: "file_search",
            detail: "{",
            arguments: #"{"query":"architecture.md","path":"."}"#,
            result: persisted,
            state: .completed
        )

        XCTAssertTrue(
            try XCTUnwrap(MacToolTraceDisplayPolicy.operationTitle(for: trace))
                .contains("architecture.md")
        )
        XCTAssertNil(MacToolTraceDisplayPolicy.headerDetail(for: trace))
        XCTAssertEqual(
            MacToolTraceDisplayPolicy.fullResult(for: trace),
            "Find files architecture.md (1 matches)"
        )
    }

    func testToolTraceDoesNotExposeHostControlEnvelope() {
        let raw = #"{"_nola_control":{"continue":true},"continuation_hint":"internal"}"#
        XCTAssertNil(
            MacToolTraceDisplayPolicy.persistedResult(
                toolName: "read_file",
                raw: raw
            )
        )

        let trace = MacAssistantTrace(
            kind: .tool,
            title: "read_file",
            detail: "",
            result: raw,
            state: .completed
        )
        XCTAssertNil(MacToolTraceDisplayPolicy.fullResult(for: trace))
    }

    func testToolTimelineUsesConcreteOperationInsteadOfCapabilityDomain() throws {
        let trace = MacAssistantTrace(
            kind: .tool,
            title: "write_file",
            detail: "",
            arguments: #"{"path":"/tmp/transformer.py","content":"..."}"#,
            state: .completed
        )

        XCTAssertEqual(
            MacToolTraceDisplayPolicy.operationTitle(for: trace),
            "\(String(localized: "Write file")) transformer.py"
        )
        XCTAssertEqual(
            MacToolPresentationPolicy.displayTitle(
                for: "bash",
                arguments: #"{"command":"python3 transformer.py"}"#
            ),
            "\(String(localized: "Run command")) transformer.py"
        )
        XCTAssertNotEqual(
            MacToolPresentationPolicy.displayTitle(for: "write_file"),
            String(localized: "Files", table: "Chat")
        )
        let outputRead = MacAssistantTrace(
            kind: .tool,
            title: "tool_output_read",
            detail: "",
            arguments: #"{"ref_id":"tool-output-1"}"#,
            state: .completed
        )
        XCTAssertNil(MacToolTraceDisplayPolicy.operationTitle(for: outputRead))
        XCTAssertEqual(
            MacToolPresentationPolicy.displayTitle(for: outputRead.title),
            String(localized: "Read tool output")
        )
        XCTAssertEqual(
            MacToolPresentationPolicy.displayTitle(
                for: "codebase_search",
                arguments: #"{"query":"SessionLedger"}"#
            ),
            "\(String(localized: "Search files")) SessionLedger"
        )
        XCTAssertEqual(
            MacToolPresentationPolicy.displayTitle(for: "codebase_index_status"),
            String(localized: "Inspect workspace")
        )
    }

    func testThinkingSummaryTracksTheStableExecutionStage() {
        let traces = [
            MacAssistantTrace(
                kind: .thinking,
                title: "Thinking",
                detail: "raw planning tokens",
                state: .completed
            ),
            MacAssistantTrace(
                kind: .tool,
                title: "file_search",
                detail: "",
                state: .completed
            ),
            MacAssistantTrace(
                kind: .thinking,
                title: "Thinking",
                detail: "raw review tokens",
                state: .completed
            ),
            MacAssistantTrace(
                kind: .tool,
                title: "read_file",
                detail: "",
                state: .completed
            ),
            MacAssistantTrace(
                kind: .thinking,
                title: "Thinking",
                detail: "raw answer tokens",
                state: .completed
            ),
        ]

        XCTAssertEqual(
            MacThinkingPresentationPolicy.summary(at: 0, in: traces),
            String(localized: "Planning the next step")
        )
        XCTAssertEqual(
            MacThinkingPresentationPolicy.summary(at: 2, in: traces),
            String(localized: "Reviewing the tool result")
        )
        XCTAssertEqual(
            MacThinkingPresentationPolicy.summary(at: 4, in: traces),
            String(localized: "Preparing the answer")
        )
    }

    func testMutableMarkdownTailKeepsIdentityWhenPromotedToCommittedContent() {
        var assembler = MacSemanticStreamAssembler()
        XCTAssertTrue(assembler.append("A **stable").isEmpty)
        let initialTail = try? XCTUnwrap(assembler.mutableTail)
        XCTAssertNotNil(initialTail)

        XCTAssertTrue(assembler.append(" rendered").isEmpty)
        XCTAssertEqual(assembler.mutableTail?.id, initialTail?.id)

        let committed = assembler.append(" phrase**.")
        XCTAssertEqual(committed.first?.id, initialTail?.id)
        XCTAssertNil(assembler.mutableTail)
        XCTAssertEqual(committed.first?.text, "A **stable rendered phrase**.")
    }

    func testPhraseAndFollowingMarkdownBlockHaveDistinctRenderIdentities() {
        let sharedBlockID = UUID()
        let phrase = MacAssistantTextNode(
            blockID: sharedBlockID,
            kind: .phrase,
            text: "Summary"
        )
        let table = MacAssistantTextNode(
            blockID: sharedBlockID,
            kind: .markdownBlock,
            text: "| A | B |\n|---|---|\n| 1 | 2 |"
        )

        let groups = MacAssistantRenderGroup.make(from: [phrase, table])

        XCTAssertEqual(groups.count, 2)
        XCTAssertEqual(Set(groups.map(\.id)).count, groups.count)
        XCTAssertEqual(groups.map(\.plainText), [phrase.text, table.text])
    }

    func testTraceReducerCanAdvanceWithoutDuplicatingCoreOwnedPresentation() throws {
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()
        let trace = try JSONDecoder().decode(
            MacAgentTraceEnvelope.self,
            from: Data(#"""
            {
              "schema_version": 1,
              "trace_id": "trace-core-owned",
              "run_id": "run-core-owned",
              "conversation_id": "conversation-core-owned",
              "message_id": "message-core-owned",
              "turn_id": "turn-core-owned",
              "event_id": "event-tool-started",
              "sequence": 1,
              "emitted_at": "2026-08-22T00:00:00Z",
              "kind": "tool.started",
              "span_id": "tool-call-1",
              "parent_span_id": null,
              "status": "running",
              "visibility": "summary",
              "payload": {
                "tool_name": "web_search",
                "arguments": {"query": "CFCA"}
              }
            }
            """#.utf8)
        )

        let outcome = reducer.apply(trace, to: &presentation, projectsContent: false)

        XCTAssertEqual(outcome.disposition, .applied)
        XCTAssertTrue(presentation.traceItems.isEmpty)
    }

    func testStreamingMarkdownKeepsAdjacentListItemsInOneStableBlock() throws {
        var assembler = MacSemanticStreamAssembler()

        XCTAssertTrue(assembler.append("- First item\n").isEmpty)
        let liveList = try XCTUnwrap(assembler.mutableTail)
        XCTAssertEqual(liveList.kind, .markdownBlock)
        XCTAssertEqual(liveList.text, "- First item")

        XCTAssertTrue(assembler.append("- Second item\n\n").isEmpty,
                      "A blank line may precede another paragraph inside this list item")
        let committed = assembler.append("Outside the list.\n").filter { $0.kind == .markdownBlock }
        XCTAssertEqual(committed.count, 1)
        XCTAssertEqual(committed.first?.id, liveList.id)
        XCTAssertEqual(committed.first?.kind, .markdownBlock)
        XCTAssertEqual(committed.first?.text, "- First item\n- Second item")
        XCTAssertEqual(
            committed.flatMap { MacMarkdownTextParser.parse($0.text) },
            [
                .listItem(marker: "•", children: [.paragraph("First item")]),
                .listItem(marker: "•", children: [.paragraph("Second item")]),
            ]
        )
    }

    func testContextSnapshotPublicationCoalescesInvisibleTokenChanges() {
        let current = MacConversationContextSnapshot(
            usedTokens: 10_000,
            contextWindowTokens: 128_000,
            promptTokenLimit: 124_000,
            isEstimated: false
        )
        let sameVisiblePercentage = MacConversationContextSnapshot(
            usedTokens: 10_040,
            contextWindowTokens: 128_000,
            promptTokenLimit: 124_000,
            isEstimated: false
        )
        let nextVisiblePercentage = MacConversationContextSnapshot(
            usedTokens: 10_200,
            contextWindowTokens: 128_000,
            promptTokenLimit: 124_000,
            isEstimated: false
        )

        XCTAssertFalse(
            MacContextSnapshotPublicationPolicy.shouldPublish(
                current: current,
                next: sameVisiblePercentage
            )
        )
        XCTAssertTrue(
            MacContextSnapshotPublicationPolicy.shouldPublish(
                current: current,
                next: nextVisiblePercentage
            )
        )
        XCTAssertTrue(
            MacContextSnapshotPublicationPolicy.shouldPublish(
                current: current,
                next: MacConversationContextSnapshot(
                    usedTokens: 0,
                    contextWindowTokens: 128_000,
                    promptTokenLimit: 124_000,
                    isEstimated: false
                )
            ),
            "Snapshot freshness must not be inferred from whether usage is zero."
        )
    }






    func testWideMarkdownTableKeepsReadableColumnsInsteadOfSqueezingToViewport() {
        let widths = MacMarkdownTableLayout.columnWidths(
            headers: ["#", "领域", "位置", "问题", "代码证据", "影响"],
            rows: [["1", "工具协议", String(repeating: "long_symbol_", count: 20),
                    String(repeating: "具体问题", count: 30),
                    String(repeating: "`model_request_count` ", count: 20),
                    String(repeating: "影响说明", count: 30)]]
        )
        XCTAssertEqual(widths.count, 6)
        XCTAssertEqual(widths[0], 64)
        XCTAssertGreaterThan(widths.reduce(0, +), 828)
        XCTAssertTrue(widths.dropFirst(2).allSatisfy { $0 == 320 })
        XCTAssertEqual(MacMarkdownTableLayout.columnWidths(headers: ["A", "B"], rows: [["x"]]), [64, 64])
    }



    func testSemanticAssemblerCommitsEquivalentPhrasesForWholeAndCharacterStreams() {
        let source = "第一小段已经完整。Second phrase is also complete. 最后一段结束！"
        var whole = MacSemanticStreamAssembler()
        let wholeNodes = whole.append(source) + whole.finish()

        var character = MacSemanticStreamAssembler()
        var characterNodes: [MacAssistantTextNode] = []
        for value in source.map(String.init) {
            characterNodes += character.append(value)
        }
        characterNodes += character.finish()

        XCTAssertEqual(wholeNodes.map(\.text), characterNodes.map(\.text))
        XCTAssertEqual(wholeNodes.map(\.kind), characterNodes.map(\.kind))
    }

    func testStreamingPresentationUsesAdaptiveBurstCadence() {
        let mediumCadence = MacStreamingPresentationTiming.stepMilliseconds(queuedSegments: 5)

        XCTAssertEqual(
            MacStreamingPresentationTiming.stepMilliseconds(queuedSegments: 1),
            MacStreamingPresentationTiming.semanticStepMilliseconds
        )
        XCTAssertLessThan(
            mediumCadence,
            MacStreamingPresentationTiming.semanticStepMilliseconds
        )
        XCTAssertGreaterThan(
            mediumCadence,
            MacStreamingPresentationTiming.burstStepMilliseconds
        )
        XCTAssertEqual(
            MacStreamingPresentationTiming.stepMilliseconds(queuedSegments: 20),
            MacStreamingPresentationTiming.burstStepMilliseconds
        )
        XCTAssertLessThan(
            MacStreamingPresentationTiming.burstStepMilliseconds,
            MacStreamingPresentationTiming.semanticStepMilliseconds
        )
    }

    func testActiveCanonicalProjectionRefreshesAreFrameCoalesced() {
        let active = MacConversationProjectionPresentationPolicy
            .coalescingMilliseconds(isActive: true)
        XCTAssertGreaterThanOrEqual(active, 16)
        XCTAssertLessThanOrEqual(active, 100)
        XCTAssertEqual(
            MacConversationProjectionPresentationPolicy
                .coalescingMilliseconds(isActive: false),
            0
        )
    }

    func testProjectionSubscriptionFailureBackoffIsBoundedAndDeduplicated() {
        var policy = MacProjectionSubscriptionRetryPolicy()
        let first = policy.failed("not admitted yet")
        XCTAssertEqual(first.milliseconds, 500)
        XCTAssertTrue(first.shouldLog)
        let second = policy.failed("not admitted yet")
        XCTAssertEqual(second.milliseconds, 1_000)
        XCTAssertFalse(second.shouldLog)
        for _ in 0..<100 { _ = policy.failed("not admitted yet") }
        XCTAssertEqual(policy.failed("not admitted yet").milliseconds, 5_000)
        XCTAssertTrue(policy.failed("service unavailable").shouldLog)
        policy.reset()
        XCTAssertTrue(policy.failed("service unavailable").shouldLog)
        XCTAssertEqual(policy.failureCount, 1)
    }

    func testSemanticAssemblerKeepsClosedFenceVisibleUntilStableBoundaryThenCommitsOnce() {
        var assembler = MacSemanticStreamAssembler()
        XCTAssertTrue(assembler.append("```swift\nlet value = 1").isEmpty)
        XCTAssertEqual(assembler.mutableTail?.kind, .code(language: "swift"))
        XCTAssertEqual(assembler.mutableTail?.text, "let value = 1")
        XCTAssertTrue(assembler.append("\n```").isEmpty)
        XCTAssertEqual(assembler.mutableTail?.kind, .code(language: "swift"))
        XCTAssertEqual(assembler.mutableTail?.text, "let value = 1\n```")
        let nodes = assembler.append("\nAfter")
        XCTAssertEqual(nodes.count, 1)
        XCTAssertEqual(nodes.first?.kind, .code(language: "swift"))
        XCTAssertEqual(nodes.first?.text, "let value = 1")
        XCTAssertEqual(assembler.mutableTail?.kind, .phrase)
        XCTAssertEqual(assembler.mutableTail?.text, "After")
    }

    func testSemanticAssemblerNeverExposesPartialFenceMarkers() {
        var assembler = MacSemanticStreamAssembler()

        XCTAssertTrue(assembler.append("```sw").isEmpty)
        XCTAssertNil(assembler.mutableTail)
        XCTAssertTrue(assembler.append("ift\nlet value = 1\n`").isEmpty)
        XCTAssertEqual(assembler.mutableTail?.kind, .code(language: "swift"))
        XCTAssertEqual(assembler.mutableTail?.text, "let value = 1\n")
        XCTAssertTrue(assembler.append("`").isEmpty)
        XCTAssertEqual(assembler.mutableTail?.text, "let value = 1\n")

        let completed = assembler.append("`\n")
        XCTAssertEqual(completed.count, 1)
        XCTAssertEqual(completed.first?.kind, .code(language: "swift"))
        XCTAssertEqual(completed.first?.text, "let value = 1")
    }

    func testSemanticAssemblerDoesNotCloseOnInlineFenceInsideCode() {
        var assembler = MacSemanticStreamAssembler()
        XCTAssertTrue(
            assembler.append("```swift\nlet marker = \"```\"\nlet value = 1").isEmpty
        )
        XCTAssertEqual(
            assembler.mutableTail?.text,
            "let marker = \"```\"\nlet value = 1"
        )
        let completed = assembler.append("\n```\n")
        XCTAssertEqual(completed.count, 1)
        XCTAssertEqual(
            completed.first?.text,
            "let marker = \"```\"\nlet value = 1"
        )
    }

    func testAssistantTraceReducerAccumulatesThinkingAndPairsToolByCallID() {
        var presentation = MacAssistantPresentation()
        presentation.appendThinking("先分析", replacing: false)
        presentation.appendThinking("，再执行。", replacing: false)
        presentation.upsertTool(externalID: "call-1", title: "Read file", detail: "config.json", state: .running)
        let toolID = presentation.traceItems.last?.id
        presentation.upsertTool(externalID: "call-1", title: "Read file", detail: "Read 18 lines", state: .completed)

        XCTAssertEqual(presentation.traceItems.first?.detail, "先分析，再执行。")
        XCTAssertEqual(presentation.traceItems.last?.id, toolID)
        XCTAssertEqual(presentation.traceItems.last?.state, .completed)
        XCTAssertEqual(presentation.traceItems.last?.detail, "Read 18 lines")
    }

    func testCanonicalThinkingAdoptsLiveFallbackWithoutDuplicatingIt() {
        var presentation = MacAssistantPresentation()
        presentation.appendThinking("正在分析", replacing: false)
        let liveID = presentation.traceItems.first?.id

        presentation.upsertCoreThinking(
            partID: "reasoning-1",
            title: "Thinking",
            detail: "正在分析完整约束",
            isComplete: true
        )

        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems.first?.id, liveID)
        XCTAssertEqual(presentation.traceItems.first?.externalID, "reasoning-1")
        XCTAssertEqual(presentation.traceItems.first?.detail, "正在分析完整约束")
        XCTAssertEqual(presentation.traceItems.first?.state, .completed)
    }

    func testLateThinkingDeltaDoesNotDuplicateCompletedCanonicalThinkingDuringRun() {
        var presentation = MacAssistantPresentation()
        presentation.upsertCoreThinking(
            partID: "reasoning-1",
            title: "Thinking Process",
            detail: "已经完成的权威思考过程",
            isComplete: true
        )

        presentation.appendFallbackThinking("已经完成的权威思考过程", replacing: false)

        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems.first?.externalID, "reasoning-1")
        XCTAssertEqual(presentation.traceItems.first?.detail, "已经完成的权威思考过程")
        XCTAssertEqual(presentation.traceItems.first?.state, .completed)
    }

    func testRepeatedToolEventsDoNotRepublishEquivalentTraceState() {
        var presentation = MacAssistantPresentation()
        presentation.upsertTool(
            externalID: "call-1",
            title: "read_file",
            detail: #"{"path":"/tmp/runtime.md"}"#,
            state: .running
        )
        let runningSnapshot = presentation

        presentation.upsertTool(
            externalID: "call-1",
            title: "read_file",
            detail: #"{"path":"/tmp/runtime.md"}"#,
            state: .running
        )
        XCTAssertEqual(presentation, runningSnapshot)

        presentation.upsertTool(
            externalID: "call-1",
            title: "read_file",
            detail: "Read 42 lines",
            state: .completed
        )
        let completedSnapshot = presentation

        presentation.upsertTool(
            externalID: "call-1",
            title: "read_file",
            detail: "Read 42 lines",
            state: .completed
        )
        XCTAssertEqual(presentation, completedSnapshot)
    }

    func testLocalModelStartupActivityKeepsOneStableRowAcrossPhaseChanges() {
        var presentation = MacAssistantPresentation.processing()
        presentation.upsertTool(
            externalID: "local-model-startup:qwen",
            title: "Local models",
            detail: "Starting local model service · Qwen",
            state: .running
        )
        let stableID = presentation.traceItems.first?.id

        presentation.upsertTool(
            externalID: "local-model-startup:qwen",
            title: "Local models",
            detail: "Loading model weights · Qwen",
            state: .running
        )
        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems.first?.id, stableID)
        XCTAssertEqual(presentation.traceItems.first?.detail, "Loading model weights · Qwen")

        presentation.upsertTool(
            externalID: "local-model-startup:qwen",
            title: "Local models",
            detail: "The selected model is running locally and ready for chat.",
            state: .completed
        )
        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems.first?.id, stableID)
        XCTAssertEqual(presentation.traceItems.first?.state, .completed)
    }

    func testPersistedLocalModelSelectionRequiresCurrentTypedEnvelope() throws {
        let profile = MacLocalModelProfile.available[0]

        XCTAssertEqual(MacLocalModelProfile.resolve(profile.id)?.id, profile.id)
        XCTAssertEqual(MacChatModelOption.parse("local:\(profile.id)")?.modelID, profile.id)
        XCTAssertEqual(MacChatModelOption.parse("local:\(profile.id)")?.source, .local)
        XCTAssertNil(MacChatModelOption.parse(profile.id))
        XCTAssertNil(MacChatModelOption.parse("legacy:\(profile.id)"))
        XCTAssertNil(MacChatModelOption.parse("local:"))
    }

    func testLocalModelStartupElapsedTextIsStableAndLocaleIndependent() {
        XCTAssertEqual(MacWorkspaceModel.MacLocalModelStartupPresentation.elapsedText(-1), "0s")
        XCTAssertEqual(MacWorkspaceModel.MacLocalModelStartupPresentation.elapsedText(12), "12s")
        XCTAssertEqual(MacWorkspaceModel.MacLocalModelStartupPresentation.elapsedText(125), "2m 5s")
    }

    func testLocalModelStartupDetailShowsConcretePhaseModelAndElapsedTime() {
        let startedAt = Date(timeIntervalSince1970: 1_000)
        let status = MacLocalModelRuntimeStatus(
            phase: .loadingWeights,
            modelID: MacLocalModelProfile.qwen35_08B.id,
            displayName: MacLocalModelProfile.qwen35_08B.displayName,
            errorDescription: nil,
            startedAt: startedAt,
            phaseStartedAt: startedAt
        )

        let detail = MacWorkspaceModel.MacLocalModelStartupPresentation.detail(
            status: status,
            profile: .qwen35_08B,
            at: Date(timeIntervalSince1970: 1_125)
        )

        XCTAssertTrue(detail.contains(String(localized: "Loading model weights", table: "ModelCenter")))
        XCTAssertTrue(detail.contains(MacLocalModelProfile.qwen35_08B.displayName))
        XCTAssertTrue(detail.contains("2m 5s"))
    }

    func testLocalModelStartupDetailAdvancesWhileWeightsRemainInOnePhase() {
        let startedAt = Date(timeIntervalSince1970: 1_000)
        let status = MacLocalModelRuntimeStatus(
            phase: .loadingWeights,
            modelID: MacLocalModelProfile.qwen35_08B.id,
            displayName: MacLocalModelProfile.qwen35_08B.displayName,
            errorDescription: nil,
            startedAt: startedAt,
            phaseStartedAt: startedAt
        )

        let first = MacWorkspaceModel.MacLocalModelStartupPresentation.detail(
            status: status,
            profile: .qwen35_08B,
            at: Date(timeIntervalSince1970: 1_001)
        )
        let later = MacWorkspaceModel.MacLocalModelStartupPresentation.detail(
            status: status,
            profile: .qwen35_08B,
            at: Date(timeIntervalSince1970: 1_009)
        )

        XCTAssertNotEqual(first, later)
        XCTAssertTrue(first.hasSuffix("1s"))
        XCTAssertTrue(later.hasSuffix("9s"))
    }

    func testAgentHostFailureKeepsExitStatusAndBoundedDiagnostic() {
        let error = MacLocalAgentRuntimeError.processFailed(1, "model configuration failed")
        let description = error.errorDescription ?? ""

        XCTAssertTrue(description.contains("1"))
        XCTAssertTrue(description.contains("model configuration failed"))

        let oversized = String(repeating: "x", count: 5_000)
        let bounded = MacLocalAgentRuntimeError.processFailed(2, oversized).errorDescription ?? ""
        XCTAssertLessThan(bounded.count, oversized.count)
        XCTAssertTrue(bounded.contains("2"))
    }

    func testVersionedTraceReducerKeepsOneBubbleProjectionAndRejectsDuplicateEvents() throws {
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()
        let fixture = try sharedLongTaskTraceFixture()
        var duplicateSnapshot: MacAssistantPresentation?
        var usage: MacAgentTraceUsage?

        for event in fixture {
            let before = presentation
            let outcome = reducer.apply(event, to: &presentation)
            if event.eventID == "event_006", duplicateSnapshot == nil {
                XCTAssertEqual(outcome.disposition, .applied)
                duplicateSnapshot = presentation
            } else if event.eventID == "event_006" {
                XCTAssertEqual(outcome.disposition, .duplicate)
                XCTAssertEqual(presentation, duplicateSnapshot)
                XCTAssertEqual(presentation, before)
            } else {
                XCTAssertEqual(outcome.disposition, .applied)
            }
            usage = outcome.usage ?? usage
        }

        XCTAssertTrue(presentation.textNodes.isEmpty)
        XCTAssertTrue(presentation.isComplete)
        XCTAssertNil(presentation.traceItems.first { $0.externalID == "run-status" })
        XCTAssertEqual(presentation.traceItems.filter { $0.kind == .tool }.count, 2)
        let search = try XCTUnwrap(
            presentation.traceItems.first { $0.externalID == "tool_search_1" }
        )
        XCTAssertEqual(search.state, .completed)
        XCTAssertEqual(search.arguments?.contains("2026年7月伊朗局势"), true)
        XCTAssertEqual(search.result?.contains("20"), true)
        let read = try XCTUnwrap(
            presentation.traceItems.first { $0.externalID == "tool_read_1" }
        )
        XCTAssertEqual(read.state, .completed)
        XCTAssertEqual(read.arguments?.contains("SOFTWARE_DESIGN.md"), true)
        XCTAssertEqual(read.result?.contains("120"), true)
        XCTAssertEqual(read.affectedFiles, ["Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md"])
        XCTAssertEqual(usage?.usedTokens, 8_192)
        XCTAssertEqual(usage?.contextWindowTokens, 131_072)
    }

    func testTraceReducerShowsCompactionProgressWhenCoreOwnsDurableContent() throws {
        let events = try JSONDecoder().decode(
            [MacAgentTraceEnvelope].self,
            from: Data(#"""
            [
              {
                "schema_version":1,"trace_id":"trace_compaction_ui","run_id":"run_1",
                "conversation_id":"conversation_1","message_id":"assistant_1","turn_id":"turn_1",
                "event_id":"event_1","sequence":1,"emitted_at":"2026-09-13T12:00:00+08:00",
                "kind":"phase.changed","status":"running","visibility":"summary",
                "payload":{"phase":"compacting","detail":"Compacting 8 context items"}
              },
              {
                "schema_version":1,"trace_id":"trace_compaction_ui","run_id":"run_1",
                "conversation_id":"conversation_1","message_id":"assistant_1","turn_id":"turn_1",
                "event_id":"event_2","sequence":2,"emitted_at":"2026-09-13T12:00:01+08:00",
                "kind":"context.compacted","status":"succeeded","visibility":"detail",
                "payload":{"phase":"compacted","summary":"Kept the active task and recent messages."}
              }
            ]
            """#.utf8)
        )
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()

        _ = reducer.apply(events[0], to: &presentation, projectsContent: false)
        let progress = try XCTUnwrap(
            presentation.traceItems.first { $0.externalID == "context-compaction" }
        )
        XCTAssertEqual(progress.state, .running)
        XCTAssertEqual(progress.title, String(localized: "Compressing conversation context"))

        _ = reducer.apply(events[1], to: &presentation, projectsContent: false)
        XCTAssertNil(presentation.traceItems.first { $0.externalID == "context-compaction" })
    }

    func testVersionedTraceReducerReportsSequenceGapButStillAppliesEvent() throws {
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()
        let skippedStart = Array(try sharedLongTaskTraceFixture().dropFirst().prefix(3))
        let event = try XCTUnwrap(skippedStart.first)
        let toolStarted = skippedStart[1]
        let followingEvent = try XCTUnwrap(skippedStart.last)
        let snapshot = presentation

        let outcome = reducer.apply(event, to: &presentation)

        XCTAssertEqual(
            outcome.disposition,
            .gap(expected: 1, received: 2)
        )
        XCTAssertEqual(presentation, snapshot)
        XCTAssertEqual(
            reducer.apply(toolStarted, to: &presentation).disposition,
            .applied
        )
        XCTAssertEqual(
            reducer.apply(followingEvent, to: &presentation).disposition,
            .applied
        )
        let search = try XCTUnwrap(presentation.traceItems.first { $0.externalID == "tool_search_1" })
        XCTAssertEqual(search.state, .running)
        XCTAssertTrue(search.arguments?.contains("2026年7月伊朗局势") == true)
        XCTAssertTrue(presentation.textNodes.isEmpty)
    }

    func testCanonicalReasoningPreservesCompleteBodyWithoutTraceDeltas() throws {
        let summary = String(repeating: "想", count: 80) + "。"
        let message = try canonicalTestMessage([
            ["kind": "reasoning", "part_id": "reasoning", "revision": 81,
             "title": "Thinking", "state": "completed", "summary": summary],
            ["kind": "text", "part_id": "answer", "revision": 1,
             "markdown": "Verified answer", "is_final": true]
        ])
        let thinking = try XCTUnwrap(message.presentation?.traceItems.first { $0.externalID == "reasoning" })
        XCTAssertEqual(thinking.detail, summary)
        XCTAssertEqual(thinking.state, .completed)
        XCTAssertEqual(message.text, "Verified answer")
    }

    func testBuiltInSkillUsesLocalizedToolPresentationInsteadOfUnknownFallback() {
        XCTAssertEqual(
            MacToolPresentationPolicy.displayTitle(for: "Skill"),
            NSLocalizedString(
                "chat.host_tool.name.skill",
                value: "Use skill",
                comment: "Localized Agent Host tool title"
            )
        )
    }

    func testCoreToolProjectionReplacesProvisionalToolTitle() {
        var presentation = MacAssistantPresentation()
        presentation.upsertTool(
            externalID: "part-tool-search",
            title: String(localized: "Tool"),
            detail: "running",
            state: .running
        )
        presentation.upsertTool(
            externalID: "part-tool-search",
            title: "read_file",
            detail: "5 tool matches",
            state: .completed,
            result: "5 tool matches"
        )

        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems[0].title, "read_file")
        XCTAssertEqual(presentation.traceItems[0].state, .completed)
    }

    func testCanonicalProtocolToolRemovesOnlyItsProvisionalRow() {
        var presentation = MacAssistantPresentation()
        presentation.upsertTool(
            externalID: "internal-part", title: String(localized: "Tool"),
            detail: "running", state: .running
        )
        presentation.upsertTool(
            externalID: "file-part", title: "read_file",
            detail: "reading", state: .running
        )
        presentation.upsertTool(
            externalID: "internal-part", title: "tool_search",
            detail: "5 tool matches", state: .completed, result: "5 tool matches"
        )
        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems.first?.externalID, "file-part")
        XCTAssertEqual(presentation.traceItems.first?.state, .running)
    }

    func testRealMacLongTaskTraceKeepsOneToolAndStableParametersAcrossDuplicateEvents() throws {
        let raw = #"""
        [
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_001",
            "sequence":1,
            "emitted_at":"2026-07-19T01:42:49.592+08:00",
            "kind":"run.started",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"running",
            "visibility":"summary",
            "payload":{"summary":"run_started"}
          },
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_002",
            "sequence":2,
            "emitted_at":"2026-07-19T01:42:49.592+08:00",
            "kind":"phase.changed",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"running",
            "visibility":"summary",
            "payload":{"phase":"testing"}
          },
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_003",
            "sequence":3,
            "emitted_at":"2026-07-19T01:42:49.628+08:00",
            "kind":"tool.started",
            "span_id":"shell_make_demo",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"running",
            "visibility":"summary",
            "payload":{
              "summary":"编译并运行测试",
              "tool_name":"shell_command",
              "call_id":"shell_make_demo",
              "arguments":{
                "command":"make && ./neural_network_demo",
                "cwd":"/Users/gaojc/Library/Application Support/Nola/Workspaces/Default/cpp_neural_network",
                "timeout_sec":30
              }
            }
          },
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_004",
            "sequence":4,
            "emitted_at":"2026-07-19T01:42:50.891+08:00",
            "kind":"tool.completed",
            "span_id":"shell_make_demo",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"succeeded",
            "visibility":"summary",
            "payload":{
              "summary":"测试完成",
              "tool_name":"shell_command",
              "call_id":"shell_make_demo",
              "result":{"exit_code":0}
            }
          },
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_004",
            "sequence":4,
            "emitted_at":"2026-07-19T01:42:50.891+08:00",
            "kind":"tool.completed",
            "span_id":"shell_make_demo",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"succeeded",
            "visibility":"summary",
            "payload":{
              "summary":"测试完成",
              "tool_name":"shell_command",
              "call_id":"shell_make_demo",
              "result":{"exit_code":0}
            }
          },
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_005",
            "sequence":5,
            "emitted_at":"2026-07-19T01:45:28.891+08:00",
            "kind":"phase.changed",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"running",
            "visibility":"summary",
            "payload":{"phase":"processing"}
          },
          {
            "schema_version":1,
            "trace_id":"trace_real_mac_long_task",
            "run_id":"run_real_mac_long_task",
            "conversation_id":"A6880E1F-0F66-4041-A497-A3E7E64542F6",
            "message_id":"assistant_real_mac_long_task",
            "turn_id":"turn_real_mac_long_task",
            "event_id":"real_006",
            "sequence":6,
            "emitted_at":"2026-07-19T01:45:29.028+08:00",
            "kind":"run.completed",
            "parent_span_id":"turn_real_mac_long_task",
            "status":"succeeded",
            "visibility":"summary",
            "payload":{"reason":"completed"}
          }
        ]
        """#
        let events = try JSONDecoder().decode(
            [MacAgentTraceEnvelope].self,
            from: try XCTUnwrap(raw.data(using: .utf8))
        )
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()
        var duplicateSnapshot: MacAssistantPresentation?

        for event in events {
            let outcome = reducer.apply(event, to: &presentation)
            if event.eventID == "real_004", duplicateSnapshot == nil {
                XCTAssertEqual(outcome.disposition, .applied)
                duplicateSnapshot = presentation
            } else if event.eventID == "real_004" {
                XCTAssertEqual(outcome.disposition, .duplicate)
                XCTAssertEqual(presentation, duplicateSnapshot)
            }
        }

        let tool = try XCTUnwrap(
            presentation.traceItems.first { $0.externalID == "shell_make_demo" }
        )
        XCTAssertEqual(presentation.traceItems.filter { $0.kind == .tool }.count, 1)
        XCTAssertEqual(tool.state, .completed)
        XCTAssertTrue(tool.arguments?.contains("make && ./neural_network_demo") == true)
        XCTAssertTrue(tool.result?.contains("\"exit_code\" : 0") == true)
        XCTAssertTrue(presentation.traceItems.filter { $0.kind == .thinking }.isEmpty)
        XCTAssertTrue(presentation.textNodes.isEmpty)
        XCTAssertTrue(presentation.isComplete)
    }

    func testAssistantPresentationRoundTrips() throws {
        var presentation = MacAssistantPresentation()
        presentation.appendThinking("Reasoning", replacing: true)
        let message = MacChatMessage(role: .assistant, text: "Answer", createdAt: .now, presentation: presentation)
        let roundTrip = try JSONDecoder().decode(MacChatMessage.self, from: JSONEncoder().encode(message))
        XCTAssertEqual(roundTrip.presentation?.traceItems.first?.detail, "Reasoning")

    }

    private func sharedLongTaskTraceFixture() throws -> [MacAgentTraceEnvelope] {
        try JSONDecoder().decode(
            [MacAgentTraceEnvelope].self,
            from: Data(contentsOf: try sharedLongTaskTraceFixtureURL())
        )
    }

    private func sharedLongTaskTraceFixtureObjects() throws -> [[String: Any]] {
        try XCTUnwrap(
            JSONSerialization.jsonObject(
                with: Data(contentsOf: try sharedLongTaskTraceFixtureURL())
            ) as? [[String: Any]]
        )
    }

    private func sharedLongTaskTraceFixtureURL() throws -> URL {
        try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "long_task_trace_v1",
                withExtension: "json"
            )
        )
    }

    func testWebSearchTracePairsStartAndFailureForBubbleDisplay() {
        var presentation = MacAssistantPresentation()
        presentation.upsertTool(
            externalID: "web-call-1",
            title: "web_search",
            detail: "特朗普 最新表态",
            state: .running
        )
        presentation.upsertTool(
            externalID: "web-call-1",
            title: "web_search",
            detail: "No parseable search results",
            state: .failed
        )

        XCTAssertEqual(presentation.traceItems.count, 1)
        XCTAssertEqual(presentation.traceItems.first?.state, .failed)
        XCTAssertEqual(presentation.traceItems.first?.detail, "No parseable search results")
    }

    func testInternalTaskStatusToolNeverAppearsInAssistantTimeline() throws {
        let eventData = Data(#"""
        {
          "schema_version": 1,
          "trace_id": "trace-internal",
          "run_id": "run-internal",
          "conversation_id": "conversation-internal",
          "message_id": "message-internal",
          "turn_id": "turn-internal",
          "event_id": "event-internal",
          "sequence": 1,
          "emitted_at": "2026-07-19T00:00:00Z",
          "kind": "tool.started",
          "span_id": "status-call",
          "parent_span_id": null,
          "status": "running",
          "visibility": "internal",
          "payload": {
            "tool_name": "agent_task_status",
            "arguments": {"status": "completed"}
          }
        }
        """#.utf8)
        let event = try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: eventData)
        var presentation = MacAssistantPresentation.processing()
        var reducer = MacAgentTraceReducer()

        XCTAssertEqual(reducer.apply(event, to: &presentation).disposition, .applied)
        XCTAssertTrue(presentation.traceItems.isEmpty)
    }

    func testToolResultCanExposeFullOutputWithoutSilentPreviewLoss() throws {
        let output = String(repeating: "result line\n", count: 1_500)
        let trace = MacAssistantTrace(
            kind: .tool,
            title: "shell",
            detail: "command completed",
            result: output,
            state: .completed
        )

        XCTAssertEqual(MacToolTraceDisplayPolicy.fullResult(for: trace), output.trimmingCharacters(in: .newlines))
        XCTAssertTrue(try XCTUnwrap(MacToolTraceDisplayPolicy.result(for: trace)).hasSuffix("…"))
    }





    @MainActor
    func testNativeViewportKeepsBottomAcrossLateGrowthAndUserReturn() async throws {
        final class Document: NSView { override var isFlipped: Bool { true } }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 1500))
        scroll.documentView = document
        let controller = MacTranscriptScrollController()
        let bridge = MacTranscriptScrollObserver.Coordinator(controller: controller)
        let attachment = NSView(frame: .zero)
        // makeNSView runs before SwiftUI inserts the observer into its scroll hierarchy.
        bridge.attachWhenReady(to: attachment)
        document.addSubview(attachment)
        defer { bridge.detach() }
        let attachmentDeadline = Date().addingTimeInterval(2)
        while abs(scroll.contentView.bounds.maxY - document.bounds.height) > 1 && Date() < attachmentDeadline {
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(scroll.contentView.bounds.maxY, document.bounds.height, accuracy: 1)

        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 300))
        NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
        document.setFrameSize(NSSize(width: 600, height: 2200))
        controller.reconcileLayout()
        XCTAssertEqual(scroll.contentView.bounds.minY, 300, accuracy: 1)
        XCTAssertFalse(controller.followsLatest)

        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 2200 - scroll.contentView.bounds.height))
        NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
        XCTAssertTrue(controller.followsLatest)
        // Simulate delayed Markdown/tool layout, well beyond two display frames.
        for height in [2800.0, 3900.0, 5000.0] {
            try await Task.sleep(for: .milliseconds(45))
            document.setFrameSize(NSSize(width: 600, height: height))
            let deadline = Date().addingTimeInterval(2)
            while abs(scroll.contentView.bounds.maxY - height) > 1 && Date() < deadline {
                try await Task.sleep(for: .milliseconds(5))
            }
            XCTAssertTrue(controller.followsLatest)
            XCTAssertEqual(scroll.contentView.bounds.maxY, height, accuracy: 1)
        }
    }

    @MainActor
    func testReadingAnchorSurvivesPrependAndOldNavigationCannotOverrideUser() {
        final class Document: NSView { override var isFlipped: Bool { true } }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 2000))
        scroll.documentView = document
        let message = NSView(frame: NSRect(x: 0, y: 400, width: 600, height: 600))
        document.addSubview(message)
        let controller = MacTranscriptScrollController()
        controller.attach(scroll)
        defer { controller.detach() }
        controller.register(messageID: UUID(), view: message)
        controller.userBeganScrolling()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 500))
        controller.userEndedScrolling(distanceFromBottom: 1100)
        message.setFrameOrigin(NSPoint(x: 0, y: 700))
        document.setFrameSize(NSSize(width: 600, height: 2300))
        controller.reconcileLayout()
        XCTAssertEqual(scroll.contentView.bounds.minY, 800, accuracy: 1)

        let navigation = controller.beginNavigation()
        controller.returnToLatest()
        XCTAssertFalse(controller.acceptsNavigation(navigation))
        controller.reconcileLayout()
        XCTAssertEqual(scroll.contentView.bounds.maxY, document.bounds.height, accuracy: 1)
        let oldConversationNavigation = controller.beginNavigation()
        controller.reset(conversationID: "another")
        XCTAssertFalse(controller.acceptsNavigation(oldConversationNavigation))
    }






    func testRepeatedToolGroupNeverMovesEarlierTimelineItems() throws {
        let firstCommand = MacAssistantTrace(
            kind: .tool,
            title: "bash",
            detail: "Inspect status",
            state: .completed
        )
        let reasoning = MacAssistantTrace(
            kind: .thinking,
            title: "Thinking",
            detail: "Reviewing the first result",
            state: .completed
        )
        let currentCommand = MacAssistantTrace(
            kind: .tool,
            title: "bash",
            detail: "Inspect diff",
            state: .running
        )

        let groups = MacAssistantTraceTimelinePolicy.groupedDetailTraces([
            firstCommand,
            reasoning,
            currentCommand,
        ])

        XCTAssertEqual(groups.count, 3)
        XCTAssertEqual(groups.map(\.id), [firstCommand.id, reasoning.id, currentCommand.id])
        let adjacent = MacAssistantTraceTimelinePolicy.groupedDetailTraces([firstCommand, currentCommand])
        XCTAssertEqual(adjacent.count, 1)
        XCTAssertEqual(adjacent[0].traces.map(\.id), [firstCommand.id, currentCommand.id])
    }

    func testActivityHistoryPageIsBoundedAndStableDuringStreaming() {
        let traces = (0..<136).map { index in
            MacAssistantTrace(kind: .tool, title: "bash", detail: "step-\(index)", state: .completed)
        }
        let latest = MacAssistantTraceTimelinePolicy.page(traces, endingAt: nil)
        XCTAssertEqual(latest.traces.count, 24)
        XCTAssertEqual(latest.traces.last?.id, traces.last?.id)
        let anchor = latest.earlierEndID
        let history = MacAssistantTraceTimelinePolicy.page(traces, endingAt: anchor)
        let appended = traces + [MacAssistantTrace(kind: .tool, title: "read_file", detail: "new", state: .running)]
        XCTAssertEqual(MacAssistantTraceTimelinePolicy.page(appended, endingAt: anchor).traces.map(\.id),
                       history.traces.map(\.id))
        var seen = latest.traces.map(\.id)
        var cursor = latest.earlierEndID
        while let end = cursor {
            let page = MacAssistantTraceTimelinePolicy.page(traces, endingAt: end)
            XCTAssertLessThanOrEqual(page.traces.count, 24)
            seen.insert(contentsOf: page.traces.map(\.id), at: 0)
            cursor = page.earlierEndID
        }
        XCTAssertEqual(seen, traces.map(\.id))
        let missing = MacAssistantTraceTimelinePolicy.page(traces, endingAt: UUID())
        XCTAssertTrue(missing.isHistory)
        XCTAssertTrue(missing.traces.isEmpty, "a removed anchor must not silently navigate to the live tail")
    }

    func testTranscriptWindowInspectsOnlyTheVisibleTailOfLongHistory() {
        let messages = (0..<10_000).map { index in
            MacChatMessage(
                role: index.isMultiple(of: 2) ? .user : .assistant,
                text: "message-\(index)",
                createdAt: Date(timeIntervalSince1970: TimeInterval(index))
            )
        }

        let window = MacTranscriptWindowPolicy.project(
            messages,
            isRunning: false,
            visibleMessageLimit: 20
        )

        XCTAssertEqual(window.messages.map(\.text), messages.suffix(20).map(\.text))
        XCTAssertTrue(window.hasEarlierVisibleMessages)
        XCTAssertEqual(window.inspectedMessageCount, 21)
    }

    func testTranscriptWindowPreservesActiveEmptyAssistantAndByteBudgetOrder() {
        let earlier = MacChatMessage(role: .user, text: "earlier", createdAt: .now)
        let large = MacChatMessage(
            role: .assistant,
            text: String(repeating: "x", count: 159_999),
            createdAt: .now
        )
        let active = MacChatMessage(role: .assistant, text: "", createdAt: .now)

        let window = MacTranscriptWindowPolicy.project(
            [earlier, large, active],
            isRunning: true,
            visibleMessageLimit: 20
        )

        XCTAssertEqual(window.messages.map(\.id), [large.id, active.id])
        XCTAssertTrue(window.hasEarlierVisibleMessages)
    }

    func testInlineTitleRenameUpdatesHeaderAndSidebarWithoutSecondaryMetadataWrite() {
        let conversationID = "rename-\(UUID().uuidString)"
        let (root, archiveStore) = makeIsolatedArchiveStore("rename")
        defer { try? FileManager.default.removeItem(at: root) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = conversationID
        model.renameSelectedConversation(to: "  Product strategy  ")

        XCTAssertEqual(model.selectedConversation?.title, "Product strategy")
        XCTAssertEqual(
            model.conversations.first(where: { $0.id == conversationID })?.title,
            "Product strategy"
        )
    }

    func testDeletingDraftConversationPreservesGeneratedFiles() async throws {
        let conversationID = "delete-draft-\(UUID().uuidString)"
        let (root, archiveStore) = makeIsolatedArchiveStore("delete-draft")
        defer { try? FileManager.default.removeItem(at: root) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let index = try XCTUnwrap(model.conversations.firstIndex { $0.id == conversationID })
        model.conversations[index].isPersisted = false
        let conversation = model.conversations[index]
        let workspace = MacAppEnvironment.shared.sessionWorkspaceURL(for: conversationID)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let generated = workspace.appendingPathComponent("generated.txt")
        try "Keep user work".write(to: generated, atomically: true, encoding: .utf8)

        model.deleteConversation(conversation)
        model.deleteConversation(conversation)
        for _ in 0..<200 where model.deletingConversationIDs.contains(conversationID) {
            await Task.yield()
        }

        XCTAssertFalse(model.conversations.contains { $0.id == conversationID })
        XCTAssertNil(model.conversationOperationError)
        XCTAssertEqual(try String(contentsOf: generated, encoding: .utf8), "Keep user work")
    }

    func testFailedConversationDeletionKeepsEntryAndExposesError() async throws {
        let conversationID = "delete-failure-\(UUID().uuidString)"
        let (root, archiveStore) = makeIsolatedArchiveStore("delete-failure")
        defer { try? FileManager.default.removeItem(at: root) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: RejectingDeletionRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let index = try XCTUnwrap(model.conversations.firstIndex { $0.id == conversationID })
        model.conversations[index].isPersisted = true
        model.deleteConversation(model.conversations[index])
        for _ in 0..<200 where model.deletingConversationIDs.contains(conversationID) {
            await Task.yield()
        }
        XCTAssertTrue(model.conversations.contains { $0.id == conversationID })
        XCTAssertNotNil(model.conversationOperationError)
        XCTAssertFalse(model.deletingConversationIDs.contains(conversationID))
    }

    func testPendingDeletionBlocksSubmissionWithoutConsumingDraft() async throws {
        let conversationID = "delete-submit-\(UUID().uuidString)"
        let (root, archiveStore) = makeIsolatedArchiveStore("delete-submit")
        defer { try? FileManager.default.removeItem(at: root) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: RejectingDeletionRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        let index = try XCTUnwrap(model.conversations.firstIndex { $0.id == conversationID })
        model.conversations[index].isPersisted = true
        model.draft = "Keep this unsent message"
        model.deleteConversation(model.conversations[index])
        XCTAssertTrue(model.isSelectedConversationDeleting)
        let state = MacComposerDisplayState(
            conversationID: conversationID, draft: model.draft, attachments: [],
            isRunning: false, isVisiblyRunning: false,
            isSubmissionBlocked: model.isSelectedConversationDeleting
        )
        XCTAssertFalse(state.canSend)
        XCTAssertTrue(state.isPrimaryActionDisabled)
        model.send()
        XCTAssertEqual(model.draft, "Keep this unsent message")
        XCTAssertFalse(model.isRunning)
        for _ in 0..<200 where model.deletingConversationIDs.contains(conversationID) {
            await Task.yield()
        }
        XCTAssertFalse(model.isSelectedConversationDeleting)
        XCTAssertEqual(model.draft, "Keep this unsent message")
        XCTAssertNotNil(model.conversationOperationError)
    }

    func testThinkingStreamSeparatesReasoningAndPacesBurstResponse() {
        var stream = MacThinkingStreamAccumulator()

        let thinking = stream.append("<think>分析问题与约束")
        XCTAssertEqual(thinking.response, "")
        XCTAssertEqual(thinking.thinking, "分析问题与约束")

        let answer = stream.append("</think>第一段结论已经完成。第二段结论也已经完成。最后给出行动建议。")
        XCTAssertFalse(answer.response.contains("分析问题与约束"))
        XCTAssertEqual(answer.response, answer.responseDelta)

        let segments = MacPacedStreamSegmenter.segments(answer.response, minimumLength: 8, maximumLength: 18)
        XCTAssertGreaterThan(segments.count, 1)
        XCTAssertEqual(segments.joined(), answer.response)

        let currentStep = MacThinkingActivityFormatter.currentStep(
            from: "先理解仓库结构。然后检查上下文存储。现在定位会话切换时的刷新路径。",
            maximumLength: 24
        )
        XCTAssertTrue(currentStep.contains("会话切换"))
        XCTAssertFalse(currentStep.contains("先理解仓库结构"))

        let latestUnpunctuatedStep = MacThinkingActivityFormatter.currentStep(
            from: "先加载技能。已经读完源代码。现在生成结构化 Word 文档并验证输出",
            maximumLength: 80
        )
        XCTAssertEqual(latestUnpunctuatedStep, "现在生成结构化 Word 文档并验证输出")
    }

    func testBrowserToolResultProjectsCurrentPageAndTabs() throws {
        let result = try JSONDecoder().decode(
            MacAgentTraceJSONValue.self,
            from: Data(#"{"output_preview":"{\"ok\":true,\"target\":\"managed\",\"session_id\":\"nola-chat-1\",\"pages\":[{\"index\":0,\"url\":\"https://example.com/first\",\"title\":\"First\",\"active\":false},{\"index\":1,\"url\":\"https://example.com/current\",\"title\":\"Current page\",\"active\":true}]}"}"#.utf8)
        )
        let snapshot = MacConversationBrowserSnapshot.reducing(
            toolName: "browser_tabs",
            arguments: nil,
            result: result,
            succeeded: true,
            previous: nil
        )

        XCTAssertEqual(snapshot?.sessionID, "nola-chat-1")
        XCTAssertEqual(snapshot?.pages.count, 2)
        XCTAssertEqual(snapshot?.activePage?.title, "Current page")
        XCTAssertEqual(snapshot?.currentURL, "https://example.com/current")
        XCTAssertTrue(snapshot?.isConnected == true)
    }

    func testBrowserToolExplicitlyEmptyTabsClearPreviouslyVisiblePage() throws {
        let opened = try JSONDecoder().decode(
            MacAgentTraceJSONValue.self,
            from: Data(#"{"running":true,"pages":[{"index":0,"url":"https://example.com/old","title":"Old","active":true}]}"#.utf8)
        )
        let previous = MacConversationBrowserSnapshot.reducing(
            toolName: "browser_tabs", arguments: nil, result: opened,
            succeeded: true, previous: nil
        )
        let closed = try JSONDecoder().decode(
            MacAgentTraceJSONValue.self,
            from: Data(#"{"running":true,"pages":[]}"#.utf8)
        )
        let snapshot = MacConversationBrowserSnapshot.reducing(
            toolName: "browser_tabs", arguments: nil, result: closed,
            succeeded: true, previous: previous
        )

        XCTAssertEqual(snapshot?.pages.count, 0)
        XCTAssertNil(snapshot?.currentURL)
        XCTAssertNil(snapshot?.currentTitle)
        XCTAssertTrue(snapshot?.isConnected == true)
    }

    func testBrowserToolCanonicalHostEnvelopeProjectsPreviewAndAddress() throws {
        let inner = #"{"display":"browser_managed_navigate: Nola","text":"Action: browser_managed_navigate","target":"managed","session_id":"nola-chat-2","preview_url":"http://127.0.0.1:4321/view?token=secret","url":"https://example.com/nola","title":"Nola"}"#
        let envelope = try JSONSerialization.data(withJSONObject: [
            "text": inner,
            "display": "browser_managed_navigate: Nola",
            "full_output_ref": NSNull(),
        ])
        let result = try JSONDecoder().decode(MacAgentTraceJSONValue.self, from: envelope)

        let snapshot = MacConversationBrowserSnapshot.reducing(
            toolName: "browser_navigate",
            arguments: nil,
            result: result,
            succeeded: true,
            previous: nil
        )

        XCTAssertEqual(snapshot?.currentURL, "https://example.com/nola")
        XCTAssertEqual(snapshot?.activePage?.title, "Nola")
        XCTAssertEqual(snapshot?.previewURL, "http://127.0.0.1:4321/view?token=secret")
    }

    func testTruncatedBrowserToolResultRetainsPreviewAndAddress() throws {
        let nested = #"{"_nola_control":{"target":"managed","session_id":"nola-chat-3","preview_url":"http://127.0.0.1:4321/view?token=secret","final_url":"https://example.com/current","title":"Current page","pages":[{"index":0,"title":"Current page","url":"https://example.com/current","active":true}]},"truncated":true}"#
        let result = try JSONDecoder().decode(
            MacAgentTraceJSONValue.self,
            from: JSONSerialization.data(withJSONObject: [
                "text": nested,
                "display": "browser_managed_navigate: Current page",
            ])
        )
        let snapshot = MacConversationBrowserSnapshot.reducing(
            toolName: "browser_navigate",
            arguments: nil,
            result: result,
            succeeded: true,
            previous: nil
        )

        XCTAssertEqual(snapshot?.sessionID, "nola-chat-3")
        XCTAssertEqual(snapshot?.currentURL, "https://example.com/current")
        XCTAssertEqual(snapshot?.activePage?.title, "Current page")
        XCTAssertEqual(snapshot?.previewURL, "http://127.0.0.1:4321/view?token=secret")
    }

    func testCanonicalCoreToolPartProjectsBrowserPreviewIntoConversation() throws {
        let nested = #"{"_nola_control":{"target":"managed","session_id":"nola-core-part","preview_url":"http://127.0.0.1:63851/view?session_id=nola-core-part&token=secret","url":"https://www.bbc.com/","title":"BBC Home"},"truncated":true}"#
        let structuredOutput = try JSONDecoder().decode(
            MacAgentTraceJSONValue.self,
            from: JSONSerialization.data(withJSONObject: [
                "display": "browser_navigate: BBC Home",
                "text": nested,
            ])
        )

        let snapshot = MacConversationBrowserSnapshot.reducingCoreTool(
            toolName: "browser_navigate",
            state: "completed",
            arguments: .object(["url": .string("https://www.bbc.com")]),
            structuredOutput: structuredOutput,
            outputPreview: nested,
            previous: nil
        )

        XCTAssertEqual(snapshot?.sessionID, "nola-core-part")
        XCTAssertEqual(snapshot?.currentURL, "https://www.bbc.com/")
        XCTAssertEqual(snapshot?.currentTitle, "BBC Home")
        XCTAssertEqual(
            snapshot?.previewURL,
            "http://127.0.0.1:63851/view?session_id=nola-core-part&token=secret"
        )
    }

    func testConversationSwitchRefreshesEachContextIndependently() async throws {
        let first = MacConversation(title: "First", preview: "", updatedAt: .now, isPinned: false)
        let second = MacConversation(title: "Second", preview: "", updatedAt: .now, isPinned: false)
        let (root, archiveStore) = makeIsolatedArchiveStore("context-switch")
        defer { try? FileManager.default.removeItem(at: root) }
        try archiveStore.restoreBackupSnapshot([
            MacStoredConversation(
                conversation: first,
                messages: [MacChatMessage(role: .user, text: "短消息", createdAt: .now)]
            ),
            MacStoredConversation(
                conversation: second,
                messages: [MacChatMessage(
                    role: .user,
                    text: String(repeating: "较长的会话内容。", count: 500),
                    createdAt: .now
                )]
            ),
        ])
        let model = MacWorkspaceModel(canonicalRepository: archiveStore, localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())

        model.selectedConversationID = first.id
        await model.loadSelectedConversationIfNeeded()
        let firstUsage = model.contextUsed

        model.selectedConversationID = second.id
        await model.loadSelectedConversationIfNeeded()
        let secondUsage = model.contextUsed

        XCTAssertEqual(firstUsage, Int(try archiveStore.conversationContextUsage(conversationID: first.id).usedTokens))
        XCTAssertEqual(secondUsage, Int(try archiveStore.conversationContextUsage(conversationID: second.id).usedTokens))
        XCTAssertFalse(model.contextUsageIsAvailable, "An imported transcript has no measured model context yet.")
        XCTAssertEqual(model.contextUsageText, String(localized: "Context not measured"))
    }

    func testSupersededColdConversationLoadCannotOccupyNewDraftLoadingState() async throws {
        let archived = MacConversation(title: "Archived", preview: "", updatedAt: .now, isPinned: false)
        let (root, repository) = makeIsolatedArchiveStore("selection-load-lease")
        defer { try? FileManager.default.removeItem(at: root) }
        try repository.restoreBackupSnapshot([
            MacStoredConversation(conversation: archived,
                messages: [MacChatMessage(role: .user, text: "Archived content", createdAt: .now)])
        ])
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        defer { model.presentationDidDetach() }
        model.selectedConversationID = archived.id
        let staleLoad = Task { @MainActor in await model.loadSelectedConversationIfNeeded() }
        await Task.yield()
        model.newConversation()
        let draftID = try XCTUnwrap(model.selectedConversationID)
        model.draft = "Keep this new draft"
        await model.loadSelectedConversationIfNeeded()
        await staleLoad.value
        XCTAssertEqual(model.selectedConversationID, draftID)
        XCTAssertFalse(model.isSelectedConversationLoading)
        XCTAssertTrue(model.selectedMessages.isEmpty)
        XCTAssertEqual(model.draft, "Keep this new draft")
    }

    func testCoreContextResponseDecodesPromptLimitAndArchiveRequest() throws {
        let json = #"""
        {
          "schema_version":2,
          "action":"summary_required",
          "messages":[],
          "tools":[],
          "payload_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "changed":true,
          "reason":"archive_required",
          "metrics":{
            "message_tokens":7000,
            "tool_schema_tokens":1000,
            "total_prompt_tokens":8000,
            "prompt_token_limit":12000,
            "reserved_output_tokens":4096,
            "usage_percent":66.6,
            "before_tokens":9000,
            "after_tokens":8000,
            "trigger_tokens":10800,
            "target_tokens":8400,
            "archive_message_count":2,
            "transaction_count":1
          },
          "component_metrics":[],
          "archive_request":{
            "archive_id":"archive-1",
            "archive_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "message_indices":[0,1],
            "message_ids":["m1","m2"],
            "messages":[
              {"id":"m1","role":"user","content":"first"},
              {"id":"m2","role":"assistant","content":"second"}
            ],
            "target_summary_tokens":1024
          },
          "future_field":true
        }
        """#

        let response = try XCTUnwrap(MacContextCore.decodeResponse(Data(json.utf8)))
        XCTAssertEqual(response.schemaVersion, 2)
        XCTAssertEqual(response.payloadDigest, String(repeating: "a", count: 64))
        XCTAssertEqual(response.metrics.promptTokenLimit, 12_000)
        XCTAssertEqual(response.metrics.totalPromptTokens, 8_000)
        XCTAssertEqual(response.archiveRequest?.archiveId, "archive-1")
        XCTAssertEqual(response.archiveRequest?.messageIds, ["m1", "m2"])
        XCTAssertEqual(response.archiveRequest?.messages.map(\.content), ["first", "second"])
    }

    func testWorkspaceUsesAuthoritativeContextSnapshotWithoutLocalRecount() {
        let conversationID = "context-\(UUID().uuidString)"
        let (root, archiveStore) = makeIsolatedArchiveStore("context-authoritative")
        defer { try? FileManager.default.removeItem(at: root) }
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = conversationID
        model.applyContextSnapshot(
            .init(
                usedTokens: 9_000,
                contextWindowTokens: 16_384,
                promptTokenLimit: 12_000,
                isEstimated: false
            ),
            conversationID: conversationID
        )

        XCTAssertEqual(model.contextUsed, 9_000)
        XCTAssertEqual(model.contextWindowTokens, 16_384)
        XCTAssertEqual(model.contextPromptLimit, 12_000)
        XCTAssertFalse(model.contextUsageIsEstimated)
        model.applyContextSnapshot(
            .init(usedTokens: 0, contextWindowTokens: 16_384,
                  promptTokenLimit: 12_000, isEstimated: false),
            conversationID: conversationID
        )
        XCTAssertEqual(model.contextUsed, 0, "A valid zero usage must not be rejected by its value.")
        XCTAssertTrue(model.contextUsageIsAvailable, "A zero numerator with a valid model budget is measured, not missing.")
    }

    private func makeIsolatedArchiveStore(
        _ label: String
    ) -> (URL, MacCanonicalConversationRepository) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-\(label)-\(UUID().uuidString)", isDirectory: true)
        return (root, MacCanonicalConversationRepository(store: try! MacApplicationPort(databaseURL: root.appendingPathComponent("nola.sqlite"))))
    }

    func testAgentContextUsagePayloadSeparatesWindowAndPromptLimit() throws {
        let payload = try JSONDecoder().decode(MacAgentTraceUsage.self, from: Data(
            #"{"used_tokens":9000,"context_window_tokens":16384,"prompt_token_limit":12288}"#.utf8
        ))
        XCTAssertEqual(payload.usedTokens, 9_000)
        XCTAssertEqual(payload.contextWindowTokens, 16_384)
        XCTAssertEqual(payload.promptTokenLimit, 12_288)
    }

    func testAuthoritativeContextSnapshotPersistsAcrossWorkspaceReload() async throws {
        let conversationID = "context-roundtrip-\(UUID().uuidString)"
        let baseDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-context-persistence-\(UUID().uuidString)", isDirectory: true)
        let applicationPort = try MacApplicationPort(databaseURL: baseDirectory.appendingPathComponent("nola.sqlite"))
        let archiveStore = MacCanonicalConversationRepository(store: applicationPort)
        defer {
            try? FileManager.default.removeItem(at: baseDirectory)
        }
        _ = try commitScriptedCanonicalTurn(
            applicationPort: applicationPort,
            conversationID: conversationID,
            title: "Context roundtrip",
            userText: "Measure context",
            answer: "Measured",
            contextUsage: .init(used: 7_500, window: 32_768, promptLimit: 28_672)
        )
        let model = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        model.selectedConversationID = conversationID
        let restored = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: archiveStore,
            localAgentRuntime: PersistenceTestLocalRuntime(),
            remoteHost: PersistenceTestRemoteHost()
        )
        restored.selectedConversationID = conversationID
        await restored.loadSelectedConversationIfNeeded()

        XCTAssertEqual(restored.contextUsed, 7_500)
        XCTAssertEqual(restored.contextWindowTokens, 32_768)
        XCTAssertEqual(restored.contextPromptLimit, 28_672)
        XCTAssertFalse(restored.contextUsageIsEstimated)
    }

    func testBackupCodecRoundTripsWithoutTouchingApplicationStores() throws {
        let conversation = MacBackupConversation(MacCanonicalConversation(
                id: "codec-conversation",
                title: "Codec conversation",
                revision: 1,
                createdAtMs: 1,
                updatedAtMs: 1,
                settingsJSON: nil,
                messages: [MacCanonicalConversationMessage(
                    id: "codec-message", runID: nil, role: "user", status: "completed",
                    ordinal: 1, createdAtMs: 1, completedAtMs: 1,
                    parts: [MacCanonicalConversationPart(
                        id: "codec-text", kind: "text", ordinal: 1, state: "completed",
                        contentText: "Round trip", contentJSON: nil, createdAtMs: 1, updatedAtMs: 1
                    )]
                )]
        ))
        let backup = MacDataBackup(
            format: MacDataBackupCodec.format,
            version: MacDataBackupCodec.version,
            exportedAt: .now,
            conversations: [conversation],
            activity: [],
            automations: [],
            settings: [],
            productSettings: [MacCanonicalProductSetting(
                key: MacProductSettingKeys.memoryEnabled,
                valueJSON: #"{"schemaVersion":1,"enabled":true}"#,
                revision: 2,
                updatedAtMs: 200
            )],
            webSources: [],
            notifications: []
        )

        let restored = try MacDataBackupCodec.decode(MacDataBackupCodec.encode(backup))

        XCTAssertEqual(restored.conversations.map(\.canonical.id), ["codec-conversation"])
        XCTAssertEqual(restored.conversations.first?.canonical, conversation.canonical)
        XCTAssertEqual(restored.conversations.first?.canonical.messages.first?.parts.first?.contentText, "Round trip")
        XCTAssertEqual(restored.productSettings.first?.key, MacProductSettingKeys.memoryEnabled)
        XCTAssertEqual(restored.productSettings.first?.revision, 2)
    }

    func testContextSummaryPromptUsesOnlyCoreSelectedArchiveMessages() {
        let request = MacCoreContextArchiveRequest(
            archiveId: "archive-1",
            archiveDigest: String(repeating: "b", count: 64),
            messageIndices: [1, 2],
            messageIds: ["m1", "m2"],
            messages: [
                MacCoreContextMessage(id: "m1", role: "user", content: "first selected fact"),
                MacCoreContextMessage(id: "m2", role: "assistant", content: "second selected decision")
            ],
            existingSummary: "previous continuation state",
            targetSummaryTokens: 512
        )

        let prompt = MacContextCore.summaryPrompt(for: request)

        XCTAssertTrue(prompt.contains("previous continuation state"))
        XCTAssertTrue(prompt.contains("first selected fact"))
        XCTAssertTrue(prompt.contains("second selected decision"))
        XCTAssertTrue(prompt.contains("512 tokens"))
    }
}

@MainActor
private final class MacStreamingPresentationHarnessState: ObservableObject {
    @Published var nodes: [MacAssistantTextNode] = []
    @Published var mutableTail: MacAssistantTextNode?
    @Published var animateNewNodes = false
}

private struct MacStreamingPresentationHarness: View {
    @ObservedObject var state: MacStreamingPresentationHarnessState

    var body: some View {
        MacAssistantPresentationView(
            nodes: state.nodes + (state.mutableTail.map { [$0] } ?? []),
            animateNewNodes: state.animateNewNodes,
            contentIsFinal: false
        )
        .frame(width: 520, alignment: .leading)
        .padding(20)
        .background(Color(nsColor: .windowBackgroundColor))
        .preferredColorScheme(.dark)
    }
}

@MainActor
private final class MacRunStatusHarnessState: ObservableObject {
    @Published var activities: [MacAgentActivity] = []
    @Published var traces: [MacAssistantTrace] = []
}

private struct MacRunStatusHarness: View {
    @ObservedObject var state: MacRunStatusHarnessState

    var body: some View {
        MacThinkingProcessRow(
            traces: state.traces,
            isStreaming: true,
            initiallyExpanded: false
        )
        .frame(width: 480, alignment: .leading)
        .padding(20)
    }
}

@MainActor
private final class MacAssistantBubbleHarnessState: ObservableObject {
    @Published var message = MacChatMessage(role: .assistant, text: "", createdAt: .now)
    @Published var activities: [MacAgentActivity] = []
    @Published var isStreaming = false
    @Published var colorScheme: ColorScheme = .dark
}

private struct MacAssistantBubbleHarness: View {
    @ObservedObject var state: MacAssistantBubbleHarnessState

    var body: some View {
        MacMessageView(
            message: state.message,
            transcriptWidth: 740,
            isStreaming: state.isStreaming,
            liveActivities: state.activities,
            isSearchMatch: false,
            onSendTaskGuidance: { _, _ in },
            onRequestTaskCancellation: { _ in },
            showsAnimatedStatusLight: false
        )
        .frame(width: 740, alignment: .leading)
        .padding(20)
        .background(state.colorScheme == .dark ? Color(red: 0.105, green: 0.105, blue: 0.115) : Color.white)
        .environment(\.colorScheme, state.colorScheme)
    }
}

private struct MacComposerOcclusionHarness: View {
    let state: MacComposerDisplayState
    private let actions = MacComposerActionSink { _ in true }
    @FocusState private var focused: Bool

    var body: some View {
        ZStack {
            LinearGradient(
                colors: [.red, .blue],
                startPoint: .leading,
                endPoint: .trailing
            )
            MacComposerView(state: state, actions: actions, composerFocused: $focused)
                .frame(width: 700)
                .padding(20)
        }
        .frame(width: 780, height: 180)
        .preferredColorScheme(.dark)
    }
}

@MainActor
private func settle(_ host: NSView, milliseconds: Int = 45) async {
    host.layoutSubtreeIfNeeded()
    await Task.yield()
    host.layoutSubtreeIfNeeded()
    try? await Task.sleep(for: .milliseconds(milliseconds))
    host.layoutSubtreeIfNeeded()
    await Task.yield()
    host.layoutSubtreeIfNeeded()
}

@MainActor
private func visualWindow(
    host: NSView,
    size: CGSize,
    appearance: NSAppearance.Name
) -> NSWindow {
    let window = NSWindow(
        contentRect: NSRect(origin: .zero, size: size),
        styleMask: [.borderless],
        backing: .buffered,
        defer: false
    )
    window.appearance = NSAppearance(named: appearance)
    window.isReleasedWhenClosed = false
    window.contentView = host
    window.orderFrontRegardless()
    return window
}

@MainActor
private func bitmapSignature(_ host: NSHostingView<MacStreamingPresentationHarness>) throws -> Int {
    let data = try bitmapPNG(host)
    return data.reduce(into: 5381) { value, byte in
        value = ((value << 5) &+ value) &+ Int(byte)
    }
}

@MainActor
private func bitmapRepresentation(
    _ host: NSView,
    width: CGFloat = 560,
    height: CGFloat? = nil
) throws -> NSBitmapImageRep {
    if let height {
        host.frame.size = CGSize(width: width, height: max(1, height))
    } else {
        let fittingSize = host.fittingSize
        host.frame.size = CGSize(width: width, height: max(1, fittingSize.height))
    }
    host.layoutSubtreeIfNeeded()
    host.displayIfNeeded()
    host.window?.displayIfNeeded()
    if let warmup = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
        host.cacheDisplay(in: host.bounds, to: warmup)
    }
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.016))
    host.layoutSubtreeIfNeeded()
    host.displayIfNeeded()
    let representation = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
    host.cacheDisplay(in: host.bounds, to: representation)
    return representation
}

@MainActor
private func bitmapPNG(_ host: NSView, width: CGFloat = 560, height: CGFloat? = nil) throws -> Data {
    try bitmapPNG(bitmapRepresentation(host, width: width, height: height))
}

private func bitmapPNG(_ representation: NSBitmapImageRep) throws -> Data {
    let encoded = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
    // NSBitmapImageRep may vend Data backed by its mutable caching-display
    // storage. XCTAttachment persists asynchronously, so retain an owned byte
    // copy before AppKit reuses that storage for the next frame.
    return encoded.withUnsafeBytes { bytes in
        guard let baseAddress = bytes.baseAddress, !bytes.isEmpty else { return Data() }
        return Data(bytes: baseAddress, count: bytes.count)
    }
}

@MainActor
private func attachSnapshot(_ host: NSView, name: String, width: CGFloat, height: CGFloat? = nil) throws {
    try attachSnapshot(try bitmapPNG(host, width: width, height: height), name: name)
}

private func attachSnapshot(_ data: Data, name: String) throws {
    let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
    attachment.name = name
    attachment.lifetime = .keepAlways
    XCTContext.runActivity(named: name) { activity in
        activity.add(attachment)
    }
}

private func assertDarkCompleteAnswerIsVisible(
    _ representation: NSBitmapImageRep,
    viewSize: CGSize,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    // These point-space regions deliberately target the body of the same
    // dark completion frame: the heading, table cells and code source. A
    // status-only frame has no bright foreground pixels in any of them.
    let regions: [(name: String, rect: CGRect, minimumInk: Int)] = [
        ("answer heading", CGRect(x: 50, y: 145, width: 620, height: 48), 80),
        ("table", CGRect(x: 50, y: 205, width: 560, height: 100), 100),
        (
            "code source",
            CGRect(x: 50, y: max(0, viewSize.height - 225), width: 650, height: 100),
            70
        ),
    ]

    for region in regions {
        XCTAssertGreaterThan(
            brightPixelCount(representation, inTopOriginPointRect: region.rect, viewSize: viewSize),
            region.minimumInk,
            "Dark completion frame is missing visible \(region.name)",
            file: file,
            line: line
        )
    }
}

@MainActor
private final class MacJournalReparentHarnessState: ObservableObject {
    @Published var inHistory = false
    let reference: MacJournalMessageReference
    let owner: MacJournalPresentationOwner
    init(reference: MacJournalMessageReference, owner: MacJournalPresentationOwner) {
        self.reference = reference
        self.owner = owner
    }
}

private struct MacJournalReparentHarness: View {
    @ObservedObject var state: MacJournalReparentHarnessState
    var body: some View {
        VStack {
            if state.inHistory {
                LazyVStack { message }
            } else { message }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
    private var message: some View {
        MacJournalMessageTextView(reference: state.reference, presentationOwner: state.owner)
    }
}

private func brightPixelCount(
    _ representation: NSBitmapImageRep,
    inTopOriginPointRect rect: CGRect,
    viewSize: CGSize
) -> Int {
    guard viewSize.width > 0, viewSize.height > 0 else { return 0 }
    let scaleX = CGFloat(representation.pixelsWide) / viewSize.width
    let scaleY = CGFloat(representation.pixelsHigh) / viewSize.height
    let minX = max(0, Int((rect.minX * scaleX).rounded(.down)))
    let maxX = min(representation.pixelsWide, Int((rect.maxX * scaleX).rounded(.up)))
    let minY = max(0, representation.pixelsHigh - Int((rect.maxY * scaleY).rounded(.up)))
    let maxY = min(representation.pixelsHigh, representation.pixelsHigh - Int((rect.minY * scaleY).rounded(.down)))
    guard minX < maxX, minY < maxY else { return 0 }

    var count = 0
    for y in minY..<maxY {
        for x in minX..<maxX {
            guard let color = representation.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
            let luminance = (0.2126 * color.redComponent)
                + (0.7152 * color.greenComponent)
                + (0.0722 * color.blueComponent)
            if color.alphaComponent > 0.5, luminance > 0.48 {
                count += 1
            }
        }
    }
    return count
}
