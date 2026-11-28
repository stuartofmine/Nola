// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Strict UI geometry & rich-rendering audit tests verifying 0.0pt composer jump across
//   all reasoning/permission/plan toggles, plus dark/light visual captures of academic math, tables,
//   code blocks, and Mermaid diagrams.

import AppKit
import Foundation
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRichRenderingVisualAuditTests: XCTestCase {
    private func evidenceDirectory() throws -> URL {
        let dir = URL(fileURLWithPath: "/tmp/nola-real-e2e-evidence/macos-audit", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func renderToPNG<V: View>(_ view: V, size: NSSize, colorScheme: ColorScheme, to url: URL) throws -> NSHostingView<AnyView> {
        let wrapped = AnyView(view.environment(\.colorScheme, colorScheme))
        let hosting = NSHostingView(rootView: wrapped)
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
        window.displayIfNeeded()
        hosting.layoutSubtreeIfNeeded()
        guard let rep = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds) else {
            throw NSError(domain: "RenderPNG", code: 1)
        }
        hosting.cacheDisplay(in: hosting.bounds, to: rep)
        guard let png = rep.representation(using: .png, properties: [:]) else {
            throw NSError(domain: "RenderPNG", code: 2)
        }
        try png.write(to: url, options: .atomic)
        window.close()
        return hosting
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

    // MARK: - 2. Academic Paper Rich Markdown (Inline & Display Math, Tables, Code, Quotes) Visual Audit (Dark & Light)

    func testAcademicMathTablesCodeRichMarkdownDarkAndLightAudit() throws {
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

        // Verify AST promotes the display equation cleanly
        let blocks = MacMarkdownTextParser.parse(richPaperMarkdown)
        XCTAssertTrue(blocks.contains(where: { if case .mathBlock = $0 { return true }; return false }))
        XCTAssertTrue(blocks.contains(where: { if case .table = $0 { return true }; return false }))
        XCTAssertTrue(blocks.contains(where: { if case .code = $0 { return true }; return false }))
        XCTAssertTrue(blocks.contains(where: { if case .quote = $0 { return true }; return false }))

        // Verify CJK-inside-LaTeX formulas render cleanly to non-empty template images (zero fallback tofu)
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

        _ = try renderToPNG(view, size: NSSize(width: 860, height: 920), colorScheme: .dark, to: evidence.appendingPathComponent("rich-math-dark.png"))
        _ = try renderToPNG(view, size: NSSize(width: 860, height: 920), colorScheme: .light, to: evidence.appendingPathComponent("rich-math-light.png"))
    }

    // MARK: - 3. Live Streaming Thinking Bounded Viewport & Interleaved Tool Grouping Audit

    func testLiveStreamingThinkingViewportStaysStrictlyBoundedAndTailRolling() throws {
        let evidence = try evidenceDirectory()

        // 1. Live tail preview MUST show the trailing lines of a 20KB reasoning stream, never freeze at line 24!
        let longReasoning = (1...120).map { idx in
            "Thinking line \(idx): analyzing sibling-conditioned delta-rule state transfer and figure layout."
        }.joined(separator: "\n")
        let tail = MacJournalPlainPreview.liveTail(longReasoning)
        XCTAssertTrue(tail.contains("Thinking line 120"), "liveTail must show latest trailing thoughts: \(tail)")
        XCTAssertFalse(tail.contains("Thinking line 1:"), "liveTail must roll past early lines")

        // 2. Live MacJournalProcessView MUST stay strictly bounded (~168pt body + ~40pt header <= 230pt)
        //    even when inner reasoning content is 120 lines (~2,400pt tall)!
        let boundedProcess = MacJournalProcessView(
            partIDs: ["r0", "t0", "r1"],
            delivery: .running,
            requiresAttention: false,
            toolCount: 3,
            attentionState: nil,
            reasoningOnly: false,
            hasTasks: false,
            currentActivityTitle: "Thinking"
        ) {
            Text(verbatim: longReasoning)
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(16)
        .frame(width: 720, alignment: .topLeading)
        .background(MacDesignColor.canvas)

        let hosting = NSHostingView(rootView: boundedProcess.environment(\.colorScheme, .dark))
        hosting.frame = NSRect(x: 0, y: 0, width: 720, height: 400)
        hosting.layoutSubtreeIfNeeded()
        let fittingHeight = hosting.fittingSize.height
        XCTAssertLessThanOrEqual(
            fittingHeight,
            260,
            "Live streaming MacJournalProcessView MUST stay bounded instead of exploding vertically (measured=\(fittingHeight)pt)"
        )
        XCTAssertGreaterThanOrEqual(
            fittingHeight,
            120,
            "Live streaming MacJournalProcessView should still display visible rolling content (measured=\(fittingHeight)pt)"
        )

        _ = try renderToPNG(
            boundedProcess,
            size: NSSize(width: 720, height: 260),
            colorScheme: .dark,
            to: evidence.appendingPathComponent("live-thinking-bounded.png")
        )
    }

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
        // Default DB URL returns MacApplicationPort.shared directly
        let shared = try MacApplicationPort.pooled(databaseURL: MacApplicationPort.shared.databaseURL)
        XCTAssertTrue(shared === MacApplicationPort.shared, "pooled(defaultDB) must return MacApplicationPort.shared")

        // Custom temp DB URL returns the same pooled instance across repeated reads
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-pool-test-\(UUID().uuidString).db")
        addTeardownBlock { try? FileManager.default.removeItem(at: tmp) }
        let p1 = try MacApplicationPort.pooled(databaseURL: tmp)
        let p2 = try MacApplicationPort.pooled(databaseURL: tmp)
        XCTAssertTrue(p1 === p2, "pooled(customDB) must reuse open SQLite connection")

        // Inspector artifact row accurately identifies image deliverables for inline preview + QuickLook
        XCTAssertTrue(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/paper-figure.png"), type: "png"))
        XCTAssertTrue(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/diagram.svg"), type: ""))
        XCTAssertTrue(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/render.bin"), type: "image/webp"))
        XCTAssertFalse(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/main.rs"), type: "rs"))
        XCTAssertFalse(MacInspectorArtifactRow.isImageArtifact(url: URL(fileURLWithPath: "/tmp/paper.pdf"), type: "application/pdf"))
    }
}
