// SPDX-License-Identifier: Apache-2.0
// Transport errors are not durable command outcomes. Validate before retiring intent.
import Foundation

nonisolated private struct IOSReceiptShape: Decodable {
    struct Failure: Decodable { let code: String; let message: String; let recoverable: Bool }
    struct Conflict: Decodable { let scope_id: String; let expected_revision: UInt64; let actual_revision: UInt64 }
    let protocol_version: UInt32
    let client_id: String
    let request_id: String
    let received_at: String
    let status: String
    let completed_at: String?
    let committed_revision: UInt64?
    let error: Failure?
    let conflict: Conflict?
}

extension IOSApplicationRuntime {
    nonisolated static func receiptFailureDiagnostic(_ data: Data, input: [String: Any]) -> String {
        let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        let receipt = body?["result"] as? [String: Any]
        let command = (input["command"] as? [String: Any])?["type"] as? String ?? ""
        let boundedCommand = command.count <= 80 && command.range(of: "^[a-z][a-z0-9_]*$", options: .regularExpression) != nil
            ? command : "invalid"
        let status = receipt?["status"] as? String ?? ""
        let knownStatus = ["accepted", "committed", "rejected", "conflict"].contains(status) ? status : "missing_or_invalid"
        let outerOK = (body?["ok"] as? Bool).map { $0 ? "true" : "false" } ?? "missing_or_invalid"
        let outerError = body?["error"] != nil
        let rawCode = (body?["error"] as? [String: Any])?["code"] as? String
        let outerCode = ["invalid_command", "invalid_envelope", "missing_revision", "application_unavailable", "application_lock_poisoned"].contains(rawCode ?? "") ? rawCode! : "other_or_absent"
        let requestMatches = receipt?["request_id"] as? String == input["request_id"] as? String
        let clientMatches = receipt?["client_id"] as? String == input["client_id"] as? String
        let versionMatches = (receipt?["protocol_version"] as? NSNumber)?.uint32Value == expectedServiceProtocol
        let resultPresent = receipt?["result"] != nil
        return "command=\(boundedCommand) json_object=\(body != nil) outer_ok=\(outerOK) outer_error=\(outerError) outer_code=\(outerCode) receipt=\(receipt != nil) status=\(knownStatus) request_matches=\(requestMatches) client_matches=\(clientMatches) version_matches=\(versionMatches) result_present=\(resultPresent)"
    }

    nonisolated static func decodeApplicationResponse(_ data: Data, input: [String: Any]) throws -> [String: Any] {
        struct Envelope: Decodable { let ok: Bool }
        let requestID = input["request_id"] as? String ?? ""
        let command = input["command"] as? [String: Any]
        let isObservation = input["expected_revision"] == nil && isReadOnlyCommand(
            type: command?["type"] as? String ?? "",
            arguments: command?["arguments"] as? [String: Any] ?? [:])
        let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        // Read-only failures cannot have an uncertain mutation outcome. Preserve
        // Core's actual read failure, including explicit storage format rejection.
        if isObservation, let header = try? JSONDecoder().decode(Envelope.self, from: data), !header.ok,
           let failure = body?["error"] as? [String: Any],
           let code = failure["code"] as? String, !code.isEmpty,
           let message = failure["message"] as? String, !message.isEmpty {
            throw IOSApplicationRuntimeError.rejected(message)
        }
        // Mutation transport errors still cannot retire the original intent.
        guard let header = try? JSONDecoder().decode(Envelope.self, from: data), header.ok,
              let receipt = body?["result"] as? [String: Any] else {
            if isObservation { throw IOSApplicationRuntimeError.invalidResponse }
            throw IOSApplicationRuntimeError.outcomePending(requestID)
        }
        if isObservation, receipt["status"] as? String == "accepted" {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        do { try validateApplicationReceipt(receipt, input: input) }
        catch {
            if isObservation { throw IOSApplicationRuntimeError.invalidResponse }
            throw error
        }
        return receipt
    }

    nonisolated static func validateApplicationReceipt(_ receipt: [String: Any], input: [String: Any]) throws {
        let requestID = input["request_id"] as? String ?? ""
        guard let data = try? JSONSerialization.data(withJSONObject: receipt),
              let shape = try? JSONDecoder().decode(IOSReceiptShape.self, from: data),
              shape.protocol_version == expectedServiceProtocol,
              shape.client_id == "ios-application", shape.client_id == input["client_id"] as? String,
              shape.request_id == requestID, !requestID.isEmpty,
              !shape.received_at.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              data.count <= 16 * 1024 * 1024 else { throw IOSApplicationRuntimeError.outcomePending(requestID) }
        let valid: Bool
        let common: Set<String> = ["protocol_version", "client_id", "request_id", "received_at", "status"]
        let allowed: Set<String>
        switch shape.status {
        case "accepted":
            allowed = common
            valid = shape.completed_at == nil && shape.error == nil && shape.conflict == nil
                && receipt["result"] == nil
        case "committed":
            allowed = common.union(["completed_at", "committed_revision", "result"])
            valid = shape.completed_at?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                && shape.error == nil && shape.conflict == nil && shape.committed_revision != 0
                && receipt["result"] != nil // JSON null is a valid command result.
        case "rejected":
            allowed = common.union(["completed_at", "error"])
            guard let error = receipt["error"] as? [String: Any],
                  Set(error.keys).isSubset(of: ["code", "message", "recoverable", "details"]),
                  error["details"] == nil || error["details"] is [String: Any],
                  shape.error?.code.range(of: "^[a-z][a-z0-9_]*$", options: .regularExpression) != nil else {
                throw IOSApplicationRuntimeError.outcomePending(requestID)
            }
            valid = shape.completed_at?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                && shape.error?.code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                && shape.error?.message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                && shape.conflict == nil && receipt["result"] == nil
        case "conflict":
            allowed = common.union(["completed_at", "conflict"])
            guard let conflict = receipt["conflict"] as? [String: Any],
                  Set(conflict.keys) == ["scope_id", "expected_revision", "actual_revision"],
                  shape.conflict?.expected_revision != shape.conflict?.actual_revision else {
                throw IOSApplicationRuntimeError.outcomePending(requestID)
            }
            valid = shape.completed_at?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                && shape.conflict?.scope_id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                && shape.error == nil && receipt["result"] == nil
        default: allowed = []; valid = false
        }
        let text = [shape.received_at, shape.completed_at, shape.error?.message, shape.conflict?.scope_id].compactMap { $0 }
        guard valid, Set(receipt.keys).isSubset(of: allowed),
              text.allSatisfy({ $0.rangeOfCharacter(from: .controlCharacters) == nil }) else {
            throw IOSApplicationRuntimeError.outcomePending(requestID)
        }
    }
}
