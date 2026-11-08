// SPDX-License-Identifier: Apache-2.0
#[path = "support/a2ui_golden.rs"]
mod fixture;
#[test]
fn shared_wire_projection_interaction_delta_and_history_are_current_core_output() {
    let golden: serde_json::Value = serde_json::from_str(include_str!("../../../tests/fixtures/a2ui/form_interaction_v0_9_1.json")).unwrap();
    assert_eq!(fixture::generate(), golden, "Regenerate explicitly with the a2ui_golden example; never update consumers independently");
}
