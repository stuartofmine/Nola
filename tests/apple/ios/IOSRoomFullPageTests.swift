// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import UIKit
@testable import Nola

@MainActor final class IOSRoomFullPageTests: XCTestCase {
    private func settle() async throws { try await Task.sleep(for: .milliseconds(200)) }
    private func input(in view: UIView) -> IOSRoomComposerTextView? {
        if let input = view as? IOSRoomComposerTextView { return input }
        return view.subviews.lazy.compactMap { self.input(in: $0) }.first
    }
    private func scrollViews(in view: UIView) -> [UIScrollView] {
        ((view as? UIScrollView).map { [$0] } ?? [])
            + view.subviews.flatMap { scrollViews(in: $0) }
    }

    func testFullRoomCandidatesPermissionProcessingAndRoomSwitch() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("room-full-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700])
        defer {
            if FileManager.default.fileExists(atPath: root.path) { try? FileManager.default.removeItem(at: root) }
        }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let container = ServiceContainer(applicationRuntime: runtime)
        container.conversationStore = try LocalConversationStore(applicationRuntime: runtime)
        container.chatManager = ChatManager(container: container, restorePersistedState: false)
        let service = IOSAgentRoomService(runtime: runtime)
        let directory = try service.createAgent(name: "研究助手 · 资料核查", description: "核对资料与引用，不访问其他成员私有上下文",
            defaults: .object([:]), directory: service.directory())
        let first = try XCTUnwrap(directory.agents.first)
        let all = try service.createAgent(name: "写作助手 · 结构表达", description: "整理结论并检查表达",
            defaults: .object([:]), directory: directory)
        let second = try XCTUnwrap(all.agents.first { $0.id != first.id })
        let a = try service.join(agent: second, room: service.join(agent: first, room: service.create(title: "青山讨论室")))
        let b = try service.join(agent: first, room: service.create(title: "另一间独立讨论室"))
        let workspace = IOSAgentRoomWorkspace()
        try workspace.open(a, container: container)
        let member = try XCTUnwrap(a.room.participants.first)
        _ = try service.submit(.init(roomID: a.room.id, submissionID: UUID().uuidString.lowercased(),
            expectedRoomRevision: a.revision, text: "隔离样本：请比较两份设计说明。未调用模型，也不会执行工具。",
            participantID: member.id, atMs: 1))
        // No executor or routing provider is installed: sends persist a receipt only.
        let submission = IOSRoomSubmissionWorkspace(service: service)
        await submission.select(a)
        let execution = try XCTUnwrap(container.chatManager.ensureExecutionForConversation(
            try XCTUnwrap(UUID(uuidString: member.contextConversationID))))
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        for width in [CGFloat(390), CGFloat(834)] {
            for dark in [false, true] {
                let view = NavigationStack {
                    IOSRoomConversationView(workspace: workspace, submission: submission)
                        .navigationTitle("青山讨论室").navigationBarTitleDisplayMode(.inline)
                }.environmentObject(container).environment(\.locale, Locale(identifier: "zh-Hans"))
                    .environment(\.colorScheme, dark ? .dark : .light)
                let controller = UIHostingController(rootView: view)
                let window = UIWindow(windowScene: scene)
                window.frame = CGRect(x: 0, y: 0, width: width, height: width == 390 ? 844 : 1112)
                window.overrideUserInterfaceStyle = dark ? .dark : .light
                window.rootViewController = controller
                window.makeKeyAndVisible()
                controller.view.frame = window.bounds
                try await settle()
                submission.edit("请核对这份隔离样本 @")
                try await settle()
                let composer = try XCTUnwrap(input(in: controller.view))
                XCTAssertTrue(composer.mentionsVisible)
                XCTAssertGreaterThanOrEqual(composer.bounds.height, 64)
                XCTAssertTrue(window.bounds.contains(composer.convert(composer.bounds, to: window)))
                capture(window, name: "room-full-candidates-\(Int(width))-\(dark ? "dark" : "light")")
                // Exercise the real mounted UITextView command and SwiftUI binding.
                composer.chooseMention(UIKeyCommand(input: UIKeyCommand.inputDownArrow, modifierFlags: [], action: #selector(IOSRoomComposerTextView.chooseMention(_:))))
                composer.chooseMention(UIKeyCommand(input: "\t", modifierFlags: [], action: #selector(IOSRoomComposerTextView.chooseMention(_:))))
                try await settle()
                XCTAssertNotNil(submission.draft.participantID)
                XCTAssertNil(submission.draft.mentionQuery)
                composer.resignFirstResponder()
                window.endEditing(true)
                // Keyboard dismissal animates independently from SwiftUI state.
                try await Task.sleep(for: .milliseconds(600))
                execution.processingState = .streaming(progress: 0)
                execution.pendingHostPermissionRequest = HostAgentPermissionRequest(id: "fixture-permission", allowConversation: false,
                    toolName: "write_file", argsPreview: "path: fixture.txt; UTF-8 bytes: 8; isolated fixture only",
                    match: "fixture.txt", reason: "隔离页面验收：请确认是否允许写入示例文件。", message: "只展示授权，不执行工具。")
                try await settle()
                let permissionComposer = try XCTUnwrap(input(in: controller.view))
                XCTAssertGreaterThanOrEqual(permissionComposer.bounds.height, 64)
                XCTAssertTrue(window.bounds.contains(permissionComposer.convert(permissionComposer.bounds, to: window)),
                    "The input must remain inside the full page while a member needs authorization")
                capture(window, name: "room-full-permission-\(Int(width))-\(dark ? "dark" : "light")")
                let permissionScroll = try XCTUnwrap(scrollViews(in: controller.view).first {
                    abs($0.bounds.width - 300) < 1 && $0.contentSize.height > $0.bounds.height
                })
                let bottom = permissionScroll.contentSize.height - permissionScroll.bounds.height
                    + permissionScroll.adjustedContentInset.bottom
                permissionScroll.setContentOffset(CGPoint(x: 0, y: bottom), animated: false)
                try await settle()
                XCTAssertEqual(permissionScroll.contentOffset.y, bottom, accuracy: 1)
                capture(window, name: "room-full-permission-actions-\(Int(width))-\(dark ? "dark" : "light")")
                execution.pendingHostPermissionRequest = nil
                execution.processingState = .idle
                if width == 390 && !dark {
                    let previous = submission.submissions[a.room.id]?.count ?? 0
                    composer.sendFromKeyboard()
                    for _ in 0..<30 where (submission.submissions[a.room.id]?.count ?? 0) == previous { try await settle() }
                    XCTAssertEqual(submission.submissions[a.room.id]?.count, previous + 1)
                    XCTAssertEqual(submission.submissions[a.room.id]?.last?.input.text, "请核对这份隔离样本 ")
                    submission.edit("A 的独立草稿")
                    try workspace.open(b, container: container)
                    try await settle()
                    XCTAssertEqual(submission.selectedRoom?.room.id, b.room.id)
                    let otherComposer = try XCTUnwrap(input(in: controller.view))
                    XCTAssertEqual(otherComposer.text, "")
                    XCTAssertFalse(otherComposer.mentionsVisible)
                    XCTAssertFalse(workspace.room?.room.participants.contains { $0.contextConversationID == member.contextConversationID } ?? true)
                    submission.edit("B 的独立草稿")
                    try workspace.open(a, container: container)
                    try await settle()
                    XCTAssertEqual(try XCTUnwrap(input(in: controller.view)).text, "A 的独立草稿")
                    XCTAssertEqual(submission.drafts[b.room.id]?.text, "B 的独立草稿")
                }
                submission.removeMention()
                window.isHidden = true
                window.rootViewController = nil
            }
        }
        workspace.close()
        try await settle()
    }

    private func capture(_ window: UIWindow, name: String) {
        window.layoutIfNeeded()
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
