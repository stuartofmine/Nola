// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents one macOS agent trace, including file references and expandable tool details.

import Foundation
import SwiftUI

struct MacTraceRow: View {
    let trace: MacAssistantTrace
    let allowsCollapse: Bool
    @Environment(\.macOpenFileReference) private var openFileReference
    @State private var isExpanded: Bool
    @State private var showsFullResult = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        trace: MacAssistantTrace,
        initiallyExpanded: Bool? = nil,
        allowsCollapse: Bool = true
    ) {
        self.trace = trace
        self.allowsCollapse = allowsCollapse
        _isExpanded = State(
            initialValue: allowsCollapse
                ? initiallyExpanded
                ?? MacAssistantTraceTimelinePolicy.initiallyExpandsDetails(for: trace.state)
                : true
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            traceHeader

            if isExpanded {
                VStack(alignment: .leading, spacing: 4) {
                    if let tool = trace.tool {
                        if let summary = tool.summary {
                            traceDetailBlock(title: String(localized: "Summary"), text: summary)
                        }
                        if let preview = tool.outputPreview, tool.output != nil {
                            // A structured result, including null, does not
                            // erase the independently supplied preview.
                            traceDetailBlock(title: String(localized: "Preview"), text: preview)
                        }
                        traceDetailBlock(title: String(localized: "Source"), text: [tool.sourceLabel, tool.origin.namespace]
                            .compactMap { $0 }.joined(separator: " · "))
                        if let permission = tool.permission {
                            // Disclosure only; executable approval continues to
                            // use the independently owned pending challenge.
                            traceDetailBlock(title: String(localized: "Permission"), text: [permission.permissionDomain,
                                permission.rationale, permission.decision].compactMap { $0 }.joined(separator: " · "))
                        }
                        ForEach(Array(tool.artifacts.enumerated()), id: \.offset) { _, artifact in
                            VStack(alignment: .leading, spacing: 2) {
                                Text([artifact.name, artifact.operation, artifact.mimeType].compactMap { $0 }.joined(separator: " · "))
                                Text(artifact.uri).font(.system(size: 10, design: .monospaced))
                            }
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                        }
                    }
                    if !secondaryFileLinks.isEmpty {
                        fileLinks(secondaryFileLinks)
                    }
                    // The read header already owns the file and line range.
                    // Repeating Path/Offset/Limit below it adds three rows but
                    // no information. Other tools keep their parameters open.
                    if trace.tool != nil || fileReadOperation == nil,
                       let arguments = MacToolTraceDisplayPolicy.parameters(for: trace) {
                        traceDetailBlock(title: String(localized: "Parameters"), text: arguments)
                    }
                    if let result = displayedResult {
                        traceDetailBlock(title: String(localized: "Result"), text: result)
                        if resultIsTruncated {
                            Button {
                                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                                    showsFullResult.toggle()
                                }
                            } label: {
                                Text(
                                    showsFullResult
                                        ? String(localized: "Show Less")
                                        : String(localized: "Show All")
                                )
                                    .font(.system(size: 10.5, weight: .semibold))
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(Color.accentColor)
                        }
                    } else if trace.state == .failed, !trace.detail.isEmpty {
                        traceDetailBlock(
                            title: String(localized: "Details"),
                            text: trace.detail
                        )
                    }
                }
                .padding(.leading, 24)
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private var traceHeader: some View {
        if allowsCollapse, !(trace.tool == nil && fileReadOperation != nil) {
            Button(action: toggleDetails) { traceHeaderContent }
                .buttonStyle(MacActionButtonStyle())
                .help(isExpanded ? String(localized: "Collapse Details") : String(localized: "Expand Details"))
                .accessibilityValue(isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
        } else {
            traceHeaderContent
        }
    }

    private func toggleDetails() {
        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
            isExpanded.toggle()
        }
    }

    private var traceHeaderContent: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Group {
                if trace.state == .running {
                    MacBreathingStatusLight(diameter: 16)
                } else {
                    Image(systemName: trace.kind == .plan ? "list.bullet.clipboard" : trace.toolSymbol)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(statusColor)
                }
            }
            .frame(width: 16, height: 16)

            if trace.tool == nil, let fileReadOperation {
                HStack(spacing: 4) {
                    Text(String(localized: "Read", table: "Chat"))
                    if let reference = fileOperationReference(fileReadOperation) {
                        Button {
                            openFileReference(reference)
                        } label: {
                            Text(reference.url.lastPathComponent)
                                .foregroundStyle(Color.accentColor)
                                .lineLimit(1)
                        }
                        .buttonStyle(.plain)
                        .help(reference.url.path)
                    } else {
                        Text(URL(fileURLWithPath: fileReadOperation.path).lastPathComponent)
                            .lineLimit(1)
                    }
                    if let lineDescription = fileLineDescription(fileReadOperation) {
                        Text(lineDescription)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                .font(.system(size: 11.5, weight: .semibold))
                .layoutPriority(1)
            } else {
                Text(MacToolPresentationPolicy.displayTitle(for: trace))
                .font(.system(size: 11.5, weight: .semibold))
                .lineLimit(1)
                .truncationMode(.middle)
                .layoutPriority(1)
            }

            if let detail = MacToolTraceDisplayPolicy.headerDetail(for: trace) {
                Text("· \(detail)")
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(0)
            }

            if !(trace.tool == nil && fileReadOperation != nil) { Spacer(minLength: 8) }
            if trace.state == .failed {
                Text(String(localized: "Failed"))
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
            } else if trace.state == .waiting {
                Text(String(localized: "Waiting"))
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.orange)
            }
            if allowsCollapse {
                if trace.tool == nil && fileReadOperation != nil {
                    // File links retain their own action; the remaining trailing
                    // space belongs to the disclosure, not the link.
                    Button(action: toggleDetails) {
                        disclosureChevron.frame(maxWidth: .infinity, alignment: .trailing)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
                    .help(isExpanded ? String(localized: "Collapse Details") : String(localized: "Expand Details"))
                    .accessibilityLabel(isExpanded ? String(localized: "Collapse Details") : String(localized: "Expand Details"))
                } else {
                    disclosureChevron.accessibilityHidden(true)
                }
            }
        }
        .padding(.vertical, 1)
        .contentShape(Rectangle())
        .accessibilityElement(children: .contain)
    }

    private var disclosureChevron: some View {
        Image(systemName: "chevron.down")
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.secondary)
            .rotationEffect(.degrees(isExpanded ? 180 : 0))
            .frame(width: 32, height: 32)
    }

    private func traceDetailBlock(title: String, text: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text(title)
                .font(.system(size: 10.5, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 34, alignment: .leading)
            Text(text)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineSpacing(1)
                .textSelection(.enabled)
        }
        .padding(.vertical, 0)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var fullResult: String? {
        MacToolTraceDisplayPolicy.fullResult(for: trace)
    }

    private var resultIsTruncated: Bool {
        guard let fullResult,
              let preview = MacToolTraceDisplayPolicy.result(for: trace) else {
            return false
        }
        return fullResult != preview
    }

    private var displayedResult: String? {
        guard let fullResult else { return nil }
        guard !showsFullResult else { return fullResult }
        return MacToolTraceDisplayPolicy.result(for: trace)
    }

    private var fileReadOperation: MacToolFileOperation? {
        MacToolTraceDisplayPolicy.fileReadOperation(for: trace)
    }

    private var secondaryFileLinks: [String] {
        guard let files = trace.affectedFiles else { return [] }
        guard let primaryPath = fileReadOperation?.path else { return files }
        let primaryURL = URL(fileURLWithPath: primaryPath).standardizedFileURL
        return files.filter {
            URL(fileURLWithPath: $0).standardizedFileURL != primaryURL
        }
    }

    private func fileOperationReference(_ operation: MacToolFileOperation) -> MacCodeReference? {
        let target = operation.startLine.map { "\(operation.path):\($0)" } ?? operation.path
        return fileReference(for: target)
    }

    private func fileLineDescription(_ operation: MacToolFileOperation) -> String? {
        if let start = operation.startLine, let end = operation.endLine {
            return String(
                format: String(localized: "Lines %lld–%lld", table: "Chat"),
                Int64(start),
                Int64(end)
            )
        }
        if let start = operation.startLine {
            return String(
                format: String(localized: "Line %lld", table: "Chat"),
                Int64(start)
            )
        }
        return nil
    }

    @ViewBuilder
    private func fileLinks(_ paths: [String]) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(paths, id: \.self) { path in
                    if let reference = fileReference(for: path) {
                        Button {
                            openFileReference(reference)
                        } label: {
                            Label(fileLabel(for: reference), systemImage: "doc.text")
                                .font(.system(size: 10.5, weight: .medium))
                                .padding(.horizontal, 8)
                                .padding(.vertical, 5)
                                .background(
                                    Color.accentColor.opacity(0.08),
                                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                                )
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(Color.accentColor)
                        .help(reference.url.path)
                    }
                }
            }
        }
    }

    private func fileReference(for path: String) -> MacCodeReference? {
        let expanded = NSString(
            string: path.trimmingCharacters(in: .whitespacesAndNewlines)
        ).expandingTildeInPath
        let url = expanded.hasPrefix("file:")
            ? URL(string: expanded)
            : URL(fileURLWithPath: expanded)
        return url.flatMap(MacCodeReferenceParser.parse)
    }

    private func fileLabel(for reference: MacCodeReference) -> String {
        guard let target = reference.targetDescription else {
            return reference.url.lastPathComponent
        }
        return "\(reference.url.lastPathComponent):\(target.dropFirst())"
    }

    private var statusColor: Color {
        switch trace.state {
        case .running: return .accentColor
        case .waiting: return .orange
        case .completed: return .green
        case .failed: return .red
        case .cancelled: return .secondary
        case .incomplete: return .secondary
        }
    }
}
