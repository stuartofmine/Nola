// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents conversation-scoped data-source access and override commands.

import AppKit
import Observation
import SwiftUI

struct MacInspectorDataSourcesView: View {
    @Bindable var model: MacWorkspaceModel
    let dataRegistry: MacDataSourceRegistry
    let platformData: MacPlatformDataService
    let webSources: MacWebSourceStore
    @Environment(\.openWindow) private var openWindow
    private struct DisplayKey: Equatable {
        let conversationID: String?
        let sourceIDs: [String]
        let revision: UInt64
        let activation: UInt64
    }
    @State private var loadedKey: DisplayKey?
    @State private var displayedOverrides: [String: Bool] = [:]
    @State private var readError: String?
    @State private var activation: UInt64 = 0

    private var displayKey: DisplayKey {
        DisplayKey(conversationID: model.selectedConversationID,
            sourceIDs: platformData.snapshots.filter { $0.kind.supportsChatContext }.map(\.sourceID)
                + webSources.sources.map(\.dataSourceID), revision: dataRegistry.overrideDisplayRevision,
            activation: activation)
    }

    var body: some View {
        let key = displayKey
        MacInspectorSection(title: String(localized: "Data Sources", table: "Chat"), showsDivider: false) {
            HStack(alignment: .firstTextBaseline) {
                Text(String(localized: "Available in This Conversation", table: "Chat"))
                    .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium))
                    .foregroundStyle(.secondary)
                Spacer()
                Button {
                    openWindow(id: "data-center")
                } label: {
                    Label(String(localized: "Manage", table: "Chat"), systemImage: "arrow.up.right")
                        .labelStyle(.titleAndIcon)
                        .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .medium))
                }
                .buttonStyle(MacPressFeedbackButtonStyle())
                .foregroundStyle(.tint)
            }

            if let readError {
                VStack(alignment: .leading, spacing: 6) {
                    Label(readError, systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(String(localized: "Retry")) { activation &+= 1 }
                        .buttonStyle(MacPressFeedbackButtonStyle())
                }
                .accessibilityIdentifier("mac-inspector-data-source-error")
            } else if loadedKey != key {
                ProgressView().controlSize(.small)
                    .accessibilityLabel(String(localized: "Loading"))
            }
            if let persistenceError = dataRegistry.persistenceError {
                Label(persistenceError, systemImage: "exclamationmark.triangle")
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("mac-inspector-data-source-save-error")
            }
            VStack(spacing: 0) {
                ForEach(platformData.snapshots.filter { $0.kind.supportsChatContext }) { snapshot in
                    conversationSourceRow(
                        sourceID: snapshot.sourceID,
                        title: snapshot.title,
                        symbol: snapshot.symbol,
                        fallback: .conversationOptIn,
                        isAvailable: snapshot.authorization == .available
                    )
                    if snapshot.id != platformData.snapshots.filter({ $0.kind.supportsChatContext }).last?.id
                        || !webSources.sources.isEmpty {
                        Divider().padding(.leading, 30)
                    }
                }
                ForEach(webSources.sources) { source in
                    conversationSourceRow(
                        sourceID: source.dataSourceID,
                        title: source.name,
                        symbol: "network",
                        fallback: source.isEnabled ? .defaultEnabled : .conversationOptIn,
                        isAvailable: source.isEnabled && source.lastError == nil && !source.cachedText.isEmpty
                    )
                    if source.id != webSources.sources.last?.id {
                        Divider().padding(.leading, 30)
                    }
                }
            }
            .background(
                Color.primary.opacity(0.026),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
            )

            Text(String(localized: "Changes here apply only to this conversation. Global access and validation live in Data Center.", table: "Chat"))
                .font(.system(size: MacDesignTokens.Typography.inspectorCaption))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .task(id: key) {
            loadedKey = nil
            readError = nil
            do {
                let values = try await dataRegistry.loadOverridesForDisplay(
                    conversationID: key.conversationID, sourceIDs: key.sourceIDs)
                guard !Task.isCancelled else { return }
                displayedOverrides = values
                loadedKey = key
            } catch {
                guard !Task.isCancelled else { return }
                readError = error.localizedDescription
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            activation &+= 1
        }
    }

    private func conversationSourceRow(
        sourceID: String,
        title: String,
        symbol: String,
        fallback: MacDataSourceAccessPolicy,
        isAvailable: Bool
    ) -> some View {
        let conversationID = model.selectedConversationID
        let policy = dataRegistry.policy(for: sourceID, fallback: fallback)
        let isLoaded = loadedKey == displayKey
        let override = isLoaded ? displayedOverrides[sourceID] : nil
        let binding = Binding(
            get: {
                dataRegistry.displayResolution(
                    sourceID: sourceID,
                    override: override,
                    fallback: fallback
                )?.enabled == true
            },
            set: { enabled in
                guard let conversationID else { return }
                dataRegistry.setConversationOverride(
                    enabled,
                    sourceID: sourceID,
                    conversationID: conversationID
                )
            }
        )

        return HStack(spacing: 9) {
            Image(systemName: symbol)
                .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .semibold))
                .foregroundStyle(isAvailable ? Color.accentColor : Color.secondary)
                .frame(width: 26, height: 26)
                .background(
                    (isAvailable ? Color.accentColor : Color.primary).opacity(isAvailable ? 0.075 : 0.028),
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .medium))
                    .lineLimit(1)
                Text(sourcePolicyTitle(policy))
                    .font(.system(size: MacDesignTokens.Typography.inspectorDetail))
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            if override != nil, let conversationID {
                Button {
                    dataRegistry.setConversationOverride(
                        nil,
                        sourceID: sourceID,
                        conversationID: conversationID
                    )
                } label: {
                    Image(systemName: "arrow.uturn.backward.circle")
                        .font(.system(size: MacDesignTokens.Typography.inspectorDetail))
                }
                .buttonStyle(MacPressFeedbackButtonStyle())
                .foregroundStyle(.secondary)
                .help(String(localized: "Use Global Setting"))
                .accessibilityLabel("\(title): \(String(localized: "Use Global Setting"))")
            }
            Toggle(title, isOn: binding)
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.mini)
                .frame(width: 38, height: 20, alignment: .trailing)
                .disabled(!isLoaded || conversationID == nil || policy == .blocked || !isAvailable)
                .help(readError ?? title)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
    }

    private func sourcePolicyTitle(_ policy: MacDataSourceAccessPolicy) -> String {
        switch policy {
        case .defaultEnabled: return String(localized: "Default On")
        case .conversationOptIn: return String(localized: "Optional")
        case .askWhenRelevant: return String(localized: "Ask When Relevant")
        case .blocked: return String(localized: "Blocked")
        }
    }
}
