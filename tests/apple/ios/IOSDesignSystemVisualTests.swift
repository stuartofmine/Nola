// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import UIKit
@testable import Nola

@MainActor final class IOSDesignSystemVisualTests: XCTestCase {
    func testCompactActionTargetsHaveHitTestableMargins() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let content = VStack(spacing: 12) {
            Button {} label: {
                HStack { Text("附件与上下文"); Spacer() }
            }.buttonStyle(NolaActionButtonStyle())
            DisclosureGroup("工具执行详情", isExpanded: .constant(false)) { Text("Details") }
                .disclosureGroupStyle(NolaRowDisclosureStyle())
            NolaSettingsToggleRow(icon: "bell", tint: DesignTokens.primaryColor,
                title: "任务完成通知", isOn: .constant(true))
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .ignoresSafeArea()
        .background(DesignTokens.backgroundColor)
        let controller = UIHostingController(rootView: content)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 320, height: 240)
        window.rootViewController = controller
        window.makeKeyAndVisible()
        defer { window.isHidden = true; window.rootViewController = nil }
        try await Task.sleep(for: .milliseconds(250))
        window.layoutIfNeeded()
        // Native UIKit hit testing, including label margins and disclosure
        // whitespace. This does not synthesize a physical touch or claim that
        // SwiftUI's private gesture recognizers dispatched an action.
        for point in [CGPoint(x: 23, y: 42), CGPoint(x: 295, y: 42),
                      CGPoint(x: 24, y: 98), CGPoint(x: 260, y: 98)] {
            XCTAssertTrue(window.point(inside: point, with: nil))
            XCTAssertNotNil(window.hitTest(point, with: nil))
        }
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = "ios-compact-hit-regions-320pt"
        attachment.lifetime = .keepAlways
        add(attachment)
        let path = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("nola-ios-hit-regions.png")
        try XCTUnwrap(image.pngData()).write(to: path)
        print("IOS_HIT_REGION_SNAPSHOT=\(path.path)")
    }

    private func scrollView(in view: UIView) -> UIScrollView? {
        if let scroll = view as? UIScrollView { return scroll }
        return view.subviews.lazy.compactMap { self.scrollView(in: $0) }.first
    }

    func testSharedManagementComponentsAtCompactAndWideSizes() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for width in [CGFloat(320), CGFloat(834)] {
            for large in [false, true] {
                for dark in [false, true] {
                    let page = NolaPage {
                        NolaPageHeader(icon: "gearshape", title: "连接与账户管理", subtitle: "账户属于当前设备；授权不等于执行推理。") {
                            Button("刷新连接") {}.buttonStyle(.bordered)
                        }
                        NolaCard {
                            NolaSettingsLinkLabel(icon: "cpu", tint: DesignTokens.primaryColor,
                                title: "模型账户与执行位置", detail: "查看本机和 Host 的连接。凭据保留在所属设备，不会在切换页面时复制。")
                            NolaSettingsToggleRow(icon: "bell", tint: DesignTokens.primaryColor,
                                title: "任务完成通知", detail: "只在任务真实结束后提示；等待授权不表示已经完成。", isOn: .constant(true))
                        }
                        DisclosureGroup("回复中的执行过程", isExpanded: .constant(true)) {
                            Text("已检查输入与结果。工具详情沿阅读方向展开，正文保持原位；长段内容应正常换行。")
                                .font(.body).textSelection(.enabled)
                                .padding(.top, 8)
                        }.disclosureGroupStyle(NolaRowDisclosureStyle())
                        NolaInlineEmptyState(icon: "doc.text", title: "没有可预览的文件", message: "选择附件后在独立阅读器中查看，返回时保留原位置。")
                    }.environment(\.dynamicTypeSize, large ? .accessibility5 : .large)
                        .environment(\.colorScheme, dark ? .dark : .light)
                    let controller = UIHostingController(rootView: page)
                    let window = UIWindow(windowScene: scene)
                    window.frame = CGRect(x: 0, y: 0, width: width, height: 1000)
                    window.overrideUserInterfaceStyle = dark ? .dark : .light
                    window.rootViewController = controller
                    window.makeKeyAndVisible()
                    try await Task.sleep(for: .milliseconds(250))
                    window.layoutIfNeeded()
                    XCTAssertEqual(controller.view.bounds.width, width, accuracy: 1)
                    let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                        window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                    }
                    let attachment = XCTAttachment(image: image)
                    attachment.name = "apple-management-\(Int(width))-\(large ? "AX5" : "default")-\(dark ? "dark" : "light")"
                    attachment.lifetime = .keepAlways
                    add(attachment)
                    let scroll = try XCTUnwrap(scrollView(in: controller.view))
                    XCTAssertLessThanOrEqual(scroll.contentSize.width, scroll.bounds.width + 1,
                        "Management content must wrap instead of creating horizontal overflow")
                    if large {
                        scroll.setContentOffset(CGPoint(x: 0, y: max(0, scroll.contentSize.height - scroll.bounds.height
                            + scroll.adjustedContentInset.bottom)), animated: false)
                        try await Task.sleep(for: .milliseconds(200))
                        let bottom = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                        }
                        let lowerAttachment = XCTAttachment(image: bottom)
                        lowerAttachment.name = try XCTUnwrap(attachment.name) + "-bottom"
                        lowerAttachment.lifetime = .keepAlways
                        add(lowerAttachment)
                    }
                    window.isHidden = true
                    window.rootViewController = nil
                }
            }
        }
    }
}
