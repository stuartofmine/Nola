// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Renders completed assistant Markdown with native typography and desktop code blocks.

import AppKit
import BeautifulMermaid
import SwiftUI
import SwiftMath

enum MacMarkdownTypography {
    static let bodySize: CGFloat = 16
    static let lineSpacing: CGFloat = 6
    static let blockSpacing: CGFloat = 12
    static let listItemSpacing: CGFloat = 8
    static let inlineCodeSize: CGFloat = 14.5
    static let secondaryBodySize: CGFloat = 15
    static func headingSize(_ level: Int) -> CGFloat {
        switch level {
        case 1: return 24
        case 2: return 21
        case 3: return 18
        default: return bodySize
        }
    }
}

enum MacMarkdownTableLayout {
    /// Intrinsic, bounded columns; wide tables scroll instead of compressing
    /// to the chat viewport. Widths depend on content, never on scroll offset.
    static func columnWidths(headers: [String], rows: [[String]]) -> [CGFloat] {
        headers.indices.map { index in
            let values = [headers[index]] + rows.compactMap { row in
                row.indices.contains(index) ? row[index] : nil
            }
            let extent = values.map { value in
                value.split(separator: "\n", omittingEmptySubsequences: false)
                    .map { line in
                        line.unicodeScalars.reduce(CGFloat.zero) { width, scalar in
                            width + (scalar.isASCII ? 7 : 13)
                        }
                    }.max() ?? 0
            }.max() ?? 0
            return min(320, max(extent <= 28 ? 64 : 144, extent + 20))
        }
    }
}

/// Explicit full-response selection uses one native text storage, so selection
/// can cross semantic paragraphs, tables and code blocks without changing the
/// chat renderer or its layout at completion.
struct MacSelectableResponseText: NSViewRepresentable {
    let text: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        let view = MacResponseSelectionTextView(frame: scroll.contentView.bounds)
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.autoresizingMask = [.width]
        view.textContainer?.widthTracksTextView = true
        view.isEditable = false
        view.isSelectable = true
        view.isRichText = false
        view.font = .systemFont(ofSize: 14)
        view.textContainerInset = NSSize(width: 12, height: 12)
        view.string = text
        scroll.documentView = view
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? NSTextView, view.string != text else { return }
        view.string = text
    }
}

final class MacResponseSelectionTextView: NSTextView {
    override var writablePasteboardTypes: [NSPasteboard.PasteboardType] { [.string] }

    override func writeSelection(to pasteboard: NSPasteboard, type: NSPasteboard.PasteboardType) -> Bool {
        guard type == .string else { return super.writeSelection(to: pasteboard, type: type) }
        let range = selectedRange()
        let source = string as NSString
        guard range.location != NSNotFound, range.length > 0, NSMaxRange(range) <= source.length else { return false }
        return pasteboard.setString(source.substring(with: range), forType: .string)
    }
}

/// Shares immutable inline parsing, not transcript ownership or appearance.
/// Eviction only causes reparsing; text and layout never depend on a cache hit.
@MainActor
private enum MacInlineMarkdownCache {
    enum Mode: Int {
        case literal, streaming, document
    }

    private final class Entry {
        let value: AttributedString
        init(_ value: AttributedString) { self.value = value }
    }

    private static let entries: NSCache<NSString, Entry> = {
        let cache = NSCache<NSString, Entry>()
        cache.countLimit = 256
        cache.totalCostLimit = 4 * 1_024 * 1_024
        return cache
    }()

    static func parse(_ source: String, mode: Mode) -> AttributedString {
        let bytes = source.utf8.count
        // Large individual blocks must not displace the reusable phrase cache.
        let key: NSString? = bytes <= 32_768
            ? "\(mode.rawValue):\(source)" as NSString : nil
        if let key, let entry = entries.object(forKey: key) { return entry.value }

        var markdown = MacMarkdownFileLinkifier.linkify(MacMarkdownMath.lineBreaks(source))
        switch mode {
        case .literal:
            break
        case .streaming:
            markdown = MacMarkdownInlinePreprocessor.prepare(
                markdown, repairingIncompleteSyntax: true
            )
        case .document:
            markdown = MacMarkdownInlinePreprocessor.prepare(
                MacMarkdownParser.preservingSoftBreaks(in: markdown),
                repairingIncompleteSyntax: false
            )
        }
        var value = (try? AttributedString(
            markdown: markdown,
            options: .init(
                interpretedSyntax: .inlineOnlyPreservingWhitespace,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        )) ?? AttributedString(source)
        value = MacMarkdownInlinePreprocessor.repairUnparsedStrongEmphasis(value, source: markdown)
        for run in Array(value.runs) where run.inlinePresentationIntent?.contains(.code) == true {
            value[run.range].font = .system(size: MacMarkdownTypography.inlineCodeSize, design: .monospaced)
        }
        if let key {
            entries.setObject(
                Entry(value), forKey: key,
                cost: bytes * 2 + markdown.utf8.count * 2 + value.runs.count * 256
            )
        }
        return value
    }

    private static let mathImages: NSCache<NSString, NSImage> = {
        let cache = NSCache<NSString, NSImage>()
        cache.countLimit = 128
        return cache
    }()

    static func text(_ source: String, mode: Mode) -> Text {
        content(source, mode: mode).text(reveals: [])
    }

    static func content(_ source: String, mode: Mode) -> MacInlineMarkdownContent {
        let segments = MacMarkdownMath.segments(source)
        var template = ""
        var replacements: [(String, Text)] = []
        var marker = "\u{E000}math"
        while source.contains(marker) { marker += "_" }
        for segment in segments {
            switch segment {
            case .text(let text):
                template += text
            case .math(let latex, let display, let original):
                let placeholder = "\(marker)\(replacements.count)\u{E001}"
                template += placeholder
                let key = "\(display): \(latex)" as NSString
                let image = mathImages.object(forKey: key) ?? MTMathImage(
                    latex: latex, fontSize: display ? 18 : 15, textColor: .black,
                    labelMode: display ? .display : .text, textAlignment: .left
                ).asImage().1
                guard let image else {
                    // Unsupported/incomplete LaTeX remains readable and copyable.
                    replacements.append((placeholder, Text(original).font(.system(size: 13, design: .monospaced))))
                    continue
                }
                image.accessibilityDescription = latex
                mathImages.setObject(image, forKey: key)
                replacements.append((placeholder, Text(Image(nsImage: image).renderingMode(.template)).baselineOffset(-3)))
            }
        }
        // Parse emphasis/links once across math boundaries, not as unrelated
        // fragments (which would leak ** in e.g. **bound $r$ is tight**).
        let attributed = parse(template, mode: mode)
        var start = attributed.startIndex
        var pieces: [MacInlineMarkdownContent.Piece] = []
        for (placeholder, replacement) in replacements {
            guard let range = attributed.range(of: placeholder) else { continue }
            pieces.append(.attributed(AttributedString(attributed[start..<range.lowerBound])))
            pieces.append(.attachment(replacement))
            start = range.upperBound
        }
        pieces.append(.attributed(AttributedString(attributed[start..<attributed.endIndex])))
        return MacInlineMarkdownContent(pieces: pieces)
    }
}

/// Caches immutable top-level Markdown partitions by source revision. Stable
/// assistant nodes are revisited whenever a later sibling streams, and parsing
/// every old code fence on each parent update makes long transcripts scale
/// with total rendered history. Mutable streaming sources bypass this cache so
/// their growing prefixes cannot evict useful completed blocks.
@MainActor
enum MacMarkdownBlockCache {
    private final class Entry {
        let blocks: [MacMarkdownBlock]
        init(_ blocks: [MacMarkdownBlock]) { self.blocks = blocks }
    }

    private static let entries: NSCache<NSString, Entry> = {
        let cache = NSCache<NSString, Entry>()
        cache.countLimit = 128
        cache.totalCostLimit = 4 * 1_024 * 1_024
        return cache
    }()

    static func parse(
        _ source: String,
        cacheable: Bool,
        parser: @MainActor (String) -> [MacMarkdownBlock] = MacMarkdownParser.parse
    ) -> [MacMarkdownBlock] {
        let bytes = source.utf8.count
        let key: NSString? = cacheable && bytes <= 128 * 1_024
            ? source as NSString
            : nil
        if let key, let entry = entries.object(forKey: key) {
            return entry.blocks
        }
        let blocks = parser(source)
        if let key {
            entries.setObject(Entry(blocks), forKey: key, cost: max(1, bytes * 2))
        }
        return blocks
    }

    static func removeAll() {
        entries.removeAllObjects()
    }
}

/// Caches the second parsing pass used to turn an immutable Markdown partition
/// into headings, lists, tables and paragraphs. `MacMarkdownBlockCache` avoids
/// rediscovering top-level code fences, but SwiftUI can still reevaluate every
/// completed document view when a later message changes. Keeping this cache
/// separate bounds both workloads and lets mutable streaming text opt out.
@MainActor
enum MacMarkdownTextBlockCache {
    private final class Entry {
        let blocks: [MacMarkdownTextBlock]
        init(_ blocks: [MacMarkdownTextBlock]) { self.blocks = blocks }
    }

    private static let entries: NSCache<NSString, Entry> = {
        let cache = NSCache<NSString, Entry>()
        cache.countLimit = 192
        cache.totalCostLimit = 4 * 1_024 * 1_024
        return cache
    }()

    static func parse(
        _ source: String,
        cacheable: Bool,
        parser: @MainActor (String) -> [MacMarkdownTextBlock] = MacMarkdownTextParser.parse
    ) -> [MacMarkdownTextBlock] {
        let bytes = source.utf8.count
        let key: NSString? = cacheable && bytes <= 128 * 1_024
            ? source as NSString
            : nil
        if let key, let entry = entries.object(forKey: key) {
            return entry.blocks
        }
        let blocks = parser(source)
        if let key {
            entries.setObject(Entry(blocks), forKey: key, cost: max(1, bytes * 2))
        }
        return blocks
    }

    static func removeAll() {
        entries.removeAllObjects()
    }
}

private struct MacInlineMarkdownContent {
    enum Piece {
        case attributed(AttributedString)
        case attachment(Text)
        var count: Int {
            switch self {
            case .attributed(let value): value.characters.count
            case .attachment: 1
            }
        }
    }
    let pieces: [Piece]

    var renderedText: String {
        pieces.map {
            switch $0 {
            case .attributed(let value): String(value.characters)
            case .attachment: "\u{FFFC}"
            }
        }.joined()
    }

    func text(reveals: [AppleInlineAppendRevealState.Segment]) -> Text {
        var output = Text("")
        var offset = 0
        for piece in pieces {
            switch piece {
            case .attachment(let text):
                if let reveal = reveals.first(where: { $0.range.contains(offset) }) {
                    output = output + text.customAttribute(ApplePhraseRevealAttribute(id: reveal.id))
                } else { output = output + text }
            case .attributed(let value):
                let end = offset + piece.count
                var cursor = offset
                for reveal in reveals where reveal.range.overlaps(offset..<end) {
                    let lower = max(cursor, reveal.range.lowerBound)
                    let upper = min(end, reveal.range.upperBound)
                    if cursor < lower {
                        output = output + slice(value, range: (cursor - offset)..<(lower - offset))
                    }
                    if lower < upper {
                        output = output + slice(value, range: (lower - offset)..<(upper - offset))
                            .customAttribute(ApplePhraseRevealAttribute(id: reveal.id))
                    }
                    cursor = max(cursor, upper)
                }
                if cursor < end { output = output + slice(value, range: (cursor - offset)..<(end - offset)) }
            }
            offset += piece.count
        }
        return output
    }

    private func slice(_ value: AttributedString, range: Range<Int>) -> Text {
        let lower = value.characters.index(value.startIndex, offsetBy: range.lowerBound)
        let upper = value.characters.index(value.startIndex, offsetBy: range.upperBound)
        return Text(AttributedString(value[lower..<upper]))
    }
}

private struct MacStreamingInlineText: View {
    let content: MacInlineMarkdownContent
    let streaming: Bool
    let reduceMotion: Bool
    var visiblePrefixCount = 0

    var body: some View {
        AppleStreamingInlineText(renderedText: content.renderedText, streaming: streaming,
            reduceMotion: reduceMotion, visiblePrefixCount: visiblePrefixCount) {
            content.text(reveals: $0)
        }
    }
}

struct MacMarkdownView: View {
    let markdown: String
    let isStreaming: Bool
    let animatesAppends: Bool
    var animatesInitialContent = true
    var suppressReveal = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var hasMounted = false
    @StateObject private var presentation: AppleMarkdownPresentationScheduler

    init(markdown: String, isStreaming: Bool, animatesAppends: Bool,
         animatesInitialContent: Bool = true, suppressReveal: Bool = false) {
        self.markdown = markdown
        self.isStreaming = isStreaming
        self.animatesAppends = animatesAppends
        self.animatesInitialContent = animatesInitialContent
        self.suppressReveal = suppressReveal
        _presentation = StateObject(wrappedValue: AppleMarkdownPresentationScheduler(
            .init(content: markdown, isStreaming: isStreaming)))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: MacMarkdownTypography.blockSpacing) {
            ForEach(presentation.buffer.renderBlocks(settleTail: !presentation.snapshot.isStreaming)) { block in
                MacMarkdownDocumentView(
                    source: block.isMutableTail ? AppleStreamingMarkdownTailNormalizer.stabilize(block.markdown) : block.markdown,
                    cacheable: !block.isMutableTail,
                    repairsIncompleteInlineSyntax: block.isMutableTail,
                    animatesAppends: animatesAppends,
                    animatesInitialContent: hasMounted || animatesInitialContent,
                    reduceMotion: reduceMotion || suppressReveal)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .macFileReferenceLinks()
        .background(MacMarkdownInputBridge(
            input: .init(content: markdown, isStreaming: isStreaming), scheduler: presentation)
            .frame(width: 0, height: 0))
        .onAppear { hasMounted = true }
        .onDisappear { presentation.cancel() }
        .transaction { $0.animation = nil }
    }
}

private struct MacMarkdownInputBridge: NSViewRepresentable {
    let input: AppleMarkdownRenderSnapshot
    let scheduler: AppleMarkdownPresentationScheduler
    func makeNSView(context: Context) -> NSView { NSView(frame: .zero) }
    func updateNSView(_ view: NSView, context: Context) { scheduler.submit(input) }

}

struct MacAssistantPresentationView: View {
    let nodes: [MacAssistantTextNode]
    let animateNewNodes: Bool
    let contentIsFinal: Bool
    var suppressReveal = false
    var previouslyRevealedNodeIDs: Set<UUID> = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: MacMarkdownTypography.blockSpacing) {
            ForEach(renderGroups) { group in
                switch group.content {
                case .phrases(let phrases):
                    MacMarkdownView(markdown: phrases.map(\.text).joined(),
                        isStreaming: !contentIsFinal, animatesAppends: animateNewNodes,
                        animatesInitialContent: !phrases.allSatisfy { previouslyRevealedNodeIDs.contains($0.id) },
                        suppressReveal: suppressReveal)
                case .block(let node):
                    renderedBlock(node)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .macFileReferenceLinks()
    }

    private var renderGroups: [MacAssistantRenderGroup] {
        MacAssistantRenderGroup.make(from: nodes)
    }

    private func inlineText(_ source: String) -> Text {
        MacInlineMarkdownCache.text(source, mode: .literal)
    }

    @ViewBuilder
    private func renderedBlock(_ node: MacAssistantTextNode) -> some View {
        switch node.kind {
        case .phrase:
            inlineText(node.text)
                .font(.system(size: MacMarkdownTypography.bodySize))
                .lineSpacing(MacMarkdownTypography.lineSpacing)
        case .markdownBlock:
            // A live structured node is still mutable. Rendering it with the
            // completed-document path exposes partial `**`, links and other
            // Markdown delimiters until the next transport delta arrives.
            // Keep structure native from its first visible frame and repair
            // only the renderer's transient copy; canonical text is untouched.
            MacMarkdownView(markdown: node.text, isStreaming: !contentIsFinal, animatesAppends: animateNewNodes,
                            animatesInitialContent: !previouslyRevealedNodeIDs.contains(node.id),
                            suppressReveal: suppressReveal)
        case .code(let language):
            MacCodeBlockView(language: language, source: node.text, streaming: animateNewNodes,
                             animatesInitialContent: !previouslyRevealedNodeIDs.contains(node.id),
                             suppressReveal: suppressReveal)
        case .mermaid:
            MacMermaidBlockView(source: node.text)
                .modifier(MacStreamingBlockRevealModifier(
                    shouldAnimate: animateNewNodes && !previouslyRevealedNodeIDs.contains(node.id),
                    reduceMotion: reduceMotion || suppressReveal))
        }
    }
}

/// Diagrams have no text runs to reveal. Their initial appearance can animate
/// without affecting measurement; textual blocks use append-range rendering.
private struct MacStreamingBlockRevealModifier: ViewModifier {
    let shouldAnimate: Bool
    let reduceMotion: Bool
    @State private var isVisible: Bool

    init(shouldAnimate: Bool, reduceMotion: Bool) {
        self.shouldAnimate = shouldAnimate
        self.reduceMotion = reduceMotion
        _isVisible = State(initialValue: !shouldAnimate || reduceMotion)
    }

    func body(content: Content) -> some View {
        content
            .opacity(isVisible || reduceMotion ? 1 : MacStreamingPresentationTiming.initialRevealOpacity)
            .onAppear {
                guard shouldAnimate, !reduceMotion, !isVisible else {
                    isVisible = true
                    return
                }
                withAnimation(.easeOut(duration: MacStreamingPresentationTiming.revealDuration)) {
                    isVisible = true
                }
            }
            .onChange(of: reduceMotion) { _, enabled in
                if enabled { isVisible = true }
            }
    }
}

struct MacAssistantRenderGroup: Identifiable {
    enum Content {
        case phrases([MacAssistantTextNode])
        case block(MacAssistantTextNode)
    }

    let id: UUID
    let content: Content

    var plainText: String {
        switch content {
        case .phrases(let phrases):
            phrases.map(\.text).joined()
        case .block(let node):
            node.text
        }
    }

    static func make(from nodes: [MacAssistantTextNode]) -> [MacAssistantRenderGroup] {
        var groups: [MacAssistantRenderGroup] = []
        var phrases: [MacAssistantTextNode] = []
        var phraseBlockID: UUID?

        func flushPhrases() {
            guard let id = phraseBlockID, !phrases.isEmpty else { return }
            groups.append(MacAssistantRenderGroup(id: id, content: .phrases(phrases)))
            phrases.removeAll(keepingCapacity: true)
            phraseBlockID = nil
        }

        for node in nodes {
            if node.kind == .phrase {
                if let phraseBlockID, phraseBlockID != node.blockID { flushPhrases() }
                phraseBlockID = node.blockID
                phrases.append(node)
            } else {
                flushPhrases()
                // A phrase group and the structured block immediately after
                // it can share an assembler blockID while Markdown is still
                // streaming. ForEach requires unique identities; reusing that
                // blockID makes SwiftUI display the phrase a second time in
                // place of the table/code block. The node ID is stable across
                // mutable-tail promotion and unique to the rendered block.
                groups.append(MacAssistantRenderGroup(id: node.id, content: .block(node)))
            }
        }
        flushPhrases()
        return groups
    }
}

private struct MacMarkdownDocumentView: View {
    let source: String
    let cacheable: Bool
    let repairsIncompleteInlineSyntax: Bool
    let animatesAppends: Bool
    var animatesInitialContent = true
    var reduceMotion = false
    var parsedBlocks: [MacMarkdownTextBlock]? = nil

    private var blocks: [MacMarkdownTextBlock] {
        parsedBlocks ?? MacMarkdownTextBlockCache.parse(source, cacheable: cacheable)
    }

    var body: some View {
        let snapshot = blocks
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(snapshot.enumerated()), id: \.offset) { index, block in
                blockView(block)
                    .padding(.top, index == 0 ? 0 : spacing(before: block, after: snapshot[index - 1]))
            }
        }
        .textSelection(.enabled)
        .tint(.accentColor)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func spacing(before block: MacMarkdownTextBlock, after previous: MacMarkdownTextBlock) -> CGFloat {
        func isList(_ block: MacMarkdownTextBlock) -> Bool {
            switch block {
            case .listItem: return true
            default: return false
            }
        }
        return isList(block) && isList(previous) ? MacMarkdownTypography.listItemSpacing : MacMarkdownTypography.blockSpacing
    }

    @ViewBuilder
    private func blockView(_ block: MacMarkdownTextBlock) -> some View {
        switch block {
        case .heading(let level, let text):
            inlineText(text)
                .font(.system(size: MacMarkdownTypography.headingSize(level), weight: .semibold))
                .padding(.top, level <= 2 ? 3 : 1)
                .padding(.bottom, level <= 2 ? 1 : 0)

        case .paragraph(let text):
            inlineText(text)
                .font(.system(size: MacMarkdownTypography.bodySize))
                .lineSpacing(MacMarkdownTypography.lineSpacing)
                .frame(maxWidth: MacDesignTokens.Layout.chatProseMaxWidth, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)

        case .listItem(let marker, let children):
            HStack(alignment: .top, spacing: 7) {
                Text(marker).font(.system(size: MacMarkdownTypography.bodySize))
                    .foregroundStyle(.secondary).frame(minWidth: 22, alignment: .trailing)
                nested(children)
            }

        case .footnote(let label, let text):
            listRow(marker: "[\(label)]", text: text, depth: 0)
                .font(.system(size: 12.5))
                .foregroundStyle(.secondary)

        case .quote(let children):
            nested(children)
                .font(.system(size: MacMarkdownTypography.secondaryBodySize))
                .lineSpacing(MacMarkdownTypography.lineSpacing)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .background(
                    Color.primary.opacity(0.028),
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                )
                .overlay(alignment: .leading) {
                    Capsule()
                        .fill(Color.accentColor.opacity(0.62))
                        .frame(width: 2.5)
                        .padding(.vertical, 7)
                }
                .padding(.vertical, 2)

        case .code(let language, let source):
            MacCodeBlockView(language: language, source: source, streaming: animatesAppends,
                             animatesInitialContent: animatesInitialContent, suppressReveal: reduceMotion)

        case .mermaid(let source):
            if repairsIncompleteInlineSyntax {
                MacCodeBlockView(language: "mermaid", source: source, streaming: animatesAppends,
                                 animatesInitialContent: animatesInitialContent, suppressReveal: reduceMotion)
            } else {
                MacMermaidBlockView(source: source)
            }

        case .divider:
            Divider()
                .opacity(0.55)
                .padding(.vertical, 3)

        case .table(let headers, let rows):
            tableView(headers: headers, rows: rows)
        }
    }

    private func inlineText(_ source: String) -> some View {
        let content = MacInlineMarkdownCache.content(
            source,
            mode: repairsIncompleteInlineSyntax ? .streaming : .document
        )
        return MacStreamingInlineText(content: content, streaming: animatesAppends,
                                      reduceMotion: reduceMotion,
                                      visiblePrefixCount: animatesInitialContent ? 0 : content.renderedText.count)
    }

    private func nested(_ children: [MacMarkdownTextBlock]) -> AnyView {
        AnyView(MacMarkdownDocumentView(source: "", cacheable: cacheable,
            repairsIncompleteInlineSyntax: repairsIncompleteInlineSyntax,
            animatesAppends: animatesAppends, animatesInitialContent: animatesInitialContent,
            reduceMotion: reduceMotion, parsedBlocks: children))
    }

    private func listRow(marker: String, text: String, depth: Int) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 7) {
            Text(marker)
                .font(.system(size: 13, weight: .medium, design: .rounded))
                .foregroundStyle(Color.accentColor.opacity(0.82))
                .frame(width: 22, alignment: .trailing)

            inlineText(text)
                .font(.system(size: MacMarkdownTypography.bodySize))
                .lineSpacing(MacMarkdownTypography.lineSpacing)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.leading, CGFloat(depth) * 16)
    }

    private func tableView(headers: [String], rows: [[String]]) -> some View {
        let widths = MacMarkdownTableLayout.columnWidths(headers: headers, rows: rows)
        return ScrollView(.horizontal, showsIndicators: true) {
            Grid(alignment: .topLeading, horizontalSpacing: 0, verticalSpacing: 0) {
                GridRow {
                    ForEach(Array(headers.enumerated()), id: \.offset) { index, header in
                        tableCell(header, width: widths[index], isHeader: true)
                    }
                }

                ForEach(Array(rows.enumerated()), id: \.offset) { rowIndex, row in
                    GridRow {
                        ForEach(Array(normalized(row, count: headers.count).enumerated()), id: \.offset) { index, cell in
                            tableCell(
                                cell,
                                width: widths[index],
                                isHeader: false,
                                isAlternatingRow: rowIndex.isMultiple(of: 2)
                            )
                        }
                    }
                }
            }
            // The viewport is bounded, not the table. Forcing the Grid to the
            // viewport width made horizontal scrolling inert and crushed long
            // evidence/code columns into unreadable vertical strips.
            .fixedSize(horizontal: true, vertical: true)
            .frame(width: widths.reduce(0, +), alignment: .topLeading)
            .background(Color.primary.opacity(0.012))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(Color.primary.opacity(0.1), lineWidth: 1)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func tableCell(
        _ source: String,
        width: CGFloat,
        isHeader: Bool,
        isAlternatingRow: Bool = false
    ) -> some View {
        inlineText(source)
            .font(.system(size: MacMarkdownTypography.secondaryBodySize, weight: isHeader ? .semibold : .regular))
            .foregroundStyle(.primary)
            .lineLimit(nil)
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(width: width, alignment: .topLeading)
            .frame(minHeight: 36, alignment: .topLeading)
            .gridCellAnchor(.topLeading)
            .background {
                if isHeader {
                    Color.primary.opacity(0.055)
                } else if isAlternatingRow {
                    Color.primary.opacity(0.024)
                } else {
                    Color.clear
                }
            }
            .overlay(alignment: .bottom) {
                Rectangle()
                    .fill(Color.primary.opacity(0.055))
                    .frame(height: 0.5)
            }
    }

    private func normalized(_ row: [String], count: Int) -> [String] {
        if row.count >= count {
            return Array(row.prefix(count))
        }
        return row + Array(repeating: "", count: count - row.count)
    }
}

enum MacMarkdownFileLinkifier {
    static func linkify(_ source: String) -> String {
        let pattern = #"`((?:/|~/)[^`\n]+)`"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else {
            return source
        }
        var result = source
        let matches = regex.matches(
            in: source,
            range: NSRange(source.startIndex..<source.endIndex, in: source)
        )
        for match in matches.reversed() {
            guard match.numberOfRanges == 2,
                  let wholeRange = Range(match.range(at: 0), in: result),
                  let pathRange = Range(match.range(at: 1), in: result) else {
                continue
            }
            let rawPath = String(result[pathRange])
            let expandedPath = NSString(string: rawPath).expandingTildeInPath
            guard let reference = MacCodeReferenceParser.parse(
                URL(fileURLWithPath: expandedPath)
            ) else {
                continue
            }
            let isExistingPath = FileManager.default.fileExists(atPath: reference.url.path)
            guard isExistingPath
                    || !reference.url.pathExtension.isEmpty
                    || reference.line != nil else {
                continue
            }
            var label = reference.url.lastPathComponent
            if let target = reference.targetDescription {
                label += ":\(target.dropFirst())"
            }
            label = label
                .replacingOccurrences(of: "[", with: "\\[")
                .replacingOccurrences(of: "]", with: "\\]")
            result.replaceSubrange(
                wholeRange,
                with: "[\(label)](\(reference.url.absoluteString))"
            )
        }
        return result
    }
}

nonisolated indirect enum MacMarkdownTextBlock: Equatable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case listItem(marker: String, children: [MacMarkdownTextBlock])
    case footnote(label: String, text: String)
    case quote([MacMarkdownTextBlock])
    case code(language: String, source: String)
    case mermaid(String)
    case divider
    case table(headers: [String], rows: [[String]])
}

enum MacMarkdownBlock: Equatable {
    case markdown(String)
    case code(language: String, source: String)
    case mermaid(String)
}

enum MacMarkdownParser {
    static func parse(_ source: String) -> [MacMarkdownBlock] {
        var blocks: [MacMarkdownBlock] = []
        var markdownLines: [String] = []
        var codeLines: [String] = []
        var language = ""
        var fenceMarker: Character = "`"
        var fenceCount = 3
        var inCode = false

        func flushMarkdown() {
            let value = markdownLines.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !value.isEmpty { blocks.append(.markdown(value)) }
            markdownLines.removeAll(keepingCapacity: true)
        }

        func flushCode() {
            let value = codeLines.joined(separator: "\n")
            if language.lowercased() == "mermaid" {
                blocks.append(.mermaid(value))
            } else {
                blocks.append(.code(language: language, source: value))
            }
            codeLines.removeAll(keepingCapacity: true)
            language = ""
        }

        let normalizedSource = source
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")

        for line in normalizedSource.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
            if inCode {
                if isClosingFence(line, marker: fenceMarker, minimumCount: fenceCount) {
                    flushCode()
                    inCode = false
                } else {
                    codeLines.append(line)
                }
            } else if let opening = openingFence(line) {
                flushMarkdown()
                language = opening.language
                fenceMarker = opening.marker
                fenceCount = opening.count
                inCode = true
            } else {
                markdownLines.append(line)
            }
        }

        if inCode {
            flushCode()
        } else {
            flushMarkdown()
        }
        return blocks.isEmpty ? [.markdown(source)] : blocks
    }

    private static func openingFence(
        _ line: String
    ) -> (marker: Character, count: Int, language: String)? {
        let indentation = line.prefix { $0 == " " }
        guard indentation.count <= 3 else { return nil }
        let content = line.dropFirst(indentation.count)
        guard let marker = content.first, marker == "`" || marker == "~" else { return nil }
        let run = content.prefix { $0 == marker }
        guard run.count >= 3 else { return nil }
        return (
            marker,
            run.count,
            content.dropFirst(run.count).trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }

    private static func isClosingFence(
        _ line: String,
        marker: Character,
        minimumCount: Int
    ) -> Bool {
        let indentation = line.prefix { $0 == " " }
        guard indentation.count <= 3 else { return false }
        let content = line.dropFirst(indentation.count)
        let run = content.prefix { $0 == marker }
        guard run.count >= minimumCount else { return false }
        return content.dropFirst(run.count).allSatisfy(\.isWhitespace)
    }

    static func preservingSoftBreaks(in source: String) -> String {
        let lines = source.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard lines.count > 1 else { return source }
        return lines.enumerated().map { index, line in
            guard index < lines.count - 1 else { return line }
            if line.trimmingCharacters(in: .whitespaces).isEmpty { return "\n" }
            if lines[index + 1].trimmingCharacters(in: .whitespaces).isEmpty { return line + "\n" }
            if line.hasSuffix("  ") || line.hasSuffix("\\") { return line + "\n" }
            return line + "  \n"
        }.joined()
    }
}

private struct MacMermaidBlockView: View {
    let source: String
    @Environment(\.colorScheme) private var colorScheme
    @State private var parseError: Error?
    @State private var diagramBounds: CGRect = .zero

    var body: some View {
        Group {
            if parseError == nil {
                ScrollView([.horizontal, .vertical], showsIndicators: true) {
                    MermaidDiagramView(
                        source: source,
                        theme: mermaidTheme,
                        parseError: $parseError,
                        diagramBounds: $diagramBounds
                    )
                    .frame(width: diagramSize.width, height: diagramSize.height)
                    .padding(14)
                }
                .frame(height: containerHeight)
            } else {
                MacCodeBlockView(language: "mermaid", source: source)
            }
        }
        .background(
            Color.primary.opacity(0.035),
            in: RoundedRectangle(cornerRadius: 10, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(Color.primary.opacity(0.09), lineWidth: 0.5)
        }
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .onChange(of: source) { _, _ in
            parseError = nil
            diagramBounds = .zero
        }
        .onChange(of: colorScheme) { _, _ in
            diagramBounds = .zero
        }
    }

    private var mermaidTheme: DiagramTheme {
        let theme = colorScheme == .dark ? DiagramTheme.zincDark : DiagramTheme.zincLight
        return theme.withTransparent(true)
    }

    private var diagramSize: CGSize {
        guard diagramBounds.width > 0, diagramBounds.height > 0 else {
            return CGSize(width: 360, height: 190)
        }
        return CGSize(
            width: max(360, ceil(diagramBounds.width)),
            height: max(190, ceil(diagramBounds.height))
        )
    }

    private var containerHeight: CGFloat {
        min(560, max(190, diagramSize.height + 28))
    }
}

/// One rendered value per mounted block. Unrelated layout/copy-button updates
/// must not rerun syntax highlighting; streamed source or appearance changes
/// replace the value, rather than accumulating historical token prefixes.
@MainActor
final class MacInlineCodeHighlightCache {
    private struct Key: Equatable {
        let source: String
        let fileExtension: String
        let dark: Bool
    }
    private var entry: (key: Key, text: AttributedString)?
    private(set) var renderCount = 0

    func text(source: String, fileExtension: String, dark: Bool) -> AttributedString {
        let key = Key(source: source, fileExtension: fileExtension, dark: dark)
        if let entry, entry.key == key { return entry.text }
        let normalized = source.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        let document = MacCodeReferenceDocument(
            url: URL(fileURLWithPath: "/inline.\(fileExtension)"),
            lines: normalized.components(separatedBy: "\n"),
            byteCount: normalized.utf8.count, encodingName: "UTF-8")
        let text = AttributedString(MacCodeSyntaxHighlighter.attributedText(
            for: document, targetLine: nil,
            appearance: NSAppearance(named: dark ? .darkAqua : .aqua)))
        entry = (key, text)
        renderCount += 1
        return text
    }
}

private struct MacCodeBlockView: View {
    let language: String
    let source: String
    var streaming = false
    var animatesInitialContent = true
    var suppressReveal = false
    @State private var copied = false
    @State private var copyFailed = false
    @State private var feedbackID = UUID()
    @State private var highlightCache = MacInlineCodeHighlightCache()
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var fileExtension: String {
        switch language.lowercased() {
        case "python", "py": return "py"
        case "javascript", "js": return "js"
        case "typescript", "ts": return "ts"
        case "jsx": return "jsx"
        case "tsx": return "tsx"
        case "shell", "bash", "zsh", "sh": return "sh"
        case "swift": return "swift"
        case "ruby", "rb": return "rb"
        case "yaml", "yml": return "yaml"
        case "toml": return "toml"
        case "json": return "json"
        case "rust", "rs": return "rs"
        case "c++", "cpp": return "cpp"
        default: return language.lowercased().isEmpty ? "txt" : language.lowercased()
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Text(language.isEmpty ? "CODE" : language.uppercased())
                    .font(.system(size: 9, weight: .bold, design: .rounded))
                    .foregroundStyle(.secondary)
                    .tracking(0.7)
                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    guard NSPasteboard.general.setString(source, forType: .string) else { copied = false; feedbackID = UUID(); copyFailed = true; return }
                    copied = true
                    feedbackID = UUID()
                    let feedback = feedbackID
                    Task { @MainActor in
                        do { try await Task.sleep(for: .seconds(1.2)) } catch { return }
                        guard feedbackID == feedback else { return }
                        copied = false
                    }
                } label: {
                    Label(
                        copied ? String(localized: "Copied") : String(localized: "Copy Code"),
                        systemImage: copied ? "checkmark" : "doc.on.doc"
                    )
                        .font(.system(size: 9.5, weight: .medium))
                }
                .buttonStyle(.plain)
                .foregroundStyle(copied ? Color.green : Color.secondary)
            }
            .padding(.horizontal, 11)
            .frame(height: 32)
            .background(Color.primary.opacity(0.045))

            Divider().opacity(0.45)

            ScrollView(.horizontal, showsIndicators: true) {
                let attributed = highlightCache.text(source: source, fileExtension: fileExtension,
                                                     dark: colorScheme == .dark)
                MacStreamingInlineText(content: MacInlineMarkdownContent(pieces: [.attributed(attributed)]),
                                       streaming: streaming, reduceMotion: reduceMotion || suppressReveal,
                                       visiblePrefixCount: animatesInitialContent ? 0 : attributed.characters.count)
                .textSelection(.enabled)
                .fixedSize(horizontal: true, vertical: true)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .onChange(of: source) { _, _ in feedbackID = UUID(); copied = false }
        .alert(String(localized: "Nola could not update the clipboard."), isPresented: $copyFailed) {
            Button("OK", role: .cancel) {}
        }
        .background(Color(nsColor: .textBackgroundColor).opacity(0.72), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 9, style: .continuous)
                .stroke(Color.primary.opacity(0.09), lineWidth: 0.5)
        }
    }
}
