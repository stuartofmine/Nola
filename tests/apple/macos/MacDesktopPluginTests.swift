// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import WebKit
import XCTest
@testable import Nola

@MainActor
final class MacDesktopPluginTests: XCTestCase {
    func testInstalledPageOpensFromWorkspaceSidebarAndClosesWhenRemoved() async throws {
        let ax = try MacHostedAccessibilitySession()
        defer { ax.restore() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let client = MacDesktopPluginClient(environment: environment)
        let profile = environment.databaseURL.deletingPathExtension().appendingPathExtension("plugins")
            .appendingPathComponent("harness/dsh/profile/profiles/web")
        let realInstall = ProcessInfo.processInfo.environment["NOLA_TEST_REAL_DESKTOP_PLUGIN"] == "1"
        let package = realInstall ? root.appendingPathComponent("board-source") : profile.appendingPathComponent("node_modules/test-board")
        try FileManager.default.createDirectory(at: package, withIntermediateDirectories: true)
        let manifest = #"{"name":"test-board","version":"1.0.0","nola":{"pages":[{"id":"board","title":"Task board","entry":"board.html"},{"id":"archive","title":"Completed tasks","entry":"board.html"}]}}"#
        try manifest.write(to: package.appendingPathComponent("package.json"), atomically: true, encoding: .utf8)
        let content = """
        <!doctype html><meta name="color-scheme" content="light dark"><style>body{font:18px system-ui;padding:32px}button{font:inherit}</style>
        <h1>Task board</h1><button id="finish">Complete task</button>
        <script>
        const button=document.getElementById('finish');
        button.onclick=event=>{
            button.textContent='Task complete';
            let isolated=false;
            try { parent.document.title } catch {isolated=true}
            parent.postMessage({label:button.textContent,trusted:event.isTrusted,isolated},'*');
        };
        onmessage=()=>{
            const rect=button.getBoundingClientRect();
            parent.postMessage({x:rect.x+rect.width/2,y:rect.y+rect.height/2},'*');
        };
        </script>
        """
        try content.write(to: package.appendingPathComponent("board.html"), atomically: true, encoding: .utf8)
        let inventory = profile.appendingPathComponent("package.json")
        if realInstall {
            let result = try await client.manage(runtime: "dsh", profile: "web", action: "install", source: package.path)
            XCTAssertTrue(result.packages.contains("test-board"))
        } else {
            try #"{"dependencies":{"test-board":"1.0.0"}}"#.write(to: inventory, atomically: true, encoding: .utf8)
        }
        let pages = try await client.pages()
        let page = try XCTUnwrap(pages.first { $0.title == "Task board" })
        let document = try await client.document(page.id)
        XCTAssertEqual(document.html, content)

        let repo = MacCanonicalConversationRepository(store: MacApplicationPort.shared)
        let host = NSHostingView(rootView: MacChatWorkspaceSnapshotContent(canonicalRepository: repo)
            .environment(\.desktopPluginClient, client).environment(\.accessibilityEnabled, true))
        host.frame = NSRect(x: 0, y: 0, width: 1180, height: 760)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let identifier = "workspace.nav.plugin." + page.id
        try await wait { self.element(host, identifier) != nil }
        XCTAssertTrue(element(host, "workspace.nav.extensions")?.accessibilityPerformPress?() ?? false)
        try await wait { self.firstSegmented(host) != nil }
        let picker = try XCTUnwrap(firstSegmented(host))
        picker.selectedSegment = 3
        XCTAssertTrue(picker.sendAction(picker.action, to: picker.target))
        let entry = "workspace.plugins.open." + page.id
        try await wait { self.element(host, entry) != nil }
        let archive = try XCTUnwrap(pages.first { $0.title == "Completed tasks" })
        XCTAssertNotNil(element(host, "workspace.plugins.open." + archive.id))
        try await attachView(host, name: "Installed plugins in native Mac library")
        XCTAssertTrue(element(host, "workspace.plugins.add")?.accessibilityPerformPress?() ?? false)
        try await wait { window.attachedSheet?.contentView != nil }
        let sheet = try XCTUnwrap(window.attachedSheet?.contentView)
        try await wait { self.element(sheet, "workspace.plugins.source") != nil }
        if realInstall { XCTAssertNil(element(sheet, "workspace.plugins.missingRuntime")) }
        if let field = firstTextField(sheet) {
            sheet.window?.makeFirstResponder(field)
            let editor = try XCTUnwrap(field.currentEditor() as? NSTextView)
            editor.insertText("@example/task-board", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        } else { XCTFail("Add plugin should expose an editable native source field") }
        try await attachView(sheet.superview ?? sheet, name: "Add plugin on this Mac")
        XCTAssertTrue(element(sheet, "workspace.plugins.cancelAdd")?.accessibilityPerformPress?() ?? false)
        try await wait { window.attachedSheet == nil }
        XCTAssertTrue(element(host, "workspace.plugins.add")?.accessibilityPerformPress?() ?? false)
        try await wait { window.attachedSheet?.contentView != nil }
        let resumedSheet = try XCTUnwrap(window.attachedSheet?.contentView)
        try await wait { self.firstTextField(resumedSheet)?.stringValue == "@example/task-board" }
        XCTAssertTrue(element(resumedSheet, "workspace.plugins.cancelAdd")?.accessibilityPerformPress?() ?? false)
        try await wait { window.attachedSheet == nil }
        XCTAssertTrue(element(host, entry)?.accessibilityPerformPress?() ?? false)
        try await wait { self.webView(in: host) != nil }
        let web = try XCTUnwrap(webView(in: host))
        try await wait { !web.isLoading && web.url != nil }
        let center = try await web.callAsyncJavaScript("""
            const frame=document.querySelector('iframe');
            return await new Promise(resolve=>{
                const timer=setTimeout(()=>resolve(null),3000);
                onmessage=e=>{
                    if(e.source!==frame.contentWindow || e.origin!=='null')return;
                    if ('x' in e.data) {clearTimeout(timer);resolve(e.data)}
                    else {window.taskReceipt=e.data}
                };
                frame.contentWindow.postMessage('position','*');
            });
            """, arguments: [:], in: nil, contentWorld: .page) as? [String: Double]
        let x = try XCTUnwrap(center?["x"])
        let y = try XCTUnwrap(center?["y"])
        let point = web.convert(NSPoint(x: x, y: web.isFlipped ? y : web.bounds.height - y), to: nil)
        let hit = try XCTUnwrap(host.hitTest(host.convert(point, from: nil)))
        XCTAssertTrue(hit == web || hit.isDescendant(of: web), "Plugin hit test reached " + String(describing: type(of: hit)))
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: type, location: point, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber, context: nil,
                eventNumber: 0, clickCount: 1, pressure: 1))
            if type == .leftMouseDown { hit.mouseDown(with: event) } else { hit.mouseUp(with: event) }
        }
        let evidence = try await web.callAsyncJavaScript("""
            for(let i=0;i<40 && !window.taskReceipt;i++) await new Promise(r=>setTimeout(r,50));
            const receipt=window.taskReceipt;
            return receipt?.trusted===true && receipt?.isolated===true && receipt?.label==='Task complete';
            """, arguments: [:], in: nil, contentWorld: .page) as? Bool
        XCTAssertEqual(evidence, true, "Expected a trusted click at \(point) on \(type(of: hit)) in \(web.bounds), flipped: \(web.isFlipped)")
        let image = try await web.takeSnapshot(configuration: nil)
        let attachment = XCTAttachment(image: image)
        attachment.name = "Installed plugin page after completing a task"; attachment.lifetime = .keepAlways
        add(attachment)
        if let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: bitmap)
            if let data = bitmap.representation(using: .png, properties: [:]) {
                let screenshot = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
                screenshot.name = "Mac workspace with installed plugin sidebar"; screenshot.lifetime = .keepAlways
                add(screenshot)
            }
        }
        XCTAssertTrue(element(host, "workspace.nav.chat")?.accessibilityPerformPress?() ?? false)
        try await wait { self.webView(in: host) == nil }
        XCTAssertTrue(element(host, identifier)?.accessibilityPerformPress?() ?? false)
        try await wait { self.webView(in: host) != nil }
        if realInstall {
            let result = try await client.manage(runtime: "dsh", profile: "web", action: "remove", source: "test-board")
            XCTAssertFalse(result.packages.contains("test-board"))
            let receipt = XCTAttachment(string: "Installed and removed test-board through the embedded Host and the DSH Web profile.")
            receipt.name = "Real DSH package lifecycle"; receipt.lifetime = .keepAlways; add(receipt)
        } else {
            try #"{"dependencies":{}}"#.write(to: inventory, atomically: true, encoding: .utf8)
        }
        NotificationCenter.default.post(name: .macDesktopPluginsChanged, object: nil)
        try await wait { self.element(host, identifier) == nil && self.webView(in: host) == nil }
    }

    private func wait(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(12)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(50)) }
        if !condition() { throw NSError(domain: "MacDesktopPluginTests", code: 1, userInfo: [NSLocalizedDescriptionKey: "Workspace failed to reach the expected state."]) }
    }

    private func firstTextField(_ view: NSView) -> NSTextField? {
        if let field = view as? NSTextField, field.isEditable { return field }
        for child in view.subviews { if let field = firstTextField(child) { return field } }
        return nil
    }

    private func firstSegmented(_ view: NSView) -> NSSegmentedControl? {
        if let control = view as? NSSegmentedControl, control.segmentCount == 4 { return control }
        for child in view.subviews { if let control = firstSegmented(child) { return control } }
        return nil
    }

    private func attachView(_ host: NSView, name: String) async throws {
        // Wait for the native pane/sheet transition to present its final layer tree.
        try await Task.sleep(for: .milliseconds(550))
        host.window?.displayIfNeeded()
        host.layoutSubtreeIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return XCTFail("Missing native bitmap") }
        host.effectiveAppearance.performAsCurrentDrawingAppearance {
            host.cacheDisplay(in: host.bounds, to: bitmap)
        }
        guard let data = bitmap.representation(using: .png, properties: [:]) else { return XCTFail("Missing image") }
        let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
        attachment.name = name; attachment.lifetime = .keepAlways; add(attachment)
    }

    private func webView(in view: NSView) -> WKWebView? {
        if let web = view as? WKWebView { return web }
        for child in view.subviews { if let web = webView(in: child) { return web } }
        return nil
    }

    private func element(_ root: NSView, _ id: String) -> AnyObject? {
        var queue: [AnyObject] = [root], seen = Set<ObjectIdentifier>()
        while let value = queue.popLast() {
            guard seen.insert(ObjectIdentifier(value)).inserted else { continue }
            if (value.accessibilityIdentifier?() ?? nil) == id { return value }
            let children = value.accessibilityChildren?() ?? []
            queue += (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject }
            if let view = value as? NSView { queue += view.subviews }
        }
        return nil
    }
}
