#[test]
fn concise_discovery_and_saved_output_descriptions_preserve_contracts() {
    let specs = specs();
    for (name, budget, required) in [
        ("tool_search", 300, vec!["activate", "does not execute or grant permission", "exposed schemas"]),
        ("Skill", 220, vec!["exact advertised Skill ID", "neither executes nor delegates"]),
        ("skill_update", 260, vec!["Core-supplied", "Never invent IDs"]),
        ("tool_output_read", 600, vec!["exact full_output_ref", "preview_end_byte_offset", "next_offset", "unread ranges do not prove full coverage"]),
    ] {
        let spec = specs.iter().find(|spec| spec.name == name).unwrap();
        assert!(spec.description.len() <= budget, "{name} description exceeds {budget} UTF-8 bytes");
        for contract in required {
            assert!(spec.description.contains(contract), "{name} lost {contract}");
        }
    }
}

#[test]
fn saved_output_search_and_read_share_structured_boundary_errors() {
    let root = tempfile::tempdir().unwrap();
    let store = ToolOutputStore::for_session(&root.path().join("fixture.db"), "owner");
    let other = ToolOutputStore::for_session(&root.path().join("fixture.db"), "other");
    store.persist("owned").unwrap();
    let foreign = other.persist("foreign").unwrap();
    let args = json!({"path": foreign, "pattern": "foreign"});
    for (capability, result) in [
        ("tool_output_read", tool_output_read_in_store(args.as_object().unwrap(), &store)),
        ("tool_output_search", tool_output_search_in_store(args.as_object().unwrap(), &store)),
    ] {
        assert!(result.error);
        let payload: Value = serde_json::from_str(&result.text).unwrap();
        assert_eq!(payload["error"], "tool_output_outside_store");
        assert_eq!(payload["tool_execution_skipped"], true);
        assert_eq!(payload["recovery"]["capability"], capability);
    }
}

#[test]
fn saved_output_schemas_publish_the_same_bounds_as_execution() {
    let catalog = specs();
    for (name, parameter, minimum, maximum) in [
        ("tool_output_list", "limit", 1, MAX_TOOL_OUTPUT_LIST),
        ("tool_output_read", "offset", 0, i64::MAX),
        ("tool_output_read", "max_bytes", 1, MAX_TOOL_OUTPUT_READ_REQUEST_BYTES),
        ("tool_output_search", "context_lines", 0, MAX_TOOL_OUTPUT_CONTEXT_LINES),
        ("tool_output_find", "context_lines", 0, MAX_TOOL_OUTPUT_CONTEXT_LINES),
        ("tool_output_search", "max_matches", 1, MAX_TOOL_OUTPUT_MATCHES),
        ("tool_output_find", "max_matches", 1, MAX_TOOL_OUTPUT_MATCHES),
        ("tool_output_find", "max_outputs", 1, MAX_TOOL_OUTPUT_LIST),
    ] {
        let schema = &catalog.iter().find(|spec| spec.name == name).unwrap().parameters;
        assert_eq!(schema["properties"][parameter]["minimum"], minimum, "{name}.{parameter}");
        assert_eq!(schema["properties"][parameter]["maximum"], maximum, "{name}.{parameter}");
    }
}

#[test]
fn large_tool_preview_exposes_exact_utf8_continuation_offset() {
    let original = "界".repeat(800);
    let outcome = clamp_outcome_with_limit(
        ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
            display: "large output".to_string(),
            text: original.clone(),
            error: false,
            artifacts: Vec::new(),
            activation: None,
        },
        700,
    );
    let envelope: Value = serde_json::from_str(&outcome.text).expect("structured envelope");
    let preview = envelope["output_preview"].as_str().expect("preview");
    let offset = envelope["preview_end_byte_offset"]
        .as_u64()
        .expect("byte offset") as usize;

    assert_eq!(offset, preview.len());
    assert_eq!(&original.as_bytes()[..offset], preview.as_bytes());
    assert!(offset > preview.chars().count());
}

#[test]
fn invalid_validation_pipeline_is_rejected_by_permission_preflight() {
    let request = ToolRequest {
        id: "validation-preflight".to_string(),
        name: "bash".to_string(),
        args: serde_json::json!({
            "command": "cargo test -q 2>&1 | tail -20",
            "operation_kind": "validate",
            "execution_mode": "foreground"
        })
        .as_object()
        .cloned()
        .unwrap(),
    };

    let outcome = preflight_tool_request(&request).expect("deterministic rejection");
    assert!(outcome.error);
    assert!(outcome
        .text
        .contains("deterministic_failure_category: truncated_validation_output"));
}
