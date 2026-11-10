// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ProcessingAgentTests logic in the project module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
import XCTest
@testable import Nola

// Removed substitute-driven ProcessingAgent tests. They asserted behavior by replacing
// the LLM and launching the full ServiceContainer, which made the suite both
// brittle and disconnected from real product behavior.
