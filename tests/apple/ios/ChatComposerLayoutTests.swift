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

    func testMobilePaletteSupportsReadingAndActionsInBothAppearances() {
        for style in [UIUserInterfaceStyle.light, .dark] {
            let traits = UITraitCollection(userInterfaceStyle: style)
            for (ink, surface) in [
                (DesignTokens.contentColor, DesignTokens.chatBackgroundColor),
                (DesignTokens.secondaryContentColor, DesignTokens.chatBackgroundColor),
                (DesignTokens.secondaryContentColor, DesignTokens.rowFillColor),
                (DesignTokens.contentColor, DesignTokens.accentSubtleColor),
                (DesignTokens.onActionColor, DesignTokens.actionColor)
            ] {
                func luminance(_ color: Color) -> Double {
                    var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
                    UIColor(color).resolvedColor(with: traits).getRed(&red, green: &green, blue: &blue, alpha: &alpha)
                    func linear(_ value: CGFloat) -> Double {
                        let v = Double(value)
                        return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
                    }
                    return linear(red) * 0.2126 + linear(green) * 0.7152 + linear(blue) * 0.0722
                }
                let a = luminance(ink), b = luminance(surface)
                XCTAssertGreaterThanOrEqual((max(a, b) + 0.05) / (min(a, b) + 0.05), 4.5)
            }
        }
    }

    func testNativeMobileConversationAppearanceAtNarrowWidthsAndLargeType() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let container = ServiceContainer(applicationRuntime: try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("fixture.sqlite")))
        for width in [CGFloat(320), CGFloat(390)] {
            for dark in [false, true] {
                for large in [false, true] {
                    let root = NativeMobileConversationFixture()
                        .environmentObject(container)
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, large ? .accessibility5 : .large)
                    let controller = UIHostingController(rootView: root)
                    controller.overrideUserInterfaceStyle = dark ? .dark : .light
                    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: width, height: 844))
                    window.rootViewController = controller
                    window.makeKeyAndVisible()
                    controller.view.frame = window.bounds
                    try await Task.sleep(for: .milliseconds(200))
                    controller.view.layoutIfNeeded()
                    let editor = try XCTUnwrap(descendants(controller.view).first { $0 is ChatPasteAwareTextView })
                    let bounds = editor.convert(editor.bounds, to: controller.view)
                    XCTAssertGreaterThan(bounds.width, width - 64)
                    XCTAssertLessThanOrEqual(bounds.maxX, width)
                    let image = UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                    }
                    let attachment = XCTAttachment(image: image)
                    attachment.name = "mobile-chat-\(Int(width))-\(dark ? "dark" : "light")-\(large ? "large-type" : "standard")"
                    attachment.lifetime = .keepAlways
                    add(attachment)
                    window.isHidden = true
                }
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

private struct NativeMobileConversationFixture: View {
    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    message(.user, "周末想找个地方走走，不想把行程排得太满。")
                    message(.assistant, "那就把目标定小一点：一段山路，一顿好吃的，再留一点空白。\n\n上午慢慢出发，在山脚吃午饭，下午选一条能随时折返的路线。天气好的话，坐到太阳往下落再回来。")
                }.padding(16)
            }
            PhoneComposerFixture()
        }.background(DesignTokens.chatBackgroundColor)
    }
    private func message(_ role: ChatMessage.Role, _ text: String) -> some View {
        ChatBubble(conversationID: nil, message: ChatMessage(role: role, content: text),
            isLatestAssistant: role == .assistant, isStreaming: false, isRetrieving: false,
            pendingHostPermissionRequest: nil, onHostPermissionDecision: { _, _ in })
    }
}
