// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Tests that the iOS Mermaid renderer produces a native image.
// - Interfaces:
//   - ChatMermaidRendererTests
// - APIs:
//   - none
//
import BeautifulMermaid
import XCTest
@testable import Nola

@MainActor
final class ChatMermaidRendererTests: XCTestCase {
    func testNativeMermaidRendererProducesImage() throws {
        let source = """
        graph TD
          A[开始] --> B{是否可用}
          B -->|是| C[显示 Mermaid 图]
          B -->|否| D[显示源码 fallback]
        """

        let graph = try MermaidRenderer.parse(source)
        XCTAssertEqual(graph.type, .flowchart)

        let image = try XCTUnwrap(MermaidRenderer.renderImage(source: source, theme: .zincLight.withTransparent(true)))
        XCTAssertGreaterThan(image.size.width, 0)
        XCTAssertGreaterThan(image.size.height, 0)
    }
}
