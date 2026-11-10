import Foundation

/// Explains Store evidence without treating it as OS liveness or retry authority.
nonisolated enum MacRunRecoveryPresentation {
    static func detail(_ observation: MacRunRecoveryInspection.Observation) -> String {
        switch observation {
        case .missing:
            return String(localized: "No committed Run record was found. Submission remains unconfirmed.", table: "Chat")
        case .identityMismatch:
            return String(localized: "The stored execution identity does not match. No recovery was attempted.", table: "Chat")
        case .inconsistent:
            return String(localized: "Stored execution state needs reconciliation. No restart was attempted.", table: "Chat")
        case .terminal:
            return String(localized: "Execution outcome confirmed. Finishing the conversation.", table: "Chat")
        case .active(let facts):
            if facts.sourceHostID != nil {
                return String(localized: "This Run belongs to a remote host. Local takeover was not attempted.", table: "Chat")
            }
            if facts.deliveries.contains(where: { $0.state == "outcome_unknown" && $0.count > 0 }) {
                return String(localized: "An execution result is unknown. No automatic retry was attempted.", table: "Chat")
            }
            return String(localized: "The Run is still active in storage. Checking status does not restart execution.", table: "Chat")
        }
    }
}
