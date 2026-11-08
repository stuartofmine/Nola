// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies native macOS Markdown keeps document structure and recognizes rich blocks.

import XCTest
import SwiftUI
import AppKit
@testable import Nola

final class MacMarkdownParserTests: XCTestCase {
    @MainActor
    func testControlInteractionColorsResolveInBothAppearances() throws {
        func renderedChannel(_ color: Color, scheme: ColorScheme) throws -> CGFloat {
            let view = NSHostingView(rootView: Rectangle().fill(color)
                .background(scheme == .dark ? Color.black : Color.white)
                .frame(width: 32, height: 32).environment(\.colorScheme, scheme))
            view.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            view.frame = NSRect(x: 0, y: 0, width: 32, height: 32)
            view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            return try XCTUnwrap(bitmap.colorAt(x: 16, y: 16)?.usingColorSpace(.sRGB)).redComponent
        }
        // Exercise SwiftUI's actual asset resolution. NSColor conversion alone
        // can cache the ambient appearance and miss the view's color scheme.
        let lightHover = try renderedChannel(MacDesignColor.hover, scheme: .light)
        let darkHover = try renderedChannel(MacDesignColor.hover, scheme: .dark)
        XCTAssertLessThan(lightHover, 0.98)
        XCTAssertGreaterThan(darkHover, 0.08)
        XCTAssertLessThan(try renderedChannel(MacDesignColor.pressed, scheme: .light), lightHover)
        XCTAssertGreaterThan(try renderedChannel(MacDesignColor.pressed, scheme: .dark), darkHover)
    }

    func testNestedMarkdownFencesKeepTheCodeBlockTogether() {
        let source = "````markdown\n```swift\nlet x = 1\n```\n\n~~~\n\n````\n\nAfter"
        let live = AppleStreamingMarkdownPartitioner.partition(source, settleTail: false)
        XCTAssertEqual(live.count, 2)
        XCTAssertTrue(live[0].markdown.hasSuffix("````\n\n"))
        XCTAssertFalse(live[0].isMutableTail)
        XCTAssertEqual(live[1].markdown, "After")
    }

    func testAutolinksPreserveCodeAndExistingMarkdown() {
        for source in ["`https://example.com/image.png?r=1`", "[photo](https://example.com/photo.png)", "<https://example.com>", "``https://example.com/`literal``"] {
            XCTAssertEqual(MacMarkdownInlinePreprocessor.prepare(source, repairingIncompleteSyntax: false), source)
        }
        XCTAssertEqual(MacMarkdownInlinePreprocessor.prepare("https://example.com。", repairingIncompleteSyntax: false), "[https://example.com](https://example.com)。")
    }

    @MainActor
    func testStaticAndSuppressedRevealLiveMarkdownKeepTheSameLayout() {
        let source = """
        ## 历史消息与实时消息

        **中文与 English** 保持相同换行、行距和代码排版。👩🏽‍💻

        ```swift
        let answer = 42
        print("你好")
        ```

        后续正文继续显示，不应因绘制路径切换而跳动。
        """
        for width: CGFloat in [420, 960] {
            func measured(_ streaming: Bool) -> CGSize {
                let view = NSHostingView(rootView: MacMarkdownView(markdown: source,
                    isStreaming: streaming, animatesAppends: streaming, suppressReveal: true)
                    .frame(width: width).fixedSize(horizontal: false, vertical: true))
                view.layoutSubtreeIfNeeded()
                return view.fittingSize
            }
            let historical = measured(false)
            let live = measured(true)
            XCTAssertGreaterThan(historical.height, 0)
            XCTAssertEqual(historical.width, live.width, accuracy: 0.5)
            XCTAssertEqual(historical.height, live.height, accuracy: 0.5)
        }
    }

    func testCJKStrongEmphasisAndLiteralCodeRemainDistinct() throws {
        for source in ["论文的**完整可编译 LaTeX 全文（含方法、实验、图表占位）**尚未写出。", "**已完成**，后续继续。"] {
            let parsed = try AttributedString(markdown: source, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))
            let repaired = MacMarkdownInlinePreprocessor.repairUnparsedStrongEmphasis(parsed, source: source)
            XCTAssertFalse(String(repaired.characters).contains("**"))
            XCTAssertTrue(repaired.runs.contains { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true })
        }
        for source in ["`**literal**`", #"\*\*原样保留\*\*"#] {
            let parsed = try AttributedString(markdown: source, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))
            XCTAssertEqual(MacMarkdownInlinePreprocessor.repairUnparsedStrongEmphasis(parsed, source: source), parsed)
        }
    }

    @MainActor
    func testNativeReviewMarkdownAndFolderConsentVisualFixture() throws {
        XCTAssertNotEqual(MacToolPresentationPolicy.displayTitle(for: "workspace_access"), "workspace_access")
        let request = MacRemotePermissionRequest(id: "folder", allowConversation: true, protocolVersion: 4, nonce: "nonce",
            hostID: "local", clientID: "mac", sessionID: "fixture", toolName: "workspace_access",
            arguments: #"{"root":"/Users/example/Desktop/paper-project"}"#,
            reason: "读取论文素材并在授权目录内准备编译环境。", message: "Confirm", expiresAt: nil)
        let source = """
        ## 论文评审
        论文的**完整可编译 LaTeX 全文（含方法、实验、图表占位）**尚未写出，这是本轮未完成的部分。

        不过，改写所需的素材已准备好：

        - **正文**（`main.tex`，374 行）：Introduction / Receiver Information and Reconstruction / Residual Encodings for Gated DeltaNet / Unified Headwise Evaluation
        - **生成表格**：`generated/table_s411_final.tex`、`table_s411_boundary_final.tex`、`table_s411_coverage.tex`、`table_common_summary.tex`、`table_factor_policy.tex`
        - **参考文献**：`references.bib`，17 条

        **结论：需要修订**。<br>正文与公式应保持可读。

        信息充分性：$\\ker P_t \\subseteq \\ker P_s$，**修正秩 $r_\\gamma$ 与增益有关**。

        | 项目 | 结果 |
        | --- | --- |
        | 数学表达式 | $S_b=P_bX+Q_b$ |
        | 环境 | `tectonic` 已检测 |
        """
        for dark in [false, true] {
            let root = VStack(alignment: .leading, spacing: 20) {
                MacMarkdownView(markdown: source, isStreaming: false, animatesAppends: false)
                MacHostPermissionCard(request: request, allowOnce: {}, allowConversation: {}, deny: {},
                                      isResolving: false, provideInstruction: { _ in })
            }.padding(20).frame(width: 640).background(Color(nsColor: .windowBackgroundColor))
                .environment(\.colorScheme, dark ? .dark : .light)
            let view = NSHostingView(rootView: root)
            view.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            view.frame.size = view.fittingSize
            view.layoutSubtreeIfNeeded()
            XCTAssertGreaterThan(view.frame.height, 200)
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let image = NSImage(size: view.bounds.size)
            image.addRepresentation(bitmap)
            let attachment = XCTAttachment(image: image)
            attachment.name = dark ? "markdown-folder-dark" : "markdown-folder-light"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    func testMathDelimitersPreserveCodeEscapesAndCurrency() {
        XCTAssertEqual(MacMarkdownMath.segments(#"value $S_b=P_bX+Q_b$ end"#), [
            .text("value "), .math(source: "S_b=P_bX+Q_b", display: false, original: "$S_b=P_bX+Q_b$"), .text(" end")
        ])
        for literal in [#"`$x$`"#, #"\$5 and \$10"#, "$5 and $10", "unfinished $x"] {
            XCTAssertEqual(MacMarkdownMath.segments(literal), [.text(literal)])
        }
        XCTAssertEqual(MacMarkdownMath.segments(#"\[a+b\]"#), [.math(source: "a+b", display: true, original: #"\[a+b\]"#)])
        XCTAssertEqual(MacMarkdownMath.segments("$$a\nb$$"), [.math(source: "a\nb", display: true, original: "$$a\nb$$")])
    }

    func testHTMLBreaksRenderWithoutExecutingHTMLOrChangingCode() {
        XCTAssertEqual(MacMarkdownMath.lineBreaks("first<br>second<BR />third"), "first\nsecond\nthird")
        XCTAssertEqual(MacMarkdownMath.lineBreaks("`<br>` and ``<br/>``"), "`<br>` and ``<br/>``")
        XCTAssertEqual(MacMarkdownMath.lineBreaks("<script>bad()</script>"), "<script>bad()</script>")
    }

    func testTableGrammarIsSharedAcrossChunksCompletionAndHistory() {
        let fixtures = [
            "| 阶段 | 状态 |\n| :- | -: |\n| 审查 | 完成 |\n\n",
            "阶段 | 状态\n--- | ---\n审查 | 完成\n\n",
            #"| a\|b | status |"# + "\n| --- | --- |\n| value | ready |\n\n",
            "| only |\n| --- |\n| ready |\n\n"
        ]
        for source in fixtures {
            let expected = MacMarkdownTextParser.parse(source)
            guard case .table = expected.first else {
                XCTFail("Expected table: \(source)")
                continue
            }
            for width in [1, 2, 7, source.count] {
                var assembler = MacSemanticStreamAssembler(identitySeed: "table-regression")
                var nodes: [MacAssistantTextNode] = []
                var remainder = source[...]
                while !remainder.isEmpty {
                    let end = remainder.index(remainder.startIndex, offsetBy: min(width, remainder.count))
                    nodes += assembler.append(String(remainder[..<end]))
                    remainder = remainder[end...]
                    if let tail = assembler.mutableTail, tail.kind == .markdownBlock {
                        XCTAssertTrue(MacMarkdownTextParser.parse(tail.text).contains {
                            if case .table = $0 { return true }
                            return false
                        }, "A published table tail must already render structurally")
                    }
                }
                nodes += assembler.finish()
                let streamed = nodes.filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                    .flatMap { MacMarkdownTextParser.parse($0.text) }
                XCTAssertEqual(streamed, expected, "chunk width \(width): \(source)")
                XCTAssertEqual(MacMarkdownTextParser.parse(assembler.rawText), expected)
            }
        }
    }

    func testTableGrammarPreservesEscapedPipesAndRejectsMismatchedColumns() {
        XCTAssertEqual(MacMarkdownTableSyntax.cells(#"| a\|b | c |"#), [#"a\|b"#, "c"])
        XCTAssertFalse(MacMarkdownTableSyntax.isRow(#"a\|b"#))
        XCTAssertFalse(MacMarkdownTableSyntax.hasHeader("a | b", divider: "---"))
        XCTAssertFalse(MacMarkdownTableSyntax.isDivider("| ::--- | --- |"))
        XCTAssertFalse(MacMarkdownTextParser.parse("a | b\n---\nvalue | other").contains {
            if case .table = $0 { return true }
            return false
        })
        var assembler = MacSemanticStreamAssembler()
        let nodes = assembler.append("a | b\n---\nNot a table.\n\n")
        XCTAssertFalse(nodes.isEmpty, "An invalid completed divider must not buffer the rest of the response")
    }

    func testRestoredStreamDoesNotReuseCommittedPhraseIdentities() {
        let seed = "restored-stream"
        let prefix = "First complete sentence.\n\n"
        var original = MacSemanticStreamAssembler(identitySeed: seed)
        let committed = original.append(prefix)
        XCTAssertFalse(committed.isEmpty)
        var restored = MacSemanticStreamAssembler(existingText: prefix, identitySeed: seed)
        let suffix = "Second complete sentence.\n\n"
        let expected = original.append(suffix) + original.finish()
        let appended = restored.append(suffix) + restored.finish()
        XCTAssertEqual(appended, expected)
        let ids = (committed + appended).map(\.id)
        XCTAssertEqual(Set(ids).count, ids.count)
        XCTAssertEqual(restored.rawText, prefix + suffix)
    }

    func testRestoredStreamKeepsUnclosedCodeFenceState() {
        let prefix = "Example:\n\n```cpp\nint main() {\n"
        let suffix = "    return 0;\n}\n```\n"
        var original = MacSemanticStreamAssembler(identitySeed: "code-stream")
        _ = original.append(prefix)
        var restored = MacSemanticStreamAssembler(existingText: prefix, identitySeed: "code-stream")
        XCTAssertEqual(restored.mutableTail, original.mutableTail)
        XCTAssertEqual(restored.append(suffix) + restored.finish(), original.append(suffix) + original.finish())
    }

    func testSoftLineBreaksRemainExplicitBeforeInlineMarkdownRendering() {
        XCTAssertEqual(
            MacMarkdownParser.preservingSoftBreaks(in: "first line\nsecond line\n\nnext paragraph"),
            "first line  \nsecond line\n\nnext paragraph"
        )
    }
    func testStreamingMarkdownPipelineIsChunkBoundaryIndependent() {
        let source = """
        ## Release plan

        First paragraph with **emphasis** and a [reference](https://example.com).

        1. Preserve earlier output.
        2. Render structure incrementally.
           - Keep nested items.

        > Streaming Markdown remains readable.

        | Surface | Status |
        | :--- | ---: |
        | Mac | Ready |
        | Web | Online |

        ```swift
        let answer = 42
        ```

        ```mermaid
        graph TD
          A --> B
        ```
        """

        var wholeAssembler = MacSemanticStreamAssembler()
        let wholeNodes = wholeAssembler.append(source) + wholeAssembler.finish()

        var characterAssembler = MacSemanticStreamAssembler()
        var characterNodes: [MacAssistantTextNode] = []
        for character in source.map(String.init) {
            characterNodes += characterAssembler.append(character)
        }
        characterNodes += characterAssembler.finish()

        var irregularAssembler = MacSemanticStreamAssembler()
        var irregularNodes: [MacAssistantTextNode] = []
        var remainder = source[...]
        let chunkSizes = [1, 7, 3, 19, 2, 31, 5, 11]
        var chunkIndex = 0
        while !remainder.isEmpty {
            let size = min(chunkSizes[chunkIndex % chunkSizes.count], remainder.count)
            let end = remainder.index(remainder.startIndex, offsetBy: size)
            irregularNodes += irregularAssembler.append(String(remainder[..<end]))
            remainder = remainder[end...]
            chunkIndex += 1
        }
        irregularNodes += irregularAssembler.finish()

        XCTAssertEqual(characterNodes.map(\.kind), wholeNodes.map(\.kind))
        XCTAssertEqual(characterNodes.map(\.text), wholeNodes.map(\.text))
        XCTAssertEqual(irregularNodes.map(\.kind), wholeNodes.map(\.kind))
        XCTAssertEqual(irregularNodes.map(\.text), wholeNodes.map(\.text))
        XCTAssertTrue(wholeNodes.contains { $0.kind == .code(language: "swift") })
        XCTAssertTrue(wholeNodes.contains { $0.kind == .mermaid })

        let groups = MacAssistantRenderGroup.make(from: characterNodes)
        XCTAssertEqual(
            groups.map(\.plainText).joined(),
            characterNodes.map(\.text).joined()
        )
        XCTAssertFalse(groups.contains {
            if case .phrases(let phrases) = $0.content {
                return phrases.count > 1 && $0.plainText.isEmpty
            }
            return false
        })

        let renderedBlocks = wholeNodes.flatMap { node -> [MacMarkdownTextBlock] in
            guard node.kind == .markdownBlock else { return [] }
            return MacMarkdownTextParser.parse(node.text)
        }
        XCTAssertTrue(renderedBlocks.contains(.heading(level: 2, text: "Release plan")))
        XCTAssertTrue(renderedBlocks.contains(.listItem(marker: "1.", children: [.paragraph("Preserve earlier output.")])))
        XCTAssertTrue(renderedBlocks.contains(.listItem(marker: "2.", children: [
            .paragraph("Render structure incrementally."),
            .listItem(marker: "•", children: [.paragraph("Keep nested items.")])
        ])))
        XCTAssertTrue(renderedBlocks.contains(.quote([.paragraph("Streaming Markdown remains readable.")])))
        XCTAssertTrue(renderedBlocks.contains {
            if case .table(let headers, let rows) = $0 {
                return headers == ["Surface", "Status"]
                    && rows == [["Mac", "Ready"], ["Web", "Online"]]
            }
            return false
        })
    }

    func testStreamingPlainTextPreservesEveryNewlineAcrossChunkBoundaries() {
        let source = "First line.\nSecond line.\n\nThird paragraph.\n"

        var wholeAssembler = MacSemanticStreamAssembler()
        let wholeNodes = wholeAssembler.append(source) + wholeAssembler.finish()

        var characterAssembler = MacSemanticStreamAssembler()
        var characterNodes: [MacAssistantTextNode] = []
        for character in source.map(String.init) {
            characterNodes += characterAssembler.append(character)
        }
        characterNodes += characterAssembler.finish()

        XCTAssertEqual(wholeNodes.map(\.text).joined(), source)
        XCTAssertEqual(characterNodes.map(\.text).joined(), source)
        XCTAssertEqual(
            MacAssistantRenderGroup.make(from: characterNodes).map(\.plainText).joined(),
            source
        )
    }

    func testStreamingTableWaitsForDividerAndCommitsBeforeStreamFinishes() {
        var assembler = MacSemanticStreamAssembler()

        XCTAssertTrue(assembler.append("| Model | Status |\n").isEmpty)
        XCTAssertNil(
            assembler.mutableTail,
            "An unconfirmed table header must not expose raw pipe syntax."
        )
        XCTAssertTrue(assembler.append("| :--- | --").isEmpty)
        XCTAssertNil(
            assembler.mutableTail,
            "A partial divider must remain hidden instead of flashing Markdown control syntax."
        )
        XCTAssertTrue(assembler.append("-: |\n| Local | Ready |\n").isEmpty)
        XCTAssertEqual(assembler.mutableTail?.kind, .markdownBlock)
        XCTAssertEqual(
            assembler.mutableTail.map { MacMarkdownTextParser.parse($0.text) },
            [
                .table(
                    headers: ["Model", "Status"],
                    rows: [["Local", "Ready"]]
                )
            ]
        )

        let nodes = assembler.append("\nNext paragraph starts.")
        let tableNode = nodes.first { $0.kind == .markdownBlock }
        XCTAssertNotNil(tableNode)
        XCTAssertEqual(
            tableNode.map { MacMarkdownTextParser.parse($0.text) },
            [
                .table(
                    headers: ["Model", "Status"],
                    rows: [["Local", "Ready"]]
                )
            ]
        )
        XCTAssertFalse(nodes.contains { $0.kind == .phrase && $0.text.contains("| Model |") })
    }

    func testInlineMarkdownDoesNotAppearAsRawControlSyntax() throws {
        let source = "A **bold** value, an `inline code` value, and a [link](https://example.com)."
        let attributed = try AttributedString(
            markdown: source,
            options: .init(
                interpretedSyntax: .inlineOnlyPreservingWhitespace,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        )

        let visible = String(attributed.characters)
        XCTAssertEqual(visible, "A bold value, an inline code value, and a link.")
        XCTAssertFalse(visible.contains("**"))
        XCTAssertFalse(visible.contains("`"))
    }

    func testStreamingDoesNotSplitSingleAsteriskEmphasis() throws {
        var assembler = MacSemanticStreamAssembler()

        XCTAssertTrue(assembler.append("This is *a deliberately long emphasized phrase, ").isEmpty)
        let nodes = assembler.append("still intact* and now complete.") + assembler.finish()

        XCTAssertFalse(nodes.isEmpty)
        let source = nodes.map(\.text).joined()
        let attributed = try AttributedString(
            markdown: source,
            options: .init(
                interpretedSyntax: .inlineOnlyPreservingWhitespace,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        )
        XCTAssertEqual(
            String(attributed.characters),
            "This is a deliberately long emphasized phrase, still intact and now complete."
        )
    }

    func testOrdinaryParenthesisDoesNotDelayCompletedPhrase() {
        var assembler = MacSemanticStreamAssembler()

        let nodes = assembler.append("Call render(value. This sentence is complete.")

        XCTAssertFalse(nodes.isEmpty)
        XCTAssertTrue(nodes.map(\.text).joined().contains("Call render(value."))
    }

    func testParagraphsAndListsRemainInOneMarkdownDocument() {
        let source = """
        # Summary

        1. First item
        2. Second item

        Closing paragraph.
        """

        XCTAssertEqual(MacMarkdownParser.parse(source), [.markdown(source)])
    }

    func testCodeAndMermaidBlocksKeepSourceOrder() {
        let source = """
        Intro

        ```swift
        let answer = 42
        ```

        ```mermaid
        graph TD
          A --> B
        ```

        Outro
        """

        XCTAssertEqual(
            MacMarkdownParser.parse(source),
            [
                .markdown("Intro"),
                .code(language: "swift", source: "let answer = 42"),
                .mermaid("graph TD\n  A --> B"),
                .markdown("Outro")
            ]
        )
    }

    func testFencedPythonInsideBlockQuoteRendersAsCode() {
        let source = """
        > 注：可用下面的 Python 验证：
        > ```python
        > import numpy as np
        > print(np.fft.fft([1, 2, 3, 4]))
        > ```
        """

        XCTAssertEqual(
            MacMarkdownTextParser.parse(source),
            [
                .quote([
                    .paragraph("注：可用下面的 Python 验证："),
                    .code(
                        language: "python",
                        source: "import numpy as np\nprint(np.fft.fft([1, 2, 3, 4]))"
                    )
                ]),
            ]
        )
    }

    func testSoftBreakPreparationPreservesParagraphBoundaries() {
        XCTAssertEqual(
            MacMarkdownParser.preservingSoftBreaks(in: "Line one\nLine two\n\nNext paragraph"),
            "Line one  \nLine two\n\nNext paragraph"
        )
    }

    func testInlineCodeFilePathBecomesNamedFileLink() {
        let linked = MacMarkdownFileLinkifier.linkify(
            "See `/tmp/Nola/Sources/Agent.swift:42` for the reducer."
        )

        XCTAssertTrue(linked.contains("[Agent.swift:42](file:///tmp/Nola/Sources/Agent.swift)"))
        XCTAssertFalse(linked.contains("`/tmp/Nola/Sources/Agent.swift:42`"))
    }

    func testTextDocumentParserPreservesBlockStructure() {
        let source = """
        ## Release plan

        First paragraph with **emphasis**.

        1. Preserve every earlier response.
        2. Keep Markdown structure intact.
           - Support nested items.

        > Streaming output stays readable.

        ---

        Final paragraph.
        """

        XCTAssertEqual(
            MacMarkdownTextParser.parse(source),
            [
                .heading(level: 2, text: "Release plan"),
                .paragraph("First paragraph with **emphasis**."),
                .listItem(marker: "1.", children: [.paragraph("Preserve every earlier response.")]),
                .listItem(marker: "2.", children: [
                    .paragraph("Keep Markdown structure intact."),
                    .listItem(marker: "•", children: [.paragraph("Support nested items.")])
                ]),
                .quote([.paragraph("Streaming output stays readable.")]),
                .divider,
                .paragraph("Final paragraph.")
            ]
        )
    }

    func testTextDocumentParserRecognizesGFMTable() {
        let source = """
        | Model | Status |
        | :--- | ---: |
        | Local | Ready |
        | Cloud | Online |
        """

        XCTAssertEqual(
            MacMarkdownTextParser.parse(source),
            [
                .table(
                    headers: ["Model", "Status"],
                    rows: [["Local", "Ready"], ["Cloud", "Online"]]
                )
            ]
        )
    }

    func testTextDocumentParserRecognizesTaskItemsAndFootnotes() {
        let source = """
        - [x] Render stable sentences
        - [ ] Add visual regression coverage

        [^stream]: Streaming source remains canonical.
        """

        XCTAssertEqual(
            MacMarkdownTextParser.parse(source),
            [
                .listItem(marker: "☑", children: [.paragraph("Render stable sentences")]),
                .listItem(marker: "☐", children: [.paragraph("Add visual regression coverage")]),
                .footnote(label: "stream", text: "Streaming source remains canonical."),
            ]
        )
    }

    func testSentenceSegmentationIsLosslessAndStableWhenTailGrows() {
        let initial = "First sentence. 第二句已经完成。Tail"
        let initialFragments = MacMarkdownSentenceSegmenter.fragments(text: initial, identitySeed: "message")
        let completedFragments = MacMarkdownSentenceSegmenter.fragments(
            text: initial + " becomes complete!",
            identitySeed: "message"
        )

        XCTAssertEqual(initialFragments.map(\.text).joined(), initial)
        XCTAssertEqual(completedFragments.map(\.text).joined(), initial + " becomes complete!")
        XCTAssertGreaterThanOrEqual(completedFragments.count, 3)
        XCTAssertEqual(
            Array(completedFragments.prefix(2).map(\.id)),
            Array(initialFragments.prefix(2).map(\.id))
        )
    }

    func testListItemOwnsAllItsParagraphsNestedCodeAndFollowingParagraph() {
        let source = """
        3. First paragraph with **emphasis**.

           Second paragraph in the same item.

           ```swift
           let answer = 42
           print(answer)
           ```

           Paragraph after the code.

           - Nested child.

             Its second paragraph.

        4. Next item.
        """
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [
            .listItem(marker: "3.", children: [
                .paragraph("First paragraph with **emphasis**."),
                .paragraph("Second paragraph in the same item."),
                .code(language: "swift", source: "let answer = 42\nprint(answer)"),
                .paragraph("Paragraph after the code."),
                .listItem(marker: "•", children: [
                    .paragraph("Nested child."),
                    .paragraph("Its second paragraph.")
                ])
            ]),
            .listItem(marker: "4.", children: [.paragraph("Next item.")])
        ])
    }

    func testQuoteOwnsHeadingListCodeAndNestedQuote() {
        let source = """
        > ## 引用中的标题
        >
        > Intro with **strong text**.
        >
        > - First child.
        >
        >   ```python
        >   print("中文")
        >   ```
        >
        > > Nested quote.

        Outside paragraph.
        """
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [
            .quote([
                .heading(level: 2, text: "引用中的标题"),
                .paragraph("Intro with **strong text**."),
                .listItem(marker: "•", children: [
                    .paragraph("First child."),
                    .code(language: "python", source: "print(\"中文\")")
                ]),
                .quote([.paragraph("Nested quote.")])
            ]),
            .paragraph("Outside paragraph.")
        ])
    }

    func testUnicodeInlineSourceIsNotReformattedOrMathUnescaped() {
        let heading = #"中文🧪 **修正秩 $r_\gamma$** 与 `路径/论文.tex`"#
        let paragraph = #"é、👩🏽‍💻：$\ker P_t \subseteq \ker P_s$；\(a_b + c\)，[参考](https://example.com/a_b)，\*字面星号\*。"#
        let cell = #"**中文 $S_b=P_bX+Q_b$**"#
        let source = "## \(heading)\n\n\(paragraph)\n\n| 公式 | 状态 |\n| --- | --- |\n| \(cell) | ✅ |"
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [
            .heading(level: 2, text: heading),
            .paragraph(paragraph),
            .table(headers: ["公式", "状态"], rows: [[cell, "✅"]])
        ])
        XCTAssertEqual(MacMarkdownMath.segments(#"$\ker P_t \subseteq \ker P_s$"#), [
            .math(source: #"\ker P_t \subseteq \ker P_s"#, display: false,
                  original: #"$\ker P_t \subseteq \ker P_s$"#)
        ])
    }

    func testFootnoteDisplayExtensionNeverRewritesCodeSource() {
        let literal = "[^sample]: literal code, not a document footnote"
        for fence in ["```", "~~~~"] {
            XCTAssertEqual(MacMarkdownTextParser.parse("\(fence)text\n\(literal)\n\(fence)"), [
                .code(language: "text", source: literal)
            ])
        }
        XCTAssertEqual(MacMarkdownTextParser.parse("    \(literal)\n    second code line"), [
            .code(language: "", source: literal + "\nsecond code line")
        ])
        XCTAssertEqual(MacMarkdownTextParser.parse("```text\n\(literal)\n```\n\n[^visible]: Visible definition."), [
            .code(language: "text", source: literal),
            .footnote(label: "visible", text: "Visible definition.")
        ])
    }

    func testReferenceLinksRetainResolvedDestinationsWithoutReformattingInlineMath() throws {
        let source = #"""
        中文🧪 **$r_\gamma$**：[完整][ref]、[折叠][]、[快捷]；\(x_i\)。

        [ref]: https://example.com/full "Reference title"
        [折叠]: https://example.com/collapsed
        [快捷]: https://example.com/shortcut
        """#
        let expected = #"中文🧪 **$r_\gamma$**：[完整](<https://example.com/full> "Reference title")、[折叠](<https://example.com/collapsed>)、[快捷](<https://example.com/shortcut>)；\(x_i\)。"#
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [.paragraph(expected)])
        let inline = try AttributedString(markdown: expected, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))
        XCTAssertEqual(Set(inline.runs.compactMap { $0.link?.absoluteString }), [
            "https://example.com/full", "https://example.com/collapsed", "https://example.com/shortcut"
        ])
    }

    func testMultilineReferenceLabelRetainsItsQuoteContainer() {
        let source = """
        > [中文 **first**
        > second][target]

        [target]: https://example.com/reference
        """
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [
            .quote([.paragraph("[中文 **first**\nsecond](<https://example.com/reference>)")])
        ])
    }

    func testLiteralAndDirectLinksAreNotReferenceRewrites() {
        let source = #"""
        [Direct](https://example.com/a_b "title")、`[ref]`、\[ref]。

        [ref]: https://example.com/reference
        """#
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [
            .paragraph(#"[Direct](https://example.com/a_b "title")、`[ref]`、\[ref]。"#)
        ])
    }

    func testContinuationLinesLoseOnlyTheirContainerPrefix() {
        let source = """
        > - 中文 **第一行**
        >   continuation $x_i$.
        >
        >   第二段 `literal`。
        """
        XCTAssertEqual(MacMarkdownTextParser.parse(source), [
            .quote([
                .listItem(marker: "•", children: [
                    .paragraph("中文 **第一行**\ncontinuation $x_i$."),
                    .paragraph("第二段 `literal`。")
                ])
            ])
        ])
    }

    func testNestedContainersKeepOneASTAcrossStreamingChunksAndCompletion() {
        let source = """
        1. First paragraph.

           Second paragraph with **中文 $r_\\gamma$**.

           ```swift
           let value = 42
           ```

           - Nested child.

        > ## Quote title
        >
        > - Quoted item.
        >
        >   ```python
        >   print("保留嵌套")
        >   ```
        >
        > Closing quoted paragraph.

        Outside paragraph.

        """
        let expected: [MacMarkdownTextBlock] = [
            .listItem(marker: "1.", children: [
                .paragraph("First paragraph."),
                .paragraph(#"Second paragraph with **中文 $r_\gamma$**."#),
                .code(language: "swift", source: "let value = 42"),
                .listItem(marker: "•", children: [.paragraph("Nested child.")])
            ]),
            .quote([
                .heading(level: 2, text: "Quote title"),
                .listItem(marker: "•", children: [
                    .paragraph("Quoted item."),
                    .code(language: "python", source: "print(\"保留嵌套\")")
                ]),
                .paragraph("Closing quoted paragraph.")
            ]),
            .paragraph("Outside paragraph.")
        ]
        XCTAssertEqual(MacMarkdownTextParser.parse(source), expected)
        for chunkSizes in [[1], [2], [7], [1, 13, 3, 29], [source.count]] {
            var assembler = MacSemanticStreamAssembler(identitySeed: "nested-containers")
            var nodes: [MacAssistantTextNode] = []
            var remainder = source[...]
            var index = 0
            while !remainder.isEmpty {
                let size = min(chunkSizes[index % chunkSizes.count], remainder.count)
                let end = remainder.index(remainder.startIndex, offsetBy: size)
                nodes += assembler.append(String(remainder[..<end]))
                remainder = remainder[end...]
                index += 1
            }
            let streamingDocument = MacMarkdownTextParser.parse(assembler.rawText)
            nodes += assembler.finish()
            // Semantic nodes own presentation boundaries and may trim their
            // separators. The assembler's canonical source owns every byte.
            XCTAssertEqual(assembler.rawText, source, "Chunk sizes: \(chunkSizes)")
            XCTAssertEqual(streamingDocument, expected, "The last streaming frame retains container ownership.")
            XCTAssertEqual(MacMarkdownTextParser.parse(assembler.rawText), streamingDocument,
                           "Finishing a stream must not flatten or relocate nested blocks.")

            let structuralNodes = nodes.filter { $0.kind == .markdownBlock }
                .flatMap { MacMarkdownTextParser.parse($0.text) }
            XCTAssertTrue(structuralNodes.contains(expected[0]),
                          "A list container cannot be committed between its paragraphs or before its code child.")
            XCTAssertTrue(structuralNodes.contains(expected[1]),
                          "The quoted heading, list, code, and paragraph must belong to one quote node.")
            let containers = structuralNodes.filter { block in
                switch block {
                case .listItem, .quote: return true
                default: return false
                }
            }
            XCTAssertEqual(containers, Array(expected.prefix(2)),
                           "Streamed containers must exactly match the final AST, with no partial or duplicate containers.")
            XCTAssertFalse(nodes.contains { node in
                if case .code = node.kind { return true }
                return false
            }, "Nested code must not escape its list or quote into a top-level semantic code node.")
        }
    }

    func testPlainTextCopyIsDistinctFromCanonicalMarkdown() {
        let source = """
        # A **strong** heading

        Paragraph with a [reference](https://example.com) and `code`.

        - [x] Completed **item**

        > Quoted *text*.

        ```swift
        let literal = "**keep**"
        ```
        """
        XCTAssertEqual(MacMarkdownTextParser.plainText(source), """
        A strong heading

        Paragraph with a reference and code.

        ☑ Completed item

        Quoted text.

        let literal = "**keep**"
        """)
        XCTAssertEqual(MacMarkdownTextParser.parse(source).first,
                       .heading(level: 1, text: "A **strong** heading"),
                       "A plain-text projection must not rewrite the canonical Markdown source.")
        XCTAssertTrue(source.contains("[reference](https://example.com)"))
    }

    @MainActor
    func testHeadingTypographyHasReadableDocumentHierarchy() {
        XCTAssertGreaterThan(MacMarkdownTypography.headingSize(1), MacMarkdownTypography.headingSize(2))
        XCTAssertGreaterThan(MacMarkdownTypography.headingSize(2), MacMarkdownTypography.headingSize(3))
        XCTAssertGreaterThan(MacMarkdownTypography.headingSize(3), MacMarkdownTypography.bodySize)
        for level in 4...6 {
            XCTAssertGreaterThanOrEqual(MacMarkdownTypography.headingSize(level), MacMarkdownTypography.bodySize,
                                       "Minor headings should not shrink below the document body.")
        }
    }

    func testInlinePreparationRepairsIncompleteStreamingSyntaxAndAutolinksCJK() {
        let prepared = MacMarkdownInlinePreprocessor.prepare(
            "Visit https://example.com/path，then **continue",
            repairingIncompleteSyntax: true
        )

        XCTAssertTrue(prepared.contains("[https://example.com/path](https://example.com/path)，"))
        XCTAssertTrue(prepared.hasSuffix("**continue**"))
    }

    func testInlinePreparationNeverExposesIncompleteEmphasisOrLinkMarkers() throws {
        let cases: [(source: String, visible: String)] = [
            ("A *soft reveal", "A soft reveal"),
            ("A _second reveal", "A second reveal"),
            ("Read [the streaming guide", "Read the streaming guide"),
        ]

        for value in cases {
            let prepared = MacMarkdownInlinePreprocessor.prepare(
                value.source,
                repairingIncompleteSyntax: true
            )
            let attributed = try AttributedString(
                markdown: prepared,
                options: .init(
                    interpretedSyntax: .inlineOnlyPreservingWhitespace,
                    failurePolicy: .returnPartiallyParsedIfPossible
                )
            )
            XCTAssertEqual(String(attributed.characters), value.visible)
        }
    }
}
