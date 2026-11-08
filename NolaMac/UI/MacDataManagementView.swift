// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides native backup, restore, size, and application-data clearing controls.

import AppKit
import SwiftUI
import UniformTypeIdentifiers

private func macDataText(_ key: String, _ fallback: String) -> String {
    let keyedValue = NSLocalizedString(key, value: key, comment: "")
    if keyedValue != key {
        return keyedValue
    }
    return NSLocalizedString(fallback, value: fallback, comment: "")
}

struct MacDataManagementView: View {
    private struct PendingRestore: Identifiable {
        let url: URL
        let preview: MacDataBackupPreview
        var id: String { url.absoluteString }
    }

    @State private var service = MacDataManagementService()
    @State private var showsImporter = false
    @State private var confirmsClear = false
    @State private var pendingRestore: PendingRestore?
    @State private var isRefreshing = false
    @State private var isExporting = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var operationInProgress: Bool {
        service.isWorking || isRefreshing || isExporting
    }

    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader(LocalizedStringKey(macDataText("mac.data.management.title", "Data Management")), symbol: "externaldrive.badge.timemachine") {
                Text(macDataText("mac.data.management.subtitle", "Back up or reset data owned by Nola for Mac"))
            } actions: {
                if operationInProgress {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityLabel(macDataText("mac.data.working", "Working"))
                }
                Button {
                    refreshData()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .macToolbarIcon()
                .disabled(operationInProgress)
                .help(macDataText("mac.data.refresh", "Refresh"))
                .accessibilityLabel(macDataText("mac.data.refresh", "Refresh"))
            }

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    MacPageMetricsBar(
                        metrics: dataMetrics,
                        isWorking: operationInProgress
                    )

                    if let errorMessage = service.errorMessage {
                        MacInlineNotice(
                            title: macDataText("mac.data.error.title", "Data Management Error"),
                            message: errorMessage,
                            kind: .failure,
                            dismiss: { service.errorMessage = nil }
                        )
                        .transition(.opacity.combined(with: .move(edge: .top)))
                    }

                    managementSection(
                        title: macDataText("mac.data.backup.title", "Backup and Restore"),
                        description: macDataText("mac.data.backup.detail", "A JSON backup includes conversations, activity, automations, notifications, web sources, prompt templates, model parameters, and non-credential preferences."),
                        symbol: "arrow.triangle.2.circlepath",
                        tint: MacDesignColor.brand
                    ) {
                        ViewThatFits(in: .horizontal) {
                            HStack {
                                backupButtons
                            }
                            VStack(alignment: .leading, spacing: MacDesignTokens.Spacing.sm) {
                                backupButtons
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }

                    managementSection(
                        title: macDataText("mac.data.reset.title", "Reset"),
                        description: macDataText(
                            "mac.data.reset.description",
                            "Clears Nola for Mac conversations, activity, automations, preferences, connection settings and credentials, workspace bookmark, and app logs. Independent CLI configuration and sessions are preserved."
                        ),
                        symbol: "exclamationmark.triangle.fill",
                        tint: .red
                    ) {
                        Button(macDataText("mac.data.clear.action", "Clear Application Data…"), role: .destructive) {
                            confirmsClear = true
                        }
                        .buttonStyle(.bordered)
                        .tint(.red)
                        .disabled(operationInProgress)
                    }

                    if let statusMessage = service.statusMessage {
                        MacInlineNotice(message: statusMessage, kind: .success, dismiss: {
                            service.statusMessage = nil
                        })
                        .transition(.opacity.combined(with: .move(edge: .top)))
                    }
                }
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.xl)
                .frame(maxWidth: MacDesignTokens.Layout.contentWidth)
                .frame(maxWidth: .infinity)
            }
        }
        .background(MacPageBackground())
        .task { refreshData() }
        .fileImporter(isPresented: $showsImporter, allowedContentTypes: [.json], allowsMultipleSelection: false) { result in
            guard case .success(let urls) = result, let url = urls.first else {
                if case .failure(let error) = result { service.errorMessage = error.localizedDescription }
                return
            }
            Task { @MainActor in
                do {
                    let preview = try await service.previewBackup(from: url)
                    pendingRestore = PendingRestore(url: url, preview: preview)
                } catch {
                    service.errorMessage = error.localizedDescription
                }
            }
        }
        .confirmationDialog(
            macDataText("mac.data.clear.title", "Clear Application Data?"),
            isPresented: $confirmsClear,
            titleVisibility: .visible
        ) {
            Button(macDataText("mac.data.cancel", "Cancel"), role: .cancel) {}
            Button(macDataText("mac.data.clear.confirm", "Clear"), role: .destructive) { Task { await service.clearApplicationData() } }
        } message: {
            Text(macDataText("mac.data.clear.detail", "This cannot be undone. Export a backup first if you may need this data later."))
        }
        .sheet(item: $pendingRestore) { pending in
            restorePreview(pending)
        }
        .animation(
            reduceMotion ? nil : .easeOut(duration: 0.18),
            value: service.statusMessage
        )
        .animation(
            reduceMotion ? nil : .easeOut(duration: 0.18),
            value: service.errorMessage
        )
    }

    private func exportBackup() {
        guard !operationInProgress else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Nola-Mac-Backup.json"
        panel.allowedContentTypes = [.json]
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            Task { @MainActor in
                isExporting = true
                defer { isExporting = false }
                do {
                    try await service.exportBackup(to: url)
                    service.statusMessage = macDataText("mac.data.status.exported", "Backup exported")
                } catch {
                    service.errorMessage = error.localizedDescription
                }
            }
        }
    }

    @ViewBuilder
    private var backupButtons: some View {
        Button(action: exportBackup) {
            Label(macDataText("mac.data.export", "Export Backup"), systemImage: "square.and.arrow.up")
        }
        .buttonStyle(.borderedProminent)
        .disabled(operationInProgress)
        Button {
            showsImporter = true
        } label: {
            Label(macDataText("mac.data.restore", "Restore Backup"), systemImage: "square.and.arrow.down")
        }
        .buttonStyle(.bordered)
        .tint(.orange)
        .disabled(operationInProgress)
        .accessibilityIdentifier("mac.data.restoreBackup")
    }

    private var dataMetrics: [MacPageMetric] {
        [
            MacPageMetric(id: "data-conversations", LocalizedStringKey(macDataText("mac.data.metric.conversations", "Conversations")), value: "\(service.conversationCount)", symbol: "bubble.left.and.bubble.right", tint: .blue),
            MacPageMetric(id: "data-activity", LocalizedStringKey(macDataText("mac.data.metric.activity", "Activity")), value: "\(service.activityCount)", symbol: "waveform.path.ecg", tint: .cyan),
            MacPageMetric(id: "data-automations", LocalizedStringKey(macDataText("mac.data.metric.automations", "Automations")), value: "\(service.automationCount)", symbol: "clock.badge.checkmark", tint: .green),
            MacPageMetric(id: "data-notifications", LocalizedStringKey(macDataText("mac.data.metric.notifications", "Notifications")), value: "\(service.notificationCount)", symbol: "bell", tint: .orange),
            MacPageMetric(id: "data-web-sources", LocalizedStringKey(macDataText("mac.data.metric.web_sources", "Web Sources")), value: "\(service.webSourceCount)", symbol: "network", tint: .indigo),
            MacPageMetric(id: "data-local-size", LocalizedStringKey(macDataText("mac.data.metric.local_data", "Local Data")), value: ByteCountFormatter.string(fromByteCount: service.dataSize, countStyle: .file), symbol: "internaldrive", tint: .purple),
        ]
    }

    private func managementSection<Content: View>(
        title: String,
        description: String,
        symbol: String,
        tint: Color,
        @ViewBuilder content: () -> Content
    ) -> some View {
        HStack(alignment: .top, spacing: 14) {
            MacIconTile(symbol: symbol, tint: tint)

            VStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(title)
                        .font(.headline)
                    Text(description)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Divider().opacity(0.55)
                content()
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
    }

    private func refreshData() {
        guard !operationInProgress else { return }
        isRefreshing = true
        Task { @MainActor in
            await service.refresh()
            isRefreshing = false
        }
    }

    private func restoreSummary(_ pending: PendingRestore) -> String {
        let format = macDataText(
            "mac.data.restore.summary.format",
            "Backup from %@ · %lld conversations · %lld activities · %lld automations · %lld notifications · %lld web sources. Restoring replaces these local collections and non-credential preferences; existing saved credentials are kept."
        )
        return String(
            format: format,
            pending.preview.exportedAt.formatted(date: .abbreviated, time: .shortened),
            Int64(pending.preview.conversationCount),
            Int64(pending.preview.activityCount),
            Int64(pending.preview.automationCount),
            Int64(pending.preview.notificationCount),
            Int64(pending.preview.webSourceCount)
        )
    }

    private func restorePreview(_ pending: PendingRestore) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: "arrow.counterclockwise.circle.fill")
                    .font(.system(size: 28, weight: .semibold))
                    .foregroundStyle(.orange)
                    .frame(width: 46, height: 46)
                    .background(.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

                VStack(alignment: .leading, spacing: 5) {
                    Text(macDataText("mac.data.restore.confirm.title", "Restore This Backup?"))
                        .font(.title3.weight(.semibold))
                    Text(restoreSummary(pending))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            Divider()

            HStack(spacing: 10) {
                Spacer(minLength: 0)
                Button(macDataText("mac.data.cancel", "Cancel")) {
                    pendingRestore = nil
                }
                .keyboardShortcut(.cancelAction)

                Button(role: .destructive) {
                    let url = pending.url
                    pendingRestore = nil
                    Task { await service.importBackup(from: url) }
                } label: {
                    Label(macDataText("mac.data.restore.action", "Restore"), systemImage: "arrow.counterclockwise")
                }
                .buttonStyle(.borderedProminent)
                .tint(.red)
                .accessibilityIdentifier("mac.data.confirmRestore")
            }
        }
        .padding(24)
        .frame(minWidth: 420, idealWidth: 520, maxWidth: 640)
        .background(MacPageBackground())
        .interactiveDismissDisabled(service.isWorking)
    }
}
