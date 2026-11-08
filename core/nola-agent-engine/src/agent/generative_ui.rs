// SPDX-License-Identifier: Apache-2.0
//! Shared capability declaration. Hosts register this only with a bound UI publisher.
use super::host::*;
use nola_domain::generative_ui_tool::UI_TOOL_ID;

pub fn capability_contract() -> serde_json::Value {
    serde_json::json!({"descriptor": capability_descriptor(), "model_tool": tool_schema()})
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
        "Publish a native form, structured detail, list or task card in this conversation using A2UI v0.9.1. Catalog: {}. Use createSurface then updateComponents/updateDataModel; root component id is root. Messages in one call must target one surface and commit atomically. expected_revision=0 creates; otherwise use the latest known revision. User edits also advance revisions; on ui_revision_conflict inspect current_revision and current_summary, review the user action, then issue a new call only if the intended patch still applies. Never guess revisions or overwrite unrelated user data. Supported components: {components}. Use {{path:'/field'}} for input bindings; Button action={{event:{{name:'submit',context:{{field:{{path:'/field'}}}}}}}}. Pure functions: {functions}. Do not use HTML, scripts, remote catalogs, credentials or permission prompts. A ready result means the complete UI was committed and is available to the client; no additional render_ui call is needed to display it. If the user requested only this form/card, give a brief final response. Do not repeat unchanged content or simulate clicks. A real user action arrives in a later ui_action input; only then process submitted values. Its surface_revision is the revision committed for that user edit; use it as the latest known expected_revision unless a later publication returned a newer revision. UI publication does not perform the business operation represented by the form. Every messages element has its own version. Button labels are separate Text children, never a label property. Minimal valid arguments: {}",
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
