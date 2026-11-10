import Foundation
import Darwin

/// Owns exactly one child and its pipes. Process lifetime is not canonical Run
/// lifetime; even an initial write failure after launch is transport uncertainty.
nonisolated final class MacLocalAgentProcessSession: @unchecked Sendable {
    struct Exit: Sendable { let status: Int32; let signaled: Bool }
    enum Failure: LocalizedError {
        case alreadyLaunched
        case initialWriteFailed(String)
        var errorDescription: String? {
            switch self {
            case .alreadyLaunched: "The owned process session was already launched."
            case .initialWriteFailed(let detail): "The launched Agent Host input transport failed: \(detail)"
            }
        }
    }

    let process: Process
    let input: FileHandle
    let output: FileHandle
    let error: FileHandle
    private let childInput: FileHandle
    private let childOutput: FileHandle
    private let childError: FileHandle
    private let registry: MacOwnedChildProcessRegistry
    private let writer: @Sendable (FileHandle, Data) throws -> Void
    private let lock = NSLock()
    private var launchAttempted = false
    private var launchInProgress = false
    private var launched = false
    private var exit: Exit?
    private var released = false
    private var waiters: [CheckedContinuation<Exit, Never>] = []

    init(process: Process, inputPipe: Pipe, outputPipe: Pipe, errorPipe: Pipe,
         registry: MacOwnedChildProcessRegistry = .shared,
         writeInitial: @escaping @Sendable (FileHandle, Data) throws -> Void = { try $0.write(contentsOf: $1) }) {
        self.process = process
        input = inputPipe.fileHandleForWriting
        output = outputPipe.fileHandleForReading
        error = errorPipe.fileHandleForReading
        childInput = inputPipe.fileHandleForReading
        childOutput = outputPipe.fileHandleForWriting
        childError = errorPipe.fileHandleForWriting
        self.registry = registry
        writer = writeInitial
        process.standardInput = inputPipe
        process.standardOutput = outputPipe
        process.standardError = errorPipe
    }

    var isReleased: Bool { lock.withLock { released } }

    /// The runtime calls launch synchronously before exposing this session to
    /// cancellation/cleanup. Async lifecycle operations follow its return.
    func launch(ticket: MacRunSubmissionTicket? = nil) throws {
        let canLaunch = lock.withLock {
            guard !launchAttempted, !released else { return false }
            launchAttempted = true
            launchInProgress = true
            return true
        }
        guard canLaunch else { throw Failure.alreadyLaunched }
        process.terminationHandler = { [weak self] process in
            self?.recordExit(Exit(status: process.terminationStatus,
                signaled: process.terminationReason == .uncaughtSignal))
        }
        do {
            try ticket?.claimDispatch()
            try process.run()
        } catch {
            // No successful launch occurred. Never use this path after a write.
            ticket?.rejectBeforeTransport()
            lock.withLock { launchInProgress = false }
            _ = releaseResources()
            throw error
        }
        lock.withLock {
            launched = true
            registry.register(process)
            launchInProgress = false
        }
        try? childInput.close()
        try? childOutput.close()
        try? childError.close()
    }

    func writeInitialRequest(_ data: Data) throws {
        do { try writer(input, data) }
        catch { throw Failure.initialWriteFailed(error.localizedDescription) }
    }

    func waitForExit() async -> Exit {
        await withCheckedContinuation { waiter in
            let ready = lock.withLock { () -> Exit? in
                if let exit { return exit }
                waiters.append(waiter)
                return nil
            }
            if let ready { waiter.resume(returning: ready) }
        }
    }

    private func recordExit(_ value: Exit) {
        let pending = lock.withLock { () -> [CheckedContinuation<Exit, Never>] in
            guard exit == nil else { return [] }
            exit = value
            let pending = waiters
            waiters.removeAll()
            return pending
        }
        for waiter in pending { waiter.resume(returning: value) }
    }

    func shutdownAndDrain(gracePeriod: Duration = .milliseconds(250)) async {
        guard lock.withLock({ launched }) else { _ = releaseResources(); return }
        if process.isRunning { process.terminate() }
        // This task owns the Process instance; it never discovers arbitrary
        // PIDs or performs global cleanup. Do not block the runtime actor.
        let escalation = Task { [self] in
            do { try await Task.sleep(for: gracePeriod) } catch { return }
            if process.isRunning { Darwin.kill(process.processIdentifier, SIGKILL) }
        }
        _ = await waitForExit()
        escalation.cancel()
        _ = releaseResources()
    }

    @discardableResult
    func releaseResources() -> Bool {
        let shouldRelease = lock.withLock {
            guard !released, !launchInProgress, !launched || exit != nil else { return false }
            released = true
            return true
        }
        guard shouldRelease else { return false }
        for handle in [input, output, error, childInput, childOutput, childError] { try? handle.close() }
        process.terminationHandler = nil
        registry.unregister(process)
        return true
    }
}
