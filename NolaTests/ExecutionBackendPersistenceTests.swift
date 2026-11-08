// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies conversation-scoped execution-unit persistence on iOS.

import XCTest
@testable import Nola

final class ExecutionBackendPersistenceTests: XCTestCase {
    func testSelectionPolicyAcceptsOnlyHostReportedAvailableExecutionUnits() throws {
        XCTAssertEqual(
            try ExecutionBackendSelectionPolicy.validatedSelection(
                "  codex  ",
                availableBackendIDs: ["nola", "codex"]
            ),
            "codex"
        )
        XCTAssertThrowsError(
            try ExecutionBackendSelectionPolicy.validatedSelection(
                "claude_code",
                availableBackendIDs: ["nola", "codex"]
            )
        )
        XCTAssertThrowsError(
            try ExecutionBackendSelectionPolicy.validatedSelection(
                "",
                availableBackendIDs: ["nola"]
            )
        )
    }

    func testExternalExecutionSessionSurvivesConversationEncoding() throws {
        let conversation = Conversation(
            title: "External executor",
            executionBackend: "claude_code",
            externalExecutionSessions: [
                "claude_code": "session-claude-42",
                "codex": "thread-codex-7",
            ]
        )

        let data = try JSONEncoder().encode(conversation)
        let restored = try JSONDecoder().decode(Conversation.self, from: data)

        XCTAssertEqual(restored.executionBackend, "claude_code")
        XCTAssertEqual(restored.externalExecutionSessions["claude_code"], "session-claude-42")
        XCTAssertEqual(restored.externalExecutionSessions["codex"], "thread-codex-7")
    }

    func testCatalogReconciliationKeepsAvailableConversationSelection() {
        XCTAssertEqual(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "codex",
                hostPreferredBackendID: "claude_code",
                availableBackendIDs: ["nola", "codex", "claude_code"]
            ),
            "codex"
        )
    }

    func testCatalogReconciliationRepairsUnavailablePersistedSelection() {
        XCTAssertEqual(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "removed_backend",
                hostPreferredBackendID: "codex",
                availableBackendIDs: ["nola", "codex"]
            ),
            "codex"
        )
        XCTAssertEqual(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "removed_backend",
                hostPreferredBackendID: "also_removed",
                availableBackendIDs: ["nola", "codex"]
            ),
            "nola"
        )
        XCTAssertNil(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "nola",
                hostPreferredBackendID: nil,
                availableBackendIDs: []
            )
        )
    }

    func testHostPermissionPresentationIsConversationScoped() {
        let owner = UUID()
        XCTAssertTrue(
            HostPermissionPresentationPolicy.shouldPresent(
                pendingConversationID: owner,
                currentConversationID: owner
            )
        )
        XCTAssertFalse(
            HostPermissionPresentationPolicy.shouldPresent(
                pendingConversationID: owner,
                currentConversationID: UUID()
            )
        )
        XCTAssertTrue(
            HostPermissionPresentationPolicy.shouldPresent(
                pendingConversationID: nil,
                currentConversationID: nil
            )
        )
    }

    func testWebHostCredentialResolutionFallsBackToCurrentSessionModel() {
        let model = ModelInfo(
            id: "\(ModelSelectionManager.webBackendIdPrefix)session-fallback",
            name: "Session Host",
            source: ModelSelectionManager.webBackendSource,
            isLocal: false,
            baseURL: "https://session-fallback-\(UUID().uuidString).invalid",
            apiKey: "in-memory-session-token",
            upstreamModelId: "web-host"
        )

        XCTAssertEqual(model.resolvedAPIKey, "in-memory-session-token")
    }

}
