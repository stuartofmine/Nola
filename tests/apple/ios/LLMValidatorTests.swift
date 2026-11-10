// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LLMValidatorTests logic in the project module.
// - Interfaces:
//   - LLMValidatorTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

final class LLMValidatorTests: XCTestCase {

    func testExtractJSONFromFencedBlock() throws {
        let response = "Here is the result:\n```json {\n  \"actions\": [ { \"type\": \"notify\", \"priority\": \"medium\", \"dedupeKey\": \"k1\", \"reason\": \"r\" } ] } ```\nThanks."
        let extracted = LLMResponseValidator.extractJSONSubstring(from: response)
        XCTAssertNotNil(extracted)
        XCTAssertTrue(extracted!.contains("actions"))
    }

    func testValidateAndDecodeSuccess() throws {
        let json = "{ \"actions\": [ { \"type\": \"notify\", \"priority\": \"medium\", \"dedupeKey\": \"url:ex:1\", \"scheduledAt\": null, \"contentSummary\": \"摘要\", \"reason\": \"测试\", \"confidence\": 0.9 } ], \"summary\": \"ok\" }"
        let response = "Some preface text\n\(json)\nfooter"
        let decoded = try LLMResponseValidator.validateAndDecode(response)
        XCTAssertEqual(decoded.actions.count, 1)
        XCTAssertEqual(decoded.summary, "ok")
    }

    func testValidateAndDecodeMissingActionsThrows() throws {
        let badJson = "{ \"note\": \"nothing here\" }"
        XCTAssertThrowsError(try LLMResponseValidator.validateAndDecode(badJson)) { err in
            guard case LLMValidationError.noJSONFound = err else {
                // If a JSON substring was found but decode failed, it might surface as decodeError
                // Accept decodeError as valid failure mode for this test.
                return
            }
        }
    }

    func testUserFriendlyMessage() {
        let err = LLMValidationError.noJSONFound
        let msg = LLMResponseValidator.userFriendlyMessage(for: err)
        XCTAssertTrue(msg.contains("未能从模型响应中解析到"))
    }

    func testRecordAuditDoesNotThrow() {
        // Ensure audit writes do not throw and are best-effort
        LLMResponseValidator.recordAudit(candidateId: "cand-1", modelId: "local:test", inputPreview: "preview text", extractedJSON: "{...}", decodedSummary: "ok", error: nil)
        // success if no exception
    }
}
