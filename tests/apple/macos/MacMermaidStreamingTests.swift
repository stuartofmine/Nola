// SPDX-License-Identifier: Apache-2.0
import AppKit
import BeautifulMermaid
import Combine
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacMermaidStreamingTests: XCTestCase {
    func testPartialDiagramHasStablePixelsAndSealedDiagramSurvivesLaterProse() async throws {
        for dark in [false, true] {
            let state = DiagramStreamState(dark: dark)
            state.append("```mermaid\n")
            let host = NSHostingView(rootView: DiagramStreamProbe(state: state))
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 520),
                styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await Task.sleep(for: .milliseconds(150))
            host.layoutSubtreeIfNeeded()
            let pendingPixels = try pixels(host)
            let source = "flowchart TD\n A[\"输入条件\"] --> B{\"是否满足？\"}\n B -->|是| C[\"继续执行\"]\n B -->|否| D[\"调整参数\"]\n"
            let characters = Array(source)
            for offset in stride(from: 0, to: characters.count, by: 4) {
                state.append(String(characters[offset..<min(offset + 4, characters.count)]))
                try await Task.sleep(for: .milliseconds(12))
                host.layoutSubtreeIfNeeded()
                XCTAssertTrue(mermaids(host).isEmpty, "An unfinished fence must never start parsing/layout")
                XCTAssertEqual(try pixels(host), pendingPixels, "Growing partial syntax must not switch between source, error and graph frames")
            }
            state.append("```\n")
            try await Task.sleep(for: .milliseconds(200))
            host.layoutSubtreeIfNeeded()
            let graph = try XCTUnwrap(mermaids(host).first)
            XCTAssertEqual(mermaids(host).count, 1)
            XCTAssertNil(graph.parseError)
            XCTAssertFalse(graph.diagramBounds.isEmpty)
            XCTAssertFalse(graph.bounds.isEmpty)
            var prepares = 0
            let existing = graph.mermaidLayer.onPrepareComplete
            graph.mermaidLayer.onPrepareComplete = { prepares += 1; existing?() }
            for prose in ["\n", "说明：", "条件分支已保留。", "\n\n接下来可以检查结果。"] {
                state.append(prose)
                try await Task.sleep(for: .milliseconds(30))
                host.layoutSubtreeIfNeeded()
                XCTAssertTrue(mermaids(host).first === graph)
                XCTAssertFalse(graph.bounds.isEmpty)
            }
            XCTAssertEqual(prepares, 0, "Appending unrelated prose must not reparse a sealed diagram")
            let previousBounds = graph.diagramBounds
            state.dark.toggle()
            try await Task.sleep(for: .milliseconds(100))
            host.layoutSubtreeIfNeeded()
            XCTAssertTrue(mermaids(host).first === graph)
            XCTAssertEqual(graph.diagramBounds, previousBounds)
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                URL(fileURLWithPath: "/tmp/nola-mermaid-stream-\(dark ? "dark" : "light").png"))
        }
    }

    func testMutableDiagramIdentityStaysTheSameWhenFenceCloses() {
        var projection = MacJournalMarkdownProjection(identitySeed: "diagram-seal")
        let open = "```mermaid\nflowchart LR\n A --> B\n"
        projection.update(open, isFinal: false)
        let pending = projection.presentation.mutableTextNode
        XCTAssertEqual(pending?.kind, .mermaid)
        XCTAssertTrue(projection.presentation.textNodes.isEmpty)
        projection.update(open + "```\n", isFinal: false)
        XCTAssertNil(projection.presentation.mutableTextNode)
        XCTAssertEqual(projection.presentation.textNodes.first?.id, pending?.id)
        XCTAssertEqual(projection.presentation.textNodes.first?.kind, .mermaid)
    }

    private func mermaids(_ root: NSView) -> [MermaidView] {
        (root as? MermaidView).map { [$0] } ?? root.subviews.flatMap(mermaids)
    }
    private func pixels(_ view: NSView) throws -> Data {
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        return Data(bytes: try XCTUnwrap(bitmap.bitmapData), count: bitmap.bytesPerRow * bitmap.pixelsHigh)
    }
}

@MainActor
private final class DiagramStreamState: ObservableObject {
    @Published var projection = MacJournalMarkdownProjection(identitySeed: "stream-diagram")
    @Published var dark: Bool
    init(dark: Bool) { self.dark = dark }
    func append(_ text: String) { projection.update(projection.text + text, isFinal: false) }
}

private struct DiagramStreamProbe: View {
    @ObservedObject var state: DiagramStreamState
    var body: some View {
        VStack {
            MacAssistantPresentationView(nodes: state.projection.presentation.renderedTextNodes,
                animateNewNodes: false, contentIsFinal: false,
                mutableNodeID: state.projection.presentation.mutableTextNode?.id)
            Spacer()
        }.padding(20).frame(width: 600, height: 520)
            .background(state.dark ? Color.black : Color.white)
            .environment(\.colorScheme, state.dark ? .dark : .light)
    }
}
