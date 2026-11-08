// SPDX-License-Identifier: Apache-2.0
//! Generate exact readonly candidate projections for the shared renderer fixture.
use nola_domain::{generative_ui::*, generative_ui_tool::UiToolInput};
fn main() {
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::args().nth(1).expect("fixture path")).unwrap()).unwrap();
    let identity: UiSurfaceIdentity = serde_json::from_value(fixture["identity"].clone()).unwrap();
    let wire: UiToolInput = serde_json::from_value(fixture["wire_input"].clone()).unwrap();
    let state = nola_kernel::generative_ui::state::transition_messages(None, &identity, &wire).unwrap();
    let interaction: UiInteraction = serde_json::from_value(fixture["interaction"].clone()).unwrap();
    let mut vectors = Vec::new();
    for mask in 1..(1 << interaction.edits.len()) {
        let edits: Vec<_> = interaction.edits.iter().enumerate().filter(|(index,_)| mask & (1 << index) != 0).map(|(_,edit)| edit.clone()).collect();
        let input = UiDraftPreviewInput { identity:identity.clone(), expected_surface_revision:state.revision, edits:edits.clone(),draft_generation:1 };
        let mut snapshot = nola_kernel::generative_ui::interaction::prepare_interaction(&state, &UiInteraction {
            identity:identity.clone(),expected_surface_revision:state.revision,edits,action:None }).unwrap().snapshot;
        snapshot.revision = state.revision;
        vectors.push(serde_json::json!({"input":input,"preview":UiDraftPreview {identity:identity.clone(),base_revision:state.revision,draft_generation:1,snapshot}}));
    }
    println!("{}",serde_json::to_string_pretty(&vectors).unwrap());
}
