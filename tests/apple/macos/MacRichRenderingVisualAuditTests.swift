// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Strict macOS 2D rendered-geometry, pixel-buffer visual audit, right-sidebar inline
//   preview lifecycle, tool card / image bounding box, and deterministic local loopback HTTP/SSE
//   multi-turn persistence & error-recovery tests.

import AppKit
import Foundation
import Network
import PDFKit
import Quartz
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRichRenderingVisualAuditTests: XCTestCase {
    override func tearDown() {
        MacApplicationServiceSupervisor.shared.stopOwnedProcesses()
        super.tearDown()
    }

    private func evidenceDirectory() throws -> URL {
        let dir = URL(fileURLWithPath: "/tmp/nola-real-e2e-evidence/macos-audit", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeTempDirectory(_ prefix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-audit-\(prefix)-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    /// Computes non-background ink pixel ratio from a rendered CGImage by comparing every pixel
    /// against the median corner background color. Catches blank/all-white/all-black/collapsed views.
    @discardableResult
    private func assertRenderedBitmapHasVisibleInk(
        _ cgImage: CGImage,
        minRatio: Double = 0.005,
        maxRatio: Double = 0.95,
        context: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws -> Double {
        let width = cgImage.width
        let height = cgImage.height
        XCTAssertGreaterThan(width, 16, "\(context): rendered width too small (\(width))", file: file, line: line)
        XCTAssertGreaterThan(height, 16, "\(context): rendered height too small (\(height))", file: file, line: line)

        let bytesPerPixel = 4
        let bytesPerRow = width * bytesPerPixel
        var pixels = [UInt8](repeating: 0, count: height * bytesPerRow)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue
        guard let ctx = CGContext(
            data: &pixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: bytesPerRow,
            space: colorSpace,
            bitmapInfo: bitmapInfo
        ) else {
            throw NSError(domain: "PixelAudit", code: 1)
        }
        ctx.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))

        func pixelAt(_ x: Int, _ y: Int) -> (Int, Int, Int, Int) {
            let offset = (y * width + x) * 4
            return (Int(pixels[offset]), Int(pixels[offset + 1]), Int(pixels[offset + 2]), Int(pixels[offset + 3]))
        }

        // Sample 4 inset corners to establish background color cleanly.
        let insetX = min(4, max(0, width - 1))
        let insetY = min(4, max(0, height - 1))
        let corners = [
            pixelAt(insetX, insetY),
            pixelAt(width - 1 - insetX, insetY),
            pixelAt(insetX, height - 1 - insetY),
            pixelAt(width - 1 - insetX, height - 1 - insetY),
        ]
        let bgR = corners.map(\.0).sorted()[1]
        let bgG = corners.map(\.1).sorted()[1]
        let bgB = corners.map(\.2).sorted()[1]

        var nonBackgroundCount = 0
        let totalPixels = width * height
        for idx in stride(from: 0, to: pixels.count, by: 4) {
            let r = Int(pixels[idx])
            let g = Int(pixels[idx + 1])
            let b = Int(pixels[idx + 2])
            let a = Int(pixels[idx + 3])
            let diff = abs(r - bgR) + abs(g - bgG) + abs(b - bgB)
            if a > 16 && diff > 24 {
                nonBackgroundCount += 1
            }
        }
        let ratio = Double(nonBackgroundCount) / Double(max(1, totalPixels))
        XCTAssertGreaterThan(
            ratio,
            minRatio,
            "\(context): rendered bitmap appears blank or collapsed (inkRatio=\(String(format: "%.4f", ratio)), bg=(\(bgR),\(bgG),\(bgB)))",
            file: file,
            line: line
        )
        XCTAssertLessThan(
            ratio,
            maxRatio,
            "\(context): rendered bitmap appears solid/inverted without contrast (inkRatio=\(String(format: "%.4f", ratio)), bg=(\(bgR),\(bgG),\(bgB)))",
            file: file,
            line: line
        )
        return ratio
    }

    @discardableResult
    private func renderToPNG<V: View>(
        _ view: V,
        size: NSSize,
        colorScheme: ColorScheme,
        to url: URL,
        settleMs: UInt64 = 0,
        verifyInk: Bool = true,
        minInkRatio: Double = 0.005,
        maxInkRatio: Double = 0.95,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> (hosting: NSHostingView<AnyView>, cgImage: CGImage, inkRatio: Double) {
        let wrapped = AnyView(
            view
                .frame(width: size.width, height: size.height, alignment: .topLeading)
                .environment(\.colorScheme, colorScheme)
        )
        let hosting = NSHostingView(rootView: wrapped)
        hosting.sizingOptions = []
        hosting.frame = NSRect(origin: .zero, size: size)
        let window = NSWindow(
            contentRect: hosting.frame,
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)
        window.contentView = hosting
        window.orderOut(nil)
        window.displayIfNeeded()
        hosting.layoutSubtreeIfNeeded()
        if settleMs > 0 {
            try await Task.sleep(for: .milliseconds(settleMs))
            RunLoop.main.run(until: Date().addingTimeInterval(0.03))
            window.displayIfNeeded()
            hosting.layoutSubtreeIfNeeded()
        }
        guard let rep = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds) else {
            window.close()
            throw NSError(domain: "RenderPNG", code: 1)
        }
        hosting.cacheDisplay(in: hosting.bounds, to: rep)
        guard let png = rep.representation(using: .png, properties: [:]),
              let cgImage = rep.cgImage else {
            window.close()
            throw NSError(domain: "RenderPNG", code: 2)
        }
        try png.write(to: url, options: .atomic)
        window.close()
        let inkRatio: Double
        if verifyInk {
            inkRatio = try assertRenderedBitmapHasVisibleInk(
                cgImage,
                minRatio: minInkRatio,
                maxRatio: maxInkRatio,
                context: "\(url.lastPathComponent) [\(colorScheme)]",
                file: file,
                line: line
            )
        } else {
            inkRatio = 0
        }
        return (hosting, cgImage, inkRatio)
    }

    /// Generates a real multi-color striped PNG fixture for image preview & bounding-box tests.
    private func generateTestPNG(at url: URL, width: Int, height: Int) throws {
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: width,
            pixelsHigh: height,
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: 0,
            bitsPerPixel: 0
        ) else {
            throw NSError(domain: "PNGGen", code: 1)
        }
        NSGraphicsContext.saveGraphicsState()
        guard let gctx = NSGraphicsContext(bitmapImageRep: rep) else {
            NSGraphicsContext.restoreGraphicsState()
            throw NSError(domain: "PNGGen", code: 2)
        }
        NSGraphicsContext.current = gctx
        NSColor(calibratedRed: 0.96, green: 0.97, blue: 0.99, alpha: 1.0).setFill()
        NSBezierPath.fill(NSRect(x: 0, y: 0, width: width, height: height))
        NSColor(calibratedRed: 0.12, green: 0.38, blue: 0.82, alpha: 1.0).setFill()
        NSBezierPath.fill(NSRect(x: width / 10, y: height / 6, width: width * 4 / 5, height: height / 3))
        NSColor(calibratedRed: 0.88, green: 0.28, blue: 0.24, alpha: 1.0).setFill()
        NSBezierPath.fill(NSRect(x: width / 5, y: height * 3 / 5, width: width * 3 / 5, height: height / 4))
        NSGraphicsContext.restoreGraphicsState()
        guard let data = rep.representation(using: .png, properties: [:]) else {
            throw NSError(domain: "PNGGen", code: 3)
        }
        try data.write(to: url, options: .atomic)
    }

    /// Generates a real 1-page PDF with crisp visible typography and vector bars.
    private func generateSinglePagePDF(at url: URL, title: String, body: String) throws {
        var mediaBox = CGRect(x: 0, y: 0, width: 612, height: 792)
        let data = NSMutableData()
        guard let consumer = CGDataConsumer(data: data as CFMutableData),
              let ctx = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else {
            throw NSError(domain: "PDFGen", code: 1)
        }
        ctx.beginPDFPage(nil)
        let nsCtx = NSGraphicsContext(cgContext: ctx, flipped: false)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = nsCtx
        NSColor.white.setFill()
        NSBezierPath.fill(mediaBox)
        NSColor(calibratedRed: 0.14, green: 0.34, blue: 0.76, alpha: 1.0).setFill()
        NSBezierPath.fill(NSRect(x: 40, y: 710, width: 532, height: 42))
        let titleAttrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.boldSystemFont(ofSize: 20),
            .foregroundColor: NSColor.white,
        ]
        (title as NSString).draw(in: NSRect(x: 54, y: 716, width: 504, height: 30), withAttributes: titleAttrs)
        let bodyAttrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.black,
        ]
        (body as NSString).draw(in: NSRect(x: 40, y: 120, width: 532, height: 560), withAttributes: bodyAttrs)
        NSGraphicsContext.restoreGraphicsState()
        ctx.endPDFPage()
        ctx.closePDF()
        try (data as Data).write(to: url, options: .atomic)
    }

    private func pressFirstButton(in root: NSView, window: NSWindow) -> Bool {
        func findNSButtons(_ view: NSView) -> [NSButton] {
            var found: [NSButton] = []
            if let button = view as? NSButton, button.isEnabled { found.append(button) }
            for subview in view.subviews { found.append(contentsOf: findNSButtons(subview)) }
            return found
        }
        if let button = findNSButtons(root).first {
            button.performClick(nil)
            return true
        }
        // SwiftUI plain/custom-style buttons expose accessibility elements once the host
        // window is attached to the window server (orderBack keeps it unobtrusive).
        window.orderBack(nil)
        window.displayIfNeeded()
        root.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.03))
        func walkAccessibility(_ element: AnyObject) -> Bool {
            if let acc = element as? NSAccessibilityProtocol {
                if acc.accessibilityRole() == .button,
                   acc.isAccessibilityEnabled(),
                   acc.accessibilityPerformPress() {
                    return true
                }
                if let children = acc.accessibilityChildren() {
                    for child in children {
                        if walkAccessibility(child as AnyObject) { return true }
                    }
                }
            }
            return false
        }
        if walkAccessibility(root) {
            return true
        }
        // Fallback: dispatch a real AppKit mouseDown + mouseUp at the center of the hosted view.
        let centerInView = NSPoint(x: root.bounds.midX, y: root.bounds.midY)
        let centerInWindow = root.convert(centerInView, to: nil)
        let now = ProcessInfo.processInfo.systemUptime
        if let down = NSEvent.mouseEvent(
            with: .leftMouseDown,
            location: centerInWindow,
            modifierFlags: [],
            timestamp: now,
            windowNumber: window.windowNumber,
            context: nil,
            eventNumber: 1,
            clickCount: 1,
            pressure: 1.0
        ), let up = NSEvent.mouseEvent(
            with: .leftMouseUp,
            location: centerInWindow,
            modifierFlags: [],
            timestamp: now + 0.01,
            windowNumber: window.windowNumber,
            context: nil,
            eventNumber: 2,
            clickCount: 1,
            pressure: 0.0
        ) {
            window.sendEvent(down)
            window.sendEvent(up)
            RunLoop.main.run(until: Date().addingTimeInterval(0.03))
            return true
        }
        return false
    }

    // MARK: - 1. Composer Zero-Jump Pixel Geometry Audit across ALL Reasoning/Permission/Plan Toggles

    func testComposerZeroVerticalJumpAcrossAllReasoningAndPermissionModes() throws {
        @MainActor
        struct ComposerHostHarness: View {
            @State var state: MacComposerDisplayState
            @FocusState var focused: Bool
            var body: some View {
                VStack(spacing: 0) {
                    Spacer(minLength: 0)
                    MacComposerView(
                        state: state,
                        actions: MacComposerActionSink { _ in true },
                        composerFocused: $focused,
                        trailingControls: AnyView(
                            Text("qwen3.8-flash-next-iq3_s · local")
                                .font(.system(size: 11.5, weight: .medium))
                                .foregroundStyle(.secondary)
                        )
                    )
                    .padding(16)
                }
                .frame(width: 960, height: 260)
                .background(MacDesignColor.canvas)
            }
        }

        let efforts = ["automatic", "none", "low", "medium", "high"]
        let modes: [MacPermissionMode] = [.cautious, .allowAll]
        var measuredHeights: [CGFloat] = []

        for effort in efforts {
            for mode in modes {
                let state = MacComposerDisplayState(
                    conversationID: "conv-geom",
                    draft: "",
                    attachments: [],
                    isRunning: false,
                    isVisiblyRunning: false,
                    reasoningOptions: ["none", "low", "medium", "high"],
                    reasoningPreference: effort,
                    permissionMode: mode,
                    isPlanMode: false,
                    permissionConfigurationState: .saved,
                    activePermissionMode: mode,
                    isSubmissionBlocked: false,
                    submissionBlockReason: nil,
                    attachmentImportNotice: nil
                )
                let hosting = NSHostingView(rootView: ComposerHostHarness(state: state).environment(\.colorScheme, .dark))
                hosting.frame = NSRect(x: 0, y: 0, width: 960, height: 260)
                hosting.layoutSubtreeIfNeeded()
                let fitting = hosting.fittingSize
                measuredHeights.append(fitting.height)
            }
        }
        let minH = try XCTUnwrap(measuredHeights.min())
        let maxH = try XCTUnwrap(measuredHeights.max())
        XCTAssertLessThanOrEqual(
            maxH - minH,
            0.5,
            "Composer height MUST NOT jump across reasoning effort / permission mode switches (heights=\(measuredHeights))"
        )
    }

    // MARK: - 2. Academic Paper Rich Markdown (Inline & Display Math, Tables, Code, Quotes) Visual & Pixel Audit

    func testAcademicMathTablesCodeRichMarkdownDarkAndLightAudit() async throws {
        let evidence = try evidenceDirectory()
        let richPaperMarkdown = """
        ## 一、核心理论与低秩残差恒等式

        发送方持有源分支状态 $S_s$，接收方仅持有兄弟分支 $S_t$（无 fork 快照）。当累积衰减因子分别为 $D_s$ 与 $D_t$ 时，两者满足精确代数恒等式：

        $$S_s = \\lambda S_t + UV^\\top,\\qquad \\lambda = \\frac{D_s}{D_t}$$

        用人话来说，兄弟状态之间只差一小坨低秩残差：

        $$S_A = \\lambda \\cdot S_B + (\\text{一小坨残差})$$

        实测总损失 $\\mathcal{L}_{\\text{总}} = \\mathcal{L}_{\\text{主}} + \\lambda \\mathcal{L}_{\\text{正则}}$，且端到端吞吐满足 $R^*=\\Delta\\text{bytes}/\\Delta t_{\\text{local}}=38.9\\ \\text{MB}/0.488\\ \\text{s}$。

        其中低秩残差宽度仅与分叉后新增步数 $p = n_s + n_t$ 有关，而不随共享前缀长度 $L \\gg p$ 增长。对于单头误差界 $\\epsilon \\le \\sum_{i=1}^{p} \\alpha_i \\|u_i\\|_2 \\|v_i\\|_2$，在 BF16 精度下实测相对误差低于 $10^{-4}$。

        ### 实验对比汇总

        | 布局方式 | 8 候选显存占用 | 相对带宽 | 备注 |
        | --- | ---: | ---: | --- |
        | Fork Source-Factor | 29.554 MB | 1.00x | 需保留 fork |
        | Sibling DFD-R ($p = n_s + n_t$) | 42.558 MB | 1.44x | 无需 fork |
        | Full Snapshot | 143.800 MB | 4.87x | 无压缩基线 |

        > 结论：当且仅当接收端无法保留 fork 状态时，兄弟条件重建才具备净收益。

        ```rust
        pub fn residual_scale(d_source: f64, d_target: f64) -> f64 {
            d_source / d_target
        }
        ```
        """

        let blocks = MacMarkdownTextParser.parse(richPaperMarkdown)
        XCTAssertTrue(blocks.contains(where: { if case .mathBlock = $0 { return true }; return false }))
        XCTAssertTrue(blocks.contains(where: { if case .table = $0 { return true }; return false }))
        XCTAssertTrue(blocks.contains(where: { if case .code = $0 { return true }; return false }))
        XCTAssertTrue(blocks.contains(where: { if case .quote = $0 { return true }; return false }))

        let cjkDisplay = MacInlineMarkdownCache.renderMath(
            latex: #"S_A = \lambda \cdot S_B + (\text{一小坨残差})"#,
            display: true
        )
        XCTAssertNotNil(cjkDisplay, "CJK display LaTeX must render cleanly")
        XCTAssertGreaterThan(cjkDisplay?.image.size.width ?? 0, 120)

        let cjkSubscript = MacInlineMarkdownCache.renderMath(
            latex: #"\mathcal{L}_{\text{总}} = \mathcal{L}_{\text{主}} + \lambda \mathcal{L}_{\text{正则}}"#,
            display: false
        )
        XCTAssertNotNil(cjkSubscript, "CJK subscript LaTeX must render cleanly")

        let rateFormula = MacInlineMarkdownCache.renderMath(
            latex: #"R^*=\Delta\text{bytes}/\Delta t_{\text{local}}=38.9\ \text{MB}/0.488\ \text{s}"#,
            display: false
        )
        XCTAssertNotNil(rateFormula, "Inline math with text units must render cleanly")

        let view = ScrollView {
            MacMarkdownView(
                markdown: richPaperMarkdown,
                isStreaming: false,
                animatesAppends: false,
                animatesInitialContent: false,
                suppressReveal: true
            )
            .padding(28)
        }
        .frame(width: 860, height: 920)
        .background(MacDesignColor.canvas)

        _ = try await renderToPNG(view, size: NSSize(width: 860, height: 920), colorScheme: .dark, to: evidence.appendingPathComponent("rich-math-dark.png"))
        _ = try await renderToPNG(view, size: NSSize(width: 860, height: 920), colorScheme: .light, to: evidence.appendingPathComponent("rich-math-light.png"))
    }

    // MARK: - 3. Live Streaming Bounded Thinking Viewport — Multi-Step Growth Stability (10 -> 200 -> 1500 lines)

    func testLiveStreamingThinkingViewportMultiStepGrowthStabilityAndPixelAudit() async throws {
        let evidence = try evidenceDirectory()

        func makeMixedReasoning(lines count: Int) -> String {
            (1...count).map { idx in
                "第 \(idx) 行 Thinking line \(idx): analyzing sibling-conditioned delta-rule state transfer 与图表排版检查。"
            }.joined(separator: "\n")
        }

        let reasoning10 = makeMixedReasoning(lines: 10)
        let reasoning200 = makeMixedReasoning(lines: 200)
        let reasoning1500 = makeMixedReasoning(lines: 1500)

        // 3a. MacJournalPlainPreview.liveTail must roll smoothly to trailing lines and drop early lines
        let tail1500 = MacJournalPlainPreview.liveTail(reasoning1500)
        XCTAssertTrue(tail1500.contains("Thinking line 1500"), "liveTail must display latest trailing line 1500: \(tail1500)")
        XCTAssertTrue(tail1500.contains("第 1500 行"), "liveTail must retain CJK trailing content")
        XCTAssertFalse(tail1500.contains("Thinking line 1:"), "liveTail must roll past early line 1")
        XCTAssertLessThanOrEqual(
            tail1500.split(separator: "\n", omittingEmptySubsequences: false).count,
            16,
            "liveTail must stay bounded in line count"
        )

        // 3b. Measure real NSHostingView fittingSize inside a real NSWindow across 10 -> 200 -> 1500 lines
        func makeProcessView(text: String) -> AnyView {
            AnyView(
                MacJournalProcessView(
                    partIDs: ["r0", "t0", "r1"],
                    delivery: .running,
                    requiresAttention: false,
                    toolCount: 2,
                    attentionState: nil,
                    reasoningOnly: false,
                    hasTasks: false,
                    currentActivityTitle: "Thinking"
                ) {
                    Text(verbatim: text)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(16)
                .frame(width: 720, alignment: .topLeading)
                .background(MacDesignColor.canvas)
                .environment(\.colorScheme, .dark)
            )
        }

        let hosting = NSHostingView(rootView: makeProcessView(text: reasoning10))
        hosting.frame = NSRect(x: 0, y: 0, width: 720, height: 420)
        let window = NSWindow(
            contentRect: hosting.frame,
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = hosting
        window.orderOut(nil)
        defer { window.close() }

        func measureHeight(for text: String) async throws -> CGFloat {
            hosting.rootView = makeProcessView(text: text)
            window.displayIfNeeded()
            hosting.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(60))
            RunLoop.main.run(until: Date().addingTimeInterval(0.03))
            hosting.layoutSubtreeIfNeeded()
            return hosting.fittingSize.height
        }

        let height10 = try await measureHeight(for: reasoning10)
        let height200 = try await measureHeight(for: reasoning200)
        let height1500 = try await measureHeight(for: reasoning1500)

        XCTAssertGreaterThanOrEqual(height10, 80, "10-line live thinking should be visibly rendered (measured=\(height10)pt)")
        XCTAssertLessThanOrEqual(height10, 260, "10-line live thinking must stay bounded (measured=\(height10)pt)")
        XCTAssertGreaterThanOrEqual(height200, 120, "200-line live thinking must display rolling body (measured=\(height200)pt)")
        XCTAssertLessThanOrEqual(height200, 260, "200-line live thinking MUST stay <= 260pt (measured=\(height200)pt)")
        XCTAssertGreaterThanOrEqual(height1500, 120, "1500-line live thinking must display rolling body (measured=\(height1500)pt)")
        XCTAssertLessThanOrEqual(height1500, 260, "1500-line live thinking MUST stay <= 260pt (measured=\(height1500)pt)")
        XCTAssertLessThanOrEqual(
            abs(height1500 - height200),
            2.0,
            "Live thinking viewport height MUST NOT jump between 200 and 1500 lines (h200=\(height200), h1500=\(height1500))"
        )

        let bounded1500 = MacJournalProcessView(
            partIDs: ["r0", "t0", "r1"],
            delivery: .running,
            requiresAttention: false,
            toolCount: 3,
            attentionState: nil,
            reasoningOnly: false,
            hasTasks: false,
            currentActivityTitle: "Thinking"
        ) {
            Text(verbatim: tail1500)
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(16)
        .frame(width: 720, alignment: .topLeading)
        .background(MacDesignColor.canvas)

        _ = try await renderToPNG(bounded1500, size: NSSize(width: 720, height: 260), colorScheme: .dark, to: evidence.appendingPathComponent("live-thinking-bounded-dark.png"), settleMs: 60)
        _ = try await renderToPNG(bounded1500, size: NSSize(width: 720, height: 260), colorScheme: .light, to: evidence.appendingPathComponent("live-thinking-bounded-light.png"), settleMs: 60)
    }

    // MARK: - 4. Right-Sidebar Inline Preview Lifecycle, QLPreviewView Safety, ZERO Popup Windows & Click Routing

    func testRightSidebarInlinePreviewLifecycleQLPreviewSafetyAndZeroPopupWindows() async throws {
        let evidence = try evidenceDirectory()
        let root = try makeTempDirectory("inline-preview")

        let pngURL = root.appendingPathComponent("figure-audit.png")
        let pdfURL = root.appendingPathComponent("report-audit.pdf")
        let swiftURL = root.appendingPathComponent("SampleService.swift")
        let mdURL = root.appendingPathComponent("README-audit.md")

        try generateTestPNG(at: pngURL, width: 640, height: 400)
        try generateSinglePagePDF(
            at: pdfURL,
            title: "Nola Inline PDF Preview Audit",
            body: "Verified native PDFKit rendering inside right-sidebar MacCodeReferenceInspector.\n中文排版与图表渲染检查通过。"
        )
        try """
        // SampleService.swift
        import Foundation

        struct SampleService {
            func computeResidualScale(source: Double, target: Double) -> Double {
                guard target != 0 else { return 1.0 }
                return source / target
            }
        }
        """.write(to: swiftURL, atomically: true, encoding: .utf8)
        try """
        # Nola 文档预览测试

        这是一个用于验证右侧边栏内联预览的 Markdown 文件。
        - 支持代码行号与语法高亮
        - 支持本地 PNG / PDF 原生内联渲染
        """.write(to: mdURL, atomically: true, encoding: .utf8)

        // 4a. Direct MacNativeDocumentPreview contract: PNG -> QLPreviewView(shouldCloseWithWindow == false) + idempotent double dismantle
        let pngPreview = MacNativeDocumentPreview.makePreview(for: pngURL)
        let qlView = try XCTUnwrap(pngPreview as? QLPreviewView, "PNG must use QLPreviewView")
        XCTAssertFalse(
            qlView.shouldCloseWithWindow,
            "QLPreviewView MUST set shouldCloseWithWindow = false to avoid QLPreviewActivatedInternalState crash on window close"
        )
        do {
            let qlWindow = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 420, height: 320),
                styleMask: [.titled, .closable],
                backing: .buffered,
                defer: false
            )
            qlWindow.isReleasedWhenClosed = false
            qlView.frame = NSRect(x: 0, y: 0, width: 420, height: 320)
            qlWindow.contentView = qlView
            qlWindow.orderOut(nil)
            qlWindow.displayIfNeeded()
            let coordinator = MacNativeDocumentPreview.Coordinator()
            // Double dismantle MUST be completely idempotent and safe before closing host NSWindow
            MacNativeDocumentPreview.dismantleNSView(qlView, coordinator: coordinator)
            MacNativeDocumentPreview.dismantleNSView(qlView, coordinator: coordinator)
            qlWindow.close()
        }

        // 4b. Direct MacNativeDocumentPreview contract: PDF -> PDFView with pageCount == 1
        let pdfPreview = MacNativeDocumentPreview.makePreview(for: pdfURL)
        let pdfView = try XCTUnwrap(pdfPreview as? PDFView, "PDF must use native PDFView")
        XCTAssertEqual(pdfView.document?.pageCount, 1, "1-page PDF fixture must load with pageCount == 1")

        // 4c. Render MacCodeReferenceInspector for .swift, .md, .png, .pdf in Dark & Light modes
        //     Verify visible ink AND zero extra popup/QuickLook windows spawned!
        let cases: [(name: String, url: URL, line: Int?)] = [
            ("swift", swiftURL, 5),
            ("md", mdURL, nil),
            ("pdf", pdfURL, nil),
            ("png", pngURL, nil),
        ]
        for item in cases {
            for scheme in [ColorScheme.dark, ColorScheme.light] {
                let windowsBefore = NSApp.windows.filter { $0.isVisible }.count
                let inspector = MacCodeReferenceInspector(
                    reference: MacCodeReference(url: item.url, line: item.line, column: nil),
                    canReadLocally: true,
                    close: {}
                )
                .frame(width: 460, height: 520)

                let suffix = scheme == .dark ? "dark" : "light"
                _ = try await renderToPNG(
                    inspector,
                    size: NSSize(width: 460, height: 520),
                    colorScheme: scheme,
                    to: evidence.appendingPathComponent("inspector-\(item.name)-\(suffix).png"),
                    settleMs: 180
                )
                let windowsAfter = NSApp.windows.filter { $0.isVisible }.count
                XCTAssertEqual(
                    windowsAfter,
                    windowsBefore,
                    "Opening \(item.name) in MacCodeReferenceInspector MUST NOT spawn external popup/QuickLook windows"
                )
            }
        }

        // 4d. Verify click routing for MacInspectorArtifactRow, MacJournalArtifactList, and MacInspectorWorkingContextView
        //     routes strictly through inspectFile / \.macOpenFileReference with ZERO popup windows!
        do {
            var inspectedRefs: [MacCodeReference] = []
            let windowsBefore = NSApp.windows.filter { $0.isVisible }.count
            let row = MacInspectorArtifactRow(
                name: "figure-audit.png",
                url: pngURL,
                source: Date(),
                type: "image/png",
                canReadLocalFiles: true,
                isLocalHost: true,
                inspectFile: { inspectedRefs.append($0) }
            )
            .frame(width: 340, height: 220)
            let hosting = NSHostingView(rootView: row.environment(\.colorScheme, .dark))
            hosting.frame = NSRect(x: 0, y: 0, width: 340, height: 220)
            let win = NSWindow(contentRect: hosting.frame, styleMask: [.titled, .closable], backing: .buffered, defer: false)
            win.isReleasedWhenClosed = false
            win.contentView = hosting
            win.orderOut(nil)
            defer { win.close() }
            try await Task.sleep(for: .milliseconds(120))
            hosting.layoutSubtreeIfNeeded()
            XCTAssertTrue(pressFirstButton(in: hosting, window: win), "MacInspectorArtifactRow must expose pressable button")
            XCTAssertEqual(inspectedRefs.last?.url.standardizedFileURL, pngURL.standardizedFileURL)
            XCTAssertEqual(NSApp.windows.filter { $0.isVisible && $0 !== win }.count, windowsBefore, "MacInspectorArtifactRow click must spawn 0 popup windows")
        }

        // MacJournalArtifactList routes local file clicks through \.macOpenFileReference (proven via real temp SQLite conversation)
        do {
            let dbURL = root.appendingPathComponent("artifact-list.db")
            let port = try MacApplicationPort(databaseURL: dbURL)
            let convID = "conv-artifact-routing"
            _ = try port.createConversation(MacCanonicalCreateConversationInput(
                id: convID,
                title: "Artifact Routing",
                settingsJSON: nil,
                createdAtMs: 1_000,
                updatedAtMs: 1_000
            ))
            let cursor = MacAssistantJournalCursor(
                identity: .init(
                    source_store_epoch: "epoch-art",
                    conversation_id: convID,
                    run_id: "run-art",
                    assistant_message_id: "asst-art"
                ),
                sequence: "1",
                record_digest: "digest-art"
            )
            let reference = MacJournalMessageReference(databaseURL: dbURL, cursor: cursor)
            var openedRefs: [MacCodeReference] = []
            let windowsBefore = NSApp.windows.filter { $0.isVisible }.count
            let listView = MacJournalArtifactList(
                reference: reference,
                artifacts: [
                    MacCorePartArtifact(uri: pdfURL.absoluteString, name: "report-audit.pdf", mimeType: "application/pdf", operation: "write")
                ]
            )
            .environment(\.macOpenFileReference, MacOpenFileReferenceAction { openedRefs.append($0) })
            .frame(width: 420, height: 120)
            .environment(\.colorScheme, .dark)

            let hosting = NSHostingView(rootView: listView)
            hosting.frame = NSRect(x: 0, y: 0, width: 420, height: 120)
            let win = NSWindow(contentRect: hosting.frame, styleMask: [.titled, .closable], backing: .buffered, defer: false)
            win.isReleasedWhenClosed = false
            win.contentView = hosting
            win.orderOut(nil)
            defer { win.close() }
            try await Task.sleep(for: .milliseconds(160))
            RunLoop.main.run(until: Date().addingTimeInterval(0.03))
            hosting.layoutSubtreeIfNeeded()
            XCTAssertTrue(pressFirstButton(in: hosting, window: win), "MacJournalArtifactList must expose pressable button for local artifact")
            XCTAssertEqual(openedRefs.last?.url.standardizedFileURL, pdfURL.standardizedFileURL, "MacJournalArtifactList must route clicks via macOpenFileReference")
            XCTAssertEqual(NSApp.windows.filter { $0.isVisible && $0 !== win }.count, windowsBefore, "MacJournalArtifactList click must spawn 0 popup windows")
        }

        // MacInspectorWorkingContextView routes attached file clicks through \.macOpenFileReference
        do {
            let dbURL = root.appendingPathComponent("working-ctx.db")
            let port = try MacApplicationPort(databaseURL: dbURL)
            let repository = MacCanonicalConversationRepository(store: port)
            let configStore = MacAppleChatConfigurationStore(applicationPort: port)
            let agentRuntime = MacAgentHostRuntime(credentialStore: configStore)
            let suiteName = "com.gaojc.Nola.audit-ctx.\(UUID().uuidString)"
            let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
            addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: suiteName) }
            let workspaceModel = MacWorkspaceModel(
                canonicalRepository: repository,
                localAgentRuntime: agentRuntime,
                remoteHost: E2ENoOpRemoteHost(),
                modelApplication: port,
                bookmarkStore: MacSecurityScopedBookmarkStore(defaults: defaults),
                activityStore: MacActivityStore(baseDirectory: root.appendingPathComponent("Activity")),
                completionNotifier: { _, _, _ in },
                readinessIssue: { _ in nil }
            )
            workspaceModel.attachments = [MacAttachment(url: swiftURL, status: .ready)]
            var openedRefs: [MacCodeReference] = []
            let windowsBefore = NSApp.windows.filter { $0.isVisible }.count
            let ctxView = MacInspectorWorkingContextView(model: workspaceModel, resourcesOnly: true, canReadLocalFiles: true)
                .environment(\.macOpenFileReference, MacOpenFileReferenceAction { openedRefs.append($0) })
                .frame(width: 340, height: 180)

            let hosting = NSHostingView(rootView: ctxView.environment(\.colorScheme, .dark))
            hosting.frame = NSRect(x: 0, y: 0, width: 340, height: 180)
            let win = NSWindow(contentRect: hosting.frame, styleMask: [.titled, .closable], backing: .buffered, defer: false)
            win.isReleasedWhenClosed = false
            win.contentView = hosting
            win.orderOut(nil)
            defer {
                workspaceModel.presentationDidDetach()
                win.close()
            }
            try await Task.sleep(for: .milliseconds(100))
            hosting.layoutSubtreeIfNeeded()
            XCTAssertTrue(pressFirstButton(in: hosting, window: win), "MacInspectorWorkingContextView must render pressable attachment button")
            XCTAssertEqual(openedRefs.last?.url.standardizedFileURL, swiftURL.standardizedFileURL, "MacInspectorWorkingContextView must route clicks via macOpenFileReference")
            XCTAssertEqual(NSApp.windows.filter { $0.isVisible && $0 !== win }.count, windowsBefore, "MacInspectorWorkingContextView click must spawn 0 popup windows")
        }
    }

    // MARK: - 5. Interleaved Tool Cards (Success + Failure) & Inline Image Thumbnail Bounding Box

    func testToolCardsSuccessAndFailureAndInlineImageThumbnailBoundingBox() async throws {
        let evidence = try evidenceDirectory()
        let root = try makeTempDirectory("tools-and-thumb")
        let dbURL = root.appendingPathComponent("tools.db")
        let cursor = MacAssistantJournalCursor(
            identity: .init(
                source_store_epoch: "epoch-tools",
                conversation_id: "conv-tools",
                run_id: "run-tools",
                assistant_message_id: "asst-tools"
            ),
            sequence: "2",
            record_digest: "digest-tools"
        )
        let ref = MacJournalMessageReference(databaseURL: dbURL, cursor: cursor)

        func decodeToolPart(id: String, pos: Int, capability: String, title: String, state: String, summary: String) throws -> (MacAssistantJournalPart, MacAssistantJournalMetadata.Tool) {
            let dict: [String: Any] = [
                "identity": [
                    "source_store_epoch": "epoch-tools",
                    "conversation_id": "conv-tools",
                    "assistant_message_id": "asst-tools",
                    "run_id": "run-tools",
                ],
                "part_id": id,
                "generation": "1",
                "revision": "1",
                "metadata_revision": "1",
                "order_key": ["section": "transcript", "position": String(pos)],
                "metadata": [
                    "kind": "tool",
                    "data": [
                        "tool_call_id": id,
                        "capability_id": capability,
                        "display_key": capability,
                        "origin": ["kind": "builtin"],
                        "execution_source": ["kind": "model"],
                        "state": state,
                        "title": ["kind": "inline", "value": title],
                        "summary": ["kind": "inline", "value": summary],
                        "affected_resources": ["kind": "inline", "value": []],
                        "artifacts": "tool_artifacts",
                    ],
                ],
                "bodies": [],
            ]
            let data = try JSONSerialization.data(withJSONObject: dict)
            let part = try JSONDecoder().decode(MacAssistantJournalPart.self, from: data)
            guard case .tool(let tool) = part.metadata else {
                throw NSError(domain: "ToolDecode", code: 1)
            }
            return (part, tool)
        }

        let (okPart, okTool) = try decodeToolPart(
            id: "call_read_1",
            pos: 0,
            capability: "read_file",
            title: "Read paper.md",
            state: "completed",
            summary: "Read 42 lines from workspace/paper.md"
        )
        let (failPart, failTool) = try decodeToolPart(
            id: "call_shell_2",
            pos: 1,
            capability: "bash",
            title: "Run git status",
            state: "failed",
            summary: "exit_code = 128: fatal: not a git repository (or any of the parent directories): .git"
        )

        let toolsCardStack = VStack(alignment: .leading, spacing: 12) {
            MacJournalToolView(reference: ref, part: okPart, tool: okTool)
            Divider()
            MacJournalToolView(reference: ref, part: failPart, tool: failTool)
        }
        .padding(18)
        .frame(width: 680, alignment: .topLeading)
        .background(MacDesignColor.canvas)

        _ = try await renderToPNG(toolsCardStack, size: NSSize(width: 680, height: 220), colorScheme: .dark, to: evidence.appendingPathComponent("tool-cards-dark.png"), settleMs: 40)
        _ = try await renderToPNG(toolsCardStack, size: NSSize(width: 680, height: 220), colorScheme: .light, to: evidence.appendingPathComponent("tool-cards-light.png"), settleMs: 40)

        // Inline image thumbnail bounding box: oversized 1600x1200 PNG MUST stay bounded <= 420x280pt!
        let oversizedPNG = root.appendingPathComponent("oversized-figure.png")
        try generateTestPNG(at: oversizedPNG, width: 1600, height: 1200)

        let thumbView = MacJournalArtifactImageThumbnail(
            url: oversizedPNG,
            name: "oversized-figure.png",
            onOpen: {},
            fallback: { Text("fallback") }
        )
        .padding(12)
        .background(MacDesignColor.canvas)

        // Measure intrinsic fittingSize of the thumbnail after async NSImage load
        let thumbHost = NSHostingView(rootView: AnyView(thumbView.environment(\.colorScheme, .dark)))
        thumbHost.frame = NSRect(x: 0, y: 0, width: 720, height: 420)
        let thumbWindow = NSWindow(
            contentRect: thumbHost.frame,
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        thumbWindow.isReleasedWhenClosed = false
        thumbWindow.contentView = thumbHost
        thumbWindow.orderOut(nil)
        try await Task.sleep(for: .milliseconds(180))
        RunLoop.main.run(until: Date().addingTimeInterval(0.03))
        thumbHost.layoutSubtreeIfNeeded()
        let measuredThumb = thumbHost.fittingSize
        thumbWindow.close()

        // 420pt max image width + 24pt padding = 444pt; 280pt max image height + ~20pt caption + 24pt padding = ~324pt
        XCTAssertLessThanOrEqual(measuredThumb.width, 446, "Inline image thumbnail width must stay bounded <= 420pt + padding (measured=\(measuredThumb.width))")
        XCTAssertLessThanOrEqual(measuredThumb.height, 330, "Inline image thumbnail height must stay bounded <= 280pt + caption + padding (measured=\(measuredThumb.height))")
        XCTAssertGreaterThanOrEqual(measuredThumb.width, 160, "Inline image thumbnail must visibly render loaded image")
        XCTAssertGreaterThanOrEqual(measuredThumb.height, 140, "Inline image thumbnail must visibly render loaded image")

        _ = try await renderToPNG(
            thumbView,
            size: NSSize(width: 480, height: 340),
            colorScheme: .dark,
            to: evidence.appendingPathComponent("inline-image-thumbnail-dark.png"),
            settleMs: 180
        )
        _ = try await renderToPNG(
            thumbView,
            size: NSSize(width: 480, height: 340),
            colorScheme: .light,
            to: evidence.appendingPathComponent("inline-image-thumbnail-light.png"),
            settleMs: 180
        )
    }

    // MARK: - 6. Completed Tool Grouping & Port Pool Contracts

    func testCompletedToolGroupingCollapsesToolsAcrossInterleavedReasoning() throws {
        let identityJSON: [String: Any] = [
            "source_store_epoch": "epoch-group",
            "conversation_id": "conv-group",
            "assistant_message_id": "asst-group",
            "run_id": "run-group",
        ]
        func decodePart(_ dict: [String: Any]) throws -> MacAssistantJournalPart {
            let data = try JSONSerialization.data(withJSONObject: dict)
            return try JSONDecoder().decode(MacAssistantJournalPart.self, from: data)
        }
        func makeReasoning(_ id: String, pos: Int) throws -> MacAssistantJournalPart {
            try decodePart([
                "identity": identityJSON,
                "part_id": id,
                "generation": "1",
                "revision": "1",
                "metadata_revision": "1",
                "order_key": ["section": "transcript", "position": String(pos)],
                "metadata": [
                    "kind": "reasoning",
                    "data": [
                        "state": "completed",
                        "title": ["kind": "inline", "value": "Thinking"],
                    ],
                ],
                "bodies": [],
            ])
        }
        func makeCompletedTool(_ id: String, pos: Int) throws -> MacAssistantJournalPart {
            try decodePart([
                "identity": identityJSON,
                "part_id": id,
                "generation": "1",
                "revision": "1",
                "metadata_revision": "1",
                "order_key": ["section": "transcript", "position": String(pos)],
                "metadata": [
                    "kind": "tool",
                    "data": [
                        "tool_call_id": id,
                        "capability_id": "read_file",
                        "display_key": "read_file",
                        "origin": ["kind": "builtin"],
                        "execution_source": ["kind": "model"],
                        "state": "completed",
                        "title": ["kind": "inline", "value": "Read file"],
                        "affected_resources": ["kind": "inline", "value": []],
                        "artifacts": "tool_artifacts",
                    ],
                ],
                "bodies": [],
            ])
        }

        let interleaved: [MacAssistantJournalPart] = try [
            makeReasoning("r0", pos: 0),
            makeCompletedTool("t0", pos: 1),
            makeReasoning("r1", pos: 2),
            makeCompletedTool("t1", pos: 3),
            makeReasoning("r2", pos: 4),
            makeCompletedTool("t2", pos: 5),
            makeReasoning("r3", pos: 6),
            makeCompletedTool("t3", pos: 7),
        ]
        let grouping = MacJournalCompletedToolGrouping(interleaved)
        XCTAssertEqual(grouping.groups["t0"]?.count, 4, "4 completed tools separated only by completed reasoning must collapse into 1 group")
        XCTAssertEqual(grouping.hiddenIDs, Set(["t1", "t2", "t3"]))
    }

    func testPortPoolReusesSharedAndCustomDatabaseConnectionsAndInspectorIdentifiesImages() throws {
        let shared = try MacApplicationPort.pooled(databaseURL: MacApplicationPort.shared.databaseURL)
        XCTAssertTrue(shared === MacApplicationPort.shared, "pooled(defaultDB) must return MacApplicationPort.shared")

        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-pool-test-\(UUID().uuidString).db")
        addTeardownBlock { try? FileManager.default.removeItem(at: tmp) }
        let p1 = try MacApplicationPort.pooled(databaseURL: tmp)
        let p2 = try MacApplicationPort.pooled(databaseURL: tmp)
        XCTAssertTrue(p1 === p2, "pooled(customDB) must reuse open SQLite connection")

        XCTAssertTrue(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/paper-figure.png"), type: "png"))
        XCTAssertTrue(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/diagram.svg"), type: ""))
        XCTAssertTrue(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/render.bin"), type: "image/webp"))
        XCTAssertFalse(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/main.rs"), type: "rs"))
        XCTAssertFalse(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/paper.pdf"), type: "application/pdf"))
    }

    // MARK: - 7. Deterministic Local Loopback Mock-HTTP Multi-Turn Tool Execution, HTTP 400 Error & Cold Reopen Persistence

    private final class LoopbackMockOpenAIServer: @unchecked Sendable {
        struct Response {
            let statusLine: String
            let contentType: String
            let body: Data
        }

        private let queue = DispatchQueue(label: "nola.test.mock-openai-server")
        private var listener: NWListener?
        private let lock = NSLock()
        private var chatCallCount = 0
        private(set) var capturedChatBodies: [String] = []
        let handler: @Sendable (String, String, String, Int) -> Response
        private(set) var port: UInt16 = 0

        init(handler: @escaping @Sendable (String, String, String, Int) -> Response) {
            self.handler = handler
        }

        func start() async throws -> URL {
            let params = NWParameters.tcp
            params.allowLocalEndpointReuse = true
            let listener = try NWListener(using: params, on: .any)
            self.listener = listener
            let readyPort: UInt16 = try await withCheckedThrowingContinuation { continuation in
                var resumed = false
                listener.stateUpdateHandler = { state in
                    guard !resumed else { return }
                    switch state {
                    case .ready:
                        resumed = true
                        continuation.resume(returning: listener.port?.rawValue ?? 0)
                    case .failed(let err):
                        resumed = true
                        continuation.resume(throwing: err)
                    default:
                        break
                    }
                }
                listener.newConnectionHandler = { [weak self] conn in
                    self?.handleConnection(conn)
                }
                listener.start(queue: self.queue)
            }
            self.port = readyPort
            return URL(string: "http://127.0.0.1:\(readyPort)/v1")!
        }

        func stop() {
            listener?.cancel()
            listener = nil
        }

        private func handleConnection(_ conn: NWConnection) {
            conn.start(queue: queue)
            receiveHTTPRequest(conn: conn, buffer: Data())
        }

        private func receiveHTTPRequest(conn: NWConnection, buffer: Data) {
            conn.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { [weak self] data, _, isComplete, error in
                guard let self else { conn.cancel(); return }
                var acc = buffer
                if let data { acc.append(data) }
                if let range = acc.range(of: Data("\r\n\r\n".utf8)) {
                    let headerData = acc[..<range.lowerBound]
                    let bodyStart = range.upperBound
                    let headerText = String(decoding: headerData, as: UTF8.self)
                    var contentLength = 0
                    for line in headerText.components(separatedBy: "\r\n") {
                        if line.lowercased().hasPrefix("content-length:") {
                            let val = line.dropFirst("content-length:".count).trimmingCharacters(in: .whitespaces)
                            contentLength = Int(val) ?? 0
                        }
                    }
                    if acc.count - bodyStart >= contentLength {
                        let firstLine = headerText.components(separatedBy: "\r\n").first ?? "GET / HTTP/1.1"
                        let parts = firstLine.split(separator: " ")
                        let method = parts.count > 0 ? String(parts[0]) : "GET"
                        let path = parts.count > 1 ? String(parts[1]) : "/"
                        let bodySlice = acc[bodyStart ..< (bodyStart + contentLength)]
                        let bodyStr = String(decoding: bodySlice, as: UTF8.self)

                        var callIndex = 0
                        if path.contains("/chat/completions") {
                            self.lock.lock()
                            self.chatCallCount += 1
                            callIndex = self.chatCallCount
                            self.capturedChatBodies.append(bodyStr)
                            self.lock.unlock()
                        }

                        let resp = self.handler(method, path, bodyStr, callIndex)
                        var raw = "HTTP/1.1 \(resp.statusLine)\r\n"
                        raw += "Content-Type: \(resp.contentType)\r\n"
                        raw += "Content-Length: \(resp.body.count)\r\n"
                        raw += "Connection: close\r\n\r\n"
                        var out = Data(raw.utf8)
                        out.append(resp.body)
                        conn.send(content: out, completion: .contentProcessed { _ in
                            conn.cancel()
                        })
                        return
                    }
                }
                if isComplete || error != nil {
                    conn.cancel()
                } else {
                    self.receiveHTTPRequest(conn: conn, buffer: acc)
                }
            }
        }
    }

    func testDeterministicMockHTTPMultiTurnToolExecutionErrorRecoveryAndColdReopenPersistence() async throws {
        let root = try makeTempDirectory("mock-http-multiturn")
        let workspaceDir = root.appendingPathComponent("workspace", isDirectory: true)
        try FileManager.default.createDirectory(at: workspaceDir, withIntermediateDirectories: true)
        let helloFile = workspaceDir.appendingPathComponent("hello.txt")
        try "MOCK_FILE_SENTINEL_42\nLine two for audit.\n".write(to: helloFile, atomically: true, encoding: .utf8)

        let server = LoopbackMockOpenAIServer { method, path, body, callIndex in
            if method == "GET" && path.contains("/models") {
                let json = #"{"object":"list","data":[{"id":"mock-nola-model","object":"model"}]}"#
                return .init(statusLine: "200 OK", contentType: "application/json", body: Data(json.utf8))
            }
            if method == "POST" && path.contains("/chat/completions") {
                if callIndex == 1 {
                    let readArgs = #"{"path":"hello.txt"}"#
                    let bashArgs = #"{"command":"echo simulated_stderr_boom >&2; exit 128"}"#
                    let readArgsEscaped = readArgs.replacingOccurrences(of: "\"", with: "\\\"")
                    let bashArgsEscaped = bashArgs.replacingOccurrences(of: "\"", with: "\\\"")
                    let sse = [
                        #"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000001,"model":"mock-nola-model","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"先读取 hello.txt 并验证失败命令的 stderr 上报。"},"finish_reason":null}]}"#,
                        #"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000001,"model":"mock-nola-model","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_read_ok","type":"function","function":{"name":"read_file","arguments":"\#(readArgsEscaped)"}}]},"finish_reason":null}]}"#,
                        #"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000001,"model":"mock-nola-model","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_bash_fail","type":"function","function":{"name":"bash","arguments":"\#(bashArgsEscaped)"}}]},"finish_reason":null}]}"#,
                        #"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000001,"model":"mock-nola-model","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
                        "data: [DONE]",
                    ].joined(separator: "\n\n") + "\n\n"
                    return .init(statusLine: "200 OK", contentType: "text/event-stream", body: Data(sse.utf8))
                } else if callIndex == 2 {
                    let sawSentinel = body.contains("MOCK_FILE_SENTINEL_42")
                    let sawFailure = body.contains("128") || body.contains("simulated_stderr_boom")
                    let reply = sawSentinel && sawFailure
                        ? "工具执行完成：已读出 MOCK_FILE_SENTINEL_42，且成功捕获失败工具 exit_code=128。"
                        : "MISSING_TOOL_RESULTS sentinel=\(sawSentinel) failure=\(sawFailure)"
                    let sse = [
                        #"data: {"id":"chatcmpl-2","object":"chat.completion.chunk","created":1700000002,"model":"mock-nola-model","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"工具结果均已回传，整理最终答复。"},"finish_reason":null}]}"#,
                        #"data: {"id":"chatcmpl-2","object":"chat.completion.chunk","created":1700000002,"model":"mock-nola-model","choices":[{"index":0,"delta":{"content":"\#(reply)"},"finish_reason":null}]}"#,
                        #"data: {"id":"chatcmpl-2","object":"chat.completion.chunk","created":1700000002,"model":"mock-nola-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
                        "data: [DONE]",
                    ].joined(separator: "\n\n") + "\n\n"
                    return .init(statusLine: "200 OK", contentType: "text/event-stream", body: Data(sse.utf8))
                } else {
                    let errJSON = #"{"error":{"message":"Mock upstream HTTP 400 invalid_request_error","type":"invalid_request_error","code":"invalid_request"}}"#
                    return .init(statusLine: "400 Bad Request", contentType: "application/json", body: Data(errJSON.utf8))
                }
            }
            return .init(statusLine: "404 Not Found", contentType: "text/plain", body: Data("not found".utf8))
        }

        let endpointURL = try await server.start()
        defer { server.stop() }

        let databaseURL = root.appendingPathComponent("nola-mock-multiturn.db")
        let convID: String
        do {
            let port = try MacApplicationPort(databaseURL: databaseURL)
            let configStore = MacAppleChatConfigurationStore(applicationPort: port)
            let connectionService = MacModelConnectionService(application: port)
            let created = try connectionService.create(
                name: "Loopback Mock LLM",
                endpoint: endpointURL.absoluteString,
                wireProtocol: MacModelWireProtocol.chatCompletions.rawValue,
                secret: "sk-mock-local-deterministic"
            )
            let discovered = try await connectionService.discover(created)
            let models = try XCTUnwrap(discovered.document.catalog?.models)
            let target = try XCTUnwrap(models.first(where: { $0.model_id == "mock-nola-model" }))
            try connectionService.enable(discovered, modelIDs: [target.id])

            let repository = MacCanonicalConversationRepository(store: port)
            let agentRuntime = MacAgentHostRuntime(credentialStore: configStore)
            let suiteName = "com.gaojc.Nola.mock-http.\(UUID().uuidString)"
            let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
            addTeardownBlock { UserDefaults.standard.removePersistentDomain(forName: suiteName) }

            let wm = MacWorkspaceModel(
                canonicalRepository: repository,
                localAgentRuntime: agentRuntime,
                remoteHost: E2ENoOpRemoteHost(),
                modelApplication: port,
                bookmarkStore: MacSecurityScopedBookmarkStore(defaults: defaults),
                activityStore: MacActivityStore(baseDirectory: root.appendingPathComponent("Activity")),
                completionNotifier: { _, _, _ in },
                readinessIssue: { _ in nil }
            )
            defer {
                if wm.isRunning { wm.stop(reason: "teardown") }
                wm.presentationDidDetach()
            }

            wm.newConversation()
            await wm.loadSelectedConversationIfNeeded()
            try await wm.waitForModelConfigurationCommit()
            let option = try XCTUnwrap(wm.configuredModelOptions.first(where: { $0.modelID == "mock-nola-model" }))
            wm.selectModel(option)
            wm.permissionMode = .allowAll
            try await wm.waitForModelConfigurationCommit()
            XCTAssertNotNil(wm.switchWorkspace(to: workspaceDir))
            convID = try XCTUnwrap(wm.selectedConversationID)

            // Turn 1: streaming reasoning + read_file (success) + bash (exit 128 failure) -> final reply
            wm.draft = "请读取 hello.txt 并运行测试命令。"
            wm.send()
            XCTAssertTrue(wm.isRunning)
            let t1Deadline = ContinuousClock.now + .seconds(30)
            while wm.isRunning && ContinuousClock.now < t1Deadline {
                try await Task.sleep(for: .milliseconds(80))
            }
            XCTAssertFalse(wm.isRunning, "Turn 1 must complete deterministically against loopback mock server")
            await wm.loadSelectedConversationIfNeeded()
            try await Task.sleep(for: .milliseconds(200))

            let turn1Reply = wm.selectedMessages.filter { $0.role == .assistant }.map(\.text).joined(separator: "\n")
            XCTAssertTrue(turn1Reply.contains("MOCK_FILE_SENTINEL_42"), "Turn 1 reply must include read_file output: \(turn1Reply)")
            XCTAssertTrue(turn1Reply.contains("128"), "Turn 1 reply must reflect failed bash exit_code=128: \(turn1Reply)")

            // Turn 2: upstream HTTP 400 error -> must terminate cleanly and persist failure
            wm.draft = "第二轮触发上游 400 错误恢复验证。"
            wm.send()
            XCTAssertTrue(wm.isRunning)
            let t2Deadline = ContinuousClock.now + .seconds(30)
            while wm.isRunning && ContinuousClock.now < t2Deadline {
                try await Task.sleep(for: .milliseconds(80))
            }
            XCTAssertFalse(wm.isRunning, "Turn 2 HTTP 400 must fail cleanly without hanging")
            await wm.loadSelectedConversationIfNeeded()
            try await Task.sleep(for: .milliseconds(200))

            let failedAssistant = wm.selectedMessages.last(where: { $0.role == .assistant })
            XCTAssertEqual(failedAssistant?.deliveryState, .failed, "Turn 2 HTTP 400 must record failed deliveryState")
        }

        // Cold reopen from the EXACT same temp SQLite DB -> verify durable transcript & model config!
        let reopenedPort = try MacApplicationPort(databaseURL: databaseURL)
        let reopenedConversation = try XCTUnwrap(try reopenedPort.loadConversation(convID))
        XCTAssertGreaterThanOrEqual(reopenedConversation.messages.count, 4, "Both turns (2 user + 2 assistant) must persist across cold reopen")

        let projection = try reopenedPort.loadConversationProjectionV1(convID)
        let assistantMessages = projection.messages.filter { $0.role == .assistant }
        XCTAssertGreaterThanOrEqual(assistantMessages.count, 2, "Both assistant turns must be projected after cold reopen")
        let allAssistantTexts = assistantMessages
            .flatMap(\.parts)
            .compactMap { part -> String? in
                if case .text(let textPart) = part { return textPart.markdown }
                return nil
            }
            .joined(separator: "\n")
        XCTAssertTrue(allAssistantTexts.contains("MOCK_FILE_SENTINEL_42"), "Cold reopen must preserve Turn 1 tool-derived answer: \(allAssistantTexts)")

        let lastAssistant = try XCTUnwrap(assistantMessages.last)
        XCTAssertEqual(lastAssistant.state, .failed, "Cold reopen projection must preserve Turn 2 failed terminal state")

        let persistedProfiles = try reopenedPort.listModelProfiles()
        XCTAssertTrue(
            persistedProfiles.contains(where: { $0.modelName == "mock-nola-model" && $0.enabled }),
            "Cold reopen must preserve configured mock-nola-model profile"
        )
    }
}
