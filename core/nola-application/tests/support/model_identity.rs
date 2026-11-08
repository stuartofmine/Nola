// SPDX-License-Identifier: Apache-2.0
//! Explicit capabilities of the isolated fixture Host, never production defaults.
#![allow(dead_code)]

pub fn identity() -> nola_agent_engine::agent::AgentTurnIdentity {
    let profile = nola_agent_engine::agent::model_capability::resolve_model_capability_profile(
        Some("openai-compatible"),
        "fixture",
    );
    nola_agent_engine::agent::AgentTurnIdentity {
        model_capability_id: Some(profile.profile_id.clone()),
        model_capability_revision: profile.revision,
        model_capability: profile,
        ..Default::default()
    }
}

pub fn config() -> nola_agent_engine::agent::AgentCoreConfig {
    nola_agent_engine::agent::AgentCoreConfig {
        turn_identity: identity(),
        ..Default::default()
    }
}
