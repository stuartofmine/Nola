// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

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
        let navigation = MacInspectorNavigationState()
        let revision = navigation.revealRevision
        for section in [MacChatInspector.InspectorTab.goal, .context, .progress, .execution] {
            navigation.setExpanded(section, true)
            XCTAssertEqual(navigation.selectedTab, section)
            XCTAssertEqual(navigation.revealRevision, revision, "Local disclosure must not request an automatic scroll")
        }
        navigation.setKnowledgeExpanded(.wiki, true)
        XCTAssertEqual(navigation.selectedTab, .context)
        XCTAssertEqual(navigation.revealRevision, revision)
        navigation.setExpanded(.context, false)
        XCTAssertFalse(navigation.expandedSections.contains(.context))
        XCTAssertTrue(navigation.expandedSections.contains(.goal))
        XCTAssertTrue(navigation.select(.context, isVisible: true))
        XCTAssertGreaterThan(navigation.revealRevision, revision)
        XCTAssertEqual(navigation.revealID, "context")
        XCTAssertEqual(MacChatInspector.InspectorTab.allCases,
            [.overview, .progress, .goal, .review, .artifacts, .context, .execution, .participants])
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
        navigation.expandedSections = []
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
        func header(_ section: MacChatInspector.InspectorTab) throws -> AnyObject {
            try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == "mac-inspector-section-" + section.rawValue })
        }
        for area in MacChatInspector.InspectorArea.allCases {
            navigation.selectedTab = area.sections[0]
            navigation.expandedSections = []
            try await Task.sleep(for: .milliseconds(100))
            let positions = try area.sections.map { try XCTUnwrap(header($0).accessibilityFrame?()).midY }
            XCTAssertEqual(positions, positions.sorted(by: >), "Each area owns its own section order")
            let visible = objects(host).compactMap { $0.accessibilityIdentifier?() ?? nil }
            for other in MacChatInspector.InspectorTab.allCases where other.area != area {
                XCTAssertFalse(visible.contains("mac-inspector-section-" + other.rawValue), "Other areas are not rows in the current area")
            }
            for dark in [false, true] {
                window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                navigation.setExpanded(area.sections[0], true)
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
        for section in [MacChatInspector.InspectorTab.goal, .context, .execution, .progress, .artifacts, .review, .participants, .overview] {
            navigation.selectedTab = section
            navigation.expandedSections = []
            try await Task.sleep(for: .milliseconds(100))
            let revision = navigation.revealRevision
            let control = try header(section)
            XCTAssertTrue(control.accessibilityPerformPress?() ?? false, section.rawValue)
            try await Task.sleep(for: .milliseconds(80))
            XCTAssertEqual(navigation.expandedSections, [section])
            XCTAssertEqual(navigation.selectedTab, section)
            let contents = objects(host)
            for (identifier, owningSection) in [("mac-inspector-workspace", MacChatInspector.InspectorTab.overview),
                                               ("mac-inspector-context-summary", .context), ("mac-inspector-browser", .execution)] {
                XCTAssertEqual(contents.contains { ($0.accessibilityIdentifier?() ?? nil) == identifier }, section == owningSection,
                    "\(identifier) belongs only in \(owningSection), not repeated in \(section)")
            }
            XCTAssertEqual(navigation.revealRevision, revision)
            XCTAssertTrue(try header(section).accessibilityPerformPress?() ?? false, section.rawValue)
            try await Task.sleep(for: .milliseconds(80))
            XCTAssertTrue(navigation.expandedSections.isEmpty)
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
        for dark in [false, true] { for width: CGFloat in [240, 340] { for resources in [false, true] {
            var opened: MacWorkspaceSection?
            let content = Group {
                if resources {
                    MacWorkspaceResourceNavigation(search: .constant(""), selection: .memory) { opened = $0 }
                } else {
                    MacConversationSidebar(model: model, selectedSection: .constant(.chat), selectConversation: { _ in },
                        loadParticipantNames: { _ in [] })
                }
            }.frame(width: width, height: 650).background(MacDesignColor.sidebar)
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
            XCTAssertEqual(settings.count, 1, "Each sidebar has exactly one settings entry")
            if !resources {
                XCTAssertEqual(controls.filter { ($0.accessibilityIdentifier?() ?? nil) == "mac-expert-library" }.count, 1)
                XCTAssertFalse(controls.contains { ($0.accessibilityIdentifier?() ?? nil)?.hasPrefix("expert.children.toggle.") == true },
                    "The conversation list must not contain expert history subtrees")
            }
            if resources {
                let destinations = controls.filter { ($0.accessibilityIdentifier?() ?? nil)?.hasPrefix("workspace.resource.") == true }
                XCTAssertEqual(destinations.count, 4)
                let models = try XCTUnwrap(destinations.first { ($0.accessibilityIdentifier?() ?? nil) == "workspace.resource.models" })
                XCTAssertTrue(models.accessibilityPerformPress?() ?? false)
                XCTAssertEqual(opened, .models)
                XCTAssertTrue(try XCTUnwrap(settings.first).accessibilityPerformPress?() ?? false)
                XCTAssertEqual(opened, .settings)
            }
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                output.appendingPathComponent("\(resources ? "resources" : "conversations")-\(Int(width))-\(dark ? "dark" : "light").png"))
        } } }
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
