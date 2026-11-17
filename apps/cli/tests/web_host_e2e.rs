// SPDX-License-Identifier: Apache-2.0
//! Unified Web Host HTTP, configuration, locale, surface, and canonical session E2E suite.

#[path = "suites/surface_contracts_web_e2e.rs"]
mod surface_contracts_web_e2e;
#[path = "suites/conversation_configuration_web_e2e.rs"]
mod conversation_configuration_web_e2e;
#[path = "suites/canonical_web_host_e2e.rs"]
mod canonical_web_host_e2e;
#[cfg(unix)]
#[path = "suites/historical_part_detail_e2e.rs"]
mod historical_part_detail_e2e;
