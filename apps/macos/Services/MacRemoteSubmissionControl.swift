import Foundation

/// Ephemeral connection selection. Never serialized into a transcript or log.
nonisolated struct MacRemoteSubmissionTarget: Sendable {
    let configuration: MacRemoteHostConfiguration
    let clientID: String
    let expectedSourceStoreEpoch: String?
    init(configuration: MacRemoteHostConfiguration, clientID: String, expectedSourceStoreEpoch: String? = nil) {
        self.configuration = configuration
        self.clientID = clientID
        self.expectedSourceStoreEpoch = expectedSourceStoreEpoch
    }
}

nonisolated struct MacRemoteSubmissionControl: Sendable {
    var queuedUserCommandID: String? = nil
    let target: MacRemoteSubmissionTarget
    let ticket: MacRunSubmissionTicket
    let beforeDispatch: @MainActor @Sendable () async throws -> Void
}

nonisolated struct MacRoomRemoteRequestPreparation: Sendable {
    let conversationID: String
    let submissionID: String
    let prompt: String
    let contextComponents: [MacCoreContextComponent]
    let target: MacRemoteSubmissionTarget
}
