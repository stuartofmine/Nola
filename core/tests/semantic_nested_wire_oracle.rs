// SPDX-License-Identifier: Apache-2.0
//! Shared cases also run through Store's bounded nested validator. No Store
//! dependency is added to Core; these are the actual production wire types.
use nola_core::agent::parts::{CitationRange, SourceEvidence, SourceLocation};
#[test]
fn actual_core_nested_wire_types_match_shared_bounded_validation_cases() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/semantic_nested_wire_cases.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let raw = case["raw"].as_str().unwrap();
        let typed = match case["kind"].as_str().unwrap() {
            "location" => serde_json::from_str::<SourceLocation>(raw).is_ok(),
            "range" => serde_json::from_str::<CitationRange>(raw).is_ok(),
            "evidence" => serde_json::from_str::<SourceEvidence>(raw).is_ok(),
            _ => unreachable!(),
        };
        let accepted = serde_json::from_str::<serde_json::Value>(raw).is_ok() && typed;
        assert_eq!(accepted, case["accepted"].as_bool().unwrap(), "{case}");
    }
}
