// SPDX-License-Identifier: Apache-2.0
// Swift adapter for Core-owned model request redaction.
import Foundation
#if canImport(NolaCore)
import NolaCore
#endif

nonisolated func redactedModelRequestBody(_ data: Data, credential: String) throws -> Data {
    #if canImport(NolaCore)
    let result = data.withUnsafeBytes { body in
        Data(credential.utf8).withUnsafeBytes { key in
            nola_core_redact_model_request_json(
                body.bindMemory(to: UInt8.self).baseAddress, body.count,
                key.bindMemory(to: UInt8.self).baseAddress, key.count
            )
        }
    }
    if let result {
        defer { nola_model_privacy_free(result) }
        return Data(bytes: result, count: strlen(result))
    }
    #endif
    throw NolaError.llm(.invalidInput(reason: "Model request privacy filter unavailable."))
}
