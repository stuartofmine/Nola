// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Persists a unified history of foreground, host, chat-only, and automated runs.

import Foundation
import Observation

typealias MacRunCompletionStatus = MacRunResult.Value.Kind

extension MacRunResult.Value.Kind {
    var title: String {
        switch self {
        case .answered: return String(localized: "Answered")
        case .actionCompleted: return String(localized: "Action completed")
        case .delegationAccepted: return String(localized: "Delegated")
        case .incomplete: return String(localized: "Incomplete")
        case .blocked: return String(localized: "Blocked")
        case .failed: return String(localized: "Failed")
        case .cancelled: return String(localized: "Cancelled")
        }
    }
}

nonisolated struct MacActivityEntry: Identifiable, Codable, Hashable, Sendable {
    enum Kind: String, Codable, CaseIterable, Sendable {
        case localAgent
        case connectedHost
        case localModel
        case chatOnly
        case automation

        var title: String {
            switch self {
            case .localAgent: return String(localized: "This Mac")
            case .connectedHost: return String(localized: "Connected Host")
            case .localModel: return String(localized: "Local Model")
            case .chatOnly: return String(localized: "Provider Model")
            case .automation: return String(localized: "Automation")
            }
        }

        var symbol: String {
            switch self {
            case .localAgent: return "desktopcomputer"
            case .connectedHost: return "network"
            case .localModel: return "cpu.fill"
            case .chatOnly: return "bubble.left.and.bubble.right"
            case .automation: return "clock.badge.checkmark"
            }
        }
    }

    enum Status: String, Codable, Hashable, Sendable {
        case answered, actionCompleted = "action_completed", delegationAccepted = "delegation_accepted", incomplete, blocked, failed, cancelled
        // Automation process settlement is not a model's business result.
        case executionFinished = "execution_finished"

        init(result: MacRunCompletionStatus) {
            switch result {
            case .answered: self = .answered
            case .actionCompleted: self = .actionCompleted
            case .delegationAccepted: self = .delegationAccepted
            case .incomplete: self = .incomplete
            case .blocked: self = .blocked
            case .failed: self = .failed
            case .cancelled: self = .cancelled
            }
        }

        /// Only canonical business success earns a success indicator. Process
        /// settlement and cancellation do not attest that the task succeeded.
        var isSuccessful: Bool {
            switch self {
            case .answered, .actionCompleted: true
            case .delegationAccepted, .incomplete, .blocked, .failed, .cancelled, .executionFinished: false
            }
        }

        var needsAttention: Bool {
            switch self {
            case .incomplete, .blocked, .failed: true
            case .answered, .actionCompleted, .delegationAccepted, .cancelled, .executionFinished: false
            }
        }

        var title: String {
            switch self {
            case .answered: String(localized: "Answered")
            case .actionCompleted: String(localized: "Action completed")
            case .delegationAccepted: String(localized: "Delegated")
            case .incomplete: String(localized: "Incomplete")
            case .blocked: String(localized: "Blocked")
            case .failed: String(localized: "Failed")
            case .cancelled: String(localized: "Cancelled")
            case .executionFinished: String(localized: "Execution finished")
            }
        }
    }

    struct AutomationIdentity: Codable, Hashable, Sendable {
        let definitionID: UUID
        let occurrenceID: String
        let runID: String?
        let notificationID: String?
    }

    let id: UUID
    let sourceID: String?
    var automationIdentity: AutomationIdentity? = nil
    let kind: Kind
    let status: Status
    let title: String
    let detail: String
    let conversationID: String?
    let startedAt: Date
    let finishedAt: Date
    let outputPreview: String
    let logPath: String?

    var duration: TimeInterval { max(0, finishedAt.timeIntervalSince(startedAt)) }
}

@MainActor
@Observable
final class MacActivityStore {
    static let shared = MacActivityStore()

    var entries: [MacActivityEntry] = []
    var lastError: String?

    private let fileManager = FileManager.default
    private let maximumEntries = 300
    private let supportDirectory: URL
    private let automationClient: (any MacAutomationApplicationClient)?
    private var rejectedStorageFormat = false

    init(
        baseDirectory: URL? = nil,
        applicationPort: MacApplicationPort? = nil,
        automationClient: (any MacAutomationApplicationClient)? = nil
    ) {
        supportDirectory = baseDirectory ?? Self.defaultSupportDirectory()
        self.automationClient = automationClient ?? applicationPort ?? (baseDirectory == nil
            ? try? MacApplicationPort(databaseURL: MacAppEnvironment.shared.databaseURL)
            : nil)
        load()
    }

    var persistenceURL: URL { storeURL }

    func recordConversationRun(
        title: String,
        conversationID: String,
        execution: MacExecutionLocation,
        status: MacRunCompletionStatus,
        startedAt: Date,
        output: String
    ) {
        let kind: MacActivityEntry.Kind
        switch execution {
        case .thisMac: kind = .localAgent
        case .connectedHost: kind = .connectedHost
        case .localModel: kind = .localModel
        case .chatOnly: kind = .chatOnly
        }
        let detail = status.title
        insert(MacActivityEntry(
            id: UUID(),
            sourceID: conversationID,
            kind: kind,
            status: .init(result: status),
            title: title,
            detail: detail,
            conversationID: conversationID,
            startedAt: startedAt,
            finishedAt: .now,
            outputPreview: String(output.trimmingCharacters(in: .whitespacesAndNewlines).prefix(8_000)),
            logPath: nil
        ))
    }

    func recordAutomationRun(
        _ automation: MacAutomation,
        startedAt: Date,
        receipt: MacAutomationExecutionReceipt,
        logURL: URL?
    ) {
        guard !entries.contains(where: { $0.automationIdentity?.notificationID == receipt.notificationID }) else { return }
        let detail = [
            automation.scheduleDescription,
            automation.dataDependencyDescription,
        ]
            .compactMap { $0 }
            .joined(separator: " · ")
        insert(MacActivityEntry(
            id: UUID(),
            sourceID: nil,
            automationIdentity: .init(definitionID: automation.id, occurrenceID: receipt.binding.occurrenceID,
                runID: receipt.binding.runID, notificationID: receipt.notificationID),
            kind: .automation,
            status: .init(result: receipt.terminal.result.value.kind),
            title: automation.name,
            detail: detail,
            conversationID: receipt.binding.conversationID,
            startedAt: startedAt,
            finishedAt: Self.date(receipt.reconciledAtMs),
            outputPreview: tailText(at: logURL),
            logPath: logURL?.path
        ))
    }

    func recordAutomationLaunchFailure(_ automation: MacAutomation, run: MacCanonicalAutomationRun) {
        guard run.status == "launch_failed",
              !entries.contains(where: { $0.automationIdentity?.occurrenceID == run.id }) else { return }
        guard let payload = run.errorJSON?.data(using: .utf8),
              let observation = try? JSONDecoder().decode(MacAutomationWorkerObservation.self, from: payload),
              case .launchFailed(let failureDetail) = observation else {
            lastError = "Automation startup failure is missing its durable diagnostic fact."
            return
        }
        insert(MacActivityEntry(id: UUID(), sourceID: nil,
            automationIdentity: .init(definitionID: automation.id, occurrenceID: run.id, runID: nil, notificationID: nil),
            kind: .automation, status: .failed, title: automation.name, detail: automation.scheduleDescription,
            conversationID: nil, startedAt: Self.date(run.startedAtMs ?? run.createdAtMs),
            finishedAt: Self.date(run.finishedAtMs ?? run.updatedAtMs), outputPreview: failureDetail, logPath: nil))
    }

    func synchronizeAutomationLogs(_ automations: [MacAutomation]) {
        guard let automationClient else { return }
        for automation in automations {
            do {
                for run in try automationClient.listAutomationRuns(automationID: automation.id.uuidString.lowercased(), limit: 10_000) {
                    if run.status == "launch_failed" { recordAutomationLaunchFailure(automation, run: run); continue }
                    guard let receipt = try automationClient.observeAutomationExecution(occurrenceID: run.id).receipt else { continue }
                    recordAutomationRun(automation, startedAt: Self.date(run.startedAtMs ?? run.createdAtMs), receipt: receipt, logURL: nil)
                }
            } catch { lastError = error.localizedDescription }
        }
    }

    func remove(_ entry: MacActivityEntry) {
        entries.removeAll { $0.id == entry.id }
        persist()
    }

    func clear() {
        // An explicit clear is the user's choice; failed reads never rewrite data.
        rejectedStorageFormat = false
        entries = []
        persist()
    }

    func restore(_ restoredEntries: [MacActivityEntry]) {
        entries = Array(restoredEntries.sorted { $0.finishedAt > $1.finishedAt }.prefix(maximumEntries))
        persist()
    }

    private func insert(_ entry: MacActivityEntry) {
        entries.insert(entry, at: 0)
        entries.sort { $0.finishedAt > $1.finishedAt }
        if entries.count > maximumEntries {
            entries.removeLast(entries.count - maximumEntries)
        }
        persist()
    }

    private func load() {
        guard let data = try? Data(contentsOf: storeURL) else { return }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        do {
            let decoded = try decoder.decode([MacActivityEntry].self, from: data)
            guard decoded.allSatisfy({ $0.kind != .automation || ($0.automationIdentity != nil && $0.sourceID == nil) }) else {
                throw CocoaError(.coderReadCorrupt)
            }
            entries = decoded.sorted { $0.finishedAt > $1.finishedAt }
        } catch {
            rejectedStorageFormat = true
            lastError = "Activity storage format is invalid; the existing file was preserved: \(error.localizedDescription)"
        }
    }

    private func persist() {
        guard !rejectedStorageFormat else { return }
        do {
            try fileManager.createDirectory(at: supportDirectory, withIntermediateDirectories: true)
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            try encoder.encode(entries).write(to: storeURL, options: .atomic)
        } catch {
            lastError = error.localizedDescription
        }
    }

    private func tailText(at url: URL?) -> String {
        guard let url,
              let handle = try? FileHandle(forReadingFrom: url) else { return "" }
        defer { try? handle.close() }
        let size = (try? handle.seekToEnd()) ?? 0
        let start = size > 64_000 ? size - 64_000 : 0
        try? handle.seek(toOffset: start)
        let data = (try? handle.readToEnd()) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    private static func defaultSupportDirectory() -> URL {
        MacAppEnvironment.shared.supportRoot
            .appendingPathComponent("Activity", isDirectory: true)
    }

    private static func date(_ milliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(milliseconds) / 1_000)
    }

    private var storeURL: URL { supportDirectory.appendingPathComponent("history.json") }
}
