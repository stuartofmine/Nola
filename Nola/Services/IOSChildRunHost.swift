// SPDX-License-Identifier: Apache-2.0
// Physical Host contexts for one admitted child Run. Core owns scheduling and state.
import Foundation

actor IOSChildRunHost {
    private struct Poll: Decodable, Sendable {
        struct Request: Decodable, Sendable { let ticket: UInt64; let envelope: AgentCoreJSONValue }
        let pump_id: String
        let transport_finished: Bool
        let error: String?
        let request: Request?
    }
    private let pump: AppleChildPump
    private let executor: DeviceAgentCoordinator
    private let conversationID: String
    private let coreSessionID: String
    private let runID: String
    private let config: AgentCoreRuntimeConfig
    private var started = false
    private var operations: [String: Task<Void, Error>] = [:]
    private var requestedStops: Set<String> = []

    init(pump: AppleChildPump, executor: DeviceAgentCoordinator,
         conversationID: String, runID: String, coreSessionID: String, config: AgentCoreRuntimeConfig) {
        self.pump = pump; self.executor = executor
        self.conversationID = conversationID; self.runID = runID; self.coreSessionID = coreSessionID; self.config = config
    }

    /// Transport completion is not a child result; only its persisted seal is.
    func run() async throws {
        guard !started else { throw DeviceAgentCoordinatorError.alreadyRunning }
        started = true
        let pump = pump
        try await withTaskCancellationHandler {
            do {
                var identity: String?
                while !Task.isCancelled {
                    let data = try await Task.detached { try pump.poll() }.value
                    let polled = try JSONDecoder().decode(Poll.self, from: data)
                    if let identity {
                        guard identity == polled.pump_id else { throw AppleChildPump.TransportError.malformed }
                    } else { identity = polled.pump_id }
                    if polled.transport_finished {
                        if let error = polled.error { throw AppleChildPump.TransportError.unconfirmed(error) }
                        try await joinOperations()
                        pump.close()
                        return
                    }
                    if let request = polled.request { try launch(request, pumpID: polled.pump_id) }
                }
                throw CancellationError()
            } catch {
                pump.close()
                for task in operations.values { task.cancel() }
                for task in operations.values { _ = await task.result }
                throw error
            }
        } onCancel: { pump.close() }
    }

    private func launch(_ request: Poll.Request, pumpID: String) throws {
        guard let effect = request.envelope.objectFields?["effect_id"]?.stringScalar,
              operations[effect] == nil else { throw AppleChildPump.TransportError.malformed }
        let pump = pump, executor = executor, config = config
        let conversationID = conversationID, runID = runID, coreSessionID = coreSessionID
        operations[effect] = Task { [self] in
            do {
                try await executor.executeChildHostRequest(request.envelope,
                    conversationID: conversationID, runID: runID, coreSessionID: coreSessionID, config: config,
                    stopTarget: { target in try await self.stopTarget(target) }) { response in
                        struct Reply: Encodable {
                            let pump_id: String
                            let ticket: UInt64
                            let response: AgentCoreJSONValue
                        }
                        let data = try JSONEncoder().encode(Reply(pump_id: pumpID, ticket: request.ticket, response: response))
                        try await Task.detached { try pump.respond(data) }.value
                    }
            } catch {
                // A requested stop still needs the executor's concrete local
                // witness and Core CancelConfirmed ACK. Task cancellation alone
                // never settles the effect or closes the shared mailbox.
                if !requestedStops.contains(effect) { pump.close() }
                throw error
            }
        }
    }

    private func stopTarget(_ effect: String) async throws {
        guard let task = operations[effect] else { return }
        requestedStops.insert(effect)
        task.cancel()
        _ = await task.result
    }

    private func joinOperations() async throws {
        for (effect, task) in operations {
            switch await task.result {
            case .success: break
            case .failure(let error):
                if !requestedStops.contains(effect) { throw error }
            }
        }
    }
}
