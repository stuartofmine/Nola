// SPDX-License-Identifier: MIT
// A connection has one response owner, including partial-write failures.
import Foundation

final class MLXHTTPResponseWriter {
    enum State { case idle, streaming, finished, failed }
    enum Failure: Error { case invalidState }
    private(set) var state: State = .idle
    private let send: (Data) async throws -> Void

    init(send: @escaping (Data) async throws -> Void) { self.send = send }

    func json(_ object: [String: Any], status: Int = 200) async throws {
        guard state == .idle else { throw Failure.invalidState }
        let body = try JSONSerialization.data(withJSONObject: object)
        let reason: String
        switch status {
        case 200: reason = "OK"
        case 401: reason = "Unauthorized"
        case 404: reason = "Not Found"
        default: reason = "Bad Request"
        }
        var bytes = Data("HTTP/1.1 \(status) \(reason)\r\nContent-Type: application/json\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n".utf8)
        bytes.append(body)
        // Even a failed send may already have written a prefix to the socket.
        state = .finished
        try await write(bytes)
    }

    func startStream() async throws {
        guard state == .idle else { throw Failure.invalidState }
        state = .streaming
        try await write(Data("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n".utf8))
    }

    func event(_ object: [String: Any]) async throws {
        guard state == .streaming else { throw Failure.invalidState }
        var bytes = Data("data: ".utf8)
        bytes.append(try JSONSerialization.data(withJSONObject: object))
        bytes.append(Data("\n\n".utf8))
        try await write(bytes)
    }

    func finishStream() async throws {
        guard state == .streaming else { throw Failure.invalidState }
        state = .finished
        try await write(Data("data: [DONE]\n\n".utf8))
    }

    func fail(_ error: Error) async {
        let previous = state
        guard previous == .idle || previous == .streaming else { return }
        let object: [String: Any] = ["error": ["message": error.localizedDescription,
            "type": previous == .streaming ? "stream_error" : "invalid_request_error"]]
        do {
            if previous == .streaming { try await event(object) }
            else { try await json(object, status: 400) }
        } catch { /* The socket owner closes a failed transport. Never retry bytes. */ }
        state = .failed
    }

    private func write(_ bytes: Data) async throws {
        do { try await send(bytes) }
        catch { state = .failed; throw error }
    }
}
