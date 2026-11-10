// SPDX-License-Identifier: Apache-2.0
// Window-local detail viewports; these values never enter the product Store.
import AppKit
import SwiftUI

@MainActor
final class MacDetailScrollPositions {
    struct PDFViewport: Equatable {
        let pageIndex: Int
        let point: NSPoint
        let scaleFactor: CGFloat
        let autoScales: Bool
    }
    private var positions: [String: NSPoint] = [:]
    private var pdfPositions: [String: PDFViewport] = [:]
    func position(for key: String) -> NSPoint? { positions[key] }
    func save(_ position: NSPoint, for key: String) {
        guard !key.isEmpty, position.x.isFinite, position.y.isFinite else { return }
        positions[key] = position
    }
    func pdfPosition(for key: String) -> PDFViewport? { pdfPositions[key] }
    func savePDF(_ viewport: PDFViewport, for key: String) {
        guard !key.isEmpty, viewport.pageIndex >= 0, viewport.point.x.isFinite, viewport.point.y.isFinite,
              viewport.scaleFactor.isFinite, viewport.scaleFactor > 0 else { return }
        pdfPositions[key] = viewport
    }
}
private struct MacDetailScrollPositionsKey: EnvironmentKey {
    static let defaultValue: MacDetailScrollPositions? = nil
}
private struct MacDetailScrollScopeKey: EnvironmentKey {
    static let defaultValue = ""
}
extension EnvironmentValues {
    var macDetailScrollPositions: MacDetailScrollPositions? {
        get { self[MacDetailScrollPositionsKey.self] }
        set { self[MacDetailScrollPositionsKey.self] = newValue }
    }
    var macDetailScrollScope: String {
        get { self[MacDetailScrollScopeKey.self] }
        set { self[MacDetailScrollScopeKey.self] = newValue }
    }
}
