// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns remote task projection subscriptions, cursors, and cancellation lifecycles.

import Foundation

@MainActor
final class MacRemoteTaskObservationCoordinator {
    enum Output {
        case projection(
            conversationID: String,
            taskID: String,
            projection: MacCoreTaskProjection
        )
        case failure(conversationID: String, taskID: String, detail: String)
    }

    typealias EventSource = (
        _ conversationID: String,
        _ afterCursor: Int64
    ) async -> AsyncThrowingStream<MacRemoteTaskProjectionEvent, Error>

    typealias OutputHandler = @MainActor (Output) -> Void

    private let events: EventSource
    private var tasks: [String: Task<Void, Never>] = [:]
    private var observationIDs: [String: UUID] = [:]

    init(events: @escaping EventSource) {
        self.events = events
    }

    func start(
        conversationID: String,
        taskID: String,
        output: @escaping OutputHandler
    ) {
        cancel(conversationID: conversationID)
        let observationID = UUID()
        observationIDs[conversationID] = observationID
        let events = self.events
        tasks[conversationID] = Task { @MainActor [weak self] in
            await Self.observe(
                conversationID: conversationID,
                taskID: taskID,
                events: events,
                output: output
            )
            self?.finish(conversationID: conversationID, observationID: observationID)
        }
    }

    func cancel(conversationID: String) {
        observationIDs[conversationID] = nil
        tasks.removeValue(forKey: conversationID)?.cancel()
    }

    func cancelAll() {
        observationIDs.removeAll()
        let activeTasks = tasks.values
        tasks.removeAll()
        activeTasks.forEach { $0.cancel() }
    }

    private func finish(conversationID: String, observationID: UUID) {
        guard observationIDs[conversationID] == observationID else { return }
        observationIDs[conversationID] = nil
        tasks[conversationID] = nil
    }

    private static func observe(
        conversationID: String,
        taskID: String,
        events: EventSource,
        output: @escaping OutputHandler
    ) async {
        var cursor: Int64 = 0
        var reconnectDelayNanoseconds: UInt64 = 100_000_000
        while !Task.isCancelled {
            do {
                let stream = await events(conversationID, cursor)
                var receivedEvent = false
                for try await event in stream {
                    if Task.isCancelled { return }
                    receivedEvent = true
                    reconnectDelayNanoseconds = 100_000_000
                    cursor = max(cursor, event.cursor)
                    guard let projection = event.projection else { continue }
                    output(.projection(
                        conversationID: conversationID,
                        taskID: taskID,
                        projection: projection
                    ))
                    if projection.tasks[taskID]?.state.isTerminal != false {
                        return
                    }
                }
                guard !Task.isCancelled else { return }
                try await Task.sleep(nanoseconds: reconnectDelayNanoseconds)
                if !receivedEvent {
                    reconnectDelayNanoseconds = min(
                        reconnectDelayNanoseconds * 2,
                        4_000_000_000
                    )
                }
            } catch is CancellationError {
                return
            } catch {
                output(.failure(
                    conversationID: conversationID,
                    taskID: taskID,
                    detail: error.localizedDescription
                ))
                return
            }
        }
    }
}
