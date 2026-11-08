// SPDX-License-Identifier: Apache-2.0
use nola_domain::{generative_ui::*, generative_ui_tool::UiToolInput};
use nola_kernel::generative_ui::{schema, state, projection, interaction};
use serde_json::{json, Value};

pub fn generate() -> Value {
    let identity = UiSurfaceIdentity {
        owner: UiSurfaceOwner { authority_id:"fixture-host".into(), conversation_id:"fixture-conversation".into(),
            participant_id:"fixture-assistant".into(), context_id:"fixture-context".into(),
            originating_run_id:"fixture-run".into(), message_id:"fixture-message".into(), part_id:"fixture-part".into() },
        surface_id:"shared-form".into(), generation:1,
    };
    let wire_input = json!({"expected_revision":0,"messages":[
        {"version":"v0.9.1","createSurface":{"surfaceId":"shared-form","catalogId":schema::NOLA_CATALOG_ID,"sendDataModel":true}},
        {"version":"v0.9.1","updateComponents":{"surfaceId":"shared-form","components":[
            {"id":"root","component":"Column","children":["title","echo","name","choice","consent","submit"]},
            {"id":"title","component":"Text","text":"Shared A2UI form"},
            {"id":"echo","component":"Text","text":{"path":"/name"}},
            {"id":"name","component":"TextField","label":"Name","value":{"path":"/name"}},
            {"id":"choice","component":"ChoicePicker","label":"Language","variant":"mutuallyExclusive",
                "options":[{"label":"中文","value":"zh"},{"label":"English","value":"en"}],"value":{"path":"/languages"}},
            {"id":"consent","component":"CheckBox","label":"Confirm","value":{"path":"/confirmed"}},
            {"id":"submit","component":"Button","child":"submit-label","action":{"event":{"name":"submit_form",
                "context":{"name":{"path":"/name"},"languages":{"path":"/languages"},"confirmed":{"path":"/confirmed"}}}}},
            {"id":"submit-label","component":"Text","text":"Submit"}
        ]}},
        {"version":"v0.9.1","updateDataModel":{"surfaceId":"shared-form","value":{"name":"Seed","languages":["en"],"confirmed":false}}}
    ]});
    let input: UiToolInput = serde_json::from_value(wire_input.clone()).unwrap();
    let initial_state = state::transition_messages(None, &identity, &input).unwrap();
    let initial_snapshot = projection::project_surface(&initial_state).unwrap().snapshot;
    let instance = |component: &str| initial_snapshot.nodes.values().find(|node| node.component_id == component).unwrap().instance_id.clone();
    let interaction = UiInteraction { identity:identity.clone(), expected_surface_revision:initial_snapshot.revision,
        edits: vec![
            UiEdit::Set { instance_id:instance("name"), binding:"value".into(), value:json!("共同验收") },
            UiEdit::Set { instance_id:instance("choice"), binding:"value".into(), value:json!(["zh"]) },
            UiEdit::Set { instance_id:instance("consent"), binding:"value".into(), value:json!(true) },
        ], action:Some(initial_snapshot.nodes[&instance("submit")].actions["action"].clone()) };
    let prepared = interaction::prepare_interaction(&initial_state, &interaction).unwrap();
    let changes = projection::changes(&initial_snapshot, &prepared.snapshot).unwrap();
    let historical_snapshot = projection::project_historical_surface(&prepared.state).unwrap();
    json!({"fixture_version":1,"wire_input":wire_input,"identity":identity,"initial_snapshot":initial_snapshot,
        "interaction":interaction,"edited_snapshot":prepared.snapshot,"changes":changes,
        "resolved_event":prepared.event.unwrap(),"historical_snapshot":historical_snapshot})
}
