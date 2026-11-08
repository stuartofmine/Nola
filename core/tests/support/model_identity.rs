// Explicit capabilities of the deterministic fixture Host, not Core defaults.
pub fn host_identity() -> nola_core::agent::AgentTurnIdentity {
    let profile = nola_core::agent::resolve_model_capability_profile(
        None, "fixture-model",
    );
    nola_core::agent::AgentTurnIdentity {
        model_capability_id: Some(profile.profile_id.clone()),
        model_capability_revision: profile.revision,
        model_capability: profile,
        ..Default::default()
    }
}
