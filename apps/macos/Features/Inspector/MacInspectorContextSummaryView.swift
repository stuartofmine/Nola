// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents context-window and prompt-cache metrics for one conversation.

import AppKit
import Observation
import SwiftUI

struct MacInspectorContextSummaryView: View {
    @Bindable var model: MacWorkspaceModel

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                Image(systemName: "gauge.with.dots.needle.50percent")
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(String(localized: "inspector.context_usage", table: "ConversationInspector"))
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .semibold))
                Spacer()
                Text(model.contextUsageText)
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .semibold, design: .monospaced))
                    .monospacedDigit()
                    .foregroundStyle(.primary)
                    .contentTransition(.numericText())
            }

            if model.contextUsageIsAvailable {
                ProgressView(value: model.contextUsageFraction)
                    .controlSize(.small)
                    .tint(MacDesignColor.brand)
                Text(model.contextUsageIsEstimated
                    ? String(localized: "Estimated next request", table: "Permissions")
                    : String(localized: "Measured request context", table: "Permissions"))
                    .font(.caption)
                    .foregroundStyle(.secondary)

            }

            HStack(spacing: 6) {
                if MacExecutionBackendPolicy.requiresNolaModel(model.executionBackend.id) {
                    Text(String(localized: "Window"))
                    Spacer(minLength: 8)
                    Text(
                        model.contextUsageIsAvailable
                            ? model.contextWindowTokens.formatted()
                            : "?"
                    )
                        .fontDesign(.monospaced)
                } else {
                    Text(model.executionBackend.displayName)
                        .lineLimit(1)
                    Text("·")
                    Text(String(localized: "Execution Unit"))
                    Spacer(minLength: 8)
                }
            }
            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium))
            .foregroundStyle(.secondary)

            HStack {
                Text(String(localized: "Cache hit rate (conversation)", table: "Chat"))
                Spacer()
                Text(model.promptCacheUsage?.rateText ?? String(localized: "Cache rate unknown", table: "Chat"))
                    .monospacedDigit()
            }
            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium))
            .foregroundStyle(.secondary)
            if let usage = model.promptCacheUsage {
                HStack {
                    if let read = usage.read_tokens, let input = usage.input_tokens {
                        Text("\(read.formatted()) / \(input.formatted()) tokens").monospacedDigit()
                    }
                    Spacer()
                }
                .font(.system(size: MacDesignTokens.Typography.inspectorCaption))
                .foregroundStyle(.tertiary)
                if usage.unreported_requests > 0 {
                    Text(String(localized: "Based on reported requests; some requests did not report cache usage.", table: "Chat"))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            if model.contextMeasurement != nil || model.promptCacheStatusText != nil {
                DisclosureGroup(String(localized: "Details")) {
                    if let measurement = model.contextMeasurement {
                        Text("\(String(localized: "Request", table: "Permissions")): \(measurement.requestID)")
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let cacheStatus = model.promptCacheStatusText {
                        Divider().opacity(0.45)
                        HStack(spacing: 7) {
                            Image(systemName: "bolt.horizontal.circle")
                                .foregroundStyle(MacDesignColor.brand)
                            Text(String(localized: "Prompt cache", table: "Chat"))
                            Spacer(minLength: 8)
                            Text(cacheStatus)
                                .foregroundStyle(.secondary)
                        }
                        .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium))

                        if let measurement = model.promptCacheMeasurement {
                            Text("\(String(localized: "Request", table: "Permissions")): \(measurement.requestID)")
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                            if let attempt = measurement.attemptID {
                                Text("\(String(localized: "Attempt", table: "Permissions")): \(attempt)")
                                    .font(.caption.monospaced())
                                    .foregroundStyle(.secondary)
                                    .textSelection(.enabled)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }

                        if model.promptCacheReadTokens != nil || model.promptCacheWriteTokens != nil {
                            HStack(spacing: 12) {
                                if let readTokens = model.promptCacheReadTokens {
                                    Label(readTokens.formatted(), systemImage: "arrow.down")
                                        .accessibilityLabel(String(localized: "Cache read", table: "Chat"))
                                }
                                if let writeTokens = model.promptCacheWriteTokens {
                                    Label(writeTokens.formatted(), systemImage: "arrow.up")
                                        .accessibilityLabel(String(localized: "Cache write", table: "Chat"))
                                }
                                Spacer()
                            }
                            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium, design: .monospaced))
                            .foregroundStyle(.tertiary)
                        }

                        if !model.promptCacheFragmentChanges.isEmpty {
                            VStack(alignment: .leading, spacing: 5) {
                                ForEach(
                                    Array(model.promptCacheFragmentChanges.prefix(4).enumerated()),
                                    id: \.offset
                                ) { _, change in
                                    HStack(spacing: 8) {
                                        Text(change.fragmentId)
                                            .lineLimit(1)
                                            .truncationMode(.middle)
                                        Spacer(minLength: 8)
                                        Text(promptCacheChangeLabel(change.kind))
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                if model.promptCacheFragmentChanges.count > 4 {
                                    Text(
                                        String(
                                            format: String(localized: "%lld more changes", table: "Chat"),
                                            Int64(model.promptCacheFragmentChanges.count - 4)
                                        )
                                    )
                                    .foregroundStyle(.tertiary)
                                }
                            }
                            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium, design: .monospaced))
                        }
                    }
                }
                .disclosureGroupStyle(MacRowDisclosureStyle(toggleIdentifier: "mac-inspector-context-details", isPlain: true))
            }
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(String(localized: "Context"))
        .accessibilityValue(model.contextUsageText)
        .accessibilityIdentifier("mac-inspector-context-summary")
    }

    private func promptCacheChangeLabel(_ kind: String) -> String {
        switch kind {
        case "added": return String(localized: "Added", table: "Chat")
        case "removed": return String(localized: "Removed", table: "Chat")
        default: return String(localized: "Changed", table: "Chat")
        }
    }
}
