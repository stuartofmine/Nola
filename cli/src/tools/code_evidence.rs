// SPDX-License-Identifier: Apache-2.0
//! Descriptive provenance for heuristic code tools. Never authorization or
//! verification evidence; language-service implementations must own their facts.

use serde_json::{json, Value};

pub(super) fn provenance(tool: &str) -> Option<Value> {
    let (backend, limitation) = match tool {
        "code_navigation" | "symbol_search" | "implementation_search" | "call_graph" => (
            "source_scan",
            "Heuristic source matches, not language-server binding resolution. Same names may refer to different symbols; incomplete results do not prove absence.",
        ),
        "code_diagnostics" => (
            "bounded_static_scan",
            "Selected static checks only. No findings do not prove compilation, type correctness, test success, or security.",
        ),
        "context_pack" | "focused_context" => (
            "heuristic_context_projection",
            "Bounded repository evidence and inferred suggestions, not semantic analysis or an executed plan. Suggested commands have not been run; candidate files and inferred risks are not exhaustive.",
        ),
        "rename_symbol" => (
            "identifier_boundary_replacement",
            "Textual identifier matches, not semantic rename. Unrelated same-name symbols may be changed; review the preview and validate affected code.",
        ),
        _ => return None,
    };
    Some(json!({"backend":backend,"semantic_binding_verified":false,
        "build_or_tests_executed":false,"limitation":limitation}))
}

pub(super) fn annotate(tool: &str, mut outcome: super::ToolOutcome) -> super::ToolOutcome {
    if outcome.error { return outcome; }
    let Some(provenance) = provenance(tool) else { return outcome; };
    // Preserve JSON results as JSON for downstream consumers. Plain text gets
    // a compact machine-readable prefix that survives result truncation.
    if let Ok(Value::Object(mut object)) = serde_json::from_str(&outcome.text) {
        object.insert("code_evidence".into(), provenance.clone());
        outcome.text = Value::Object(object).to_string();
    } else {
        outcome.text = format!("code_evidence: {provenance}\n{}", outcome.text);
    }
    match outcome.data.as_mut() {
        Some(Value::Object(object)) => {
            object.insert("code_evidence".into(), provenance);
        }
        None => {
            // Plain-text handlers still need explicit metadata in the canonical
            // result. Do not promote presentation JSON into authoritative data.
            // Core projects explicit data instead of presentation when present;
            // retain the opaque text field that its data=None path provided.
            outcome.data = Some(json!({"text": outcome.text, "code_evidence": provenance}));
        }
        Some(_) => {
            // Preserve an existing non-object business contract verbatim.
            // The provenance remains available in presentation text.
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_results_never_claim_semantic_or_build_verification() {
        for tool in ["code_navigation", "symbol_search", "implementation_search", "call_graph", "code_diagnostics", "rename_symbol", "context_pack", "focused_context"] {
            let evidence = provenance(tool).unwrap();
            assert_eq!(evidence["semantic_binding_verified"], false);
            assert_eq!(evidence["build_or_tests_executed"], false);
        }
        assert!(provenance("bash").is_none());
    }

    #[test]
    fn annotation_preserves_payload_format_and_errors() {
        let result = annotate("rename_symbol", super::super::ok("{\"changed\":2}".into(), "renamed".into()));
        let parsed: Value = serde_json::from_str(&result.text).unwrap();
        assert_eq!(parsed["changed"], 2);
        assert_eq!(parsed["code_evidence"]["backend"], "identifier_boundary_replacement");
        let data = result.data.unwrap();
        assert_eq!(data["code_evidence"]["backend"], "identifier_boundary_replacement");
        assert_eq!(data["text"], result.text);
        assert!(data.get("changed").is_none(), "presentation is not business data");
        let failed = annotate("code_navigation", super::super::error("missing input".into()));
        assert_eq!(failed.text, "missing input");
        let plain = annotate("code_diagnostics", super::super::ok("no findings".into(), "scan".into()));
        assert!(plain.text.starts_with("code_evidence: "));
        assert!(plain.text.ends_with("no findings"));
        assert_eq!(plain.data.unwrap()["code_evidence"]["backend"], "bounded_static_scan");
    }

    #[test]
    fn annotation_preserves_explicit_business_data() {
        let structured = annotate("focused_context", super::super::ok("snippets".into(), "context".into())
            .with_data(json!({"files": ["main.rs"]})));
        let data = structured.data.unwrap();
        assert_eq!(data["files"], json!(["main.rs"]));
        assert_eq!(data["code_evidence"]["backend"], "heuristic_context_projection");
        let array = annotate("focused_context", super::super::ok("snippets".into(), "context".into())
            .with_data(json!(["main.rs"])));
        assert_eq!(array.data, Some(json!(["main.rs"])));
        let failed = annotate("focused_context", super::super::error("unavailable".into())
            .with_data(json!({"reason": "denied"})));
        assert_eq!(failed.data, Some(json!({"reason": "denied"})));
    }
}
