// SPDX-License-Identifier: Apache-2.0
//! Explicit capabilities of isolated test Hosts, not runtime configuration defaults.
#![allow(dead_code)]

pub fn identity() -> nola_core::agent::AgentTurnIdentity {
    let profile = nola_core::agent::model_capability::resolve_model_capability_profile(
        Some("openai-compatible"),
        "fixture",
    );
    nola_core::agent::AgentTurnIdentity {
        model_capability_id: Some(profile.profile_id.clone()),
        model_capability_revision: profile.revision,
        model_capability: profile,
        ..Default::default()
    }
}

pub fn config() -> nola_core::agent::AgentCoreConfig {
    nola_core::agent::AgentCoreConfig {
        turn_identity: identity(),
        ..Default::default()
    }
}
