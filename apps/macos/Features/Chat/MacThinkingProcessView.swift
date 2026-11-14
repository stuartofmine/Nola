// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents the ordered reasoning and tool activity for one macOS assistant turn.

import SwiftUI

enum MacThinkingDisclosurePolicy {
    static func showsTimeline(
        isStreaming: Bool,
        hasFailedTrace: Bool,
        hasExplicitDisclosureChoice: Bool,
        isExpanded: Bool
    ) -> Bool {
        // Permission, input and blocking-error surfaces remain outside this
        // disclosure. Streaming and failed internal steps do not open details.
        return hasExplicitDisclosureChoice && isExpanded
    }
}

struct MacThinkingProcessRow: View {
    let traces: [MacAssistantTrace]
    let isStreaming: Bool
    @State private var isExpanded: Bool
    @State private var hasExplicitDisclosureChoice = false
    @State private var historyEndID: UUID?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        traces: [MacAssistantTrace],
        isStreaming: Bool,
        initiallyExpanded: Bool = false
    ) {
        self.traces = traces
        self.isStreaming = isStreaming
        _isExpanded = State(initialValue: initiallyExpanded)
    }

    private var visibleTraces: [MacAssistantTrace] {
        MacAssistantArtifactPresentationPolicy.visibleProcessTraces(traces)
    }

    private var displayedTraceGroups: [MacAssistantTraceGroup] {
        MacAssistantTraceTimelinePolicy.groupedDetailTraces(tracePage.traces)
    }

    private var tracePage: MacAssistantTraceTimelinePolicy.Page {
        MacAssistantTraceTimelinePolicy.page(visibleTraces, endingAt: historyEndID)
    }

    private var hasFailedTrace: Bool {
        visibleTraces.contains(where: { $0.state == .failed })
    }

    private var title: String {
        MacAssistantProcessTitlePolicy.title(
            traces: visibleTraces,
            isStreaming: isStreaming
        )
    }

    private var showsTimeline: Bool {
        MacThinkingDisclosurePolicy.showsTimeline(
            isStreaming: isStreaming,
            hasFailedTrace: hasFailedTrace,
            hasExplicitDisclosureChoice: hasExplicitDisclosureChoice,
            isExpanded: isExpanded
        )
    }

    private var collapsedTitle: String {
        String(localized: "Execution process", table: "Chat")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    let nextValue = !showsTimeline
                    hasExplicitDisclosureChoice = true
                    isExpanded = nextValue
                }
            } label: {
            HStack(alignment: .center, spacing: 8) {
                Group {
                    if isStreaming {
                        MacBreathingStatusLight(diameter: 16)
                    } else {
                        Image(systemName: "list.bullet")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(width: 16, height: 16)

                MacFlowingStatusText(text: Text(showsTimeline ? title : collapsedTitle),
                    isRunning: isStreaming && visibleTraces.last?.state == .running)
                    .font(.system(size: 11.5, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                Spacer(minLength: 8)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 8.5, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .frame(width: 18, height: 18)
                        .rotationEffect(.degrees(showsTimeline ? 180 : 0))
            }
            .contentShape(Rectangle())
            }
            .buttonStyle(MacActionButtonStyle())
            .help(showsTimeline ? String(localized: "Collapse Details") : String(localized: "Expand Details"))
            .accessibilityLabel(title)
            .accessibilityValue(
                showsTimeline
                    ? String(localized: "Expanded")
                    : String(localized: "Collapsed")
            )

            if showsTimeline {
                VStack(alignment: .leading, spacing: 8) {
                    if tracePage.earlierEndID != nil || tracePage.isHistory {
                        HStack {
                            if let earlierEndID = tracePage.earlierEndID {
                                Button(String(localized: "Earlier activity", table: "Chat")) { historyEndID = earlierEndID }
                            }
                            Spacer()
                            if tracePage.isHistory {
                                if let newerEndID = tracePage.newerEndID {
                                    Button(String(localized: "Newer activity", table: "Chat")) { historyEndID = newerEndID }
                                }
                                Button(String(localized: "Latest activity", table: "Chat")) { historyEndID = nil }
                            }
                        }
                        .buttonStyle(MacActionButtonStyle())
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                    }
                    ForEach(displayedTraceGroups) { group in
                        if group.isToolGroup {
                            MacGroupedToolTraceRow(group: group)
                        } else {
                            thinkingStepView(trace: group.representative)
                        }
                    }
                }
                .padding(.leading, 18)
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .animation(nil, value: visibleTraces)
    }

    @ViewBuilder
    private func thinkingStepView(trace: MacAssistantTrace) -> some View {
        if trace.kind == .status, trace.state == .failed {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(.red)
                    .padding(.top, 1)
                VStack(alignment: .leading, spacing: 3) {
                    Text(trace.title)
                        .font(.system(size: 11.5, weight: .semibold))
                        .foregroundStyle(.primary)
                    if !trace.detail.isEmpty {
                        Text(trace.detail)
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.red.opacity(0.065), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .stroke(Color.red.opacity(0.18), lineWidth: 0.5)
            }
        } else if trace.kind == .status, trace.state.isTerminal {
            Text(trace.detail)
                .font(.system(size: 11.5))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                .padding(.vertical, 2)
        } else {
            MacThinkingPhaseRow(
                trace: trace,
                summary: conciseThinkingSummary(for: trace)
            )
        }
    }

    private func conciseThinkingSummary(for trace: MacAssistantTrace) -> String {
        conciseProcessSummary(for: trace)
    }

    private func conciseProcessSummary(for trace: MacAssistantTrace) -> String {
        if trace.kind == .status {
            switch trace.state {
            case .failed:
                return String(localized: "Failed")
            case .cancelled:
                return String(localized: "Cancelled")
            case .incomplete:
                return trace.title
            case .completed:
                return String(localized: "Completed")
            case .waiting:
                return String(localized: "Waiting")
            case .running:
                return trace.title == "Nola"
                    ? String(localized: "Nola is processing")
                    : trace.title
            }
        }
        let summary = MacThinkingActivityFormatter.currentStep(
            from: trace.detail,
            maximumLength: 140
        )
        return summary == String(localized: "Working…")
            ? String(localized: "Thinking")
            : summary
    }

    private func localizedToolTitle(for trace: MacAssistantTrace) -> String {
        let rawTitle = MacToolPresentationPolicy.displayTitle(for: trace)
        let normalized = rawTitle
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")

        if normalized == "skill context" {
            return String(localized: "Skill Context")
        }
        return rawTitle
    }
}

private struct MacGroupedToolTraceRow: View {
    let group: MacAssistantTraceGroup
    @State private var isExpanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var trace: MacAssistantTrace { group.representative }

    private var operationTitle: String {
        MacToolPresentationPolicy.displayTitle(for: trace)
    }

    private var title: String {
        let operation = group.state == .running || group.state == .waiting
            ? "\(String(localized: "Running")) \(operationTitle)"
            : operationTitle
        return group.traces.count > 1 ? "\(operation) · \(group.traces.count)" : operation
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    if group.state == .running {
                        MacBreathingStatusLight(diameter: 16)
                    } else {
                        Image(systemName: trace.toolSymbol)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(group.state == .failed ? Color.red : Color.secondary)
                            .frame(width: 16, height: 16)
                    }
                    MacFlowingStatusText(text: Text(title), isRunning: group.state == .running)
                        .font(.system(size: 11.5, weight: .semibold))
                        .foregroundStyle(group.state == .failed ? Color.red : Color.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 8.5, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .frame(width: 18, height: 18)
                        .rotationEffect(.degrees(isExpanded ? 180 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(MacPressFeedbackButtonStyle())
            .help(isExpanded ? String(localized: "Collapse Details") : String(localized: "Expand Details"))

            if isExpanded {
                VStack(alignment: .leading, spacing: 7) {
                    ForEach(group.traces) { trace in
                        MacTraceRow(
                            trace: trace,
                            initiallyExpanded: false,
                            allowsCollapse: true
                        )
                    }
                }
                .padding(.leading, 18)
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
    }
}

struct MacThinkingPhaseRow: View {
    let trace: MacAssistantTrace
    let summary: String
    @State private var localExpanded: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.journalPresentationOwner) private var journalOwner
    @Environment(\.journalProcessPartIDs) private var processPartIDs

    private var isExpanded: Bool {
        get { trace.externalID.flatMap { journalOwner?.expansions[$0]?.userChoice } ?? localExpanded }
        nonmutating set {
            if let journalOwner, let partID = trace.externalID {
                var choice = journalOwner.expansions[partID] ?? MacJournalActivityExpansion()
                choice.setExpanded(newValue)
                journalOwner.expansions[partID] = choice
            } else { localExpanded = newValue }
        }
    }

    init(trace: MacAssistantTrace, summary: String) {
        self.trace = trace
        self.summary = summary
        _localExpanded = State(initialValue: false)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    isExpanded.toggle()
                    if isExpanded { journalOwner?.retainProcessExpansion(partIDs: trace.externalID.map { [$0] } ?? [], in: processPartIDs) }
                }
            } label: {
                HStack(alignment: .center, spacing: 8) {
                    Circle()
                        .fill(trace.state == .running ? Color.accentColor : Color.secondary.opacity(0.55))
                        .frame(width: 5, height: 5)
                        .frame(width: 16, height: 16)
                    MacFlowingStatusText(text: Text(summary), isRunning: trace.state == .running)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    if !trace.detail.isEmpty {
                        Image(systemName: "chevron.down")
                            .font(.system(size: 8, weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .frame(width: 18, height: 18)
                            .rotationEffect(.degrees(isExpanded ? 180 : 0))
                    }
                }
            }
            .buttonStyle(MacPressFeedbackButtonStyle())
            .disabled(trace.detail.isEmpty)
            .help(isExpanded ? String(localized: "Collapse Details") : String(localized: "Expand Details"))

            if isExpanded, !trace.detail.isEmpty {
                Text(trace.detail)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineSpacing(2)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, 24)
                    .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
