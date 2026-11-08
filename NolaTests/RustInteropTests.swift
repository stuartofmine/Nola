// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements RustInteropTests logic in the project module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
import XCTest
@testable import Nola

// Removed optional Rust interop tests. They accepted nil FFI results as success
// and could hang the hosted simulator test process when the dynamic library was
// unavailable or mislinked.
