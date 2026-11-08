// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import AppKit
@testable import Nola

final class MacInspectorProgressTests: XCTestCase {
    @MainActor
    func testStatusSymbolsDoNotClaimReportedWorkWasVerified() {
        XCTAssertEqual(MacInspectorProgressContent.symbol("reported_done"), "circle")
        XCTAssertEqual(MacInspectorProgressContent.symbol("cancelled"), "minus.circle")
        XCTAssertEqual(MacInspectorProgressContent.symbol("completed"), "checkmark.circle")
        XCTAssertEqual(MacInspectorProgressContent.symbol("blocked"), "exclamationmark.circle")
    }

    @MainActor
    func testPopulatedProgressAtNarrowWidth() throws {
        let snapshot = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(#"{"can_pause":true,"can_resume":false,"can_cancel":true,"active":{"id":"goal-fixture","objective":"整理研究资料，完成方案与验证","status":"running","requirements":{"r":{"id":"r","description":"保留来源，并确认引用可以追溯到原文","status":"blocked"}},"work":{"entries":[{"id":"w","content":"资料已整理，等待校验引用","progress":"reported_done","supersedes":null}]}}}"#.utf8))
        let todos = try JSONDecoder().decode([MacInspectorProgressSnapshot.Todo].self, from: Data(#"[{"id":"a","content":"读取资料","status":"completed"},{"id":"b","content":"核对较长的跨语言引用与上下文，确认所有结论都有来源依据","status":"in_progress"},{"id":"c","content":"交付总结","status":"pending"}]"#.utf8))
        for scheme in [ColorScheme.light, .dark] {
            let renderer = ImageRenderer(content: MacInspectorProgressContent(snapshot: snapshot, todos: todos, tasks: [])
                .font(.system(size: 13)).padding(16).frame(width: 320)
                .background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, scheme))
            let image = try XCTUnwrap(renderer.cgImage)
            XCTAssertEqual(image.width, 320)
            XCTAssertGreaterThan(image.height, 200)
            let png = try XCTUnwrap(NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]))
            try png.write(to: URL(fileURLWithPath: "/tmp/nola-progress-populated-\(scheme == .light ? "light" : "dark").png"))
        }
        let duplicate = try JSONDecoder().decode([MacInspectorProgressSnapshot.Todo].self, from: JSONEncoder().encode(todos.map { ["id": $0.id, "content": $0.content, "status": $0.status] }))
        XCTAssertEqual(todos, duplicate)
    }

    func testSameConversationRefreshRetainsContentIncludingOnFailure() {
        var state = MacInspectorRefreshState()
        XCTAssertTrue(state.begin(owner: "a"))
        state.finish(succeeded: true)
        XCTAssertFalse(state.begin(owner: "a"))
        XCTAssertTrue(state.loaded)
        state.finish(succeeded: false)
        XCTAssertTrue(state.loaded)
        XCTAssertTrue(state.failed)
        XCTAssertFalse(state.begin(owner: "a"))
        XCTAssertTrue(state.loaded)
        XCTAssertFalse(state.failed)
    }

    func testDifferentConversationNeverDisplaysPreviousFacts() {
        var state = MacInspectorRefreshState()
        _ = state.begin(owner: "a")
        state.finish(succeeded: true)
        XCTAssertTrue(state.begin(owner: "b"))
        XCTAssertFalse(state.loaded)
        state.finish(succeeded: false)
        XCTAssertFalse(state.loaded)
        XCTAssertTrue(state.failed)
    }

    func testRequirementsAndUnverifiedWorkRemainDistinct() throws {
        let json = #"{"can_pause":true,"can_resume":false,"can_cancel":true,"active":{"id":"goal-fixture","objective":"Ship","status":"running","requirements":{"r":{"id":"r","description":"Verify output","status":"blocked"}},"work":{"entries":[{"id":"old","content":"First plan","progress":"planned","supersedes":null},{"id":"new","content":"Revised plan","progress":"reported_done","supersedes":"old"}]}}}"#
        let snapshot = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(json.utf8))
        XCTAssertEqual(snapshot.active?.requirements["r"]?.description, "Verify output")
        XCTAssertEqual(snapshot.active?.requirements["r"]?.status, "blocked")
        XCTAssertEqual(snapshot.active?.work.current.map(\.id), ["new"])
        XCTAssertEqual(snapshot.active?.work.current.first?.progress, "reported_done")
    }

    func testPlanDoesNotRequireGoalAndRetainsRealStatus() throws {
        let goal = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(#"{"can_pause":false,"can_resume":false,"can_cancel":false,"active":null}"#.utf8))
        XCTAssertNil(goal.active)
        let todos = try JSONDecoder().decode([MacInspectorProgressSnapshot.Todo].self,
            from: Data(#"[{"id":"s","content":"Review","status":"in_progress"}]"#.utf8))
        XCTAssertEqual(todos.first?.content, "Review")
        XCTAssertEqual(todos.first?.status, "in_progress")
    }

    func testNeedsUserPauseUsesSourceCapabilitiesAndRejectsMissingFields() throws {
        let payload = #"{"can_pause":true,"can_resume":true,"can_cancel":true,"active":{"id":"goal","objective":"Verify","status":"needs_user","requirements":{},"work":{"entries":[]}}}"#
        let decoded = try JSONDecoder().decode(MacInspectorProgressSnapshot.self, from: Data(payload.utf8))
        XCTAssertEqual(decoded.active?.status, "needs_user")
        XCTAssertTrue(decoded.can_pause)
        XCTAssertTrue(decoded.can_resume)
        XCTAssertTrue(decoded.can_cancel)
        for key in ["can_pause", "can_resume", "can_cancel"] {
            var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(payload.utf8)) as? [String: Any])
            object.removeValue(forKey: key)
            XCTAssertThrowsError(try JSONDecoder().decode(MacInspectorProgressSnapshot.self,
                from: JSONSerialization.data(withJSONObject: object)))
        }
    }

    func testMalformedGoalIsNotAnEmptyPlan() {
        XCTAssertThrowsError(try JSONDecoder().decode(MacInspectorProgressSnapshot.self,
            from: Data(#"{"can_pause":true,"can_resume":false,"can_cancel":true,"active":{"id":"goal-fixture","objective":"Missing required state"}}"#.utf8)))
    }
}
