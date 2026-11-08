// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LogViewerView logic in the iOS UI layer.
// - Interfaces:
//   - LogViewerView
//   - LogFileStore
// - APIs:
//   - none
//
//
//  LogViewerView.swift
//  Nola
//

import SwiftUI

struct LogViewerView: View {
    @State private var logText: String = ""
    @State private var fileSizeBytes: Int64 = 0
    @State private var isLoading: Bool = false
    @State private var showingShareSheet: Bool = false
    @State private var confirmsClear = false
    @State private var searchText = ""

    private var displayedLogText: String {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return logText }
        return logText
            .split(whereSeparator: \Character.isNewline)
            .filter { $0.localizedCaseInsensitiveContains(query) }
            .joined(separator: "\n")
    }

    var body: some View {
        VStack(spacing: 0) {
            if logText.isEmpty {
                VStack(spacing: 10) {
                    Image(systemName: "doc.text.magnifyingglass")
                        .font(.system(size: 34))
                        .foregroundColor(.secondary)
                    Text(L10n.settingsLogsEmptyTitle)
                        .font(.headline)
                    Text(L10n.settingsLogsEmptyDescription)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(24)
            } else if displayedLogText.isEmpty {
                ContentUnavailableView.search(text: searchText)
            } else {
                ScrollView {
                    Text(displayedLogText)
                        .font(.caption)
                        .monospaced()
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding()
                }
            }
        }
        .navigationTitle(L10n.settingsLogsTitle)
        .searchable(
            text: $searchText,
            prompt: Text(NSLocalizedString("settings.logs.search", value: "Search logs", comment: "Search log entries"))
        )
        .toolbar {
            ToolbarItemGroup(placement: .navigationBarTrailing) {
                Button {
                    showingShareSheet = true
                } label: {
                    Image(systemName: "square.and.arrow.up")
                }
                .disabled(!LogFileStore.sharedFileExists)
                .accessibilityLabel(NSLocalizedString("settings.logs.share", value: "Share logs", comment: "Share logs action"))

                Button(role: .destructive) {
                    confirmsClear = true
                } label: {
                    Image(systemName: "trash")
                }
                .disabled(logText.isEmpty)
                .accessibilityLabel(NSLocalizedString("settings.logs.clear", value: "Clear logs", comment: "Clear logs action"))
            }
        }
        .safeAreaInset(edge: .bottom) {
            HStack {
                Text("settings.logs.footer.size_format".localized(DisplayFormatters.byteSize(fileSizeBytes)))
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer()
                Button {
                    Task { await reload() }
                } label: {
                    HStack(spacing: 7) {
                        if isLoading {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Image(systemName: "arrow.clockwise")
                        }
                        Text(L10n.settingsLogsReload)
                            .font(.callout.weight(.medium))
                    }
                    .frame(minHeight: 44)
                }
                .disabled(isLoading)
            }
            .padding(.horizontal)
            .padding(.vertical, 8)
            .background(.ultraThinMaterial)
        }
        .task {
            await reload()
        }
        .sheet(isPresented: $showingShareSheet) {
            let url = LogFileStore.shared.logFileURL()
            ShareSheet(items: [url])
        }
        .confirmationDialog(
            NSLocalizedString("settings.logs.clear.confirm.title", value: "Clear all logs?", comment: "Clear logs confirmation title"),
            isPresented: $confirmsClear,
            titleVisibility: .visible
        ) {
            Button(NSLocalizedString("settings.logs.clear", value: "Clear logs", comment: "Clear logs action"), role: .destructive) {
                Task {
                    await LogFileStore.shared.clear()
                    searchText = ""
                    await reload()
                }
            }
            Button(L10n.cancel, role: .cancel) { }
        } message: {
            Text(NSLocalizedString("settings.logs.clear.confirm.message", value: "This permanently removes the local diagnostic log file.", comment: "Clear logs confirmation message"))
        }
    }

    private func reload() async {
        await MainActor.run { isLoading = true }
        let text = await LogFileStore.shared.readLastLines(maxLines: 100, maxBytes: 250_000)
        let size = await LogFileStore.shared.currentFileSizeBytes()
        await MainActor.run {
            logText = text
            fileSizeBytes = size
            isLoading = false
        }
    }

}

private extension LogFileStore {
    static var sharedFileExists: Bool {
        FileManager.default.fileExists(atPath: LogFileStore.shared.logFileURL().path)
    }
}

#Preview {
    NavigationStack {
        LogViewerView()
    }
}
