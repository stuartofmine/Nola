// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import AppKit
@testable import Nola

@MainActor
final class MacInspectorInteractionTests: XCTestCase {
    func testInspectorAreasPartitionEveryToolAndRetainOtherAreaDisclosureState() {
        let state = MacInspectorNavigationState()
        let areas = MacChatInspector.InspectorArea.allCases
        XCTAssertEqual(areas.flatMap(\.sections), MacChatInspector.InspectorTab.allCases)
        XCTAssertEqual(Set(areas.flatMap(\.sections)).count, MacChatInspector.InspectorTab.allCases.count)
        state.setExpanded(.goal, true)
        _ = state.select(.artifacts, isVisible: true)
        XCTAssertEqual(state.selectedTab.area, .artifacts)
        XCTAssertTrue(state.expandedSections.contains(.goal))
        _ = state.select(.context, isVisible: true)
        XCTAssertEqual(state.selectedTab.area, .context)
        XCTAssertTrue(state.expandedSections.contains(.artifacts))
        XCTAssertTrue(state.expandedSections.contains(.goal))
    }

    func testOpeningSectionsKeepsOtherSectionsExpanded() {
        let state = MacInspectorNavigationState()
        state.selectedTab = .context
        state.selectedTab = .progress
        XCTAssertEqual(state.expandedSections, [.overview, .context, .progress])
        state.expandedSections.remove(.context)
        let revision = state.revealRevision
        state.selectedTab = .context
        XCTAssertTrue(state.expandedSections.contains(.context))
        XCTAssertGreaterThan(state.revealRevision, revision)
    }

    func testOpeningContextDoesNotOpenAnUnrequestedEditor() {
        let state = MacInspectorNavigationState(initialPane: "context")
        XCTAssertTrue(state.expandedKnowledge.isEmpty)
        state.knowledgePane = .points
        XCTAssertEqual(state.expandedKnowledge, [.points])
    }

    func testDirectKnowledgeEntryExpandsTheRequestedEditor() {
        let state = MacInspectorNavigationState(initialPane: "wiki")
        XCTAssertEqual(state.selectedTab, .context)
        XCTAssertTrue(state.expandedSections.contains(.context))
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
        let state = MacInspectorNavigationState(initialPane: "wiki")
        XCTAssertEqual(state.revealID, "knowledge-wiki")
        state.selectedTab = .execution
        XCTAssertEqual(state.revealID, "execution")
        state.knowledgePane = .recall
        XCTAssertEqual(state.selectedTab, .context)
        XCTAssertEqual(state.revealID, "knowledge-recall")
        XCTAssertTrue(state.expandedSections.contains(.execution))
    }

    func testToolbarReopensCollapsedSelectedSectionInsteadOfHidingInspector() {
        let state = MacInspectorNavigationState()
        state.expandedSections.remove(.overview)
        XCTAssertTrue(state.select(.overview, isVisible: true))
        XCTAssertTrue(state.expandedSections.contains(.overview))
        XCTAssertFalse(state.select(.overview, isVisible: true))
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
                    let navigation = MacInspectorNavigationState(initialPane: page)
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
