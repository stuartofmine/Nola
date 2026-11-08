// SPDX-License-Identifier: MIT
// In-app diagnostics and privacy-safe live log viewer for the macOS client.

import AppKit
import Combine
import Foundation
import SwiftUI
import UniformTypeIdentifiers

enum MacDiagnosticLogLevel: String, CaseIterable, Identifiable, Sendable {
    case error = "ERROR"
    case warning = "WARN"
    case info = "INFO"
    case debug = "DEBUG"
    case trace = "TRACE"
    case other = "OTHER"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .error: String(localized: "Errors")
        case .warning: String(localized: "Warnings")
        case .info: String(localized: "Info")
        case .debug: String(localized: "Debug")
        case .trace: String(localized: "Trace")
        case .other: String(localized: "Other")
        }
    }

    var tint: Color {
        switch self {
        case .error: .red
        case .warning: .orange
        case .info: .blue
        case .debug: .secondary
        case .trace: .secondary.opacity(0.72)
        case .other: .secondary
        }
    }
}

struct MacDiagnosticLogEntry: Identifiable, Hashable, Sendable {
    let id: Int
    let timestamp: String
    let level: MacDiagnosticLogLevel
    let module: String
    let message: String
    let rawLine: String
}

enum MacDiagnosticLogParser {
    nonisolated static func parse(_ text: String) -> [MacDiagnosticLogEntry] {
        text
            .split(whereSeparator: \Character.isNewline)
            .enumerated()
            .map { offset, line in
                parseLine(String(line), id: offset)
            }
    }

    nonisolated static func filter(
        _ entries: [MacDiagnosticLogEntry],
        level: MacDiagnosticLogLevel?,
        module: String,
        query: String
    ) -> [MacDiagnosticLogEntry] {
        let normalizedQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return entries.filter { entry in
            guard level == nil || entry.level == level else { return false }
            guard module.isEmpty || entry.module == module else { return false }
            guard !normalizedQuery.isEmpty else { return true }
            return entry.rawLine.localizedCaseInsensitiveContains(normalizedQuery)
        }
    }

    private nonisolated static func parseLine(_ line: String, id: Int) -> MacDiagnosticLogEntry {
        let pattern = #"^\[([^\]]+)\]\[([^\]]+)\]\[([^\]]+)\]\s?(.*)$"#
        guard let regex = try? NSRegularExpression(pattern: pattern),
              let match = regex.firstMatch(in: line, range: NSRange(line.startIndex..., in: line)),
              match.numberOfRanges == 5,
              let timestampRange = Range(match.range(at: 1), in: line),
              let levelRange = Range(match.range(at: 2), in: line),
              let moduleRange = Range(match.range(at: 3), in: line),
              let messageRange = Range(match.range(at: 4), in: line) else {
            return MacDiagnosticLogEntry(
                id: id,
                timestamp: "",
                level: .other,
                module: "app",
                message: line,
                rawLine: line
            )
        }

        let rawLevel = String(line[levelRange]).uppercased()
        let level: MacDiagnosticLogLevel
        switch rawLevel {
        case "ERROR": level = .error
        case "WARN", "WARNING": level = .warning
        case "INFO": level = .info
        case "DEBUG": level = .debug
        case "TRACE": level = .trace
        default: level = .other
        }

        return MacDiagnosticLogEntry(
            id: id,
            timestamp: String(line[timestampRange]),
            level: level,
            module: String(line[moduleRange]),
            message: String(line[messageRange]),
            rawLine: line
        )
    }
}

enum MacDiagnosticLogSanitizer {
    nonisolated static func sanitize(_ input: String) -> String {
        var value = input
        let replacements: [(String, String)] = [
            (#"(?i)bearer\s+[A-Za-z0-9._~+/=-]+"#, "Bearer <redacted>"),
            (#"(?i)(api[_ -]?key|access[_ -]?token|refresh[_ -]?token|authorization|cookie|prompt|response|thinking|content|preview|delta|payload|body)\s*(?:\([^)]*\))?\s*[:=]\s*[^,}\]\n]+"#, "$1=<redacted>"),
            (#"(?i)https?://([^/\s?#]+)[^\s,}\]]*"#, "https://$1/<redacted>"),
            (#"(?<![A-Za-z0-9])/(?:Users|private|var|tmp|Volumes)/[^\s,}\]]+"#, "<path-redacted>")
        ]
        for (pattern, template) in replacements {
            guard let regex = try? NSRegularExpression(pattern: pattern) else { continue }
            let range = NSRange(value.startIndex..., in: value)
            value = regex.stringByReplacingMatches(in: value, range: range, withTemplate: template)
        }
        return value
    }
}

struct MacDiagnosticLogFileStatus: Equatable, Sendable {
    let fileName: String
    let fileExists: Bool
    let sizeBytes: Int64
    let lastModifiedAt: Date?
    let isWritable: Bool
    let errorDescription: String?
}

struct MacDiagnosticLogSnapshot: Sendable {
    let entries: [MacDiagnosticLogEntry]
    let status: MacDiagnosticLogFileStatus
}

struct MacDiagnosticLogSource: Sendable {
    private let directoryURL: URL
    let logFileURL: URL
    private let maxBytes: Int
    private let maxLines: Int

    nonisolated init(baseDirectory: URL? = nil, maxBytes: Int = 2_000_000, maxLines: Int = 2_000) {
        let root = baseDirectory ?? MacAppEnvironment.shared.supportRoot
        directoryURL = root.appendingPathComponent("Logs", isDirectory: true)
        logFileURL = directoryURL.appendingPathComponent("nola.log", isDirectory: false)
        self.maxBytes = maxBytes
        self.maxLines = maxLines
    }

    nonisolated func load() -> MacDiagnosticLogSnapshot {
        let fileManager = FileManager.default
        do {
            try fileManager.createDirectory(at: directoryURL, withIntermediateDirectories: true)
            let exists = fileManager.fileExists(atPath: logFileURL.path)
            let attributes = exists ? try fileManager.attributesOfItem(atPath: logFileURL.path) : [:]
            let size = (attributes[.size] as? NSNumber)?.int64Value ?? 0
            let modified = attributes[.modificationDate] as? Date
            let writable = exists
                ? fileManager.isWritableFile(atPath: logFileURL.path)
                : fileManager.isWritableFile(atPath: directoryURL.path)
            let text = exists ? try readTail() : ""
            return MacDiagnosticLogSnapshot(
                entries: MacDiagnosticLogParser.parse(MacDiagnosticLogSanitizer.sanitize(text)),
                status: MacDiagnosticLogFileStatus(
                    fileName: logFileURL.lastPathComponent,
                    fileExists: exists,
                    sizeBytes: size,
                    lastModifiedAt: modified,
                    isWritable: writable,
                    errorDescription: writable ? nil : String(localized: "Write failed")
                )
            )
        } catch {
            return MacDiagnosticLogSnapshot(
                entries: [],
                status: MacDiagnosticLogFileStatus(
                    fileName: logFileURL.lastPathComponent,
                    fileExists: false,
                    sizeBytes: 0,
                    lastModifiedAt: nil,
                    isWritable: false,
                    errorDescription: MacDiagnosticLogSanitizer.sanitize(error.localizedDescription)
                )
            )
        }
    }

    nonisolated func clear() throws {
        let fileManager = FileManager.default
        if fileManager.fileExists(atPath: logFileURL.path) {
            try fileManager.removeItem(at: logFileURL)
        }
    }

    private nonisolated func readTail() throws -> String {
        let handle = try FileHandle(forReadingFrom: logFileURL)
        defer { try? handle.close() }
        let size = try handle.seekToEnd()
        let offset = size > UInt64(maxBytes) ? size - UInt64(maxBytes) : 0
        try handle.seek(toOffset: offset)
        let data = try handle.readToEnd() ?? Data()
        var text = String(decoding: data, as: UTF8.self)
        if offset > 0, let firstNewline = text.firstIndex(of: "\n") {
            text.removeSubrange(...firstNewline)
        }
        return text.split(whereSeparator: \Character.isNewline).suffix(maxLines).joined(separator: "\n")
    }
}

@MainActor
final class MacDiagnosticsLogViewerModel: ObservableObject {
    @Published private(set) var entries: [MacDiagnosticLogEntry] = []
    @Published private(set) var status: MacDiagnosticLogFileStatus?
    @Published private(set) var lastRefreshAt: Date?
    @Published private(set) var revision = 0
    @Published private(set) var sourceError: String?
    @Published var operationError: String?

    let source: MacDiagnosticLogSource

    init(source: MacDiagnosticLogSource = MacDiagnosticLogSource()) {
        self.source = source
    }

    func refresh() async {
        let source = source
        let snapshot = await Task.detached(priority: .utility) {
            source.load()
        }.value
        let contentChanged = entries != snapshot.entries || status != snapshot.status
        entries = snapshot.entries
        status = snapshot.status
        lastRefreshAt = Date()
        sourceError = snapshot.status.errorDescription
        if contentChanged {
            revision &+= 1
        }
    }

    func clear() async {
        do {
            let source = source
            try await Task.detached(priority: .utility) {
                try source.clear()
            }.value
            Logger.shared.clear()
            operationError = nil
        } catch {
            operationError = MacDiagnosticLogSanitizer.sanitize(error.localizedDescription)
        }
        await refresh()
    }
}

struct MacDiagnosticsCenterView: View {
    @StateObject private var model = MacDiagnosticsLogViewerModel()
    @AppStorage(MacDebugLoggingConfiguration.enabledKey) private var detailedLoggingEnabled = false
    @AppStorage(MacDebugLoggingConfiguration.levelKey) private var configuredLevel = "debug"
    @State private var selectedLevel: MacDiagnosticLogLevel?
    @State private var selectedModule = ""
    @State private var query = ""
    @State private var isPaused = false
    @State private var followsLatest = true
    @State private var confirmsClear = false
    @State private var transientNotice: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var filteredEntries: [MacDiagnosticLogEntry] {
        MacDiagnosticLogParser.filter(
            model.entries,
            level: selectedLevel,
            module: selectedModule,
            query: query
        )
    }

    private var availableModules: [String] {
        Array(Set(model.entries.map(\.module))).sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    var body: some View {
        VStack(spacing: 0) {
            MacPageHeader(
                LocalizedStringKey(String(localized: "Runtime Diagnostics")),
                symbol: "waveform.path.ecg"
            ) {
                Text(String(localized: "Runtime status, logs, and support export."))
            } actions: {
                Button {
                    isPaused.toggle()
                    if !isPaused {
                        Task { await model.refresh() }
                    }
                } label: {
                    Image(systemName: isPaused ? "play.fill" : "pause.fill")
                }
                .macToolbarIcon()
                .help(isPaused ? String(localized: "Resume") : String(localized: "Pause"))
                .accessibilityLabel(isPaused ? String(localized: "Resume") : String(localized: "Pause"))
            }
            statusHeader
            filterBar
            logViewer
            actionBar
        }
        .frame(minWidth: 480, minHeight: 520)
        .background(MacPageBackground())
        .task {
            while !Task.isCancelled {
                if !isPaused {
                    await model.refresh()
                }
                try? await Task.sleep(for: .milliseconds(850))
            }
        }
        .onChange(of: detailedLoggingEnabled) { _, _ in
            MacDebugLoggingConfiguration.apply()
        }
        .onChange(of: configuredLevel) { _, _ in
            MacDebugLoggingConfiguration.apply()
        }
        .confirmationDialog(
            String(localized: "Clear Logs"),
            isPresented: $confirmsClear,
            titleVisibility: .visible
        ) {
            Button(String(localized: "Clear Logs"), role: .destructive) {
                Task {
                    await model.clear()
                    if model.operationError == nil {
                        showTransientNotice(String(localized: "Logs Cleared"))
                    }
                }
            }
            Button(String(localized: "Cancel"), role: .cancel) {}
        }
    }

    private var statusHeader: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 13) {
                Image(systemName: isPaused ? "pause.fill" : "waveform.path.ecg")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(recordingTint)
                    .frame(width: 34, height: 34)
                    .background(recordingTint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 3) {
                    Text(recordingStatusTitle)
                        .font(.headline)
                    Text(recordingStatusDescription)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }

                Spacer(minLength: 16)

                Toggle(String(localized: "Detailed Logging"), isOn: $detailedLoggingEnabled)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .accessibilityLabel(String(localized: "Detailed Logging"))
            }

            Divider()

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 14) {
                    loggingLevelPicker
                    Spacer(minLength: 12)
                    diagnosticMetadata
                }
                VStack(alignment: .leading, spacing: 10) {
                    loggingLevelPicker
                    diagnosticMetadata
                }
            }

            if let error = visibleError, !error.isEmpty {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            }
        }
        .padding(16)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
        .padding(.vertical, 12)
    }

    private var recordingTint: Color {
        if isPaused { return .orange }
        return detailedLoggingEnabled ? .green : .blue
    }

    private var recordingStatusTitle: String {
        if isPaused { return String(localized: "Logs Paused") }
        return detailedLoggingEnabled
            ? String(localized: "Detailed Logging")
            : String(localized: "Warnings & Errors")
    }

    private var recordingStatusDescription: String {
        if isPaused { return String(localized: "Live log updates are paused. Resume to continue following new entries.") }
        return detailedLoggingEnabled
            ? String(localized: "Detailed logging is active. New runtime events will appear here automatically.")
            : String(localized: "Only warnings and errors are recorded until debug logging is enabled in Settings.")
    }

    private var loggingLevelPicker: some View {
        Picker(String(localized: "Level"), selection: $configuredLevel) {
            Text(String(localized: "Errors")).tag("error")
            Text(String(localized: "Warnings")).tag("warn")
            Text(String(localized: "Info")).tag("info")
            Text(String(localized: "Debug")).tag("debug")
            Text(String(localized: "Trace")).tag("trace")
        }
        .frame(width: 190)
        .disabled(!detailedLoggingEnabled)
    }

    private var diagnosticMetadata: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                diagnosticMetadataItems
            }
            VStack(alignment: .leading, spacing: 6) {
                diagnosticMetadataItems
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    @ViewBuilder
    private var diagnosticMetadataItems: some View {
        lastRefreshLabel
        Label(
            String(format: String(localized: "%lld entries"), Int64(model.entries.count)),
            systemImage: "text.line.first.and.arrowtriangle.forward"
        )
        Label(byteCount(model.status?.sizeBytes ?? 0), systemImage: "internaldrive")
    }

    @ViewBuilder
    private var lastRefreshLabel: some View {
        if let refresh = model.lastRefreshAt {
            Label {
                Text(refresh, style: .time)
                    .monospacedDigit()
            } icon: {
                Image(systemName: "arrow.clockwise")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private var visibleError: String? {
        model.operationError ?? model.sourceError
    }

    private var filterBar: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                filterControls
                Spacer()
                resultCount
            }

            VStack(alignment: .leading, spacing: 9) {
                HStack(spacing: 10) {
                    Picker(String(localized: "Level"), selection: $selectedLevel) {
                        Text(String(localized: "All")).tag(nil as MacDiagnosticLogLevel?)
                        ForEach(MacDiagnosticLogLevel.allCases.filter { $0 != .other }) { level in
                            Text(level.title).tag(level as MacDiagnosticLogLevel?)
                        }
                    }
                    .labelsHidden()
                    Picker(String(localized: "Module"), selection: $selectedModule) {
                        Text(String(localized: "All modules")).tag("")
                        ForEach(availableModules, id: \.self) { module in
                            Text(module).tag(module)
                        }
                    }
                    .labelsHidden()
                    Spacer()
                    resultCount
                }
                TextField(String(localized: "Search logs"), text: $query)
                    .textFieldStyle(.roundedBorder)
            }
        }
        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
        .padding(.vertical, 10)
    }

    private var filterControls: some View {
        Group {
            Picker(String(localized: "Level"), selection: $selectedLevel) {
                Text(String(localized: "All")).tag(nil as MacDiagnosticLogLevel?)
                ForEach(MacDiagnosticLogLevel.allCases.filter { $0 != .other }) { level in
                    Text(level.title).tag(level as MacDiagnosticLogLevel?)
                }
            }
            .labelsHidden()
            .frame(width: 125)
            Picker(String(localized: "Module"), selection: $selectedModule) {
                Text(String(localized: "All modules")).tag("")
                ForEach(availableModules, id: \.self) { module in
                    Text(module).tag(module)
                }
            }
            .labelsHidden()
            .frame(width: 170)

            TextField(String(localized: "Search logs"), text: $query)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 360)
        }
    }

    private var resultCount: some View {
        Text("\(filteredEntries.count) / \(model.entries.count)")
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
    }

    private var logViewer: some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical) {
                if filteredEntries.isEmpty {
                    emptyState
                        .frame(maxWidth: .infinity, minHeight: 300)
                } else {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(filteredEntries) { entry in
                            logRow(entry)
                                .id(entry.id)
                            Divider().opacity(0.45)
                        }
                    }
                    .padding(.vertical, 6)
                }
            }
            .background(Color(nsColor: .textBackgroundColor).opacity(0.5))
            .onChange(of: model.revision) { _, _ in
                guard !isPaused, followsLatest, let last = filteredEntries.last else { return }
                withAnimation(reduceMotion ? nil : .easeOut(duration: 0.28)) {
                    proxy.scrollTo(last.id, anchor: .bottom)
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color(nsColor: .separatorColor).opacity(0.72))
        }
        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
        .accessibilityIdentifier("mac.diagnostics.logViewer")
    }

    private var emptyState: some View {
        VStack(spacing: 12) {
            Image(systemName: emptyStateIcon)
                .font(.system(size: 28, weight: .medium))
                .foregroundStyle(recordingTint)
            Text(emptyStateTitle)
                .font(.headline)
            Text(emptyStateDescription)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 440)

            if !query.isEmpty || selectedLevel != nil || !selectedModule.isEmpty {
                Button(String(localized: "Clear")) {
                    query = ""
                    selectedLevel = nil
                    selectedModule = ""
                }
                .buttonStyle(.bordered)
            } else if !detailedLoggingEnabled {
                Button(String(localized: "Detailed Logging")) {
                    detailedLoggingEnabled = true
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(32)
    }

    private var emptyStateIcon: String {
        if model.status?.isWritable == false { return "exclamationmark.triangle" }
        if !query.isEmpty || selectedLevel != nil || !selectedModule.isEmpty { return "line.3.horizontal.decrease.circle" }
        if detailedLoggingEnabled { return "waveform.path.ecg" }
        return "ladybug.slash"
    }

    private var emptyStateTitle: String {
        if model.status?.isWritable == false { return String(localized: "Log file unavailable.") }
        if !query.isEmpty || selectedLevel != nil || !selectedModule.isEmpty {
            return String(localized: "No log entries match current filter")
        }
        if detailedLoggingEnabled { return String(localized: "Waiting for log entries") }
        return String(localized: "Detailed logging is off.")
    }

    private var emptyStateDescription: String {
        if let error = visibleError, !error.isEmpty { return error }
        if !query.isEmpty || selectedLevel != nil || !selectedModule.isEmpty {
            return String(localized: "Adjust the filters or search query.")
        }
        if detailedLoggingEnabled {
            return String(localized: "Detailed logging is active. New runtime events will appear here automatically.")
        }
        return String(localized: "Only warnings and errors are recorded until debug logging is enabled in Settings.")
    }

    private var actionBar: some View {
        HStack(spacing: 10) {
            Group {
                if let transientNotice {
                    Label(transientNotice, systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                        .transition(.opacity)
                }
            }
            .font(.caption.weight(.medium))
            .frame(minWidth: 120, alignment: .leading)
            Spacer(minLength: 0)
            Button {
                followsLatest.toggle()
                if followsLatest {
                    Task { await model.refresh() }
                }
            } label: {
                Label(
                    followsLatest ? String(localized: "Following Latest") : String(localized: "Follow Latest"),
                    systemImage: followsLatest ? "arrow.down.to.line.compact" : "arrow.down.to.line"
                )
            }
            .buttonStyle(.bordered)
            .help(followsLatest ? String(localized: "Stop Following") : String(localized: "Follow Latest"))
            .accessibilityValue(followsLatest ? String(localized: "On") : String(localized: "Off"))
            ViewThatFits(in: .horizontal) {
                actionButtons
                compactActionButtons
            }
        }
        .buttonStyle(.bordered)
        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
        .padding(.vertical, 12)
    }

    private var actionButtons: some View {
        HStack(spacing: 8) {
            Button {
                copyFilteredLogs()
            } label: {
                Label(String(localized: "Copy"), systemImage: "doc.on.doc")
            }
            .disabled(filteredEntries.isEmpty)

            Button {
                exportFilteredLogs()
            } label: {
                Label(String(localized: "Export Logs"), systemImage: "square.and.arrow.up")
            }
            .disabled(filteredEntries.isEmpty)

            Button {
                revealLog()
            } label: {
                Label(String(localized: "Reveal Log in Finder"), systemImage: "folder")
            }

            Spacer()

            Button(role: .destructive) {
                confirmsClear = true
            } label: {
                Label(String(localized: "Clear Logs"), systemImage: "trash")
            }
        }
    }

    private var compactActionButtons: some View {
        Menu {
            Button {
                copyFilteredLogs()
            } label: {
                Label(String(localized: "Copy"), systemImage: "doc.on.doc")
            }
            .disabled(filteredEntries.isEmpty)
            Button {
                exportFilteredLogs()
            } label: {
                Label(String(localized: "Export Logs"), systemImage: "square.and.arrow.up")
            }
            .disabled(filteredEntries.isEmpty)
            Button {
                revealLog()
            } label: {
                Label(String(localized: "Reveal Log in Finder"), systemImage: "folder")
            }
            Divider()
            Button(String(localized: "Clear Logs"), role: .destructive) {
                confirmsClear = true
            }
        } label: {
            Label(String(localized: "More"), systemImage: "ellipsis.circle")
        }
        .accessibilityIdentifier("mac.diagnostics.compactActions")
    }

    private func logRow(_ entry: MacDiagnosticLogEntry) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(entry.timestamp)
                    .foregroundStyle(.tertiary)
                    .frame(width: 170, alignment: .leading)
                Text(entry.level.rawValue)
                    .foregroundStyle(entry.level.tint)
                    .fontWeight(.semibold)
                    .frame(width: 52, alignment: .leading)
                Text(entry.module)
                    .foregroundStyle(.secondary)
                    .frame(width: 124, alignment: .leading)
                Text(entry.message)
                    .foregroundStyle(.primary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 8) {
                    Text(entry.level.rawValue)
                        .foregroundStyle(entry.level.tint)
                        .fontWeight(.semibold)
                    Text(entry.module)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    Text(entry.timestamp)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
                Text(entry.message)
                    .foregroundStyle(.primary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.system(.caption, design: .monospaced))
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func byteCount(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    private func copyFilteredLogs() {
        let text = filteredEntries.map(\.rawLine).joined(separator: "\n")
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(MacDiagnosticLogSanitizer.sanitize(text), forType: .string)
        showTransientNotice(String(localized: "Copied"))
    }

    private func exportFilteredLogs() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.plainText]
        panel.nameFieldStringValue = "nola-log.txt"
        let text = MacDiagnosticLogSanitizer.sanitize(filteredEntries.map(\.rawLine).joined(separator: "\n"))
        let completion: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try text.write(to: url, atomically: true, encoding: .utf8)
                model.operationError = nil
                showTransientNotice(String(localized: "Exported"))
            } catch {
                model.operationError = MacDiagnosticLogSanitizer.sanitize(error.localizedDescription)
            }
        }
        if let window = NSApp.keyWindow {
            panel.beginSheetModal(for: window, completionHandler: completion)
        } else {
            panel.begin(completionHandler: completion)
        }
    }

    private func revealLog() {
        let url = model.source.logFileURL
        if FileManager.default.fileExists(atPath: url.path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } else {
            NSWorkspace.shared.open(url.deletingLastPathComponent())
        }
    }

    private func showTransientNotice(_ message: String) {
        withAnimation(reduceMotion ? nil : .easeOut(duration: 0.16)) {
            transientNotice = message
        }
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2))
            guard transientNotice == message else { return }
            withAnimation(reduceMotion ? nil : .easeIn(duration: 0.16)) {
                transientNotice = nil
            }
        }
    }

}
