// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Collects desktop runtime diagnostics and creates portable support bundles.

import Foundation
import Observation

struct MacDiagnosticFact: Identifiable, Hashable, Sendable {
    let id: String
    let title: String
    let value: String
    let symbol: String
    let isHealthy: Bool
}

@MainActor
@Observable
final class MacDiagnosticsService {
    var facts: [MacDiagnosticFact] = []
    var logText = ""
    var logSize: Int64 = 0
    var isRefreshing = false
    var isExporting = false
    var errorMessage: String?

    private let fileManager = FileManager.default

    func refresh() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        defer { isRefreshing = false }

        async let text = LogFileStore.shared.readLastLines(maxLines: 2_000, maxBytes: 1_000_000)
        async let size = LogFileStore.shared.currentFileSizeBytes()
        let agentRuntimeAvailable = locateAgentRuntime() != nil
        let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "–"
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "–"
        let architecture = ProcessInfo.processInfo.machineArchitecture
        let chat = MacAppleChatConfigurationStore.shared.displayValues()
        let host = MacRemoteHostConfigurationStore.shared.displayValues()
        let loggingState = Logger.shared.runtimeLoggingState

        facts = [
            MacDiagnosticFact(
                id: "app",
                title: String(localized: "Nola for Mac"),
                value: "\(appVersion) (\(build))",
                symbol: "app",
                isHealthy: true
            ),
            MacDiagnosticFact(
                id: "system",
                title: String(localized: "System"),
                value: "macOS \(ProcessInfo.processInfo.operatingSystemVersionString) · \(architecture)",
                symbol: "desktopcomputer",
                isHealthy: true
            ),
            MacDiagnosticFact(
                id: "agentRuntime",
                title: String(localized: "Local Agent Runtime"),
                value: agentRuntimeAvailable ? String(localized: "Available") : String(localized: "Unavailable"),
                symbol: "gearshape.2",
                isHealthy: agentRuntimeAvailable
            ),
            MacDiagnosticFact(
                id: "chat",
                title: String(localized: "Provider Model"),
                value: chat.model.isEmpty ? String(localized: "Not configured") : chat.model,
                symbol: "bubble.left.and.bubble.right",
                isHealthy: !chat.model.isEmpty && !chat.baseURL.isEmpty
            ),
            MacDiagnosticFact(
                id: "host",
                title: String(localized: "Connected Host"),
                value: host.url.isEmpty ? String(localized: "Not configured") : sanitizedOrigin(host.url),
                symbol: "network",
                isHealthy: host.url.isEmpty || !host.token.isEmpty
            ),
            MacDiagnosticFact(
                id: "logging",
                title: String(localized: "Debug logging"),
                value: loggingState.isDetailedLoggingEnabled
                    ? localizedTitle(for: loggingState.minimumLevel)
                    : String(localized: "Off"),
                symbol: "text.alignleft",
                isHealthy: true
            ),
        ]
        logText = Logger.sanitizeForDiagnostics(await text)
        logSize = await size
    }

    func clearLogs() async {
        await LogFileStore.shared.clear()
        await refresh()
    }

    func createSupportBundle() async throws -> URL {
        guard !isExporting else {
            throw NSError(
                domain: "MacDiagnosticsService",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: String(localized: "An export is already running.")]
            )
        }
        isExporting = true
        defer { isExporting = false }

        let facts = self.facts.map {
            MacDiagnosticFact(
                id: $0.id,
                title: $0.title,
                value: Logger.sanitizeForDiagnostics($0.value),
                symbol: $0.symbol,
                isHealthy: $0.isHealthy
            )
        }
        let diagnosticLog = Logger.sanitizeForDiagnostics(
            await LogFileStore.shared.readLastLines(maxLines: 20_000, maxBytes: 1_000_000)
        )
        let temporaryRoot = fileManager.temporaryDirectory
        return try await Task.detached {
            let manager = FileManager.default
            let identifier = UUID().uuidString.lowercased()
            let staging = temporaryRoot.appendingPathComponent("Nola-Diagnostics-\(identifier)", isDirectory: true)
            let output = temporaryRoot.appendingPathComponent("Nola-Diagnostics-\(identifier).zip")
            try manager.createDirectory(at: staging, withIntermediateDirectories: true)

            let metadata: [String: Any] = [
                "created_at": ISO8601DateFormatter().string(from: Date()),
                "facts": facts.map { ["id": $0.id, "title": $0.title, "value": $0.value, "healthy": $0.isHealthy] },
            ]
            let metadataData = try JSONSerialization.data(withJSONObject: metadata, options: [.prettyPrinted, .sortedKeys])
            try metadataData.write(to: staging.appendingPathComponent("environment.json"), options: .atomic)

            if !diagnosticLog.isEmpty {
                try diagnosticLog.write(
                    to: staging.appendingPathComponent("nola.log"),
                    atomically: true,
                    encoding: .utf8
                )
            }

            let process = Process()
            let errorPipe = Pipe()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
            process.arguments = ["-c", "-k", "--sequesterRsrc", "--keepParent", staging.path, output.path]
            process.standardOutput = FileHandle.nullDevice
            process.standardError = errorPipe
            try process.run()
            let errorTask = Task.detached { try errorPipe.fileHandleForReading.readToEnd() ?? Data() }
            process.waitUntilExit()
            let errorData = try await errorTask.value
            try? manager.removeItem(at: staging)
            guard process.terminationStatus == 0 else {
                throw NSError(
                    domain: "MacDiagnosticsService",
                    code: Int(process.terminationStatus),
                    userInfo: [NSLocalizedDescriptionKey: String(decoding: errorData, as: UTF8.self)]
                )
            }
            return output
        }.value
    }

    private func locateAgentRuntime() -> URL? {
        if let configured = ProcessInfo.processInfo.environment["NOLA_AGENT_HOST_PATH"] {
            let url = URL(fileURLWithPath: configured)
            if fileManager.isExecutableFile(atPath: url.path) { return url }
        }
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: "nola-agent-host"),
           fileManager.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        return nil
    }

    private func sanitizedOrigin(_ rawValue: String) -> String {
        guard let components = URLComponents(string: rawValue),
              let scheme = components.scheme,
              let host = components.host else {
            return String(localized: "Available")
        }
        var origin = "\(scheme)://\(host)"
        if let port = components.port { origin += ":\(port)" }
        return origin
    }

    private func localizedTitle(for level: Logger.Level) -> String {
        switch level {
        case .error, .warn, .info: return String(localized: "Info")
        case .debug: return String(localized: "Debug")
        case .trace: return String(localized: "Trace")
        }
    }
}

private extension ProcessInfo {
    var machineArchitecture: String {
        var systemInfo = utsname()
        uname(&systemInfo)
        return withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: 1) { String(cString: $0) }
        }
    }
}
