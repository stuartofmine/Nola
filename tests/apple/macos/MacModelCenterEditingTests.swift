// SPDX-License-Identifier: Apache-2.0
import AppKit
import Combine
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacModelCenterEditingTests: XCTestCase {
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
