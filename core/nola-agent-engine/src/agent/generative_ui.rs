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
        "Publish Intelligence UI in this conversation using A2UI v0.9.1: native forms, grouped detail cards, tabbed comparisons, adjustable choices and interactive controls. When the Host declares graphical rendering, proactively use this when it makes the requested information or next step clearer and easier to operate; the user need not ask for a form or name a tool. Keep simple conversational answers as text. For read-only cards, omit buttons and inputs; use supported Card/Column/Row/Tabs/Text components. This publishes native layouts, not diagrams. Do not simulate geographic maps, diagrams, arrows or spatial alignment with Text cards or whitespace. Use the client's declared graphical diagram renderer for those outputs. This catalog has no map, canvas, SVG or freeform drawing component; do not claim geographic or graphical precision that these components cannot represent. Catalog: {}. Use createSurface then updateComponents/updateDataModel; root component id is root. Messages in one call must target one surface and commit atomically. expected_revision=0 creates; otherwise use the latest known revision. User edits also advance revisions; on ui_revision_conflict inspect current_revision and current_summary, review the user action, then issue a new call only if the intended patch still applies. Never guess revisions or overwrite unrelated user data. Supported components: {components}. Use {{path:'/field'}} for input bindings; Button action={{event:{{name:'submit',context:{{field:{{path:'/field'}}}}}}}}. Pure functions: {functions}. Do not use HTML, scripts, remote catalogs, credentials or permission prompts. A ready result means the complete UI was committed and is available to the client; no additional render_ui call is needed to display it. If the user requested only this form/card, give a brief final response. Do not repeat unchanged content or simulate clicks. A real user action arrives in a later ui_action input; only then process submitted values. Its surface_revision is the revision committed for that user edit; use it as the latest known expected_revision unless a later publication returned a newer revision. UI publication does not perform the business operation represented by the form. Every messages element has its own version. Button labels are separate Text children, never a label property. Minimal valid arguments: {}",
        nola_kernel::generative_ui::schema::NOLA_CATALOG_ID,
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
