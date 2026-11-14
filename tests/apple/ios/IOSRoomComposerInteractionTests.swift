// SPDX-License-Identifier: Apache-2.0
import UIKit
import XCTest
@testable import Nola

@MainActor final class IOSRoomComposerInteractionTests: XCTestCase {
    func testExpertRepliesFollowTheirParentWithoutSortingCanonicalMessages() {
        struct Item { let id: String; let parent: String? }
        let ordinary = [Item(id: "user-1", parent: nil), Item(id: "answer-1", parent: nil),
                        Item(id: "user-2", parent: nil)]
        let replies = [Item(id: "expert-2", parent: "user-2"), Item(id: "expert-1", parent: "user-1")]
        let merged = IOSExpertTimelineOrder.attach(ordinary: ordinary, replies: replies,
            ordinaryID: { $0.id }, parentID: { $0.parent })
        XCTAssertEqual(merged.map(\.id), ["user-1", "expert-1", "answer-1", "user-2", "expert-2"])
        let repeated = IOSExpertTimelineOrder.attach(ordinary: ordinary, replies: replies,
            ordinaryID: { $0.id }, parentID: { $0.parent })
        XCTAssertEqual(repeated.map(\.id), merged.map(\.id))
    }

    func testConversationComposerUsesMentionKeysAndPreservesIME() throws {
        let input = ChatPasteAwareTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 80))
        input.text = "讨论 @研"
        input.mentionsVisible = true
        var actions: [String] = []
        input.mentionCommand = { actions.append($0); return true }
        for key in [UIKeyCommand.inputDownArrow, "\t", "\r", UIKeyCommand.inputEscape] {
            let command = try XCTUnwrap(input.keyCommands?.first { $0.input == key && $0.modifierFlags.isEmpty })
            _ = input.perform(command.action, with: command)
        }
        XCTAssertEqual(actions, ["down", "accept", "accept", "dismiss"])
        XCTAssertEqual(input.text, "讨论 @研")
        input.setMarkedText("研究", selectedRange: NSRange(location: 2, length: 0))
        XCTAssertFalse(input.keyCommands?.contains { $0.input == "\t" && $0.modifierFlags.isEmpty } ?? false)
        XCTAssertNil(input.inputAccessoryView)
    }

    func testSharedMentionQueryPreservesUnicodeAndRejectsEmail() throws {
        for text in ["mail@example.org", "text @name continued", "@@name"] {
            XCTAssertNil(ExpertMentionQuery.atEnd(of: text))
        }
        let text = "👩🏽‍💻 讨论 @研究"
        let query = try XCTUnwrap(ExpertMentionQuery.atEnd(of: text))
        XCTAssertEqual(query.query, "研究")
        XCTAssertEqual(query.removing(from: text), "👩🏽‍💻 讨论 ")
        XCTAssertEqual(ExpertMentionQuery.atEnd(of: "@")?.query, "")
    }

    func testNativeKeyboardCommandsSelectMentionsWithoutSending() throws {
        let input = IOSRoomComposerTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 80))
        input.text = "Discuss @"
        input.mentionsVisible = true
        var actions: [String] = [], sends = 0
        input.mentionCommand = { actions.append($0); return true }
        input.sendMessage = { sends += 1 }
        for key in [UIKeyCommand.inputDownArrow, UIKeyCommand.inputUpArrow, "\t", "\r", UIKeyCommand.inputEscape] {
            let command = try XCTUnwrap(input.keyCommands?.first { $0.input == key && $0.modifierFlags.isEmpty })
            _ = input.perform(command.action, with: command)
        }
        XCTAssertEqual(actions, ["down", "up", "accept", "accept", "dismiss"])
        XCTAssertEqual(input.text, "Discuss @")
        XCTAssertEqual(sends, 0)
    }

    func testMarkedTextAndHiddenCandidatesDoNotHijackEditingOrSend() throws {
        let input = IOSRoomComposerTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 80))
        var calls = 0
        input.mentionCommand = { _ in calls += 1; return true }
        input.sendMessage = { calls += 1 }
        input.mentionsVisible = true
        input.setMarkedText("研究", selectedRange: NSRange(location: 2, length: 0))
        XCTAssertNotNil(input.markedTextRange)
        input.chooseMention(UIKeyCommand(input: "\r", modifierFlags: [], action: #selector(IOSRoomComposerTextView.chooseMention(_:))))
        input.sendFromKeyboard()
        XCTAssertEqual(calls, 0)
        input.unmarkText()
        input.mentionsVisible = false
        XCTAssertFalse(input.keyCommands?.contains { $0.modifierFlags.isEmpty && $0.input == "\t" } ?? false)
        input.insertText("\n")
        XCTAssertTrue(input.text.hasSuffix("\n"))
        input.sendFromKeyboard()
        XCTAssertEqual(calls, 1)
    }
}
