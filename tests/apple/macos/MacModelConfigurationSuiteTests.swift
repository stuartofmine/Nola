// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS model center editing, connections, import bridge, reasoning configuration, local/host model selection, SIWC account/profile admission, and attempt presentation suite.

import AppKit
import Combine
import Foundation
import SwiftUI
import XCTest
@testable import Nola

final class MacLocalModelEndpointAndAttemptPresentationTests: XCTestCase {
    private enum EndpointTestError: Error { case unavailable }

    private func attemptPresentation(_ state: String, successor: String? = nil,
        provider: String? = "provider-a", model: String? = "model-a",
        nextProvider: String? = "provider-a", nextModel: String? = "model-a") -> AppleModelAttemptPresentation {
        .init(state: state, provider: provider, model: model, successorAttemptID: successor,
            successorProvider: nextProvider, successorModel: nextModel)
    }

    func testLocalModelEndpointIdentityCloudIsolationAndCapturedInstanceValidation() throws {
        let model = MacLocalModelProfile.qwen35_08B.id
        let original = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345/v1",
            apiKey: "old-token", modelID: model, contextWindowTokens: 65536, profileRevision: 7)
        let fresh = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:54321/v1",
            apiKey: "fresh-token", modelID: model, contextWindowTokens: 65536, profileRevision: 7)
        let route = try original.nativeRoute()
        XCTAssertTrue(NSDictionary(dictionary: route).isEqual(to: try fresh.nativeRoute()))
        let text = String(decoding: try JSONSerialization.data(withJSONObject: route, options: [.sortedKeys]), as: UTF8.self)
        XCTAssertFalse(text.contains("token"))
        XCTAssertFalse(text.contains("127.0.0.1"))
        for replacement in [
            MacMLXOpenAIService.Endpoint(baseURL: fresh.baseURL, apiKey: fresh.apiKey, modelID: "different-model", contextWindowTokens: 65536, profileRevision: 7),
            MacMLXOpenAIService.Endpoint(baseURL: fresh.baseURL, apiKey: fresh.apiKey, modelID: model, contextWindowTokens: 65536, profileRevision: 8)
        ] {
            XCTAssertFalse(NSDictionary(dictionary: route).isEqual(to: try replacement.nativeRoute()))
        }
        XCTAssertThrowsError(try MacMLXOpenAIService.Endpoint(baseURL: fresh.baseURL, apiKey: fresh.apiKey, modelID: model, contextWindowTokens: 65536).nativeRoute())

        for modelID in [nil, "cloud-model"] as [String?] {
            XCTAssertNil(try MacLocalModelEndpointSelection.resolve(modelID: modelID) { _ in
                XCTFail("Remote route must not consult the local model service")
                throw EndpointTestError.unavailable
            })
        }
        XCTAssertThrowsError(try MacLocalModelEndpointSelection.resolve(modelID: model) { _ in throw EndpointTestError.unavailable }) { error in
            XCTAssertTrue(error is EndpointTestError)
        }

        var calls = 0
        let resolved = try XCTUnwrap(MacLocalModelEndpointSelection.resolve(modelID: model) { profile in
            calls += 1
            return MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345", apiKey: "test-instance-token", modelID: profile.id, contextWindowTokens: 32768)
        })
        XCTAssertEqual(calls, 1)
        XCTAssertEqual(resolved.baseURL, "http://127.0.0.1:12345")
        XCTAssertEqual(resolved.apiKey, "test-instance-token")
        XCTAssertEqual(resolved.contextWindowTokens, 32768)

        let captured = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345", apiKey: "original", modelID: model, contextWindowTokens: 32768)
        for replacement in [
            MacMLXOpenAIService.Endpoint(baseURL: captured.baseURL, apiKey: "replacement", modelID: model, contextWindowTokens: 32768),
            MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:54321", apiKey: captured.apiKey, modelID: model, contextWindowTokens: 32768),
            MacMLXOpenAIService.Endpoint(baseURL: captured.baseURL, apiKey: captured.apiKey, modelID: model, contextWindowTokens: 65536),
        ] {
            XCTAssertThrowsError(try MacLocalModelEndpointSelection.resolve(modelID: model, captured: captured) { _ in replacement }) { error in
                XCTAssertTrue(error is MacLocalModelEndpointSelection.SelectionError)
            }
        }
        XCTAssertEqual(try MacLocalModelEndpointSelection.resolve(modelID: model, captured: captured) { _ in captured }, captured)
        XCTAssertThrowsError(try MacLocalModelEndpointSelection.resolve(modelID: "cloud-model", captured: captured) { _ in
            XCTFail("A mismatched route must not consult the runtime")
            return captured
        })
    }

    func testModelAttemptPresentationSuccessionFailureAndDisplayName() {
        for state in ["failed", "cancelled", "completed", "running"] {
            let item = attemptPresentation(state, successor: "next-attempt")
            XCTAssertEqual(item.kind, .restarted)
            XCTAssertFalse(item.isFailure)
            XCTAssertEqual(item.modelName, "model-a")
            XCTAssertNil(item.successorModelName)
        }
        XCTAssertEqual(attemptPresentation("failed", successor: "next", provider: nil, nextProvider: "other").kind, .restarted)
        XCTAssertEqual(attemptPresentation("failed", successor: "next", model: nil, nextModel: "other").kind, .restarted)
        XCTAssertEqual(attemptPresentation("failed", successor: "next", nextProvider: "provider-b").kind, .routeChanged)
        XCTAssertEqual(attemptPresentation("failed", successor: "next", nextModel: "model-b").successorModelName, "model-b")
        XCTAssertEqual(attemptPresentation("failed", successor: "next", nextModel: "model-b").kind, .routeChanged)

        for (state, kind) in [("pending", AppleModelAttemptPresentation.Kind.pending), ("running", .running),
            ("waiting_for_permission", .waiting), ("waiting_for_input", .waiting), ("completed", .completed),
            ("sealed", .completed), ("cancelled", .cancelled), ("failed", .failed), ("unknown", .recorded)] {
            let item = attemptPresentation(state, nextProvider: "provider-b", nextModel: "model-b")
            XCTAssertEqual(item.kind, kind)
            XCTAssertEqual(item.isFailure, state == "failed")
            XCTAssertFalse(item.hasSuccessor)
            XCTAssertNil(item.successorModelName)
        }

        let localItem = attemptPresentation("failed", successor: "recovered", provider: "device_local", model: "qwen",
            nextProvider: "device_local", nextModel: "qwen")
        XCTAssertEqual(localItem.modelName, "qwen")
        XCTAssertEqual(localItem.kind, .restarted)
        XCTAssertFalse(localItem.modelName.contains("device_local"))
        XCTAssertEqual(attemptPresentation("running", provider: "opaque-provider", model: nil).modelName, "—")
    }
}

@MainActor
final class MacModelCenterEditingTests: XCTestCase {
    func testModelSourceSwitchPreservesConnectionSearch() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("model-source-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let application = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.sqlite"))
        let navigation = MacModelCenterNavigationState(selectedTab: .api)
        let preference = MacDefaultChatModelPresentation(store: .init(application: application))
        let hosting = NSHostingView(rootView: MacModelCenterView(remoteHost: PersistenceTestRemoteHost(),
            navigation: navigation, preference: preference).environment(\.accessibilityEnabled, true)
                .environment(\.colorScheme, .light))
        hosting.frame = NSRect(x: 0, y: 0, width: 800, height: 760)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting)
        let search = try XCTUnwrap(elements(hosting).compactMap { $0 as? NSTextField }.first {
            $0.placeholderString == String(localized: "Search connections", table: "ModelCenter")
        })
        search.selectText(nil)
        let editor = try XCTUnwrap(search.currentEditor() as? NSTextView)
        editor.insertText("draft connection search", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        window.makeFirstResponder(hosting)
        try await settle(hosting)
        navigation.showLocalModels()
        try await settle(hosting)
        navigation.showRemoteModels()
        try await settle(hosting)
        let restored = try XCTUnwrap(elements(hosting).compactMap { $0 as? NSTextField }.first {
            $0.placeholderString == String(localized: "Search connections", table: "ModelCenter")
        })
        XCTAssertEqual(restored.stringValue, "draft connection search")
        for width: CGFloat in [680, 1040] {
            for scheme: ColorScheme in [.light, .dark] {
                hosting.rootView = MacModelCenterView(remoteHost: PersistenceTestRemoteHost(),
                    navigation: navigation, preference: preference).environment(\.accessibilityEnabled, true)
                    .environment(\.colorScheme, scheme)
                window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                window.setContentSize(NSSize(width: width, height: 760))
                hosting.frame = NSRect(x: 0, y: 0, width: width, height: 760)
                try await settle(hosting)
                let search = try XCTUnwrap(elements(hosting).compactMap { $0 as? NSTextField }.first {
                    $0.placeholderString == String(localized: "Search connections", table: "ModelCenter")
                })
                XCTAssertEqual(search.stringValue, "draft connection search")
                let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
                hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
                let image = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])),
                    uniformTypeIdentifier: "public.png")
                image.name = "model-source-retained-\(Int(width))-\(scheme == .dark ? "dark" : "light")"
                image.lifetime = .keepAlways
                add(image)
            }
        }
    }

    func testParameterDraftReturnsToCleanWhenRestoredToBaseline() {
        let parameters = ModelParameters.webAlignedCloudDefaults
        var draft = MacModelParameterDraft(parameters: parameters)
        draft.update("invalid", field: "temperature", decimal: \.temperature, range: 0...2)
        XCTAssertTrue(draft.isEdited)
        draft.update(String(parameters.temperature), field: "temperature", decimal: \.temperature, range: 0...2)
        XCTAssertFalse(draft.isEdited)
        draft.parameters.showThinking.toggle()
        XCTAssertTrue(draft.isEdited)
        draft.parameters.showThinking.toggle()
        XCTAssertFalse(draft.isEdited)
    }

    func testConflictNoticeBelongsToRetainedEditingSession() {
        let state = MacRemoteModelEditingState()
        state.needsReload = true
        state.notice = MacRemoteModelManagerNotice(title: "Conflict", message: "Reload", kind: .warning, offersReload: true)
        state.navigate(to: .connection("connection:a"))
        XCTAssertTrue(state.needsReload)
        XCTAssertEqual(state.notice?.title, "Conflict")
        XCTAssertTrue(state.notice?.offersReload == true)
    }

    func testRestoredParameterDraftClearsPendingWriteAndNavigationPrompt() {
        let state = MacRemoteModelEditingState()
        state.endpoints = fixtures()
        state.savedEndpoints = state.endpoints
        state.selectedEndpointID = "connection:a"
        let parameters = ModelParameters.webAlignedCloudDefaults
        var draft = MacModelParameterDraft(parameters: parameters)
        draft.parameters.showThinking.toggle()
        state.updateParameterDraft(draft, for: "profile:a")
        XCTAssertTrue(state.hasUnsavedChanges)
        draft.parameters.showThinking.toggle()
        state.updateParameterDraft(draft, for: "profile:a")
        XCTAssertNil(state.pendingModelParameters["profile:a"])
        XCTAssertFalse(state.hasUnsavedChanges)
        XCTAssertFalse(state.requestNavigation(to: .connection("connection:b")))
    }

    func testNewModelKeepsInitialParameterWriteEvenWithoutEdits() {
        let state = MacRemoteModelEditingState()
        let draft = MacModelParameterDraft(parameters: .webAlignedCloudDefaults)
        state.updateParameterDraft(draft, for: "new-model")
        XCTAssertNotNil(state.pendingModelParameters["new-model"])
        XCTAssertTrue(state.hasUnsavedChanges)
    }
    func testHostedLocalSourceRemountPreservesSearchInspectionAndCompactNavigation() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-local-ui-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let application = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.sqlite"))
        let fixture = LocalModelRemountFixture(application: application)
        let model = try XCTUnwrap(ModelDownloader.ModelType.allCases.first)
        let hosting = NSHostingView(rootView: LocalModelRemountRoot(fixture: fixture)
            .environment(\.accessibilityEnabled, true))
        hosting.frame = NSRect(x: 0, y: 0, width: 680, height: 1000)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting)
        let search = try XCTUnwrap(elements(hosting).compactMap { $0 as? NSTextField }.first {
            $0.placeholderString == String(localized: "Search models", table: "ModelCenter")
        })
        search.selectText(nil)
        let editor = try XCTUnwrap(search.currentEditor() as? NSTextView)
        editor.insertText(model.displayName, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        window.makeFirstResponder(hosting)
        try await settle(hosting)
        try await press("local-model-library-" + model.rawValue, in: hosting)
        XCTAssertEqual(fixture.editing.inspectedModelID, model.rawValue)
        XCTAssertTrue(fixture.editing.showsCompactDetail)
        fixture.showsLocal = false
        try await settle(hosting)
        fixture.showsLocal = true
        try await settle(hosting)
        XCTAssertEqual(fixture.editing.inspectedModelID, model.rawValue)
        XCTAssertTrue(fixture.editing.showsCompactDetail)
        try await press("local-model-back", in: hosting)
        let restored = try XCTUnwrap(elements(hosting).compactMap { $0 as? NSTextField }.first {
            $0.placeholderString == String(localized: "Search models", table: "ModelCenter")
        })
        XCTAssertEqual(restored.stringValue, model.displayName)
    }

    func testHostedSaveAndContinueCommitsParametersAndOpensExactModel() async throws {
        try await withHostedManager { host, state, application in
            try await self.typeOutput("4096", in: host)
            XCTAssertEqual(state.pendingModelParameters["profile:a"]?.maxOutputTokens, 4096)
            try await self.press("model-library-profile:b", in: host)
            XCTAssertEqual(state.pendingDestination, .model(connectionID: "connection:b", profileID: "profile:b"))
            try await self.press("model-navigation-save", title: String(localized: "Save and Continue", table: "ModelCenter"), in: host)
            XCTAssertEqual(state.selectedModelID, "profile:b")
            XCTAssertFalse(state.hasUnsavedChanges)
            XCTAssertEqual(try ModelParametersStore(applicationPort: application).parameterSnapshot(for: "profile:a").parameters.maxOutputTokens, 4096)
        }
    }

    func testHostedParameterConflictRetainsDraftThenDiscardReachesHealthyConnection() async throws {
        try await withHostedManager { host, state, application in
            try await self.typeOutput("4096", in: host)
            let parameters = ModelParametersStore(applicationPort: application)
            let original = try parameters.parameterSnapshot(for: "profile:a")
            var concurrent = original.parameters
            concurrent.maxOutputTokens = 8192
            _ = try parameters.saveParameters(concurrent, snapshot: original)
            try await self.press("model-library-profile:b", in: host)
            try await self.press("model-navigation-save", title: String(localized: "Save and Continue", table: "ModelCenter"), in: host)
            XCTAssertEqual(state.selectedModelID, "profile:a")
            XCTAssertEqual(state.pendingModelParameters["profile:a"]?.maxOutputTokens, 4096)
            XCTAssertEqual(state.pendingDestination, .model(connectionID: "connection:b", profileID: "profile:b"))
            XCTAssertTrue(state.needsReload)
            try await self.press("model-library-profile:b", in: host)
            try await self.press("model-navigation-discard", title: String(localized: "Discard Changes", table: "ModelCenter"), in: host)
            XCTAssertEqual(state.selectedModelID, "profile:b")
            XCTAssertFalse(state.needsReload)
            XCTAssertFalse(state.hasUnsavedChanges)
            XCTAssertEqual(try parameters.parameterSnapshot(for: "profile:a").parameters.maxOutputTokens, 8192)
        }
    }

    func testHostedStaleUneditedParametersReloadWholeConnectionRevision() async throws {
        try await withHostedManager { host, state, application in
            let parameters = ModelParametersStore(applicationPort: application)
            let original = try parameters.parameterSnapshot(for: "profile:b")
            var concurrent = original.parameters
            concurrent.maxOutputTokens = 7777
            _ = try parameters.saveParameters(concurrent, snapshot: original)
            try await self.press("model-library-profile:b", in: host)
            XCTAssertNotNil(state.parameterErrors["profile:b"])
            XCTAssertTrue(state.needsReload)
            try await self.press("model-parameter-reload", in: host)
            XCTAssertNil(state.parameterErrors["profile:b"])
            XCTAssertFalse(state.needsReload)
            XCTAssertEqual(state.parameterDrafts["profile:b"]?.parameters.maxOutputTokens, 7777)
            XCTAssertEqual(state.savedRevisions["profile:b"], try application.modelProfile("profile:b")?.revision)
        }
    }

    private func withHostedManager(_ operation: (NSView, MacRemoteModelEditingState, MacApplicationPort) async throws -> Void) async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-model-ui-\(UUID())")
        defer { try? FileManager.default.removeItem(at: directory) }
        let application = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.sqlite"))
        try MacAppleChatConfigurationStore(applicationPort: application).replaceEndpoints(fixtures())
        let state = MacRemoteModelEditingState()
        let hosting = NSHostingView(rootView: MacRemoteModelManagerView(editingState: state, application: application)
            .environment(\.accessibilityEnabled, true).transaction { $0.disablesAnimations = true })
        hosting.frame = NSRect(x: 0, y: 0, width: 1100, height: 1100)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting)
        try await operation(hosting, state, application)
    }

    private func typeOutput(_ text: String, in host: NSView) async throws {
        let field = try XCTUnwrap(elements(host).compactMap { $0 as? NSTextField }.first {
            $0.accessibilityIdentifier() == "model-parameter-output"
                || $0.placeholderString == String(localized: "model.parameters.max_output")
        }, diagnostics(host))
        field.selectText(nil)
        let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
        editor.insertText(text, replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        host.window?.makeFirstResponder(host)
        try await settle(host)
    }

    private func press(_ id: String, title: String? = nil, in host: NSView) async throws {
        let candidates = elements(host)
        let button = try XCTUnwrap(candidates.first { $0.accessibilityIdentifier?() == id }
            ?? candidates.first {
                guard let native = $0 as? NSButton, native.window != host.window else { return false }
                return title != nil && native.title == title
            }, diagnostics(host))
        // SwiftUI may invalidate the AX node synchronously while handling the
        // press and return false even though its action ran. Every caller below
        // asserts the resulting UI state and/or canonical database write.
        _ = button.accessibilityPerformPress?()
        try await settle(host)
    }

    private func settle(_ host: NSView) async throws {
        for _ in 0..<8 {
            try await Task.sleep(for: .milliseconds(25))
            host.layoutSubtreeIfNeeded()
        }
    }

    private func elements(_ host: NSView) -> [AnyObject] {
        var queue: [AnyObject] = [host]
        if let window = host.window { queue.append(window) }
        var visited: Set<ObjectIdentifier> = []
        var result: [AnyObject] = []
        var index = 0
        while index < queue.count, result.count < 6000 {
            let object = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children: [Any] = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
            if let window = object as? NSWindow {
                if let content = window.contentView { queue.append(content) }
                queue.append(contentsOf: window.sheets)
                queue.append(contentsOf: window.childWindows ?? [])
            }
        }
        return result
    }

    private func diagnostics(_ host: NSView) -> String {
        elements(host).map { "\(type(of: $0)) id=\($0.accessibilityIdentifier?() ?? "") label=\($0.accessibilityLabel?() ?? "")" }.joined(separator: "\n")
    }

    func testDirtyNavigationRetainsExactModelAcrossSaveFailureAndStay() {
        let state = MacRemoteModelEditingState()
        state.endpoints = fixtures()
        state.savedEndpoints = state.endpoints
        state.selectedEndpointID = "connection:a"
        state.selectedModelID = "profile:a"
        state.endpoints[0].name = "Edited"
        let destination = MacRemoteModelDestination.model(connectionID: "connection:b", profileID: "profile:b")
        XCTAssertTrue(state.requestNavigation(to: destination))
        XCTAssertEqual(state.pendingDestination, destination)
        XCTAssertEqual(state.selectedModelID, "profile:a")
        // A failed save does not mutate the owner. Keeping the draft also keeps
        // the concrete destination available for the next successful attempt.
        XCTAssertEqual(state.endpoints[0].name, "Edited")
        state.pendingDestination = nil
        XCTAssertEqual(state.selectedModelID, "profile:a")
        XCTAssertTrue(state.hasUnsavedChanges)
        XCTAssertTrue(state.requestNavigation(to: destination))
        state.endpoints = state.savedEndpoints
        state.navigate(to: destination)
        XCTAssertEqual(state.selectedEndpointID, "connection:b")
        XCTAssertEqual(state.selectedModelID, "profile:b")
        XCTAssertNil(state.pendingDestination)
    }

    func testSameConnectionNavigationKeepsSharedDraftAndInvalidParameterText() {
        let state = MacRemoteModelEditingState()
        state.endpoints = fixtures()
        state.savedEndpoints = state.endpoints
        state.selectedEndpointID = "connection:a"
        var draft = MacModelParameterDraft(parameters: .webAlignedCloudDefaults)
        draft.update("not a number", field: "temperature", decimal: \.temperature, range: 0...2)
        state.parameterDrafts["profile:a"] = draft
        let destination = MacRemoteModelDestination.connection("connection:a")
        XCTAssertFalse(state.requestNavigation(to: destination))
        state.navigate(to: destination)
        XCTAssertEqual(state.parameterDrafts["profile:a"]?.text["temperature"], "not a number")
        XCTAssertEqual(state.parameterDrafts["profile:a"]?.invalidFields, ["temperature"])
        XCTAssertTrue(state.hasUnsavedChanges)
    }

    func testDraftRejectsInvalidInputAndAcceptsCorrectionWithoutLosingOtherFields() {
        var draft = MacModelParameterDraft(parameters: .webAlignedCloudDefaults)
        let original = draft.parameters.maxOutputTokens
        draft.update("12x", field: "output", integer: \.maxOutputTokens, range: ModelParameterLimits.outputTokens)
        XCTAssertEqual(draft.parameters.maxOutputTokens, original)
        XCTAssertTrue(draft.invalidFields.contains("output"))
        draft.update("4096", field: "output", integer: \.maxOutputTokens, range: ModelParameterLimits.outputTokens)
        XCTAssertEqual(draft.parameters.maxOutputTokens, 4096)
        XCTAssertTrue(draft.invalidFields.isEmpty)
        XCTAssertEqual(draft.parameters.temperature, ModelParameters.webAlignedCloudDefaults.temperature)
    }

    func testPresentationUsesExplicitDefaultNotPriorityAndReportsUnavailable() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-model-center-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let application = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.sqlite"))
        let connections = MacAppleChatConfigurationStore(applicationPort: application)
        try connections.replaceEndpoints(fixtures())
        let presentation = MacDefaultChatModelPresentation(store: .init(application: application))
        XCTAssertFalse(presentation.isDefault("profile:a"))
        presentation.select(.profile("profile:b"))
        XCTAssertNil(presentation.error)
        XCTAssertTrue(presentation.isDefault("profile:b"))
        XCTAssertFalse(presentation.isDefault("profile:a"))
        var changed = try connections.endpointSnapshot()
        var endpoints = changed.endpoints
        endpoints[0].models[0].priority = 100
        try connections.replaceEndpoints(endpoints, expectedRevisions: changed.revisions)
        presentation.reload()
        XCTAssertTrue(presentation.isDefault("profile:b"))
        changed = try connections.endpointSnapshot()
        endpoints = changed.endpoints
        endpoints[1].isEnabled = false
        try connections.replaceEndpoints(endpoints, expectedRevisions: changed.revisions)
        presentation.reload()
        XCTAssertEqual(presentation.resolution, .unavailable(profileID: "profile:b", origin: .default, reason: .disabled))
        XCTAssertTrue(presentation.isDefault("profile:b"))
        XCTAssertFalse(presentation.canSelect("profile:b"))
    }

    private func fixtures() -> [MacRemoteEndpointConfiguration] {
        [
            .init(id: "connection:a", name: "A", baseURL: "https://a.invalid/v1",
                  models: [.init(id: "profile:a", modelName: "same-model", executionCapabilities: .init())]),
            .init(id: "connection:b", name: "B", baseURL: "https://b.invalid/v1",
                  models: [.init(id: "profile:b", modelName: "same-model", executionCapabilities: .init())])
        ]
    }
}

@MainActor
private final class LocalModelRemountFixture: ObservableObject {
    @Published var showsLocal = true
    let editing = MacLocalModelEditingState()
    let downloader = ModelDownloader(logger: Logger.shared)
    let preference: MacDefaultChatModelPresentation
    init(application: MacApplicationPort) {
        preference = MacDefaultChatModelPresentation(store: .init(application: application))
    }
}

private struct LocalModelRemountRoot: View {
    @ObservedObject var fixture: LocalModelRemountFixture
    var body: some View {
        if fixture.showsLocal {
            MacLocalModelManagerView(showsHeader: false, downloader: fixture.downloader,
                                     preference: fixture.preference, editingState: fixture.editing)
        } else { Color.clear }
    }
}

@MainActor final class MacModelConnectionServiceTests: XCTestCase {
    func testEndpointFormRejectsRoutesThatServiceWillReject() {
        XCTAssertTrue(MacModelConnectionsView.isValidEndpoint("https://models.example/v1"))
        for endpoint in ["https://models.example/v1?key=secret", "https://models.example/v1#route",
                         "https://user:secret@models.example/v1", "file:///tmp/models"] {
            XCTAssertFalse(MacModelConnectionsView.isValidEndpoint(endpoint), endpoint)
        }
    }

    func testImportedProviderFalseSurvivesOmittedOptionalCapabilityFields() throws {
        let data = Data(#"{"toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":false}"#.utf8)
        let declaration = try JSONDecoder().decode(MacModelExecutionCapabilities.self, from: data)
        XCTAssertFalse(declaration.toolCalls)
        XCTAssertFalse(declaration.samplingRequiresNoReasoning)
        XCTAssertNil(declaration.chatOutputTokenField)
    }

    func testSourceManagementDistinguishesLiteralLoopbackFromRemoteOwner() throws {
        for address in ["http://127.0.0.1:5001", "https://[::1]:5001"] {
            XCTAssertTrue(MacModelConnectionsView.isLocalManagementHost(try XCTUnwrap(URL(string: address))))
        }
        for address in ["https://host.example", "http://192.0.2.1:5001", "http://localhost:5001",
                        "http://127.0.0.1.example:5001", "ftp://127.0.0.1", "http://user@127.0.0.1"] {
            XCTAssertFalse(MacModelConnectionsView.isLocalManagementHost(try XCTUnwrap(URL(string: address))))
        }
    }

    func testCapabilityOverrideProjectsRuntimeAndClearPreservesSelection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at:root) }
        let application = try MacApplicationPort(databaseURL:root.appendingPathComponent("override.sqlite"))
        let service = MacModelConnectionService(application:application)
        let initial = try service.create(name:"Fixture", endpoint:"https://fixture.invalid/v1", wireProtocol:"responses", secret:"")
        let modelData = Data(#"{"model_id":"fixture","display_name":"Fixture","protocol":"responses","context_window":null,"output_limit":null,"modalities":[],"reasoning_levels":[],"execution_capabilities":null}"#.utf8)
        let model = try JSONDecoder().decode(MacModelConnectionSnapshot.Model.self, from:modelData)
        let modelJSON = try JSONDecoder().decode(MacCanonicalJSONValue.self, from:modelData)
        let catalog: MacModelConnectionSnapshot = try service.command(["type":.string("refresh_connection_catalog"),
            "connection_id":.string(initial.id), "expected_revision":.signedInteger(initial.revision),
            "models":.array([modelJSON]), "observed_at_ms":.signedInteger(1)])
        let selected: MacModelConnectionSnapshot = try service.command(["type":.string("configure_connection_models"),
            "connection_id":.string(initial.id), "expected_revision":.signedInteger(catalog.revision),
            "enabled_model_ids":.array([.string(model.id)]), "selected_model_id":.string(model.id),
            "updated_at_ms":.signedInteger(2)])
        let capabilities = MacCanonicalJSONValue.object(["toolCalls":.bool(true), "imageInput":.bool(false),
            "reasoning":.bool(false), "samplingParameters":.bool(false)])
        let overridden = try service.setCapabilityOverride(selected, model:model, capabilities:capabilities)
        XCTAssertNil(overridden.document.catalog?.models.first?.execution_capabilities)
        XCTAssertNotNil(overridden.document.capability_assessments?.first?.user_override)
        struct Profile: Decodable { let selected: Bool; let source_json: String }
        let profiles: [Profile] = try service.command(["type":.string("list_profiles")])
        let profile = try XCTUnwrap(profiles.first)
        XCTAssertTrue(profile.selected)
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with:Data(profile.source_json.utf8)) as? [String:Any])
        XCTAssertEqual(source["executionCapabilityBasis"] as? String, "user_override")
        XCTAssertEqual((source["executionCapabilities"] as? [String:Any])?["toolCalls"] as? Bool, true)
        let cleared = try service.setCapabilityOverride(overridden, model:model, capabilities:nil)
        XCTAssertNil(cleared.document.capability_assessments?.first?.user_override)
        let after: [Profile] = try service.command(["type":.string("list_profiles")])
        XCTAssertTrue(try XCTUnwrap(after.first).selected)
        XCTAssertThrowsError(try service.setCapabilityOverride(selected, model:model, capabilities:capabilities))
    }

    func testCatalogTransportBoundsStreamingBodyAndRejectsRedirect() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CatalogFixtureProtocol.self]
        for path in ["oversized", "declared", "redirect"] {
            let started = ContinuousClock.now
            do {
                _ = try await AppleModelCatalogTransport.fetch(
                    URLRequest(url: URL(string: "https://catalog.invalid/" + path)!),
                    configuration: configuration)
                XCTFail("Expected rejection: " + path)
            } catch let error as URLError {
                XCTAssertEqual(error.code, path == "redirect" ? .badServerResponse : .dataLengthExceedsMaximum)
            }
            XCTAssertLessThan(ContinuousClock.now - started, .seconds(5), "A buffered catalog response must not spend seconds iterating individual bytes")
        }
        let (data, _) = try await AppleModelCatalogTransport.fetch(
            URLRequest(url: URL(string: "https://catalog.invalid/valid")!), configuration: configuration)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"data\":[]}")
    }

    func testCancellingActiveCatalogReadSettlesAndReleasesCapacity() async throws {
        let started = expectation(description: "Catalog request reached its transport")
        CatalogFixtureProtocol.observer.set { started.fulfill() }
        defer { CatalogFixtureProtocol.observer.set(nil) }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [CatalogFixtureProtocol.self]
        let request = Task {
            try await AppleModelCatalogTransport.fetch(
                URLRequest(url: URL(string: "https://catalog.invalid/waiting")!), configuration: configuration)
        }
        defer { request.cancel() }
        await fulfillment(of: [started], timeout: 2)
        request.cancel()
        do {
            _ = try await request.value
            XCTFail("A cancelled catalog read cannot report a completed catalog")
        } catch is CancellationError {} catch { XCTFail("Unexpected cancellation result: \(error)") }
        let (data, _) = try await AppleModelCatalogTransport.fetch(
            URLRequest(url: URL(string: "https://catalog.invalid/valid")!), configuration: configuration)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"data\":[]}")
    }

    func testCatalogRedirectDelegateNeverForwardsCredential() {
        let transport = AppleModelCatalogTransport()
        let url = URL(string: "https://catalog.invalid/models")!
        let task = URLSession.shared.dataTask(with: url)
        transport.urlSession(URLSession.shared, task: task,
            willPerformHTTPRedirection: HTTPURLResponse(url: url, statusCode: 302, httpVersion: nil,
                headerFields: ["Location": "https://untrusted.invalid"])!,
            newRequest: URLRequest(url: URL(string: "https://untrusted.invalid")!)) { request in
                XCTAssertNil(request)
            }
        task.cancel()
    }

    func testAlreadyCancelledCatalogDoesNotStartNetwork() async {
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await AppleModelCatalogTransport.fetch(
                URLRequest(url: URL(string: "https://catalog.invalid/must-not-start")!))
        }
        do {
            _ = try await task.value
            XCTFail("Cancelled task must not issue a directory request")
        } catch is CancellationError {
        } catch {
            XCTFail("Expected cancellation, got \(error)")
        }
    }

    func testAnonymousConnectionUsesCurrentCoreCommandsAndDisconnectDoesNotDelete() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("connection.sqlite"))
        let service = MacModelConnectionService(application: application)
        let created = try service.create(name: "Fixture", endpoint: "http://127.0.0.1:9999/v1",
            wireProtocol: "responses", secret: "")
        XCTAssertTrue(created.document.connection.enabled)
        XCTAssertNil(created.document.connection.account_id)
        XCTAssertEqual(try service.credential(connectionID: created.id, endpoint: "http://127.0.0.1:9999/v1",
            wireProtocol: "responses").secret, "")
        _ = try service.disconnect(created)
        let retained = try XCTUnwrap(service.connections().first(where: { $0.id == created.id }))
        XCTAssertFalse(retained.document.connection.enabled)
        XCTAssertThrowsError(try service.credential(connectionID: created.id, endpoint: "http://127.0.0.1:9999/v1",
            wireProtocol: "responses"))
    }
}

private nonisolated final class CatalogFixtureObserver: @unchecked Sendable {
    private let lock = NSLock()
    private var started: (() -> Void)?
    func set(_ callback: (() -> Void)?) { lock.withLock { started = callback } }
    func notify() { let callback = lock.withLock { started }; callback?() }
}

private final class CatalogFixtureProtocol: URLProtocol, @unchecked Sendable {
    static let observer = CatalogFixtureObserver()
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "catalog.invalid" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let path = request.url!.lastPathComponent
        if path == "waiting" { Self.observer.notify(); return }
        let response = HTTPURLResponse(url: request.url!, statusCode: path == "redirect" ? 302 : 200,
            httpVersion: "HTTP/1.1", headerFields: path == "declared" ? ["Content-Length": "2097153"] : [:])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: path == "oversized" ? Data(repeating: 32, count: 2097153) : Data("{\"data\":[]}".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

final class MacModelImportBridgeTests: XCTestCase {
    private nonisolated static func readFrame(_ file: FileHandle) throws -> [String:Any] {
        var data = Data()
        while let byte = try file.read(upToCount: 1), !byte.isEmpty {
            if byte.first == 10 { return try XCTUnwrap(JSONSerialization.jsonObject(with:data) as? [String:Any]) }
            data.append(byte)
            if data.count > 4 * 1024 * 1024 { throw NSError(domain:"fixture", code:1) }
        }
        throw NSError(domain:"fixture", code:2)
    }
    private nonisolated static func writeFrame(_ value: [String:Any], to file: FileHandle) throws {
        var data = try JSONSerialization.data(withJSONObject:value)
        data.append(10)
        try file.write(contentsOf:data)
    }
    private nonisolated static func modelFixture() throws -> MacModelConnectionSnapshot.Model {
        try JSONDecoder().decode(MacModelConnectionSnapshot.Model.self, from:Data(#"{"model_id":"fixture-model","display_name":"Fixture","protocol":"responses","context_window":null,"output_limit":null,"modalities":[],"reasoning_levels":[],"execution_capabilities":null}"#.utf8))
    }
    func testProbePrepareRejectsUnboundModelReceiptOverPrivatePipe() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at:root) }
        let application = try MacApplicationPort(databaseURL:root.appendingPathComponent("fixture.sqlite"))
        let service = MacModelConnectionService(application:application)
        let snapshot = try service.create(name:"Fixture", endpoint:"https://fixture.invalid/v1", wireProtocol:"responses", secret:"")
        let incoming = Pipe(), outgoing = Pipe()
        let bridge = MacModelImportBridge(application:application, fixtureInput:incoming.fileHandleForWriting, fixtureOutput:outgoing.fileHandleForReading)
        defer { bridge.cancel() }
        let requestInput = incoming.fileHandleForReading, responseOutput = outgoing.fileHandleForWriting
        let worker = Task.detached {
            let request = try Self.readFrame(requestInput)
            XCTAssertEqual(request["type"] as? String, "probe_prepare")
            try Self.writeFrame(["type":"probe_prepare", "request_id":request["request_id"]!,
                "confirmation":["request_id":UUID().uuidString.lowercased(), "connection_id":snapshot.id,
                    "expected_revision":snapshot.revision, "account_id":NSNull(), "account_revision":NSNull(),
                    "credential_binding":String(repeating:"a", count:64), "model_id":"wrong-model",
                    "protocol":"responses", "max_requests":1, "acknowledge_possible_charges":false]], to:responseOutput)
        }
        do { _ = try await bridge.prepareProbe(snapshot:snapshot, model:Self.modelFixture()); XCTFail("Unbound model must be rejected") }
        catch {}
        try await worker.value
    }

    func testProbeConfirmationRequiresCostApprovalAndExactResultIdentity() async throws {
        for wrongIdentity in [false, true] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at:root) }
            let application = try MacApplicationPort(databaseURL:root.appendingPathComponent("fixture.sqlite"))
            let service = MacModelConnectionService(application:application)
            let snapshot = try service.create(name:"Fixture", endpoint:"https://fixture.invalid/v1", wireProtocol:"responses", secret:"")
            let incoming = Pipe(), outgoing = Pipe()
            let bridge = MacModelImportBridge(application:application, fixtureInput:incoming.fileHandleForWriting, fixtureOutput:outgoing.fileHandleForReading)
            defer { bridge.cancel() }
            let snapshotData = try JSONEncoder().encode(snapshot)
            let requestInput = incoming.fileHandleForReading, responseOutput = outgoing.fileHandleForWriting
            let worker = Task.detached {
                let preparation = try Self.readFrame(requestInput)
                let id = UUID().uuidString.lowercased()
                try Self.writeFrame(["type":"probe_prepare", "request_id":preparation["request_id"]!,
                    "confirmation":["request_id":id, "connection_id":snapshot.id,
                        "expected_revision":snapshot.revision, "account_id":NSNull(), "account_revision":NSNull(),
                        "credential_binding":String(repeating:"a", count:64), "model_id":"fixture-model",
                        "protocol":"responses", "max_requests":1, "acknowledge_possible_charges":false]], to:responseOutput)
                let confirmation = try Self.readFrame(requestInput)
                XCTAssertEqual(confirmation["type"] as? String, "probe")
                let body = try XCTUnwrap(confirmation["request"] as? [String:Any])
                XCTAssertEqual(body["acknowledge_possible_charges"] as? Bool, true)
                XCTAssertEqual(body["request_id"] as? String, id)
                var result = try XCTUnwrap(JSONSerialization.jsonObject(with:snapshotData) as? [String:Any])
                result["revision"] = snapshot.revision + 1
                if wrongIdentity {
                    var document = result["document"] as! [String:Any]
                    var connection = document["connection"] as! [String:Any]
                    connection["id"] = "other-owner"
                    document["connection"] = connection
                    result["document"] = document
                }
                try Self.writeFrame(["type":"probe", "request_id":confirmation["request_id"]!, "snapshot":result], to:responseOutput)
            }
            let prepared = try await bridge.prepareProbe(snapshot:snapshot, model:Self.modelFixture())
            do { _ = try await bridge.confirmProbe(requestID:prepared.request_id, acknowledgePossibleCharges:false); XCTFail("Must require cost approval") }
            catch {}
            do {
                let result = try await bridge.confirmProbe(requestID:prepared.request_id, acknowledgePossibleCharges:true)
                XCTAssertFalse(wrongIdentity)
                XCTAssertEqual(result.id, snapshot.id)
            } catch { XCTAssertTrue(wrongIdentity) }
            try await worker.value
        }
    }

    func testProbeCredentialReadIsBoundToApprovedLocalAccountAndNonce() throws {
        let data = Data(#"{"revision":2,"document":{"format_version":1,"connection":{"id":"connection","display_name":"Fixture","base_url":"https://fixture.invalid/v1","protocols":["responses"],"account_id":"local-account","source":"manual","source_id":null,"enabled":true},"catalog":null}}"#.utf8)
        let snapshot = try JSONDecoder().decode(MacModelConnectionSnapshot.self, from: data)
        let approved: [String:Any] = ["connection_id":"connection", "endpoint":"https://fixture.invalid/v1",
            "protocol":"responses", "account_id":"local-account", "nonce":UUID().uuidString]
        XCTAssertTrue(MacModelImportBridge.matchesProbeCredentialBinding(approved, snapshot:snapshot, wireProtocol:"responses"))
        for (key, value) in [("connection_id","other"), ("endpoint","https://other.invalid/v1"),
            ("protocol","anthropic"), ("account_id","remote-account"), ("nonce","invalid")] {
            var changed = approved; changed[key] = value
            XCTAssertFalse(MacModelImportBridge.matchesProbeCredentialBinding(changed, snapshot:snapshot, wireProtocol:"responses"))
        }
        XCTAssertFalse(MacModelImportBridge.matchesProbeCredentialBinding(approved, snapshot:snapshot, wireProtocol:"anthropic"))
    }

    func testCredentialSinkRequiresApprovedConnectionEndpointProtocolAndAccount() throws {
        let data = Data(#"{"plan_id":"fixture","expires_at_ms":1,"candidates":[{"candidate_id":"candidate","connection":{"id":"connection","display_name":"Fixture","base_url":"https://fixture.invalid/v1","protocols":["responses"],"account_id":"existing-account","source":"codex","source_id":"fixture","enabled":true},"models":[],"credential_status":"available","issue":null,"existing_revision":1}]}"#.utf8)
        let candidate = try XCTUnwrap(JSONDecoder().decode(MacModelImportPlan.self, from: data).candidates.first)
        let approved: [String: Any] = ["connection_id": "connection", "endpoint": "https://fixture.invalid/v1",
            "protocols": ["responses"], "account_id": "existing-account"]
        XCTAssertTrue(MacModelImportBridge.matchesCredentialBinding(approved, candidate: candidate))
        for (key, value) in [("connection_id", "other"), ("endpoint", "https://other.invalid/v1"), ("account_id", "other-account")] {
            var changed = approved; changed[key] = value
            XCTAssertFalse(MacModelImportBridge.matchesCredentialBinding(changed, candidate: candidate))
        }
        var changed = approved; changed["protocols"] = ["chat_completions"]
        XCTAssertFalse(MacModelImportBridge.matchesCredentialBinding(changed, candidate: candidate))
    }
}

final class MacReasoningConfigurationTests: XCTestCase {
    func testLaunchDescriptorKeepsOptionalSummaryOutOfBooleanCapabilities() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        for summary in [nil, false, true] as [Bool?] {
            let request = try MacLocalAgentRunRequest(
                conversationID: "summary-launch-fixture", prompt: "fixture", projectURL: root,
                attachmentURLs: [], modelID: "fixture",
                modelExecutionCapabilities: .init(reasoningSummarySupported: summary),
                appEnvironment: environment, applicationPort: port)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let descriptor = try XCTUnwrap(payload["mac_launch"] as? [String: Any])
            let capabilities = try XCTUnwrap(descriptor["model_capabilities"] as? [String: Bool])
            XCTAssertNil(capabilities["reasoning_summary_supported"])
            XCTAssertEqual(payload["reasoning_summary_supported"] as? Bool, summary)
        }
    }

    func testCanonicalRunKeepsCapturedCurrentPromptAndCoreOwnedHistory() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let host = MacAgentHostRuntime()
        for prompt in ["Explain blue sky. Begin SKY_REVIEW", "Now explain red sunsets. Begin SUNSET_REVIEW", "再解释一下", "再解释一下"] {
            let request = try MacLocalAgentRunRequest(conversationID: "turn-input-fixture",
                prompt: prompt, projectURL: root, attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                appEnvironment: environment, applicationPort: port)
            let data = try await host.initialRequestData(for: request, localEndpoint: nil)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(payload["prompt"] as? String, prompt)
            XCTAssertNil(payload["history"], "Canonical run must load history once from Core")
        }
    }

    func testSharedTurnPreparationPreservesRepeatedCommittedMessages() {
        let turn = ConversationTurnInput(currentUserText: "same", historySource: .prepared([
            .init(role: "system", content: "separate context"),
            .init(role: "user", content: "same"), .init(role: "assistant", content: "first answer")]))
        XCTAssertEqual(turn.input, "same")
        XCTAssertEqual(turn.history.map(\.role), ["user", "assistant"])
        XCTAssertEqual(turn.history.map(\.content), ["same", "first answer"])
        XCTAssertTrue(ConversationTurnInput(currentUserText: "first", historySource: .coreManaged).history.isEmpty)
    }

    func testNativeLaunchDoesNotDuplicateStdinPermissionPolicy() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let host = MacAgentHostRuntime()
        for mode in MacPermissionMode.allCases {
            let request = try MacLocalAgentRunRequest(
                conversationID: "permission-envelope-fixture", prompt: "fixture", projectURL: root,
                attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                permissionMode: mode, appEnvironment: environment, applicationPort: port
            )
            let arguments = await host.arguments(for: request, localEndpoint: nil)
            XCTAssertFalse(arguments.contains("--yes"))
            XCTAssertFalse(arguments.contains("--network-policy"))
            XCTAssertTrue(arguments.contains("--host-request-stdin"))
            let data = try await host.initialRequestData(for: request, localEndpoint: nil)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            XCTAssertEqual(payload["permission_profile"] as? String, mode.permissionProfile)
        }
    }

    func testConditionalSamplingUsesResolvedNoneEvenForDefaultValues() async throws {
        let payload = try await samplingPayload(effort: "none", capabilities: .init(
            reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true))
        XCTAssertEqual(payload["sampling_override"] as? Bool, true)
        XCTAssertEqual(payload["temperature"] as? Double, 0.7)
        XCTAssertEqual(payload["top_p"] as? Double, 0.9)
        XCTAssertEqual((payload["reasoning_selection"] as? [String: Any])?["value"] as? String, "none")
    }

    func testConditionalSamplingOmitsDefaultsForResolvedReasoning() async throws {
        let payload = try await samplingPayload(effort: "high", capabilities: .init(
            reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true))
        XCTAssertEqual(payload["sampling_override"] as? Bool, false)
    }

    func testExplicitNondefaultSamplingStillRejectsIncompatibleReasoning() async throws {
        do {
            _ = try await samplingPayload(effort: "high", capabilities: .init(
                reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true),
                explicit: true, temperature: 0)
            XCTFail("Explicit sampling cannot be silently omitted")
        } catch {
            XCTAssertEqual((error as NSError).domain, "NolaMac.LocalAgentRequest")
            XCTAssertEqual((error as NSError).code, 4)
        }
    }

    func testUnconditionalSamplingDeclarationSendsFrozenDefaults() async throws {
        let payload = try await samplingPayload(effort: "high", capabilities: .init(
            reasoning: true, samplingParameters: true))
        XCTAssertEqual(payload["sampling_override"] as? Bool, true)
    }

    func testUnknownSamplingCapabilityIsNotPromoted() async throws {
        let payload = try await samplingPayload(effort: nil, capabilities: nil)
        XCTAssertEqual(payload["sampling_override"] as? Bool, false)
    }

    private func samplingPayload(
        effort: String?, capabilities: MacModelExecutionCapabilities?,
        explicit: Bool = false, temperature: Double = 0.7
    ) async throws -> [String: Any] {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = temperature
        parameters.topP = 0.9
        parameters.maxOutputTokens = 4096
        let declaration = effort.map {
            MacReasoningConfiguration(control: .effort, responses: true, efforts: [$0])
        } ?? MacReasoningConfiguration()
        let request = try MacLocalAgentRunRequest(
            conversationID: "sampling-fixture", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: "fixture", modelTestMode: true,
            reasoningConfiguration: declaration, reasoningTier: "concise",
            modelParameters: parameters, modelExecutionCapabilities: capabilities,
            wireProtocol: .responses, samplingOverride: explicit,
            appEnvironment: environment, applicationPort: port
        )
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    func testLocalMLXAlwaysSendsFrozenSamplingEvenWhenValuesEqualUIDefaults() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.7
        parameters.topP = 0.9
        parameters.maxOutputTokens = 4096
        let modelID = MacLocalModelProfile.qwen35_08B.id
        let request = try MacLocalAgentRunRequest(
            conversationID: "local-sampling", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: modelID, modelTestMode: true,
            modelParameters: parameters, wireProtocol: .chatCompletions,
            samplingOverride: false, appEnvironment: environment, applicationPort: port
        )
        let endpoint = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1:12345", apiKey: "fixture-token", modelID: modelID, contextWindowTokens: 32768, profileRevision: 1)
        let host = MacAgentHostRuntime()
        let arguments = await host.arguments(for: request, localEndpoint: endpoint)
        XCTAssertFalse(arguments.contains("--api-key"))
        XCTAssertFalse(arguments.contains(endpoint.apiKey))
        let data = try await host.initialRequestData(for: request, localEndpoint: endpoint)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["api_key"] as? String, endpoint.apiKey)
        XCTAssertEqual(payload["sampling_override"] as? Bool, true)
        XCTAssertEqual(payload["temperature"] as? Double, 0.7)
        XCTAssertEqual(payload["top_p"] as? Double, 0.9)
    }

    func testApplicationReasoningResolutionUsesEffectiveProtocolAndSharedTierMapping() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("reasoning.db"))
        // Stale UI metadata must not choose the transport or weaken admission.
        let declaration = MacReasoningConfiguration(control: .effort, responses: false, efforts: ["high", "low"])
        let value = try port.resolveReasoning(
            declaration: declaration, wireProtocol: .responses, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "balanced"
        )
        XCTAssertEqual(value, .object(["kind": .string("effort"), "value": .string("low")]))
        XCTAssertEqual(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "balanced"
        ), .object(["kind": .string("effort"), "value": .string("low")]))
        XCTAssertThrowsError(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .responses, outputLimit: 4096,
            supportsReasoning: false, requested: .null, tier: "balanced"
        ))
    }

    func testApplicationReasoningBudgetChecksOnlySelectedTierAgainstEffectiveOutput() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("reasoning.db"))
        let declaration = MacReasoningConfiguration(control: .budget)
        XCTAssertEqual(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "concise"
        ), .object(["kind": .string("budget"), "tokens": .number(1024)]))
        XCTAssertThrowsError(try port.resolveReasoning(
            declaration: declaration, wireProtocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, requested: .null, tier: "detailed"
        ))
        XCTAssertEqual(try port.resolveReasoning(
            declaration: nil, wireProtocol: .responses, outputLimit: 4096,
            supportsReasoning: false, requested: .null, tier: "detailed"
        ), .null)
    }

    func testNativeHostPayloadUsesApplicationResolutionAndFrozenContext() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.maxOutputTokens = 4096
        parameters.maxContextLength = 123456
        let request = try MacLocalAgentRunRequest(
            conversationID: "reasoning-fixture", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: "unknown-provider-model", modelTestMode: true,
            reasoningConfiguration: .init(control: .budget), reasoningPreference: "automatic",
            reasoningTier: "concise", modelParameters: parameters,
            modelExecutionCapabilities: .init(reasoning: true), wireProtocol: .anthropic,
            appEnvironment: environment, applicationPort: port
        )
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
        XCTAssertEqual(selection["kind"] as? String, "budget")
        XCTAssertEqual(selection["tokens"] as? Int, 1024)
        XCTAssertEqual(payload["max_output_tokens"] as? Int, 4096)
        XCTAssertEqual(payload["context_window_tokens"] as? Int, 123456)
    }

    func testNativeHostDoesNotFallBackWhenApplicationPortIsMissing() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let request = try MacLocalAgentRunRequest(
            conversationID: "missing-port", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelTestMode: true,
            appEnvironment: MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root),
            applicationPort: nil
        )
        do {
            _ = try await MacAgentHostRuntime().initialRequestData(for: request)
            XCTFail("Missing Application must not silently use local reasoning logic")
        } catch {
            XCTAssertEqual(error as? MacApplicationPortError, .serviceUnavailable)
        }
    }

    func testNativeHostTierDoesNotSubmitTheOldLocalSelectionAsASecondOverride() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let request = try MacLocalAgentRunRequest(
            conversationID: "tier-precedence", prompt: "fixture", projectURL: root,
            attachmentURLs: [], modelID: "fixture", modelTestMode: true,
            reasoningConfiguration: .init(control: .effort, responses: true, efforts: ["low", "medium", "high"]),
            reasoningPreference: "low", reasoningTier: "balanced",
            modelExecutionCapabilities: .init(reasoning: true), wireProtocol: .responses,
            appEnvironment: environment, applicationPort: port
        )
        let data = try await MacAgentHostRuntime().initialRequestData(for: request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
        XCTAssertEqual(selection["value"] as? String, "medium")
    }

    func testNativeHostPreservesAllFiveComposerEfforts() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let efforts = ["none", "low", "medium", "high", "xhigh"]
        for effort in efforts {
            let request = try MacLocalAgentRunRequest(
                conversationID: "exact-effort", prompt: "fixture", projectURL: root,
                attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                reasoningConfiguration: .init(control: .effort, responses: true, efforts: efforts),
                reasoningPreference: effort,
                modelExecutionCapabilities: .init(reasoning: true), wireProtocol: .responses,
                appEnvironment: environment, applicationPort: port)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let selection = try XCTUnwrap(payload["reasoning_selection"] as? [String: Any])
            XCTAssertEqual(selection["value"] as? String, effort)
        }
    }

    func testBudgetEditorRetainsInvalidRawInputAndRejectsItsConfiguration() {
        for text in ["", "-", "1.5", "NaN", "123x", "9999999999999999999999999999"] {
            var draft = MacReasoningBudgetDraft()
            var configuration = MacReasoningConfiguration(control: .budget)
            configuration.budgets[0] = draft.update(text, at: 0)
            XCTAssertEqual(draft.text(at: 0, value: configuration.budgets[0]), text)
            XCTAssertFalse(configuration.isValid(for: .anthropic), "Invalid edit must not save the old 1024 budget: \(text)")
            configuration.budgets[0] = draft.update("2048", at: 0)
            XCTAssertTrue(configuration.isValid(for: .anthropic))
        }
    }

    func testBudgetEditorDisplaysReloadedValueInsteadOfStaleDraft() {
        var draft = MacReasoningBudgetDraft()
        XCTAssertEqual(draft.update("-", at: 0), -1)
        XCTAssertEqual(draft.text(at: 0, value: 2048), "2048")
        XCTAssertEqual(draft.update("002048", at: 0), 2048)
        XCTAssertEqual(draft.text(at: 0, value: 2048), "002048")
        XCTAssertEqual(draft.text(at: 1, value: 4096), "4096")
    }

    func testAutomaticDoesNotMakeInvalidBudgetDraftSaveable() {
        var draft = MacReasoningBudgetDraft()
        var configuration = MacReasoningConfiguration(control: .budget)
        configuration.budgets[0] = draft.update("-", at: 0)
        configuration.control = .automatic
        for wireProtocol in MacModelWireProtocol.allCases {
            XCTAssertFalse(configuration.isValid(for: wireProtocol))
            XCTAssertFalse(configuration.isValid(for: wireProtocol, preferenceOverride: "automatic"))
        }
        configuration.budgets[0] = draft.update("2048", at: 0)
        XCTAssertTrue(configuration.isValid(for: .anthropic))
    }

    func testInactiveReasoningFieldsStillObeySharedSchema() {
        for control in MacReasoningConfiguration.Control.allCases {
            var configuration = MacReasoningConfiguration(control: control, efforts: ["low", "medium", "high"])
            let wireProtocol: MacModelWireProtocol = control == .budget ? .anthropic : .responses
            XCTAssertTrue(configuration.isValid(for: wireProtocol))
            configuration.efforts.append("unknown-effort")
            XCTAssertFalse(configuration.isValid(for: wireProtocol))
            configuration.efforts.removeLast()
            configuration.budgets[0] = -1
            XCTAssertFalse(configuration.isValid(for: wireProtocol))
        }
        XCTAssertTrue(MacReasoningConfiguration(control: .automatic, budgets: [0]).isValid(for: .responses),
                      "Nonnegative inactive budgets are schema-valid; three-tier constraints apply only to budget control")
    }

    func testUnsupportedDerivedEffortIsRejected() {
        let configuration = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        XCTAssertFalse(configuration.isValid(for: .responses, preferenceOverride: "xhigh"))
        XCTAssertTrue(configuration.isValid(for: .chatCompletions, preferenceOverride: "high"))
    }

    func testDerivedPreferenceIsValidatedAgainstCurrentModelCapability() {
        let configuration = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        XCTAssertTrue(configuration.isValid(for: .responses, preferenceOverride: nil))
        XCTAssertTrue(configuration.isValid(for: .responses, preferenceOverride: "low"))
        XCTAssertTrue(configuration.isValid(for: .responses, preferenceOverride: "automatic"))
        XCTAssertFalse(configuration.isValid(for: .responses, preferenceOverride: "medium"))
    }

    func testModelCapabilityRoundTripsWithoutASecondDefaultLevel() throws {
        let configuration = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        XCTAssertTrue(configuration.isValid(for: .responses))
        let restored = try JSONDecoder().decode(MacReasoningConfiguration.self, from: JSONEncoder().encode(configuration))
        XCTAssertEqual(restored, configuration)
        let encoded = try XCTUnwrap(String(data: JSONEncoder().encode(configuration), encoding: .utf8))
        XCTAssertFalse(encoded.contains("defaultPreference"))
    }

    func testConversationModelParametersStayWithTheirModelAcrossPersistence() throws {
        let first = MacConversationModelOverride(temperature: 0.25)
        let second = MacConversationModelOverride(maxOutputTokens: 8192)
        let overrides = ["configured:first": first, "configured:second": second]
        let restored = try JSONDecoder().decode([String: MacConversationModelOverride].self, from: JSONEncoder().encode(overrides))
        XCTAssertEqual(restored["configured:first"]?.temperature, 0.25)
        XCTAssertEqual(restored["configured:second"]?.maxOutputTokens, 8192)
        let encoded = try XCTUnwrap(String(data: JSONEncoder().encode(overrides), encoding: .utf8))
        XCTAssertFalse(encoded.contains("reasoning"), "Sampling-only overrides do not add a reasoning preference")
    }
    func testProtocolSwitchValidatesInheritedContractAgainstEffectiveProtocol() {
        let budget = MacReasoningConfiguration(control: .budget)
        XCTAssertTrue(budget.isValid(for: .anthropic))
        XCTAssertFalse(budget.isValid(for: .responses))
        let effort = MacReasoningConfiguration(control: .effort, efforts: ["high"])
        XCTAssertTrue(effort.isValid(for: .responses))
        XCTAssertTrue(effort.isValid(for: .anthropic))
        XCTAssertTrue(effort.isValid(for: .chatCompletions), "Exact effort controls preserve sparse declarations")
        var inheritedBudget = budget
        inheritedBudget.outputLimit = 4096
        XCTAssertFalse(inheritedBudget.isValid(for: .anthropic), "Session output limits must validate inherited budgets too")
    }
    func testThinkingControlsAreProtocolSpecific() {
        XCTAssertEqual(MacReasoningConfiguration.availableControls(for: .responses), [.automatic, .effort])
        XCTAssertEqual(MacReasoningConfiguration.availableControls(for: .chatCompletions), [.automatic, .effort])
        XCTAssertEqual(MacReasoningConfiguration.availableControls(for: .anthropic), [.automatic, .effort, .budget])
        XCTAssertFalse(MacReasoningConfiguration(control: .budget).supports(.responses))
        XCTAssertTrue(MacReasoningConfiguration(control: .effort).supports(.anthropic))
    }
    func testReasoningMenuLabelsAreBundled() throws {
        let path = try XCTUnwrap(Bundle.main.path(forResource: "Reasoning", ofType: "strings", inDirectory: nil, forLocalization: "en"))
        let data = try Data(contentsOf: URL(fileURLWithPath: path))
        let labels = try XCTUnwrap(PropertyListSerialization.propertyList(from: data, format: nil) as? [String: String])
        for effort in ["automatic", "none", "minimal", "low", "medium", "high", "xhigh", "max"] {
            XCTAssertNotNil(labels["reasoning.effort." + effort])
        }
    }

    func testComposerPreservesAllDeclaredEfforts() throws {
        let efforts = ["none", "low", "medium", "high", "xhigh"]
        for responses in [false, true] {
            let configuration = MacReasoningConfiguration(control: .effort, responses: responses, efforts: efforts)
            XCTAssertEqual(configuration.options, efforts)
            for effort in efforts {
                XCTAssertEqual(configuration.selection(effort)?["value"] as? String, effort)
                let override = MacConversationModelOverride(reasoningPreference: effort)
                let decoded = try JSONDecoder().decode(MacConversationModelOverride.self, from: JSONEncoder().encode(override))
                XCTAssertEqual(decoded.applying(to: ModelParameters()).reasoningPreference, effort)
            }
            XCTAssertNil(configuration.selection("balanced"))
        }
        let state = MacComposerDisplayState(conversationID: "test", draft: "", attachments: [],
            isRunning: false, isVisiblyRunning: false, reasoningOptions: efforts, reasoningPreference: "xhigh")
        XCTAssertEqual(state.reasoningOptions, efforts)
        XCTAssertEqual(state.reasoningPreference, "xhigh")
    }

    func testEachResponseDepthHasUserVisibleOutputGuidance() {
        let instructions = MacResponseStyle.allCases.map(\.responseInstruction)
        XCTAssertEqual(Set(instructions).count, 3)
        XCTAssertTrue(instructions.allSatisfy { !$0.isEmpty })
        XCTAssertTrue(MacResponseStyle.concise.responseInstruction.contains("concisely"))
        XCTAssertTrue(MacResponseStyle.detailed.responseInstruction.contains("thorough"))
    }

    func testResponseDepthInstructionSurvivesCoreContextPreparation() {
        let instruction = MacResponseStyle.concise.responseInstruction
        let messages = MacContextCore.hostContextMessages(
            userPrompt: "Summarize the results",
            components: [MacCoreContextComponent(
                kind: "prompt_template",
                id: "macos:response-depth",
                content: instruction
            )],
            contextWindowTokens: 32_000
        )
        XCTAssertTrue(messages.contains { $0.content.contains(instruction) })
    }

    @MainActor
    func testModelCenterNavigationTargetsSelectedRemoteProfile() {
        let navigation = MacModelCenterNavigationState()
        XCTAssertTrue(navigation.showsLocalModels)
        navigation.showRemoteModel(profileID: "current-profile")
        XCTAssertTrue(navigation.showsRemoteModels)
        XCTAssertEqual(navigation.focusedRemoteProfileID, "current-profile")
        let firstRevision = navigation.focusRevision
        navigation.showRemoteModel(profileID: "current-profile")
        XCTAssertGreaterThan(navigation.focusRevision, firstRevision,
                             "Opening the same profile again must refocus its editor")
        navigation.showLocalModels()
        XCTAssertTrue(navigation.showsLocalModels)
        navigation.showSelectedModel(remoteProfileID: "next-profile")
        XCTAssertEqual(navigation.focusedRemoteProfileID, "next-profile")
        navigation.showSelectedModel(remoteProfileID: nil)
        XCTAssertTrue(navigation.showsLocalModels)
        XCTAssertNil(navigation.focusedRemoteProfileID)
    }

    @MainActor
    func testPermissionChipUsesTypedActionWithoutSubmittingDraft() {
        var selected: MacPermissionMode?
        let sink = MacComposerActionSink { action in
            guard case .selectPermissionMode(let conversationID, let mode) = action else {
                XCTFail("Permission selection must not submit or rewrite a draft")
                return false
            }
            XCTAssertEqual(conversationID, "conversation")
            selected = mode
            return true
        }
        XCTAssertTrue(sink.perform(.selectPermissionMode(conversationID: "conversation", .cautious)))
        XCTAssertEqual(selected, .cautious)
    }
    @MainActor
    func testModelReasoningContractSurvivesCanonicalReload() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("test.sqlite"))
        let store = MacAppleChatConfigurationStore(applicationPort: port)
        let reasoning = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high"])
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "model", modelName: "fixture",
                executionCapabilities: MacModelExecutionCapabilities(reasoning: true,
                    chatOutputTokenField: .maxCompletionTokens), reasoning: reasoning)],
            wireProtocol: .responses
        )])
        XCTAssertEqual(store.endpoints().first?.models.first?.reasoning, reasoning)
        XCTAssertEqual(store.load(model: "model")?.reasoning, reasoning)
        XCTAssertEqual(store.routePlan(preferredModel: "model")?.primary.reasoning, reasoning)
        XCTAssertEqual(store.routePlan(preferredModel: "model")?.primary.executionCapabilities.chatOutputTokenField,
            .maxCompletionTokens)
    }

    func testUnknownCapabilityDoesNotAdvertiseFakeControls() {
        XCTAssertTrue(MacReasoningConfiguration().options.isEmpty)
        XCTAssertNil(MacReasoningConfiguration().selection("high"))
    }

    @MainActor
    func testModelNameDoesNotInventSupportedReasoningEfforts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("reasoning-metadata-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = MacAppleChatConfigurationStore(
            applicationPort: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db"))
        )
        try store.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Gateway", baseURL: "https://example.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "named", modelName: "gpt-5.5", executionCapabilities: .init())],
            wireProtocol: .responses
        )])
        XCTAssertNil(store.load(model: "named")?.reasoning)
        XCTAssertTrue(MacReasoningConfiguration().options.isEmpty)
    }

    func testResponsesUsesOnlyDeclaredEfforts() {
        let config = MacReasoningConfiguration(control: .effort, responses: true, efforts: ["low", "high", "xhigh"])
        XCTAssertEqual(config.options, ["low", "high", "xhigh"])
        XCTAssertNil(config.selection("medium"))
        XCTAssertNil(config.selection("automatic"))
        XCTAssertEqual(config.selection("xhigh")?["value"] as? String, "xhigh")
    }

    func testChatCompletionsPreservesExactEfforts() {
        let config = MacReasoningConfiguration(control: .effort, efforts: ["low", "medium", "high"])
        XCTAssertEqual(config.options, ["low", "medium", "high"])
        XCTAssertEqual(config.selection("medium")?["value"] as? String, "medium")
    }

    func testBudgetsAreExplicitAndBelowOutputLimit() {
        var config = MacReasoningConfiguration(control: .budget, budgets: [1024, 2048, 4096], outputLimit: 8192)
        XCTAssertEqual(config.selection("4096")?["tokens"] as? Int, 4096)
        config.outputLimit = 4096
        XCTAssertTrue(config.options.isEmpty)
        config.outputLimit = 8192
        config.responses = true
        XCTAssertTrue(config.options.isEmpty)
    }

    @MainActor
    func testNativePlaceholderDoesNotEnterDraftOrUndoHistory() {
        let view = MacAttachmentPasteTextView(frame: NSRect(x: 0, y: 0, width: 320, height: 30))
        view.placeholder = "给 Nola 发消息"
        view.font = .systemFont(ofSize: 14.5)
        view.textContainerInset = NSSize(width: 0, height: 5)
        view.textContainer?.lineFragmentPadding = 0
        XCTAssertEqual(view.string, "")
        XCTAssertEqual(view.textContainerOrigin.x, 0)
        XCTAssertEqual(view.textContainerOrigin.y, 5)
        XCTAssertEqual(view.selectedRange(), NSRange(location: 0, length: 0))
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            view.appearance = NSAppearance(named: name)
            view.layoutSubtreeIfNeeded()
            let bitmap = try! XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let image = NSImage(size: view.bounds.size)
            image.addRepresentation(bitmap)
            let attachment = XCTAttachment(image: image)
            attachment.name = "composer-native-placeholder-\(name.rawValue)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }
}

@MainActor
final class MacLocalModelConfigurationCanonicalEndToEndTests: XCTestCase {
    func testStaleEditorCannotOverwriteNewerParameters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-mac-cas-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("state.db"))
        let configuration = MacLocalModelConfigurationStore(applicationPort: port)
        let profile = MacLocalModelProfile.qwen35_08B
        let snapshot = try configuration.parameterSnapshot(for: profile)
        var first = snapshot.parameters
        first.temperature = 0.2
        let revision = try configuration.saveParameters(first, for: profile, expectedRevision: snapshot.revision)
        var stale = snapshot.parameters
        stale.temperature = 0.9
        XCTAssertThrowsError(try configuration.saveParameters(stale, for: profile, expectedRevision: snapshot.revision)) { error in
            guard case MacApplicationServiceError.revisionConflict(let scope, let expected, let actual) = error else {
                return XCTFail("Expected typed conflict, got \(error)")
            }
            XCTAssertEqual(scope, "model_profile:\(profile.id)")
            XCTAssertEqual(expected, snapshot.revision)
            XCTAssertEqual(actual, revision)
        }
        XCTAssertEqual(try configuration.parameterSnapshot(for: profile).parameters.temperature, 0.2, accuracy: 0.001)
        first.temperature = 0.3
        let next = try configuration.saveParameters(first, for: profile, expectedRevision: revision)
        XCTAssertGreaterThan(next, revision)
    }

    func testFirstParameterConfigurationDoesNotSelectDefaultCatalogModel() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-local-unselected-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("product.db"))
        let configuration = MacLocalModelConfigurationStore(applicationPort: canonical)
        let profile = MacLocalModelProfile.qwen35_08B
        try configuration.saveParameters(.localDefaults, for: profile)
        let stored = try XCTUnwrap(canonical.modelProfile(profile.id))
        XCTAssertFalse(stored.selected, "Editing default catalog parameters must not select or start the model.")
        XCTAssertEqual(stored.provider, "local")
        XCTAssertEqual(stored.runtime, "mlx")
    }

    func testParameterEditPreservesCanonicalSourceAndSelection() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-local-parameter-only-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let canonical = try MacApplicationPort(databaseURL: root.appendingPathComponent("product.db"))
        let profile = MacLocalModelProfile.qwen35_08B
        let source = #"{"modelID":"owned-model","format":"mlx","installedDirectory":"/owned/model"}"#
        try canonical.upsertModelProfile(MacCanonicalModelProfile(
            id: profile.id, displayName: "Owned model", provider: "local",
            modelName: profile.id, runtime: "mlx", sourceJSON: source,
            parametersJSON: "{}", selected: false, enabled: true,
            revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        let configuration = MacLocalModelConfigurationStore(applicationPort: canonical)
        XCTAssertThrowsError(try configuration.parameterSnapshot(for: profile),
                             "Incomplete stored parameters must not silently become an editable default snapshot.")
        var parameters = ModelParameters.localDefaults
        parameters.temperature = 0.21
        try configuration.saveParameters(parameters, for: profile)
        let stored = try XCTUnwrap(canonical.modelProfile(profile.id))
        XCTAssertFalse(stored.selected)
        XCTAssertEqual(stored.sourceJSON, source)
        XCTAssertEqual(stored.displayName, "Owned model")
        XCTAssertEqual(configuration.parameters(for: profile).temperature, 0.21, accuracy: 0.001)
    }

    func testParametersAndSelectionSurviveRealServiceColdReopen() throws {
        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-local-model-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workspace, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let database = workspace.appendingPathComponent("nola.db")
        let profile = MacLocalModelProfile.qwen35_4B
        var parameters = ModelParameters()
        parameters.maxContextLength = 131_072
        parameters.temperature = 0.35

        do {
            let canonical = try MacApplicationPort(databaseURL: database)
            let configuration = MacLocalModelConfigurationStore(applicationPort: canonical)
            try configuration.saveParameters(parameters, for: profile)
            try configuration.select(profile)
            XCTAssertEqual(configuration.selectedModelID, profile.id)
        }

        let reopened = try MacApplicationPort(databaseURL: database)
        let configuration = MacLocalModelConfigurationStore(applicationPort: reopened)
        XCTAssertEqual(configuration.selectedModelID, profile.id)
        XCTAssertEqual(configuration.selectedProfile.id, profile.id)
        XCTAssertEqual(configuration.parameters(for: profile).maxContextLength, 131_072)
        XCTAssertEqual(configuration.parameters(for: profile).temperature, 0.35, accuracy: 0.001)
        let stored = try XCTUnwrap(reopened.modelProfile(profile.id))
        XCTAssertTrue(stored.selected)
        XCTAssertEqual(stored.provider, "local")
        XCTAssertEqual(stored.runtime, "mlx")
    }
}

@MainActor
final class MacConnectedHostModelSelectionTests: XCTestCase {
    func testConflictRefreshesOnlyOriginalOwnerWithoutReplayingEdit() async throws {
        let (root, model) = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: root) }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.setFixtureRemoteModels([profile(id: "first"), profile(id: "second")])
        let owner = try XCTUnwrap(model.selectedConversationID)
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.sqlite"))
        let original = try XCTUnwrap(port.loadConversationMetadata(owner))
        let winner = try XCTUnwrap(model.configuredModelOptions.first)
        let loser = try XCTUnwrap(model.configuredModelOptions.last)
        let patch = String(decoding: try JSONSerialization.data(withJSONObject: ["modelSelectionID": winner.id]), as: UTF8.self)
        _ = try port.patchConversationConfiguration(.init(conversationID: owner,
            expectedRevision: XCTUnwrap(original.configurationRevision), patchJSON: patch, updatedAtMs: 1)).committedSnapshot()
        model.selectModel(loser)
        model.newConversation()
        let other = try XCTUnwrap(model.selectedConversationID)
        model.draft = "other conversation draft"
        let otherIdentity = model.selectedConversation?.modelSelectionID
        do {
            try await model.waitForModelConfigurationCommit(conversationID: owner)
            XCTFail("The stale edit must fail before recovery")
        } catch {}
        try await model.waitForModelConfigurationCommit(conversationID: owner)
        XCTAssertEqual(model.selectedConversationID, other)
        XCTAssertEqual(model.draft, "other conversation draft")
        XCTAssertEqual(model.selectedConversation?.modelSelectionID, otherIdentity)
        model.selectedConversationID = owner
        XCTAssertEqual(model.selectedConversation?.modelSelectionID, winner.id)
        XCTAssertEqual(model.selectedConversation?.configurationRevision, try XCTUnwrap(original.configurationRevision) + 1)
    }
    func testHostAndProfileIdentityIsCanonicalAndSeparateFromLocalNames() {
        let option = MacChatModelOption(source: .connectedHost, modelID: "shared-name",
            displayName: "Shared", configurationID: "profile:/中文", hostID: "host:/中文")
        let parsed = MacChatModelOption.parse(option.id)
        XCTAssertEqual(parsed?.source, .connectedHost)
        XCTAssertEqual(parsed?.configurationID, "profile:/中文")
        XCTAssertEqual(parsed?.hostID, "host:/中文")
        XCTAssertEqual(parsed?.id, option.id)
        XCTAssertFalse(option.id.contains("="))
        XCTAssertFalse(option.id.contains("/"))
        XCTAssertFalse(option.id.contains("+"))
        XCTAssertNotEqual(option.id, MacChatModelOption(source: .configured,
            modelID: "shared-name", displayName: "Shared", configurationID: "profile:/中文").id)
        XCTAssertNotEqual(option.id, MacChatModelOption(source: .connectedHost,
            modelID: "shared-name", displayName: "Shared", configurationID: "profile:/中文", hostID: "other-host").id)
        XCTAssertNil(MacChatModelOption.parse("connectedHost::"))
        XCTAssertNil(MacChatModelOption.parse("connectedHost:aG9zdA==:cHJvZmlsZQ=="))
        XCTAssertNil(MacChatModelOption.parse("connectedHost:aG9zdA:cHJvZmlsZQ:extra"))
    }

    func testConnectedHostSelectionDoesNotAliasSameNamedProfilesOrAnotherHost() async throws {
        let (root, model) = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: root) }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.updateRemoteHostConfigurationState(true)
        model.setFixtureRemoteModels([profile(id: "first"), profile(id: "second")])
        XCTAssertEqual(model.configuredModelOptions.count, 2)
        XCTAssertTrue(model.downloadedModelOptions.isEmpty)
        let second = try XCTUnwrap(model.configuredModelOptions.last)
        model.selectModel(second)
        XCTAssertTrue(model.selectedModelConfigurationPending)
        model.draft = "retained while saving"
        XCTAssertFalse(model.selectedComposerState.canSend)
        model.send()
        XCTAssertFalse(model.isRunning)
        XCTAssertEqual(model.draft, "retained while saving")
        XCTAssertThrowsError(try model.remoteModelIntentForSubmission(tier: .balanced))
        try await model.waitForModelConfigurationCommit()
        XCTAssertEqual(model.selectedRemoteModelProfile?.profileID, "second")
        XCTAssertNil(model.selectedRemoteConfiguration, "Host profiles must never resolve against the local model center.")

        model.setFixtureRemoteModels([profile(id: "first")])
        XCTAssertNil(model.selectedModelOption, "Removed explicit profile must not silently fall back by name.")
        XCTAssertThrowsError(try model.remoteModelIntentForSubmission(tier: .balanced))
        model.setFixtureRemoteModels([profile(id: "second", host: "other-host")])
        XCTAssertNil(model.selectedModelOption, "Same profile ID on another Host is a different selection.")
        model.updateRemoteHostConfigurationState(false)
        XCTAssertTrue(model.configuredModelOptions.isEmpty)
    }

    func testSubmissionFreezesHostRevisionTierAndOnlyExplicitConversationOverrides() async throws {
        let (root, model) = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: root) }
        model.executionHost = .remote
        try await model.waitForModelConfigurationCommit()
        model.setFixtureRemoteModels([profile(id: "profile", revision: 7)])
        model.selectModel(try XCTUnwrap(model.configuredModelOptions.first))
        try await model.waitForModelConfigurationCommit()
        let inherited = try XCTUnwrap(model.remoteModelIntentForSubmission(tier: .balanced))
        XCTAssertTrue((try inherited.payload()["overrides"] as? [String: Any])?.isEmpty == true)
        var overrides = MacConversationModelOverride()
        overrides.temperature = 0
        overrides.seed = .value(0)
        model.conversationModelOverride = overrides
        try await model.waitForModelConfigurationCommit()
        let frozen = try XCTUnwrap(model.remoteModelIntentForSubmission(tier: .concise))

        overrides.temperature = 1.8
        model.conversationModelOverride = overrides
        try await model.waitForModelConfigurationCommit()
        model.setFixtureRemoteModels([profile(id: "profile", revision: 8)])
        model.style = .detailed
        try await model.waitForModelConfigurationCommit()
        let payload = try frozen.payload()
        XCTAssertEqual(payload["profile_id"] as? String, "profile")
        XCTAssertEqual(payload["expected_revision"] as? UInt64, 7)
        XCTAssertEqual(payload["tier"] as? String, "concise")
        let fields = try XCTUnwrap(payload["overrides"] as? [String: Any])
        XCTAssertEqual(fields["temperature"] as? Double, 0)
        XCTAssertEqual((fields["generation_options"] as? [String: Any])?["seed"] as? Int, 0)
        XCTAssertNil(fields["top_p"])
        XCTAssertNil(fields["max_output_tokens"])
        XCTAssertNil(fields["model_protocol"])
        XCTAssertNil(fields["reasoning"])
        XCTAssertEqual(try model.remoteModelIntentForSubmission(tier: model.style)?.expectedRevision, 8)
        model.executionHost = .local
        try await model.waitForModelConfigurationCommit()
        XCTAssertNil(try model.remoteModelIntentForSubmission(tier: .balanced))
    }

    private func profile(id: String, host: String = "host", revision: UInt64 = 1) -> MacRemoteModelProfile {
        MacRemoteModelProfile(hostID: host, profileID: id, revision: revision,
            displayName: "Same model", modelName: "same-model")
    }

    private func makeWorkspace() throws -> (URL, MacWorkspaceModel) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-host-model-selection-\(UUID().uuidString)", isDirectory: true)
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.sqlite"))
        )
        let conversation = MacConversation(title: "Remote test", preview: "", updatedAt: .now, isPinned: false)
        try repository.restoreBackupSnapshot([MacStoredConversation(conversation: conversation, messages: [])])
        let model = MacWorkspaceModel(initialConversationID: conversation.id, canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost(capturedHostID: "host"))
        // This fixture exercises configuration persistence/admission without a
        // presented window. Otherwise the selection lifecycle's deferred task
        // starts live Host observation as soon as an awaited commit yields.
        // The Host double admits explicit selection writes, while an actual
        // send or unrelated observation must still fail.
        model.presentationDidDetach()
        return (root, model)
    }
}

final class MacSiwcAccountBridgeTests: XCTestCase {
    private nonisolated static func read(_ file: FileHandle) throws -> [String: Any] {
        var bytes = Data()
        while let byte = try file.read(upToCount: 1), !byte.isEmpty {
            if byte.first == 10 { return try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any]) }
            bytes.append(byte)
        }
        throw NSError(domain: "fixture", code: 1)
    }
    private nonisolated static func write(_ frame: [String: Any], _ file: FileHandle) throws {
        var bytes = try JSONSerialization.data(withJSONObject: frame); bytes.append(10)
        try file.write(contentsOf: bytes)
    }
    func testPrivatePipeBindsOwnerAndNeverAcceptsForeignFlowOrAuthorizationURL() async throws {
        for variant in ["valid", "owner", "url"] {
            let input = Pipe(), output = Pipe()
            let bridge = MacSiwcAccountBridge(fixtureInput: input.fileHandleForWriting, fixtureOutput: output.fileHandleForReading)
            defer { bridge.close() }
            let owner = UUID().uuidString.lowercased(), account = UUID().uuidString.lowercased()
            let reader = input.fileHandleForReading, writer = output.fileHandleForWriting
            let worker = Task.detached {
                let hello = try Self.read(reader)
                try Self.write(["type": "hello", "request_id": hello["request_id"]!, "owner_id": owner, "protocol": 1], writer)
                let request = try Self.read(reader)
                let body = try XCTUnwrap(request["request"] as? [String: Any])
                XCTAssertEqual(body["account_id"] as? String, account)
                XCTAssertEqual(body["acknowledge_authorization"] as? Bool, true)
                try Self.write(["type": "flow", "request_id": request["request_id"]!, "flow": [
                    "owner_id": variant == "owner" ? UUID().uuidString : owner,
                    "account_id": account, "flow_id": UUID().uuidString, "expected_account_revision": NSNull(),
                    "phase": "awaiting_authorization", "authorization_url": variant == "url" ? "https://evil.invalid/authorize" : "https://auth.openai.com/authorize?state=fixture",
                    "error": NSNull(), "private_session_may_be_retained": false,
                    "account": NSNull(), "inference_performed": false]], writer)
            }
            do {
                let flow = try await bridge.start(accountID: account, revision: nil, name: "Fixture")
                XCTAssertEqual(variant, "valid")
                XCTAssertEqual(flow.account_id, account)
            } catch { XCTAssertNotEqual(variant, "valid") }
            try await worker.value
        }
    }
    func testBrowserURLRejectsCredentialUserInfoAndNonHTTPS() {
        for url in ["http://auth.openai.com/authorize", "https://auth.openai.com.evil.invalid/authorize",
                    "https://user@auth.openai.com/authorize", "https://auth.openai.com:444/authorize"] {
            XCTAssertNil(MacSiwcAccountBridge.authorizationURL(url))
        }
    }
    func testErrorsUseFixedAllowlistWithoutEchoingSourceContent() {
        XCTAssertNotEqual(MacSiwcBridgeFailure.sanitized("native_siwc_refresh_result_unknown").localizedDescription,
            MacSiwcBridgeFailure.sanitized("native_siwc_core_commit_conflict_private_session_retained").localizedDescription)
        XCTAssertEqual(MacSiwcBridgeFailure.sanitized("token=private-provider-response").localizedDescription,
            MacSiwcBridgeFailure.unavailable.localizedDescription)
        XCTAssertFalse(MacSiwcBridgeFailure.sanitized("token=private-provider-response").localizedDescription.contains("token="))
    }
    func testManagementUsesExactAccountAndSessionRevisionWithoutInference() async throws {
        let input = Pipe(), output = Pipe()
        let bridge = MacSiwcAccountBridge(fixtureInput: input.fileHandleForWriting, fixtureOutput: output.fileHandleForReading)
        defer { bridge.close() }
        let id = UUID().uuidString.lowercased(), owner = UUID().uuidString.lowercased()
        let account = MacModelAccountSnapshot(revision: 7, account: .init(id: id, display_name: "Fixture",
            source: "nola_chatgpt", credential_ref: "siwc:" + id, state: "ready"))
        let reader = input.fileHandleForReading, writer = output.fileHandleForWriting
        let worker = Task.detached {
            let hello = try Self.read(reader)
            try Self.write(["type": "hello", "request_id": hello["request_id"]!, "owner_id": owner, "protocol": 1], writer)
            let frame = try Self.read(reader)
            XCTAssertEqual(frame["type"] as? String, "sign_out")
            let body = try XCTUnwrap(frame["request"] as? [String: Any])
            XCTAssertEqual(body["account_id"] as? String, id)
            XCTAssertEqual(body["expected_account_revision"] as? Int, 7)
            XCTAssertEqual(body["expected_session_revision"] as? Int, 4)
            XCTAssertEqual(body["acknowledge_sign_out"] as? Bool, true)
            let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(account))
            try Self.write(["type": "account", "request_id": frame["request_id"]!, "result": [
                "owner_id": owner, "account_id": id, "account": value, "connection": NSNull(),
                "session": ["account_id": id, "session_revision": 5, "signed_in": false,
                    "plan_usage_authorized": false, "reauthorization_required": true, "revocation_unconfirmed": true],
                "inference_performed": false]], writer)
        }
        let result = try await bridge.manage("sign_out", account: account, sessionRevision: 4, acknowledgeSignOut: true)
        XCTAssertEqual(result.session?.signed_in, false)
        XCTAssertEqual(result.session?.revocation_unconfirmed, true)
        try await worker.value
    }
    func testCatalogUsesFreshRevisionsAndRejectsForeignOrUnsupportedConnections() async throws {
        for variant in ["valid", "foreign", "version"] {
            let input = Pipe(), output = Pipe()
            let bridge = MacSiwcAccountBridge(fixtureInput: input.fileHandleForWriting, fixtureOutput: output.fileHandleForReading)
            defer { bridge.close() }
            let id = UUID().uuidString.lowercased(), owner = UUID().uuidString.lowercased()
            let account = MacModelAccountSnapshot(revision: 7, account: .init(id: id, display_name: "Fixture",
                source: "nola_chatgpt", credential_ref: "siwc:" + id, state: "ready"))
            let updated = MacModelAccountSnapshot(revision: 8, account: account.account)
            let reader = input.fileHandleForReading, writer = output.fileHandleForWriting
            let worker = Task.detached {
                let hello = try Self.read(reader)
                try Self.write(["type": "hello", "request_id": hello["request_id"]!, "owner_id": owner, "protocol": 1], writer)
                for action in ["account_status", "models"] {
                    let frame = try Self.read(reader)
                    XCTAssertEqual(frame["type"] as? String, action)
                    let body = try XCTUnwrap(frame["request"] as? [String: Any])
                    XCTAssertEqual(body["expected_account_revision"] as? Int, action == "models" ? 8 : 7)
                    if action == "models" { XCTAssertEqual(body["expected_session_revision"] as? Int, 11) }
                    else { XCTAssertTrue(body["expected_session_revision"] is NSNull) }
                    let connection: [String: Any] = ["revision": 1, "document": ["format_version": variant == "version" ? 2 : 1,
                        "connection": ["id": "siwc-" + id, "display_name": "Fixture", "base_url": "https://chatgpt.com",
                            "protocols": ["siwc_responses"], "account_id": variant == "foreign" ? UUID().uuidString : id,
                            "source": "nola_chatgpt", "enabled": false]]]
                    try Self.write(["type": "account", "request_id": frame["request_id"]!, "result": [
                        "owner_id": owner, "account_id": id,
                        "account": try JSONSerialization.jsonObject(with: JSONEncoder().encode(updated)),
                        "session": ["account_id": id, "session_revision": 11, "signed_in": true,
                            "plan_usage_authorized": true, "reauthorization_required": false, "revocation_unconfirmed": false],
                        "connection": action == "models" ? connection as Any : NSNull(), "inference_performed": false]], writer)
                }
            }
            do {
                let connectionID = try await bridge.discoverModels(account: account)
                XCTAssertEqual(variant, "valid")
                XCTAssertEqual(connectionID, "siwc-" + id)
            } catch { XCTAssertNotEqual(variant, "valid") }
            try await worker.value
        }
    }
}

private nonisolated final class DelayedSiwcFixture: MacSiwcAccountTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<MacSiwcFlow, Error>?
    private var selected = ""
    func start(accountID: String, revision: Int64?, name: String) async throws -> MacSiwcFlow {
        try await withCheckedThrowingContinuation { value in
            lock.lock(); selected = accountID; continuation = value; lock.unlock()
        }
    }
    func update(_ action: String) async throws -> MacSiwcFlow { throw NSError(domain: "Unexpected automatic action", code: 1) }
    func discoverModels(account: MacModelAccountSnapshot) async throws -> String { throw NSError(domain: "Unexpected discovery", code: 1) }
    func close() {}
    func hasStarted() -> Bool { lock.lock(); defer { lock.unlock() }; return continuation != nil }
    func complete() -> Bool {
        lock.lock(); let pending = continuation; continuation = nil; let account = selected; lock.unlock()
        guard let pending else { return false }
        pending.resume(returning: MacSiwcFlow(owner_id: UUID().uuidString, flow_id: UUID().uuidString,
            account_id: account, expected_account_revision: nil, phase: "authorized", authorization_url: nil,
            error: nil, private_session_may_be_retained: false, account: nil, inference_performed: false))
        return true
    }
}

private nonisolated final class GuidedSiwcFixture: MacSiwcAccountTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var accountID = ""
    private var states = ["awaiting_authorization", "awaiting_authorization", "authorized"]
    private var actions: [String] = []
    var failSave = false
    var failDiscovery = false
    var startWithoutURL = false
    var finishPhase = "committed"
    var startFailure: MacSiwcBridgeFailure?
    func calls() -> [String] { lock.withLock { actions } }
    func start(accountID: String, revision: Int64?, name: String) async throws -> MacSiwcFlow {
        lock.withLock { self.accountID = accountID; actions.append("start") }
        if let startFailure { throw startFailure }
        return value("awaiting_authorization", includeURL: !startWithoutURL)
    }
    func update(_ action: String) async throws -> MacSiwcFlow {
        let phase = lock.withLock {
            actions.append(action)
            return action == "status" ? (states.isEmpty ? "authorized" : states.removeFirst()) : action == "cancel" ? "cancelled" : finishPhase
        }
        if action == "finish", failSave { throw MacSiwcBridgeFailure.privateSessionRetained }
        return value(phase)
    }
    func discoverModels(account: MacModelAccountSnapshot) async throws -> String {
        lock.withLock { actions.append("models") }
        if failDiscovery { throw MacSiwcBridgeFailure.authorizationNetwork }
        return "fixture-connection"
    }
    func close() {}
    private func value(_ phase: String, includeURL: Bool = true) -> MacSiwcFlow {
        let id = lock.withLock { accountID }
        return MacSiwcFlow(owner_id: "fixture-owner", flow_id: "fixture-flow", account_id: id,
            expected_account_revision: nil, phase: phase,
            authorization_url: phase == "awaiting_authorization" && includeURL ? "https://auth.openai.com/authorize?state=fixture" : nil,
            error: nil, private_session_may_be_retained: false,
            account: phase == "committed" ? MacModelAccountSnapshot(revision: 1,
                account: .init(id: id, display_name: "Fixture", source: "nola_chatgpt", credential_ref: "siwc:" + id, state: "ready")) : nil,
            inference_performed: false)
    }
}

@MainActor final class MacSiwcSignInModelTests: XCTestCase {
    private func waitUntilIdle(_ model: MacSiwcSignInModel) async throws {
        for _ in 0..<300 {
            if !model.running { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Sign-in did not settle")
    }
    func testSignInDismissalWaitsForCancellationAcknowledgement() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture()
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in true },
            pollInterval: .seconds(30), makeBridge: { _ in transport })
        XCTAssertFalse(model.requiresConfirmedDismissal)
        model.begin(application: app, name: "Fixture")
        XCTAssertTrue(model.requiresConfirmedDismissal)
        for _ in 0..<100 {
            if model.flow?.phase == "awaiting_authorization" { break }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertEqual(model.flow?.phase, "awaiting_authorization")
        XCTAssertTrue(model.requiresConfirmedDismissal)
        model.cancel()
        XCTAssertTrue(model.requiresConfirmedDismissal, "Closing must wait for the private bridge acknowledgement")
        try await waitUntilIdle(model)
        XCTAssertTrue(model.cancellationConfirmed)
        XCTAssertFalse(model.requiresConfirmedDismissal)
        XCTAssertEqual(transport.calls().filter { $0 == "cancel" }.count, 1)
        XCTAssertFalse(transport.calls().contains("finish"))
        model.close()
    }

    func testGuidedSignInOpensBrowserOnceSavesOnceAndLoadsModelsOnExplicitChoice() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture()
        var opened: [URL] = []
        let model = MacSiwcSignInModel(account: nil, openBrowser: { opened.append($0); return true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        XCTAssertEqual(opened.count, 1, "status polling must not reopen browser tabs")
        XCTAssertEqual(opened.first?.host, "auth.openai.com")
        XCTAssertEqual(model.flow?.phase, "committed")
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        XCTAssertFalse(transport.calls().contains("models"), "sign-in alone must not discover or select models")
        model.chooseModels()
        try await waitUntilIdle(model)
        XCTAssertEqual(model.connectionID, "fixture-connection")
        XCTAssertEqual(transport.calls().filter { $0 == "models" }.count, 1)
        model.close()
    }
    func testBrowserFailureAllowsExplicitReopenAndSaveFailureIsNotRetriedAutomatically() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.failSave = true
        var attempts = 0
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in attempts += 1; return attempts > 1 },
            pollInterval: .milliseconds(50), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        for _ in 0..<100 {
            if model.browserOpenFailed { break }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertTrue(model.browserOpenFailed)
        model.openAuthorizationPage()
        XCTAssertFalse(model.browserOpenFailed)
        try await waitUntilIdle(model)
        XCTAssertEqual(attempts, 2)
        XCTAssertNotNil(model.error)
        XCTAssertEqual(model.flow?.phase, "authorized")
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        model.close()
    }
    func testDiscoveryFailureKeepsSavedAccountAndDoesNotRestartAuthorization() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.failDiscovery = true
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        model.chooseModels()
        try await waitUntilIdle(model)
        XCTAssertEqual(model.flow?.phase, "committed")
        XCTAssertNotNil(model.error)
        XCTAssertNil(model.connectionID)
        XCTAssertEqual(transport.calls().filter { $0 == "start" }.count, 1)
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        model.close()
    }
    func testCatalogRetryCreatesFreshTransportWithoutRepeatingSignIn() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let signIn = GuidedSiwcFixture(), failedCatalog = GuidedSiwcFixture(), recoveredCatalog = GuidedSiwcFixture()
        failedCatalog.failDiscovery = true
        var created = 0
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in true },
            pollInterval: .milliseconds(1), makeBridge: { _ in
                created += 1
                return created == 1 ? signIn : created == 2 ? failedCatalog : recoveredCatalog
            })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        model.chooseModels(); try await waitUntilIdle(model)
        XCTAssertNotNil(model.error)
        XCTAssertEqual(model.flow?.phase, "committed")
        model.chooseModels(); try await waitUntilIdle(model)
        XCTAssertNil(model.error)
        XCTAssertEqual(model.connectionID, "fixture-connection")
        XCTAssertEqual(created, 3)
        XCTAssertEqual(signIn.calls().filter { $0 == "start" }.count, 1)
        XCTAssertFalse(recoveredCatalog.calls().contains("start"))
        model.close()
    }
    func testDelayedBrowserLinkStillOpensAndUnconfirmedSaveNeverShowsSuccess() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.startWithoutURL = true; transport.finishPhase = "authorized"
        var opened = 0
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in opened += 1; return true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        XCTAssertEqual(opened, 1)
        XCTAssertEqual(model.flow?.phase, "authorized")
        XCTAssertNotNil(model.error)
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        model.close()
    }
    func testUncertainStartRequiresAccountReviewInsteadOfBlindRetry() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.startFailure = .unavailable
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in XCTFail("No verified browser link"); return false },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        XCTAssertFalse(model.retrySignInAllowed)
        XCTAssertNotNil(model.error)
        XCTAssertNil(model.flow)
        model.close()
    }
    func testRealSignInPrimaryActionsConnectThenOpenModelSelection() async throws {
        let previousPolicy = NSApp.activationPolicy()
        NSApp.setActivationPolicy(.regular)
        defer { NSApp.setActivationPolicy(previousPolicy) }
        NSApp.activate()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture()
        var browserCount = 0, selected: String?
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in browserCount += 1; return true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        let controller = NSHostingController(rootView: MacSiwcSignInView(application: app, account: nil, model: model, saved: { selected = $0 }))
        let view = controller.view
        view.frame = NSRect(x: 0, y: 0, width: 460, height: 600)
        let window = NSWindow(contentRect: view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = controller; window.makeKeyAndOrderFront(nil)
        defer { model.close(); window.orderOut(nil); window.contentViewController = nil; window.close() }
        func button(_ identifier: String) -> AnyObject? {
            var queue: [AnyObject] = [window, view], visited = Set<ObjectIdentifier>()
            while !queue.isEmpty && visited.count < 4000 {
                let element = queue.removeFirst()
                guard visited.insert(ObjectIdentifier(element)).inserted else { continue }
                let label = identifier == "mac-chatgpt-sign-in"
                    ? String(localized: "Sign in with ChatGPT", table: "ModelCenter")
                    : String(localized: "Choose models", table: "ModelCenter")
                let role = (element.accessibilityRole?() ?? nil)?.rawValue
                if element.accessibilityIdentifier?() == identifier
                    || (role == "AXButton" && (element.accessibilityLabel?() == label || element.accessibilityTitle?() == label)) { return element }
                let children: [Any] = (element.accessibilityChildren?() ?? nil) ?? []
                queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
                if let v = element as? NSView { queue.append(contentsOf: v.subviews) }
            }
            return nil
        }
        func pressPrimary(_ identifier: String) throws {
            if let element = button(identifier) {
                XCTAssertEqual(element.accessibilityPerformPress?(), true)
                return
            }
            // AppKit may defer SwiftUI's AX descendants until an external AX
            // client connects. Exercise the real default button through its
            // keyboard equivalent in that case, never call the model action.
            let enter = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: "\r",
                charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
            if !window.performKeyEquivalent(with: enter) { window.sendEvent(enter) }
        }
        for _ in 0..<4 {
            view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(30)); await Task.yield()
        }
        try pressPrimary("mac-chatgpt-sign-in")
        await Task.yield()
        try await waitUntilIdle(model)
        for _ in 0..<4 {
            view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(30)); await Task.yield()
        }
        XCTAssertEqual(browserCount, 1)
        XCTAssertEqual(model.flow?.phase, "committed")
        try pressPrimary("mac-chatgpt-choose-models")
        await Task.yield()
        try await waitUntilIdle(model)
        for _ in 0..<30 { if selected != nil { break }; try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(selected, "fixture-connection", "The next action must open model selection, not return to account settings")
    }
    func testSignInPagesFitBothAppearancesWithReadableText() async throws {
        let previousPolicy = NSApp.activationPolicy()
        NSApp.setActivationPolicy(.regular)
        defer { NSApp.setActivationPolicy(previousPolicy) }
        NSApp.activate()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-signin-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let captures = FileManager.default.temporaryDirectory.appendingPathComponent("NolaInteractionScreenshots", isDirectory: true)
        try FileManager.default.createDirectory(at: captures, withIntermediateDirectories: true)
        for dark in [false, true] {
            for phase in ["initial", "awaiting_authorization", "committed", "failed"] {
                let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in XCTFail("Rendering must not open a browser"); return false })
                if phase != "initial" {
                    model.flow = MacSiwcFlow(owner_id: "fixture", flow_id: "fixture", account_id: "fixture", expected_account_revision: nil,
                        phase: phase, authorization_url: phase == "awaiting_authorization" ? "https://auth.openai.com/authorize?state=fixture" : nil,
                        error: nil, private_session_may_be_retained: false, account: nil, inference_performed: false)
                }
                model.browserOpenFailed = phase == "awaiting_authorization"
                if phase == "failed" { model.error = MacSiwcBridgeFailure.authorizationNetwork.localizedDescription }
                let view = NSHostingView(rootView: MacSiwcSignInView(application: app, account: nil, model: model)
                    .environment(\.colorScheme, dark ? .dark : .light)
                    .environment(\.dynamicTypeSize, .accessibility1)
                    .background(Color(nsColor: .windowBackgroundColor)))
                let size = view.fittingSize
                XCTAssertEqual(size.width, 460, accuracy: 1)
                XCTAssertLessThanOrEqual(size.height, 650, "The sheet must fit a compact desktop window: \(phase)")
                view.frame = NSRect(origin: .zero, size: size)
                let window = NSWindow(contentRect: view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false; window.contentView = view
                window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                window.makeKeyAndOrderFront(nil)
                view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
                try await Task.sleep(for: .milliseconds(100))
                if let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                    view.cacheDisplay(in: view.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                    try data.write(to: captures.appendingPathComponent("signin-\(phase)-\(dark ? "dark" : "light").png"))
                } else { XCTFail("Could not render the sign-in page") }
                window.orderOut(nil); window.contentView = nil; window.close(); model.close()
            }
        }
    }
    func testClosingOwnerRejectsLateAuthorizationWithoutFinishing() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = DelayedSiwcFixture()
        let model = MacSiwcSignInModel(account: nil, makeBridge: { _ in transport })
        model.begin(application: application, name: "Fixture")
        // Let start suspend on the actual async boundary, then invalidate the owner.
        for _ in 0..<100 { if transport.hasStarted() { break }; await Task.yield() }
        XCTAssertTrue(transport.hasStarted())
        model.close()
        XCTAssertFalse(model.running)
        for _ in 0..<100 {
            if transport.complete() { break }
            await Task.yield()
        }
        await Task.yield()
        XCTAssertNil(model.flow)
        XCTAssertNil(model.error)
    }
}

@MainActor final class MacSiwcProfileAdmissionTests: XCTestCase {
    nonisolated final class Reads: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        func read() -> String? { lock.lock(); defer { lock.unlock() }; count += 1; return nil }
        var total: Int { lock.lock(); defer { lock.unlock() }; return count }
    }
    func testSiwcProfileFreezesRevisionAndNeverReadsAPIKeyOrAddsFallback() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let service = MacModelConnectionService(application: port)
        let accountID = UUID().uuidString.lowercased()
        let account: MacModelAccountSnapshot = try service.command(["type": .string("save_account"), "input": .object([
            "id": .string(accountID), "display_name": .string("Fixture"), "source": .string("nola_chatgpt"),
            "credential_ref": .string("siwc:" + accountID), "state": .string("ready")]),
            "expected_revision": .null, "updated_at_ms": .signedInteger(1)])
        let connection: MacModelConnectionSnapshot = try service.command(["type": .string("save_connection"), "input": .object([
            "id": .string("siwc-" + accountID), "display_name": .string("Fixture"), "base_url": .string("https://api.openai.com/v1"),
            "protocols": .array([.string("siwc_responses")]), "account_id": .string(account.id),
            "source": .string("nola_chatgpt"), "source_id": .string(account.id), "enabled": .bool(true)]),
            "expected_revision": .null, "updated_at_ms": .signedInteger(1)])
        let catalog: MacModelConnectionSnapshot = try service.command(["type": .string("refresh_connection_catalog"),
            "connection_id": .string(connection.id), "expected_revision": .signedInteger(connection.revision),
            "models": .array([.object(["model_id": .string("fixture-model"), "display_name": .string("Fixture Model"),
                "protocol": .string("siwc_responses"), "context_window": .null, "output_limit": .null,
                "modalities": .array([]), "reasoning_levels": .array([.string("low"), .string("high"), .string("ultra")]), "execution_capabilities": .null])]),
            "observed_at_ms": .signedInteger(2)])
        try service.enable(catalog, modelIDs: ["siwc_responses:fixture-model"])
        let profile = try XCTUnwrap(try port.listModelProfiles().first)
        let reads = Reads()
        let store = MacAppleChatConfigurationStore(applicationPort: port, credentialReader: { _ in reads.read() })
        let configuration = try XCTUnwrap(store.loadThrowing(model: profile.id))
        XCTAssertEqual(try XCTUnwrap(configuration.reasoning).efforts, ["low", "high"])
        XCTAssertEqual(configuration.reasoning?.responses, true)
        let selected = try XCTUnwrap(configuration.siwcProfile)
        XCTAssertEqual(selected.revision, profile.revision)
        XCTAssertEqual(configuration.wireProtocol, .siwcResponses)
        let accountBinding: MacCanonicalJSONValue = .object([
            "account_id": .string(account.id), "account_revision": .signedInteger(account.revision),
            "credential_ref": .string("siwc:" + account.id), "client_id": .string("fixture-client"),
            "subject": .string("fixture-subject")])
        let frozen = AppleFrozenResendModel<MacCanonicalJSONValue>(version: 1, primary: .object([
            "kind": .string("http"), "endpoint": .string("https://api.openai.com/v1"),
            "endpoint_query_sha256": .null, "protocol": .string("siwc_responses"),
            "model": .string(configuration.model), "siwc_account": accountBinding,
            "credential_sha256": .string(MacRunModelCredential.digest("")), "output_token_field": .string("omitted")]),
            fallbacks: .array([]), generation: .object(["prompt_cache_key_sha256": .string(String(repeating: "a", count: 64)),
                "prompt_cache_key_prefix_sha256": .null]), output_admission: .object([:]), model_capability: .object([:]))
        let resend = try MacEditedResendModelResolver.resolve(frozen, application: port)
        XCTAssertEqual(resend.siwcProfile, selected)
        XCTAssertEqual(resend.profileID, profile.id)
        XCTAssertTrue(resend.fallbacks.isEmpty)
        XCTAssertNil(resend.nativeRevision)
        let plan = try XCTUnwrap(store.routePlanThrowing(preferredModel: profile.id))
        XCTAssertTrue(plan.fallbacks.isEmpty)
        let request = try MacLocalAgentRunRequest(conversationID: "fixture-room", prompt: "fixture",
            projectURL: root, attachmentURLs: [], modelID: configuration.model, modelProfileID: profile.id,
            siwcProfile: selected, endpoint: configuration.baseURL.absoluteString,
            modelExecutionCapabilities: configuration.executionCapabilities, wireProtocol: .siwcResponses,
            memoryEnabled: false, databaseURLOverride: port.databaseURL, applicationPort: port)
        let captured = try MacLocalAgentCredentialGate.capture(request, store: store)
        XCTAssertTrue(captured.credentials.isEmpty)
        XCTAssertEqual(reads.total, 0)
        let runtime = MacAgentHostRuntime(credentialStore: store)
        let data = try await runtime.initialRequestData(for: request)
        let wire = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(wire["model_profile_id"] as? String, profile.id)
        XCTAssertEqual(wire["expected_profile_revision"] as? Int64, profile.revision)
        XCTAssertNil(wire["api_key"])
        XCTAssertEqual((wire["mac_launch"] as? [String: Any])?["model_profile_revision"] as? Int64, profile.revision)
        let resumed = try MacLocalAgentOwnedLaunchSnapshot.resumeRequestData(original: data,
            conversationID: "fixture-room", runID: request.submissionID, assistantMessageID: request.assistantMessageID)
        let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: resumed) as? [String: Any])
        XCTAssertEqual((frame["configuration"] as? [String: Any])?["expected_profile_revision"] as? Int64, profile.revision)
        let arguments = await runtime.arguments(for: request, localEndpoint: nil)
        XCTAssertFalse(arguments.contains("--model")); XCTAssertFalse(arguments.contains("--endpoint"))
        // A changed profile must not silently recapture a newer selection.
        // Managed profiles can only change through their owning connection.
        let currentConnection = try XCTUnwrap(service.connections().first { $0.id == connection.id })
        _ = try service.disconnect(currentConnection)
        XCTAssertThrowsError(try MacLocalAgentCredentialGate.launch(selection: captured, isResume: true,
            store: store, operation: { XCTFail("Changed profile must not dispatch") }))
        XCTAssertThrowsError(try MacEditedResendModelResolver.resolve(frozen, application: port),
            "An edited resend must not reuse a disconnected account/model")
        XCTAssertEqual(reads.total, 0)
    }
}
