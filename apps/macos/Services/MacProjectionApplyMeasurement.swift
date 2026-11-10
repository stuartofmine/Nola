// SPDX-License-Identifier: Apache-2.0
// Diagnostic apply proxies, never renderer frame or canonical business facts.
import Foundation
import CryptoKit
import os

nonisolated enum MacProjectionApplyMeasurement {
    enum Stage: String, Sendable { case windowLoad = "window_load_proxy", windowRefresh = "window_refresh_proxy", workspacePublication = "workspace_publication_proxy" }
    enum Outcome: String, Sendable { case applied, unchanged, discarded, cancelled, failed }
    struct Cursor: Equatable, Sendable {
        let generation: UInt64
        let eventSequence: UInt64
        let projectionRevision: UInt64
    }
    struct Ticket: Sendable {
        let id: String
        let session: String
        let stage: Stage
        let generation: UInt64
        let startedNS: UInt64
    }
    struct Sample: Equatable, Sendable {
        let observationID: String
        let session: String
        let stage: String
        let outcome: String
        let generation: UInt64
        let eventSequence: UInt64?
        let projectionRevision: UInt64?
        let monotonicNS: UInt64
        let elapsedNS: UInt64
        let selected: Bool
    }

    // Fixed at first use. No event queues, per-session maps or file operations.
    static let enabled = ProcessInfo.processInfo.environment["NOLA_MAC_PROJECTION_MEASUREMENT"] == "1"
    private static let clockDomain = UUID().uuidString
    private static let log = OSLog(subsystem: "com.nola.mac", category: "ProjectionApplyProxy")
    private static let buildDigest: String = {
        let value = ProcessInfo.processInfo.environment["NOLA_MEASUREMENT_BUILD_DIGEST"] ?? ""
        return value.count == 64 && value.utf8.allSatisfy({ (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }) ? value.lowercased() : "missing"
    }()

    static func sessionDigest(_ id: String) -> String {
        SHA256.hash(data: Data(id.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    static func begin(session: String, stage: Stage, generation: UInt64) -> Ticket? {
        guard enabled else { return nil }
        let ticket = Ticket(id: UUID().uuidString, session: sessionDigest(session), stage: stage,
            generation: generation, startedNS: DispatchTime.now().uptimeNanoseconds)
        emit(ticket: ticket, sample: nil)
        return ticket
    }

    /// Pure capture policy also used by source tests. A stale generation never
    /// gains an applied cursor, even if a caller supplies a newer window.
    static func sample(ticket: Ticket, outcome: Outcome, cursor: Cursor?, nowNS: UInt64, selected: Bool) -> Sample? {
        guard nowNS >= ticket.startedNS else { return nil }
        let stale = cursor.map { $0.generation != ticket.generation } ?? false
        let outcome = stale && (outcome == .applied || outcome == .unchanged) ? Outcome.discarded : outcome
        let appliedCursor = outcome == .applied || outcome == .unchanged ? cursor : nil
        return Sample(observationID: ticket.id, session: ticket.session, stage: ticket.stage.rawValue,
            outcome: outcome.rawValue, generation: ticket.generation,
            eventSequence: appliedCursor?.eventSequence, projectionRevision: appliedCursor?.projectionRevision,
            monotonicNS: nowNS, elapsedNS: nowNS - ticket.startedNS, selected: selected)
    }

    static func finish(_ ticket: Ticket?, outcome: Outcome, cursor: Cursor?, selected: Bool = false) {
        guard let ticket, let sample = sample(ticket: ticket, outcome: outcome, cursor: cursor,
            nowNS: DispatchTime.now().uptimeNanoseconds, selected: selected) else { return }
        emit(ticket: ticket, sample: sample)
    }

    private static func emit(ticket: Ticket, sample: Sample?) {
        // Fixed schema, bounded scalar fields; no text, paths or original IDs.
        // os_signpost is an Instruments correlation point, NOT frame-presented.
        let fields = "v=1 process=\(ProcessInfo.processInfo.processIdentifier) clock=\(clockDomain) build=\(buildDigest) observation=\(ticket.id) session=\(ticket.session) stage=\(ticket.stage.rawValue) event=\(sample == nil ? "begin" : "end") outcome=\(sample?.outcome ?? "pending") generation=\(ticket.generation) cursor=\(sample?.eventSequence.map(String.init) ?? "missing") revision=\(sample?.projectionRevision.map(String.init) ?? "missing") monotonic_ns=\(sample?.monotonicNS ?? ticket.startedNS) elapsed_ns=\(sample.map { String($0.elapsedNS) } ?? "missing") selected=\(sample?.selected == true ? "true" : "false")"
        os_signpost(.event, log: log, name: "ProjectionApplyProxy", "%{public}@", fields as NSString)
    }
}
