import XCTest
import AppKit
import Combine
import SwiftUI
@testable import Nola

@MainActor
final class MacComposerRecoveryStateTests: XCTestCase {

    func testHeldReturnDoesNotSubmitOrAcceptAnotherSuggestion() throws {
        let editor = MacAttachmentPasteTextView()
        var sends = 0
        var accepts = 0
        editor.submitHandler = { sends += 1 }
        editor.mentionHandler = { _ in accepts += 1; return true }
        for code in [UInt16(36), UInt16(76)] {
            let repeated = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
                characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: true, keyCode: code))
            editor.keyDown(with: repeated)
        }
        XCTAssertEqual(sends, 0)
        XCTAssertEqual(accepts, 0)
        let press = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
        editor.keyDown(with: press)
        XCTAssertEqual(accepts, 1)
        editor.mentionHandler = { _ in false }
        editor.keyDown(with: press)
        XCTAssertEqual(sends, 1)
    }


    func testUnresolvedRunBlocksNewSubmissionButKeepsDraft() {
        var state = MacComposerDisplayState(conversationID: "conversation", draft: "next question",
            attachments: [], isRunning: false, isVisiblyRunning: false, isSubmissionBlocked: true)
        XCTAssertFalse(state.canSend)
        XCTAssertTrue(state.isPrimaryActionDisabled)
        XCTAssertEqual(state.draft, "next question")
        state.isSubmissionBlocked = false
        XCTAssertTrue(state.canSend)
        XCTAssertFalse(state.isPrimaryActionDisabled)
    }

    func testSubmissionBlockDoesNotDisableStopForAnExecutingRun() {
        let state = MacComposerDisplayState(conversationID: "conversation", draft: "",
            attachments: [], isRunning: true, isVisiblyRunning: true, isSubmissionBlocked: true)
        XCTAssertFalse(state.canSend)
        XCTAssertFalse(state.isPrimaryActionDisabled)
    }
}

@MainActor
private final class AttachmentLayoutState: ObservableObject {
    @Published var attachments: [MacAttachment] = []
}

private struct AttachmentLayoutHarness: View {
    @ObservedObject var state: AttachmentLayoutState
    @FocusState private var focused: Bool
    private let actions = MacComposerActionSink { _ in true }

    var body: some View {
        MacComposerView(state: .init(conversationID: "layout-fixture", draft: "",
            attachments: state.attachments, isRunning: false, isVisiblyRunning: false),
            actions: actions, composerFocused: $focused)
            .frame(width: 680)
            .padding(10)
    }
}
