// SPDX-License-Identifier: Apache-2.0
// Mechanical extraction: included in the original module to preserve private access.

#[cfg(test)]
mod artifact_media_signature_tests {
    use super::*;

    #[test]
    fn html_download_is_not_advertised_as_an_image() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("photo.png");
        fs::write(&path, b"<!doctype html><title>Not found</title>").unwrap();
        assert_eq!(verified_artifact_mime(&path).as_deref(), Some("application/octet-stream"));
        fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        assert_eq!(verified_artifact_mime(&path).as_deref(), Some("image/png"));
        fs::write(&path, b"%PDF-1.7").unwrap();
        assert_eq!(verified_artifact_mime(&path).as_deref(), Some("application/pdf"));
    }
}

#[cfg(test)]
mod configured_skill_index_admission_tests {
    use super::*;

    fn write_skill(root: &Path, id: &str) {
        fs::create_dir_all(root).expect("isolated Skill directory");
        fs::write(
            root.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: isolated fixture\n---\n\nBody\n"),
        )
        .expect("isolated Skill instructions");
    }

    #[test]
    fn explicit_missing_or_non_directory_root_is_an_error_not_an_empty_catalog() {
        let fixture = tempfile::tempdir().unwrap();
        let valid = fixture.path().join("valid");
        write_skill(&valid, "valid-skill");
        let missing = fixture.path().join("missing");
        let context = SkillRuntimeContext::from_configured_dirs(Some(vec![missing.clone()]));
        assert!(context
            .index()
            .unwrap_err()
            .contains("configured Skill root"));
        assert!(context.catalog_prompt().is_err());
        assert!(configured_skill_context_index(&[valid.clone(), missing]).is_err());

        let file_root = fixture.path().join("ordinary-file");
        fs::write(&file_root, "not a directory").unwrap();
        assert!(configured_skill_context_index(&[file_root])
            .unwrap_err()
            .contains("not a directory"));
        let empty = fixture.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert!(configured_skill_context_index(&[empty]).unwrap().is_empty());
        let disabled = SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        assert!(disabled.index().unwrap().is_empty());
        assert!(!disabled.enabled());
        assert_eq!(configured_skill_context_index(&[valid]).unwrap().len(), 1);
    }

    #[test]
    fn managed_custom_root_is_optional_only_when_the_path_is_absent() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("custom");
        assert!(optional_custom_skill_context_index(&root)
            .unwrap()
            .is_empty());
        fs::create_dir(&root).unwrap();
        assert!(optional_custom_skill_context_index(&root)
            .unwrap()
            .is_empty());
        write_skill(&root.join("fixture"), "custom-skill");
        let items = optional_custom_skill_context_index(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "custom-skill");
        let file_root = fixture.path().join("custom-file");
        fs::write(&file_root, "not a directory").unwrap();
        assert!(optional_custom_skill_context_index(&file_root)
            .unwrap_err()
            .contains("not a directory"));
    }

    #[test]
    fn complete_instruction_limit_is_admitted_but_truncated_metadata_is_not() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let header = "---\nname: bounded-skill\ndescription: bounded fixture\n---\n\n";
        let instructions = format!(
            "{header}{}",
            "x".repeat(MAX_SKILL_BYTES as usize - header.len())
        );
        fs::write(root.join("SKILL.md"), &instructions).unwrap();
        assert_eq!(
            configured_skill_context_index(&[root.to_path_buf()]).unwrap()[0]["id"],
            "bounded-skill"
        );
        fs::write(root.join("SKILL.md"), format!("{instructions}x")).unwrap();
        let error = configured_skill_context_index(&[root.to_path_buf()]).unwrap_err();
        assert!(error.contains("100000-byte instruction limit"), "{error}");
    }

    #[test]
    fn one_hundred_complete_skills_are_allowed_but_the_next_candidate_is_not_partial() {
        let fixture = tempfile::tempdir().unwrap();
        for index in 0..100 {
            write_skill(
                &fixture.path().join(format!("skill-{index:03}")),
                &format!("skill-{index:03}"),
            );
        }
        let roots = [fixture.path().to_path_buf()];
        assert_eq!(configured_skill_context_index(&roots).unwrap().len(), 100);
        write_skill(&fixture.path().join("skill-100"), "skill-100");
        let error = configured_skill_context_index(&roots).unwrap_err();
        assert!(error.contains("discovery limit exceeded (100)"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn linked_directories_are_checked_before_traversal_and_dangling_roots_are_not_absent() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let trusted = fixture.path().join("trusted");
        let outside = fixture.path().join("outside");
        fs::create_dir(&trusted).unwrap();
        write_skill(&outside, "outside-skill");
        symlink(&outside, trusted.join("linked-directory")).unwrap();
        let error = configured_skill_context_index(&[trusted]).unwrap_err();
        assert!(
            error.contains("directory resolved outside its authorized root"),
            "{error}"
        );

        let dangling = fixture.path().join("dangling-custom");
        symlink(fixture.path().join("missing-target"), &dangling).unwrap();
        assert!(optional_custom_skill_context_index(&dangling)
            .unwrap_err()
            .contains("exists but cannot be resolved"));
    }
}

#[cfg(test)]
mod explicit_host_artifact_contract_tests {
    use super::explicit_host_artifacts_from_value;
    use serde_json::json;

    #[test]
    fn accepts_only_valid_explicit_metadata_and_never_path_like_text() {
        let uri = url::Url::from_file_path(std::env::temp_dir().join("device-result.txt"))
            .expect("absolute file URI")
            .to_string();
        let value = json!({
            "text": format!("created {uri}"),
            "_meta": {
                "nola/artifacts": [
                    {"uri": uri, "name": "device-result.txt", "mime_type": "text/plain", "operation": "create"},
                    {"uri": uri, "name": "duplicate", "mime_type": null, "operation": "create"}
                ]
            }
        });
        let artifacts = explicit_host_artifacts_from_value(&value).expect("valid contract");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].name, "device-result.txt");

        assert!(
            explicit_host_artifacts_from_value(&json!({"text": format!("created {uri}")}))
                .expect("plain text has no contract")
                .is_empty()
        );
        assert!(explicit_host_artifacts_from_value(&json!({
            "_meta": {"nola/artifacts": [{"uri": "relative.txt", "name": "bad", "mime_type": null, "operation": "create"}]}
        }))
        .is_err());
    }
}

#[cfg(test)]
mod mac_native_computer_bridge_tests {
    use super::{decode_mac_native_bridge_response, mac_native_computer_bridge_supports};
    use serde_json::json;

    #[test]
    fn native_bridge_preserves_typed_commit_and_failed_readback_evidence() {
        for passed in [true, false] {
            let response = json!({
                "type": "device_tool_result", "request_id": "call", "is_error": !passed,
                "output": {"automation_id": "record", "persisted": true},
                "execution": {
                    "local_stop": null,
                    "state": if passed { "completed" } else { "partial" },
                    "effects": [{"resource_uri": "nola://automation/record", "operation": "automation.upsert",
                        "before_version": null, "after_version": "revision:3"}],
                    "checks": [{"kind":"postcondition", "resource_uri": "nola://automation/record", "version": "revision:3",
                        "predicate": "durable_definition_matches", "passed": passed}]
                }
            });
            let outcome = decode_mac_native_bridge_response(&response, "call").unwrap();
            assert_eq!(outcome.error, !passed);
            assert_eq!(outcome.data.unwrap()["automation_id"], "record");
            let receipt = outcome.execution.unwrap();
            assert_eq!(
                receipt.effects[0].after_version.as_deref(),
                Some("revision:3")
            );
            assert_eq!(receipt.checks[0].passed, passed);
            assert!(
                decode_mac_native_bridge_response(&response, "foreign-call")
                    .unwrap()
                    .error
            );
        }
    }

    #[test]
    fn native_bridge_does_not_promote_business_text_or_malformed_receipts_to_evidence() {
        let mut response = json!({
            "type": "device_tool_result", "request_id": "call", "is_error": false,
            "output": {"persisted": true, "read_back_verified": true,
                "execution": {"state": "completed"}}
        });
        assert!(decode_mac_native_bridge_response(&response, "call")
            .unwrap()
            .execution
            .is_none());
        response["execution"] = json!({"state": "completed", "local_stop": null, "effects": [], "checks": [{
            "kind":"postcondition", "resource_uri": "nola://automation/record", "version": "", "predicate": "readback", "passed": true
        }]});
        assert!(
            decode_mac_native_bridge_response(&response, "call")
                .unwrap()
                .error
        );
    }

    #[test]
    fn permission_bound_keyboard_tools_always_route_back_to_mac_app() {
        for name in [
            "computer_type_text",
            "computer_press_key",
            "computer_hotkey",
            "computer_key_down",
            "computer_key_up",
            "computer_copy_selection",
        ] {
            assert!(
                mac_native_computer_bridge_supports(name),
                "{name} escaped the Mac app bridge"
            );
        }
        assert!(mac_native_computer_bridge_supports("nola_orchestrate"));
    }
}

#[cfg(test)]
mod required_argument_tests {
    use super::{
        required_tool_argument_error, specs, validate_tool_request,
        validation_pipeline_hides_result, ToolRequest, MAX_WRITE_BYTES,
    };
    use serde_json::{Map, Value};

    #[test]
    fn rejects_empty_write_and_bash_calls_before_execution() {
        for (name, expected_fields) in [
            ("write_file", ["path"].as_slice()),
            ("append_file", ["path", "content"].as_slice()),
            ("bash", ["command"].as_slice()),
        ] {
            let spec = specs()
                .into_iter()
                .find(|spec| spec.name == name)
                .expect("tool spec");
            let request = ToolRequest {
                id: format!("test-{name}"),
                name: name.to_string(),
                args: Map::new(),
            };
            let error = required_tool_argument_error(&spec, &request)
                .expect("empty required arguments must fail");
            for field in expected_fields {
                assert!(error.contains(field), "{error}");
            }
            let outcome = validate_tool_request(&request).expect("invalid request outcome");
            assert!(outcome.error);
            assert!(outcome.text.contains("tool_execution_skipped: true"));
            assert!(outcome
                .text
                .contains("deterministic_failure_category: invalid_tool_arguments"));
        }
    }

    #[test]
    fn rejects_write_file_payload_shape_before_execution() {
        for args in [
            serde_json::json!({"path": "/tmp/result.txt"}),
            serde_json::json!({
                "path": "/tmp/result.txt",
                "content": "text",
                "json_content": {"value": 1}
            }),
        ] {
            let request = ToolRequest {
                id: "write-payload".to_string(),
                name: "write_file".to_string(),
                args: args.as_object().unwrap().clone(),
            };
            let outcome = validate_tool_request(&request).expect("invalid request outcome");
            assert!(outcome.error);
            assert!(outcome.text.contains("tool_execution_skipped: true"));
            assert!(outcome
                .text
                .contains("deterministic_failure_category: invalid_tool_arguments"));
        }
    }

    #[test]
    fn write_file_contract_rejects_foreign_operation_kind_and_keeps_explicit_preview() {
        use nola_core::agent::host::HostCapabilityEffect;

        let catalog = crate::agent_host_driver::build_cli_host_catalog("write-contract-test");
        let writer = catalog.capabilities.get("write_file").unwrap();
        let valid = serde_json::json!({"path": "transformer.cpp", "content": "// fixture"});
        assert!(writer.validate_input(&valid).is_ok());
        assert_eq!(writer.effect_for(&valid), HostCapabilityEffect::Mutate);
        assert_eq!(writer.input_schema["additionalProperties"], false);
        assert!(writer.input_schema["properties"]
            .get("operation_kind")
            .is_none());
        for kind in ["create", "mutate", "inspect", "validate"] {
            let mut invalid = valid.clone();
            invalid["operation_kind"] = Value::String(kind.to_string());
            assert!(writer
                .validate_input(&invalid)
                .unwrap_err()
                .contains("unexpected field(s): operation_kind"));
        }
        let mut preview = valid;
        preview["dry_run"] = Value::Bool(true);
        assert!(writer.validate_input(&preview).is_ok());
        assert_eq!(
            writer.effect_for(&preview),
            HostCapabilityEffect::Investigate
        );
        let description = writer.input_schema["description"].as_str().unwrap();
        assert!(description.contains("Create or replace"));
        assert!(description.contains("literal file text"));
        assert!(description.contains("edit_file"));
    }

    #[test]
    fn file_write_schemas_describe_byte_capacity_without_character_constraints() {
        for name in ["write_file", "append_file"] {
            let spec = specs()
                .into_iter()
                .find(|spec| spec.name == name)
                .expect("file write tool spec");
            let content = &spec.parameters["properties"]["content"];
            assert!(content.get("maxLength").is_none());
            let description = content["description"].as_str().unwrap();
            assert!(description.contains(&MAX_WRITE_BYTES.to_string()));
            assert!(description.contains("UTF-8 bytes (not characters)"));
            assert_eq!(
                spec.parameters["properties"]["ephemeral"]["type"],
                "boolean"
            );
            assert_eq!(spec.parameters["properties"]["ephemeral"]["default"], false);
        }
        let write_file = specs()
            .into_iter()
            .find(|spec| spec.name == "write_file")
            .expect("write_file tool spec");
        assert_eq!(write_file.parameters["required"], serde_json::json!(["path", "content"]));
        assert!(write_file.parameters.get("oneOf").is_none());
        assert!(write_file.parameters["properties"].get("json_content").is_none());
        let bash = specs()
            .into_iter()
            .find(|spec| spec.name == "bash")
            .expect("bash tool spec");
        assert_eq!(bash.parameters["properties"]["command"]["maxLength"], super::BASH_COMMAND_MAX_CHARS);
    }

    #[test]
    fn accepts_empty_replacement_text_for_deletions() {
        let spec = specs()
            .into_iter()
            .find(|spec| spec.name == "edit_file")
            .expect("edit_file spec");
        let request = ToolRequest {
            id: "delete-range".to_string(),
            name: "edit_file".to_string(),
            args: Map::from_iter([
                ("path".to_string(), Value::String("src/lib.rs".to_string())),
                (
                    "old_text".to_string(),
                    Value::String("obsolete".to_string()),
                ),
                ("new_text".to_string(), Value::String(String::new())),
            ]),
        };
        assert_eq!(required_tool_argument_error(&spec, &request), None);
        assert!(validate_tool_request(&request).is_none());
    }

    #[test]
    fn rejects_validation_pipelines_that_hide_the_real_result() {
        for command in [
            "cargo check 2>&1 | head -40",
            "cargo test | tail -20",
            "python -m pytest | /usr/bin/head -n 5",
        ] {
            assert!(validation_pipeline_hides_result(command), "{command}");
            let request = ToolRequest {
                id: "validation".to_string(),
                name: "bash".to_string(),
                args: Map::from_iter([
                    ("command".to_string(), Value::String(command.to_string())),
                    (
                        "operation_kind".to_string(),
                        Value::String("validate".to_string()),
                    ),
                ]),
            };
            let outcome = validate_tool_request(&request).expect("guard outcome");
            assert!(outcome.error);
            assert!(outcome.text.contains("truncated_validation_output"));
        }
        assert!(!validation_pipeline_hides_result("cargo test"));
        assert!(!validation_pipeline_hides_result(
            "set -o pipefail; cargo test 2>&1 | tail -20"
        ));
        assert!(!validation_pipeline_hides_result(
            "cargo test 2>&1 | tee /tmp/cargo-test.log"
        ));
        for command in [
            "git diff -- README.md | head -120",
            "git status --short | tail -20",
        ] {
            assert!(!validation_pipeline_hides_result(command), "{command}");
            let request = ToolRequest {
                id: "read-only-pipeline".to_string(),
                name: "bash".to_string(),
                args: Map::from_iter([
                    ("command".to_string(), Value::String(command.to_string())),
                    (
                        "operation_kind".to_string(),
                        Value::String("validate".to_string()),
                    ),
                ]),
            };
            assert!(validate_tool_request(&request).is_none(), "{command}");
        }
    }

    #[test]
    fn read_only_shell_commands_are_not_forced_into_an_unavailable_structured_path() {
        let request = ToolRequest {
            id: "bash-read-only".to_string(),
            name: "bash".to_string(),
            args: Map::from_iter([
                (
                    "command".to_string(),
                    Value::String("cat ~/Desktop/nola/README.md".to_string()),
                ),
                (
                    "operation_kind".to_string(),
                    Value::String("inspect".to_string()),
                ),
            ]),
        };
        assert!(validate_tool_request(&request).is_none());
    }

    #[test]
    fn inspection_pipelines_may_bound_display_output() {
        let request = ToolRequest {
            id: "bash-inspect".to_string(),
            name: "bash".to_string(),
            args: Map::from_iter([
                (
                    "command".to_string(),
                    Value::String("git diff | head -40".to_string()),
                ),
                (
                    "operation_kind".to_string(),
                    Value::String("inspect".to_string()),
                ),
            ]),
        };
        assert!(validate_tool_request(&request).is_none());
    }
}

#[cfg(test)]
mod engineering_effect_contract_tests {
    use super::{
        tool_effect_declaration_with_read_only, tool_requires_validation_after_mutation,
        tool_transaction_boundary,
    };
    use nola_core::agent::host::{HostCapabilityEffect, HostCapabilityTransactionBoundary};
    use serde_json::json;

    #[test]
    fn shell_effect_defaults_to_unknown_execution_without_inferring_from_command_text() {
        let declaration = tool_effect_declaration_with_read_only("bash", false);
        assert_eq!(
            declaration.resolve(&json!({"command":"arbitrary command text"})),
            HostCapabilityEffect::Execute
        );
        assert_eq!(
            declaration.resolve(&json!({
                "command":"arbitrary command text",
                "operation_kind":"inspect"
            })),
            HostCapabilityEffect::Investigate
        );
        assert_eq!(
            declaration.resolve(&json!({
                "command":"arbitrary command text",
                "operation_kind":"mutate"
            })),
            HostCapabilityEffect::Mutate
        );
    }

    #[test]
    fn shell_calls_require_declared_purpose_without_inferring_it_from_command_text() {
        use super::{tool_is_read_only, validate_tool_request, ToolRequest};
        let declaration = tool_effect_declaration_with_read_only("bash", false);
        let catalog = crate::agent_host_driver::build_cli_host_catalog("optional-shell-purpose");
        let shell = catalog.capabilities.get("bash").unwrap();
        for command in ["ls -la", "pwd", "cargo test", "rm file", "搜索 current", "opaque-tool --mode=x"] {
            let mut args = json!({"command":command});
            let mut request = ToolRequest {
                id: "optional-shell-purpose".into(),
                name: "bash".into(),
                args: args.as_object().unwrap().clone(),
            };
            assert!(validate_tool_request(&request).is_some(), "{command}");
            assert!(shell.validate_input(&args).is_err(), "{command}");
            // Previously admitted calls retain conservative effect accounting.
            assert_eq!(declaration.resolve(&args), HostCapabilityEffect::Execute);
            args["operation_kind"] = json!("execute");
            request.args = args.as_object().unwrap().clone();
            assert!(validate_tool_request(&request).is_none(), "{command}");
            assert!(shell.validate_input(&args).is_ok(), "{command}");
            assert_eq!(declaration.resolve(&args), HostCapabilityEffect::Execute);
            assert!(!tool_is_read_only("bash"));
        }
        // Foreign or mistyped declarations remain recoverable schema errors.
        for purpose in [json!("create"), json!(true), json!(null)] {
            let args = json!({"command":"ls -la", "operation_kind":purpose});
            assert!(shell.validate_input(&args).is_err());
        }
    }

    #[test]
    fn dry_run_effect_uses_the_host_declared_boolean_argument() {
        let declaration = tool_effect_declaration_with_read_only("write_file", false);
        assert_eq!(
            declaration.resolve(&json!({"path":"result", "dry_run":true})),
            HostCapabilityEffect::Investigate
        );
        assert_eq!(
            declaration.resolve(&json!({"path":"result", "dry_run":false})),
            HostCapabilityEffect::Mutate
        );
        assert!(!tool_requires_validation_after_mutation("write_file"));
    }

    #[test]
    fn task_observation_does_not_create_a_side_effect_recovery_boundary() {
        for name in ["list_tasks", "read_task", "wait_task", "bash_background_wait"] {
            assert_eq!(tool_effect_declaration_with_read_only(name, true).resolve(&json!({})),
                HostCapabilityEffect::Investigate);
        }
        for name in ["spawn_agent", "send_agent_message"] {
            assert_eq!(tool_effect_declaration_with_read_only(name, true).resolve(&json!({})),
                HostCapabilityEffect::Execute);
        }
    }

    #[test]
    fn host_state_writes_declare_canonical_readback_validation() {
        for name in ["memory_save", "nola_orchestrate"] {
            assert_eq!(
                tool_effect_declaration_with_read_only(name, false).resolve(&json!({})),
                HostCapabilityEffect::MutateAndValidate
            );
            assert!(!tool_requires_validation_after_mutation(name));
        }
    }

    #[test]
    fn host_state_mutations_do_not_request_workspace_checkpoints() {
        assert_eq!(
            tool_transaction_boundary("memory_save"),
            HostCapabilityTransactionBoundary::HostState
        );
        assert_eq!(
            tool_transaction_boundary("nola_orchestrate"),
            HostCapabilityTransactionBoundary::HostState
        );
        assert_eq!(
            tool_transaction_boundary("write_file"),
            HostCapabilityTransactionBoundary::Workspace
        );
        assert_eq!(
            tool_transaction_boundary("bash"),
            HostCapabilityTransactionBoundary::Workspace
        );
    }
}

#[cfg(test)]
mod skill_schema_tests {
    use super::{
        tool_schemas_for_host, tool_schemas_with_host_availability, HostCapabilityAvailability,
        SkillRuntimeContext,
    };
    use serde_json::Value;

    #[test]
    fn model_sees_only_the_exact_skill_invocation_protocol() {
        let skills = SkillRuntimeContext::default();
        let schemas = tool_schemas_with_host_availability(true, &skills, || {
            HostCapabilityAvailability::from_captured(true, None)
        });
        let names = schemas
            .iter()
            .filter_map(|schema| {
                schema
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        assert!(names.contains(&"Skill".to_string()));
        assert!(!names.contains(&"skill_context".to_string()));
        assert!(names.contains(&"skill_skip".to_string()));
        let invoke = schemas
            .iter()
            .find(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some("Skill")
            })
            .expect("Skill schema");
        assert_eq!(
            invoke
                .pointer("/function/parameters/properties/skill/type")
                .and_then(Value::as_str),
            Some("string")
        );
        assert!(invoke
            .pointer("/function/parameters/properties/skill/oneOf")
            .is_none());
    }

    #[test]
    fn disabled_skill_availability_omits_invocation_and_skip_together() {
        for (configured, manifest) in [
            (false, None),
            (false, Some(r#"{"schema_version":1,"skills_enabled":true}"#)),
            (true, Some(r#"{"schema_version":1,"skills_enabled":false}"#)),
        ] {
            let skills = SkillRuntimeContext::from_configured_dirs((!configured).then(Vec::new));
            let schemas = tool_schemas_with_host_availability(true, &skills, || {
                HostCapabilityAvailability::from_captured(configured, manifest)
            });
            let names = schemas
                .iter()
                .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
                .collect::<Vec<_>>();
            for skill_name in ["Skill", "skill_invoke", "skill_update", "skill_skip"] {
                assert!(!names.contains(&skill_name), "unexpected {skill_name}");
            }
            assert!(!names.contains(&"skill_context"));
            assert!(names.contains(&"grep"));
        }
    }

    #[test]
    fn grep_schema_accepts_a_single_file_path() {
        let schemas = tool_schemas_for_host();
        let grep = schemas
            .iter()
            .find(|schema| schema.pointer("/function/name").and_then(Value::as_str) == Some("grep"))
            .expect("grep schema");

        assert_eq!(
            grep.pointer("/function/parameters/properties/path/type")
                .and_then(Value::as_str),
            Some("string")
        );
    }
}

#[cfg(test)]
mod host_capability_availability_tests {
    use super::{
        tool_schemas_with_host_availability, HostCapabilityAvailability, SkillRuntimeContext,
    };
    use serde_json::Value;
    use std::cell::Cell;

    const SKILL_NAMES: &[&str] = &["Skill", "skill_invoke", "skill_update", "skill_skip"];

    #[test]
    fn pure_availability_keeps_defaults_only_without_a_manifest_and_rejects_invalid_manifests() {
        for manifest in [
            None,
            Some("invalid JSON"),
            Some(r#"{"schema_version":2,"skills_enabled":false,"memory_enabled":false}"#),
            Some(r#"{"schema_version":"1","skills_enabled":false}"#),
            Some(r#"{"schema_version":1,"skills_enabled":null,"memory_enabled":"false"}"#),
        ] {
            let enabled = HostCapabilityAvailability::from_captured(true, manifest);
            for name in SKILL_NAMES.iter().copied().chain([
                "memory_search",
                "memory_save",
                "computer_screenshot",
                "chrome_native_tabs",
                "browser_open",
                "grep",
            ]) {
                assert_eq!(enabled.enabled(name), manifest.is_none(), "invalid manifest must fail closed for {name}");
            }
            let no_configured_skills = HostCapabilityAvailability::from_captured(false, manifest);
            for name in SKILL_NAMES {
                assert!(!no_configured_skills.enabled(name));
            }
            assert_eq!(no_configured_skills.enabled("grep"), manifest.is_none());
        }
    }

    #[test]
    fn pure_manifest_flags_preserve_each_capability_branch() {
        let disabled = HostCapabilityAvailability::from_captured(
            true,
            Some(
                r#"{"schema_version":1,"skills_enabled":false,"memory_enabled":false,"native_computer_enabled":false,"native_browser_enabled":false,"managed_browser_enabled":false}"#,
            ),
        );
        for name in SKILL_NAMES.iter().copied().chain([
            "memory_search",
            "memory_save",
            "computer_screenshot",
            "chrome_native_tabs",
            "browser_open",
        ]) {
            assert!(!disabled.enabled(name), "unexpected enabled {name}");
        }
        for name in [
            "grep",
            "read_file",
            "computer",
            "chrome_native",
            "browser",
            "unknown_tool",
        ] {
            assert!(
                disabled.enabled(name),
                "unrelated capability changed for {name}"
            );
        }
        let managed_only = HostCapabilityAvailability::from_captured(
            true,
            Some(
                r#"{"schema_version":1,"managed_browser_enabled":true,"native_browser_enabled":false}"#,
            ),
        );
        assert!(managed_only.enabled("browser_open"));
        assert!(!managed_only.enabled("chrome_native_tabs"));
        let native_only = HostCapabilityAvailability::from_captured(
            true,
            Some(
                r#"{"schema_version":1,"managed_browser_enabled":false,"native_browser_enabled":true}"#,
            ),
        );
        assert!(native_only.enabled("browser_open"));
        assert!(native_only.enabled("chrome_native_tabs"));
    }

    #[test]
    fn directory_captures_once_and_next_build_refreshes_all_skill_entries() {
        let skills = SkillRuntimeContext::default();
        let captures = Cell::new(0);
        let supplier = || {
            let capture = captures.get();
            captures.set(capture + 1);
            let manifest = match capture {
                0 => None,
                1 => Some(r#"{"schema_version":1,"skills_enabled":false}"#),
                _ => Some(r#"{"schema_version":1,"skills_enabled":true}"#),
            };
            HostCapabilityAvailability::from_captured(capture != 0, manifest)
        };
        let first = tool_schemas_with_host_availability(true, &skills, &supplier);
        assert_eq!(captures.get(), 1);
        for name in ["Skill", "skill_update", "skill_skip"] {
            assert!(!first.iter().any(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some(name)
            }));
        }
        let manifest_disabled = tool_schemas_with_host_availability(true, &skills, &supplier);
        assert_eq!(captures.get(), 2);
        for name in ["Skill", "skill_update", "skill_skip"] {
            assert!(!manifest_disabled.iter().any(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some(name)
            }));
        }
        let next = tool_schemas_with_host_availability(true, &skills, &supplier);
        assert_eq!(captures.get(), 3);
        for name in ["Skill", "skill_update", "skill_skip"] {
            assert!(next.iter().any(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some(name)
            }));
        }
        let legacy = tool_schemas_with_host_availability(false, &skills, &supplier);
        assert_eq!(captures.get(), 4);
        for name in ["Skill", "skill_skip"] {
            assert!(legacy.iter().any(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some(name)
            }));
        }
        assert!(!legacy.iter().any(|schema| {
            schema.pointer("/function/name").and_then(Value::as_str) == Some("skill_update")
        }));
    }
}

#[cfg(test)]
mod safe_read_only_bash_tests {
    use super::{
        bash_command_is_safe_read_only, bash_command_path_policy_check,
        bash_command_paths_stay_in_allowed_dirs, BashCommandPathPolicy,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nola-safe-read-only-bash-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn value() -> usize { 1 }\n").unwrap();
        fs::write(root.join(".env"), "TOKEN=secret\n").unwrap();
        root
    }

    fn allowed(command: &str, root: &PathBuf) -> bool {
        bash_command_is_safe_read_only(command, root.to_str(), std::slice::from_ref(root))
    }

    #[test]
    fn allows_scoped_read_only_commands_and_chains() {
        let root = fixture();
        assert!(allowed("ls -la", &root));
        assert!(allowed("cat -n src/lib.rs", &root));
        assert!(allowed("wc -l src/lib.rs && cat -n src/lib.rs", &root));
        assert!(allowed("cd src && pwd && rg value lib.rs", &root));
        assert!(allowed("git status --short", &root));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_single_quoted_awk_programs_without_treating_fields_as_paths() {
        let root = fixture();
        fs::write(root.join("sales.csv"), "date,revenue\n2026-07-01,42\n").unwrap();
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "awk -F',' 'NR>1 {r+=$2} END {print r}' sales.csv",
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_multiline_python_source_with_xml_namespaces_and_workspace_paths() {
        let root = fixture();
        fs::write(root.join("budget.xlsx"), "fixture\n").unwrap();
        let command = format!(
            "python3 -c \"\nimport zipfile\nwith zipfile.ZipFile('{}/budget.xlsx') as archive:\n    namespace = '{{http://schemas.openxmlformats.org/spreadsheetml/2006/main}}'\n    print('.//row', namespace, archive.namelist())\n\"",
            root.display()
        );
        assert!(bash_command_paths_stay_in_allowed_dirs(
            &command,
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ordinary_external_paths_do_not_block_explicitly_approved_programs() {
        let root = fixture();
        for command in [
            "python3 -c \"open('/etc/passwd').read()\"",
            "node -e \"require('fs').readFileSync('/etc/passwd')\"",
        ] {
            assert!(
                bash_command_paths_stay_in_allowed_dirs(
                    command,
                    root.to_str(),
                    std::slice::from_ref(&root)
                ),
                "{command}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_writes_shell_expansion_and_unsafe_read_variants() {
        let root = fixture();
        for command in [
            "cat src/lib.rs > copy.rs",
            "cat $(pwd)/src/lib.rs",
            "find . -exec cat {} ;",
            "sed -i '' s/value/changed/ src/lib.rs",
            "sort -o sorted.txt src/lib.rs",
            "tree -o tree.txt",
            "git reset --hard",
            "rg --pre 'sh -c whoami' value .",
        ] {
            assert!(!allowed(command, &root), "{command}");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_paths_outside_workspace_but_marks_sensitive_files() {
        let root = fixture();
        assert!(allowed("cat /etc/passwd", &root));
        assert!(allowed("ls ..", &root));
        assert!(!allowed("cat .env", &root));
        assert!(!allowed("cat ~/.ssh/id_rsa", &root));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn distinguishes_unsupported_shell_syntax_from_ordinary_external_paths() {
        let root = fixture();
        assert_eq!(
            bash_command_path_policy_check(
                "python3 - <<'PY'\nprint('ok')\nPY",
                root.to_str(),
                std::slice::from_ref(&root),
            ),
            BashCommandPathPolicy::UnsupportedSyntax,
        );
        assert_eq!(
            bash_command_path_policy_check(
                "cat /etc/passwd",
                root.to_str(),
                std::slice::from_ref(&root),
            ),
            BashCommandPathPolicy::Allowed,
        );
        assert_eq!(
            bash_command_path_policy_check(
                "python3 src/lib.rs",
                root.to_str(),
                std::slice::from_ref(&root),
            ),
            BashCommandPathPolicy::Allowed,
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_workspace_build_and_git_commands_with_fd_duplication() {
        let root = fixture();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        let root_text = root.to_string_lossy();
        for command in [
            format!("cd {root_text} && cargo check --quiet 2>&1 | head -100"),
            format!("cargo check --manifest-path {root_text}/Cargo.toml 2>&1 | head -100"),
            format!("git -C {root_text} diff --stat HEAD 2>&1 | tail -30"),
        ] {
            assert!(
                bash_command_paths_stay_in_allowed_dirs(
                    &command,
                    root.to_str(),
                    std::slice::from_ref(&root)
                ),
                "{command}"
            );
        }
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "cat src/lib.rs > /tmp/nola-copy.rs",
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn secret_content_policy_is_not_a_directory_grant_and_git_metadata_is_safe() {
        let root = fixture();
        fs::write(root.join(".env"), "FIXTURE_SECRET=example").unwrap();
        let dirs = std::slice::from_ref(&root);
        assert_eq!(
            bash_command_path_policy_check("cat .env", root.to_str(), dirs),
            BashCommandPathPolicy::SensitiveResource
        );
        assert_eq!(
            bash_command_path_policy_check("git check-ignore -v .env", root.to_str(), dirs),
            BashCommandPathPolicy::Allowed
        );
        assert_eq!(
            bash_command_path_policy_check("git check-ignore /etc/.env", root.to_str(), dirs),
            BashCommandPathPolicy::Allowed
        );
        assert_eq!(
            bash_command_path_policy_check("git diff -- .env", root.to_str(), dirs),
            BashCommandPathPolicy::SensitiveResource
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_find_grouping_with_escaped_parentheses_inside_workspace() {
        let root = fixture();
        assert!(bash_command_paths_stay_in_allowed_dirs(
            r#"find . -type f \( -name "*.rs" -o -name "*.toml" \) -not -path "*/target/*" | head -100"#,
            root.to_str(),
            std::slice::from_ref(&root),
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_escaped_regex_operators_inside_double_quotes() {
        let root = fixture();
        for command in [
            r#"grep -rn "class A\|struct A" src"#,
            r#"git diff -- src/lib.rs | grep -n "value=\|result=""#,
        ] {
            assert_eq!(
                bash_command_path_policy_check(command, root.to_str(), std::slice::from_ref(&root),),
                BashCommandPathPolicy::Allowed,
                "{command}",
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_exit_status_display_after_workspace_validation() {
        let root = fixture();
        fs::write(root.join("validate.py"), "print('validated')\n").unwrap();
        fs::write(root.join("input.csv"), "value\n42\n").unwrap();
        let command = format!(
            "cd {} && python3 validate.py --input input.csv && echo \"--- exit=$? ---\"",
            root.display()
        );
        assert!(bash_command_paths_stay_in_allowed_dirs(
            &command,
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        assert!(!bash_command_paths_stay_in_allowed_dirs(
            "echo \"$(cat /etc/passwd)\"",
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn authorized_path_policy_accepts_workspace_redirects_but_read_only_policy_does_not() {
        let root = fixture();
        let inside = format!("printf 'ok\\n' > '{}/result.txt'", root.display());
        assert!(bash_command_paths_stay_in_allowed_dirs(
            &inside,
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        assert!(!bash_command_is_safe_read_only(
            &inside,
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        assert!(bash_command_paths_stay_in_allowed_dirs(
            "printf 'no\\n' > '/tmp/nola-outside.txt'",
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_shell_line_continuations_without_weakening_path_boundaries() {
        let root = fixture();
        let skill_root = root.join("skill");
        fs::create_dir_all(&skill_root).unwrap();
        fs::write(skill_root.join("create.py"), "print('ok')\n").unwrap();
        fs::write(root.join("spec.json"), "{}\n").unwrap();
        let command = format!(
            "python3 {}/create.py \\\n+  --spec spec.json \\\n+  --output briefing.pptx",
            skill_root.display()
        );
        assert!(bash_command_paths_stay_in_allowed_dirs(
            &command,
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let outside = format!("cat \\\n+  {}/../secret.txt", root.display());
        assert!(bash_command_paths_stay_in_allowed_dirs(
            &outside,
            root.to_str(),
            std::slice::from_ref(&root)
        ));
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[test]
fn transaction_roundtrip_preserves_data_and_execution_separately_from_text() {
    use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState};
    let mut outcome =
        ok_data(json!({"answer": 42}), "summary".into()).with_execution(ExecutionReceipt {
            state: ExecutionState::Deferred,
            ..Default::default()
        });
    outcome.full_output_ref = Some("/fixture/output.json".into());
    let saved = tool_outcome_transaction_value(&outcome);
    let restored = tool_outcome_from_transaction(&saved).unwrap();
    assert_eq!(restored.data, outcome.data);
    assert_eq!(restored.execution, outcome.execution);
    assert_eq!(restored.text, outcome.text);
    assert_eq!(restored.full_output_ref, outcome.full_output_ref);
    let mut malformed = saved;
    malformed["execution"]["state"] = json!("forged_state");
    assert!(tool_outcome_from_transaction(&malformed).is_none());
}

#[cfg(test)]
mod pending_permission_trace_tests {
    use super::*;
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn trace(ask: &str, sequence: u64) -> Value {
        json!({"conversation_id":"session","trace_id":"trace","run_id":"run","turn_id":"turn",
            "sequence":sequence,"kind":"tool.permission.requested","span_id":"original-call",
            "payload":{"permission_request_id":ask,"call_id":"original-call","tool_name":"write_file"}})
    }
    fn request(id: &str) -> ToolRequest {
        ToolRequest {
            id: id.into(),
            name: "write_file".into(),
            args: Default::default(),
        }
    }

    #[test]
    fn canonical_trace_identity_cannot_be_fabricated_or_consumed_by_wrong_tool() {
        let _serial = TEST_LOCK.lock().unwrap();
        let _scope = enter_host_permission_session("session");
        let mut missing_id = trace("ask", 1);
        missing_id["payload"]
            .as_object_mut()
            .unwrap()
            .remove("permission_request_id");
        set_pending_native_permission_trace(missing_id);
        assert!(
            take_pending_host_permission_trace("session", &request("ask"), &json!({}), "why")
                .is_none()
        );
        set_pending_native_permission_trace(trace("ask", 2));
        assert!(take_pending_host_permission_trace(
            "session",
            &request("wrong-ask"),
            &json!({}),
            "why"
        )
        .is_none());
        let mut wrong_tool = request("ask");
        wrong_tool.name = "delete_file".into();
        assert!(
            take_pending_host_permission_trace("session", &wrong_tool, &json!({}), "why").is_none()
        );
        let mut wrong_call = trace("ask", 3);
        wrong_call["payload"]["call_id"] = json!("other-call");
        wrong_call["span_id"] = json!("other-call");
        set_pending_native_permission_trace(wrong_call);
        let taken =
            take_pending_host_permission_trace("session", &request("ask"), &json!({}), "why")
                .unwrap();
        assert_eq!(taken["sequence"], 2);
        assert_eq!(taken["payload"]["call_id"], "original-call");
    }

    #[test]
    fn registry_crosses_threads_without_crossing_conversations() {
        let _serial = TEST_LOCK.lock().unwrap();
        let mut trace = trace("thread-ask", 1);
        trace["conversation_id"] = json!("thread-conversation");
        std::thread::spawn(move || set_pending_native_permission_trace(trace))
            .join()
            .unwrap();
        assert!(take_pending_host_permission_trace(
            "other-conversation",
            &request("thread-ask"),
            &json!({}),
            "why"
        )
        .is_none());
        assert_eq!(
            take_pending_host_permission_trace(
                "thread-conversation",
                &request("thread-ask"),
                &json!({}),
                "why"
            )
            .unwrap()["conversation_id"],
            "thread-conversation"
        );
    }

    #[test]
    fn reissue_requires_newer_sequence_and_scopes_isolate_generation_and_capacity() {
        let _serial = TEST_LOCK.lock().unwrap();
        let _scope = enter_host_permission_session("session");
        set_pending_native_permission_trace(trace("ask", 3));
        set_pending_native_permission_trace(trace("ask", 2));
        set_pending_native_permission_trace(trace("ask", 4));
        {
            let _nested = enter_host_permission_session("other-session");
            assert!(take_pending_host_permission_trace(
                "other-session",
                &request("ask"),
                &json!({}),
                "why"
            )
            .is_none());
        }
        assert_eq!(
            take_pending_host_permission_trace("session", &request("ask"), &json!({}), "why")
                .unwrap()["sequence"],
            4
        );
        for index in 0..129 {
            set_pending_native_permission_trace(trace(&format!("ask-{index}"), index));
        }
        assert!(take_pending_host_permission_trace(
            "session",
            &request("ask-128"),
            &json!({}),
            "why"
        )
        .is_none());
        assert!(take_pending_host_permission_trace(
            "session",
            &request("ask-0"),
            &json!({}),
            "why"
        )
        .is_some());
        let mut next_turn = trace("next-turn-ask", 200);
        next_turn["turn_id"] = json!("next-turn");
        set_pending_native_permission_trace(next_turn);
        assert!(take_pending_host_permission_trace(
            "session",
            &request("ask-1"),
            &json!({}),
            "why"
        )
        .is_none());
        assert!(take_pending_host_permission_trace(
            "session",
            &request("next-turn-ask"),
            &json!({}),
            "why"
        )
        .is_some());
        let mut oversized = trace("too-large", 201);
        oversized["payload"]["summary"] = json!("x".repeat(32 * 1024));
        set_pending_native_permission_trace(oversized);
        assert!(take_pending_host_permission_trace(
            "session",
            &request("too-large"),
            &json!({}),
            "why"
        )
        .is_none());
    }
}
