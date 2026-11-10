// SPDX-License-Identifier: MIT
import Foundation

@main
struct HTTPResponseWriterTests {
    enum FixtureError: Error { case brokenPipe, inference }
    final class Sink {
        var writes: [Data] = []
        var failAt: Int?
        func send(_ data: Data) async throws {
            writes.append(data)
            if writes.count == failAt { throw FixtureError.brokenPipe }
        }
        var text: String { writes.map { String(decoding: $0, as: UTF8.self) }.joined() }
    }

    static func main() async throws { try await runTests() }

    nonisolated static func runTests() async throws {
        let normal = Sink()
        let writer = MLXHTTPResponseWriter(send: normal.send)
        try await writer.startStream()
        try await writer.event(["content": "你好"])
        try await writer.finishStream()
        await writer.fail(FixtureError.inference)
        await reject { try await writer.finishStream() }
        await reject { try await writer.event([:]) }
        precondition(normal.writes.count == 3 && normal.text.hasSuffix("data: [DONE]\n\n"))

        let interrupted = Sink()
        let stream = MLXHTTPResponseWriter(send: interrupted.send)
        try await stream.startStream()
        try await stream.event(["content": "partial"])
        await stream.fail(FixtureError.inference)
        await stream.fail(FixtureError.inference)
        precondition(interrupted.writes.count == 3)
        precondition(interrupted.text.contains("stream_error") && !interrupted.text.contains("[DONE]"))
        precondition(interrupted.text.components(separatedBy: "HTTP/1.1").count == 2)

        // A failing transport may have consumed any prefix of the submitted data.
        for failedWrite in [1, 2, 3] {
            let sink = Sink()
            sink.failAt = failedWrite
            let response = MLXHTTPResponseWriter(send: sink.send)
            await reject {
                try await response.startStream()
                try await response.event(["content": "partial"])
                try await response.finishStream()
            }
            await response.fail(FixtureError.inference)
            precondition(sink.writes.count == failedWrite && response.state == .failed)
        }
        let partialJSON = Sink()
        partialJSON.failAt = 1
        let json = MLXHTTPResponseWriter(send: partialJSON.send)
        await reject { try await json.json(["content": "partial"]) }
        await json.fail(FixtureError.inference)
        precondition(partialJSON.writes.count == 1)

        let beforeHeaders = Sink()
        let early = MLXHTTPResponseWriter(send: beforeHeaders.send)
        await early.fail(FixtureError.inference)
        precondition(beforeHeaders.writes.count == 1 && beforeHeaders.text.hasPrefix("HTTP/1.1 400"))
        precondition(beforeHeaders.text.contains("invalid_request_error"))
        print("MLX HTTP writer contracts passed: single response, partial-write failures, stream errors and terminal states")
    }

    static func reject(_ body: () async throws -> Void) async {
        do { try await body(); fatalError("Expected writer rejection") }
        catch { }
    }
}
