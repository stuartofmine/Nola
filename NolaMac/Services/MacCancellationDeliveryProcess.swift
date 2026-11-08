// SPDX-License-Identifier: Apache-2.0
import Foundation

/// A cancellation-only control transport. Exit status is never Run status.
nonisolated enum MacCancellationDeliveryProcess {
    static func frame(_ request: MacRunCancellationRequest) throws -> Data {
        var data = try JSONSerialization.data(withJSONObject: [
            "operation": "cancel", "session_id": request.conversationID,
            "expected_run_id": request.expectedRunID,
            "assistant_message_id": request.assistantMessageID,
            "cancellation_id": request.cancellationID,
        ], options: [.sortedKeys])
        data.append(0x0A)
        return data
    }

    static func arguments(_ request: MacRunCancellationRequest) -> [String] {
        ["run", "--json", "--host-request-stdin", "--session", request.conversationID]
    }

    static func run(executable: URL, databaseURL: URL,
                    request: MacRunCancellationRequest,
                    timeout: Duration = .seconds(30)) async throws {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments(request)
        // Control does not enter the old (possibly revoked) workspace or reuse
        // model credentials, permission grants, or the original tool environment.
        process.currentDirectoryURL = databaseURL.deletingLastPathComponent()
        let inherited = ProcessInfo.processInfo.environment
        var environment = inherited.filter { ["PATH", "HOME", "TMPDIR", "LANG", "LC_ALL"].contains($0.key) }
        environment["NOLA_DB_PATH"] = databaseURL.path
        process.environment = environment
        let session = MacLocalAgentProcessSession(process: process,
            inputPipe: Pipe(), outputPipe: Pipe(), errorPipe: Pipe())
        let input = try frame(request)
        try Task.checkCancellation()
        try session.launch()
        // Drain concurrently even when output exceeds the retained bound.
        let output = Task.detached { drain(session.output, retaining: 0) }
        let errors = Task.detached { drain(session.error, retaining: 4_000) }
        do {
            try session.writeInitialRequest(input)
            try session.input.close()
        } catch {
            await session.shutdownAndDrain()
            _ = await output.value
            _ = await errors.value
            throw error
        }
        let deadline = Task {
            do { try await Task.sleep(for: timeout) } catch { return }
            await session.shutdownAndDrain()
        }
        let exit = await withTaskCancellationHandler {
            await session.waitForExit()
        } onCancel: {
            Task { await session.shutdownAndDrain() }
        }
        deadline.cancel()
        _ = await output.value
        let detail = await errors.value
        session.releaseResources()
        try Task.checkCancellation()
        guard exit.status == 0 && !exit.signaled else {
            throw MacLocalAgentRuntimeError.processFailed(exit.status,
                String(decoding: detail, as: UTF8.self))
        }
    }

    private static func drain(_ handle: FileHandle, retaining limit: Int) -> Data {
        var retained = Data()
        while let chunk = try? handle.read(upToCount: 16_384), !chunk.isEmpty {
            if retained.count < limit { retained.append(chunk.prefix(limit - retained.count)) }
        }
        return retained
    }
}
