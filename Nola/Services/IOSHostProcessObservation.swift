// SPDX-License-Identifier: Apache-2.0
// Trusted OS observations only; Core owns lease encoding and death reconciliation.
// sysctl KERN_PROC_PID: Apple sysctl(3), System/Conceptual/ManPages_iPhoneOS.
// Sandbox denial remains Unknown, regardless of Simulator availability.
import Foundation
import Darwin

nonisolated enum IOSHostProcessObservation {
    static func observe(pid: Int32) -> AgentCoreJSONValue {
        func unknown() -> AgentCoreJSONValue {
            .object(["state": .string("unknown"), "pid": .signedInteger(Int64(pid))])
        }
        guard pid > 0, let boot = bootIdentity() else { return unknown() }
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.size
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
        let status = sysctl(&mib, UInt32(mib.count), &info, &size, nil, 0)
        if status == 0, size == MemoryLayout<kinfo_proc>.size,
           info.kp_proc.p_pid == pid {
            let started = info.kp_proc.p_un.__p_starttime
            guard started.tv_sec > 0, started.tv_usec >= 0, started.tv_usec < 1_000_000 else { return unknown() }
            return .object(["state": .string("running"), "pid": .signedInteger(Int64(pid)),
                "boot_id": .string(boot), "start_id": .string("\(started.tv_sec):\(started.tv_usec)")])
        }
        // EPERM is not evidence of death. No process listing, shell command or
        // guessed clock is used when the iOS sandbox denies observation.
        if kill(pid, 0) == -1, errno == ESRCH {
            return .object(["state": .string("gone"), "pid": .signedInteger(Int64(pid)), "boot_id": .string(boot)])
        }
        return unknown()
    }

    static func currentWorkerIdentity() -> AgentCoreJSONValue {
        workerIdentity(observation: observe(pid: getpid()), nonce: UUID().uuidString.lowercased())
    }

    static func workerIdentity(observation: AgentCoreJSONValue, nonce: String) -> AgentCoreJSONValue {
        if case .object(var fields) = observation, fields["state"] == .string("running") {
            fields.removeValue(forKey: "state")
            fields["kind"] = .string("process")
            fields["nonce"] = .string(nonce)
            return .object(fields)
        }
        return .object(["kind": .string("unobservable"), "nonce": .string(nonce)])
    }

    private static func bootIdentity() -> String? {
        var bytes = [CChar](repeating: 0, count: 97)
        var size = bytes.count
        guard sysctlbyname("kern.bootsessionuuid", &bytes, &size, nil, 0) == 0,
              size > 1, size <= bytes.count, bytes[size - 1] == 0 else { return nil }
        let value = String(decoding: bytes.prefix(size - 1).map { UInt8(bitPattern: $0) }, as: UTF8.self)
        return UUID(uuidString: value)?.uuidString.lowercased()
    }
}
