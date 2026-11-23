// SPDX-License-Identifier: Apache-2.0
//! Shared capability declaration. Hosts register this only with a bound UI publisher.
use super::host::*;
use nola_domain::generative_ui_tool::UI_TOOL_ID;

pub fn capability_contract() -> serde_json::Value {
    serde_json::json!({"descriptor": capability_descriptor(), "model_tool": tool_schema()})
}

/// This exact Core-owned route only publishes validated conversation content.
/// It cannot execute a Button's business action or access external resources.
/// Explicit permission rules still apply; this only removes the unknown-risk
/// product-default confirmation from an ordinary reply presentation.
pub fn is_local_publication(descriptor: &HostCapabilityDescriptor) -> bool {
    let origin = &descriptor.input_normalization.origin;
    descriptor.id == UI_TOOL_ID
        && descriptor.route == HostCapabilityRoute::Tool
        && descriptor.permission_domain.is_none()
        && origin.kind == HostCapabilityOriginKind::Builtin
        && origin.provider_id.as_deref() == Some("nola")
        && origin.namespace.as_deref() == Some("generative_ui")
        && descriptor.input_normalization.transaction_boundary == HostCapabilityTransactionBoundary::HostState
        && descriptor.input_normalization.permission_contract.resource_plan.is_none()
        && descriptor.input_normalization.mcp_route.is_none()
}

/// Model-facing schema shares the capability's contract; hosts supply routing.
pub fn tool_schema() -> serde_json::Value {
    let descriptor = capability_descriptor();
    serde_json::json!({"type":"function", "function": {
        "name": descriptor.id,
        "description": descriptor.input_schema.get("description").and_then(serde_json::Value::as_str).unwrap_or(""),
        "parameters": descriptor.input_schema,
    }})
}

pub fn capability_descriptor() -> HostCapabilityDescriptor {
    let mut schema = nola_kernel::generative_ui::schema::tool_input_schema()
        .expect("pinned A2UI model contract must be self-contained");
    let components = nola_kernel::generative_ui::schema::SUPPORTED_COMPONENTS.join(", ");
    let functions = nola_kernel::generative_ui::schema::SUPPORTED_FUNCTIONS.join(", ");
    schema["description"] = serde_json::json!(format!(
        "Render native interactive or structured UI in this conversation using A2UI v0.9.1. \
         When the Host declares graphical rendering, proactively use render_ui whenever a native UI makes the answer clearer or easier to act on (the user does not need to name this tool): \
         multi-field forms, preference/parameter inputs, tabbed comparisons, interactive checklists/calculators/filters, and grouped status or detail cards. \
         Keep ordinary conversational replies as plain text; this is a native UI layout tool, not a diagram/map canvas, and does not allow HTML, scripts, or external catalogs. \
         Rules: every item in messages needs version=\"v0.9.1\" and one operation (createSurface, updateComponents, updateDataModel, deleteSurface) targeting one surfaceId; \
         createSurface requires catalogId=\"{}\"; updateComponents MUST include a root component with id=\"root\"; \
         Button requires \"child\" pointing to a separate Text component ID (never \"label\") and action={{\"event\":{{\"name\":\"...\",\"context\":{{...}}}}}}; \
         Card uses singular \"child\" (Column/Row/List use \"children\"); bind inputs with {{\"path\":\"/field\"}}; \
         expected_revision=0 creates a new surface, otherwise pass the latest committed revision (or surface_revision from a ui_action); \
         after a Ready result, give a brief final reply without re-calling render_ui; real user clicks arrive in a later ui_action turn. \
         Components: {components}. Pure functions: {functions}. \
         Example 1 (read-only tabbed comparison): {} \
         Example 2 (interactive form): {}",
        nola_kernel::generative_ui::schema::NOLA_CATALOG_ID,
        nola_kernel::generative_ui::schema::minimal_tabs_example(),
        nola_kernel::generative_ui::schema::minimal_form_example()
    ));
    HostCapabilityDescriptor {
        id: UI_TOOL_ID.into(), title: "Conversation UI".into(),
        route: HostCapabilityRoute::Tool, availability: CapabilityAvailability::Available,
        // A basic publisher must be callable even on Hosts without tool_search.
        exposure: HostCapabilityExposure::Visible, permission_domain: None,
        input_schema: schema,
        input_normalization: HostInputNormalization {
            transaction_boundary: HostCapabilityTransactionBoundary::HostState,
            origin: HostCapabilityOrigin { kind: HostCapabilityOriginKind::Builtin,
                provider_id: Some("nola".into()), provider_title: Some("Nola".into()), namespace: Some("generative_ui".into()) },
            ..Default::default()
        },
        output_policy: HostOutputPolicy { preview_char_limit: 4000, persist_full_output: true, allow_artifacts: false },
        supports_streaming: false, supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Idempotent,
    }
}

#[cfg(test)]
mod publication_permission_tests {
    use super::*;
    use crate::agent::{permissions::*, protocol::AgentInputOrigin};

    #[test]
    fn local_ui_publication_needs_no_cautious_prompt_but_explicit_deny_still_wins() {
        let ui = capability_descriptor();
        let subject = ui.permission_subject(&serde_json::json!({})).unwrap();
        let mut rules = PermissionRuleSet { rules: vec![PermissionRule {
            capability_pattern: "*".into(), resource_pattern: "*".into(),
            decision: PermissionPolicyDecision::Ask, source: PermissionRuleSource::ProductDefault,
        }] };
        let evaluate = |rules: &PermissionRuleSet, descriptor: &HostCapabilityDescriptor| {
            rules.evaluate_resolved_for_origin_with_explanation(&subject, AgentInputOrigin::InteractiveUser,
                !is_local_publication(descriptor) && nola_kernel::permission_contract_requires_confirmation(
                    &descriptor.input_normalization.permission_contract, None)).decision
        };
        assert_eq!(evaluate(&rules, &ui), PermissionPolicyDecision::Allow);
        let mut external = ui.clone();
        external.id = "write_file".into();
        assert_eq!(evaluate(&rules, &external), PermissionPolicyDecision::Ask);
        let mut extension = ui.clone();
        extension.input_normalization.origin.provider_id = Some("extension".into());
        assert_eq!(evaluate(&rules, &extension), PermissionPolicyDecision::Ask);
        rules.rules.push(PermissionRule { capability_pattern: subject.capability.clone(),
            resource_pattern: "*".into(), decision: PermissionPolicyDecision::Deny,
            source: PermissionRuleSource::UserGlobal });
        assert_eq!(evaluate(&rules, &ui), PermissionPolicyDecision::Deny);
    }
}
