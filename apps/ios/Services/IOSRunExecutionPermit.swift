import Foundation

/// A synchronous dispatch fence shared across executor actor hops. Revocation
/// prevents new admissions; it cannot undo an operation already admitted.
nonisolated public final class IOSRunExecutionPermit: @unchecked Sendable {
    private let lock = NSLock()
    private var revoked = false

    public init() {}

    public func revoke() {
        lock.lock()
        revoked = true
        lock.unlock()
    }

    public func checkAdmission() throws {
        lock.lock()
        defer { lock.unlock() }
        if revoked { throw CancellationError() }
    }
}
