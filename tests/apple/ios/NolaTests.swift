// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NolaTests logic in the project module.
// - Interfaces:
//   - NolaTests
// - APIs:
//   - none
//
//
//  NolaTests.swift
//  NolaTests
//
//  Created by 高金超 on 2025/11/7.
//

import Testing
@testable import Nola

struct NolaTests {

    @Test func example() async throws {
        // Write your test here and use APIs like `#expect(...)` to check expected conditions.
    }

    @Test func responseStylePromptOnlyIncludesSelectedStyle() async throws {
        let prompt = try await PromptTemplates.chatSystem(responseStyle: .playful)
        #expect(prompt.contains("The active user-facing surface is Nola for iPhone and iPad."))
        #expect(prompt.contains("Claim only outcomes established by successful observed results; distinguish generated text from created artifacts."))
        #expect(prompt.contains("Do not identify a real person"))
        #expect(prompt.contains("Do not initiate or offer image generation"))
        #expect(prompt.contains("Current response style: playful."))
        #expect(prompt.contains("<user_configured_instructions id=\"ios:user-response-style\">"))
        #expect(!prompt.contains("Current response style: professional."))
        #expect(!prompt.contains("Current response style: casual."))
    }

}
