// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS chat bubble layout, capability presentation, assistant presentation session/visibility, code references, motion, streaming reveal, tool presentation, scroll, and surface context suite.

import AppKit
import Combine
import Darwin
import Foundation
import ObjectiveC.runtime
import SwiftUI
import XCTest
@testable import Nola

final class MacChatBubbleCapabilityAndVisibilitySuiteTests: XCTestCase {
    @MainActor
    func testLongDraftInsertionRestoresCaretOnUndoAndRedo() async throws {
        final class Draft: ObservableObject {
            @Published var text = (1...18).map { "第\($0)行 📝" }.joined(separator: "\n")
            @Published var caret: AppleMentionEditor.Caret?
        }
        struct Editor: View {
            @ObservedObject var draft: Draft
            @FocusState var focused: Bool
            var body: some View {
                MacComposerTextEditor(text: $draft.text, conversationID: "test", focused: $focused,
                    onPasteAttachments: { false }, onDropAttachments: { _ in false }, onSubmit: {},
                    mentionEditor: .init(caretRequest: draft.caret, restore: { value, _ in draft.text = value }))
            }
        }
        let draft = Draft()
        let original = draft.text
        let hosting = NSHostingView(rootView: Editor(draft: draft).frame(width: 320, height: 96))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 96),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil }
        func settle() async throws {
            hosting.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(80))
        }
        func findInput(in view: NSView) -> NSTextView? {
            if let input = view as? NSTextView { return input }
            return view.subviews.lazy.compactMap { findInput(in: $0) }.first
        }
        try await settle()
        let input = try XCTUnwrap(findInput(in: hosting))
        XCTAssertTrue(window.makeFirstResponder(input))
        input.setSelectedRange(NSRange(location: 2, length: 0))
        let previous = AppleMentionEditor.Snapshot(text: original, mentions: [], caret: 2)
        let appended = ComposerSuggestion.inserting("请继续", into: original)
        draft.text = appended
        draft.caret = .init(location: appended.utf16.count, before: previous)
        try await settle()
        XCTAssertEqual(input.selectedRange().location, appended.utf16.count)
        let scrolledToEnd = input.visibleRect.minY
        XCTAssertGreaterThan(scrolledToEnd, 0)
        let undo = try XCTUnwrap(input.undoManager)
        undo.undo()
        try await settle()
        XCTAssertEqual(draft.text, original)
        XCTAssertEqual(input.selectedRange().location, 2)
        XCTAssertLessThan(input.visibleRect.minY, scrolledToEnd)
        undo.redo()
        try await settle()
        input.insertText("。", replacementRange: input.selectedRange())
        XCTAssertEqual(input.string, appended + "。")
    }

    @MainActor
    func testStarterPromptsReflowAndRemainClickable() async throws {
        for width: CGFloat in [320, 760] {
            var selected: [String] = []
            let root = MacEmptyConversationView { selected.append($0) }
                .frame(width: width, height: 400)
                .background(Color("NolaCanvas"))
            let host = NSHostingView(rootView: root)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 400),
                                  styleMask: [.titled], backing: .buffered, defer: false)
            window.contentView = host
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil) }
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(150))

            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let image = NSImage(size: host.bounds.size)
            image.addRepresentation(bitmap)
            let attachment = XCTAttachment(image: image)
            attachment.name = "starter-prompts-\(Int(width))"
            attachment.lifetime = .keepAlways
            add(attachment)
            func click(x: CGFloat, y: CGFloat) async throws {
                let point = NSPoint(x: x, y: y)
                for kind in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                    window.sendEvent(try XCTUnwrap(NSEvent.mouseEvent(with: kind, location: point, modifierFlags: [],
                        timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                        context: nil, eventNumber: 0, clickCount: 1, pressure: 1)))
                }
                await Task.yield()
            }
            // Sample a vertical line through the compact stack, or a horizontal
            // line through the wide row. Count unique actions that actually fire.
            var rowY: CGFloat?
            for y in stride(from: CGFloat(16), to: 384, by: 16) {
                try await click(x: width / 2, y: y)
                if width > 320, !selected.isEmpty { rowY = y; break }
            }
            if width > 320 {
                let y = try XCTUnwrap(rowY)
                for x in stride(from: CGFloat(16), to: width, by: 16) {
                    try await click(x: x, y: y)
                }
            }
            XCTAssertEqual(Set(selected).count, 3, "All suggestions must respond along the visible row or column")
        }
    }

    func testChatBubbleLayoutAndCapabilityActionPresentation() {
        XCTAssertEqual(MacDesignTokens.Layout.chatProseMaxWidth, 720)
        XCTAssertLessThanOrEqual(MacDesignTokens.Layout.chatProseMaxWidth, MacChatBubbleLayout.transcriptWidth(availableWidth: 1_440))
        XCTAssertGreaterThanOrEqual(MacDesignTokens.Control.iconButton, 32)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 520), 472, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 900), 720, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 1_440), 720, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 2_200), 720, accuracy: 0.1)
        var previous: CGFloat = 0
        for width in stride(from: 0, through: 2_400, by: 1) {
            let column = MacChatBubbleLayout.transcriptWidth(availableWidth: CGFloat(width))
            XCTAssertGreaterThanOrEqual(column, previous)
            XCTAssertLessThanOrEqual(column, max(0, CGFloat(width) - 48))
            XCTAssertLessThanOrEqual(column, 720)
            previous = column
        }
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 0), 0)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 200), 200)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 472), 415.36, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 1_200), 620, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 2_048), 620, accuracy: 0.1)

        for kind in [MacCapabilitySnapshot.Kind.files, .shell, .git, .network, .browser] {
            let capability = MacCapabilitySnapshot(kind: kind, title: "Runtime capability", detail: "Resolved per run", symbol: "circle", state: .optional)
            XCTAssertFalse(capability.hasUserAction, "\(kind) exposed a button without an action")
        }
        XCTAssertTrue(MacCapabilitySnapshot(kind: .accessibility, title: "Accessibility", detail: "Required", symbol: "accessibility", state: .permissionRequired).hasUserAction)
        XCTAssertTrue(MacCapabilitySnapshot(kind: .automation, title: "Automation", detail: "Optional", symbol: "app.badge.checkmark", state: .optional).hasUserAction)
    }

    func testAssistantPresentationFieldVisibilityPlaceholderAndTimelineGrouping() {
        for field in ["title", "name", "citation_section", "source_snippet"] {
            XCTAssertTrue(MacJournalSourceFieldPresentation.isPrimary(field))
        }
        for field in ["uri", "source_evidence", "mime_type", "future_field", ""] {
            XCTAssertFalse(MacJournalSourceFieldPresentation.isPrimary(field))
        }
        for field in ["tool_summary", "tool_artifacts"] {
            XCTAssertTrue(MacJournalToolFieldPresentation.isResult(field))
        }
        for field in ["tool_input", "tool_output", "tool_output_text", "tool_resources", "permission_decision", "permission_rationale", "future_field", ""] {
            XCTAssertFalse(MacJournalToolFieldPresentation.isResult(field))
        }
        for state in [MacAssistantTrace.State.completed, .cancelled] {
            let placeholder = MacAssistantTrace(kind: .thinking, title: "正在思考中", detail: "  ", state: state)
            XCTAssertFalse(MacAssistantArtifactPresentationPolicy.shouldShowInProcess(placeholder))
        }
        for state in [MacAssistantTrace.State.running, .failed, .incomplete] {
            let trace = MacAssistantTrace(kind: .thinking, title: "Thinking", detail: "", state: state)
            XCTAssertTrue(MacAssistantArtifactPresentationPolicy.shouldShowInProcess(trace))
        }
        XCTAssertTrue(MacAssistantArtifactPresentationPolicy.shouldShowInProcess(MacAssistantTrace(kind: .thinking, title: "Thinking", detail: "Analysis", state: .completed)))

        let emptyAssistant = MacChatMessage(role: .assistant, text: "", createdAt: Date(), presentation: .processing())
        XCTAssertTrue(macChatMessageIsVisible(emptyAssistant, isActive: true))
        XCTAssertFalse(macChatMessageIsVisible(emptyAssistant, isActive: false))

        var withThinking = MacAssistantPresentation.processing()
        withThinking.appendThinking("Inspecting the request", replacing: false)
        XCTAssertTrue(macChatMessageIsVisible(MacChatMessage(role: .assistant, text: "", createdAt: Date(), presentation: withThinking), isActive: false))

        func makePresentation() -> MacAssistantPresentation {
            var value = MacAssistantPresentation.processing()
            value.upsertCoreThinking(partID: "reasoning-1", title: "Thinking", detail: "Inspecting the runtime", isComplete: true)
            value.upsertTool(externalID: "tool-1", title: "read_file", detail: "Read runtime.swift", state: .completed)
            return value
        }
        let first = makePresentation().traceItems
        let second = makePresentation().traceItems
        XCTAssertEqual(first.map(\.id), second.map(\.id))
        XCTAssertEqual(first.map(\.startedAt), second.map(\.startedAt))
        XCTAssertEqual(first.map(\.finishedAt), second.map(\.finishedAt))

        var multiPresentation = MacAssistantPresentation.processing()
        multiPresentation.upsertCoreThinking(partID: "reasoning-1", title: "Thinking", detail: "Inspecting", isComplete: true)
        multiPresentation.upsertTool(externalID: "tool-1", title: "read_file", detail: "One", state: .completed)
        multiPresentation.upsertCoreThinking(partID: "reasoning-2", title: "Thinking", detail: "Comparing", isComplete: true)
        multiPresentation.upsertTool(externalID: "tool-2", title: "grep", detail: "Two", state: .running)
        XCTAssertEqual(MacAssistantTraceTimelinePolicy.visibleDetailTraces(multiPresentation.traceItems).compactMap(\.externalID),
                       ["reasoning-1", "tool-1", "reasoning-2", "tool-2"])

        let traces = [
            MacAssistantTrace(externalID: "tree-1", kind: .tool, title: "file_tree", detail: "First root", state: .completed),
            MacAssistantTrace(externalID: "thinking-1", kind: .thinking, title: "Thinking", detail: "Comparing roots", state: .completed),
            MacAssistantTrace(externalID: "tree-2", kind: .tool, title: "file_tree", detail: "Second root", state: .running),
            MacAssistantTrace(externalID: "glob-1", kind: .tool, title: "glob", detail: "Swift files", state: .completed),
        ]
        let groups = MacAssistantTraceTimelinePolicy.groupedDetailTraces(traces)
        XCTAssertEqual(groups.count, 4)
        XCTAssertEqual(groups.flatMap(\.traces).compactMap(\.externalID), ["tree-1", "thinking-1", "tree-2", "glob-1"])
        XCTAssertEqual(groups[2].state, .running)
    }

    @MainActor
    func testAssistantPresentationSessionAndToolVisualContracts() throws {
        // Session append, interrupt, replace, parser completion, and lease rules
        let s1 = MacAssistantPresentationSession(identitySeed: "message")
        let accepted = "# Heading\n\nA paragraph accepted before the synchronous adapter runs."
        _ = s1.accept(authoritative: accepted)
        let old1 = s1.beginReveal()
        _ = s1.nextFrame(lease: old1)
        XCTAssertNotEqual(s1.visibleText, accepted)
        let suffix = "\n\nThe appended final paragraph."
        let frame = s1.appendImmediately(suffix)
        XCTAssertEqual(frame.text, accepted + suffix)
        XCTAssertEqual(s1.visibleText, s1.acceptedText)
        XCTAssertEqual(s1.assembler.rawText, accepted + suffix)
        XCTAssertEqual(s1.pendingSegmentCount, 0)
        XCTAssertNil(s1.nextFrame(lease: old1))
        XCTAssertNil(s1.accept(authoritative: accepted + suffix))
        s1.flushPending()
        XCTAssertNil(s1.nextFrame(lease: s1.beginReveal()))

        let longText = (1...30).map { "Paragraph \($0) contains accepted but not yet visible text.\n" }.joined()
        let s2 = MacAssistantPresentationSession(identitySeed: "message")
        XCTAssertNil(s2.accept(authoritative: longText))
        s2.flushPending()
        let old2 = s2.beginReveal()
        XCTAssertNotNil(s2.nextFrame(lease: old2))
        let prefix = s2.visibleText
        XCTAssertNotEqual(prefix, longText)
        XCTAssertEqual(s2.acceptedText, longText)
        XCTAssertNil(s2.accept(authoritative: longText))
        let resumed2 = s2.beginReveal()
        XCTAssertNil(s2.nextFrame(lease: old2))
        while s2.nextFrame(lease: resumed2) != nil {}
        XCTAssertEqual(s2.visibleText, longText)
        XCTAssertEqual(s2.pendingSegmentCount, 0)
        XCTAssertTrue(s2.visibleText.hasPrefix(prefix))

        let s3 = MacAssistantPresentationSession(identitySeed: "message")
        _ = s3.accept(authoritative: "Original long answer with a queued suffix that must not return.")
        s3.flushPending()
        let old3 = s3.beginReveal()
        _ = s3.nextFrame(lease: old3)
        let replacement = s3.accept(authoritative: "Corrected answer")
        XCTAssertEqual(replacement?.text, "Corrected answer")
        XCTAssertTrue(replacement?.replacesText == true)
        XCTAssertNil(s3.nextFrame(lease: old3))
        XCTAssertEqual(s3.pendingSegmentCount, 0)
        XCTAssertEqual(s3.visibleText, "Corrected answer")
        _ = s3.accept(authoritative: "Corrected answer with continuation")
        s3.flushPending()
        let next3 = s3.beginReveal()
        while s3.nextFrame(lease: next3) != nil {}
        XCTAssertEqual(s3.visibleText, "Corrected answer with continuation")

        let s4 = MacAssistantPresentationSession(identitySeed: "message")
        _ = s4.accept(authoritative: "# Heading\n\nFirst paragraph.\n\nUnfinished last line")
        s4.flushPending()
        let lease4 = s4.beginReveal()
        var committed: [MacAssistantTextNode] = []
        while let f = s4.nextFrame(lease: lease4) { committed += f.nodes }
        let tail = s4.assembler.mutableTail
        XCTAssertFalse(committed.isEmpty)
        let completion = s4.finishFrame(lease: lease4)
        XCTAssertEqual(completion?.nodes.last?.id, tail?.id)
        XCTAssertTrue(completion?.isComplete == true)
        XCTAssertNil(s4.finishFrame(lease: lease4))
        XCTAssertEqual(s4.assembler.rawText, s4.visibleText)

        let s5 = MacAssistantPresentationSession(identitySeed: "message")
        _ = s5.accept(authoritative: "A pending sentence that has not been revealed.")
        s5.flushPending()
        let old5 = s5.beginReveal()
        XCTAssertNil(s5.finishFrame(lease: old5))
        let current5 = s5.beginReveal()
        while s5.nextFrame(lease: current5) != nil {}
        XCTAssertNil(s5.finishFrame(lease: old5))
        XCTAssertNotNil(s5.finishFrame(lease: current5))

        // Tool presentation visual contracts
        for state in ["running", "completed", "succeeded", "failed", "waiting", "cancelled"] {
            let trace = MacAssistantTrace(kind: .tool, title: "web_search", detail: state, state: state == "running" ? .running : .completed)
            XCTAssertNil(MacToolTraceDisplayPolicy.headerDetail(for: trace))
        }
        func projectedTool(title: String, capability: String, origin: [String: String] = ["kind": "builtin"]) throws -> MacAssistantTrace {
            let message: [String: Any] = ["message_id": "assistant", "role": "assistant", "state": "running",
                "revision": 1, "created_at_ms": 1, "parts": [["kind": "tool", "execution_source": ["kind": "model"], "part_id": "tool", "revision": 1,
                "tool_call_id": "call", "capability_id": capability, "title": title, "state": "running",
                "input": [:], "summary": "", "origin": origin, "artifacts": [], "affected_resources": []]]]
            let decoded = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self, from: JSONSerialization.data(withJSONObject: message))
            return try XCTUnwrap(MacCanonicalConversationRepository.projectMessage(decoded).presentation?.traceItems.first)
        }
        let toolTrace = try projectedTool(title: "Tool", capability: "write_file")
        XCTAssertEqual(toolTrace.title, "Tool")
        XCTAssertEqual(toolTrace.capabilityID, "write_file")
        XCTAssertEqual(MacToolTraceDisplayPolicy.operationTitle(for: toolTrace), "Tool")
        XCTAssertNil(MacToolTraceDisplayPolicy.headerDetail(for: toolTrace))
        XCTAssertEqual(MacToolPresentationPolicy.displayTitle(for: toolTrace), MacToolPresentationPolicy.displayTitle(for: "write_file"))
        XCTAssertNotEqual(MacToolPresentationPolicy.displayTitle(for: "tool_search"), "tool_search")
        for name in ["write_file", "read_file", "append_file", "bash", "web_search"] {
            let t = try projectedTool(title: name, capability: name)
            let shown = MacToolPresentationPolicy.displayTitle(for: t)
            XCTAssertEqual(shown, MacToolPresentationPolicy.displayTitle(for: name))
            XCTAssertNotEqual(shown, name)
            XCTAssertEqual(t.title, name)
            XCTAssertEqual(t.capabilityID, name)
        }
        let mcpTrace = try projectedTool(title: "Export my research", capability: "vendor_export", origin: ["kind": "mcp", "provider_id": "research"])
        XCTAssertEqual(MacToolPresentationPolicy.displayTitle(for: mcpTrace), "Export my research")
        XCTAssertEqual(mcpTrace.capabilityID, "vendor_export")
        let skillTrace = try projectedTool(title: "Create Word document", capability: "word-document-authoring",
            origin: ["kind": "skill", "provider_id": "word-document-authoring", "provider_title": "Word"])
        XCTAssertEqual(skillTrace.title, "Create Word document")
        XCTAssertEqual(MacToolPresentationPolicy.displayTitle(for: skillTrace), "Create Word document")
        XCTAssertNil(MacToolTraceDisplayPolicy.headerDetail(for: MacAssistantTrace(kind: .tool, title: "web_search", detail: #"{"text":"large tool payload…"#, state: .completed)))
        let builtinTitle = MacToolPresentationPolicy.displayTitle(for: "word-document-authoring")
        XCTAssertEqual(builtinTitle, MacBuiltInSkillRegistry.descriptors.first(where: { $0.id == .word })?.title)
        XCTAssertFalse(builtinTitle.contains("Unknown"))
        XCTAssertFalse(builtinTitle.contains("未知"))
    }
}

final class MacCodeReferenceTests: XCTestCase {
    @MainActor
    func testPreparedPaletteMatchesPerRunAppearanceResolution() throws {
        let line = "let answer = 42; print(\"你好👩🏽‍💻\") // comment"
        let document = MacCodeReferenceDocument(url: URL(fileURLWithPath: "/inline.swift"),
            lines: Array(repeating: line, count: 250), byteCount: line.utf8.count * 250,
            encodingName: "UTF-8")
        for name in [NSAppearance.Name.aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua] {
            let appearance = try XCTUnwrap(NSAppearance(named: name))
            var expected = NSAttributedString(string: "")
            appearance.performAsCurrentDrawingAppearance {
                let base = MacCodeSyntaxHighlighter.attributedText(for: document, targetLine: 125)
                let resolved = NSMutableAttributedString(attributedString: base)
                base.enumerateAttributes(in: NSRange(location: 0, length: base.length)) { attributes, range, _ in
                    for key in [NSAttributedString.Key.foregroundColor, .backgroundColor] {
                        if let color = attributes[key] as? NSColor,
                           let rgb = color.usingColorSpace(.deviceRGB) {
                            resolved.addAttribute(key, value: rgb, range: range)
                        }
                    }
                }
                expected = resolved
            }
            let started = SuspendingClock.now
            let actual = MacCodeSyntaxHighlighter.attributedText(for: document, targetLine: 125, appearance: appearance)
            print("prepared_code_palette appearance=\(name.rawValue) lines=250 elapsed=\(started.duration(to: .now))")
            XCTAssertTrue(actual.isEqual(to: expected), "Palette preparation must preserve every font, token color, line number and target background")
        }
    }

    @MainActor
    func testInlineHighlightCacheReusesOnlyCurrentSourceLanguageAndAppearance() {
        let cache = MacInlineCodeHighlightCache()
        let source = "let answer = 42\r\n// 你好👩🏽‍💻"
        let first = cache.text(source: source, fileExtension: "swift", dark: true)
        for _ in 0..<20 {
            XCTAssertEqual(cache.text(source: source, fileExtension: "swift", dark: true), first)
        }
        XCTAssertEqual(cache.renderCount, 1, "Layout updates must reuse highlighted content")
        XCTAssertTrue(String(first.characters).contains("\n  2  // 你好👩🏽‍💻"))
        let light = cache.text(source: source, fileExtension: "swift", dark: false)
        XCTAssertNotEqual(first, light, "Appearance changes must resolve colors again")
        _ = cache.text(source: source, fileExtension: "py", dark: false)
        let appended = cache.text(source: source + "\nprint(answer)", fileExtension: "py", dark: false)
        XCTAssertTrue(String(appended.characters).contains("print(answer)"))
        XCTAssertEqual(cache.renderCount, 4)
        XCTAssertEqual(cache.text(source: source, fileExtension: "swift", dark: true), first)
        XCTAssertEqual(cache.renderCount, 5, "Old streamed prefixes must not accumulate in the cache")
    }

    @MainActor
    func testLongCodeHighlightingPreservesTokensUnicodeAndLineNumbers() throws {
        for (suffix, keyword, comment) in [("swift", "let", "//"), ("py", "def", "#"),
                                             ("ts", "const", "//"), ("cpp", "return", "//")] {
            let line = "\(keyword) value = 42; \"你好👩🏽‍💻\" \(comment) note"
            let document = MacCodeReferenceDocument(url: URL(fileURLWithPath: "/inline.\(suffix)"),
                lines: Array(repeating: line, count: 500), byteCount: line.utf8.count * 500,
                encodingName: "UTF-8")
            let started = SuspendingClock.now
            let result = MacCodeSyntaxHighlighter.attributedText(for: document, targetLine: 500)
            print("long_code_highlighting language=\(suffix) lines=500 elapsed=\(started.duration(to: .now))")
            let lines = result.string.components(separatedBy: "\n")
            XCTAssertEqual(lines.count, 500)
            XCTAssertEqual(lines.first, "  1  \(line)")
            XCTAssertEqual(lines.last, "500  \(line)")
            for (token, expected) in [(keyword, NSColor.systemPurple), ("42", .systemBlue),
                                       ("你好👩🏽‍💻", .systemRed), ("note", .systemGreen)] {
                let range = (result.string as NSString).range(of: token)
                XCTAssertNotEqual(range.location, NSNotFound)
                let actual = try XCTUnwrap(result.attribute(.foregroundColor,
                    at: range.location, effectiveRange: nil) as? NSColor)
                XCTAssertEqual(actual, expected)
            }
            let finalLine = (result.string as NSString).range(of: "500  ")
            XCTAssertNotNil(result.attribute(.backgroundColor, at: finalLine.location, effectiveRange: nil))
        }
    }

    @MainActor
    func testCodeForegroundResolvesAgainstExplicitAppearanceNotAmbientAppearance() throws {
        let document = MacCodeReferenceDocument(
            url: URL(fileURLWithPath: "/inline.swift"),
            lines: ["let answer = 42", "print(answer)"],
            byteCount: 28,
            encodingName: "UTF-8"
        )
        for dark in [false, true] {
            let requested = try XCTUnwrap(NSAppearance(named: dark ? .darkAqua : .aqua))
            let opposite = try XCTUnwrap(NSAppearance(named: dark ? .aqua : .darkAqua))
            var text = NSAttributedString(string: "")
            opposite.performAsCurrentDrawingAppearance {
                text = MacCodeSyntaxHighlighter.attributedText(
                    for: document, targetLine: nil, appearance: requested
                )
            }
            XCTAssertTrue(text.string.contains("print(answer)"))
            for token in ["answer", "print", "1"] {
                let range = (text.string as NSString).range(of: token)
                let color = try XCTUnwrap(text.attribute(.foregroundColor, at: range.location, effectiveRange: nil) as? NSColor)
                let rgb = try XCTUnwrap(color.usingColorSpace(.deviceRGB))
                let brightness = (rgb.redComponent + rgb.greenComponent + rgb.blueComponent) / 3
                if dark {
                    XCTAssertGreaterThan(brightness, 0.5, "\(token) must remain visible on dark code background")
                } else {
                    XCTAssertLessThan(brightness, 0.5, "\(token) must remain visible on light code background")
                }
            }
        }
    }

    func testParsesAbsoluteMarkdownPathWithLineAndColumn() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("Nola Code Reference \(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let file = directory.appendingPathComponent("Example File.swift")
        try "one\ntwo\nthree\n".write(to: file, atomically: true, encoding: .utf8)
        let encoded = file.path.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed)!
        let target = try XCTUnwrap(URL(string: "\(encoded):2:4"))

        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(target))
        XCTAssertEqual(reference.url, file.standardizedFileURL)
        XCTAssertEqual(reference.line, 2)
        XCTAssertEqual(reference.column, 4)
    }

    func testParsesFileURLGitHubFragmentAndRejectsWebURL() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("reference-\(UUID().uuidString).py")
        try "alpha\nbeta\n".write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }

        var components = try XCTUnwrap(URLComponents(url: file, resolvingAgainstBaseURL: false))
        components.fragment = "L2C3"
        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(try XCTUnwrap(components.url)))

        XCTAssertEqual(reference.url, file.standardizedFileURL)
        XCTAssertEqual(reference.line, 2)
        XCTAssertEqual(reference.column, 3)
        XCTAssertNil(MacCodeReferenceParser.parse(try XCTUnwrap(URL(string: "https://example.com/source.swift:42"))))
    }

    func testLoadsTextDocumentWithoutChangingLineIdentity() throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("reference-\(UUID().uuidString).swift")
        try "first\r\nsecond\r\nthird".write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }

        let reference = MacCodeReference(url: file, line: 2, column: nil)
        let document = try MacCodeReferenceLoader.load(reference)

        XCTAssertEqual(document.lines, ["first", "second", "third"])
        XCTAssertEqual(document.encodingName, "UTF-8")
        XCTAssertEqual(document.lineCount, 3)
    }

    func testSyntaxPreviewAddsLineNumbersAndTokenHighlighting() {
        let document = MacCodeReferenceDocument(
            url: URL(fileURLWithPath: "/tmp/example.py"),
            lines: [
                "def add(a, b):",
                "    # sum the values",
                "    return a + b",
            ],
            byteCount: 48,
            encodingName: "UTF-8"
        )

        let highlighted = MacCodeSyntaxHighlighter.attributedText(
            for: document,
            targetLine: 2
        )

        XCTAssertTrue(highlighted.string.contains("1  def add"))
        XCTAssertTrue(highlighted.string.contains("2      # sum the values"))
        let defRange = (highlighted.string as NSString).range(of: "def")
        XCTAssertNotNil(
            highlighted.attribute(
                .foregroundColor,
                at: defRange.location,
                effectiveRange: nil
            )
        )
        let targetRange = (highlighted.string as NSString).range(
            of: "2      # sum the values"
        )
        XCTAssertNotNil(
            highlighted.attribute(
                .backgroundColor,
                at: targetRange.location,
                effectiveRange: nil
            )
        )
    }

    func testMissingFileStillParsesAndFailsInsidePreviewLoader() throws {
        let missing = URL(fileURLWithPath: "/tmp/nola-missing-\(UUID().uuidString).swift")
        let target = try XCTUnwrap(URL(string: "\(missing.path):12"))
        let reference = try XCTUnwrap(MacCodeReferenceParser.parse(target))

        XCTAssertEqual(reference.line, 12)
        XCTAssertThrowsError(try MacCodeReferenceLoader.load(reference)) { error in
            guard case MacCodeReferenceLoadError.missing = error else {
                return XCTFail("Expected a missing-file error, got \(error)")
            }
        }
    }
}

@MainActor
final class MacProcessingMotionTests: XCTestCase {
    func testHighlightMovesWithoutMovingTextAndWrapsOutsideGlyphs() async throws {
        for scheme in [ColorScheme.light, .dark] {
            let start = try await frame(phase: 0, highlight: true, scheme: scheme)
            let middle = try await frame(phase: 0.5, highlight: true, scheme: scheme)
            let end = try await frame(phase: 1, highlight: true, scheme: scheme)
            let staticFrame = try await frame(phase: 0.5, highlight: false, scheme: scheme)
            XCTAssertEqual(start.pixels, end.pixels, "The loop must wrap while the highlight is outside the glyphs")
            XCTAssertEqual(start.pixels, staticFrame.pixels, "Readable base text remains beneath the moving highlight")
            XCTAssertNotEqual(middle.pixels, staticFrame.pixels)
            XCTAssertEqual(start.size, middle.size, "Animation must never resize the status row")
        }
    }

    func testActualTimelineFlowsOnlyWhileActiveAndMotionIsAllowed() async throws {
        for (active, reduced, running) in [(true, false, true), (true, true, true), (false, false, true), (true, false, false)] {
            let host = NSHostingView(rootView:
                MacStatusTimelineText(text: Text("正在处理中"), isRunning: running,
                    isActive: active, reduceMotion: reduced)
                    .font(.system(size: 18)).padding(20).frame(width: 260, height: 80))
            let window = mount(host)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await Task.sleep(for: .milliseconds(120))
            let first = try pixels(host)
            var changed = false
            for _ in 0..<5 {
                try await Task.sleep(for: .milliseconds(220))
                if try pixels(host) != first { changed = true }
            }
            XCTAssertEqual(changed, active && !reduced && running,
                "Reduced motion, background and settled status must remain static")
        }
    }

    private func frame(phase: Double, highlight: Bool, scheme: ColorScheme) async throws -> (pixels: Data, size: NSSize) {
        let host = NSHostingView(rootView: MacStatusTextFrame(text: Text("正在处理中"), phase: phase, showsHighlight: highlight)
            .font(.system(size: 18)).padding(20).background(scheme == .dark ? Color.black : .white)
            .environment(\.colorScheme, scheme).frame(width: 260, height: 80))
        let window = mount(host)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await Task.sleep(for: .milliseconds(60))
        let data = try pixels(host)
        if phase == 0.5 && highlight {
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to:
                URL(fileURLWithPath: "/tmp/nola-processing-\(scheme == .dark ? "dark" : "light").png"))
        }
        return (data, host.fittingSize)
    }

    private func mount(_ host: NSView) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 260, height: 80),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        return window
    }

    private func pixels(_ host: NSView) throws -> Data {
        host.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        return Data(bytes: try XCTUnwrap(bitmap.bitmapData), count: bitmap.bytesPerRow * bitmap.pixelsHigh)
    }
}

@MainActor
final class MacReplyDisclosureMotionTests: XCTestCase {
    func testProcessBodyHasIntermediateFramesAndRapidReversalRemainsUsable() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let root = VStack(alignment: .leading, spacing: 0) {
            MacJournalProcessView(partIDs: ["fixture-reasoning"], delivery: .completed,
                requiresAttention: false, toolCount: 0, attentionState: nil,
                reasoningOnly: true) {
                Rectangle().fill(.white).frame(width: 180, height: 100)
            }
            Spacer(minLength: 0)
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(.black).environment(\.colorScheme, .dark).environment(\.accessibilityEnabled, true)
        let host = NSHostingView(rootView: root)
        host.frame = NSRect(x: 0, y: 0, width: 480, height: 240)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(host, 250)
        let closed = try energy(host)
        func toggle() throws {
            let button = try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == "chat.journal.process.toggle" })
            XCTAssertTrue(button.accessibilityPerformPress?() ?? false)
        }
        try toggle(); try await settle(host, 45)
        let active = try energy(host)
        try await settle(host, 260)
        let opened = try energy(host)
        XCTAssertGreaterThan(opened, closed + 1_000_000)
        if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            XCTAssertGreaterThan(active, closed + 1_000)
            XCTAssertLessThan(active, opened - 1_000, "The actual process body must have an intermediate drawing frame")
        }
        try toggle(); try await settle(host, 45); try toggle(); try await settle(host, 45)
        try toggle(); try await settle(host, 260)
        XCTAssertLessThan(try energy(host), closed + 100_000, "Rapid reversal must finish collapsed without residual content")
        try toggle(); try await settle(host, 260)
        XCTAssertGreaterThan(try energy(host), closed + 1_000_000, "The control must still reopen")
    }

    func testLiveRetryKeepsHistoryClosedAndPreservesReaderChoice() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let state = ProcessRecoveryState()
        let host = NSHostingView(rootView: ProcessRecoveryProbe(state: state))
        host.frame = NSRect(x: 0, y: 0, width: 480, height: 300)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        func visibleHistory() -> [AnyObject] { objects(host).filter { ($0.accessibilityIdentifier?() ?? nil) == "history.detail" } }
        func toggle() throws {
            let button = try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == "chat.journal.process.toggle" })
            XCTAssertTrue(button.accessibilityPerformPress?() ?? false)
        }
        try await settle(host, 100)
        XCTAssertTrue(visibleHistory().isEmpty, "Retrying must not expose the earlier failed tool call")
        state.ids.append("successful-retry"); state.currentTitle = nil
        try await settle(host, 100)
        XCTAssertTrue(visibleHistory().isEmpty, "Completion must remain compact")
        try toggle(); try await settle(host, 280)
        XCTAssertFalse(visibleHistory().isEmpty, "A user can inspect retained steps")
        state.ids.append("next-step"); state.currentTitle = "Next step"
        try await settle(host, 100)
        XCTAssertFalse(visibleHistory().isEmpty, "New events must preserve an explicit reading choice")
        try toggle(); try await settle(host, 280)
        state.ids.append("later-step")
        try await settle(host, 100)
        XCTAssertTrue(visibleHistory().isEmpty, "Later deltas must not reopen manually collapsed history")
    }

    func testExpandedProcessRemainsActiveBetweenToolStepsUntilRunEnds() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let state = ProcessLifecycleState()
        let host = NSHostingView(rootView: ProcessLifecycleProbe(state: state))
        host.frame = NSRect(x: 0, y: 0, width: 520, height: 300)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        func toggle() throws -> AnyObject {
            try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == "chat.journal.process.toggle" })
        }
        let processing = String(localized: "Processing")
        try await settle(host, 150)
        XCTAssertEqual(try toggle().accessibilityLabel?(), processing)
        XCTAssertTrue(try toggle().accessibilityPerformPress?() ?? false)
        try await settle(host, 280)
        XCTAssertEqual(try toggle().accessibilityLabel?(), processing, "Opening history must not turn a live run into a finished process")
        state.title = "Reading a source"
        try await settle(host, 100)
        XCTAssertEqual(try toggle().accessibilityLabel?(), "Reading a source")
        state.title = nil
        try await settle(host, 100)
        XCTAssertEqual(try toggle().accessibilityLabel?(), processing, "A completed tool does not finish the model run")
        state.delivery = .completed
        try await settle(host, 100)
        XCTAssertEqual(try toggle().accessibilityLabel?(), String(localized: "Execution process", table: "Chat"))
        XCTAssertTrue(objects(host).contains { ($0.accessibilityIdentifier?() ?? nil) == "lifecycle.history" })
    }

    private func settle(_ view: NSView, _ milliseconds: Int) async throws {
        try await Task.sleep(for: .milliseconds(milliseconds))
        view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
    }
    private func energy(_ view: NSView) throws -> Int {
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        let bytes = try XCTUnwrap(bitmap.bitmapData)
        var total = 0
        for y in 0..<bitmap.pixelsHigh { for x in 0..<bitmap.pixelsWide {
            let offset = y * bitmap.bytesPerRow + x * bitmap.samplesPerPixel
            total += Int(bytes[offset]) + Int(bytes[offset + 1]) + Int(bytes[offset + 2])
        } }
        return total
    }
    private func objects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], visited = Set<ObjectIdentifier>(), result: [AnyObject] = []
        var index = 0
        while index < queue.count && result.count < 4_000 {
            let item = queue[index]; index += 1
            guard visited.insert(ObjectIdentifier(item)).inserted else { continue }
            result.append(item)
            let children = item.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = item as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }
}

@MainActor
private final class ProcessRecoveryState: ObservableObject {
    @Published var ids = ["failed-attempt", "running-retry"]
    @Published var currentTitle: String? = "Retrying command"
}

private struct ProcessRecoveryProbe: View {
    @ObservedObject var state: ProcessRecoveryState
    var body: some View {
        VStack(alignment: .leading) {
            MacJournalProcessView(partIDs: state.ids, delivery: state.currentTitle == nil ? nil : .running,
                requiresAttention: false, toolCount: state.ids.count, attentionState: nil,
                reasoningOnly: false, currentActivityTitle: state.currentTitle) {
                    Text(state.ids.joined(separator: " / ")).accessibilityIdentifier("history.detail")
                }
            Spacer()
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .environment(\.accessibilityEnabled, true)
    }
}

@MainActor
private final class ProcessLifecycleState: ObservableObject {
    @Published var delivery: MacChatMessage.DeliveryState = .running
    @Published var title: String? = nil
}

private struct ProcessLifecycleProbe: View {
    @ObservedObject var state: ProcessLifecycleState
    var body: some View {
        VStack(alignment: .leading) {
            MacJournalProcessView(partIDs: ["completed-tool"], delivery: state.delivery,
                requiresAttention: false, toolCount: 1, attentionState: nil,
                reasoningOnly: false, currentActivityTitle: state.title) {
                    Text("Completed step").accessibilityIdentifier("lifecycle.history")
                }
            Spacer()
        }.environment(\.accessibilityEnabled, true)
    }
}

@MainActor
final class MacStreamingAppendRevealTests: XCTestCase {
    func testLiveMarkdownAccessibilityTracksEveryAppendAndFinalMarker() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let state = AppendRevealFixture(kind: .markdownBlock, text: "1. First", suppressed: false)
        let hosting = NSHostingView(rootView: AppendRevealRoot(state: state)
            .environment(\.accessibilityEnabled, true))
        hosting.frame = NSRect(x: 0, y: 0, width: 780, height: 360)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting, milliseconds: 100)
        XCTAssertTrue(accessibleText(hosting).contains("First"))
        state.text = "1. First item has appended words.\n2. Second"
        try await settle(hosting, milliseconds: 100)
        XCTAssertTrue(accessibleText(hosting).contains("First item has appended words."))
        XCTAssertTrue(accessibleText(hosting).contains("Second"))
        state.text += " item has its complete suffix.\n\nNOLA-STREAM-END"
        state.contentIsFinal = true
        try await settle(hosting, milliseconds: 450)
        let completed = accessibleText(hosting)
        XCTAssertTrue(completed.contains("Second item has its complete suffix."), completed)
        XCTAssertTrue(completed.contains("NOLA-STREAM-END"), completed)
    }

    func testCompletedSuffixStaysVisiblyCompleteAfterRevealSettles() async throws {
        let state = AppendRevealFixture(kind: .phrase, text: "REEDITED=32", suppressed: false)
        let hosting = NSHostingView(rootView: AppendRevealRoot(state: state))
        hosting.frame = NSRect(x: 0, y: 0, width: 500, height: 100)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = hosting; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting, milliseconds: 600)
        let before = try bitmap(hosting)
        state.text += "3"
        state.contentIsFinal = true
        try await settle(hosting, milliseconds: Int((AppleStreamingRevealTiming.duration + 0.2) * 1000))
        let completed = try bitmap(hosting)
        try await settle(hosting, milliseconds: 450)
        let later = try bitmap(hosting)
        XCTAssertGreaterThan(brightPixels(completed), brightPixels(before) + 5,
                             "The final glyph must actually be visible, not only accessible")
        XCTAssertEqual(changedPixels(completed, later), 0,
                       "A settled terminal response must not keep changing or lose its final glyph")
    }

    private func accessibleText(_ root: NSView) -> String {
        var queue: [AnyObject] = [root]
        var visited = Set<ObjectIdentifier>()
        var values: [String] = []
        var cursor = 0
        while cursor < queue.count && visited.count < 4_000 {
            let object = queue[cursor]
            cursor += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            if let label = object.accessibilityLabel?() ?? nil { values.append(label) }
            let rawValue: Any? = object.accessibilityValue?() ?? nil
            if let value = rawValue { values.append(String(describing: value)) }
            let raw: [Any] = (object.accessibilityChildren?() ?? nil) ?? []
            queue.append(contentsOf: (raw + NSAccessibility.unignoredChildren(from: raw)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return values.joined(separator: "\n")
    }

    func testStableMutableIdentityRevealsOnlyAppendedCharactersAndFinalSuffix() {
        let now = Date(timeIntervalSince1970: 100)
        var state = AppleInlineAppendRevealState(text: "Visible", streaming: true, visiblePrefixCount: 7, at: now)
        state = state.updating(text: "Visible first", streaming: true, reduceMotion: false, at: now)
        let first = state.segments[0]
        XCTAssertEqual(first.range, 7..<13)
        state = state.updating(text: "Visible first final", streaming: false, reduceMotion: false,
                               at: now.addingTimeInterval(0.05))
        XCTAssertEqual(state.segments[0].id, first.id)
        XCTAssertEqual(state.segments[0].startedAt, first.startedAt)
        XCTAssertEqual(state.segments[1].range, 13..<19)
        XCTAssertTrue(state.segments.allSatisfy { $0.range.lowerBound >= 7 })
        let settled = state.updating(text: state.text, streaming: false, reduceMotion: false,
                                     at: now.addingTimeInterval(1))
        XCTAssertTrue(settled.segments.isEmpty)
        let oldTail = AppleStreamingRevealTiming.remaining(elapsed: 0.25, position: 1)
        let newHead = AppleStreamingRevealTiming.remaining(elapsed: 0.20, position: 0, precedingElapsed: 0.25)
        XCTAssertGreaterThanOrEqual(newHead, oldTail, "A later batch cannot appear ahead of the preceding tail")
        XCTAssertEqual(AppleStreamingRevealTiming.remaining(elapsed: 0.53, position: 1, precedingElapsed: 0.58), 0)
        XCTAssertEqual(AppleStreamingRevealTiming.remaining(elapsed: 0.20, position: 1,
            completionElapsed: 0.16, precedingElapsed: 0.25), 0)
    }

    func testLargeBurstKeepsFullTextButBoundsAnimatedInk() {
        let now = Date()
        let source = String(repeating: "中文👨‍👩‍👧‍👦 é ", count: 2_000)
        var state = AppleInlineAppendRevealState(text: "", streaming: true, at: now)
        state = state.updating(text: source, streaming: true, reduceMotion: false, at: now)
        XCTAssertEqual(state.text, source)
        XCTAssertLessThanOrEqual(state.segments.reduce(0) { $0 + $1.range.count }, 384)
        XCTAssertEqual(state.segments.last?.range.upperBound, source.count)
        let first = state.segments.last?.id
        state = state.updating(text: source + "追加", streaming: true, reduceMotion: false, at: now.addingTimeInterval(0.02))
        XCTAssertEqual(state.segments.first?.id, first)
        XCTAssertLessThanOrEqual(state.segments.reduce(0) { $0 + $1.range.count }, 384)
        XCTAssertEqual(state.text, source + "追加")
    }

    func testHistoryReplacementAndReducedMotionNeverReplayExistingText() {
        let now = Date(timeIntervalSince1970: 100)
        let history = AppleInlineAppendRevealState(text: "Past", streaming: false, at: now)
        XCTAssertTrue(history.segments.isEmpty)
        let replacement = history.updating(text: "Replacement", streaming: true, reduceMotion: false, at: now)
        XCTAssertTrue(replacement.segments.isEmpty)
        let motion = replacement.updating(text: "Replacement new", streaming: true, reduceMotion: true, at: now)
        XCTAssertTrue(motion.segments.isEmpty)
        XCTAssertTrue(motion.updating(text: motion.text, streaming: true, reduceMotion: false, at: now).segments.isEmpty)
    }

    func testHostedStructuredAppendAndTerminalFramesRevealWithoutMovingPrefix() async throws {
        try await checkFrames(kind: .markdownBlock, prefix: "- OLD PREFIX ", suffix: "new words rise into clarity", final: false)
        try await checkFrames(kind: .markdownBlock, prefix: "- OLD PREFIX ", suffix: "last words arrive at completion", final: true)
    }

    func testHostedSamePhraseWithMathStillRevealsAppendedProse() async throws {
        try await checkFrames(kind: .phrase, prefix: "OLD $x^2$ ", suffix: "new words rise after the equation", final: false)
    }

    func testHostedSuppressedRevealPaintsFinalGlyphsImmediately() async throws {
        try await checkFrames(kind: .markdownBlock, prefix: "- OLD PREFIX ", suffix: "immediate accessible text", final: false, suppressed: true)
    }

    func testLiveFinalUnmatchedMarkdownMatchesReopenedHistory() async throws {
        for kind in [MacAssistantTextNode.Kind.phrase, .markdownBlock] {
            let source = kind == .phrase ? "Prefix **unfinished" : "- Prefix **unfinished"
            let state = AppendRevealFixture(kind: kind, text: source, suppressed: false)
            let hosting = NSHostingView(rootView: AppendRevealRoot(state: state))
            hosting.frame = NSRect(x: 0, y: 0, width: 780, height: 180)
            let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            window.makeKeyAndOrderFront(nil)
            defer { window.orderOut(nil); window.contentView = nil; window.close() }
            try await settle(hosting, milliseconds: 430)
            let provisional = try bitmap(hosting)
            // Match Journal: the owner has streamed, but syntax is now final.
            // No transport-animation flag is repurposed as parser finality.
            state.contentIsFinal = true
            try await settle(hosting, milliseconds: 430)
            let completed = try bitmap(hosting)
            let history = AppendRevealFixture(kind: kind, text: source, suppressed: false)
            history.streaming = false
            history.contentIsFinal = true
            hosting.rootView = AppendRevealRoot(state: history)
            try await settle(hosting, milliseconds: 430)
            let reopened = try bitmap(hosting)
            XCTAssertGreaterThan(brightPixels(completed), 80, "Completed text must remain visibly drawn.")
            XCTAssertGreaterThan(changedPixels(provisional, completed), 40,
                                 "Finality must stop repairing the unmatched emphasis marker.")
            XCTAssertEqual(changedPixels(completed, reopened), 0,
                           "A live-completed response must use the same Markdown interpretation as history.")
        }
    }

    private func checkFrames(kind: MacAssistantTextNode.Kind, prefix: String, suffix: String,
                             final: Bool, suppressed: Bool = false) async throws {
        let state = AppendRevealFixture(kind: kind, text: prefix, suppressed: suppressed)
        let hosting = NSHostingView(rootView: AppendRevealRoot(state: state))
        hosting.frame = NSRect(x: 0, y: 0, width: 780, height: 180)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        // The reference prefix must finish its initial 0.52 s ink reveal.
        // A 0.43 s reference still contains a fading final math glyph.
        try await settle(hosting, milliseconds: Int((AppleStreamingRevealTiming.duration + 0.2) * 1000))
        let before = try bitmap(hosting)
        let size = hosting.fittingSize
        state.text += suffix
        if final { state.contentIsFinal = true }
        try await settle(hosting, milliseconds: 35)
        let active = try bitmap(hosting)
        let activeSize = hosting.fittingSize
        try await settle(hosting, milliseconds: 400)
        let settled = try bitmap(hosting)
        XCTAssertEqual(activeSize, hosting.fittingSize, "Drawing-only reveal must not change the laid-out size.")
        XCTAssertEqual(size.height, hosting.fittingSize.height, "The one-line fixture must not reflow.")
        XCTAssertEqual(changedPixels(before, active, maxX: 90), 0, "Already-visible prefix must remain pixel stable.")
        XCTAssertEqual(changedPixels(before, settled, maxX: 90), 0, "Completion must not erase the visible prefix.")
        XCTAssertGreaterThan(brightPixels(settled), brightPixels(before) + 40,
                             "Completed suffix must add visible text, not merely differ by becoming blank.")
        let changed = changedPixels(active, settled)
        if suppressed { XCTAssertEqual(changed, 0) }
        else { XCTAssertGreaterThan(changed, 40, "New glyphs must visibly progress through reveal frames.") }
        for (name, image) in [("active", active), ("settled", settled)] {
            let data = try XCTUnwrap(image.representation(using: .png, properties: [:]))
            let attachment = XCTAttachment(data: data, uniformTypeIdentifier: "public.png")
            attachment.name = "append-reveal-\(kind)-\(final)-\(suppressed)-\(name)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    private func settle(_ view: NSView, milliseconds: Int) async throws {
        view.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(milliseconds))
        view.layoutSubtreeIfNeeded()
        view.displayIfNeeded()
    }

    private func bitmap(_ view: NSView) throws -> NSBitmapImageRep {
        let image = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: image)
        return image
    }

    private func changedPixels(_ lhs: NSBitmapImageRep, _ rhs: NSBitmapImageRep, maxX: Int? = nil) -> Int {
        var changed = 0
        let end = min(maxX ?? lhs.pixelsWide, lhs.pixelsWide)
        for y in 0..<min(lhs.pixelsHigh, rhs.pixelsHigh) {
            for x in 0..<end {
                guard let a = lhs.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
                      let b = rhs.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if abs(a.redComponent - b.redComponent) + abs(a.greenComponent - b.greenComponent)
                    + abs(a.blueComponent - b.blueComponent) > 0.02 { changed += 1 }
            }
        }
        return changed
    }

    private func brightPixels(_ image: NSBitmapImageRep) -> Int {
        var count = 0
        for y in stride(from: 0, to: image.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: image.pixelsWide, by: 2) {
                guard let color = image.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if min(color.redComponent, color.greenComponent, color.blueComponent) > 0.35 { count += 1 }
            }
        }
        return count
    }
}

@MainActor
private final class AppendRevealFixture: ObservableObject {
    let id = UUID()
    let blockID = UUID()
    let kind: MacAssistantTextNode.Kind
    let suppressed: Bool
    @Published var text: String
    @Published var streaming = true
    @Published var contentIsFinal = false
    init(kind: MacAssistantTextNode.Kind, text: String, suppressed: Bool) {
        self.kind = kind
        self.text = text
        self.suppressed = suppressed
    }
}

private struct AppendRevealRoot: View {
    @ObservedObject var state: AppendRevealFixture
    var body: some View {
        MacAssistantPresentationView(nodes: [MacAssistantTextNode(id: state.id, blockID: state.blockID,
                                                                 kind: state.kind, text: state.text)],
                                     animateNewNodes: state.streaming, contentIsFinal: state.contentIsFinal,
                                     suppressReveal: state.suppressed)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .foregroundStyle(.white).background(.black).preferredColorScheme(.dark)
    }
}

final class MacStreamingPresentationRegressionTests: XCTestCase {
    func testConversationTraceSwitchRetiresOldStreamOnlyAfterAdmission() throws {
        func event(_ trace: String, _ sequence: Int, conversation: String = "conversation") throws -> MacAgentTraceEnvelope {
            try XCTUnwrap(MacAgentTraceEnvelope.decode([
                "schema_version": 1, "trace_id": trace, "run_id": "turn-\(trace)",
                "conversation_id": conversation, "turn_id": "turn-\(trace)",
                "event_id": "\(trace)-\(sequence)", "sequence": sequence,
                "emitted_at": "2026-10-12T00:00:00Z", "kind": "phase.changed",
                "status": "running", "visibility": "summary", "payload": ["phase": "compacting"],
            ]))
        }
        for projectsContent in [true, false] {
            var owner = MacConversationTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            XCTAssertEqual(owner.apply(try event("a", 1), to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            XCTAssertEqual(owner.apply(try event("b", 0), to: &presentation).disposition,
                .outOfOrder(last: 0, received: 0))
            XCTAssertEqual(owner.apply(try event("a", 1), to: &presentation).disposition, .duplicate)
            XCTAssertEqual(owner.apply(try event("b", 1, conversation: "foreign"),
                to: &presentation).disposition, .identityMismatch)
            XCTAssertEqual(owner.apply(try event("b", 1), to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            let before = presentation
            for sequence in [1, 2, 100] {
                XCTAssertEqual(owner.apply(try event("a", sequence), to: &presentation,
                    projectsContent: projectsContent).disposition, .retiredTrace)
                XCTAssertEqual(presentation, before)
            }
            XCTAssertEqual(owner.apply(try event("b", 2), to: &presentation).disposition, .applied)
        }
    }

    func testTraceConflictRejectsChangedPayloadWithoutConsumingSequence() throws {
        func event(_ sequence: Int, id: String, phase: String = "compacting",
                   trace: String = "trace") throws -> MacAgentTraceEnvelope {
            try XCTUnwrap(MacAgentTraceEnvelope.decode([
                "schema_version": 1, "trace_id": trace, "run_id": "run",
                "conversation_id": "conversation", "turn_id": "turn", "event_id": id,
                "sequence": sequence, "emitted_at": "2026-10-12T00:00:00Z",
                "kind": "phase.changed", "status": "running", "visibility": "summary",
                "payload": ["phase": phase],
            ]))
        }
        for projectsContent in [true, false] {
            var reducer = MacAgentTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            let original = try event(1, id: "first")
            XCTAssertEqual(reducer.apply(original, to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            let before = presentation
            for conflict in [try event(1, id: "first", phase: "running"),
                             try event(2, id: "first")] {
                XCTAssertEqual(reducer.apply(conflict, to: &presentation,
                    projectsContent: projectsContent).disposition, .eventConflict)
                XCTAssertEqual(presentation, before)
            }
            XCTAssertEqual(reducer.apply(try event(0, id: "bad", trace: "foreign"),
                to: &presentation).disposition, .identityMismatch)
            XCTAssertEqual(reducer.apply(original, to: &presentation).disposition, .duplicate,
                "A rejected new trace must not erase the old replay history")
            XCTAssertEqual(reducer.apply(try event(2, id: "second"), to: &presentation).disposition, .applied)
        }
    }

    func testTraceIdentityRejectionDoesNotResetCursorOrConsumeEventID() throws {
        func event(_ sequence: Int, trace: String = "trace", run: String = "run",
                   conversation: String = "conversation") throws -> MacAgentTraceEnvelope {
            let data = try JSONSerialization.data(withJSONObject: [
                "schema_version": 1, "trace_id": trace, "run_id": run,
                "conversation_id": conversation, "turn_id": "turn", "event_id": "event-\(sequence)",
                "sequence": sequence, "emitted_at": "2026-10-12T00:00:00Z",
                "kind": "phase.changed", "status": "running", "visibility": "summary",
                "payload": ["phase": "compacting"],
            ])
            return try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
        }
        for projectsContent in [true, false] {
            var reducer = MacAgentTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            XCTAssertEqual(reducer.apply(try event(1), to: &presentation,
                projectsContent: projectsContent).disposition, .applied)
            let before = presentation
            for foreign in [try event(1, run: "other"), try event(2, run: "other"),
                            try event(2, conversation: "other"),
                            try event(1, trace: "other", conversation: "other")] {
                let rejected = reducer.apply(foreign, to: &presentation, projectsContent: projectsContent)
                XCTAssertEqual(rejected.disposition, .identityMismatch)
                XCTAssertFalse(rejected.isTerminal)
                XCTAssertEqual(presentation, before)
            }
            XCTAssertEqual(reducer.apply(try event(1), to: &presentation).disposition, .duplicate)
            XCTAssertEqual(reducer.apply(try event(2), to: &presentation).disposition, .applied)
            XCTAssertEqual(reducer.apply(try event(1, trace: "next", run: "next-run"),
                to: &presentation).disposition, .identityMismatch)
        }
    }

    func testValidationDiagnosticsDoNotReplaceRunStateOrBecomeCompleted() throws {
        for projectsContent in [true, false] {
            for status in ["incomplete", "blocked", "evidence_insufficient", "failed", "error"] {
                var reducer = MacAgentTraceReducer()
                var presentation = MacAssistantPresentation.processing()
                presentation.upsertStatus(externalID: "run-status", title: "Run", detail: "Original failure", state: .failed)
                func event(_ sequence: Int, _ status: String) throws -> MacAgentTraceEnvelope {
                    let data = try JSONSerialization.data(withJSONObject: [
                        "schema_version": 1, "trace_id": "trace", "run_id": "run",
                        "conversation_id": "conversation", "turn_id": "turn", "event_id": "event-\(sequence)",
                        "sequence": sequence, "emitted_at": "2026-10-12T00:00:00Z",
                        "kind": "validation.updated", "status": "incomplete", "visibility": "summary",
                        "payload": ["validation": ["status": status, "summary": "Diagnostic"]],
                    ])
                    return try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
                }
                let outcome = reducer.apply(try event(1, status), to: &presentation, projectsContent: projectsContent)
                XCTAssertFalse(outcome.isTerminal)
                let diagnostic = try XCTUnwrap(presentation.traceItems.first { $0.externalID == "validation-diagnostic" })
                XCTAssertEqual(diagnostic.state, ["failed", "error"].contains(status) ? .failed : .incomplete)
                XCTAssertEqual(presentation.traceItems.first { $0.externalID == "run-status" }?.detail, "Original failure")
                XCTAssertTrue(MacAssistantPresentation.preservesAcrossCanonicalProjection(diagnostic))
                presentation.finish()
                XCTAssertEqual(presentation.traceItems.first { $0.externalID == "validation-diagnostic" }?.state, diagnostic.state)
                _ = reducer.apply(try event(2, "not_required"), to: &presentation, projectsContent: projectsContent)
                XCTAssertFalse(presentation.traceItems.contains { $0.externalID == "validation-diagnostic" })
                XCTAssertEqual(presentation.traceItems.first { $0.externalID == "run-status" }?.detail, "Original failure")
            }
        }
    }

    func testCoreOwnedCompactionStatusSettlesOnEveryTerminalPath() throws {
        for terminal in ["run.completed", "run.failed", "run.cancelled", "context.compacted"] {
            var reducer = MacAgentTraceReducer()
            var presentation = MacAssistantPresentation.processing()
            func event(_ sequence: Int, _ kind: String, _ payload: [String: String]) throws -> MacAgentTraceEnvelope {
                let data = try JSONSerialization.data(withJSONObject: [
                    "schema_version": 1, "trace_id": "trace", "run_id": "run",
                    "conversation_id": "conversation", "turn_id": "turn", "event_id": "event-\(sequence)",
                    "sequence": sequence, "emitted_at": "2026-09-21T00:00:00Z",
                    "kind": kind, "status": "running", "visibility": "summary", "payload": payload,
                ])
                return try JSONDecoder().decode(MacAgentTraceEnvelope.self, from: data)
            }
            _ = reducer.apply(try event(1, "phase.changed", ["phase": "compacting"]), to: &presentation, projectsContent: false)
            XCTAssertTrue(presentation.traceItems.contains { $0.externalID == "context-compaction" })
            _ = reducer.apply(try event(2, "tool.progress", ["summary": "model_attempt_started"]), to: &presentation, projectsContent: true)
            XCTAssertFalse(presentation.traceItems.contains { $0.kind == .tool })
            XCTAssertTrue(presentation.traceItems.contains { $0.externalID == "context-compaction" })
            _ = reducer.apply(try event(3, terminal, [:]), to: &presentation, projectsContent: false)
            XCTAssertFalse(presentation.traceItems.contains { $0.externalID == "context-compaction" }, terminal)
        }
    }

    func testLargeProviderPayloadIsReleasedAsSeveralLosslessSemanticSegments() {
        let input = String(repeating: "这一小段内容应该平滑浮现，而不是一次跳到最终结果。", count: 80)
        var buffer = MacPacedStreamBuffer(minimumLength: 18, maximumLength: 56)

        let streamed = buffer.append(input)
        let completed = streamed + buffer.finish()

        XCTAssertGreaterThan(completed.count, 10)
        XCTAssertEqual(completed.joined(), input)
        XCTAssertTrue(completed.allSatisfy { $0.count <= 56 })
    }

    func testMarkdownMutableTailKeepsItsIdentityWhenFinalized() throws {
        let seed = UUID().uuidString
        var assembler = MacSemanticStreamAssembler(identitySeed: seed)

        _ = assembler.append("## 稳定标题\n\n正在流式显示的段落")
        let mutableID = try XCTUnwrap(assembler.mutableTail?.id)
        let finalized = assembler.finish()

        XCTAssertEqual(finalized.last?.id, mutableID)
        XCTAssertEqual(finalized.last?.text, "正在流式显示的段落")
    }

    func testIncompleteProseTailIsVisibleAndKeepsIdentityAtSemanticCommit() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: UUID().uuidString)
        var presentation = MacAssistantPresentation.processing()

        presentation.append(assembler.append("尚未形成完整语义边界的正文"))
        presentation.updateMutableTextNode(assembler.mutableTail)
        let liveNode = try XCTUnwrap(presentation.renderedTextNodes.last)
        XCTAssertEqual(liveNode.text, "尚未形成完整语义边界的正文")

        presentation.append(assembler.finish())
        presentation.updateMutableTextNode(nil)
        XCTAssertEqual(presentation.renderedTextNodes.map(\.text), ["尚未形成完整语义边界的正文"])
        XCTAssertEqual(presentation.renderedTextNodes.last?.id, liveNode.id)
    }

    func testDefaultPacingReleasesShortWordGroupsWithoutLosingText() {
        let input = "Streaming output should feel immediate while keeping every character intact."
        var buffer = MacPacedStreamBuffer()

        let streamed = buffer.append(input)
        let completed = streamed + buffer.finish()

        XCTAssertGreaterThan(streamed.count, 1)
        XCTAssertEqual(completed.joined(), input)
        XCTAssertTrue(completed.allSatisfy { $0.count <= 24 })
    }

    func testOpenCodeBlockRemainsVisibleForIncrementalUpdates() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: UUID().uuidString)
        var presentation = MacAssistantPresentation.processing()

        presentation.append(assembler.append("```swift\nlet value = 1"))
        presentation.updateMutableTextNode(assembler.mutableTail)
        let node = try XCTUnwrap(presentation.renderedTextNodes.last)

        XCTAssertEqual(node.kind, .code(language: "swift"))
        XCTAssertEqual(node.text, "let value = 1")
    }

    func testLiveStructuredBlockRendersIncompleteInlineMarkdownWithoutRawMarkers() throws {
        var assembler = MacSemanticStreamAssembler(identitySeed: UUID().uuidString)

        XCTAssertTrue(assembler.append("- **正在审").isEmpty)
        let node = try XCTUnwrap(assembler.mutableTail)
        XCTAssertEqual(node.kind, .markdownBlock)
        let item = try XCTUnwrap(MacMarkdownTextParser.parse(node.text).first)
        guard case .listItem(_, let children) = item,
              case .paragraph(let source) = children.first else {
            return XCTFail("Expected the mutable tail to remain a rendered list item")
        }
        let prepared = MacMarkdownInlinePreprocessor.prepare(
            source,
            repairingIncompleteSyntax: true
        )
        let attributed = try AttributedString(
            markdown: prepared,
            options: .init(
                interpretedSyntax: .inlineOnlyPreservingWhitespace,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        )

        XCTAssertEqual(String(attributed.characters), "正在审")
        XCTAssertFalse(String(attributed.characters).contains("**"))
    }
}

@MainActor
private final class RecordedRunLoopActivityProbe {
    private var observer: CFRunLoopObserver?
    private var started: TimeInterval? = ProcessInfo.processInfo.systemUptime
    private var maximum: TimeInterval = 0
    private var count = 0
    private var reports = 0

    init() {
        observer = CFRunLoopObserverCreateWithHandler(nil,
            CFRunLoopActivity.afterWaiting.rawValue | CFRunLoopActivity.beforeWaiting.rawValue,
            // Observe before-waiting after AppKit/Core Animation commit hooks,
            // otherwise their layout work would be misclassified as idle time.
            true, CFIndex.max) { [weak self] _, activity in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if activity == .afterWaiting {
                        if self.started == nil { self.started = ProcessInfo.processInfo.systemUptime }
                    } else {
                        self.finishSpan()
                    }
                }
            }
        if let observer { CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes) }
    }

    private func finishSpan() {
        guard let started else { return }
        let elapsed = ProcessInfo.processInfo.systemUptime - started
        maximum = max(maximum, elapsed)
        count += 1
        self.started = nil
        if elapsed > 0.016, reports < 12 {
            reports += 1
            print("recorded_runloop_active uptime=\(started) duration_ms=\(elapsed * 1_000)")
        }
    }

    func stop(phase: String) {
        finishSpan()
        if let observer { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, .commonModes) }
        observer = nil
        print("recorded_runloop_summary phase=\(phase) spans=\(count) active_max_ms=\(maximum * 1_000)")
    }
}

/// Opt-in instrumentation in the isolated XCTest process only. The developer's
/// running app is untouched and the original AppKit implementation always runs.
@MainActor
private final class RecordedLayoutReentryProbe {
    private let method: Method
    private let original: IMP
    private var replacement: IMP?
    private var active: Set<ObjectIdentifier> = []
    private var reports = 0
    private var calls = 0

    init?() {
        let selector = #selector(NSView.layoutSubtreeIfNeeded)
        guard let method = class_getInstanceMethod(NSView.self, selector) else { return nil }
        self.method = method
        original = method_getImplementation(method)
        typealias Layout = @convention(c) (NSView, Selector) -> Void
        let invoke = unsafeBitCast(original, to: Layout.self)
        let block: @convention(block) (NSView) -> Void = { [weak self] view in
            guard Thread.isMainThread else { invoke(view, selector); return }
            MainActor.assumeIsolated {
                guard let self else { invoke(view, selector); return }
                self.calls += 1
                let identity = ObjectIdentifier(view)
                let inserted = self.active.insert(identity).inserted
                if !inserted, self.reports < 5 {
                    self.reports += 1
                    print("recorded_layout_reentry view=\(type(of: view)) stack=\(Thread.callStackSymbols.joined(separator: " | "))")
                    fflush(stdout)
                }
                defer { if inserted { self.active.remove(identity) } }
                let start = ProcessInfo.processInfo.systemUptime
                invoke(view, selector)
                let elapsed = ProcessInfo.processInfo.systemUptime - start
                if elapsed > 0.04 {
                    print("recorded_slow_layout uptime=\(start) duration_ms=\(elapsed * 1_000) view=\(type(of: view))")
                }
            }
        }
        let replacement = imp_implementationWithBlock(block)
        self.replacement = replacement
        method_setImplementation(method, replacement)
    }

    func stop() {
        guard let replacement else { return }
        method_setImplementation(method, original)
        imp_removeBlock(replacement)
        self.replacement = nil
        print("recorded_layout_probe calls=\(calls) reports=\(reports)")
    }
}

@MainActor
final class MacTranscriptCompletionScrollTests: XCTestCase {
    func testComposerMeasurementDoesNotMutateLiveTextLayout() {
        let editor = NSTextView(frame: NSRect(x: 0, y: 0, width: 480, height: 80))
        editor.font = .systemFont(ofSize: 14.5)
        editor.textContainerInset = NSSize(width: 0, height: 5)
        editor.textContainer?.lineFragmentPadding = 0
        editor.string = String(repeating: "输入框文本 wrapping text ", count: 8)
        let frame = editor.frame
        let containerSize = editor.textContainer?.containerSize
        let text = editor.string
        let wide = MacComposerTextMeasurement.size(for: editor, width: 480)
        let narrow = MacComposerTextMeasurement.size(for: editor, width: 120)
        XCTAssertGreaterThan(narrow.height, wide.height)
        XCTAssertLessThanOrEqual(narrow.height, 132)
        XCTAssertEqual(editor.frame, frame)
        XCTAssertEqual(editor.textContainer?.containerSize, containerSize)
        XCTAssertEqual(editor.string, text)
        editor.string = ""
        XCTAssertEqual(MacComposerTextMeasurement.size(for: editor, width: 480).height, 30)
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = 26
        paragraph.maximumLineHeight = 26
        editor.defaultParagraphStyle = paragraph
        editor.typingAttributes[.paragraphStyle] = paragraph
        XCTAssertEqual(MacComposerTextMeasurement.size(for: editor, width: 480).height, 36)
        editor.string = "Hello"
        XCTAssertEqual(MacComposerTextMeasurement.size(for: editor, width: 480).height, 36)
    }

    func testInterruptionSurfaceFitsShortContentAndBoundsLongContent() async throws {
        let state = InterruptionLayoutState()
        let host = NSHostingView(rootView: InterruptionLayoutHarness(state: state))
        host.frame = NSRect(x: 0, y: 0, width: 600, height: 500)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        for height in [64.0, 400.0, 48.0] {
            state.height = height
            try await settle(host)
            let scroll = try XCTUnwrap(views(host).compactMap { $0 as? NSScrollView }.first)
            XCTAssertEqual(scroll.frame.height, min(height, 200), accuracy: 2,
                "Short notices must shrink; only overflowing content should use the height cap")
        }
        state.height = 20
        state.showDisclosure = true
        for expanded in [false, true, false] {
            state.expanded = expanded
            try await settle(host)
            let scroll = try XCTUnwrap(views(host).compactMap { $0 as? NSScrollView }.first)
            if expanded {
                XCTAssertEqual(scroll.frame.height, 200, accuracy: 2)
            } else {
                XCTAssertLessThan(scroll.frame.height, 80,
                    "Collapsed diagnostics must release their content height")
            }
        }
    }

    @MainActor private final class InterruptionLayoutState: ObservableObject {
        @Published var height = 64.0
        @Published var showDisclosure = false
        @Published var expanded = false
    }
    private struct InterruptionLayoutHarness: View {
        @ObservedObject var state: InterruptionLayoutState
        var body: some View {
            VStack {
                Spacer()
                VStack {
                MacConversationInterruptionSurface(maxHeight: 200) {
                    Text("Recovery status").frame(height: state.height)
                    if state.showDisclosure {
                        DisclosureGroup("Diagnostic details", isExpanded: $state.expanded) {
                            Text("Long diagnostics").frame(height: 400)
                        }
                    }
                }
                Text("Composer remains visible")
                }
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// Opt-in local acceptance: never open the user's live store or construct
    /// an execution driver that could resume captured pending effects.
    func testRecordedLongConversationProjectionAndBodyReads() async throws {
        let layoutProbe = ProcessInfo.processInfo.environment["NOLA_DIAGNOSE_LAYOUT_REENTRY"] == "1"
            ? RecordedLayoutReentryProbe() : nil
        defer { layoutProbe?.stop() }
        let environment = ProcessInfo.processInfo.environment
        guard let path = environment["NOLA_LONG_CONTEXT_FIXTURE_DB"],
              let conversationID = environment["NOLA_LONG_CONTEXT_CONVERSATION"] else {
            throw XCTSkip("Supply a consistent private SQLite backup and conversation ID")
        }
        let source = URL(fileURLWithPath: path).resolvingSymlinksInPath()
        let temporary = URL(fileURLWithPath: NSTemporaryDirectory()).resolvingSymlinksInPath().path
        let sharedTemporary = URL(fileURLWithPath: "/tmp").resolvingSymlinksInPath().path
        guard source.path.hasPrefix(sharedTemporary + "/") || source.path.hasPrefix(temporary + "/") else {
            XCTFail("Recorded fixture must be a private temporary backup, not the live database")
            return
        }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-recorded-read-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let databaseURL = root.appendingPathComponent("fixture.sqlite")
        try FileManager.default.copyItem(at: source, to: databaseURL)
        // Preserve this isolated copy for receipt/RSS diagnostics. No credentials
        // or message bodies are printed in test logs.
        let start = ContinuousClock.now
        let awakeStart = SuspendingClock.now
        let port = try MacApplicationPort(databaseURL: databaseURL)
        let portDuration = start.duration(to: .now)
        let indexStart = ContinuousClock.now
        let summaries = try port.listConversations()
        XCTAssertTrue(summaries.contains { $0.id == conversationID })
        let indexDuration = indexStart.duration(to: .now)
        let windowStart = ContinuousClock.now
        let awakeWindowStart = SuspendingClock.now
        let window = try port.loadConversationWindow(conversationID)
        let windowDuration = windowStart.duration(to: .now)
        let awakeWindowDuration = awakeWindowStart.duration(to: .now)
        print("recorded_long_context_phase window=\(windowDuration) awake_window=\(awakeWindowDuration) sleep_overlap=\(windowDuration - awakeWindowDuration > .seconds(1))")
        let mappingStart = ContinuousClock.now
        let mapped = try MacCanonicalConversationRepository.projectMessages(window, databaseURL: databaseURL)
        XCTAssertFalse(mapped.isEmpty)
        print("recorded_long_context_phase message_mapping=\(mappingStart.duration(to: .now))")
        let settingsStart = ContinuousClock.now
        _ = try port.loadConversationSettings(conversationID)
        print("recorded_long_context_phase settings=\(settingsStart.duration(to: .now))")
        let repository = MacCanonicalConversationRepository(store: port)
        let projectionStart = ContinuousClock.now
        let stored = try XCTUnwrap(try repository.load(conversationID: conversationID))
        let projectionDuration = projectionStart.duration(to: .now)
        let warmStart = ContinuousClock.now
        let warm = try XCTUnwrap(try repository.load(conversationID: conversationID))
        XCTAssertEqual(warm.messages.count, stored.messages.count)
        let warmDuration = warmStart.duration(to: .now)
        XCTAssertFalse(stored.messages.isEmpty)
        // Exercise the production async entry used during completion. A main
        // actor heartbeat distinguishes responsive suspension from sync I/O.
        var heartbeatCount = 0
        let heartbeat = Task { @MainActor in
            while !Task.isCancelled {
                heartbeatCount += 1
                do { try await Task.sleep(for: .milliseconds(10)) } catch { break }
            }
        }
        let asyncStart = ContinuousClock.now
        let asyncStored: MacStoredConversation?
        do {
            asyncStored = try await repository.loadAsync(conversationID: conversationID, windowed: true)
        } catch {
            heartbeat.cancel()
            throw error
        }
        heartbeat.cancel()
        await heartbeat.value
        XCTAssertEqual(asyncStored?.messages.map(\.id), mapped.map(\.id),
            "The async window must match the bounded canonical window, not the full archive")
        XCTAssertGreaterThan(heartbeatCount, 1, "Recorded read must suspend instead of blocking the UI actor")
        print("recorded_long_context_async elapsed=\(asyncStart.duration(to: .now)) main_actor_heartbeats=\(heartbeatCount)")
        // Completion still synchronously persists Mac-owned settings. Measure
        // that exact path on the private copy before changing its ordering.
        let settingsBeforeSave = try port.loadConversationSettings(conversationID)
        let settingsObjectBefore = try settingsBeforeSave.map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any])
        }
        let metadataSaveStart = ContinuousClock.now
        try repository.saveMetadata(conversation: stored.conversation, knowledge: stored.knowledge)
        let metadataSaveDuration = metadataSaveStart.duration(to: .now)
        let settingsAfterSave = try XCTUnwrap(try port.loadConversationSettings(conversationID))
        let settingsObjectAfter = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(settingsAfterSave.utf8)) as? [String: Any])
        // A cross-client field must survive even when Mac does not own it.
        if let style = settingsObjectBefore?["promptResponseStyle"] {
            let preserved = try XCTUnwrap(settingsObjectAfter["promptResponseStyle"])
            XCTAssertEqual(String(describing: preserved), String(describing: style))
        }
        print("recorded_long_context_metadata synchronous_save=\(metadataSaveDuration)")
        for cycle in 0..<3 {
            let beforeUIBytes = try residentBytes()
            try await exerciseRecordedTranscript(stored.messages, conversationID: conversationID, captureSnapshots: cycle == 0)
            try await Task.sleep(for: .milliseconds(250))
            print("recorded_long_context_memory cycle=\(cycle) before_ui_bytes=\(beforeUIBytes) after_window_closed_bytes=\(try residentBytes())")
        }
        if environment["NOLA_LONG_CONTEXT_FULL_WORKSPACE"] == "1" {
            try await exerciseRecordedWorkspace(repository: repository, port: port,
                conversationID: conversationID, expectedIDs: Set(stored.messages.map(\.id)))
        }
        var partCount = 0
        var bodyCount = 0
        var byteCount = 0
        let bodyStart = ContinuousClock.now
        for message in stored.messages {
            guard let reference = message.journalReference else { continue }
            let parts = try await MacJournalTextRead.parts(reference)
            partCount += parts.count
            for part in parts {
                for content in part.bodies {
                    var offset = "0"
                    repeat {
                        let range = try await MacJournalTextRead.range(reference, partID: part.part_id,
                            content: content, offset: offset)
                        byteCount += range.text.utf8.count
                        offset = range.next_offset_bytes
                        if range.end_of_content { break }
                    } while true
                    XCTAssertEqual(offset, content.utf8_bytes)
                    bodyCount += 1
                }
            }
        }
        XCTAssertGreaterThan(partCount, 0)
        XCTAssertGreaterThan(bodyCount, 0)
        print("recorded_long_context messages=\(stored.messages.count) parts=\(partCount) bodies=\(bodyCount) bytes=\(byteCount) port=\(portDuration) cold_index=\(indexDuration) projection=\(projectionDuration) warm_projection=\(warmDuration) body_reads=\(bodyStart.duration(to: .now)) isolated_db=\(databaseURL.path)")
        let wallDuration = start.duration(to: .now)
        let awakeDuration = awakeStart.duration(to: .now)
        // ContinuousClock includes machine sleep. Keep such runs for behavior
        // verification, but do not treat their wall durations as UI stalls.
        print("recorded_long_context_clock wall=\(wallDuration) awake=\(awakeDuration) performance_sample_valid=\(wallDuration - awakeDuration <= .seconds(1))")
    }

    func testLongToolPreviewBoundsLayoutWithoutChangingSource() {
        let source = (0..<1000).map { "line \($0) 错误诊断" }.joined(separator: "\n")
        let preview = MacJournalPlainPreview.text(source)
        XCTAssertEqual(preview.components(separatedBy: "\n").count, 24)
        XCTAssertTrue(source.hasPrefix(preview))
        XCTAssertTrue(source.contains("line 999"))
        XCTAssertEqual(MacJournalPlainPreview.text("short\noutput"), "short\noutput")
        let unbroken = String(repeating: "👨‍👩‍👧‍👦", count: 5000)
        XCTAssertEqual(MacJournalPlainPreview.text(unbroken).count, 4096)
    }

    func testStreamingGeometryBurstCoalescesAndUserScrollWins() async throws {
        final class Document: NSView { override var isFlipped: Bool { true } }
        final class Clip: NSClipView {
            var moves = 0
            override func scroll(to point: NSPoint) {
                moves += 1
                super.scroll(to: point)
            }
        }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 500))
        let clip = Clip(frame: scroll.bounds)
        scroll.contentView = clip
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 5000))
        scroll.documentView = document
        let controller = MacTranscriptScrollController()
        controller.attach(scroll)
        defer { controller.detach() }
        for _ in 0..<200 { controller.contentChanged() }
        XCTAssertEqual(clip.moves, 0, "Geometry callbacks must not scroll synchronously inside layout")
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(clip.moves, 1, "One stream burst should cause one native scroll")
        controller.viewportChanged()
        for _ in 0..<200 { controller.contentChanged() }
        controller.userBeganScrolling()
        clip.scroll(to: NSPoint(x: 0, y: 300))
        controller.userEndedScrolling(distanceFromBottom: 4200)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(clip.bounds.minY, 300, accuracy: 1,
                       "An already queued follow must not undo the user's scroll")
        XCTAssertFalse(controller.followsLatest)
        controller.returnToLatest()
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(scroll.documentVisibleRect.maxY, 5000, accuracy: 1)
    }

    func testGestureEndPreservesLastUserIntentAcrossContentGrowth() throws {
        final class Document: NSView { override var isFlipped: Bool { true } }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 500))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 1000))
        scroll.documentView = document
        let controller = MacTranscriptScrollController()
        controller.attach(scroll)
        defer { controller.detach() }

        controller.userBeganScrolling()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 500))
        controller.userScrolled(distanceFromBottom: 0)
        document.setFrameSize(NSSize(width: 600, height: 1600))
        controller.contentChanged()
        controller.userEndedScrolling(distanceFromBottom: 600)
        controller.reconcileLayout()
        XCTAssertTrue(controller.followsLatest)
        XCTAssertEqual(scroll.documentVisibleRect.maxY, 1600, accuracy: 1)

        controller.userBeganScrolling()
        controller.userScrolled(distanceFromBottom: 0)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 200))
        controller.userScrolled(distanceFromBottom: 900)
        document.setFrameSize(NSSize(width: 600, height: 2000))
        controller.userEndedScrolling(distanceFromBottom: 1300)
        controller.reconcileLayout()
        XCTAssertFalse(controller.followsLatest)
        XCTAssertEqual(scroll.documentVisibleRect.minY, 200, accuracy: 1)
    }

    func testRetiredGestureCannotOverrideSessionOrNavigationIntent() {
        let controller = MacTranscriptScrollController()
        controller.reset(conversationID: "first")
        controller.userBeganScrolling()
        let oldGeneration = controller.generation
        controller.reset(conversationID: "second")
        controller.userEndedScrolling(distanceFromBottom: 800)
        XCTAssertTrue(controller.followsLatest)
        XCTAssertGreaterThan(controller.generation, oldGeneration)

        controller.userBeganScrolling()
        let navigation = controller.beginNavigation()
        controller.userEndedScrolling(distanceFromBottom: 0)
        XCTAssertTrue(controller.acceptsNavigation(navigation),
            "A late gesture-end must not turn explicit navigation into following")
        controller.returnToLatest()
        controller.userEndedScrolling(distanceFromBottom: 800)
        XCTAssertTrue(controller.followsLatest)
        XCTAssertFalse(controller.acceptsNavigation(navigation))
    }

    func testReadingAnchorSurvivesDelayedMediaGrowthAndProcessCollapse() {
        final class Document: NSView { override var isFlipped: Bool { true } }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 500))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 4000))
        scroll.documentView = document
        let marker = NSView(frame: NSRect(x: 0, y: 1000, width: 600, height: 800))
        document.addSubview(marker)
        let controller = MacTranscriptScrollController()
        controller.attach(scroll)
        defer { controller.detach() }
        controller.register(messageID: UUID(), view: marker)
        controller.userBeganScrolling()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 1100))
        controller.userEndedScrolling(distanceFromBottom: 2400)
        for origin in [1600.0, 700.0, 1200.0] {
            // Media above the reader finishes loading; then a process folds.
            marker.setFrameOrigin(NSPoint(x: 0, y: origin))
            controller.reconcileLayout()
            XCTAssertEqual(scroll.documentVisibleRect.minY - marker.frame.minY, 100, accuracy: 1)
            XCTAssertFalse(controller.followsLatest)
        }
    }

    func testFollowUsesMeasuredFooterWhenDocumentRetainsPreCompletionExtent() async throws {
        final class Document: NSView { override var isFlipped: Bool { true } }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 500))
        let document = Document(frame: NSRect(x: 0, y: 0, width: 600, height: 6000))
        scroll.documentView = document
        let controller = MacTranscriptScrollController()
        controller.attach(scroll)
        defer { controller.detach() }
        let footer = MacTranscriptLatestAnchor.AnchorView(frame: NSRect(x: 0, y: 5820, width: 600, height: 180))
        footer.controller = controller
        document.addSubview(footer)
        controller.registerLatestAnchor(footer)
        controller.reconcileLayout()
        XCTAssertEqual(scroll.documentVisibleRect.maxY, 6000, accuracy: 1)

        // Completion shrinks the measured tail before lazy history updates the
        // document estimate. No document-frame notification is delivered here.
        footer.setFrameOrigin(NSPoint(x: 0, y: 1820))
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(document.bounds.height, 6000)
        XCTAssertEqual(scroll.documentVisibleRect.maxY, 2000, accuracy: 1)
        XCTAssertLessThan(scroll.documentVisibleRect.minY, footer.frame.minY)
        XCTAssertEqual(controller.distanceFromLatest(), 0, accuracy: 1,
                       "User scrolling to the measured end must restore follow despite stale document extent.")
        controller.userBeganScrolling()
        controller.userEndedScrolling(distanceFromBottom: controller.distanceFromLatest())
        XCTAssertTrue(controller.followsLatest)

        controller.unregisterLatestAnchor(footer)
        footer.removeFromSuperview()
        controller.returnToLatest()
        controller.reconcileLayout()
        XCTAssertEqual(scroll.documentVisibleRect.maxY, 2000, accuracy: 1,
                       "A transient footer remount must not follow a stale document estimate.")
        document.addSubview(footer)
        controller.registerLatestAnchor(footer)

        controller.userBeganScrolling()
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 300))
        controller.userEndedScrolling(distanceFromBottom: 1200)
        footer.setFrameOrigin(NSPoint(x: 0, y: 2200))
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertFalse(controller.followsLatest)
        XCTAssertEqual(scroll.documentVisibleRect.minY, 300, accuracy: 1)
        controller.returnToLatest()
        controller.reconcileLayout()
        XCTAssertEqual(scroll.documentVisibleRect.maxY, 2380, accuracy: 1)
    }

    func testProductionTranscriptArrowKeepsTerminalAnswerInViewportAfterReflow() async throws {
        try await exerciseProductionTranscriptArrow(reduceMotion: false)
    }

    func testDisabledAnimationTranscriptArrowPreservesHistoryAndReturnsToLatest() async throws {
        try await exerciseProductionTranscriptArrow(reduceMotion: true)
    }

    private func exerciseProductionTranscriptArrow(reduceMotion: Bool) async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let answerID = UUID()
        let fixture = try MacAssistantJournalTestFixture(conversationID: "completion-scroll",
            runID: "completion-run", assistantMessageID: "completion-answer")
        let initial = try fixture.append((0..<35).map {
            "Answer paragraph \($0). " + String(repeating: "Visible answer text. ", count: 8)
        }.joined(separator: "\n\n"))
        let history = (0..<12).map { index in
            MacChatMessage(role: index.isMultiple(of: 2) ? .user : .assistant,
                text: "Earlier message \(index). " + String(repeating: "Earlier content. ", count: 20), createdAt: .now)
        }
        func transcript(cursor: MacAssistantJournalCursor, running: Bool, clearance: CGFloat, darkMode: Bool = false) -> some View {
            var answer = MacChatMessage(id: answerID, role: .assistant, text: "", createdAt: .now)
            answer.journalReference = .init(databaseURL: fixture.databaseURL, cursor: cursor)
            return MacTranscriptView(conversationID: "completion-scroll", messages: history + [answer],
                isRunning: running, liveActivities: [], searchText: "", bottomClearance: clearance,
                terminalRevision: running ? 0 : 1,
                onSendTaskGuidance: { _, _, _ in }, onRequestTaskCancellation: { _, _ in })
                .environment(\.accessibilityEnabled, true)
                .environment(\.colorScheme, darkMode ? .dark : .light)
                .background(darkMode ? Color.black : Color.white)
                .transaction { $0.disablesAnimations = reduceMotion }
        }
        let host = NSHostingView(rootView: transcript(cursor: initial, running: true, clearance: 180))
        host.frame = NSRect(x: 0, y: 0, width: 760, height: 640)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .aqua)
        window.backgroundColor = .windowBackgroundColor
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(host)
        let scroll = try XCTUnwrap(views(host).compactMap { $0 as? NSScrollView }.first)
        let document = try XCTUnwrap(scroll.documentView)
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 200))
        NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
        try await settle(host)
        let arrow = try XCTUnwrap(accessibilityObjects(host).first {
            ($0.accessibilityLabel?() ?? nil) == String(localized: "Go to Latest Message")
        })
        _ = arrow.accessibilityPerformPress?()
        try await settle(host)
        XCTAssertFalse(accessibilityObjects(host).contains {
            ($0.accessibilityLabel?() ?? nil) == String(localized: "Go to Latest Message")
        }, "Pressing the real arrow must return the transcript to follow mode.")

        let appended = try fixture.append("\n\nFinal appended paragraph.\n\nTerminal answer marker.")
        host.rootView = transcript(cursor: appended, running: true, clearance: 180)
        try await settle(host)
        _ = try fixture.finish()
        let final = try XCTUnwrap(fixture.heads().first { $0.identity.assistant_message_id == fixture.assistantMessageID })
        for clearance in [180.0, 140.0] {
            host.rootView = transcript(cursor: final, running: false, clearance: clearance)
            try await settle(host)
            let footer = try XCTUnwrap(views(host).compactMap { $0 as? MacTranscriptLatestAnchor.AnchorView }.first)
            let frame = footer.convert(footer.bounds, to: document)
            XCTAssertEqual(scroll.documentVisibleRect.maxY, frame.maxY, accuracy: 2)
            XCTAssertLessThan(scroll.documentVisibleRect.minY, frame.minY,
                              "Follow mode must leave answer space above the composer clearance.")
            XCTAssertTrue(accessibilityObjects(host).contains {
                let value: Any? = $0.accessibilityValue?() ?? nil
                return String(describing: value ?? "").contains("Terminal answer marker")
            }, "The production terminal answer must remain rendered after completion.")
        }
        for darkMode in [false, true] {
          for width in [640, 1440] {
            window.appearance = NSAppearance(named: darkMode ? .darkAqua : .aqua)
            host.rootView = transcript(cursor: final, running: false, clearance: 140, darkMode: darkMode)
            window.setContentSize(NSSize(width: width, height: 760))
            try await settle(host)
            let marker = try XCTUnwrap(accessibilityObjects(host).first {
                let value: Any? = $0.accessibilityValue?() ?? nil
                return String(describing: value ?? "").contains("Terminal answer marker")
            })
            let markerFrame: NSRect = marker.accessibilityFrame?() ?? .zero
            let viewport = window.convertToScreen(scroll.contentView.convert(scroll.contentView.bounds, to: nil))
            XCTAssertFalse(markerFrame.isEmpty)
            XCTAssertTrue(markerFrame.intersects(viewport),
                          "The final answer must visibly intersect the viewport after completion and resize.")
            let markerRect = host.convert(window.convertFromScreen(markerFrame), from: nil).intersection(host.bounds)
            let markerBitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: markerRect))
            host.cacheDisplay(in: markerRect, to: markerBitmap)
            var inkPixels = 0
            for y in stride(from: 0, to: markerBitmap.pixelsHigh, by: 2) {
                for x in stride(from: 0, to: markerBitmap.pixelsWide, by: 2) {
                    guard let color = markerBitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                    let brightness = (color.redComponent + color.greenComponent + color.blueComponent) / 3
                    if color.alphaComponent > 0.9, darkMode ? brightness > 0.7 : brightness < 0.65 {
                        inkPixels += 1
                    }
                }
            }
            XCTAssertGreaterThan(inkPixels, 20,
                "The final answer's AX bounds must contain painted glyphs, not merely a blank text layout.")
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            let suffix = darkMode ? "-dark" : ""
            try png.write(to: URL(fileURLWithPath: "/tmp/nola-chat-terminal-\(width)\(suffix).png"))
            let image = NSImage(size: host.bounds.size)
            image.addRepresentation(bitmap)
            let attachment = XCTAttachment(image: image)
            attachment.name = "transcript-follow-after-completion-\(width)\(suffix)"
            attachment.lifetime = .keepAlways
            add(attachment)
          }
        }
    }

    /// Render the recorded projection without creating an agent driver. Journal
    /// readers remain real; captured pending tool calls must never be resumed.
    private func exerciseRecordedWorkspace(repository: MacCanonicalConversationRepository,
                                           port: MacApplicationPort, conversationID: String,
                                           expectedIDs: Set<UUID>) async throws {
        let workspaceStart = SuspendingClock.now
        // Never inject the real runtime into a captured active conversation.
        // Persistence-only adapters reject execution; the production workspace
        // may inspect recovery, but cannot dispatch or resume captured effects.
        let runtime = PersistenceTestLocalRuntime()
        let remote = PersistenceTestRemoteHost()
        let suite = "nola-recorded-workspace-\(UUID())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = MacWorkspaceModel(initialConversationID: conversationID,
            canonicalRepository: repository, localAgentRuntime: runtime, remoteHost: remote,
            modelApplication: port, bookmarkStore: MacSecurityScopedBookmarkStore(defaults: defaults))
        let modelReady = SuspendingClock.now
        print("recorded_workspace_phase model_init=\(workspaceStart.duration(to: modelReady))")
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let host = NSHostingView(rootView: MacChatWorkspaceView(initialConversationID: conversationID,
            forcesInspectorVisible: true, canonicalRepository: repository,
            localAgentRuntime: runtime, remoteHost: remote,
            inspectorDependencies: MacInspectorDependencies(dataRegistry: MacDataSourceRegistry.shared, platformData: MacPlatformDataService.shared,
                webSources: MacWebSourceStore.shared, chromeMessaging: MacChromeMessagingService.shared),
            model: model).environment(\.accessibilityEnabled, true).environment(\.colorScheme, .dark))
        host.frame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled, .resizable],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let windowAttached = SuspendingClock.now
        print("recorded_workspace_phase window_attach=\(modelReady.duration(to: windowAttached))")
        // The mounted production view starts its own initial load. Do not
        // issue another load here or delay readiness sampling until it returns.
        try await measureRecordedWorkspaceReadiness(host, model: model, start: workspaceStart, phase: "initial")
        if ProcessInfo.processInfo.environment["NOLA_SERVICE_QUERY_TIMING"] == "1" {
            let timings = MacApplicationServiceSupervisor.shared.queryTimingsForTesting(databaseURL: port.databaseURL)
            XCTAssertFalse(timings.isEmpty, "An enabled diagnostic must actually capture service queries")
            print(timings)
        }
        try await settle(host)
        XCTAssertFalse(model.selectedMessages.isEmpty)
        XCTAssertEqual(model.selectedConversationDisplayStatus, .uncertain)
        XCTAssertTrue(accessibilityObjects(host).contains {
            let value: Any? = $0.accessibilityValue?() ?? nil
            return ($0.accessibilityLabel?() ?? nil) == MacConversationDisplayStatus.uncertain.title
                || (value as? String) == MacConversationDisplayStatus.uncertain.title
        }, "The rendered overview must disclose the recorded unresolved Run, not Ready")
        // Expected IDs come from the fixture read before the window opens.
        // Loading the full archive here blocks the main actor and contaminates
        // the following history interaction with a test-only synchronous read.
        let latestIDs = model.selectedMessages.map(\.id)
        var visitedIDs = Set(latestIDs)
        var pages = 0
        while model.hasEarlierStoredMessages && pages < 50 {
            if ProcessInfo.processInfo.environment["NOLA_PROFILE_HISTORY"] == "1" {
                // Opt-in rendezvous for an external sampler, before hydration
                // starts. Otherwise startup samples can miss the short stall.
                print("recorded_profile_history_begin page=\(pages + 1)")
                fflush(stdout)
                try await Task.sleep(for: .seconds(2))
            }
            if let directory = ProcessInfo.processInfo.environment["NOLA_HISTORY_PROFILE_DIRECTORY"] {
                // An external sampler can wait for this exact phase rather
                // than guessing from process launch. Do not add a settling
                // delay: it can hide the hydration tail being investigated.
                let marker = URL(fileURLWithPath: directory, isDirectory: true)
                    .appendingPathComponent("history-page-\(pages + 1).pid")
                try String(ProcessInfo.processInfo.processIdentifier)
                    .write(to: marker, atomically: true, encoding: .utf8)
            }
            let previousIDs = model.selectedMessages.map(\.id)
            let earlier = try XCTUnwrap(accessibilityObjects(host).first {
                ($0.accessibilityLabel?() ?? nil) == String(localized: "Load earlier messages")
            })
            XCTAssertEqual(earlier.accessibilityPerformPress?(), true)
            let deadline = SuspendingClock.now + .seconds(15)
            while model.selectedMessages.map(\.id) == previousIDs && SuspendingClock.now < deadline {
                try await settle(host)
            }
            XCTAssertNotEqual(model.selectedMessages.map(\.id), previousIDs,
                "Pressing the real history control must publish a different canonical page")
            guard model.selectedMessages.map(\.id) != previousIDs else { break }
            let pageIDs = model.selectedMessages.map(\.id)
            visitedIDs.formUnion(pageIDs)
            pages += 1
            let anchor = try XCTUnwrap(views(host).compactMap { $0 as? MacTranscriptLatestAnchor.AnchorView }.first)
            let scroll = try XCTUnwrap(anchor.enclosingScrollView)
            try await exerciseRecordedScrollSweep(host, scroll: scroll, phase: "history_page_\(pages)")
            if ProcessInfo.processInfo.environment["NOLA_SERVICE_QUERY_TIMING"] == "1" {
                print("recorded_history_service_timings page=\(pages)")
                print(MacApplicationServiceSupervisor.shared.queryTimingsForTesting(databaseURL: port.databaseURL))
            }
            XCTAssertEqual(model.selectedMessages.map(\.id), pageIDs,
                "An immutable recorded page must not acquire unrelated live rows while scrolling")
            print("recorded_full_workspace_history page=\(pages) visible_messages=\(model.selectedMessages.count) resident_bytes=\(try residentBytes())")
        }
        XCTAssertGreaterThan(pages, 0)
        XCTAssertEqual(visitedIDs, expectedIDs, "Every recorded message must be reachable through history paging")
        let latest = try XCTUnwrap(accessibilityObjects(host).first {
            ($0.accessibilityIdentifier?() ?? nil) == "chat.history.returnToLatest"
        })
        XCTAssertEqual(latest.accessibilityPerformPress?(), true)
        let latestDeadline = SuspendingClock.now + .seconds(15)
        while model.isViewingStoredHistory && SuspendingClock.now < latestDeadline {
            try await settle(host)
        }
        XCTAssertFalse(model.isViewingStoredHistory)
        XCTAssertEqual(model.selectedMessages.map(\.id), latestIDs)
        for cycle in 0..<3 {
            model.newConversation()
            let draftID = try XCTUnwrap(model.selectedConversationID)
            model.draft = "RECORDED-WORKSPACE-DRAFT-\(cycle)"
            try await settle(host)
            XCTAssertFalse(model.selectedComposerState.isRunning)
            let start = SuspendingClock.now
            model.selectedConversationID = conversationID
            try await measureRecordedWorkspaceReadiness(host, model: model, start: start, phase: "switch_\(cycle)")
            try await settle(host)
            XCTAssertFalse(model.selectedMessages.isEmpty)
            let anchor = try XCTUnwrap(views(host).compactMap { $0 as? MacTranscriptLatestAnchor.AnchorView }.first)
            let scroll = try XCTUnwrap(anchor.enclosingScrollView)
            try await exerciseRecordedScrollSweep(host, scroll: scroll)
            print("recorded_full_workspace cycle=\(cycle) switch_and_sweep=\(start.duration(to: .now)) resident_bytes=\(try residentBytes())")
            model.selectedConversationID = draftID
            await model.loadSelectedConversationIfNeeded()
            try await settle(host)
            XCTAssertEqual(model.draft, "RECORDED-WORKSPACE-DRAFT-\(cycle)")
        }
        model.selectedConversationID = conversationID
        await model.loadSelectedConversationIfNeeded()
        try await settle(host)
        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: "/tmp/nola-recorded-full-workspace.png"))
    }

    private func exerciseRecordedTranscript(_ messages: [MacChatMessage], conversationID: String, captureSnapshots: Bool) async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let start = ContinuousClock.now
        let host = NSHostingView(rootView:
            MacTranscriptView(conversationID: conversationID, messages: messages,
                isRunning: false, liveActivities: [], searchText: "", bottomClearance: 140,
                terminalRevision: 0, onSendTaskGuidance: { _, _, _ in }, onRequestTaskCancellation: { _, _ in })
                .environment(\.accessibilityEnabled, true)
                .environment(\.colorScheme, .dark)
                .background(Color(nsColor: .windowBackgroundColor)))
        host.frame = NSRect(x: 0, y: 0, width: 1100, height: 760)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(host)
        let scroll = try XCTUnwrap(views(host).compactMap { $0 as? NSScrollView }.first)
        let document = try XCTUnwrap(scroll.documentView)
        // Wait for journal readers to populate and the layout to become stable,
        // not merely for the synchronous projection to exist.
        var previousHeight: CGFloat = -1
        var stable = 0
        for _ in 0..<80 {
            try await settle(host)
            let height = document.bounds.height
            stable = abs(height - previousHeight) < 1 ? stable + 1 : 0
            previousHeight = height
            if stable >= 8 { break }
        }
        XCTAssertGreaterThanOrEqual(stable, 8, "Recorded transcript must reach a stable layout")
        XCTAssertGreaterThan(document.bounds.height, scroll.bounds.height)
        print("recorded_long_context_ui initial_settled=\(start.duration(to: .now)) document_height=\(document.bounds.height)")
        print("recorded_long_context_memory settled_bytes=\(try residentBytes())")
        if captureSnapshots {
            try await exerciseRecordedScrollSweep(host, scroll: scroll)
            let group = try XCTUnwrap(accessibilityObjects(host).first {
                ($0.accessibilityLabel?() ?? nil) == String(localized: "\(3) completed tools", table: "Chat")
            }, "Recorded completed tools should expose their disclosure control")
            let collapsedHeight = document.bounds.height
            XCTAssertEqual(group.accessibilityPerformPress?(), true)
            try await settle(host)
            XCTAssertGreaterThan(document.bounds.height, collapsedHeight,
                "Opening the real completed-tool group must mount its original rows")
            XCTAssertEqual(group.accessibilityPerformPress?(), true)
            try await settle(host)
            XCTAssertEqual(document.bounds.height, collapsedHeight, accuracy: 2)
        }
        for width in [640, 1440] {
            window.setContentSize(NSSize(width: width, height: 760))
            try await settle(host)
            NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
            let target = max(0, (document.bounds.height - scroll.bounds.height) / 2)
            scroll.contentView.scroll(to: NSPoint(x: 0, y: target))
            scroll.reflectScrolledClipView(scroll.contentView)
            NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll)
            try await settle(host)
            let held = scroll.documentVisibleRect.minY
            try await settle(host)
            XCTAssertEqual(scroll.documentVisibleRect.minY, held, accuracy: 2,
                "Asynchronous journal reads must not pull a user away from history")
            let arrow = try XCTUnwrap(accessibilityObjects(host).first {
                ($0.accessibilityLabel?() ?? nil) == String(localized: "Go to Latest Message")
            })
            let followStart = ContinuousClock.now
            _ = arrow.accessibilityPerformPress?()
            try await settle(host)
            let footer = try XCTUnwrap(views(host).compactMap { $0 as? MacTranscriptLatestAnchor.AnchorView }.first)
            let frame = footer.convert(footer.bounds, to: document)
            XCTAssertEqual(scroll.documentVisibleRect.maxY, frame.maxY, accuracy: 2)
            XCTAssertLessThan(scroll.documentVisibleRect.minY, frame.minY)
            print("recorded_long_context_ui width=\(width) follow_settled=\(followStart.duration(to: .now)) document_height=\(document.bounds.height)")
            print("recorded_long_context_memory width=\(width) resident_bytes=\(try residentBytes())")
            if captureSnapshots {
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: bitmap)
                let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                try png.write(to: URL(fileURLWithPath: "/tmp/nola-recorded-transcript-\(width).png"))
            }
        }
    }

    private func residentBytes() throws -> UInt64 {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
        let capacity = Int(count)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: capacity) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        guard result == KERN_SUCCESS else {
            throw NSError(domain: "RecordedTranscriptMemory", code: Int(result))
        }
        return info.resident_size
    }

    private func exerciseRecordedScrollSweep(_ host: NSView, scroll: NSScrollView, phase: String = "transcript") async throws {
        let document = try XCTUnwrap(scroll.documentView)
        let diagnoseRunLoop = ProcessInfo.processInfo.environment["NOLA_DIAGNOSE_RUNLOOP"] == "1"
        let activityProbe = diagnoseRunLoop ? RecordedRunLoopActivityProbe() : nil
        defer { activityProbe?.stop(phase: phase) }
        let naturalLayout = ProcessInfo.processInfo.environment["NOLA_LONG_CONTEXT_NATURAL_LAYOUT"] == "1"
        // Diagnostic control: separate elapsed-time triggers from viewport
        // position without changing the recorded content or normal 120-step gate.
        let requestedSteps = Int(ProcessInfo.processInfo.environment["NOLA_SCROLL_SWEEP_STEPS"] ?? "") ?? 120
        let steps = min(480, max(60, requestedSteps))
        var milliseconds: [Double] = []
        var wakeDelays: [Double] = []
        var worst = (index: 0, scroll: 0.0, layout: 0.0, display: 0.0)
        var worstWake = (index: 0, milliseconds: 0.0)
        func ms(_ duration: Duration) -> Double {
            let value = duration.components
            return Double(value.seconds) * 1_000 + Double(value.attoseconds) / 1e15
        }
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        defer { NotificationCenter.default.post(name: NSScrollView.didEndLiveScrollNotification, object: scroll) }
        if let directory = ProcessInfo.processInfo.environment["NOLA_HISTORY_PROFILE_DIRECTORY"] {
            let marker = URL(fileURLWithPath: directory, isDirectory: true)
                .appendingPathComponent("sweep-\(phase).pid")
            try String(ProcessInfo.processInfo.processIdentifier)
                .write(to: marker, atomically: true, encoding: .utf8)
        }
        for index in 0..<steps {
            let start = SuspendingClock.now
            let available = max(0, document.bounds.height - scroll.contentView.bounds.height)
            scroll.contentView.scroll(to: NSPoint(x: 0, y: available * CGFloat(index) / CGFloat(steps - 1)))
            scroll.reflectScrolledClipView(scroll.contentView)
            let scrolled = SuspendingClock.now
            if !naturalLayout { host.layoutSubtreeIfNeeded() }
            let laidOut = SuspendingClock.now
            if !naturalLayout { host.displayIfNeeded() }
            let displayed = SuspendingClock.now
            let elapsed = ms(start.duration(to: displayed))
            if elapsed > worst.scroll + worst.layout + worst.display {
                worst = (index, ms(start.duration(to: scrolled)), ms(scrolled.duration(to: laidOut)), ms(laidOut.duration(to: displayed)))
            }
            milliseconds.append(elapsed)
            // Let real SwiftUI/AppKit work run between native scroll updates.
            let sleepStart = SuspendingClock.now
            try await Task.sleep(for: .milliseconds(16))
            let wakeDelay = max(0, ms(sleepStart.duration(to: SuspendingClock.now)) - 16)
            if wakeDelay > 40, diagnoseRunLoop || ProcessInfo.processInfo.environment["NOLA_DIAGNOSE_BODY_HYDRATION"] == "1" {
                print("recorded_slow_wake uptime=\(ProcessInfo.processInfo.systemUptime) phase=\(phase) index=\(index) late_ms=\(wakeDelay)")
            }
            wakeDelays.append(wakeDelay)
            if wakeDelay > worstWake.milliseconds { worstWake = (index, wakeDelay) }
        }
        milliseconds.sort()
        wakeDelays.sort()
        // Scheduling lateness includes main-actor congestion, not just rendering;
        // it must not be reported as frame time or physical scrolling FPS.
        print("recorded_scroll_schedule phase=\(phase) natural_layout=\(naturalLayout) steps=\(steps) wake_lateness_p95_ms=\(wakeDelays[steps * 95 / 100]) wake_lateness_max_ms=\(wakeDelays[steps - 1]) worst_wake_index=\(worstWake.index)")
        // In natural mode AppKit/SwiftUI layout occurs during suspension, not
        // inside the synchronous scroll-update interval measured above.
        print("recorded_long_context_scroll samples=\(milliseconds.count) includes_forced_layout=\(!naturalLayout) update_p50_ms=\(milliseconds[steps / 2]) update_p95_ms=\(milliseconds[steps * 95 / 100]) update_max_ms=\(milliseconds[steps - 1])")
        print("recorded_scroll_phases phase=\(phase) worst_index=\(worst.index) scroll_ms=\(worst.scroll) layout_ms=\(worst.layout) display_ms=\(worst.display)")
        XCTAssertEqual(scroll.documentVisibleRect.maxY, document.bounds.maxY, accuracy: 2)
    }

    /// Sample readiness before the scroll sweep and its fixed settling delays.
    /// This is a conservative observed UI-ready time, not display-server FPS or
    /// the exact first painted frame. No captured work is submitted.
    private func measureRecordedWorkspaceReadiness(_ host: NSView, model: MacWorkspaceModel,
                                                   start: SuspendingClock.Instant, phase: String) async throws {
        let deadline = SuspendingClock.now + .seconds(15)
        repeat {
            try await Task.sleep(for: .milliseconds(16))
            let editor = views(host).compactMap { $0 as? MacAttachmentPasteTextView }.first
            let hasRenderedAnswer = accessibilityObjects(host).contains {
                let value: Any? = $0.accessibilityValue?() ?? nil
                return ($0.accessibilityIdentifier?() ?? nil) == "mac-assistant-response-bubble"
                    && !(value as? String ?? "").isEmpty
            }
            if !model.selectedMessages.isEmpty, hasRenderedAnswer,
               let editor, editor.isEditable, !editor.visibleRect.isEmpty {
                let originalDraft = editor.string
                let marker = "READINESS-PROBE"
                editor.insertText(marker, replacementRange: NSRange(location: 0, length: (originalDraft as NSString).length))
                XCTAssertEqual(model.draft, marker, "Native input must commit to the selected conversation")
                editor.insertText(originalDraft, replacementRange: NSRange(location: 0, length: (editor.string as NSString).length))
                XCTAssertEqual(model.draft, originalDraft)
                print("recorded_workspace_ready phase=\(phase) observed_ready=\(start.duration(to: .now)) sample_interval_ms=16 native_edit_verified=true")
                return
            }
        } while SuspendingClock.now < deadline
        XCTFail("Recorded workspace did not expose rendered answer text and an editable composer: \(phase)")
    }

    private func settle(_ host: NSView) async throws {
        let naturalLayout = ProcessInfo.processInfo.environment["NOLA_LONG_CONTEXT_NATURAL_LAYOUT"] == "1"
        for _ in 0..<12 {
            try await Task.sleep(for: .milliseconds(20))
            if !naturalLayout {
                host.layoutSubtreeIfNeeded()
                host.displayIfNeeded()
            }
        }
    }

    private func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(views) }

    private func accessibilityObjects(_ root: NSView) -> [AnyObject] {
        var queue: [AnyObject] = [root]
        var visited: Set<ObjectIdentifier> = []
        var result: [AnyObject] = []
        var cursor = 0
        while cursor < queue.count, result.count < 20000 {
            let object = queue[cursor]
            cursor += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children: [Any] = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: children.map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }
}

@MainActor final class MacMessageSurfaceContextTests: XCTestCase {
    private func identity(conversation: String = "child", message: String = "assistant", run: String = "run", generation: UInt64 = 7) -> AppleUISurfaceIdentity {
        .init(owner: .init(authority_id: "real-authority", conversation_id: conversation, participant_id: "expert",
            context_id: "private-context", originating_run_id: run, message_id: message, part_id: "part"),
            surface_id: "form", generation: generation)
    }
    private func context(read: @escaping (AppleUISurfaceIdentity, UInt64?) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue>,
                         submit: @escaping (AppleUISubmission<MacCanonicalJSONValue>) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue> = { _ in .rejected("Host rejected") }) -> MacUISurfaceContext {
        .init(readChanges: read, preview: { _ in throw MacApplicationPortError.invalidResponse },
              readRevision: { _ in nil }, pendingSubmission: { _ in nil }, submit: submit)
    }
    func testChildScopeForwardsCompleteIdentityAndRevisionToOriginalHost() async throws {
        let expected = identity()
        var observed: AppleUISurfaceIdentity?
        var observedRevision: UInt64?
        let callbacks = context(read: { identity, revision in
            observed = identity; observedRevision = revision
            return .unchanged(identity, revision ?? 0)
        }).scoped(to: .init(conversationID: "child", messageID: "assistant", runID: "run"), isCurrent: { true })
        _ = try await callbacks.readChanges(expected, 13)
        XCTAssertEqual(observed, expected)
        XCTAssertEqual(observed?.generation, 7)
        XCTAssertEqual(observedRevision, 13)
        for foreign in [identity(conversation: "parent"), identity(message: "other"), identity(run: "next-run")] {
            do { _ = try await callbacks.readChanges(foreign, nil); XCTFail("Foreign owner must not reach child Host") }
            catch { }
            XCTAssertEqual(observed, expected)
        }
    }
    func testParentLifetimeInvalidatesPendingAndLateReadsWithoutChangingTheChildOwner() async throws {
        var current = true
        let expected = identity()
        let gate = MacSurfaceContextReadGate()
        let callbacks = context(read: { identity, revision in
            await gate.suspend()
            return .unchanged(identity, revision ?? 0)
        }).scoped(to: .init(conversationID: "child", messageID: "assistant", runID: "run"), isCurrent: { current })
        let read = Task { try await callbacks.readChanges(expected, 3) }
        try await gate.waitUntilSuspended()
        current = false
        gate.release()
        do { _ = try await read.value; XCTFail("Late result from the detached parent must not render") }
        catch { }
        XCTAssertThrowsError(try callbacks.pendingSubmission(expected))
        XCTAssertEqual(expected.owner.conversation_id, "child")
    }
    func testReadonlyExpertContextCannotReachGenericQueuedRunFactory() async throws {
        var submissions = 0
        var raw = context(read: { identity, revision in .unchanged(identity, revision ?? 0) }, submit: { _ in
            submissions += 1
            return .rejected("Host result")
        })
        raw.permitsFollowupActions = false
        let callbacks = raw.scoped(to: .init(conversationID:"child", messageID:"assistant", runID:"run"), isCurrent:{true})
        XCTAssertFalse(callbacks.permitsFollowupActions)
        let input = AppleUISubmission<MacCanonicalJSONValue>(interaction:.init(identity:identity(),expected_surface_revision:1,edits:[],action:nil),
            submission_id:UUID().uuidString,submitted_at_ms:1)
        guard case .rejected = await callbacks.submit(input) else { return XCTFail("Read-only expert action must remain explicit unavailable") }
        XCTAssertEqual(submissions,0)
    }

    func testReadonlyVersionValidationRequiresWholeIdentityRevisionPhaseAndNoActions() throws {
        let url = try XCTUnwrap(Bundle(for:Self.self).url(forResource:"form_interaction_v0_9_1",withExtension:"json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with:Data(contentsOf:url)) as? [String:Any])
        func decode(_ key:String) throws -> AppleUISnapshot<MacCanonicalJSONValue> {
            try JSONDecoder().decode(AppleUISnapshot<MacCanonicalJSONValue>.self,
                from:JSONSerialization.data(withJSONObject:try XCTUnwrap(fixture[key])))
        }
        let historical = try decode("historical_snapshot")
        let reference = AppleUISurfaceReference(identity:historical.identity,revision:historical.revision)
        XCTAssertTrue(AppleUIReadOnlyReferenceValidation.matches(historical,reference:reference))
        XCTAssertFalse(AppleUIReadOnlyReferenceValidation.matches(historical,
            reference:.init(identity:historical.identity,revision:historical.revision+1)))
        let interactive = try decode("edited_snapshot")
        XCTAssertFalse(AppleUIReadOnlyReferenceValidation.matches(interactive,
            reference:.init(identity:interactive.identity,revision:interactive.revision)))
        let injected = AppleUISnapshot(identity:interactive.identity,revision:interactive.revision,
            catalog_id:interactive.catalog_id,catalog_digest:interactive.catalog_digest,phase:.readOnly,
            root_instance_id:interactive.root_instance_id,nodes:interactive.nodes,summary:interactive.summary)
        XCTAssertFalse(AppleUIReadOnlyReferenceValidation.matches(injected,
            reference:.init(identity:interactive.identity,revision:interactive.revision)),
            "A renderer must not make a mutable snapshot readonly by changing only its phase")
    }

    func testActualCanonicalSourceDeterminesLocalAttachmentAccessEvenWithoutJournalReference() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("attachment-source-\(UUID())")
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        defer { try? FileManager.default.removeItem(at:directory) }
        let port = try MacApplicationPort(databaseURL:directory.appendingPathComponent("mirror.db"))
        _ = try port.createConversation(.init(id:"local",title:"Local",settingsJSON:nil,createdAtMs:1,updatedAtMs:1))
        let source = try MacAssistantJournalTestFixture(conversationID:"remote",runID:"remote-run",assistantMessageID:"remote-assistant")
        _ = try source.append("Remote source")
        try source.ingest(into:port,through:try XCTUnwrap(source.heads().last))
        let model = MacWorkspaceModel(canonicalRepository:MacCanonicalConversationRepository(store:port),
            localAgentRuntime:PersistenceTestLocalRuntime(),remoteHost:PersistenceTestRemoteHost(),modelApplication:port)
        defer { model.presentationDidDetach() }
        XCTAssertTrue(try model.isLocalConversationSource("local"))
        XCTAssertFalse(try model.isLocalConversationSource("remote"))
        XCTAssertThrowsError(try model.isLocalConversationSource("missing"))
        model.selectedConversationID = "local"
        XCTAssertFalse(try model.isLocalConversationSource("remote"), "The parent's selected local context grants no local file access to a source-owned child")
    }

}

@MainActor private final class MacSurfaceContextReadGate {
    private var continuation: CheckedContinuation<Void, Never>?
    func suspend() async { await withCheckedContinuation { continuation = $0 } }
    func waitUntilSuspended() async throws {
        let deadline = SuspendingClock.now + .seconds(3)
        while continuation == nil, SuspendingClock.now < deadline { await Task.yield() }
        guard continuation != nil else { throw MacApplicationPortError.invalidResponse }
    }
    func release() { continuation?.resume(); continuation = nil }
}
