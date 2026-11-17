// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS compact tab container, conversation inspector, design system visuals, onboarding guidance, UI interaction policies, workspace page matrix, web host connection visuals, and activity detail reader suite.

import Foundation
import SwiftUI
import UIKit
import Vision
import XCTest
@testable import Nola

@MainActor
final class IOSCompactTabContainerTests: XCTestCase {
    private enum Tab { case chat, settings }

    func testCachedRootRespectsHardwareSafeAreaAndOnlyRefreshesChangedContentRevision() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let controller = IOSCompactTabContainer<Tab>.Controller()
        controller.additionalSafeAreaInsets = .init(top: 24, left: 0, bottom: 12, right: 0)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = controller; window.makeKeyAndVisible()
        defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
        controller.select(.chat, content: AnyView(TabLabelProbe(text: "initial")), revision: 0)
        try await Task.sleep(for: .milliseconds(100))
        window.layoutIfNeeded()
        let host = try XCTUnwrap(controller.children.first)
        XCTAssertGreaterThan(controller.view.safeAreaInsets.top, 20)
        XCTAssertGreaterThanOrEqual(host.view.frame.minY, controller.view.safeAreaInsets.top - 0.5)
        XCTAssertLessThanOrEqual(host.view.frame.maxY, controller.view.bounds.maxY - controller.view.safeAreaInsets.bottom + 0.5)
        func label(_ view: UIView) -> UILabel? {
            if let label = view as? UILabel { return label }
            return view.subviews.lazy.compactMap(label).first
        }
        XCTAssertEqual(label(host.view)?.text, "initial")
        controller.select(.chat, content: AnyView(TabLabelProbe(text: "unrelated render")), revision: 0)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(label(host.view)?.text, "initial")
        controller.select(.chat, content: AnyView(TabLabelProbe(text: "theme changed")), revision: 1)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(label(host.view)?.text, "theme changed")
    }

    func testReturningToActivityKeepsItsSnapshotWithoutReloading() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let controller = IOSCompactTabContainer<Tab>.Controller()
        let window = UIWindow(windowScene: scene); window.rootViewController = controller; window.makeKeyAndVisible()
        defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
        var loads = 0
        let activity = AnyView(AppleActivityView(load: { _ in
            loads += 1
            return try JSONDecoder().decode(AppleActivityPage.self, from: Data(#"{"entries":[],"next_cursor":null}"#.utf8))
        }, detail: { _ in nil }, openSource: { _ in }))
        controller.select(.chat, content: activity, revision: 0)
        for _ in 0..<50 where loads == 0 { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(loads, 1)
        controller.select(.settings, content: AnyView(Text("Settings")), revision: 0)
        try await Task.sleep(for: .milliseconds(50))
        controller.select(.chat, content: activity, revision: 0)
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(loads, 1)
    }

    func testOnlySelectedRootIsAttachedAndReturningRetainsItsController() {
        let container = IOSCompactTabContainer<Tab>.Controller()
        container.select(.chat, content: AnyView(Text("Chat")))
        let chat = container.children.first
        XCTAssertEqual(container.children.count, 1)
        container.select(.settings, content: AnyView(Text("Settings")))
        XCTAssertEqual(container.children.count, 1)
        XCTAssertNil(chat?.parent)
        XCTAssertFalse(container.children.first === chat)
        container.select(.chat, content: AnyView(Text("Updated chat")))
        XCTAssertTrue(container.children.first === chat)
        XCTAssertTrue(chat?.parent === container)
    }

    func testUpdatingCurrentRootDoesNotReplaceItsViewOrCreateSystemTabBar() {
        let container = IOSCompactTabContainer<Tab>.Controller()
        container.select(.chat, content: AnyView(Text("Chat")))
        let host = container.children.first
        container.select(.chat, content: AnyView(Text("Updated")))
        XCTAssertTrue(container.children.first === host)
        XCTAssertEqual(container.children.count, 1)
        func containsTabBar(_ view: UIView) -> Bool {
            view is UITabBar || view.subviews.contains(where: containsTabBar)
        }
        XCTAssertFalse(containsTabBar(container.view))
        XCTAssertTrue((host as? UIHostingController<AnyView>)?.safeAreaRegions == .container)
    }
}

private struct TabLabelProbe: UIViewRepresentable {
    let text: String
    func makeUIView(context: Context) -> UILabel { UILabel() }
    func updateUIView(_ label: UILabel, context: Context) { label.text = text }
}

final class IOSConversationInspectorTests: XCTestCase {
    func testEmptyCoreGoalProjectionOmitsActive() throws {
        let session: AgentCoreJSONValue = .object(["goal_projection": .object(["applied_operations": .object([:])])])
        let updated = try IOSInspectorProgressSnapshot.applyingGoalObservation(.null, to: session)
        let goal = try XCTUnwrap(updated.objectFields?["goal_projection"])
        let projection = try JSONDecoder().decode(IOSInspectorProgressFacts.GoalProjection.self,
            from: JSONEncoder().encode(goal))
        XCTAssertNil(projection.active)
        XCTAssertNil(projection.control_token)
        XCTAssertFalse(projection.can_pause || projection.can_resume || projection.can_cancel)
    }

    @MainActor func testPairedProgressReadsEncryptedSourceProjectionEnvelope() async throws {
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!,
            token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
            XCTAssertEqual(request.url?.path, "/api/mobile/chat/agent/v2/progress")
            XCTAssertEqual(request.httpMethod, "POST")
            let requestEnvelope = try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as! [String: Any]
            let query = try WebAgentProvider.decryptMobileJSON(requestEnvelope, token: "fixture-token")
            XCTAssertEqual(Set(query.keys), ["expected_host_id", "conversation_id"])
            XCTAssertEqual(query["expected_host_id"] as? String, "host")
            XCTAssertEqual(query["conversation_id"] as? String, "owner")
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": "host", "conversation_id": "owner",
                "goal_projection": NSNull(), "plan": ["conversation_id": "owner", "todos": []]
            ], token: "fixture-token")
            envelope["type"] = "encrypted"
            let data = try JSONSerialization.data(withJSONObject: envelope)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream {
                for byte in data { $0.yield(byte) }; $0.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
        let snapshot = try await provider.conversationProgress(conversationID: "owner")
        XCTAssertEqual(snapshot.facts?.conversation_id, "owner")
        XCTAssertEqual(snapshot.plan?.conversation_id, "owner")
    }

    func testRemoteProgressUsesExactSourceAndPlanOwnerWithoutLocalTaskFallback() throws {
        let payload: AgentCoreJSONValue = .object([
            "source_host_id": .string("host"), "conversation_id": .string("owner"),
            "goal_projection": .null,
            "plan": .object(["conversation_id": .string("owner"), "todos": .array([
                .object(["id": .string("todo"), "content": .string("Source fact"), "status": .string("pending")])])])])
        let result = try IOSInspectorProgressSnapshot.remote(payload, sourceHostID: "host", conversationID: "owner")
        XCTAssertEqual(result.plan?.todos.first?.content, "Source fact")
        XCTAssertNil(result.facts?.task_projection)
        XCTAssertNil(result.facts?.goal_projection)
        XCTAssertThrowsError(try IOSInspectorProgressSnapshot.remote(payload, sourceHostID: "foreign", conversationID: "owner"))
        XCTAssertThrowsError(try IOSInspectorProgressSnapshot.remote(payload, sourceHostID: "host", conversationID: "foreign"))
        guard case .object(var fields) = payload else { return XCTFail() }
        fields["plan"] = .object(["conversation_id": .string("foreign"), "todos": .array([])])
        XCTAssertThrowsError(try IOSInspectorProgressSnapshot.remote(.object(fields), sourceHostID: "host", conversationID: "owner"))
        fields["plan"] = .null
        fields.removeValue(forKey: "goal_projection")
        XCTAssertThrowsError(try IOSInspectorProgressSnapshot.remote(.object(fields), sourceHostID: "host", conversationID: "owner"))
    }

    func testPlanWithoutGoalPreservesConversationOwner() throws {
        let plan = try JSONDecoder().decode(IOSInspectorProgressFacts.Plan.self,
            from: Data(#"{"conversation_id":"owner-a","todos":[{"id":"s","content":"Verify","status":"pending"}]}"#.utf8))
        XCTAssertEqual(plan.conversation_id, "owner-a")
        XCTAssertEqual(plan.todos.first?.content, "Verify")
        XCTAssertEqual(plan.todos.first?.status, "pending")
    }

    func testGoalWorkDoesNotPromoteReportedDoneToCompletion() throws {
        let data = Data(#"{"control_token":"token","can_pause":true,"can_resume":false,"can_cancel":true,"applied_operations":{},"active":{"id":"goal","objective":"Ship","status":"running","requirements":{"r":{"id":"r","description":"Review","status":"blocked"}},"work":{"entries":[{"id":"old","content":"Old","progress":"planned","supersedes":null},{"id":"new","content":"New","progress":"reported_done","supersedes":"old"}]}}}"#.utf8)
        let projection = try JSONDecoder().decode(IOSInspectorProgressFacts.GoalProjection.self, from: data)
        XCTAssertEqual(projection.active?.work.current.map(\.id), ["new"])
        XCTAssertEqual(projection.active?.work.current.first?.progress, "reported_done")
        XCTAssertEqual(projection.active?.requirements["r"]?.status, "blocked")
    }

    func testGoalControlsUseCoreObservationForNeedsUserAndRejectForeignOwner() throws {
        let session: AgentCoreJSONValue = .object(["goal_projection": .object([
            "active": .object(["id": .string("goal"), "status": .string("needs_user")])])])
        let observation: AgentCoreJSONValue = .object([
            "goal_id": .string("goal"), "control_token": .string("token"),
            "can_pause": .bool(true), "can_resume": .bool(false), "can_cancel": .bool(true)])
        let updated = try IOSInspectorProgressSnapshot.applyingGoalObservation(observation, to: session)
        let goal = updated.objectFields?["goal_projection"]?.objectFields
        XCTAssertEqual(goal?["can_pause"], .bool(true))
        XCTAssertEqual(goal?["can_resume"], .bool(false))
        XCTAssertEqual(goal?["can_cancel"], .bool(true))
        var foreign = try XCTUnwrap(observation.objectFields)
        foreign["goal_id"] = .string("other")
        XCTAssertThrowsError(try IOSInspectorProgressSnapshot.applyingGoalObservation(.object(foreign), to: session))
        foreign["goal_id"] = .string("goal")
        foreign.removeValue(forKey: "can_pause")
        XCTAssertThrowsError(try IOSInspectorProgressSnapshot.applyingGoalObservation(.object(foreign), to: session))
    }

    func testMalformedGoalFailsInsteadOfDisplayingNoPlan() {
        XCTAssertThrowsError(try JSONDecoder().decode(IOSInspectorProgressFacts.GoalProjection.self,
            from: Data(#"{"active":{"objective":"Incomplete"}}"#.utf8)))
    }

    @MainActor func testInspectorEmptyPlanAndResourcesRenderAtPhoneAndTabletWidths() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("inspector-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        _ = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for width in [CGFloat(390), CGFloat(834)] {
            for dark in [false, true] {
                for pane in [IOSConversationInspectorPane.progress, .goals, .resources] {
                    let content = IOSConversationInspector(pane: pane, conversationID: nil, loadProgress: { _ in .init(facts: nil, plan: nil) },
                        documents: [UserUploadedContextItem(name: "隔离设计资料 · 不调用模型", content: "Fixture document")],
                        processingState: .idle)
                        .environment(\.locale, Locale(identifier: "zh-Hans"))
                    let controller = UIHostingController(rootView: content)
                    let window = UIWindow(windowScene: scene)
                    window.frame = CGRect(x: 0, y: 0, width: width, height: 900)
                    window.overrideUserInterfaceStyle = dark ? .dark : .light
                    window.rootViewController = controller
                    window.makeKeyAndVisible()
                    controller.view.frame = window.bounds
                    try await Task.sleep(for: .milliseconds(250))
                    controller.view.layoutIfNeeded()
                    let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                        window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                    }
                    let attachment = XCTAttachment(image: image)
                    attachment.name = "inspector-\(pane.rawValue)-\(Int(width))-\(dark ? "dark" : "light")"
                    attachment.lifetime = .keepAlways
                    add(attachment)
                    XCTAssertGreaterThan(image.size.width, 300)
                    window.isHidden = true
                    window.rootViewController = nil
                }
            }
        }
    }
}

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
                                title: "模型账户与执行位置")
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

@MainActor final class IOSOnboardingGuidanceTests: XCTestCase {
    func testAllGuidePagesRenderWithoutChangingPermissionsOrSavedExecutionPreferences() async throws {
        let suite = "onboarding-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        preferences.set("existing-host", forKey: "onboarding.runtime.preference")
        preferences.set("existing-sources", forKey: "onboarding.planned.sources")
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for (width, largeText): (CGFloat, Bool) in [(320, false), (390, true), (768, false)] {
            for dark in [false, true] {
                for step in 0..<4 {
                    let controller = UIHostingController(rootView: OnboardingView(initialStep: step)
                        .defaultAppStorage(preferences)
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, largeText ? .accessibility3 : .large)
                        .transaction { $0.animation = nil; $0.disablesAnimations = true })
                    let window = UIWindow(windowScene: scene)
                    window.frame = CGRect(x: 0, y: 0, width: width, height: 900)
                    window.overrideUserInterfaceStyle = dark ? .dark : .light
                    window.rootViewController = controller; window.makeKeyAndVisible()
                    defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
                    try await Task.sleep(for: .milliseconds(100))
                    controller.view.layoutIfNeeded()
                    let image = UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                    }
                    let png = try XCTUnwrap(image.pngData())
                    XCTAssertGreaterThan(png.count, 10_000)
                    try assertFooterUsesThemeActionColor(image, dark: dark)
                    let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                    attachment.name = "onboarding-\(step)-\(Int(width))-\(dark ? "dark" : "light")\(largeText ? "-accessibility" : "")"
                    attachment.lifetime = .keepAlways; add(attachment)
                    XCTAssertFalse(preferences.bool(forKey: "hasCompletedOnboarding"))
                    XCTAssertEqual(preferences.string(forKey: "onboarding.runtime.preference"), "existing-host")
                    XCTAssertEqual(preferences.string(forKey: "onboarding.planned.sources"), "existing-sources")
                    window.isHidden = true; window.rootViewController = nil; window.windowScene = nil
                }
            }
        }
    }

    private func assertFooterUsesThemeActionColor(_ image: UIImage, dark: Bool) throws {
        let expected = UIColor(DesignTokens.actionColor).resolvedColor(with: UITraitCollection(userInterfaceStyle: dark ? .dark : .light))
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        XCTAssertTrue(expected.getRed(&red, green: &green, blue: &blue, alpha: &alpha))
        let expectedBytes = [red, green, blue].map { Double($0) * 255 }
        var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
        let raster = try XCTUnwrap(image.cgImage)
        try pixels.withUnsafeMutableBytes { bytes in
            let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 64, height: 64,
                bitsPerComponent: 8, bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue))
            context.draw(raster, in: CGRect(x: 0, y: 0, width: 64, height: 64))
        }
        // The footer occupies the bottom quarter. Allow system button materials
        // and antialiasing while rejecting an untinted white prominent button.
        let matches = stride(from: 48 * 64 * 4, to: pixels.count, by: 4).filter { index in
            (0..<3).allSatisfy { abs(Double(pixels[index + $0]) - expectedBytes[$0]) < 40 }
        }.count
        XCTAssertGreaterThan(matches, 10, "The rendered primary action must use the theme color in both appearances")
    }
}

final class IOSUIInteractionPolicyTests: XCTestCase {
    @MainActor
    func testTimelineGeometryDefersCoalescesAndCancelsLayoutFeedback() async throws {
        let updates = ChatTimelineGeometryUpdates()
        var values: [Bool] = []
        updates.submit(atLatest: false) { values.append($0) }
        updates.submit(atLatest: true) { values.append($0) }
        XCTAssertTrue(values.isEmpty, "Geometry must not mutate view state inside the preference callback")
        try await Task.sleep(nanoseconds: 40_000_000)
        XCTAssertEqual(values, [true])
        updates.submit(atLatest: false) { values.append($0) }
        updates.cancel()
        try await Task.sleep(nanoseconds: 40_000_000)
        XCTAssertEqual(values, [true], "A retired viewport cannot update a new conversation")
    }

    @MainActor
    func testInkThemeMaintainsTextAndControlContrastAcrossAppearances() throws {
        func luminance(_ color: UIColor) -> Double {
            var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
            XCTAssertTrue(color.getRed(&r, green: &g, blue: &b, alpha: &a))
            let linear = [r, g, b].map { component -> Double in
                let value = Double(component)
                return value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4)
            }
            return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
        }
        for appearance in [UIUserInterfaceStyle.light, .dark] {
            for contrast in [UIAccessibilityContrast.normal, .high] {
                let traits = UITraitCollection(traitsFrom: [
                    UITraitCollection(userInterfaceStyle: appearance),
                    UITraitCollection(accessibilityContrast: contrast)
                ])
                for (foreground, background, minimum) in [
                    ("NolaTextPrimary", "NolaSurface", 4.5),
                    ("NolaTextSecondary", "NolaSurfaceRaised", 4.5),
                    ("AccentColor", "NolaSurface", 4.5),
                    ("NolaOnAccent", "AccentColor", 4.5),
                    ("NolaControlOutline", "NolaSurfaceRaised", 3.0)
                ] {
                    let fg = try XCTUnwrap(UIColor(named: foreground)).resolvedColor(with: traits)
                    let bg = try XCTUnwrap(UIColor(named: background)).resolvedColor(with: traits)
                    let first = luminance(fg), second = luminance(bg)
                    XCTAssertGreaterThanOrEqual((max(first, second) + 0.05) / (min(first, second) + 0.05), minimum,
                        "\(foreground) / \(background), \(appearance), \(contrast)")
                }
            }
        }
    }

    @MainActor
    func testFeedbackFailureRetainsDraftAndRetryUsesIdenticalInput() {
        var draft = ChatLearningFeedbackDraft()
        draft.focus = .correction
        draft.note = "  Correct the date to Friday.  "
        draft.appliesToFuture = true
        let statement = draft.statement
        XCTAssertFalse(draft.save { text, future in
            XCTAssertEqual(text, statement)
            XCTAssertTrue(future)
            return false
        })
        XCTAssertTrue(draft.saveFailed)
        XCTAssertTrue(draft.isDirty)
        XCTAssertEqual(draft.note, "  Correct the date to Friday.  ")
        XCTAssertEqual(draft.focus, .correction)
        XCTAssertTrue(draft.save { text, future in
            XCTAssertEqual(text, statement)
            XCTAssertTrue(future)
            return true
        })
        XCTAssertFalse(draft.saveFailed)
    }

    @MainActor
    func testFeedbackPersistenceRequiresExplicitSuccess() {
        XCTAssertFalse(ChatLearningFeedbackDraft.didPersist(nil))
        XCTAssertFalse(ChatLearningFeedbackDraft.didPersist([:]))
        XCTAssertFalse(ChatLearningFeedbackDraft.didPersist(["ok": .bool(false), "error": .string("disk full")]))
        XCTAssertFalse(ChatLearningFeedbackDraft.didPersist(["ok": .string("true")]))
        XCTAssertTrue(ChatLearningFeedbackDraft.didPersist(["ok": .bool(true)]))
    }

    @MainActor
    func testFeedbackCancelProtectionIncludesFocusToggleWhitespaceAndFailedDefaultSave() {
        var draft = ChatLearningFeedbackDraft()
        XCTAssertFalse(draft.appliesToFuture)
        XCTAssertFalse(draft.isDirty)
        draft.note = " "
        XCTAssertTrue(draft.isDirty)
        draft.note = ""
        draft.focus = .tone
        XCTAssertTrue(draft.isDirty)
        draft.focus = .concise
        draft.appliesToFuture = true
        XCTAssertTrue(draft.isDirty)
        draft.appliesToFuture = false
        XCTAssertFalse(draft.save { _, _ in false })
        XCTAssertTrue(draft.isDirty)
    }

    func testConversationLimitResetChangesOnlyDraftAndPreservesTypedValues() {
        let saved = IOSConversationGenerationDraft(contextEnabled: true, outputEnabled: true, contextText: "64000", outputText: "2048")
        var draft = saved
        draft.inheritModelDefaults()
        XCTAssertNotEqual(draft, saved)
        XCTAssertEqual(draft.value, IOSConversationGenerationOverride())
        XCTAssertEqual(saved.value?.maxContextLength, 64000)
        XCTAssertEqual(draft.contextText, "64000")
        draft.contextEnabled = true
        draft.outputEnabled = true
        XCTAssertEqual(draft, saved)
    }

    func testConversationLimitInvalidDraftStillCountsAsUnsaved() {
        let saved = IOSConversationGenerationDraft(contextEnabled: true, outputEnabled: false, contextText: "64000", outputText: "2048")
        var draft = saved
        draft.contextText = ""
        XCTAssertNil(draft.value)
        XCTAssertNotEqual(draft, saved)
    }

    func testConversationLimitDraftRejectsOutOfRangeValuesButCanInherit() {
        var draft = IOSConversationGenerationDraft(contextEnabled: true, outputEnabled: true, contextText: "511", outputText: "63")
        XCTAssertNil(draft.value)
        draft.contextText = "4000001"
        draft.outputText = "1000001"
        XCTAssertNil(draft.value)
        draft.inheritModelDefaults()
        XCTAssertEqual(draft.value, IOSConversationGenerationOverride())
    }

    func testLiveReasoningStartsVisibleAndHistoricalReasoningStartsCollapsed() {
        XCTAssertFalse(ChatThinkingDisclosurePolicy.initialExpanded(isFinished: true))
        XCTAssertTrue(ChatThinkingDisclosurePolicy.initialExpanded(isFinished: false))
    }

    func testTerminalResponseFeedbackUsesCommittedStateRatherThanToolFailure() {
        let key = ChatConversationSurfaceMeta.stateKey
        XCTAssertEqual(ChatResponseStatusPolicy.terminalState([key: "failed"]), .failed)
        XCTAssertEqual(ChatResponseStatusPolicy.terminalState([key: "cancelled"]), .cancelled)
        XCTAssertNil(ChatResponseStatusPolicy.terminalState([key: "completed"]))
        XCTAssertNil(ChatResponseStatusPolicy.terminalState([key: "running"]))
        XCTAssertNil(ChatResponseStatusPolicy.terminalState([:]))
    }

    func testLongToolTextHasAnExplicitReaderWithoutSplittingGraphemes() {
        XCTAssertFalse(ChatToolTextPresentation.needsReader(String(repeating: "a", count: 1200)))
        XCTAssertTrue(ChatToolTextPresentation.needsReader(String(repeating: "a", count: 1201)))
        XCTAssertFalse(ChatToolTextPresentation.needsReader(String(repeating: "👩‍💻", count: 1200)))
        XCTAssertTrue(ChatToolTextPresentation.needsReader(String(repeating: "👩‍💻", count: 1201)))
    }

    func testReasoningCompletionPreservesBothUserDisclosureChoices() {
        XCTAssertTrue(ChatThinkingDisclosurePolicy.expandedAfterStatusChange(current: true, isFinished: true))
        XCTAssertFalse(ChatThinkingDisclosurePolicy.expandedAfterStatusChange(current: false, isFinished: true))
        XCTAssertFalse(ChatThinkingDisclosurePolicy.expandedAfterStatusChange(
            current: false, isFinished: false, userChangedExpansion: true), "A new round must not undo manual collapse")
        XCTAssertTrue(ChatThinkingDisclosurePolicy.expandedAfterStatusChange(
            current: false, isFinished: false, userChangedExpansion: false))
    }

    @MainActor
    func testComposerHasDynamicTypeAndAnAccessibleName() {
        let view = UITextView()
        view.font = .systemFont(ofSize: 9)
        ChatComposerAccessibility.configure(view)
        XCTAssertTrue(view.adjustsFontForContentSizeCategory)
        XCTAssertEqual(view.accessibilityLabel, L10n.chatInputPlaceholder)
        XCTAssertEqual(view.font, UIFont.preferredFont(forTextStyle: .body, compatibleWith: view.traitCollection))
    }

    @MainActor
    func testComposerRefreshUsesAccessibilitySizeTraits() {
        let view = UITextView()
        if #available(iOS 17.0, *) {
            view.traitOverrides.preferredContentSizeCategory = .accessibilityExtraExtraExtraLarge
        }
        ChatComposerAccessibility.configure(view)
        XCTAssertEqual(view.font, UIFont.preferredFont(forTextStyle: .body, compatibleWith: view.traitCollection))
        XCTAssertTrue(view.adjustsFontForContentSizeCategory)
    }
}

@MainActor final class IOSWorkspacePageMatrixTests: XCTestCase {
    func testTopLevelTabsRespectIslandSafeAreaAtNarrowWidthsAndLargeText() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-tab-pages-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        try runtime.createConversation(id: UUID().uuidString, title: "最近的研究对话", settingsJSON: #"{"knowledge":{"keyPointsMarkdown":"保留有来源的结论，之后可以从这里继续。"}}"#, createdAtMs: 1, updatedAtMs: 1)
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for (width, largeText): (CGFloat, Bool) in [(320, false), (390, false), (390, true)] {
            for dark in [false, true] {
                let controller = IOSCompactTabContainer<String>.Controller()
                let window = UIWindow(windowScene: scene)
                window.frame = CGRect(x: 0, y: 0, width: width, height: 844)
                window.overrideUserInterfaceStyle = dark ? .dark : .light
                window.rootViewController = controller; window.makeKeyAndVisible()
                controller.additionalSafeAreaInsets.top = max(0, 62 - controller.view.safeAreaInsets.top)
                defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
                for tab in ["chat", "information", "activity", "settings"] {
                    let content: AnyView
                    switch tab {
                    case "chat": content = AnyView(NavigationStack { ChatConversationsList(search: .constant(""), onSelect: { _ in }) })
                    case "information": content = AnyView(IOSInformationView(path: .constant([]), pendingRequests: .constant([])))
                    case "activity": content = AnyView(NavigationStack { IOSConversationActivityView().navigationTitle(Text("information.activity")).navigationBarTitleDisplayMode(.inline) })
                    default: content = AnyView(SettingsView())
                    }
                    controller.select(tab, content: AnyView(content.environmentObject(container)
                        .environment(\.locale, Locale(identifier: "zh-Hans"))
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, largeText ? .accessibility3 : .large)), revision: 0)
                    try await Task.sleep(for: .milliseconds(300))
                    window.layoutIfNeeded()
                    let child = try XCTUnwrap(controller.children.first)
                    XCTAssertGreaterThanOrEqual(child.view.frame.minY, 61.5, tab)
                    XCTAssertLessThanOrEqual(child.view.frame.maxX, width + 0.5, tab)
                    let png = try XCTUnwrap(UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                    }.pngData())
                    XCTAssertGreaterThan(png.count, 10_000)
                    let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                    attachment.name = "tab-\(tab)-\(Int(width))-\(dark ? "dark" : "light")\(largeText ? "-accessibility" : "")"
                    attachment.lifetime = .keepAlways; add(attachment)
                }
                window.isHidden = true
            }
        }
    }

    func testEveryManagementDestinationAtPhoneAndTabletWidthsInBothThemes() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-pages-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        defer {
            if FileManager.default.fileExists(atPath: root.path) { try? FileManager.default.removeItem(at: root) }
        }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let destinations = IOSWorkspaceDestination.allCases
        for (width, largeText): (CGFloat, Bool) in [(320, false), (390, false), (768, false), (390, true)] {
            for dark in [false, true] {
                for destination in destinations {
                    let view = NavigationStack { IOSWorkspaceManagementContent(destination: destination) }
                        .environmentObject(container)
                        .environment(\.colorScheme, dark ? .dark : .light)
                        .environment(\.dynamicTypeSize, largeText ? .accessibility3 : .large)
                    let controller = UIHostingController(rootView: view)
                    let window = UIWindow(windowScene: scene)
                    window.frame = CGRect(x: 0, y: 0, width: width, height: 900)
                    window.overrideUserInterfaceStyle = dark ? .dark : .light
                    window.rootViewController = controller
                    window.makeKeyAndVisible()
                    defer {
                        window.isHidden = true
                        window.rootViewController = nil
                        window.windowScene = nil
                    }
                    controller.view.frame = window.bounds
                    try await Task.sleep(for: .milliseconds(200))
                    controller.view.layoutIfNeeded()
                    XCTAssertEqual(controller.view.bounds.width, width, accuracy: 0.1)
                    let image = UIGraphicsImageRenderer(size: window.bounds.size).image { _ in
                        controller.view.drawHierarchy(in: controller.view.bounds, afterScreenUpdates: true)
                    }
                    let raster = try XCTUnwrap(image.cgImage)
                    var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
                    try pixels.withUnsafeMutableBytes { bytes in
                        let bitmap = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 64, height: 64,
                            bitsPerComponent: 8, bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                        bitmap.draw(raster, in: CGRect(x: 0, y: 0, width: 64, height: 64))
                    }
                    let luminance = stride(from: 0, to: pixels.count, by: 4).map { Int(pixels[$0]) + Int(pixels[$0 + 1]) + Int(pixels[$0 + 2]) }
                    XCTAssertGreaterThan((luminance.max() ?? 0) - (luminance.min() ?? 0), 60,
                        "Page content must actually be painted: \(destination)")
                    let png = try XCTUnwrap(image.pngData())
                    XCTAssertGreaterThan(png.count, 10_000, "Blank management page: \(destination)")
                    // Keep native-resolution evidence without retaining a full
                    // uncompressed bitmap for every page until this test ends.
                    let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
                    attachment.name = "page-\(destination)-\(Int(width))-\(dark ? "dark" : "light")\(largeText ? "-accessibility" : "")"
                    attachment.lifetime = .keepAlways
                    add(attachment)
                    window.isHidden = true
                }
            }
        }
    }
}

@MainActor
final class WebHostConnectionUIVisualTests: XCTestCase {
    func testConnectionSheetRendersSetupAndInputsAtPhoneSize() throws {
        let view = NavigationStack {
            WebHostConfigurationView(existingModel: nil) { _ in }
        }
        .environment(\.locale, Locale(identifier: "zh-Hans"))

        let controller = UIHostingController(rootView: view)
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        )
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = controller
        window.makeKeyAndVisible()
        controller.view.frame = window.bounds
        controller.view.setNeedsLayout()
        controller.view.layoutIfNeeded()

        let renderer = UIGraphicsImageRenderer(bounds: window.bounds)
        let image = renderer.image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let data = try XCTUnwrap(image.pngData())
        XCTAssertGreaterThan(data.count, 20_000)
        let recognition = VNRecognizeTextRequest()
        recognition.recognitionLanguages = ["zh-Hans", "en-US"]
        recognition.recognitionLevel = .accurate
        try VNImageRequestHandler(cgImage: XCTUnwrap(image.cgImage)).perform([recognition])
        let visibleText = recognition.results?.compactMap { $0.topCandidates(1).first?.string }
            .joined().filter { !$0.isWhitespace } ?? ""
        let pasteLabel = NSLocalizedString("model.web_host.paste_setup", value: "Paste Connection Setup", comment: "")
            .filter { !$0.isWhitespace }
        XCTAssertTrue(visibleText.contains(pasteLabel), "The pairing action must be visible, not an empty control row")
        try data.write(to: URL(fileURLWithPath: "/tmp/nola-ios-web-host-connection.png"), options: .atomic)
    }

    func testConnectionFailuresUseStableLocalizedMessages() {
        XCTAssertEqual(
            webHostPairingErrorMessage(WebHostPairingFailure.identityChanged),
            NSLocalizedString("model.web_host.error.identity_changed", comment: "")
        )
        XCTAssertEqual(
            webHostPairingErrorMessage(URLError(.timedOut)),
            NSLocalizedString("model.web_host.error.timeout", comment: "")
        )
        XCTAssertEqual(
            webHostPairingErrorMessage(URLError(.cannotConnectToHost)),
            NSLocalizedString("model.web_host.error.unreachable", comment: "")
        )
    }
    func testSavedPairingTokenNeverLoadsForAnEditedDestination() throws {
        let original = "https://host.example/routed"
        var reads = 0
        for changed in ["https://other.example/routed", "https://host.example:8443/routed",
                        "http://host.example/routed", "https://host.example/other"] {
            let url = try XCTUnwrap(IOSHostConnectionDraftPolicy.endpoint(changed))
            let token = IOSHostConnectionDraftPolicy.token(entered: "", for: url, original: original) {
                reads += 1
                return "saved-secret"
            }
            XCTAssertEqual(token, "", changed)
        }
        XCTAssertEqual(reads, 0, "An edited destination must not even read the saved credential")
        let approved = try XCTUnwrap(IOSHostConnectionDraftPolicy.endpoint(original))
        XCTAssertEqual(IOSHostConnectionDraftPolicy.token(entered: "", for: approved, original: original) {
            reads += 1; return "saved-secret"
        }, "saved-secret")
        XCTAssertEqual(reads, 1)
        XCTAssertEqual(IOSHostConnectionDraftPolicy.token(entered: " replacement ", for: approved, original: original) {
            XCTFail("Explicit pairing token must not read a saved secret"); return nil
        }, "replacement")
    }

    func testManualPairingRejectsAmbiguousCredentialDestinations() {
        for input in ["https://host.example?token=x", "https://host.example#fragment",
                      "https://user:password@host.example", "https://host.example:0",
                      "https://host.example:65536", "file:///tmp/host"] {
            XCTAssertNil(IOSHostConnectionDraftPolicy.endpoint(input), input)
        }
        XCTAssertNotNil(IOSHostConnectionDraftPolicy.endpoint("http://192.168.1.2:8080"))
    }

    func testPermissionCallbackRequiresTheOriginalStepAndPresentation() async {
        let owner = UUID()
        // Simulate returning from an asynchronous OS request after the UI moved
        // or closed. The production completion uses this same ownership gate.
        await Task.yield()
        XCTAssertTrue(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: owner, cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 1,
            owner: owner, activeOwner: owner, cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: nil, cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: UUID(), cancelled: false))
        XCTAssertFalse(PermissionSetupRequestCompletion.accepts(sourceIndex: 0, currentIndex: 0,
            owner: owner, activeOwner: owner, cancelled: true))
    }

}

@MainActor
final class AppleActivityDetailReaderTests: XCTestCase {
    func testLateResultAndLateErrorCannotReplaceAnotherActivity() async throws {
        for fails in [false, true] {
            let reader = AppleActivityDetailReader()
            let a = try entry("a"), b = try entry("b")
            var pending: CheckedContinuation<AppleActivityDetail?, Error>?
            let old = Task {
                await reader.load(a) { _ in try await withCheckedThrowingContinuation { pending = $0 } }
            }
            for _ in 0..<100 where pending == nil { await Task.yield() }
            let continuation = try XCTUnwrap(pending)
            await reader.load(b) { _ in try self.detail(b, text: "B result") }
            if fails { continuation.resume(throwing: FixtureFailure.failed) }
            else { continuation.resume(returning: try detail(a, text: "late A result")) }
            await old.value
            XCTAssertNil(reader.value(for: a))
            XCTAssertEqual(reader.value(for: b)?.result_text, "B result")
            XCTAssertNil(reader.error(for: b))
            XCTAssertFalse(reader.isLoading(b))
        }
    }

    func testRefreshFailurePreservesOnlyTheSameOwnersLastSuccessfulResult() async throws {
        let reader = AppleActivityDetailReader(), a = try entry("a"), b = try entry("b")
        await reader.load(a) { _ in try self.detail(a, text: "saved A result") }
        await reader.load(a) { _ in throw FixtureFailure.failed }
        XCTAssertEqual(reader.value(for: a)?.result_text, "saved A result")
        XCTAssertNotNil(reader.error(for: a))
        // Even before B's asynchronous task starts, its view cannot read A.
        XCTAssertNil(reader.value(for: b))
        XCTAssertNil(reader.error(for: b))
        XCTAssertTrue(reader.isLoading(b))
    }

    func testDetailMustMatchAllThreeCanonicalOwnerFields() async throws {
        let a = try entry("a")
        for key in ["run_id", "conversation_id", "assistant_message_id"] {
            let reader = AppleActivityDetailReader()
            var payload = fields(a)
            payload[key] = "foreign"
            let foreign = try JSONDecoder().decode(AppleActivityEntry.self, from: JSONSerialization.data(withJSONObject: payload))
            await reader.load(a) { _ in try self.detail(foreign, text: "must not be shown") }
            XCTAssertNil(reader.value(for: a), key)
            XCTAssertNotNil(reader.error(for: a), key)
        }
    }

    private enum FixtureFailure: Error { case failed }
    private func entry(_ id: String) throws -> AppleActivityEntry {
        try JSONDecoder().decode(AppleActivityEntry.self, from: JSONSerialization.data(withJSONObject: [
            "run_id": id, "conversation_id": "conversation-" + id, "assistant_message_id": "assistant-" + id,
            "source": "agent", "title": id, "created_at_ms": 1, "completed_at_ms": 2,
            "status": "completed", "needs_response": false, "is_current": true]))
    }
    private func detail(_ entry: AppleActivityEntry, text: String) throws -> AppleActivityDetail {
        return try JSONDecoder().decode(AppleActivityDetail.self, from: JSONSerialization.data(withJSONObject: [
            "entry": fields(entry), "result_text": text, "result_has_more": false,
            "source_conversation_id": entry.conversation_id, "review_in_source": false,
            "context_scope": "conversation"]))
    }
    private func fields(_ entry: AppleActivityEntry) -> [String: Any] {
        ["run_id": entry.run_id, "conversation_id": entry.conversation_id, "assistant_message_id": entry.assistant_message_id,
         "source": entry.source, "title": entry.title, "created_at_ms": entry.created_at_ms,
         "completed_at_ms": entry.completed_at_ms.map { $0 as Any } ?? NSNull(), "status": entry.status,
         "needs_response": entry.needs_response, "is_current": entry.is_current]
    }
}
