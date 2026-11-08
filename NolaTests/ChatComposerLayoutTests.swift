// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit
import XCTest
@testable import Nola

@MainActor final class ChatComposerLayoutTests: XCTestCase {
    func testPhoneComposerKeepsWritingAreaAboveToolbarInBothAppearances() async throws {
        for width in [CGFloat(320), CGFloat(390)] {
            for dark in [false, true] {
                let root = PhoneComposerFixture().frame(width: width)
                    .environment(\.colorScheme, dark ? .dark : .light).ignoresSafeArea()
                let controller = UIHostingController(rootView: root)
                let size = controller.sizeThatFits(in: CGSize(width: width, height: 500))
                XCTAssertLessThanOrEqual(size.height, 116, "An empty composer should leave space for the conversation")
                let window = UIWindow(frame: CGRect(origin: .zero, size: size))
                window.rootViewController = controller
                window.makeKeyAndVisible()
                controller.view.frame = window.bounds
                try await Task.sleep(for: .milliseconds(100))
                controller.view.layoutIfNeeded()
                let editor = try XCTUnwrap(descendants(controller.view).first { $0 is ChatPasteAwareTextView })
                let bounds = editor.convert(editor.bounds, to: controller.view)
                XCTAssertGreaterThan(bounds.width, width - 64, "The toolbar must not squeeze the writing area")
                XCTAssertGreaterThanOrEqual(size.height - bounds.maxY, 44, "The toolbar belongs below the writing area")
                let image = UIGraphicsImageRenderer(size: size).image { _ in
                    controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                }
                let attachment = XCTAttachment(image: image)
                attachment.name = "composer-\(Int(width))-\(dark ? "dark" : "light")"
                attachment.lifetime = .keepAlways
                add(attachment)
                window.isHidden = true
            }
        }
    }

    private func descendants(_ view: UIView) -> [UIView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }
}

private struct PhoneComposerFixture: View {
    @State private var text = ""
    @FocusState private var focused: Bool
    var body: some View {
        ChatComposer(inputText: $text, attachments: [], attachmentMessage: nil,
            isImportingAttachments: false, isEditable: true, isGenerating: false, focus: $focused,
            onBeginAttachFiles: { { _ in } }, onRemoveAttachment: { _ in }, onSend: {}, onStop: {},
            showsModelControl: true, modelControl: HStack(spacing: 2) {
                Menu {} label: {
                Label("A long model name", systemImage: "chevron.down")
                    .font(.caption).lineLimit(1).frame(minHeight: 44)
                }
                ReasoningEffortMenu(efforts: ["none", "low", "medium", "high", "xhigh"], preference: "medium", select: { _ in })
            })
            .background(Color(uiColor: .systemBackground))
    }
}
