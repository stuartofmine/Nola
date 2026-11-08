// SPDX-License-Identifier: Apache-2.0
import AppKit
import Combine
import SwiftUI
import XCTest
import Darwin
import ObjectiveC.runtime
@testable import Nola

/// Scheduling lateness is not itself a frame-time measurement. Record main
/// run-loop active spans separately, excluding time spent waiting for events.
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
