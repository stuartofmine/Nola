// SPDX-License-Identifier: Apache-2.0
// Native transport for Core's committed child driver. No execution state lives here.
import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated final class AppleChildPump: @unchecked Sendable {
    enum TransportError: LocalizedError {
        case unavailable, malformed, unconfirmed(String)
        var errorDescription: String? {
            switch self {
            case .unavailable: return "Child transport is unavailable; execution outcome remains unconfirmed"
            case .malformed: return "Child transport returned an invalid response"
            case .unconfirmed(let reason): return reason
            }
        }
    }
    #if canImport(NolaCore)
    private let handle: OpaquePointer
    #endif
    private let gate = NSCondition()
    private var closing = false
    private var calls = 0

    #if canImport(NolaCore)
    init(admittedHandle: OpaquePointer) { handle = admittedHandle }
    #endif

    deinit {
        #if canImport(NolaCore)
        close()
        gate.lock()
        while calls != 0 { gate.wait() }
        gate.unlock()
        nola_core_child_pump_free(handle)
        #endif
    }

    /// Transport closure is not proof that a physical operation stopped.
    func close() {
        gate.lock()
        closing = true
        #if canImport(NolaCore)
        nola_core_child_pump_close(handle)
        #endif
        gate.unlock()
    }

    func poll() throws -> Data {
        try call {
            #if canImport(NolaCore)
            return nola_core_child_pump_poll(handle, 100)
            #else
            throw TransportError.unavailable
            #endif
        }
    }

    /// Returns only after Core has committed the response or rejected it.
    func respond(_ data: Data) throws {
        let result = try call {
            #if canImport(NolaCore)
            return data.withUnsafeBytes { bytes in
                nola_core_child_pump_respond(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
            }
            #else
            throw TransportError.unavailable
            #endif
        }
        guard let object = try JSONSerialization.jsonObject(with: result) as? [String: Any],
              object["committed"] as? Bool == true else { throw TransportError.malformed }
    }

    private func call(_ operation: () throws -> UnsafeMutablePointer<CChar>?) throws -> Data {
        gate.lock()
        guard !closing else { gate.unlock(); throw TransportError.unavailable }
        calls += 1
        gate.unlock()
        defer { gate.lock(); calls -= 1; gate.broadcast(); gate.unlock() }
        guard let pointer = try operation() else { throw TransportError.unavailable }
        #if canImport(NolaCore)
        defer { nola_core_free_c_string(pointer) }
        #endif
        let data = Data(String(cString: pointer).utf8)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let ok = object["ok"] as? Bool else { throw TransportError.malformed }
        guard ok else {
            let reason = (object["error"] as? [String: Any])?["message"] as? String ?? "Child transport is unconfirmed"
            throw TransportError.unconfirmed(reason)
        }
        guard let result = object["result"] else { throw TransportError.malformed }
        return try JSONSerialization.data(withJSONObject: result)
    }
}
