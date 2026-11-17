// SPDX-License-Identifier: Apache-2.0
//! Consolidated integration contracts for `nola-runtime-driver`.

#[path = "../../nola-application/tests/support/model_identity.rs"]
pub mod fixture_model_identity;

#[path = "suites/committed_runtime_e2e.rs"]
pub mod committed_runtime_e2e;

#[path = "suites/opaque_protocol.rs"]
pub mod opaque_protocol;
