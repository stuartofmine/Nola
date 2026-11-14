// SPDX-License-Identifier: Apache-2.0
import AppKit
import BeautifulMermaid
import XCTest
@testable import Nola

@MainActor
final class MacDiagramPresentationTests: XCTestCase {
    // Five separate campaigns, not one continuous march or a geographic map.
    // Source: https://shandong-chorography.org/database/zzmj/section/9/article/9/
    func testCampaignTopologyRendersNativeDiagramInBothAppearances() throws {
        let fixture = """
        flowchart BT
            H["汉中：北伐前进基地"]
            H -->|228年春：主力| Q1["祁山、陇右"]
            H -.->|228年春：赵云、邓芝疑军| J["箕谷；声言由斜谷取郿"]
            Q1 -.->|街亭失守后撤军| R1["撤回汉中"]
            H -->|228年冬：出散关| C["围陈仓；粮尽撤军"]
            H -->|229年| W["攻取武都、阴平"]
            H -->|231年| Q4["再出祁山；粮尽撤军"]
            H -->|234年：出斜谷| Z["五丈原；诸葛亮病逝后撤军"]
        """
        let source = try ProcessInfo.processInfo.environment["NOLA_REAL_DIAGRAM_SOURCE"].map {
            try String(contentsOfFile: $0, encoding: .utf8)
        } ?? fixture
        XCTAssertEqual(try MermaidRenderer.parse(source).type, .flowchart)
        for (name, theme) in [("light", DiagramTheme.zincLight), ("dark", DiagramTheme.zincDark)] {
            // Exercise the AppKit view used by MacMarkdownView, including
            // its coordinate transform; the image-export helper is a different path.
            let view = MermaidView(frame: .zero)
            view.theme = theme
            view.source = source
            XCTAssertNil(view.parseError)
            XCTAssertGreaterThan(view.diagramBounds.width, 0)
            XCTAssertGreaterThan(view.diagramBounds.height, 0)
            view.frame = CGRect(origin: .zero, size: view.diagramBounds.size)
            let window = NSWindow(contentRect: view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = view
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(
                "nola-route-diagram-\(ProcessInfo.processInfo.processIdentifier)-\(name).png")
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
            print("DIAGRAM_FIXTURE_ARTIFACT=\(url.path)")
        }
    }
}
