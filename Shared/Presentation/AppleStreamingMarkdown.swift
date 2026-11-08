// SPDX-License-Identifier: Apache-2.0
// Incremental Markdown presentation shared by the native Apple clients.
import Foundation
import Combine
import SwiftUI

struct AppleStreamingMarkdownBuffer: Equatable {
    private(set) var source = ""
    private(set) var visible = ""
    private var streamingBlocks: [AppleStreamingMarkdownRenderBlock] = []
    private var mutableTailSource = ""
    /// Testable performance evidence for the most recent partition pass. A
    /// terminal lifecycle update must parse only the mutable suffix rather
    /// than rebuilding every already-rendered Markdown block.
    private(set) var lastParsedCharacterCount = 0

    var isSettled: Bool {
        source == visible
    }

    mutating func receive(
        _ nextContent: String,
        isStreaming: Bool,
        reduceMotion: Bool,
        now: Date = Date()
    ) {
        guard isStreaming else {
            finish(with: nextContent)
            return
        }
        switch AuthoritativeTextReconciliation.reconcile(authoritative: nextContent, accepted: source) {
        case .unchanged:
            return
        case .append(let delta):
            source = nextContent
            visible = nextContent
            appendStreamingMarkdown(delta)
        case .replace(let replacement):
            source = replacement
            visible = replacement
            lastParsedCharacterCount = replacement.count
            streamingBlocks = AppleStreamingMarkdownPartitioner.partition(replacement, settleTail: false)
            mutableTailSource = streamingBlocks.last?.isMutableTail == true
                ? streamingBlocks.last?.markdown ?? "" : ""
        }
    }

    mutating func finish(with finalContent: String) {
        switch AuthoritativeTextReconciliation.reconcile(authoritative: finalContent, accepted: source) {
        case .unchanged:
            break
        case .append(let delta):
            source = finalContent
            visible = finalContent
            appendStreamingMarkdown(delta)
        case .replace(let replacement):
            source = replacement
            visible = replacement
            lastParsedCharacterCount = replacement.count
            streamingBlocks = AppleStreamingMarkdownPartitioner.partition(replacement, settleTail: true)
            mutableTailSource = ""
            return
        }

        // Completion changes the lifecycle of only the mutable tail. Preserve
        // the immutable prefix and its SwiftUI identities so terminal
        // promotion cannot trigger a full Markdown-tree parse and redraw.
        guard !mutableTailSource.isEmpty else {
            lastParsedCharacterCount = 0
            return
        }
        let stableCount = streamingBlocks.last?.isMutableTail == true
            ? streamingBlocks.count - 1
            : streamingBlocks.count
        let stableBlocks = Array(streamingBlocks.prefix(stableCount))
        lastParsedCharacterCount = mutableTailSource.count
        let settledTail = AppleStreamingMarkdownPartitioner.partition(
            mutableTailSource,
            settleTail: true
        ).map { block in
            AppleStreamingMarkdownRenderBlock(
                ordinal: stableCount + block.ordinal,
                markdown: block.markdown,
                isMutableTail: false
            )
        }
        source = finalContent
        visible = finalContent
        streamingBlocks = stableBlocks + settledTail
        mutableTailSource = ""
    }

    mutating func prepareForCompletion(
        _ finalContent: String,
        reduceMotion: Bool,
        now: Date = Date()
    ) {
        finish(with: finalContent)
    }

    func renderBlocks(settleTail: Bool) -> [AppleStreamingMarkdownRenderBlock] {
        if !settleTail {
            return streamingBlocks
        }
        if mutableTailSource.isEmpty {
            return streamingBlocks
        }
        return AppleStreamingMarkdownPartitioner.partition(
            visible,
            settleTail: settleTail
        )
    }

    private mutating func appendStreamingMarkdown(_ appended: String) {
        guard !appended.isEmpty else { return }
        let stableCount = streamingBlocks.last?.isMutableTail == true
            ? streamingBlocks.count - 1
            : streamingBlocks.count
        let stableBlocks = Array(streamingBlocks.prefix(stableCount))
        let updatedTail = mutableTailSource + appended
        lastParsedCharacterCount = updatedTail.count
        let reparsedTail = AppleStreamingMarkdownPartitioner.partition(
            updatedTail,
            settleTail: false
        ).map { block in
            AppleStreamingMarkdownRenderBlock(
                ordinal: stableCount + block.ordinal,
                markdown: block.markdown,
                isMutableTail: block.isMutableTail
            )
        }
        streamingBlocks = stableBlocks + reparsedTail
        mutableTailSource = reparsedTail.last?.isMutableTail == true
            ? reparsedTail.last?.markdown ?? ""
            : ""
    }
}

nonisolated struct AppleStreamingMarkdownRenderBlock: Identifiable, Equatable, Sendable {
    let ordinal: Int
    let markdown: String
    let isMutableTail: Bool

    var id: String { "stream-markdown-\(ordinal)" }
}

enum AppleStreamingMarkdownPartitioner {
    static func partition(
        _ markdown: String,
        settleTail: Bool
    ) -> [AppleStreamingMarkdownRenderBlock] {
        guard !markdown.isEmpty else { return [] }

        let lines = markdown.split(
            separator: "\n",
            omittingEmptySubsequences: false
        ).map(String.init)
        var completed: [String] = []
        var current = ""
        var fenceMarker: Character?
        var fenceLength = 0

        for index in lines.indices {
            let line = lines[index]
            let hasFollowingLine = index < lines.index(before: lines.endIndex)
            current += line
            if hasFollowingLine {
                current += "\n"
            }

            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let marker = trimmed.first.flatMap { $0 == "`" || $0 == "~" ? $0 : nil }
            let markerLength = marker.map { marker in trimmed.prefix(while: { $0 == marker }).count } ?? 0
            if markerLength >= 3 {
                if fenceMarker == nil {
                    fenceMarker = marker
                    fenceLength = markerLength
                } else if marker == fenceMarker, markerLength >= fenceLength,
                          trimmed.dropFirst(markerLength).trimmingCharacters(in: .whitespaces).isEmpty {
                    fenceMarker = nil
                    fenceLength = 0
                }
            }

            // `split(omittingEmptySubsequences: false)` represents a single
            // trailing newline as a final empty element. That is not a blank
            // line yet: committing here would detach a table header or list
            // prefix from rows arriving in the next Core delta.
            if fenceMarker == nil,
               trimmed.isEmpty,
               hasFollowingLine,
               !current.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                completed.append(current)
                current = ""
            }
        }

        var blocks = completed.enumerated().map { index, content in
            AppleStreamingMarkdownRenderBlock(ordinal: index, markdown: content, isMutableTail: false)
        }
        if !current.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            blocks.append(AppleStreamingMarkdownRenderBlock(
                ordinal: blocks.count, markdown: current, isMutableTail: !settleTail))
        }
        return blocks
    }
}

enum AppleStreamingMarkdownTailNormalizer {
    static func stabilize(_ source: String) -> String {
        guard !source.isEmpty else { return source }
        let inlineSource = markdownOutsideFences(source)
        var suffix = ""
        if inlineSource.filter({ $0 == "`" }).count.isMultiple(of: 2) == false {
            suffix += "`"
        }
        if occurrenceCount(of: "**", in: inlineSource).isMultiple(of: 2) == false {
            suffix += "**"
        }
        if occurrenceCount(of: "__", in: inlineSource).isMultiple(of: 2) == false {
            suffix += "__"
        }
        if occurrenceCount(of: "~~", in: inlineSource).isMultiple(of: 2) == false {
            suffix += "~~"
        }
        return source + suffix
    }

    private static func markdownOutsideFences(_ source: String) -> String {
        var output: [String] = []
        var openFence: Character?
        var openFenceCount = 0

        for line in source.components(separatedBy: "\n") {
            let trimmed = line.drop(while: { $0 == " " || $0 == "\t" })
            let marker = trimmed.first.flatMap { character in
                (character == "`" || character == "~") ? character : nil
            }
            let markerCount = marker.map { character in
                trimmed.prefix(while: { $0 == character }).count
            } ?? 0
            let isFenceLine = marker != nil && markerCount >= 3

            if openFence == nil {
                if isFenceLine {
                    openFence = marker
                    openFenceCount = markerCount
                } else {
                    output.append(line)
                }
            } else if marker == openFence,
                      markerCount >= openFenceCount,
                      String(trimmed.dropFirst(markerCount))
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                        .isEmpty {
                openFence = nil
                openFenceCount = 0
            }
        }
        return output.joined(separator: "\n")
    }

    private static func occurrenceCount(of marker: String, in source: String) -> Int {
        guard !marker.isEmpty else { return 0 }
        var count = 0
        var searchRange = source.startIndex..<source.endIndex
        while let range = source.range(of: marker, range: searchRange) {
            count += 1
            searchRange = range.upperBound..<source.endIndex
        }
        return count
    }
}

struct AppleMarkdownRenderSnapshot: Equatable {
    let content: String
    let isStreaming: Bool
}

/// One owner per message view. Native update delivery accepts inputs without
/// mutating observable state during SwiftUI's render transaction.
@MainActor
final class AppleMarkdownPresentationScheduler: ObservableObject {
    @Published private(set) var snapshot: AppleMarkdownRenderSnapshot
    private(set) var buffer = AppleStreamingMarkdownBuffer()
    private var pending: AppleMarkdownRenderSnapshot?
    private var scheduled: Task<Void, Never>?
    private var generation = 0

    init(_ snapshot: AppleMarkdownRenderSnapshot) {
        self.snapshot = snapshot
        buffer.receive(snapshot.content, isStreaming: snapshot.isStreaming, reduceMotion: true)
    }

    func submit(_ next: AppleMarkdownRenderSnapshot) {
        guard next != (pending ?? snapshot) else { return }
        pending = next
        guard scheduled == nil else { return }
        let revision = generation
        scheduled = Task { @MainActor [weak self] in
            do { try await Task.sleep(nanoseconds: 16_000_000) } catch { return }
            guard let self, self.generation == revision, !Task.isCancelled else { return }
            self.scheduled = nil
            guard let latest = self.pending else { return }
            self.pending = nil
            guard latest != self.snapshot else { return }
            var transaction = Transaction(animation: nil)
            transaction.disablesAnimations = true
            withTransaction(transaction) {
                self.buffer.receive(latest.content, isStreaming: latest.isStreaming, reduceMotion: true)
                self.snapshot = latest
            }
        }
    }

    func cancel() {
        generation += 1
        scheduled?.cancel()
        scheduled = nil
        pending = nil
    }
}

// The original Mac ink reveal: a soft head travels through newly appended
// glyphs. It affects drawing only; accepted text and layout are never delayed.
nonisolated enum AppleStreamingRevealTiming {
    static let duration = 0.52
    static let completionDuration = 0.16
    static func remaining(elapsed: TimeInterval, position: Double, completionElapsed: TimeInterval? = nil,
                          precedingElapsed: TimeInterval? = nil) -> Double {
        let elapsed = if let completionElapsed {
            max(0, elapsed - completionElapsed) +
                max(0, duration - max(0, elapsed - completionElapsed)) * min(1, max(0, completionElapsed) / completionDuration)
        } else { elapsed }
        let progress = min(1, max(0, elapsed / duration))
        let localProgress = min(1, max(0, (progress * 1.34 - position) / 0.34))
        let value = pow(1 - localProgress, 3)
        guard let precedingElapsed else { return value }
        // Later network batches must not light up ahead of the previous tail.
        // They still settle within the same bounded reveal window.
        return max(value, remaining(elapsed: precedingElapsed, position: 1, completionElapsed: completionElapsed))
    }
}

/// Reveals belong to appended rendered characters, not node IDs. A mutable
/// journal node can grow many times without changing identity.
struct AppleInlineAppendRevealState {
    struct Segment {
        let id: UUID
        let range: Range<Int>
        let startedAt: Date
    }
    var text: String
    var segments: [Segment]
    var hasStreamed: Bool
    var completedAt: Date?

    init(text: String, streaming: Bool, visiblePrefixCount: Int = 0, at now: Date) {
        self.text = text
        hasStreamed = streaming
        let start = min(text.count, max(0, visiblePrefixCount))
        segments = streaming && start < text.count
            ? [Segment(id: UUID(), range: start..<text.count, startedAt: now)] : []
    }

    func updating(text newText: String, streaming: Bool, reduceMotion: Bool, at now: Date) -> Self {
        var result = self
        result.hasStreamed = hasStreamed || streaming
        result.segments.removeAll {
            now.timeIntervalSince($0.startedAt) >= AppleStreamingRevealTiming.duration
        }
        if reduceMotion {
            result.text = newText
            result.segments = []
            return result
        }
        if streaming { result.completedAt = nil }
        else if result.hasStreamed && result.completedAt == nil { result.completedAt = now }
        if let completed = result.completedAt, now.timeIntervalSince(completed) >= AppleStreamingRevealTiming.completionDuration {
            result.segments = []
        }
        guard newText != text else { return result }
        result.text = newText
        // Corrections and structure replacements become readable immediately;
        // they must not replay already-visible text.
        guard newText.hasPrefix(text) else {
            result.segments = []
            return result
        }
        if result.hasStreamed, newText.count > text.count {
            result.segments.append(.init(id: UUID(), range: text.count..<newText.count, startedAt: now))
            // Only a short animation window is retained, even under burst input.
            result.segments = Array(result.segments.suffix(128))
        }
        return result
    }
}


struct ApplePhraseRevealAttribute: TextAttribute { let id: UUID }

private struct ApplePhraseTextRenderer: TextRenderer {
    let now: Date
    let startedAt: [UUID: Date]
    let completedAt: Date?
    let reduceMotion: Bool

    func draw(layout: Text.Layout, in context: inout GraphicsContext) {
        var preceding: [UUID: Date] = [:]
        var previous: Date?
        for (id, start) in startedAt.sorted(by: { $0.value < $1.value }) {
            preceding[id] = previous
            previous = start
        }
        var totals: [UUID: Int] = [:]
        for line in layout {
            for run in line {
                if let mark = run[ApplePhraseRevealAttribute.self], startedAt[mark.id] != nil {
                    totals[mark.id, default: 0] += run.count
                }
            }
        }
        var offsets: [UUID: Int] = [:]
        for line in layout {
            let active = !reduceMotion && line.contains { run in
                guard let mark = run[ApplePhraseRevealAttribute.self], let start = startedAt[mark.id] else { return false }
                return now.timeIntervalSince(start) < AppleStreamingRevealTiming.duration
            }
            if !active { context.draw(line); continue }
            for run in line {
                if !reduceMotion, let mark = run[ApplePhraseRevealAttribute.self], let start = startedAt[mark.id] {
                    for glyph in run {
                        let offset = offsets[mark.id, default: 0]
                        offsets[mark.id] = offset + 1
                        let position = Double(offset) / Double(max(1, (totals[mark.id] ?? run.count) - 1))
                        let remaining = AppleStreamingRevealTiming.remaining(elapsed: now.timeIntervalSince(start), position: position,
                            completionElapsed: completedAt.map { now.timeIntervalSince($0) },
                            precedingElapsed: preceding[mark.id].map { now.timeIntervalSince($0) })
                        var ink = context
                        ink.opacity *= 1 - remaining
                        ink.translateBy(x: 0, y: 3.0 * remaining)
                        if remaining > 0.08 { ink.addFilter(.blur(radius: 0.65 * remaining)) }
                        ink.draw(glyph)
                    }
                } else { context.draw(run) }
            }
        }
    }
}

struct AppleStreamingInlineText: View {
    private struct Update: Hashable { let text: String; let streaming: Bool; let reduceMotion: Bool }
    let renderedText: String
    let streaming: Bool
    let reduceMotion: Bool
    let render: ([AppleInlineAppendRevealState.Segment]) -> Text
    @State private var reveal: AppleInlineAppendRevealState

    init(renderedText: String, streaming: Bool, reduceMotion: Bool, visiblePrefixCount: Int = 0,
         render: @escaping ([AppleInlineAppendRevealState.Segment]) -> Text) {
        self.renderedText = renderedText; self.streaming = streaming; self.reduceMotion = reduceMotion; self.render = render
        _reveal = State(initialValue: AppleInlineAppendRevealState(text: renderedText,
            streaming: streaming && !reduceMotion, visiblePrefixCount: visiblePrefixCount, at: Date()))
    }

    var body: some View {
        if streaming || reveal.hasStreamed { animatedText }
        else { render([]) }
    }

    private var animatedText: some View {
        let projection = reveal.updating(text: renderedText, streaming: streaming, reduceMotion: reduceMotion, at: Date())
        let starts = Dictionary(uniqueKeysWithValues: projection.segments.map { ($0.id, $0.startedAt) })
        let text = render(projection.segments)
        return TimelineView(.animation(minimumInterval: 1.0 / 60, paused: starts.isEmpty || reduceMotion)) { timeline in
            if #available(iOS 18.0, macOS 15.0, *) {
                text.textRenderer(ApplePhraseTextRenderer(now: timeline.date, startedAt: starts, completedAt: projection.completedAt, reduceMotion: reduceMotion))
            } else {
                text
            }
        }
        .task(id: Update(text: renderedText, streaming: streaming, reduceMotion: reduceMotion)) {
            reveal = projection
            guard let latest = projection.segments.map(\.startedAt).max() else { return }
            do { try await Task.sleep(for: .seconds(max(0, min(latest.addingTimeInterval(AppleStreamingRevealTiming.duration), projection.completedAt?.addingTimeInterval(AppleStreamingRevealTiming.completionDuration) ?? .distantFuture).timeIntervalSinceNow))) }
            catch { return }
            reveal.segments = []
        }
    }
}

private struct AppleStreamingRevealActiveKey: EnvironmentKey { static let defaultValue = false }
extension EnvironmentValues {
    var appleStreamingRevealActive: Bool {
        get { self[AppleStreamingRevealActiveKey.self] }
        set { self[AppleStreamingRevealActiveKey.self] = newValue }
    }
}

struct AppleStreamingAttributedText: View {
    let value: AttributedString
    @Environment(\.appleStreamingRevealActive) private var streaming
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        AppleStreamingInlineText(renderedText: String(value.characters), streaming: streaming, reduceMotion: reduceMotion) { segments in
            var text = Text("")
            var cursor = 0
            func slice(_ range: Range<Int>) -> Text {
                let lower = value.characters.index(value.startIndex, offsetBy: range.lowerBound)
                let upper = value.characters.index(value.startIndex, offsetBy: range.upperBound)
                return Text(AttributedString(value[lower..<upper]))
            }
            for segment in segments {
                if cursor < segment.range.lowerBound { text = text + slice(cursor..<segment.range.lowerBound) }
                text = text + slice(segment.range).customAttribute(ApplePhraseRevealAttribute(id: segment.id))
                cursor = segment.range.upperBound
            }
            if cursor < value.characters.count { text = text + slice(cursor..<value.characters.count) }
            return text
        }
    }
}
