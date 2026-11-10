// SPDX-License-Identifier: Apache-2.0
// Mechanical extraction: included in the original module to preserve private access.

#[cfg(test)]
#[test]
fn mobile_attachment_text_decodes_utf8_and_utf16_bom() {
    let utf8 =
        extract_mobile_attachment_text(b"\xEF\xBB\xBFalpha\nbeta", "notes.md", "text/markdown")
            .expect("UTF-8 text");
    assert_eq!(utf8.parser, "cli_plain_text");
    assert_eq!(utf8.text, "alpha\nbeta");
    let utf16 =
        extract_mobile_attachment_text(&[0xFF, 0xFE, b'h', 0, b'i', 0], "notes.txt", "text/plain")
            .expect("UTF-16 text");
    assert_eq!(utf16.text, "hi");
}

#[cfg(test)]
#[test]
fn mobile_attachment_text_cleans_html() {
    let parsed = extract_mobile_attachment_text(
        b"<!doctype html><html><body><h1>Title</h1><script>hidden()</script></body></html>",
        "page.html",
        "text/html",
    )
    .expect("HTML text");
    assert_eq!(parsed.parser, "cli_html_text");
    assert!(parsed.text.contains("Title"));
    assert!(!parsed.text.contains("hidden()"));
}

#[cfg(test)]
#[test]
fn mobile_attachment_text_rejects_binary_documents_instead_of_emitting_garbage() {
    for (bytes, filename, mime_type, expected_format) in [
        (
            &b"%PDF-1.7\nopaque"[..],
            "report.bin",
            "application/octet-stream",
            "pdf",
        ),
        (
            &b"PK\x03\x04opaque"[..],
            "report.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "archive_document",
        ),
        (&b"\x89PNG\r\n\x1a\n"[..], "scan.png", "image/png", "image"),
    ] {
        let error = extract_mobile_attachment_text(bytes, filename, mime_type)
            .err()
            .expect("binary attachment must be rejected");
        assert_eq!(error.code, "unsupported_binary_format");
        assert_eq!(error.format, expected_format);
    }
}

#[cfg(test)]
mod host_permission_broker_tests {
    use super::*;

    fn evidence(tool: &str) -> crate::host_tool_permission_evidence::EvidenceScope {
        crate::host_tool_permission_evidence::enter(json!({"catalog_revision":1,
            "tool":{"tool_id":tool,"route":"agent_host","schema":{"type":"function",
                "function":{"name":tool,"parameters":{"type":"object"}}}}}))
    }

    fn pending_trace(request: &ToolRequest, sequence: u64, call_id: &str) -> Value {
        json!({
            "schema_version":1,"trace_id":"trace","run_id":"run","turn_id":"turn",
            "conversation_id":"session","message_id":"message","event_id":format!("event-{sequence}"),
            "sequence":sequence,"emitted_at":"2026-09-08T00:00:00Z",
            "kind":"tool.permission.requested","status":"waiting","visibility":"summary",
            "span_id":call_id,
            "payload":{"permission_request_id":request.id,"call_id":call_id,"tool_name":request.name}
        })
    }

    fn round_trip(decision_name: &str, expected: ToolPermissionResolution) {
        round_trip_resolution(
            json!({"type":"authorization","decision":decision_name}),
            expected,
        );
    }

    fn round_trip_resolution(resolution: Value, expected: ToolPermissionResolution) {
        let request = ToolRequest {
            id: format!("broker-{}", new_id("request")),
            name: "delete_file".to_string(),
            args: serde_json::from_value(json!({"path": "notes.txt"})).expect("tool args"),
        };
        let (event_tx, event_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _evidence = crate::host_tool_permission_evidence::enter(json!({
                "catalog_revision": 1,
                "tool":{"tool_id":"delete_file","route":"agent_host",
                    "schema":{"type":"function","function":{"name":"delete_file","parameters":{"type":"object"}}}}
            }));
            let mut trace = pending_trace(&request, 1, "original-call");
            trace["conversation_id"] = json!("test-session");
            crate::tools::set_pending_native_permission_trace(trace);
            wait_for_host_permission(
                &request,
                "test-host",
                "test-client",
                "test-session",
                &event_tx,
                None,
                &[nola_core::agent::host::PermissionInstruction {
                    permission_request_id: "original-permission".into(),
                    instruction: "Keep the original file".into(),
                }],
                true,
            )
        });
        let frame = String::from_utf8(
            event_rx
                .recv_timeout(StdDuration::from_secs(2))
                .expect("permission event"),
        )
        .expect("utf8 event");
        let event: Value = serde_json::from_str(
            frame
                .trim()
                .strip_prefix("data: ")
                .expect("SSE data prefix"),
        )
        .expect("permission JSON");
        assert_eq!(event["tool_catalog_evidence"]["catalog_revision"],1);
        assert_eq!(event["tool_catalog_evidence"]["tool"]["tool_id"],event["tool_name"]);
        let decision_value = json!({
            "protocol_version": event["protocol_version"],
            "nonce": event["nonce"],
            "request_id": event["id"],
            "host_id": event["host_id"],
            "client_id": event["client_id"],
            "session_id": event["session_id"],
            "tool_name": event["tool_name"],
            "resolution": resolution,
        });
        assert_eq!(
            event["permission_instructions"][0]["instruction"],
            "Keep the original file"
        );
        assert_eq!(event["safe_review_text"], "delete_file: \"notes.txt\"");
        assert!(
            event.get("args").is_none(),
            "permission envelope must not include raw args"
        );
        assert_eq!(event["trace"]["payload"]["arguments"], json!({}));
        let mut invalid = decision_value.clone();
        invalid["resolution"] = json!({"type":"intervention","intervention":{"type":"provide_instruction","instruction":"字".repeat(1366)}});
        assert!(
            submit_host_permission_decision(serde_json::from_value(invalid).unwrap()).is_err(),
            "over-budget reply must be rejected before consuming the waiter"
        );
        submit_host_permission_decision(
            serde_json::from_value(decision_value.clone()).expect("valid decision"),
        )
        .expect("decision accepted");
        assert_eq!(handle.join().expect("broker thread"), expected);
        submit_host_permission_decision(
            serde_json::from_value(decision_value.clone()).expect("valid replay"),
        )
        .expect("lost HTTP acknowledgement retries return the same transport acceptance");
        let mut conflicting = decision_value;
        conflicting["resolution"] = json!({"type":"intervention","intervention":{"type":"provide_instruction","instruction":"different response"}});
        assert!(
            submit_host_permission_decision(serde_json::from_value(conflicting).unwrap()).is_err()
        );
    }

    #[test]
    fn permission_timeout_retires_transport_without_denial_and_reissue_accepts_only_new_nonce() {
        let _evidence = evidence("write_file");
        let request = ToolRequest {
            id: new_id("recoverable-ask"),
            name: "write_file".into(),
            args: serde_json::from_value(json!({"path":"original.txt"})).unwrap(),
        };
        let (tx, rx) = mpsc::channel();
        crate::tools::set_pending_native_permission_trace(pending_trace(
            &request,
            1,
            "original-call",
        ));
        let result = wait_for_host_permission_with_timeout(
            &request,
            "host",
            "client",
            "session",
            &tx,
            None,
            &[],
            true,
            StdDuration::ZERO,
        );
        assert_eq!(
            result,
            ToolPermissionResolution::TransportInterrupted(
                nola_host_protocol::PermissionTransportInterruptionReason::Timeout
            )
        );
        fn decode(frame: Vec<u8>) -> Value {
            serde_json::from_str(
                String::from_utf8(frame)
                    .unwrap()
                    .trim()
                    .strip_prefix("data: ")
                    .unwrap(),
            )
            .unwrap()
        }
        fn response(event: &Value) -> HostPermissionResponse {
            serde_json::from_value(json!({
                "protocol_version":event["protocol_version"],"nonce":event["nonce"],
                "request_id":event["id"],"host_id":event["host_id"],"client_id":event["client_id"],
                "session_id":event["session_id"],"tool_name":event["tool_name"],
                "resolution":{"type":"authorization","decision":"allow_once"}
            }))
            .unwrap()
        }
        let expired = decode(rx.recv().unwrap());
        assert!(submit_host_permission_decision(response(&expired)).is_err());
        let next = request.clone();
        let handle = thread::spawn(move || {
            let _evidence = evidence("write_file");
            crate::tools::set_pending_native_permission_trace(pending_trace(
                &next,
                2,
                "original-call",
            ));
            wait_for_host_permission_with_timeout(
                &next,
                "host",
                "client",
                "session",
                &tx,
                None,
                &[],
                true,
                StdDuration::from_secs(2),
            )
        });
        let renewed = decode(rx.recv_timeout(StdDuration::from_secs(2)).unwrap());
        assert_eq!(renewed["id"], expired["id"]);
        assert_eq!(renewed["safe_review_text"], expired["safe_review_text"]);
        assert!(renewed.get("args").is_none());
        assert_ne!(renewed["nonce"], expired["nonce"]);
        assert!(submit_host_permission_decision(response(&expired)).is_err());
        submit_host_permission_decision(response(&renewed)).unwrap();
        assert_eq!(handle.join().unwrap(), ToolPermissionResolution::Allow);
    }

    #[test]
    fn disconnected_permission_stream_reports_interruption_without_waiting_for_timeout() {
        let _evidence = evidence("write_file");
        let request = ToolRequest {
            id: new_id("disconnected-ask"),
            name: "write_file".into(),
            args: Default::default(),
        };
        let (tx, rx) = mpsc::channel();
        drop(rx);
        crate::tools::set_pending_native_permission_trace(pending_trace(
            &request,
            1,
            "original-call",
        ));
        assert_eq!(
            wait_for_host_permission_with_timeout(
                &request,
                "host",
                "client",
                "session",
                &tx,
                None,
                &[],
                true,
                StdDuration::ZERO
            ),
            ToolPermissionResolution::TransportInterrupted(
                nola_host_protocol::PermissionTransportInterruptionReason::Disconnected
            )
        );
    }

    #[test]
    fn multiple_canonical_asks_reach_actual_broker_in_reverse_order() {
        let (tx, rx) = mpsc::channel();
        let first = ToolRequest {
            id: new_id("ask-first"),
            name: "write_file".into(),
            args: Default::default(),
        };
        let second = ToolRequest {
            id: new_id("ask-second"),
            name: "delete_file".into(),
            args: Default::default(),
        };
        let _scope = crate::tools::enter_host_permission_session("session");
        crate::tools::set_pending_native_permission_trace(pending_trace(&first, 1, "call-first"));
        crate::tools::set_pending_native_permission_trace(pending_trace(&second, 2, "call-second"));
        for (request, call_id) in [(&second, "call-second"), (&first, "call-first")] {
            let _evidence = evidence(&request.name);
            assert_eq!(
                wait_for_host_permission_with_timeout(
                    request,
                    "host",
                    "client",
                    "session",
                    &tx,
                    None,
                    &[],
                    true,
                    StdDuration::ZERO
                ),
                ToolPermissionResolution::TransportInterrupted(
                    nola_host_protocol::PermissionTransportInterruptionReason::Timeout
                )
            );
            let frame =
                String::from_utf8(rx.recv_timeout(StdDuration::from_secs(1)).unwrap()).unwrap();
            let event: Value =
                serde_json::from_str(frame.trim().strip_prefix("data: ").unwrap()).unwrap();
            assert_eq!(event["id"], request.id);
            assert_eq!(
                event["trace"]["payload"]["permission_request_id"],
                request.id
            );
            assert_eq!(event["trace"]["payload"]["call_id"], call_id);
            assert_eq!(event["trace"]["payload"]["tool_name"], request.name);
        }
    }

    #[test]
    fn host_permission_allow_resumes_original_execution_once() {
        round_trip("allow_once", ToolPermissionResolution::Allow);
    }

    #[test]
    fn approval_preview_is_fail_closed_for_scripts_file_content_and_credential_urls() {
        fn request(name: &str, args: Value) -> ToolRequest {
            ToolRequest {
                id: "preview".into(),
                name: name.into(),
                args: args.as_object().unwrap().clone(),
            }
        }
        assert_eq!(
            host_permission_safe_review_text(&request("delete_file", json!({"path":"notes.txt"}))),
            Some("delete_file: \"notes.txt\"".into())
        );
        let configured = crate::mcp_connection_authority::bind_configurations(
            &[json!({"server_id":"preview","enabled":true,"endpoint":"http://127.0.0.1:1/mcp"})],
            "preview-host", &uuid::Uuid::new_v4().to_string()).unwrap();
        crate::mcp_host::register_configured_servers(&configured).unwrap();
        let reference = configured[0]["connection_ref"].clone();
        let preview = host_permission_safe_review_text(&request("mcp_connect", json!({"connection_ref":reference}))).unwrap();
        assert!(preview.contains("http://127.0.0.1:1"));
        assert!(preview.contains("Configuration SHA-256:"));
        assert!(host_permission_safe_review_text(&request("mcp_connect",
            json!({"connection_ref":reference,"command":"extra script"}))).is_none());
        assert!(host_permission_safe_review_text(&request("mcp_connect", json!({"connection_ref":"unknown"}))).is_none());
        assert_eq!(
            host_permission_safe_review_text(&request(
                "delete_file",
                json!({"path":"notes.txt", "dry_run":true})
            )),
            Some("preview delete_file: \"notes.txt\"".into())
        );
        assert!(host_permission_safe_review_text(&request("agent_plan_file", json!({
            "root":"/tmp/owned-project", "action":"write", "slug":"audit", "content":"private plan", "dry_run":false
        }))).unwrap().contains("Content SHA-256:"));
        assert!(host_permission_safe_review_text(&request("bash", json!({"command":"python3 -m unittest discover -s tests", "cwd":"/tmp/owned-project", "operation_kind":"execute"})))
            .unwrap().contains("Command: python3 -m unittest"));
        let skill_progress = host_permission_safe_review_text(&request("skill_update", json!({
            "action":"progress", "run_id":"analyze-data", "step_id":"inspect", "completed":true
        }))).unwrap();
        assert!(skill_progress.contains("analyze-data"));
        let source = "text(12.50 + 7.50 - 2);\ntext('done');";
        assert!(host_permission_safe_review_text(&request("code_mode_exec", json!({"source":source})))
            .unwrap().contains(source));
        assert!(host_permission_safe_review_text(&request("code_mode_exec", json!({
            "source":"text(1)", "unknown":"must reject"
        }))).is_none());
        assert!(host_permission_safe_review_text(&request("code_mode_wait", json!({
            "cell_id":"owned-cell", "after_output_sequence":0,"wait_ms":1000,"terminate":false
        }))).unwrap().contains("\"terminate\":false"));
        for (name, args) in [
            (
                "bash",
                json!({"command":"echo fixture-secret-value", "operation_kind":"execute"}),
            ),
            (
                "append_file",
                json!({"path":"notes.txt", "content":"fixture-secret-value"}),
            ),
            ("delete_file", json!({"path":"secret-token.txt"})),
            (
                "delete_file",
                json!({"path":"notes.txt", "unknown_effect":"fixture-secret-value"}),
            ),
            (
                "delete_file",
                json!({"path":"notes.txt", "dry_run":"false"}),
            ),
            ("unknown_tool", json!({"path":"notes.txt"})),
            ("web_fetch", json!({"url":"https://user:pass@example.com/"})),
            (
                "web_fetch",
                json!({"url":"https://example.com/?token=fixture-secret-value"}),
            ),
            (
                "web_fetch",
                json!({"url":"https://example.com/private/opaque"}),
            ),
        ] {
            assert_eq!(
                host_permission_safe_review_text(&request(name, args)),
                None,
                "{name} must fail closed"
            );
        }
        assert_eq!(
            host_permission_safe_review_text(&request(
                "web_fetch",
                json!({"url":"https://example.com/"})
            )),
            Some("web_fetch: \"https://example.com/\"".into())
        );
    }

    #[test]
    fn ui_permission_preview_binds_publication_without_exposing_form_values() {
        fn preview(args: Value) -> Option<String> {
            host_permission_safe_review_text(&ToolRequest {
                id: "ui-preview".into(), name: "render_ui".into(),
                args: args.as_object().unwrap().clone(),
            })
        }
        let args = json!({"expected_revision":2,"messages":[{
            "version":"v0.9.1","updateDataModel":{
                "surfaceId":"private-surface-name","path":"/private-field",
                "value":"fixture-private-form-value"
            }
        }]});
        let review = preview(args.clone()).unwrap();
        assert!(review.contains("Expected revision: 2"));
        assert!(review.contains("Protocol messages: 1"));
        assert!(review.contains("does not authorize form actions"));
        for private in ["private-surface-name", "private-field", "fixture-private-form-value"] {
            assert!(!review.contains(private));
        }
        assert_eq!(preview(args.clone()).unwrap(), review);
        let mut changed = args.clone();
        changed["messages"][0]["updateDataModel"]["value"] = json!("changed");
        assert_ne!(preview(changed).unwrap(), review);
        let mut unknown = args;
        unknown["execute_business_action"] = json!(true);
        assert!(preview(unknown).is_none());
        assert!(preview(json!({"expected_revision":0,"messages":[]})).is_none());
        assert!(preview(json!({"expected_revision":"2","messages":[{}]})).is_none());
    }

    #[test]
    fn write_permission_preview_binds_exact_target_mode_and_body_digest_without_body() {
        fn preview(args: Value) -> Option<String> {
            host_permission_safe_review_text(&ToolRequest {id:"write-preview".into(), name:"write_file".into(),
                args:args.as_object().unwrap().clone()})
        }
        let body = "fixture-private-body-你好";
        let args = json!({"path":"/tmp/owned/existing.txt","content":body,"expected_sha256":"a".repeat(64)});
        let value = preview(args.clone()).unwrap();
        assert!(value.contains("/tmp/owned/existing.txt"));
        assert!(value.contains("replace full file if expected version matches"));
        assert!(value.contains(&format!("UTF-8 bytes: {}",body.len())));
        assert!(!value.contains(body));
        use sha2::{Digest,Sha256};
        assert!(value.contains(&format!("Content SHA-256: {:x}",Sha256::digest(body.as_bytes()))));
        let mut changed = args.clone(); changed["content"] = json!("other");
        assert_ne!(preview(changed).unwrap(),value);
        let mut dry = args.clone(); dry["dry_run"] = json!(true);
        assert!(preview(dry).unwrap().contains("preview only; no write"));
        for (key,value) in [("path",json!(" notes.txt ")), ("content",json!(null)),
            ("expected_sha256",json!("invalid")),("dry_run",json!("false")),("ephemeral",json!(1)),
            ("unknown_effect",json!("other operation"))] {
            let mut invalid=args.clone(); invalid[key]=value;
            assert!(preview(invalid).is_none(),"{key}");
        }
    }

    #[test]
    fn browser_permission_envelope_never_serializes_unreviewable_script_args() {
        let _evidence = evidence("bash");
        let request = ToolRequest {
            id: new_id("secret-script-ask"), name: "bash".into(),
            args: serde_json::from_value(json!({"command":"curl -H 'Authorization: Bearer fixture-secret-value' https://example.com/", "operation_kind":"execute"})).unwrap(),
        };
        let (tx, rx) = mpsc::channel();
        crate::tools::set_pending_native_permission_trace(pending_trace(
            &request,
            1,
            "script-call",
        ));
        assert_eq!(
            wait_for_host_permission_with_timeout(
                &request,
                "host",
                "client",
                "session",
                &tx,
                None,
                &[],
                true,
                StdDuration::ZERO
            ),
            ToolPermissionResolution::TransportInterrupted(
                nola_host_protocol::PermissionTransportInterruptionReason::Timeout
            )
        );
        let frame = String::from_utf8(rx.recv().unwrap()).unwrap();
        assert!(!frame.contains("fixture-secret-value"));
        assert!(!frame.contains("Authorization: Bearer"));
        let event: Value =
            serde_json::from_str(frame.trim().strip_prefix("data: ").unwrap()).unwrap();
        assert!(event["safe_review_text"].is_null());
        assert!(event.get("args").is_none());
        assert_eq!(event["trace"]["payload"]["arguments"], json!({}));
    }

    #[test]
    fn host_permission_deny_resumes_original_execution_once() {
        round_trip("deny", ToolPermissionResolution::Deny);
    }

    #[test]
    fn host_permission_conversation_grant_resumes_original_execution_once() {
        round_trip(
            "allow_conversation",
            ToolPermissionResolution::AllowConversation,
        );
    }
    #[test]
    fn host_permission_custom_preserves_instruction_without_authorizing() {
        round_trip_resolution(
            json!({"type":"intervention","intervention":{"type":"provide_instruction","instruction":"先预览，不要修改"}}),
            ToolPermissionResolution::Custom("先预览，不要修改".into()),
        );
    }
}

#[cfg(test)]
mod ui_locale_cookie_tests {
    use super::{parse_ui_locale_cookie, preferred_request_language};
    use crate::i18n::Language;

    #[test]
    fn explicit_ui_cookie_overrides_browser_language_for_all_supported_locales() {
        for (value, expected) in [
            ("zh-Hans", Language::ZhHans),
            ("en", Language::En),
            ("de", Language::De),
            ("es", Language::Es),
            ("fr", Language::Fr),
            ("ja", Language::Ja),
            ("ru", Language::Ru),
        ] {
            let cookie = format!(
                "nola_web_session=opaque-auth; nola_ui_locale={value}; nola_web_csrf=opaque-csrf"
            );
            assert_eq!(
                preferred_request_language(Some(&cookie), Some("en-US,zh-CN;q=0.9")),
                Some(expected)
            );
        }
    }

    #[test]
    fn clearing_or_invalid_preference_restores_browser_then_host_fallback() {
        for cookie in [
            None,
            Some(""),
            Some("nola_ui_locale="),
            Some("nola_ui_locale=system"),
            Some("nola_ui_locale=invalid"),
        ] {
            assert_eq!(
                preferred_request_language(cookie, Some("fr-FR;q=0.9,en;q=0.7")),
                Some(Language::Fr)
            );
            // None is handled only by request_language's existing host-default fallback.
            assert_eq!(preferred_request_language(cookie, None), None);
        }
        assert_eq!(preferred_request_language(None, Some("*")), None);
    }

    #[test]
    fn cookie_values_are_exact_enums_not_html_encoded_tags_or_language_aliases() {
        for value in [
            "EN",
            "zh",
            "zh-CN",
            "en-US",
            " en",
            "\"en\"",
            "%65n",
            "en=fr",
            "<script>alert(1)</script>",
        ] {
            assert_eq!(
                parse_ui_locale_cookie(&format!("nola_ui_locale={value}")),
                None
            );
        }
        assert_eq!(
            parse_ui_locale_cookie("other_nola_ui_locale=fr; nola_web_session=ja"),
            None
        );
        assert_eq!(parse_ui_locale_cookie("NOLA_UI_LOCALE=de"), None);
        assert_eq!(
            parse_ui_locale_cookie("nola_ui_locale=fr; nola_ui_locale=ja"),
            None
        );
        assert_eq!(
            parse_ui_locale_cookie("nola_ui_locale=fr; nola_ui_locale=fr"),
            None
        );
        assert_eq!(
            preferred_request_language(Some("nola_ui_locale=fr; nola_ui_locale=ja"), Some("de-DE")),
            Some(Language::De)
        );
    }
}

#[cfg(test)]
#[test]
fn model_test_skips_mcp_refresh_and_clears_unrelated_catalog() {
    let mut settings = AgentSettings::default();
    settings.mcp_servers = vec![json!({"id":"broken-server","enabled":true})];
    prepare_turn_mcp_servers(&mut settings, true, |_| {
        panic!("a chat-only model test must not refresh MCP")
    })
    .expect("model test has no MCP dependency");
    assert!(settings.mcp_servers.is_empty());
}

#[cfg(test)]
#[test]
fn web_skill_startup_bootstraps_once_without_repairing_initialized_resources() {
    let initialized = std::cell::Cell::new(false);
    let writes = std::cell::Cell::new(0);
    for expected in [true, false] {
        let did_initialize = initialize_web_skill_sources_if_never_initialized(
            None,
            || {
                Ok(vec![
                    PathBuf::from("/fixture/built-in"),
                    PathBuf::from("/fixture/custom"),
                ])
            },
            |_| initialized.get(),
            || {
                writes.set(writes.get() + 1);
                initialized.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(did_initialize, expected);
    }
    assert_eq!(writes.get(), 1);
    assert!(!initialize_web_skill_sources_if_never_initialized(
        None,
        || Ok(vec![
            PathBuf::from("/fixture/built-in"),
            PathBuf::from("/fixture/custom")
        ]),
        |path| path == std::path::Path::new("/fixture/built-in"),
        || panic!("an initialized root cannot be repaired implicitly"),
    )
    .unwrap());
}

#[cfg(test)]
#[test]
fn web_skill_startup_skips_all_explicit_sources_and_propagates_initialization_errors() {
    for selection in [Vec::new(), vec![PathBuf::from("/fixture/explicit")]] {
        assert!(!initialize_web_skill_sources_if_never_initialized(
            Some(&selection),
            || panic!("explicit roots must not read managed source paths"),
            |_| panic!("explicit roots must not inspect managed resources"),
            || panic!("explicit roots must not synchronize managed assets"),
        )
        .unwrap());
    }
    assert_eq!(
        initialize_web_skill_sources_if_never_initialized(
            None,
            || Ok(vec![
                PathBuf::from("/fixture/built-in"),
                PathBuf::from("/fixture/custom")
            ]),
            |_| false,
            || Err("injected initialization failure".to_string()),
        ),
        Err("injected initialization failure".to_string())
    );
}

#[cfg(test)]
#[test]
fn web_skill_runtime_honors_host_roots_without_granting_filesystem_access() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("only-this-skill");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("SKILL.md"), "---\nname: runtime-owned-fixture\ndescription: isolated source\n---\nFixture instructions.\n").unwrap();
    let settings = AgentSettings::default();
    let skills =
        web_skill_runtime_for_settings(Some(std::slice::from_ref(&root)), &settings).unwrap();
    assert_eq!(skills.configured_dirs(), Some(std::slice::from_ref(&root)));
    assert_eq!(
        skills
            .index()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        vec!["runtime-owned-fixture"]
    );
    let disabled = web_skill_runtime_for_settings(Some(&[]), &settings).unwrap();
    assert!(!disabled.enabled());
    let missing = fixture.path().join("missing-root");
    assert!(web_skill_runtime_for_settings(Some(&[missing]), &settings).is_err());
    let mut config = Config::for_test().unwrap();
    config.allowed_dirs = vec![fixture.path().join("user-authorized")];
    let allowed = config.allowed_dirs.clone();
    config.skill_dirs = skills.configured_dirs().map(<[PathBuf]>::to_vec);
    apply_agent_tool_settings(&mut config, &settings);
    assert_eq!(config.allowed_dirs, allowed);
    assert_eq!(config.skill_dirs, Some(vec![root]));
}

#[cfg(test)]
fn web_local_candidate_schemas(settings: &AgentSettings) -> Vec<Value> {
    // Standalone catalog tests explicitly use the managed default selection.
    web_local_candidate_schemas_from_catalog(
        settings,
        &crate::tools::tool_schemas_with_skills(
            &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
        ),
    )
}

#[cfg(test)]
fn resolve_web_agent_host_tools_for_turn(
    settings: &AgentSettings,
    permission_rules: &[WebPermissionRule],
) -> Vec<Value> {
    resolve_web_agent_host_tools_from_supplier(settings, permission_rules, || {
        crate::tools::tool_schemas_with_skills(
            &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
        )
    })
}

#[cfg(test)]
fn resolve_web_agent_host_tools_from_supplier(
    settings: &AgentSettings,
    permission_rules: &[WebPermissionRule],
    supply_catalog: impl FnOnce() -> Vec<Value>,
) -> Vec<Value> {
    // A catalog may legitimately change between resolutions. Both routes in
    // this resolution must nevertheless consume the same captured exposure.
    let catalog = supply_catalog();
    resolve_web_agent_host_tools_from_catalog(settings, permission_rules, &catalog)
}

#[cfg(test)]
#[test]
fn web_chat_catalog_does_not_change_with_user_wording() {
    let settings = AgentSettings::default();
    let catalog = tool_schemas();
    let search = resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog);
    let names = search
        .iter()
        .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let plain = resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog);
    let plain_names = plain
        .iter()
        .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();

    assert_eq!(names, plain_names);
    assert_eq!(search, plain);
    assert!(names.contains(&"system_time"));
    assert!(names.contains(&"web_fetch"));
    assert!(names.contains(&"web_search"));
}

#[cfg(test)]
#[test]
fn web_context_estimate_uses_host_exposure_without_removing_deferred_tools() {
    let mut settings = AgentSettings::default();
    settings.enable_local_dev_tools = true;
    settings.host_tools_enabled = true;
    let catalog = tool_schemas();
    let available = resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog);
    let candidates = candidates_from_schemas(&available, ToolRoute::AgentHost);
    let mut restricted = candidates.clone();
    restrict_web_tool_candidates(&mut restricted, false, None);
    assert!(restricted.is_empty(), "explicit Host disablement wins over Web defaults");
    restricted = candidates.clone();
    restrict_web_tool_candidates(&mut restricted, true, Some(&["read_file".into()]));
    assert_eq!(restricted.len(), 1);
    assert_eq!(restricted[0].schema["function"]["name"], "read_file");
    restrict_web_tool_candidates(&mut restricted, true, Some(&[]));
    assert!(restricted.is_empty());
    restricted = candidates;
    restrict_web_tool_candidates(&mut restricted, true, None);
    assert_eq!(restricted.len(), available.len(), "discovery keeps the authorized catalog");
    let initial = crate::agent_host_driver::initial_model_tool_schemas(available.clone());
    assert!(!initial.is_empty());
    assert!(initial.len() < available.len());
    let names = initial
        .iter()
        .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"tool_search"));
    assert!(!names.contains(&"web_search"));
    assert!(names.contains(&"write_file"), "foundational editing remains visible; visibility is not authorization");
    assert!(!names.contains(&"memory_save"));
    assert!(available.iter().any(|schema| {
        schema.pointer("/function/name").and_then(Value::as_str) == Some("web_search")
    }));
    assert!(available.iter().any(|schema| {
        schema.pointer("/function/name").and_then(Value::as_str) == Some("write_file")
    }));
    assert_eq!(
        available,
        resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog),
    );
}

#[cfg(test)]
#[test]
fn web_host_catalog_is_captured_once_for_both_routes_and_refreshes_next_resolution() {
    use std::cell::{Cell, RefCell};

    let mut settings = AgentSettings::default();
    settings.enable_local_dev_tools = false;
    settings.host_tools_enabled = true;
    let schema = |name: &str, description: &str| {
        json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": {"type": "object", "properties": {}}
            }
        })
    };
    let first = vec![
        schema("web_fetch", "first exposure"),
        schema("write_file", "first exposure"),
    ];
    let second = vec![schema("web_fetch", "next exposure")];
    let source = RefCell::new(first.clone());
    let reads = Cell::new(0);
    let available = resolve_web_agent_host_tools_from_supplier(&settings, &[], || {
        reads.set(reads.get() + 1);
        let captured = source.borrow().clone();
        // Model a legal exposure refresh immediately after the source read.
        *source.borrow_mut() = second.clone();
        captured
    });
    assert_eq!(reads.get(), 1);
    assert_eq!(
        available,
        resolve_web_agent_host_tools_from_catalog(&settings, &[], &first),
    );
    assert!(available.iter().any(|schema| {
        schema.pointer("/function/name").and_then(Value::as_str) == Some("write_file")
    }));
    assert!(available.iter().all(|schema| {
        schema
            .pointer("/function/description")
            .and_then(Value::as_str)
            == Some("first exposure")
    }));

    let refreshed = resolve_web_agent_host_tools_from_supplier(&settings, &[], || {
        reads.set(reads.get() + 1);
        source.borrow().clone()
    });
    assert_eq!(reads.get(), 2);
    assert_eq!(
        refreshed,
        resolve_web_agent_host_tools_from_catalog(&settings, &[], &second),
    );
    assert_ne!(available, refreshed);
    assert!(refreshed.iter().all(|schema| {
        schema.pointer("/function/name").and_then(Value::as_str) != Some("write_file")
    }));
}

#[cfg(test)]
#[test]
fn web_agent_exposes_explicit_local_write_tool_when_developer_tools_are_enabled() {
    let mut settings = AgentSettings::default();
    settings.enable_local_dev_tools = true;
    let tools = resolve_web_agent_host_tools_for_turn(&settings, &[]);
    let names = tools
        .iter()
        .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        names.contains(&"write_file"),
        "explicit Web Agent write_file was removed from the resolved catalog: {names:?}"
    );
}

#[cfg(test)]
#[test]
fn web_permission_rules_reach_core_without_mutating_the_tool_catalog() {
    let mut settings = AgentSettings::default();
    settings.enable_local_dev_tools = true;
    let rule = WebPermissionRule {
        id: 1,
        tool_name: "write_file".to_string(),
        behavior: "deny".to_string(),
        match_pattern: "*".to_string(),
    };

    let catalog = tool_schemas();
    let baseline = resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog);
    let denied =
        resolve_web_agent_host_tools_from_catalog(&settings, std::slice::from_ref(&rule), &catalog);
    assert_eq!(
        baseline, denied,
        "permission policy must not rewrite availability"
    );
    assert!(denied.iter().any(|schema| {
        schema.pointer("/function/name").and_then(Value::as_str) == Some("write_file")
    }));

    let normalized = web_permission_rules_for_core(&[rule]);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].capability_pattern, "edit");
    assert_eq!(
        normalized[0].decision,
        nola_core::agent::permissions::PermissionPolicyDecision::Deny
    );
}

#[cfg(test)]
#[test]
fn native_bridge_permission_profile_is_strict_and_defaults_to_cautious() {
    let settings = AgentSettings::default();
    assert_eq!(
        permission_profile_for_request(&json!({}), &settings)
            .unwrap()
            .canonical_name(),
        "cautious"
    );
    for name in ["cautious", "allow_all"] {
        assert_eq!(
            permission_profile_for_request(&json!({"permission_profile": name}), &settings)
                .unwrap()
                .canonical_name(),
            name
        );
    }
    assert!(
        permission_profile_for_request(&json!({"permission_profile": "moderate"}), &settings)
            .is_err()
    );
    assert!(
        permission_profile_for_request(&json!({"permission_profile": true}), &settings).is_err()
    );
}

#[cfg(test)]
#[test]
fn stored_permission_profile_rejects_invalid_explicit_values_without_rewriting_settings() {
    assert_eq!(agent_settings_from_stored_value(None).unwrap().permission_profile, "cautious");
    assert_eq!(agent_settings_from_stored_value(Some(json!({}))).unwrap().permission_profile, "cautious");
    for invalid in [json!("moderate"),json!("allow-all"),json!("unknown"),json!(true),Value::Null,json!("")] {
        let stored = json!({"permission_profile": invalid, "custom_system_prompt": "keep"});
        assert!(agent_settings_from_stored_value(Some(stored.clone())).is_err());
        assert_eq!(stored["custom_system_prompt"], "keep");
    }
    for profile in ["cautious", "allow_all"] {
        let settings = agent_settings_from_stored_value(Some(json!({"permission_profile":profile,"custom_system_prompt":"keep","host_tools_enabled":false}))).unwrap();
        assert_eq!(settings.permission_profile,profile);
        assert_eq!(settings.custom_system_prompt,"keep");
        assert!(!settings.host_tools_enabled);
    }
    for invalid in [Value::Null,json!([]),json!({"host_tools_enabled":"invalid"})] {
        assert!(agent_settings_from_stored_value(Some(invalid)).is_err());
    }
    let root = tempfile::tempdir().unwrap();
    let mut application = crate::application_port::CliApplicationPort::persistent(&root.path().join("review.db")).unwrap();
    let original = json!({"permission_profile":"cautious","custom_system_prompt":"keep",
        "builtin_skill_overrides":[{"id":"fixture-document","enabled":false,"content":"Fictional prior user instructions."}]});
    assert!(agent_settings_from_stored_value(Some(original.clone())).is_err());
    let saved: nola_application::ProductSetting = application.execute(nola_application::ApplicationCommand::PutProductSettingChecked {
        key:"web.agent.settings".into(), value_json:original.to_string(), expected_revision:None, updated_at_ms:1,
    }).unwrap();
    assert!(confirm_core_skill_settings(&mut application, saved.revision + 1).is_err());
    let key = confirm_core_skill_settings(&mut application, saved.revision).unwrap();
    let retained: Option<nola_application::ProductSetting> = application.execute(nola_application::ApplicationCommand::GetProductSetting {key:key.clone()}).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&retained.unwrap().value_json).unwrap(), original["builtin_skill_overrides"]);
    let current: Option<nola_application::ProductSetting> = application.execute(nola_application::ApplicationCommand::GetProductSetting {key:"web.agent.settings".into()}).unwrap();
    let current: Value = serde_json::from_str(&current.unwrap().value_json).unwrap();
    assert!(current.get("builtin_skill_overrides").is_none());
    assert_eq!(current["custom_system_prompt"], original["custom_system_prompt"]);
    assert_eq!(agent_settings_from_stored_value(Some(current)).unwrap().skill_settings_backup_key.as_deref(), Some(key.as_str()));
}

#[cfg(test)]
#[test]
fn device_bridge_preserves_only_explicit_artifacts_from_a_real_waiting_result() {
    let _environment = crate::store::test_env_lock();
    HOST_DEVICE_TOOL_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("device challenges")
        .clear();
    HOST_DEVICE_TOOL_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("device waiters")
        .clear();
    let artifact_uri = url::Url::from_file_path(std::env::temp_dir().join("device-output.txt"))
        .expect("device Artifact URI")
        .to_string();
    let request = ToolRequest {
        id: "device-artifact-request".to_string(),
        name: "device_write_file".to_string(),
        args: serde_json::Map::new(),
    };
    let issued = device_bridge_now_ms();
    let runtime_tool = DeviceBridgeRuntimeTool {
        schema: json!({"type":"function","function":{"name":"device_write_file","parameters":{"type":"object"}},"x_nola":{"target":"device"}}),
        capability: "files_write".to_string(),
        result_egress_receipt: nola_core::tool_policy::RuntimeResultEgressReceipt {
            tool_id: "device_write_file".to_string(),
            route: ToolRoute::Device,
            manifest_id: "manifest".to_string(),
            host_id: "host".to_string(),
            client_id: "client".to_string(),
            session_id: "session".to_string(),
            issued_at_ms: issued,
            expires_at_ms: issued + 60_000,
            allow_result_egress: true,
        },
        result_egress_grant: nola_core::tool_policy::ResultEgressGrant {
            tool_id: "device_write_file".to_string(),
            route: ToolRoute::Device,
            manifest_id: "manifest".to_string(),
            host_id: "host".to_string(),
            client_id: "client".to_string(),
            session_id: "session".to_string(),
            expires_at_ms: issued + 60_000,
        },
    };
    let (event_tx, _event_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        wait_for_device_tool(
            &request,
            &runtime_tool,
            "host",
            "client",
            "session",
            &event_tx,
            None,
        )
    });
    let challenge = (0..100)
        .find_map(|_| {
            let challenge = HOST_DEVICE_TOOL_CHALLENGES
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .ok()
                .and_then(|challenges| challenges.values().next().cloned());
            if challenge.is_none() {
                std::thread::sleep(StdDuration::from_millis(10));
            }
            challenge
        })
        .expect("device request enters waiting state");
    let challenge = challenge.request;
    let sender = HOST_DEVICE_TOOL_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("device waiters")
        .get(&challenge.nonce)
        .cloned()
        .expect("device waiter");
    sender
        .send(nola_core::device_bridge::DeviceToolResult {
            protocol_version: nola_core::device_bridge::DEVICE_BRIDGE_PROTOCOL_VERSION,
            nonce: challenge.nonce,
            request_id: challenge.request_id,
            host_id: challenge.host_id,
            client_id: challenge.client_id,
            session_id: challenge.session_id,
            tool_name: challenge.tool_name,
            output: json!({
                "text": format!("wrote {artifact_uri}"),
                "_meta": {"nola/artifacts": [{
                    "uri": artifact_uri,
                    "name": "device-output.txt",
                    "mime_type": "text/plain",
                    "operation": "create"
                }]}
            }),
            error: false,
        })
        .expect("deliver device result");
    let outcome = worker.join().expect("device wait worker");
    assert!(!outcome.error);
    assert_eq!(outcome.artifacts.len(), 1);
    assert_eq!(outcome.artifacts[0].name, "device-output.txt");
    assert_eq!(outcome.artifacts[0].operation.as_deref(), Some("create"));
}

#[cfg(test)]
#[test]
fn web_code_memory_candidates_preserve_discovery_and_host_restrictions() {
    let mut catalog = tool_schemas();
    catalog.extend(crate::code_intelligence_provider::model_tool_schemas());
    let mut settings = AgentSettings::default();
    settings.host_tools_enabled = true;
    settings.enable_local_dev_tools = false;
    let available = resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog);
    let names = available.iter().filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str)).collect::<Vec<_>>();
    for id in ["codebase_index", "codebase_search", "codebase_read_symbol", "codebase_trace"] {
        assert!(names.contains(&id), "native capability missing: {id}");
    }
    let candidates = candidates_from_schemas(&available, ToolRoute::AgentHost);
    let mut disabled = candidates.clone();
    restrict_web_tool_candidates(&mut disabled, false, None);
    assert!(disabled.is_empty());
    let mut explicit = candidates;
    restrict_web_tool_candidates(&mut explicit, true, Some(&["codebase_search".into()]));
    assert_eq!(explicit.len(), 1);
    assert_eq!(explicit[0].schema["function"]["name"], "codebase_search");
    settings.host_tools_enabled = false;
    let disabled = resolve_web_agent_host_tools_from_catalog(&settings, &[], &catalog);
    assert!(!disabled.iter().any(|schema| schema["function"]["name"].as_str().is_some_and(|name| name.starts_with("codebase_"))));
}
