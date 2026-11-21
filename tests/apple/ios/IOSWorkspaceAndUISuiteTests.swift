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
    func testWelcomeRendersWithoutChangingPermissionsOrSavedExecutionPreferences() async throws {
        let suite = "onboarding-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        preferences.set("existing-host", forKey: "onboarding.runtime.preference")
        preferences.set("existing-sources", forKey: "onboarding.planned.sources")
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for (width, largeText): (CGFloat, Bool) in [(320, false), (390, true), (768, false)] {
            for dark in [false, true] {
                let controller = UIHostingController(rootView: OnboardingView(chooseModel: {}, skip: {})
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
                attachment.name = "onboarding-\(Int(width))-\(dark ? "dark" : "light")\(largeText ? "-accessibility" : "")"
                attachment.lifetime = .keepAlways; add(attachment)
                XCTAssertFalse(preferences.bool(forKey: "hasCompletedOnboarding"))
                XCTAssertEqual(preferences.string(forKey: "onboarding.runtime.preference"), "existing-host")
                XCTAssertEqual(preferences.string(forKey: "onboarding.planned.sources"), "existing-sources")
                window.isHidden = true; window.rootViewController = nil; window.windowScene = nil
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
    func testColorsetsAndDesignTokensMatchCanonicalTokensAndWCAGContrast() throws {
        struct TokenPalette: Decodable {
            let light: [String: String]
            let dark: [String: String]
        }
        struct TokenFile: Decodable {
            let color: TokenPalette
        }

        func hexToSRGB(_ hex: String) throws -> (r: Double, g: Double, b: Double) {
            let trimmed = hex.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)
                .trimmingCharacters(in: CharacterSet(charactersIn: "#"))
            guard trimmed.count == 6, let value = UInt32(trimmed, radix: 16) else {
                throw NSError(domain: "TokenCheck", code: 1, userInfo: [NSLocalizedDescriptionKey: "Invalid hex token: \(hex)"])
            }
            return (
                Double((value >> 16) & 0xFF) / 255.0,
                Double((value >> 8) & 0xFF) / 255.0,
                Double(value & 0xFF) / 255.0
            )
        }

        func resolveSRGB(_ color: UIColor, traits: UITraitCollection) throws -> (r: Double, g: Double, b: Double) {
            let resolved = color.resolvedColor(with: traits)
            var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
            if let srgb = CGColorSpace(name: CGColorSpace.sRGB),
               let converted = resolved.cgColor.converted(to: srgb, intent: .defaultIntent, options: nil),
               let components = converted.components,
               components.count >= 3 {
                return (Double(components[0]), Double(components[1]), Double(components[2]))
            }
            XCTAssertTrue(resolved.getRed(&r, green: &g, blue: &b, alpha: &a))
            return (Double(r), Double(g), Double(b))
        }

        func relativeLuminance(_ rgb: (r: Double, g: Double, b: Double)) -> Double {
            func linearize(_ c: Double) -> Double {
                c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
            }
            return linearize(rgb.r) * 0.2126 + linearize(rgb.g) * 0.7152 + linearize(rgb.b) * 0.0722
        }

        func contrastRatio(_ a: (r: Double, g: Double, b: Double), _ b: (r: Double, g: Double, b: Double)) -> Double {
            let l1 = relativeLuminance(a)
            let l2 = relativeLuminance(b)
            return (max(l1, l2) + 0.05) / (min(l1, l2) + 0.05)
        }

        let repoRoot = try IOSVisualPixelAssertions.findRepositoryRoot()
        let tokensURL = repoRoot.appendingPathComponent("shared/design-tokens/tokens.json")
        let tokenFile = try JSONDecoder().decode(TokenFile.self, from: Data(contentsOf: tokensURL))
        let lightRoles = tokenFile.color.light
        let darkRoles = tokenFile.color.dark

        let colorsetRoleMap: [(asset: String, role: String)] = [
            ("AccentColor", "accent"),
            ("NolaAccentSubtle", "accentSubtle"),
            ("NolaCanvas", "canvas"),
            ("NolaControlOutline", "outline"),
            ("NolaDanger", "danger"),
            ("NolaDivider", "border"),
            ("NolaOnAccent", "onAccent"),
            ("NolaSuccess", "success"),
            ("NolaSurface", "surface"),
            ("NolaSurfaceRaised", "surfaceSubtle"),
            ("NolaTextPrimary", "ink"),
            ("NolaTextSecondary", "inkSecondary"),
            ("NolaWarning", "warning")
        ]

        let designTokenRoleMap: [(name: String, color: Color, role: String)] = [
            ("backgroundColor", DesignTokens.backgroundColor, "canvas"),
            ("chatBackgroundColor", DesignTokens.chatBackgroundColor, "canvas"),
            ("surfaceColor", DesignTokens.surfaceColor, "surface"),
            ("cardBackgroundColor", DesignTokens.cardBackgroundColor, "surface"),
            ("elevatedColor", DesignTokens.elevatedColor, "surfaceSubtle"),
            ("secondaryBackgroundColor", DesignTokens.secondaryBackgroundColor, "surfaceSubtle"),
            ("rowFillColor", DesignTokens.rowFillColor, "surfaceSubtle"),
            ("mutedFillColor", DesignTokens.mutedFillColor, "surfaceSubtle"),
            ("contentColor", DesignTokens.contentColor, "ink"),
            ("secondaryContentColor", DesignTokens.secondaryContentColor, "inkSecondary"),
            ("primaryColor", DesignTokens.primaryColor, "ink"),
            ("actionColor", DesignTokens.actionColor, "accent"),
            ("accentSubtleColor", DesignTokens.accentSubtleColor, "accentSubtle"),
            ("borderColor", DesignTokens.borderColor, "border"),
            ("separatorColor", DesignTokens.separatorColor, "border"),
            ("strongBorderColor", DesignTokens.strongBorderColor, "outline"),
            ("onActionColor", DesignTokens.onActionColor, "onAccent"),
            ("onAccentColor", DesignTokens.onAccentColor, "onAccent"),
            ("successColor", DesignTokens.successColor, "success"),
            ("warningColor", DesignTokens.warningColor, "warning"),
            ("dangerColor", DesignTokens.dangerColor, "danger")
        ]

        for (style, roles) in [(UIUserInterfaceStyle.light, lightRoles), (.dark, darkRoles)] {
            let traits = UITraitCollection {
                $0.userInterfaceStyle = style
                $0.accessibilityContrast = .normal
            }
            for (asset, role) in colorsetRoleMap {
                let expectedHex = try XCTUnwrap(roles[role], "Missing \(role) in tokens.json")
                let expected = try hexToSRGB(expectedHex)
                let color = try XCTUnwrap(UIColor(named: asset), "Missing colorset \(asset)")
                let actual = try resolveSRGB(color, traits: traits)
                XCTAssertEqual(actual.r, expected.r, accuracy: 0.015, "\(asset) R mismatch (\(style.rawValue))")
                XCTAssertEqual(actual.g, expected.g, accuracy: 0.015, "\(asset) G mismatch (\(style.rawValue))")
                XCTAssertEqual(actual.b, expected.b, accuracy: 0.015, "\(asset) B mismatch (\(style.rawValue))")
            }

            for (name, tokenColor, role) in designTokenRoleMap {
                let expectedHex = try XCTUnwrap(roles[role], "Missing \(role) in tokens.json")
                let expected = try hexToSRGB(expectedHex)
                let actual = try resolveSRGB(UIColor(tokenColor), traits: traits)
                XCTAssertEqual(actual.r, expected.r, accuracy: 0.015, "DesignTokens.\(name) R mismatch (\(style.rawValue))")
                XCTAssertEqual(actual.g, expected.g, accuracy: 0.015, "DesignTokens.\(name) G mismatch (\(style.rawValue))")
                XCTAssertEqual(actual.b, expected.b, accuracy: 0.015, "DesignTokens.\(name) B mismatch (\(style.rawValue))")
            }

            for contrast in [UIAccessibilityContrast.normal, .high] {
                let contrastTraits = UITraitCollection {
                    $0.userInterfaceStyle = style
                    $0.accessibilityContrast = contrast
                }
                for (foreground, background, minimum) in [
                    ("NolaTextPrimary", "NolaCanvas", 4.5),
                    ("NolaTextPrimary", "NolaSurface", 4.5),
                    ("NolaTextSecondary", "NolaSurfaceRaised", 4.5),
                    ("AccentColor", "NolaSurface", 4.5),
                    ("NolaOnAccent", "AccentColor", 4.5),
                    ("NolaSuccess", "NolaSurface", 4.5),
                    ("NolaWarning", "NolaSurface", 4.5),
                    ("NolaDanger", "NolaSurface", 4.5),
                    ("NolaControlOutline", "NolaSurfaceRaised", 3.0)
                ] {
                    let fg = try resolveSRGB(XCTUnwrap(UIColor(named: foreground)), traits: contrastTraits)
                    let bg = try resolveSRGB(XCTUnwrap(UIColor(named: background)), traits: contrastTraits)
                    XCTAssertGreaterThanOrEqual(
                        contrastRatio(fg, bg), minimum,
                        "\(foreground) / \(background), style=\(style.rawValue), contrast=\(contrast.rawValue)"
                    )
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

    @MainActor
    func testChatContextSheetUploadedDocumentSaveRoundTrip() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-docs-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let databaseURL = root.appendingPathComponent("fixture.sqlite")
        let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)

        let expectedDoc = UserUploadedContextItem(
            name: "Release Runbook",
            content: "Step 1: Verify Warm Paper and Warm Ink parity.",
            description: "Verification guide",
            enabled: true
        )
        let savedFromCallback = try container.chatManager.updateUploadedDocuments([expectedDoc], expected: container.chatMemoryProfileStore.loadProfile()).uploadedDocuments
        XCTAssertEqual(savedFromCallback.map(\.name), ["Release Runbook"])

        let reopenedRuntime = try IOSApplicationRuntime(databaseURL: databaseURL)
        let reopenedContainer = ServiceContainer(applicationRuntime: reopenedRuntime)
        let reloaded = reopenedContainer.chatMemoryProfileStore.loadProfile().uploadedDocuments
        XCTAssertEqual(reloaded.count, 1)
        XCTAssertEqual(reloaded.first?.id, expectedDoc.id)
        XCTAssertEqual(reloaded.first?.name, expectedDoc.name)
        XCTAssertEqual(reloaded.first?.content, expectedDoc.content)
        XCTAssertEqual(reloaded.first?.enabled, true)
    }

    @MainActor
    func testChatContextSheetShowsLocalMCPServersWhenHostMCPListIsEmpty() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-local-mcp-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)

        let localMCP = UserMCPServer(
            name: "Local Knowledge MCP",
            endpoint: "https://mcp.local.example/sse",
            description: "Local workspace index",
            enabled: true
        )
        let sheet = NavigationStack {
            ChatContextSheet(
                displayMode: .context,
                conversationID: UUID(),
                conversationTitle: "MCP Check",
                contextUsage: nil,
                sessionMemory: .constant("Notes"),
                knowledgePoints: .constant("Key facts"),
                conversationWiki: .constant("Wiki"),
                longTermMemory: .constant("Durable preference"),
                userPreferences: .constant("Concise output"),
                mcpServers: .constant([localMCP]),
                uploadedSkills: .constant([]),
                hostUploadedSkills: [],
                hostUploadedDocuments: [],
                hostMCPServers: [],
                uploadedDocuments: .constant([]),
                dataSources: [],
                selectedDataSourceIDs: .constant([]),
                selectedSystemDataSourceTypes: .constant([]),
                onSave: {},
                onSaveMemoryProfile: {},
                onSaveUploadedDocuments: { $0 },
                onSaveConversationKnowledge: { _ in },
                onCompact: { "Compacted" },
                onReload: { nil },
                onDiscard: {},
                onUseSkill: { _ in },
                onSearchMemory: { _ in [] },
                initialContextPane: .tools
            )
            .environmentObject(container)
        }

        let image = try await IOSVisualPixelAssertions.assertPresentedSheet(
            sheet,
            label: "sheet-chat-context-local-mcp-light",
            userInterfaceStyle: .light,
            locale: Locale(identifier: "en-US"),
            settleMilliseconds: 260
        )
        let visibleText = try recognizeNormalizedText(in: image)
        XCTAssertTrue(
            visibleText.contains("LocalKnowledgeMCP"),
            "Expected local MCP server to be visibly rendered when hostMCPServers is empty; OCR=\(visibleText)"
        )
    }

    @MainActor
    func testGoalSlashCommandPrefillsConversationInspectorObjective() async throws {
        let parsed = try XCTUnwrap(ComposerCommand.parse("/goal Ship v1.0 release"))
        XCTAssertEqual(parsed.command, .goal)
        XCTAssertEqual(parsed.argument, "Ship v1.0 release")

        let hostRoot = UIViewController()
        hostRoot.view.backgroundColor = UIColor(DesignTokens.backgroundColor)
        let window = try IOSVisualPixelAssertions.makeWindow(
            size: CGSize(width: 390, height: 844),
            userInterfaceStyle: .light,
            rootViewController: hostRoot
        )
        defer { IOSVisualPixelAssertions.teardownWindow(window) }
        try await IOSVisualPixelAssertions.settleLayout(on: window, milliseconds: 50)

        let inspector = IOSConversationInspector(
            pane: .goals,
            conversationID: UUID().uuidString,
            loadProgress: { _ in
                try? await Task.sleep(nanoseconds: 150_000_000)
                return IOSInspectorProgressSnapshot(supportsGoalControl: true, facts: nil, plan: nil)
            },
            submitGoalControl: { _, _ in true },
            documents: [],
            processingState: .idle,
            initialObjective: parsed.argument
        )
        .environment(\.colorScheme, .light)
        .environment(\.locale, Locale(identifier: "en-US"))

        let inspectorController = UIHostingController(rootView: inspector)
        inspectorController.modalPresentationStyle = .pageSheet
        hostRoot.present(inspectorController, animated: false)

        let deadline = Date().addingTimeInterval(2.5)
        var goalStartController: UIViewController?
        while Date() < deadline {
            await Task.yield()
            if let nested = inspectorController.presentedViewController,
               !nested.isBeingPresented,
               nested.view.window != nil {
                goalStartController = nested
                break
            }
            try await Task.sleep(nanoseconds: 25_000_000)
        }

        let targetController = try XCTUnwrap(
            goalStartController,
            "Goal start sheet should automatically present with /goal objective prefilled"
        )
        try await IOSVisualPixelAssertions.settleLayout(on: window, milliseconds: 220)
        let image = try IOSVisualPixelAssertions.captureImage(of: targetController.view)
        let cgImage = try XCTUnwrap(image.cgImage)
        try IOSVisualPixelAssertions.assertCGImage(
            cgImage,
            label: "sheet-goal-slash-prefill-light",
            isDark: false,
            pointSize: targetController.view.bounds.size
        )
        IOSVisualPixelAssertions.maybeSaveArtifact(image: image, name: "sheet-goal-slash-prefill-light")
        let visibleText = try recognizeNormalizedText(in: image)
        XCTAssertTrue(
            visibleText.contains("Shipv1.0release") || visibleText.contains("Shipv10release"),
            "Prefilled /goal objective must render visibly in the inspector sheet; OCR=\(visibleText)"
        )
    }

    @MainActor
    func testSettingsClearLocalDataRemovesPersistedRecords() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-clear-all-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let databaseURL = root.appendingPathComponent("fixture.sqlite")
        let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
        try runtime.createConversation(
            id: UUID().uuidString,
            title: "Conversation to Clear",
            settingsJSON: "{}",
            createdAtMs: 1,
            updatedAtMs: 1
        )
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)

        _ = try container.chatManager.updateUploadedDocuments([
            UserUploadedContextItem(name: "Temp Doc", content: "Body", description: "Desc", enabled: true)
        ], expected: container.chatMemoryProfileStore.loadProfile())
        let baseline = container.chatMemoryProfileStore.loadProfile()
        _ = try container.chatMemoryProfileStore.saveMemoryText(
            longTermMemory: "User prefers dark theme",
            userPreferences: "Concise replies",
            expected: baseline
        )
        let profileStore = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        _ = try profileStore.saveSkill(
            UserUploadedContextItem(name: "Custom Skill", content: "Instructions", description: "Desc", enabled: true),
            expected: nil
        )
        _ = try profileStore.saveMCP(
            UserMCPServer(name: "Custom MCP", endpoint: "https://mcp.example.com/sse", description: "Desc", enabled: true),
            expected: nil
        )

        let cleared = await SettingsView.clearLocalData(container: container)
        XCTAssertTrue(cleared)

        let reopenedRuntime = try IOSApplicationRuntime(databaseURL: databaseURL)
        let reopenedStore = try LocalConversationStore(applicationRuntime: reopenedRuntime)
        let reopenedContainer = ServiceContainer(applicationRuntime: reopenedRuntime)
        XCTAssertTrue(try reopenedStore.getAllConversations().isEmpty)
        XCTAssertTrue(reopenedContainer.chatMemoryProfileStore.loadProfile().uploadedDocuments.isEmpty)
        XCTAssertTrue(try reopenedRuntime.listMemories().isEmpty)
        XCTAssertTrue(try reopenedRuntime.listSkills().filter { $0.source != "built_in" }.isEmpty)
        XCTAssertTrue(try reopenedRuntime.listMCPServers().isEmpty)
    }

    private func recognizeNormalizedText(in image: UIImage) throws -> String {
        let recognition = VNRecognizeTextRequest()
        recognition.recognitionLanguages = ["en-US", "zh-Hans"]
        recognition.recognitionLevel = .accurate
        try VNImageRequestHandler(cgImage: XCTUnwrap(image.cgImage)).perform([recognition])
        return recognition.results?
            .compactMap { $0.topCandidates(1).first?.string }
            .joined()
            .filter { !$0.isWhitespace } ?? ""
    }
}

@MainActor final class IOSWorkspacePageMatrixTests: XCTestCase {
    private struct RenderMatrixVariant {
        let name: String
        let size: CGSize
        let safeArea: UIEdgeInsets
        let sizeClass: UIUserInterfaceSizeClass
        let dynamicType: DynamicTypeSize
    }

    private static let variants: [RenderMatrixVariant] = [
        RenderMatrixVariant(
            name: "compact-320",
            size: CGSize(width: 320, height: 568),
            safeArea: .zero,
            sizeClass: .compact,
            dynamicType: .large
        ),
        RenderMatrixVariant(
            name: "island-390",
            size: CGSize(width: 390, height: 844),
            safeArea: UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0),
            sizeClass: .compact,
            dynamicType: .large
        ),
        RenderMatrixVariant(
            name: "island-390-a11y3",
            size: CGSize(width: 390, height: 844),
            safeArea: UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0),
            sizeClass: .compact,
            dynamicType: .accessibility3
        ),
        RenderMatrixVariant(
            name: "regular-834",
            size: CGSize(width: 834, height: 1194),
            safeArea: .zero,
            sizeClass: .regular,
            dynamicType: .large
        )
    ]

    private func makeIsolatedFixture() throws -> (root: URL, container: ServiceContainer, conversationID: UUID) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-matrix-\(UUID())")
        try FileManager.default.createDirectory(
            at: root,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let convID = UUID()
        try runtime.createConversation(
            id: convID.uuidString,
            title: "最近的研究对话",
            settingsJSON: #"{"knowledge":{"keyPointsMarkdown":"保留有来源的结论，之后可以从这里继续。"}}"#,
            createdAtMs: 1,
            updatedAtMs: 1
        )
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)
        return (root, container, convID)
    }

    func testMainTabViewAndOnboardingRespectSafeAreaAcrossSizesThemesAndDynamicType() async throws {
        let fixture = try makeIsolatedFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }

        for dark in [false, true] {
            let style: UIUserInterfaceStyle = dark ? .dark : .light
            let themeName = dark ? "dark" : "light"

            try await IOSVisualPixelAssertions.assertHostedView(
                OnboardingView(chooseModel: {}, skip: {}),
                label: "onboarding-\(themeName)",
                size: CGSize(width: 390, height: 844),
                safeAreaInsets: UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0),
                userInterfaceStyle: style,
                horizontalSizeClass: .compact,
                dynamicTypeSize: .large
            )

            try await IOSVisualPixelAssertions.assertHostedView(
                NavigationStack { ChatView() }.environmentObject(fixture.container),
                label: "chat-view-\(themeName)",
                size: CGSize(width: 390, height: 844),
                safeAreaInsets: UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0),
                userInterfaceStyle: style,
                horizontalSizeClass: .compact,
                dynamicTypeSize: .large
            )

            for variant in Self.variants {
                for tab in NolaTab.allCases {
                    try await IOSVisualPixelAssertions.assertHostedView(
                        MainTabView(initialTab: tab).environmentObject(fixture.container),
                        label: "main-tab-\(tab.rawValue)-\(variant.name)-\(themeName)",
                        size: variant.size,
                        safeAreaInsets: variant.safeArea,
                        userInterfaceStyle: style,
                        horizontalSizeClass: variant.sizeClass,
                        dynamicTypeSize: variant.dynamicType
                    )
                }
            }
        }
    }

    func testEveryManagementDestinationAtPhoneAndTabletWidthsInBothThemes() async throws {
        let fixture = try makeIsolatedFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }

        for variant in Self.variants {
            for dark in [false, true] {
                let style: UIUserInterfaceStyle = dark ? .dark : .light
                let themeName = dark ? "dark" : "light"
                for destination in IOSWorkspaceDestination.allCases {
                    try await IOSVisualPixelAssertions.assertHostedView(
                        NavigationStack { IOSWorkspaceManagementContent(destination: destination) }
                            .environmentObject(fixture.container),
                        label: "page-\(destination)-\(variant.name)-\(themeName)",
                        size: variant.size,
                        safeAreaInsets: variant.safeArea,
                        userInterfaceStyle: style,
                        horizontalSizeClass: variant.sizeClass,
                        dynamicTypeSize: variant.dynamicType
                    )
                }
            }
        }
    }

    func testPresentedSheetsRenderCleanWarmSurfacesInBothThemes() async throws {
        let fixture = try makeIsolatedFixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }

        let sampleModel = ModelInfo(
            id: "gpt-5-preview",
            name: "GPT-5 Workspace",
            source: "configured",
            isLocal: false,
            baseURL: "https://api.example.com/v1"
        )
        let sampleDoc = UserUploadedContextItem(
            name: "Architecture Notes",
            content: "Warm Paper & Warm Ink palette spec",
            description: "Design guidelines",
            enabled: true
        )
        let sampleSkill = UserUploadedContextItem(
            name: "Release Verifier",
            content: "Run all platform checks carefully.",
            description: "Verification skill",
            enabled: true
        )

        for dark in [false, true] {
            let style: UIUserInterfaceStyle = dark ? .dark : .light
            let themeName = dark ? "dark" : "light"

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                NavigationStack {
                    ChatContextSheet(
                        displayMode: .context,
                        conversationID: fixture.conversationID,
                        conversationTitle: "最近的研究对话",
                        contextUsage: nil,
                        sessionMemory: .constant("Session notes"),
                        knowledgePoints: .constant("Key findings"),
                        conversationWiki: .constant("Wiki overview"),
                        longTermMemory: .constant("Long term notes"),
                        userPreferences: .constant("Warm Ink"),
                        mcpServers: .constant([]),
                        uploadedSkills: .constant([sampleSkill]),
                        hostUploadedSkills: [],
                        hostUploadedDocuments: [],
                        hostMCPServers: [],
                        uploadedDocuments: .constant([sampleDoc]),
                        dataSources: [],
                        selectedDataSourceIDs: .constant([]),
                        selectedSystemDataSourceTypes: .constant([]),
                        onSave: {},
                        onSaveMemoryProfile: {},
                        onSaveUploadedDocuments: { $0 },
                        onSaveConversationKnowledge: { _ in },
                        onCompact: { "Compacted" },
                        onReload: { nil },
                        onDiscard: {},
                        onUseSkill: { _ in },
                        onSearchMemory: { _ in [] }
                    )
                    .environmentObject(fixture.container)
                },
                label: "sheet-chat-context-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                ChatMoreSettingsSheet(
                    selectedModel: sampleModel,
                    contextUsage: nil,
                    hostComputerUseStatus: nil,
                    hostNativeBrowserStatus: nil,
                    pairedHostIdentity: nil,
                    conversationGenerationOverride: IOSConversationGenerationOverride(
                        maxContextLength: nil,
                        maxOutputTokens: nil
                    ),
                    onSaveGenerationOverride: { _ in },
                    hybridDeviceToolsEnabled: .constant(true),
                    hybridFilesEnabled: .constant(false),
                    hybridClipboardEnabled: .constant(false),
                    hybridLocationEnabled: .constant(false),
                    hybridCalendarEnabled: .constant(false),
                    enableRetrieval: .constant(true),
                    permissionMode: .constant(.cautious),
                    autoCompactTriggerPercent: .constant(85),
                    responseStyle: .constant(nil),
                    onOpenContextSettings: {},
                    onOpenSkillLibrary: {},
                    onForgetHostPairing: {}
                )
                .environmentObject(fixture.container),
                label: "sheet-chat-more-settings-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                ModelPickerSheet(
                    selectedModel: .constant(sampleModel),
                    availableModels: [sampleModel],
                    onSelected: { _ in }
                ),
                label: "sheet-model-picker-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                IOSConversationInspector(
                    pane: .goals,
                    conversationID: fixture.conversationID.uuidString,
                    loadProgress: { _ in
                        IOSInspectorProgressSnapshot(supportsGoalControl: true, facts: nil, plan: nil)
                    },
                    submitGoalControl: { _, _ in true },
                    documents: [sampleDoc],
                    processingState: .idle
                ),
                label: "sheet-conversation-inspector-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                IOSConversationWorkTools(
                    conversationTitle: "最近的研究对话",
                    modelLabel: "GPT-5 Workspace",
                    hasOwner: true,
                    onRename: {},
                    select: { _ in }
                ),
                label: "sheet-conversation-work-tools-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                IOSWorkspaceManagementSheet(destination: .models, notificationKey: nil)
                    .environmentObject(fixture.container),
                label: "sheet-workspace-management-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                NavigationStack {
                    PersonalCapabilityEditorView(
                        item: .skill(0, sampleSkill),
                        errorMessage: nil,
                        onSave: { _, _, _ in }
                    )
                },
                label: "sheet-personal-capability-editor-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                AddPromptTemplateSheet().environmentObject(fixture.container),
                label: "sheet-add-prompt-template-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                PermissionsSetupView().environmentObject(fixture.container),
                label: "sheet-permissions-setup-\(themeName)",
                userInterfaceStyle: style
            )

            try await IOSVisualPixelAssertions.assertPresentedSheet(
                NavigationStack {
                    WebHostConfigurationView(existingModel: nil) { _, _ in }
                },
                label: "sheet-web-host-config-\(themeName)",
                userInterfaceStyle: style
            )
        }
    }
}

@MainActor
final class WebHostConnectionUIVisualTests: XCTestCase {
    func testConnectionSheetRendersSetupAndInputsAtPhoneSize() async throws {
        #if targetEnvironment(simulator)
        // A disabled system Paste control is deliberately dim. Verify its
        // visible, actionable state with a local plaintext test item.
        UIPasteboard.general.setItems([["public.utf8-plain-text": "Nola UI fixture"]],
                                      options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)])
        let change = UIPasteboard.general.changeCount
        defer { if UIPasteboard.general.changeCount == change { UIPasteboard.general.setItems([], options: [.localOnly: true]) } }
        #endif
        let view = NavigationStack {
            WebHostConfigurationView(existingModel: nil) { _, _ in }
        }
        let image = try await IOSVisualPixelAssertions.assertHostedView(
            view,
            label: "nola-ios-web-host-connection",
            size: CGSize(width: 390, height: 844),
            userInterfaceStyle: .light,
            locale: Locale(identifier: "zh-Hans"),
            settleMilliseconds: 150
        )
        let recognition = VNRecognizeTextRequest()
        recognition.recognitionLanguages = ["zh-Hans", "en-US"]
        recognition.recognitionLevel = .accurate
        try VNImageRequestHandler(cgImage: XCTUnwrap(image.cgImage)).perform([recognition])
        let observations = recognition.results ?? []
        func text(_ line: VNRecognizedTextObservation) -> String {
            line.topCandidates(1).first?.string.lowercased().filter { !$0.isWhitespace } ?? ""
        }
        let row = try XCTUnwrap(observations.first { line in
            text(line).hasPrefix("connectionsetup") || text(line).hasPrefix("连接信息")
        }, "The setup row must have a readable title")
        let paste = observations.contains { line in
            guard abs(line.boundingBox.midY - row.boundingBox.midY) < 0.025 else { return false }
            return text(line).hasSuffix("paste") || text(line).hasSuffix("粘贴")
        }
        XCTAssertTrue(paste, "Expected Paste beside the setup title; recognized \(observations.map(text))")
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
    func testPairingFailureRestoresPreviousKeychainState() throws {
        let url = try XCTUnwrap(URL(string: "http://127.0.0.1:1/" + UUID().uuidString))
        defer { ModelSelectionManager.forgetWebHostIdentity(for: url) }
        let id = UUID().uuidString
        XCTAssertTrue(ModelSelectionManager.saveWebHostToken("previous", for: url.absoluteString))
        XCTAssertThrowsError(try ModelSelectionManager.saveVerifiedHostPairing("invalid pin", token: "new", for: url))
        XCTAssertNil(ModelSelectionManager.pairedWebHostIdentity(for: url))
        XCTAssertEqual(ModelSelectionManager.webHostToken(for: url.absoluteString), "previous")
        XCTAssertThrowsError(try ModelSelectionManager.saveVerifiedHostPairing(id, token: "new", for: url) { _, _ in false })
        XCTAssertNil(ModelSelectionManager.pairedWebHostIdentity(for: url))
        XCTAssertEqual(ModelSelectionManager.webHostToken(for: url.absoluteString), "previous")
        try ModelSelectionManager.saveVerifiedHostPairing(id, token: "verified", for: url)
        XCTAssertThrowsError(try ModelSelectionManager.saveVerifiedHostPairing(id, token: "new", for: url) { _, _ in false })
        XCTAssertThrowsError(try ModelSelectionManager.saveVerifiedHostPairing(UUID().uuidString, token: "new", for: url))
        XCTAssertEqual(ModelSelectionManager.pairedWebHostIdentity(for: url), id)
        XCTAssertEqual(ModelSelectionManager.webHostToken(for: url.absoluteString), "verified")
    }

    func testForgettingPairingRemovesOnlyItsIdentityAndToken() throws {
        let local = "http://127.0.0.1:1/" + UUID().uuidString
        let first = try XCTUnwrap(URL(string: local + "/first"))
        let second = try XCTUnwrap(URL(string: local + "/second"))
        defer { ModelSelectionManager.forgetWebHostIdentity(for: second) }
        for host in [first, second] {
            XCTAssertTrue(ModelSelectionManager.installEphemeralWebHostCredential(
                token: "fixture", identity: UUID().uuidString, for: host
            ))
            XCTAssertEqual(ModelSelectionManager.webHostToken(for: host.absoluteString), "fixture")
        }
        ModelSelectionManager.forgetWebHostIdentity(for: first)
        XCTAssertNil(ModelSelectionManager.pairedWebHostIdentity(for: first))
        XCTAssertNil(ModelSelectionManager.webHostToken(for: first.absoluteString))
        XCTAssertNotNil(ModelSelectionManager.pairedWebHostIdentity(for: second))
        XCTAssertEqual(ModelSelectionManager.webHostToken(for: second.absoluteString), "fixture")
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
