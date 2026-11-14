// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import UIKit
@testable import Nola

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
