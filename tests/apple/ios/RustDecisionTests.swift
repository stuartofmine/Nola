// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements RustDecisionTests logic in the project module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
import XCTest
@testable import Nola

// Removed FFI decision tests from the hosted iOS unit target. Rust linkage is
// already covered by build/link validation; runtime FFI belongs in a dedicated
// integration target with the framework explicitly installed.
