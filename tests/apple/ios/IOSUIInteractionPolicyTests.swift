import XCTest
import UIKit
@testable import Nola

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
