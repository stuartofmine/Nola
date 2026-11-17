// SPDX-License-Identifier: Apache-2.0
//! Consolidated Model Catalog, Connections, Endpoints & Generation Contract suite:
//! pure reasoning/headroom resolution, endpoint route invalidation vs metadata
//! preservation, catalog CAS/parameters/MCP/sync-outbox rollback, cross-connection
//! UOW visibility, and model connection probe/override/refresh/protocol lifecycles.

use nola_application::{
    execute_application_command, resolve_model_reasoning, resolve_model_reasoning_request,
    ApplicationCommand, McpCommand, McpServer, ModelCommand, ModelExecutionCapabilities,
    ModelProfile, ModelRuntime, ModelRuntimePhase,
};
use nola_domain::{
    generation::{ModelProtocol, ReasoningSelection},
    model_connection::*,
    reasoning_tiers::ReasoningTier,
};
use nola_store::NolaStore;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::tempdir;

fn effort(value: &str) -> Option<ReasoningSelection> {
    Some(ReasoningSelection::Effort {
        value: value.into(),
    })
}

#[test]
fn model_reasoning_resolution_and_protocol_headroom_contract() {
    // 1. Native query tier resolution & ambiguous input rejection.
    let declared = json!({"control":"effort","efforts":["low","high"],"defaultPreference":"invalid-but-shadowed"});
    assert_eq!(
        resolve_model_reasoning_request(
            &declared,
            ModelProtocol::Responses,
            16384,
            true,
            &Value::Null,
            Some(ReasoningTier::Balanced)
        )
        .unwrap(),
        json!({"selection":{"kind":"effort","value":"low"}})
    );
    assert!(resolve_model_reasoning_request(
        &declared,
        ModelProtocol::Responses,
        16384,
        true,
        &json!({"kind":"effort","value":"high"}),
        Some(ReasoningTier::Balanced)
    )
    .is_err());
    assert!(resolve_model_reasoning_request(
        &declared,
        ModelProtocol::Responses,
        16384,
        false,
        &Value::Null,
        Some(ReasoningTier::Balanced)
    )
    .is_err());
    for limit in [0, 63, 1_000_001] {
        assert!(resolve_model_reasoning_request(
            &Value::Null,
            ModelProtocol::Responses,
            limit,
            false,
            &Value::Null,
            None
        )
        .is_err());
    }
    assert_eq!(
        resolve_model_reasoning_request(
            &Value::Null,
            ModelProtocol::ChatCompletions,
            4096,
            false,
            &Value::Null,
            Some(ReasoningTier::Detailed)
        )
        .unwrap(),
        json!({"selection":null})
    );

    // 2. Missing inherits defaultPreference while explicit Null clears across all protocols.
    for (protocol, decl, expected) in [
        (
            ModelProtocol::Responses,
            json!({"control":"effort","efforts":["low","high"],"defaultPreference":"high"}),
            effort("high"),
        ),
        (
            ModelProtocol::ChatCompletions,
            json!({"control":"effort","efforts":["low","medium","high"],"defaultPreference":"balanced"}),
            effort("medium"),
        ),
        (
            ModelProtocol::Anthropic,
            json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"}),
            Some(ReasoningSelection::Budget { tokens: 8192 }),
        ),
    ] {
        assert_eq!(
            resolve_model_reasoning(&decl, protocol, 16384, true, None).unwrap(),
            expected
        );
        assert_eq!(
            resolve_model_reasoning(&decl, protocol, 16384, true, Some(&Value::Null)).unwrap(),
            None
        );
    }

    // 3. Explicit selection overrides invalid defaultPreference without mutating declaration.
    for invalid_default in [json!("unsupported"), json!(42), json!({"kind":"effort"})] {
        let decl = json!({"control":"effort","efforts":["low","high"],"defaultPreference":invalid_default});
        let original = decl.clone();
        assert!(resolve_model_reasoning(&decl, ModelProtocol::Responses, 16384, true, None).is_err());
        assert_eq!(
            resolve_model_reasoning(
                &decl,
                ModelProtocol::Responses,
                16384,
                true,
                Some(&json!({"kind":"effort","value":"low"}))
            )
            .unwrap(),
            effort("low")
        );
        assert_eq!(
            resolve_model_reasoning(
                &decl,
                ModelProtocol::Responses,
                16384,
                true,
                Some(&Value::Null)
            )
            .unwrap(),
            None
        );
        assert_eq!(decl, original);
    }

    // 4. Malformed or unknown request fields never silently inherit.
    let strict_decl =
        json!({"control":"effort","efforts":["low","high"],"defaultPreference":"high"});
    for req in [
        json!(false),
        json!("low"),
        json!(0),
        json!([]),
        json!({}),
        json!({"kind":"effort","value":5}),
        json!({"kind":"effort","value":"low","tokens":1024}),
        json!({"kind":"effort","value":"unknown"}),
        json!({"kind":"effort","value":"medium"}),
        json!({"kind":"budget","tokens":1024.5}),
        json!({"kind":"budget","tokens":-1}),
        json!({"kind":"budget","tokens":"1024"}),
        json!({"kind":"other","value":"low"}),
    ] {
        assert!(
            resolve_model_reasoning(
                &strict_decl,
                ModelProtocol::Responses,
                16384,
                true,
                Some(&req)
            )
            .is_err(),
            "accepted malformed selection: {req}"
        );
    }

    // 5. Automatic/absent control, unsupported capability, and inactive metadata schema validation.
    for decl in [
        Value::Null,
        json!({"control":"automatic","efforts":["low","medium","high"],"budgets":[1024,4096,8192]}),
    ] {
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::Anthropic,
        ] {
            assert_eq!(
                resolve_model_reasoning(&decl, protocol, 16384, false, None).unwrap(),
                None
            );
            for req in [
                json!({"kind":"effort","value":"low"}),
                json!({"kind":"budget","tokens":1024}),
            ] {
                assert!(resolve_model_reasoning(&decl, protocol, 16384, true, Some(&req)).is_err());
            }
        }
    }
    assert!(
        resolve_model_reasoning(&strict_decl, ModelProtocol::Responses, 16384, false, None).is_err()
    );
    assert!(resolve_model_reasoning(
        &strict_decl,
        ModelProtocol::Responses,
        16384,
        false,
        Some(&json!({"kind":"effort","value":"low"}))
    )
    .is_err());
    assert_eq!(
        resolve_model_reasoning(
            &strict_decl,
            ModelProtocol::Responses,
            16384,
            false,
            Some(&Value::Null)
        )
        .unwrap(),
        None
    );

    for bad_fields in [
        json!({"efforts":"low"}),
        json!({"budgets":["1024"]}),
        json!({"responses":"true"}),
        json!({"outputLimit":"16384"}),
        json!({"outputLimit":-1}),
        json!({"outputLimit":16384.5}),
        json!({"unexpected":true}),
    ] {
        let mut decl = json!({"control":"automatic"});
        decl.as_object_mut()
            .unwrap()
            .extend(bad_fields.as_object().unwrap().clone());
        assert!(resolve_model_reasoning(&decl, ModelProtocol::Responses, 16384, true, None).is_err());
    }

    // 6. Anthropic budget headroom, turn output limit, and native effort distinctness.
    let budget_decl =
        json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"});
    assert_eq!(
        resolve_model_reasoning(&budget_decl, ModelProtocol::Anthropic, 8193, true, None).unwrap(),
        Some(ReasoningSelection::Budget { tokens: 8192 })
    );
    assert!(
        resolve_model_reasoning(&budget_decl, ModelProtocol::Anthropic, 8192, true, None).is_err()
    );
    for protocol in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        assert!(resolve_model_reasoning(&budget_decl, protocol, 16384, true, None).is_err());
    }
    for tokens in [0, 1023, 2048, 16384] {
        assert!(resolve_model_reasoning(
            &budget_decl,
            ModelProtocol::Anthropic,
            16384,
            true,
            Some(&json!({"kind":"budget","tokens":tokens}))
        )
        .is_err());
    }
    assert!(
        resolve_model_reasoning(&budget_decl, ModelProtocol::Anthropic, 4096, true, None).is_err()
    );
    assert_eq!(
        resolve_model_reasoning(
            &budget_decl,
            ModelProtocol::Anthropic,
            4096,
            true,
            Some(&json!({"kind":"budget","tokens":1024}))
        )
        .unwrap(),
        Some(ReasoningSelection::Budget { tokens: 1024 })
    );
    assert_eq!(
        resolve_model_reasoning(
            &budget_decl,
            ModelProtocol::Anthropic,
            4096,
            true,
            Some(&Value::Null)
        )
        .unwrap(),
        None
    );
    for too_high in [4096, 8192] {
        assert!(resolve_model_reasoning(
            &budget_decl,
            ModelProtocol::Anthropic,
            4096,
            true,
            Some(&json!({"kind":"budget","tokens":too_high}))
        )
        .is_err());
    }

    let native_effort =
        json!({"control":"effort","efforts":["none","low","high","max"],"defaultPreference":"high"});
    assert_eq!(
        resolve_model_reasoning(&native_effort, ModelProtocol::Anthropic, 8192, true, None).unwrap(),
        effort("high")
    );
    assert_eq!(
        resolve_model_reasoning(
            &native_effort,
            ModelProtocol::Anthropic,
            8192,
            true,
            Some(&Value::Null)
        )
        .unwrap(),
        None
    );
    assert_eq!(
        resolve_model_reasoning(
            &native_effort,
            ModelProtocol::Anthropic,
            8192,
            true,
            Some(&json!({"kind":"effort","value":"none"}))
        )
        .unwrap(),
        effort("none")
    );
    assert!(resolve_model_reasoning(
        &native_effort,
        ModelProtocol::Anthropic,
        8192,
        true,
        Some(&json!({"kind":"effort","value":"medium"}))
    )
    .is_err());
    assert!(resolve_model_reasoning(
        &native_effort,
        ModelProtocol::Anthropic,
        8192,
        true,
        Some(&json!({"kind":"budget","tokens":1024}))
    )
    .is_err());
}

#[test]
fn endpoint_execution_declarations_metadata_edits_stale_cas_and_native_resolution() {
    const ORIGINAL_URL: &str = "https://fixture.invalid/v1";
    let endpoint_profile = |id: &str, endpoint: &str, selected: bool, tools: bool| ModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "openai-compatible".into(),
        model_name: format!("model-{id}"),
        runtime: "remote".into(),
        source_json: json!({
            "endpointID": endpoint, "endpointName": "Original", "endpointEnabled": true,
            "baseURL": ORIGINAL_URL, "wireProtocol": "responses", "capabilities": ["chat"],
            "executionCapabilities": {
                "toolCalls": tools, "imageInput": false, "reasoning": false, "samplingParameters": false
            },
            "fixtureMetadata": {"preserve": true}
        })
        .to_string(),
        parameters_json: json!({"maxOutputTokens": 16384}).to_string(),
        selected,
        enabled: true,
        revision: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    };
    let list_profiles = |store: &mut NolaStore| -> Vec<ModelProfile> {
        serde_json::from_value(
            execute_application_command(
                store,
                ApplicationCommand::Model {
                    command: ModelCommand::ListProfiles,
                },
            )
            .unwrap(),
        )
        .unwrap()
    };
    let get_profile = |store: &mut NolaStore, id: &str| -> ModelProfile {
        serde_json::from_value::<Option<ModelProfile>>(
            execute_application_command(
                store,
                ApplicationCommand::Model {
                    command: ModelCommand::GetProfile { id: id.into() },
                },
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap()
    };
    let endpoint_revisions = |profiles: &[ModelProfile]| -> BTreeMap<String, i64> {
        profiles
            .iter()
            .filter(|p| serde_json::from_str::<Value>(&p.source_json).unwrap()["endpointID"] == "endpoint")
            .map(|p| (p.id.clone(), p.revision))
            .collect()
    };
    let configure = |store: &mut NolaStore,
                     expected_revisions: BTreeMap<String, i64>,
                     base_url: &str,
                     protocol: &str,
                     name: &str,
                     enabled: bool,
                     at: i64| {
        execute_application_command(
            store,
            ApplicationCommand::Model {
                command: ModelCommand::ConfigureEndpoint {
                    endpoint_id: "endpoint".into(),
                    expected_revisions,
                    name: name.into(),
                    base_url: base_url.into(),
                    wire_protocol: protocol.into(),
                    enabled,
                    updated_at_ms: at,
                },
            },
        )
    };
    let sync_rows = |conn: &Connection| -> Vec<String> {
        conn.prepare(
            "SELECT json_object('sequence',local_sequence,'operation',operation_id,
             'type',entity_type,'id',entity_id,'revision',revision,'payload',payload_json,
             'status',status) FROM sync_outbox ORDER BY local_sequence",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
    };

    // 1. Metadata & equivalent normalized URL edits preserve execution declarations; stale CAS rejects atomically;
    //    subsequent URL/protocol change invalidates executionCapabilities while keeping models readable.
    let directory = tempdir().unwrap();
    let database = directory.path().join("endpoint-suite.db");
    let mut store = NolaStore::open(&database).unwrap();
    for input in [
        endpoint_profile("first", "endpoint", true, true),
        endpoint_profile("second", "endpoint", false, false),
        endpoint_profile("foreign", "other", false, true),
    ] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input },
            },
        )
        .unwrap();
    }
    let initial = list_profiles(&mut store);
    let stale_revs = endpoint_revisions(&initial);

    for (base_url, name, enabled, at) in [
        (" HTTPS://FIXTURE.invalid:443/v1 ", "Renamed", false, 20),
        (ORIGINAL_URL, "Renamed again", true, 30),
    ] {
        let cur = list_profiles(&mut store);
        configure(
            &mut store,
            endpoint_revisions(&cur),
            base_url,
            "responses",
            name,
            enabled,
            at,
        )
        .unwrap();
        for prev in &initial {
            let current = get_profile(&mut store, &prev.id);
            if prev.id == "foreign" {
                assert_eq!(&current, prev);
                continue;
            }
            let src: Value = serde_json::from_str(&current.source_json).unwrap();
            let orig_src: Value = serde_json::from_str(&prev.source_json).unwrap();
            assert_eq!(src["executionCapabilities"], orig_src["executionCapabilities"]);
            assert_eq!(
                ModelExecutionCapabilities::from_source(&src).unwrap().tool_calls,
                prev.id == "first"
            );
            assert_eq!(src["baseURL"], ORIGINAL_URL);
            assert_eq!(src["endpointName"], name);
            assert_eq!(src["endpointEnabled"], enabled);
        }
    }

    // Stale CAS rejection preserves profiles and sync_outbox.
    let before_stale = list_profiles(&mut store);
    let conn = Connection::open(&database).unwrap();
    let before_sync = sync_rows(&conn);
    let err = configure(
        &mut store,
        stale_revs,
        "https://different.invalid/v1",
        "chat_completions",
        "Stale",
        false,
        35,
    )
    .unwrap_err();
    assert!(err.contains("revision_conflict"), "{err}");
    assert_eq!(list_profiles(&mut store), before_stale);
    assert_eq!(sync_rows(&conn), before_sync);

    // URL and protocol changes invalidate executionCapabilities across cold reopen.
    for (idx, (base_url, protocol)) in [
        ("https://different.invalid/v1", "responses"),
        ("https://fixture.invalid/v2", "responses"),
        ("https://fixture.invalid/v2", "chat_completions"),
    ]
    .into_iter()
    .enumerate()
    {
        let cur = list_profiles(&mut store);
        let returned: Vec<ModelProfile> = serde_json::from_value(
            configure(
                &mut store,
                endpoint_revisions(&cur),
                base_url,
                protocol,
                "RouteChanged",
                true,
                40 + idx as i64,
            )
            .unwrap(),
        )
        .unwrap();
        let after = list_profiles(&mut store);
        assert_eq!(returned, after);
        for prev in &cur {
            let updated = get_profile(&mut store, &prev.id);
            if prev.id == "foreign" {
                assert_eq!(&updated, prev);
                continue;
            }
            let src: Value = serde_json::from_str(&updated.source_json).unwrap();
            assert!(src["executionCapabilities"].is_null());
            assert!(ModelExecutionCapabilities::from_source(&src)
                .unwrap_err()
                .starts_with("model_execution_capabilities_required:"));
            assert_eq!(src["baseURL"], base_url);
            assert_eq!(src["wireProtocol"], protocol);
            assert_eq!(src["fixtureMetadata"], json!({"preserve":true}));
            assert!(updated.revision > prev.revision);
        }
    }
    let final_profiles = list_profiles(&mut store);
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(list_profiles(&mut reopened), final_profiles);

    // 2. Native ResolveRemoteModelProfile uses exact profile & Core automatic reasoning.
    let mut mem = NolaStore::open(":memory:").unwrap();
    for (id, url, selected) in [
        ("first", ORIGINAL_URL, true),
        ("second", "https://second.invalid/v1", false),
    ] {
        let mut input = endpoint_profile(id, id, selected, false);
        let mut src: Value = serde_json::from_str(&input.source_json).unwrap();
        src["baseURL"] = json!(url);
        src["reasoning"] = json!({"control":"automatic"});
        src["executionCapabilities"]["reasoning"] = json!(true);
        src["credentialRef"] = json!(format!("fixture:{id}"));
        input.source_json = src.to_string();
        execute_application_command(
            &mut mem,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input },
            },
        )
        .unwrap();
    }
    let second = list_profiles(&mut mem)
        .into_iter()
        .find(|p| p.id == "second")
        .unwrap();
    let query = ApplicationCommand::ResolveRemoteModelProfile {
        profile_id: second.id.clone(),
        expected_revision: second.revision,
        request: json!({}),
    };
    assert!(query.is_read_only_query());
    let resolved = execute_application_command(&mut mem, query).unwrap();
    assert_eq!(resolved["resolved"]["endpoint"], "https://second.invalid/v1");
    assert_eq!(resolved["credential_ref"], "fixture:second");
    assert_eq!(resolved["resolved"]["runtime"]["reasoning_selection"], Value::Null);
    assert!(execute_application_command(
        &mut mem,
        ApplicationCommand::ResolveRemoteModelProfile {
            profile_id: second.id,
            expected_revision: second.revision + 1,
            request: json!({}),
        }
    )
    .is_err());
}

#[test]
fn model_catalog_cas_parameters_mcp_and_sync_rollback_lifecycle() {
    // 1. Wire contracts: ReplaceProfiles requires expected_revisions & ReportRuntime rejects unknown phases.
    assert!(serde_json::from_value::<ModelCommand>(json!({
        "type": "replace_profiles", "provider": "local", "runtime": "llama.cpp",
        "profiles": [], "updated_at_ms": 20
    }))
    .is_err());
    assert!(serde_json::from_value::<ModelCommand>(json!({
        "type": "replace_profiles", "provider": "local", "runtime": "llama.cpp",
        "profiles": [], "updated_at_ms": 20, "expected_revisions": {}
    }))
    .is_ok());
    assert!(serde_json::from_value::<ApplicationCommand>(json!({
        "type": "model",
        "command": {
            "type": "report_runtime",
            "input": {
                "host_id": "mac-local", "profile_id": "local:qwen-4b", "phase": "warming_up",
                "endpoint_json": null, "process_id": null, "error": null,
                "started_at_ms": null, "stopped_at_ms": null, "updated_at_ms": 12
            }
        }
    }))
    .is_err());

    // 2. Shared SQLite database exercising Model + Runtime + MCP cold reopen, ReplaceProfiles CAS & rollback,
    //    provider-scope isolation, ResetParameters, ConfigureParameters, tombstones, and sync_outbox rollback.
    let local_profile = || ModelProfile {
        id: "local:qwen-4b".into(),
        display_name: "Qwen 4B".into(),
        provider: "local".into(),
        model_name: "qwen3.5-4b".into(),
        runtime: "llama.cpp".into(),
        source_json: r#"{"artifact_path":"/models/qwen.gguf"}"#.into(),
        parameters_json: r#"{"temperature":0.7}"#.into(),
        selected: true,
        enabled: true,
        revision: 0,
        created_at_ms: 10,
        updated_at_ms: 10,
        deleted_at_ms: None,
    };

    let directory = tempdir().unwrap();
    let database = directory.path().join("catalog-mcp-sync.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let mut remote_other = local_profile();
        remote_other.id = "remote:other".into();
        remote_other.provider = "remote".into();
        remote_other.selected = false;
        for input in [local_profile(), remote_other] {
            execute_application_command(
                &mut store,
                ApplicationCommand::Model {
                    command: ModelCommand::UpsertProfile { input },
                },
            )
            .unwrap();
        }
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ReportRuntime {
                    input: ModelRuntime {
                        host_id: "mac-local".into(),
                        profile_id: "local:qwen-4b".into(),
                        phase: ModelRuntimePhase::Ready,
                        endpoint_json: Some(r#"{"url":"http://127.0.0.1:8080"}"#.into()),
                        process_id: Some(42),
                        error: None,
                        started_at_ms: Some(11),
                        stopped_at_ms: None,
                        updated_at_ms: 12,
                    },
                },
            },
        )
        .unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::Mcp {
                command: McpCommand::Upsert {
                    input: McpServer {
                        id: "filesystem".into(),
                        display_name: "Filesystem".into(),
                        enabled: true,
                        configuration_json: r#"{"transport":"stdio"}"#.into(),
                        revision: 0,
                        created_at_ms: 13,
                        updated_at_ms: 13,
                        deleted_at_ms: None,
                    },
                },
            },
        )
        .unwrap();

        // ReplaceProfiles CAS, stale revision, stale membership, and invalid-profile rollback.
        let orig = store.model_profile("local:qwen-4b").unwrap().unwrap();
        let expected = BTreeMap::from([(orig.id.clone(), orig.revision)]);
        let mut invalid = local_profile();
        invalid.id = "local:invalid".into();
        invalid.selected = false;
        invalid.parameters_json = "not-json".into();
        assert!(execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ReplaceProfiles {
                    provider: "local".into(),
                    runtime: "llama.cpp".into(),
                    profiles: vec![local_profile(), invalid],
                    updated_at_ms: 15,
                    expected_revisions: expected.clone(),
                },
            }
        )
        .is_err());
        assert_eq!(store.model_profile("local:qwen-4b").unwrap().unwrap(), orig);

        let mut renamed = local_profile();
        renamed.display_name = "Updated in second window".into();
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ReplaceProfiles {
                    provider: "local".into(),
                    runtime: "llama.cpp".into(),
                    profiles: vec![renamed.clone()],
                    updated_at_ms: 20,
                    expected_revisions: expected.clone(),
                },
            },
        )
        .unwrap();
        let committed = store.model_profile("local:qwen-4b").unwrap().unwrap();
        assert!(committed.revision > orig.revision);

        assert!(execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ReplaceProfiles {
                    provider: "local".into(),
                    runtime: "llama.cpp".into(),
                    profiles: vec![local_profile()],
                    updated_at_ms: 25,
                    expected_revisions: expected,
                },
            }
        )
        .unwrap_err()
        .contains("revision_conflict"));

        // ResetParameters atomic missing-ID rollback and valid update.
        let before_reset = serde_json::to_value(store.list_model_profiles().unwrap()).unwrap();
        assert!(execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ResetParameters {
                    profile_ids: vec!["local:qwen-4b".into(), "missing".into()],
                    parameters_json: r#"{"temperature":0.2}"#.into(),
                    updated_at_ms: 28,
                },
            }
        )
        .is_err());
        assert_eq!(
            serde_json::to_value(store.list_model_profiles().unwrap()).unwrap(),
            before_reset
        );
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ResetParameters {
                    profile_ids: vec!["local:qwen-4b".into()],
                    parameters_json: r#"{"temperature":0.2}"#.into(),
                    updated_at_ms: 30,
                },
            },
        )
        .unwrap();
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let restored_prof: Option<ModelProfile> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::Model {
                command: ModelCommand::GetProfile {
                    id: "local:qwen-4b".into(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    let restored_prof = restored_prof.unwrap();
    assert_eq!(restored_prof.parameters_json, r#"{"temperature":0.2}"#);
    assert!(restored_prof.selected);

    let rt_val: Option<ModelRuntime> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::Model {
                command: ModelCommand::GetRuntime {
                    host_id: "mac-local".into(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(rt_val.unwrap().phase, ModelRuntimePhase::Ready);

    let servers: Vec<McpServer> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::Mcp {
                command: McpCommand::List,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].id, "filesystem");

    // Stale membership check, provider-scoped clear, ConfigureParameters, and tombstone rejection.
    let mut added = local_profile();
    added.id = "local:new".into();
    added.selected = false;
    execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: added.clone(),
            },
        },
    )
    .unwrap();
    assert!(execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "local".into(),
                runtime: "llama.cpp".into(),
                profiles: vec![],
                updated_at_ms: 40,
                expected_revisions: BTreeMap::from([(
                    restored_prof.id.clone(),
                    restored_prof.revision
                )]),
            },
        }
    )
    .unwrap_err()
    .contains("revision_conflict"));

    let new_rev = reopened.model_profile("local:new").unwrap().unwrap().revision;
    execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "local".into(),
                runtime: "llama.cpp".into(),
                profiles: vec![],
                updated_at_ms: 45,
                expected_revisions: BTreeMap::from([
                    (restored_prof.id, restored_prof.revision),
                    ("local:new".into(), new_rev),
                ]),
            },
        },
    )
    .unwrap();
    assert!(reopened.model_profile("remote:other").unwrap().is_some());

    // ConfigureParameters preserves existing fields and rejects tombstones.
    let mut cfg_input = local_profile();
    cfg_input.id = "local:cfg".into();
    cfg_input.selected = false;
    execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters {
                input: cfg_input.clone(),
            },
        },
    )
    .unwrap();
    let mut canonical = cfg_input.clone();
    canonical.selected = true;
    canonical.source_json = r#"{"artifact_path":"/owned/actual.gguf"}"#.into();
    canonical.display_name = "Canonical name".into();
    execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: canonical.clone(),
            },
        },
    )
    .unwrap();
    cfg_input.parameters_json = r#"{"temperature":0.1}"#.into();
    execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters {
                input: cfg_input.clone(),
            },
        },
    )
    .unwrap();
    let stored_cfg = reopened.model_profile("local:cfg").unwrap().unwrap();
    assert!(stored_cfg.selected);
    assert_eq!(stored_cfg.source_json, canonical.source_json);
    assert_eq!(stored_cfg.display_name, canonical.display_name);
    assert_eq!(stored_cfg.parameters_json, cfg_input.parameters_json);
    reopened.delete_model_profile("local:cfg", 50).unwrap();
    assert!(execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters { input: cfg_input }
        }
    )
    .is_err());
    drop(reopened);

    // 3. Sync outbox failure rollback + deleted/live scope ownership immutability.
    let sync_profile = |id: &str, selected: bool| ModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "fixture".into(),
        model_name: id.into(),
        runtime: "remote".into(),
        source_json: "{}".into(),
        parameters_json: "{}".into(),
        selected,
        enabled: true,
        revision: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    };
    let sync_db_path = directory.path().join("sync-atomicity.db");
    let mut sync_store = NolaStore::open(&sync_db_path).unwrap();
    for input in [sync_profile("first", true), sync_profile("second", false)] {
        execute_application_command(
            &mut sync_store,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input },
            },
        )
        .unwrap();
    }
    let sync_conn = Connection::open(&sync_db_path).unwrap();
    let read_sync = |c: &Connection| -> Vec<String> {
        c.prepare(
            "SELECT json_object('sequence',local_sequence,'operation',operation_id,
             'type',entity_type,'id',entity_id,'revision',revision,'deleted',deleted,
             'payload',payload_json,'status',status) FROM sync_outbox ORDER BY local_sequence",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
    };
    let expected_fixture_revs: BTreeMap<String, i64> = sync_store
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .map(|p| (p.id, p.revision))
        .collect();
    let before_models = execute_application_command(
        &mut sync_store,
        ApplicationCommand::Model {
            command: ModelCommand::ListProfiles,
        },
    )
    .unwrap();
    let before_sync_rows = read_sync(&sync_conn);

    let mut cfg_param = sync_profile("second", false);
    cfg_param.parameters_json = r#"{"temperature":0.25}"#.into();
    cfg_param.updated_at_ms = 20;

    for (cmd, failing_id) in [
        (
            ModelCommand::ReplaceProfiles {
                provider: "fixture".into(),
                runtime: "remote".into(),
                profiles: vec![sync_profile("new-first", true), sync_profile("new-second", false)],
                updated_at_ms: 20,
                expected_revisions: expected_fixture_revs,
            },
            "new-second",
        ),
        (
            ModelCommand::ResetParameters {
                profile_ids: vec!["first".into(), "second".into()],
                parameters_json: r#"{"temperature":0.25}"#.into(),
                updated_at_ms: 20,
            },
            "second",
        ),
        (ModelCommand::ConfigureParameters { input: cfg_param }, "second"),
    ] {
        sync_conn
            .execute_batch(&format!(
                "CREATE TRIGGER reject_fixture_sync BEFORE INSERT ON sync_outbox
                 WHEN NEW.entity_id='{failing_id}'
                 BEGIN SELECT RAISE(ABORT, 'fixture sync write rejected'); END;"
            ))
            .unwrap();
        let err = execute_application_command(
            &mut sync_store,
            ApplicationCommand::Model { command: cmd },
        )
        .unwrap_err();
        assert!(err.contains("fixture sync write rejected"), "{err}");
        assert_eq!(
            execute_application_command(
                &mut sync_store,
                ApplicationCommand::Model {
                    command: ModelCommand::ListProfiles
                }
            )
            .unwrap(),
            before_models
        );
        assert_eq!(read_sync(&sync_conn), before_sync_rows);
        sync_conn
            .execute_batch("DROP TRIGGER reject_fixture_sync;")
            .unwrap();
    }

    // Deleted profile ID cannot be taken over by another provider, and live/deleted scope cannot transfer.
    let mut foreign = sync_profile("shared-id", false);
    foreign.provider = "another-provider".into();
    execute_application_command(
        &mut sync_store,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input: foreign },
        },
    )
    .unwrap();
    execute_application_command(
        &mut sync_store,
        ApplicationCommand::Model {
            command: ModelCommand::DeleteProfile {
                id: "shared-id".into(),
                deleted_at_ms: 10,
            },
        },
    )
    .unwrap();
    let cur_revs: BTreeMap<String, i64> = sync_store
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .filter(|p| p.provider == "fixture" && p.runtime == "remote")
        .map(|p| (p.id, p.revision))
        .collect();
    assert!(execute_application_command(
        &mut sync_store,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "fixture".into(),
                runtime: "remote".into(),
                profiles: vec![sync_profile("shared-id", true)],
                updated_at_ms: 20,
                expected_revisions: cur_revs,
            },
        }
    )
    .is_err());

    for (id, deleted) in [("first", false), ("deleted-stable", true)] {
        if deleted {
            execute_application_command(
                &mut sync_store,
                ApplicationCommand::Model {
                    command: ModelCommand::UpsertProfile {
                        input: sync_profile(id, false),
                    },
                },
            )
            .unwrap();
            execute_application_command(
                &mut sync_store,
                ApplicationCommand::Model {
                    command: ModelCommand::DeleteProfile {
                        id: id.into(),
                        deleted_at_ms: 12,
                    },
                },
            )
            .unwrap();
        }
        for change_runtime in [false, true] {
            let mut changed = sync_profile(id, false);
            if change_runtime {
                changed.runtime = "other-runtime".into();
            } else {
                changed.provider = "other-provider".into();
            }
            assert!(execute_application_command(
                &mut sync_store,
                ApplicationCommand::Model {
                    command: ModelCommand::UpsertProfile { input: changed }
                }
            )
            .is_err());
        }
    }
}

#[test]
fn model_unit_of_work_cross_connection_visibility() {
    let profile = |id: &str| ModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "fixture".into(),
        model_name: id.into(),
        runtime: "remote".into(),
        source_json: r#"{"baseURL":"https://fixture.invalid/v1"}"#.into(),
        parameters_json: "{}".into(),
        selected: true,
        enabled: true,
        revision: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    };
    let directory = tempdir().unwrap();
    let database = directory.path().join("visibility.db");
    {
        let mut seed = NolaStore::open(&database).unwrap();
        execute_application_command(
            &mut seed,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile {
                    input: profile("original"),
                },
            },
        )
        .unwrap();
    }
    let reader = NolaStore::open(&database).unwrap();
    for commit in [false, true] {
        let db_path = database.clone();
        let (staged_tx, staged_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let replacement = profile("replacement");
        let writer = std::thread::spawn(move || {
            let mut store = NolaStore::open(&db_path).unwrap();
            store.begin_write_unit_of_work().unwrap();
            let expected_revision = store.model_profile("original").unwrap().unwrap().revision;
            execute_application_command(
                &mut store,
                ApplicationCommand::Model {
                    command: ModelCommand::ReplaceProfiles {
                        provider: "fixture".into(),
                        runtime: "remote".into(),
                        profiles: vec![replacement],
                        updated_at_ms: 20,
                        expected_revisions: BTreeMap::from([("original".into(), expected_revision)]),
                    },
                },
            )
            .unwrap();
            staged_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            if commit {
                store.commit_write_unit_of_work().unwrap();
            } else {
                store.rollback_write_unit_of_work().unwrap();
            }
        });
        staged_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let during = reader.list_model_profiles().unwrap();
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        assert_eq!(during.len(), 1);
        assert_eq!(during[0].id, "original");
        assert!(during[0].selected);
        let after = reader.list_model_profiles().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, if commit { "replacement" } else { "original" });
        assert!(after[0].selected);
    }
}

fn conn_execute(store: &mut NolaStore, command: ModelCommand) -> Result<Value, String> {
    execute_application_command(store, ApplicationCommand::Model { command })
}

fn make_connection(id: &str) -> ModelConnection {
    ModelConnection {
        id: id.into(),
        display_name: format!("Connection {id}"),
        base_url: "https://fixture.invalid/v1".into(),
        protocols: vec![ModelProtocol::Responses, ModelProtocol::ChatCompletions],
        account_id: None,
        source: ModelSourceKind::Manual,
        source_id: None,
        enabled: true,
    }
}

fn catalog_model(name: &str, protocol: ModelProtocol) -> CatalogModel {
    CatalogModel {
        model_id: name.into(),
        display_name: name.into(),
        protocol,
        context_window: Some(128000),
        output_limit: Some(8192),
        modalities: vec!["text".into()],
        reasoning_levels: vec![],
        summary_supported: None,
        execution_capabilities: Some(
            nola_domain::model_capabilities::ModelExecutionCapabilities::default(),
        ),
    }
}

fn save_conn(
    store: &mut NolaStore,
    input: ModelConnection,
    revision: Option<i64>,
) -> ModelConnectionSnapshot {
    serde_json::from_value(
        conn_execute(
            store,
            ModelCommand::SaveConnection {
                input,
                expected_revision: revision,
                updated_at_ms: 1,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

fn refresh_conn(
    store: &mut NolaStore,
    id: &str,
    revision: i64,
    models: Vec<CatalogModel>,
    at: i64,
) -> ModelConnectionSnapshot {
    serde_json::from_value(
        conn_execute(
            store,
            ModelCommand::RefreshConnectionCatalog {
                connection_id: id.into(),
                expected_revision: revision,
                models,
                observed_at_ms: at,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

fn select_conn(
    store: &mut NolaStore,
    id: &str,
    revision: i64,
    enabled: &[&str],
    selected: Option<&str>,
) -> ModelConnectionSnapshot {
    serde_json::from_value(
        conn_execute(
            store,
            ModelCommand::ConfigureConnectionModels {
                connection_id: id.into(),
                expected_revision: revision,
                enabled_model_ids: enabled.iter().map(|s| (*s).into()).collect(),
                selected_model_id: selected.map(str::to_owned),
                updated_at_ms: 10,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn model_connections_probe_evidence_overrides_and_durable_rollback() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("connections-durable.sqlite");
    let mut store = NolaStore::open(&path).unwrap();

    // 1. Empty connections survive reopen without discovery and reject duplicate None revision.
    let empty_first = save_conn(&mut store, make_connection("empty-one"), None);
    assert_eq!(empty_first.revision, 1);
    assert!(store.list_model_profiles().unwrap().is_empty());
    assert!(conn_execute(
        &mut store,
        ModelCommand::SaveConnection {
            input: make_connection("empty-one"),
            expected_revision: None,
            updated_at_ms: 2,
        }
    )
    .unwrap_err()
    .contains("revision_conflict"));

    // 2. Durable connection write failure rolls back created runtime profiles ("rollback-one").
    let rb_saved = save_conn(&mut store, make_connection("rollback-one"), None);
    let rb_cat = refresh_conn(
        &mut store,
        "rollback-one",
        rb_saved.revision,
        vec![catalog_model("alpha", ModelProtocol::Responses)],
        2,
    );
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(
        "CREATE TRIGGER reject_connection BEFORE UPDATE ON product_settings
         WHEN NEW.key='model.connection.rollback-one' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;",
    )
    .unwrap();
    assert!(conn_execute(
        &mut store,
        ModelCommand::ConfigureConnectionModels {
            connection_id: "rollback-one".into(),
            expected_revision: rb_cat.revision,
            enabled_model_ids: vec!["responses:alpha".into()],
            selected_model_id: Some("responses:alpha".into()),
            updated_at_ms: 3,
        }
    )
    .is_err());
    assert!(store.list_model_profiles().unwrap().is_empty());
    assert_eq!(store.model_connection("rollback-one").unwrap().unwrap(), rb_cat);
    sql.execute_batch("DROP TRIGGER reject_connection;").unwrap();

    // 3. Capability evidence rejects independent account revision change ("account-probe").
    let account = ModelAccount {
        id: "probe-account".into(),
        display_name: "Probe account".into(),
        source: ModelSourceKind::Manual,
        credential_ref: Some("opaque-fixture".into()),
        state: ModelAccountState::Ready,
    };
    let saved_acc: ModelAccountSnapshot = serde_json::from_value(
        conn_execute(
            &mut store,
            ModelCommand::SaveAccount {
                input: account.clone(),
                expected_revision: None,
                updated_at_ms: 1,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let mut acc_route = make_connection("account-probe");
    acc_route.account_id = Some(account.id.clone());
    let acc_route = save_conn(&mut store, acc_route, None);
    let acc_route = refresh_conn(
        &mut store,
        "account-probe",
        acc_route.revision,
        vec![catalog_model("text", ModelProtocol::Responses)],
        2,
    );
    conn_execute(
        &mut store,
        ModelCommand::ReserveModelProbe {
            input: ModelProbeAttempt {
                request_id: "account-probe-attempt".into(),
                connection_id: "account-probe".into(),
                connection_revision: acc_route.revision,
                account_id: Some(account.id.clone()),
                account_revision: Some(saved_acc.revision),
                credential_binding: "a".repeat(64),
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                started_at_ms: 3,
                finished_at_ms: None,
                text_confirmed: None,
            },
        },
    )
    .unwrap();
    let mut denied_acc = account;
    denied_acc.state = ModelAccountState::Denied;
    conn_execute(
        &mut store,
        ModelCommand::SaveAccount {
            input: denied_acc,
            expected_revision: Some(saved_acc.revision),
            updated_at_ms: 4,
        },
    )
    .unwrap();
    assert!(conn_execute(
        &mut store,
        ModelCommand::RecordConnectionCapabilityEvidence {
            connection_id: "account-probe".into(),
            expected_revision: acc_route.revision,
            model_id: "text".into(),
            protocol: ModelProtocol::Responses,
            evidence: ModelCapabilityEvidence {
                request_id: "account-probe-attempt".into(),
                connection_revision: acc_route.revision,
                account_revision: Some(saved_acc.revision),
                credential_binding: "a".repeat(64),
                observed_at_ms: 5,
                requests_sent: 1,
                text_generation: true,
            },
        }
    )
    .is_err());

    // 4. Probe reservation uniqueness across cold reopen, user override vs declaration, and route invalidation ("probe").
    let probe_init = save_conn(&mut store, make_connection("probe"), None);
    let probe_cat = refresh_conn(
        &mut store,
        "probe",
        probe_init.revision,
        vec![catalog_model("text", ModelProtocol::Responses)],
        20,
    );
    let probe_sel = select_conn(
        &mut store,
        "probe",
        probe_cat.revision,
        &["responses:text"],
        Some("responses:text"),
    );
    let attempt = ModelProbeAttempt {
        request_id: "fixture-probe".into(),
        connection_id: "probe".into(),
        connection_revision: probe_sel.revision,
        account_id: None,
        account_revision: None,
        credential_binding: "a".repeat(64),
        model_id: "text".into(),
        protocol: ModelProtocol::Responses,
        started_at_ms: 30,
        finished_at_ms: None,
        text_confirmed: None,
    };
    conn_execute(
        &mut store,
        ModelCommand::ReserveModelProbe {
            input: attempt.clone(),
        },
    )
    .unwrap();
    assert!(conn_execute(
        &mut store,
        ModelCommand::ReserveModelProbe {
            input: attempt.clone()
        }
    )
    .is_err());

    let observed: ModelConnectionSnapshot = serde_json::from_value(
        conn_execute(
            &mut store,
            ModelCommand::RecordConnectionCapabilityEvidence {
                connection_id: "probe".into(),
                expected_revision: probe_sel.revision,
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                evidence: ModelCapabilityEvidence {
                    request_id: attempt.request_id.clone(),
                    connection_revision: probe_sel.revision,
                    account_revision: None,
                    credential_binding: attempt.credential_binding.clone(),
                    observed_at_ms: 40,
                    requests_sent: 1,
                    text_generation: true,
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    let overridden: ModelConnectionSnapshot = serde_json::from_value(
        conn_execute(
            &mut store,
            ModelCommand::SetConnectionCapabilityOverride {
                connection_id: "probe".into(),
                expected_revision: observed.revision,
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                capabilities: Some(nola_domain::model_capabilities::ModelExecutionCapabilities {
                    tool_calls: true,
                    ..Default::default()
                }),
                updated_at_ms: 50,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let profiles = conn_execute(&mut store, ModelCommand::ListProfiles).unwrap();
    let src: Value = serde_json::from_str(profiles[0]["source_json"].as_str().unwrap()).unwrap();
    assert_eq!(src["executionCapabilityBasis"], "user_override");
    assert_eq!(src["executionCapabilities"]["toolCalls"], true);

    let cleared: ModelConnectionSnapshot = serde_json::from_value(
        conn_execute(
            &mut store,
            ModelCommand::SetConnectionCapabilityOverride {
                connection_id: "probe".into(),
                expected_revision: overridden.revision,
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                capabilities: None,
                updated_at_ms: 60,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(cleared.document.capability_assessments.as_ref().unwrap()[0]
        .evidence
        .is_some());
    let profiles = conn_execute(&mut store, ModelCommand::ListProfiles).unwrap();
    let src: Value = serde_json::from_str(profiles[0]["source_json"].as_str().unwrap()).unwrap();
    assert_eq!(src["executionCapabilityBasis"], "catalog_declaration");
    assert_eq!(src["executionCapabilities"]["toolCalls"], false);

    let mut changed_route = cleared.document.connection.clone();
    changed_route.base_url = "https://other.fixture.invalid/v1".into();
    let changed_snap = save_conn(&mut store, changed_route, Some(cleared.revision));
    assert!(changed_snap.document.capability_assessments.is_none());
    drop(store);

    let mut reopened = NolaStore::open(&path).unwrap();
    assert_eq!(reopened.list_model_connections().unwrap().len(), 4);
    assert!(conn_execute(
        &mut reopened,
        ModelCommand::ReserveModelProbe { input: attempt }
    )
    .is_err());
}

#[test]
fn model_connections_import_catalog_refresh_protocols_and_capability_resolution() {
    // 1. ImportConnection atomic rollback & shared-account binding guard.
    {
        let mut store = NolaStore::open(":memory:").unwrap();
        let mut route = make_connection("atomic");
        route.account_id = Some("account".into());
        let account = ModelAccount {
            id: "account".into(),
            display_name: "Account".into(),
            source: ModelSourceKind::Manual,
            credential_ref: Some("private-reference".into()),
            state: ModelAccountState::Ready,
        };
        let mut invalid = catalog_model("invalid", ModelProtocol::Responses);
        invalid.context_window = Some(0);
        let import_cmd = |models| ModelCommand::ImportConnection {
            input: route.clone(),
            expected_revision: None,
            account: Some(account.clone()),
            expected_account_revision: None,
            models,
            enable_models: true,
            updated_at_ms: 10,
        };
        assert!(conn_execute(&mut store, import_cmd(vec![invalid])).is_err());
        assert_eq!(conn_execute(&mut store, ModelCommand::ListAccounts).unwrap(), json!([]));
        assert_eq!(conn_execute(&mut store, ModelCommand::ListConnections).unwrap(), json!([]));
        assert!(store.list_model_profiles().unwrap().is_empty());

        conn_execute(
            &mut store,
            import_cmd(vec![catalog_model("valid", ModelProtocol::Responses)]),
        )
        .unwrap();
        let before_acc = conn_execute(&mut store, ModelCommand::ListAccounts).unwrap();
        let before_conn = conn_execute(&mut store, ModelCommand::ListConnections).unwrap();
        let before_prof = store.list_model_profiles().unwrap();
        let mut rep_acc = account.clone();
        rep_acc.credential_ref = Some("new-private-reference".into());
        assert!(conn_execute(
            &mut store,
            ModelCommand::ImportConnection {
                input: route.clone(),
                expected_revision: Some(999),
                account: Some(rep_acc),
                expected_account_revision: Some(before_acc[0]["revision"].as_i64().unwrap()),
                models: vec![catalog_model("replacement", ModelProtocol::Responses)],
                enable_models: true,
                updated_at_ms: 20,
            }
        )
        .is_err());
        assert_eq!(conn_execute(&mut store, ModelCommand::ListAccounts).unwrap(), before_acc);
        assert_eq!(conn_execute(&mut store, ModelCommand::ListConnections).unwrap(), before_conn);
        assert_eq!(store.list_model_profiles().unwrap(), before_prof);

        let mut shared = make_connection("shared");
        shared.account_id = route.account_id.clone();
        save_conn(&mut store, shared, None);
        assert!(conn_execute(
            &mut store,
            ModelCommand::ImportConnection {
                input: route,
                expected_revision: Some(before_conn[0]["revision"].as_i64().unwrap()),
                account: Some(account),
                expected_account_revision: Some(before_acc[0]["revision"].as_i64().unwrap()),
                models: vec![],
                enable_models: false,
                updated_at_ms: 30,
            }
        )
        .unwrap_err()
        .contains("import_account_shared_requires_separate_binding"));
    }

    // 2. Shared in-memory store exercising account secrets, duplicate sources, unowned writes,
    //    omission/return overrides, refresh selection stability, multi-protocol identity,
    //    route change baseline, unknown stored format, stale profile revision, tool defaults,
    //    sparse chat efforts, and observed anthropic efforts.
    let mut store = NolaStore::open(":memory:").unwrap();

    // Account secret rejection & source immutability.
    let codex_acc = ModelAccount {
        id: "codex-acc".into(),
        display_name: "Subscription".into(),
        source: ModelSourceKind::Codex,
        credential_ref: Some("vault-handle".into()),
        state: ModelAccountState::Ready,
    };
    conn_execute(
        &mut store,
        ModelCommand::SaveAccount {
            input: codex_acc.clone(),
            expected_revision: None,
            updated_at_ms: 1,
        },
    )
    .unwrap();
    let mut malicious = serde_json::to_value(&codex_acc).unwrap();
    malicious["api_key"] = json!("not-a-real-secret");
    assert!(serde_json::from_value::<ModelAccount>(malicious).is_err());
    let mut altered_acc = codex_acc.clone();
    altered_acc.source = ModelSourceKind::KimiWork;
    assert!(conn_execute(
        &mut store,
        ModelCommand::SaveAccount {
            input: altered_acc,
            expected_revision: Some(1),
            updated_at_ms: 2
        }
    )
    .is_err());

    // Duplicate OpenCode source & unowned profile/setting writes.
    let mut opencode = make_connection("opencode-one");
    opencode.source = ModelSourceKind::OpenCode;
    opencode.source_id = Some("provider-entry".into());
    let oc_saved = save_conn(&mut store, opencode.clone(), None);
    opencode.id = "opencode-two".into();
    assert!(conn_execute(
        &mut store,
        ModelCommand::SaveConnection {
            input: opencode,
            expected_revision: None,
            updated_at_ms: 2
        }
    )
    .unwrap_err()
    .contains("source_already_imported"));
    let oc_cat = refresh_conn(
        &mut store,
        "opencode-one",
        oc_saved.revision,
        vec![catalog_model("alpha", ModelProtocol::Responses)],
        3,
    );
    select_conn(
        &mut store,
        "opencode-one",
        oc_cat.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let oc_prof = store.list_model_profiles().unwrap().remove(0);
    assert!(conn_execute(
        &mut store,
        ModelCommand::DeleteProfile {
            id: oc_prof.id.clone(),
            deleted_at_ms: 4
        }
    )
    .is_err());
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::PutProductSetting {
            key: "model.connection.opencode-one".into(),
            value_json: "{}".into(),
            updated_at_ms: 4,
        }
    )
    .is_err());

    // Unknown stored format_version rejected without rewriting data.
    let fmt_saved = save_conn(&mut store, make_connection("bad-format"), None);
    let mut corrupt = serde_json::to_value(fmt_saved.document).unwrap();
    corrupt["format_version"] = json!(9);
    let raw_corrupt = corrupt.to_string();
    let setting = store
        .put_product_setting("model.connection.bad-format", &raw_corrupt, 2)
        .unwrap();
    assert!(conn_execute(
        &mut store,
        ModelCommand::SaveConnection {
            input: make_connection("bad-format"),
            expected_revision: Some(setting.revision),
            updated_at_ms: 3,
        }
    )
    .is_err());
    assert_eq!(
        store
            .product_setting("model.connection.bad-format")
            .unwrap()
            .unwrap()
            .value_json,
        raw_corrupt
    );

    // Catalog omission & return preserve user capability override, and refresh never removes manual models.
    let mut mem = NolaStore::open(":memory:").unwrap();
    let init = save_conn(&mut mem, make_connection("one"), None);
    let cat = refresh_conn(
        &mut mem,
        "one",
        init.revision,
        vec![
            catalog_model("alpha", ModelProtocol::Responses),
            catalog_model("beta", ModelProtocol::Responses),
        ],
        2,
    );
    assert!(mem.list_model_profiles().unwrap().is_empty());
    let overridden: ModelConnectionSnapshot = serde_json::from_value(
        conn_execute(
            &mut mem,
            ModelCommand::SetConnectionCapabilityOverride {
                connection_id: "one".into(),
                expected_revision: cat.revision,
                model_id: "alpha".into(),
                protocol: ModelProtocol::Responses,
                capabilities: Some(
                    nola_domain::model_capabilities::ModelExecutionCapabilities {
                        tool_calls: false,
                        ..Default::default()
                    },
                ),
                updated_at_ms: 3,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let sel = select_conn(
        &mut mem,
        "one",
        overridden.revision,
        &["responses:alpha", "responses:beta"],
        Some("responses:alpha"),
    );
    let disabled = select_conn(
        &mut mem,
        "one",
        sel.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let before_refresh = mem.list_model_profiles().unwrap();
    let absent = refresh_conn(
        &mut mem,
        "one",
        disabled.revision,
        vec![catalog_model("beta", ModelProtocol::Responses)],
        20,
    );
    assert_eq!(before_refresh, mem.list_model_profiles().unwrap());
    assert_eq!(
        absent.document.capability_assessments,
        overridden.document.capability_assessments
    );
    let returned = refresh_conn(
        &mut mem,
        "one",
        absent.revision,
        vec![catalog_model("alpha", ModelProtocol::Responses)],
        21,
    );
    let returned_sel = select_conn(
        &mut mem,
        "one",
        returned.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let alpha_prof = mem
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.model_name == "alpha")
        .unwrap();
    let alpha_src: Value = serde_json::from_str(&alpha_prof.source_json).unwrap();
    assert_eq!(alpha_src["executionCapabilityBasis"], "user_override");
    assert_eq!(alpha_src["executionCapabilities"]["toolCalls"], false);

    // Stale profile revision cannot overwrite parameters in commit_model_connection.
    let stale_profs = mem.list_model_profiles().unwrap();
    let stale_revs = stale_profs.iter().map(|p| (p.id.clone(), p.revision)).collect();
    let mut edited_param = stale_profs[0].clone();
    edited_param.parameters_json = "{\"maxOutputTokens\":1000}".into();
    mem.upsert_model_profile(&edited_param).unwrap();
    assert!(mem
        .commit_model_connection(
            &returned_sel.document,
            Some(returned_sel.revision),
            &stale_profs,
            &stale_revs,
            25
        )
        .is_err());
    assert_eq!(
        mem.list_model_profiles().unwrap()[0].parameters_json,
        edited_param.parameters_json
    );

    // Same model on distinct protocols & route change invalidation -> protocol_agent_baseline.
    let mut proto_mem = NolaStore::open(":memory:").unwrap();
    let p_saved = save_conn(&mut proto_mem, make_connection("proto"), None);
    let p_cat = refresh_conn(
        &mut proto_mem,
        "proto",
        p_saved.revision,
        vec![
            catalog_model("same", ModelProtocol::Responses),
            catalog_model("same", ModelProtocol::ChatCompletions),
        ],
        2,
    );
    let p_first = select_conn(
        &mut proto_mem,
        "proto",
        p_cat.revision,
        &["responses:same", "chat_completions:same"],
        Some("responses:same"),
    );
    let two_profs = proto_mem.list_model_profiles().unwrap();
    assert_eq!(two_profs.len(), 2);
    assert_ne!(two_profs[0].id, two_profs[1].id);
    let p_switched = select_conn(
        &mut proto_mem,
        "proto",
        p_first.revision,
        &["responses:same", "chat_completions:same"],
        Some("chat_completions:same"),
    );
    assert_eq!(
        proto_mem
            .list_model_profiles()
            .unwrap()
            .iter()
            .filter(|p| p.selected)
            .count(),
        1
    );
    let mut renamed_conn = make_connection("proto");
    renamed_conn.display_name = "Renamed".into();
    let p_renamed = save_conn(&mut proto_mem, renamed_conn, Some(p_switched.revision));
    let after_rename = proto_mem.list_model_profiles().unwrap();
    assert!(two_profs.iter().all(|p| after_rename.iter().any(|a| a.id == p.id)));

    let mut route_changed = make_connection("proto");
    route_changed.base_url = "https://other.invalid/v1".into();
    let p_changed = save_conn(&mut proto_mem, route_changed, Some(p_renamed.revision));
    assert!(p_changed.document.catalog.is_none());
    assert!(conn_execute(
        &mut proto_mem,
        ModelCommand::RefreshConnectionCatalog {
            connection_id: "proto".into(),
            expected_revision: p_renamed.revision,
            models: vec![catalog_model("same", ModelProtocol::Responses)],
            observed_at_ms: 30,
        }
    )
    .unwrap_err()
    .contains("revision_conflict"));
    select_conn(
        &mut proto_mem,
        "proto",
        p_changed.revision,
        &["responses:same"],
        Some("responses:same"),
    );
    let baseline_src: Value = serde_json::from_str(
        &proto_mem
            .list_model_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.enabled)
            .unwrap()
            .source_json,
    )
    .unwrap();
    assert_eq!(baseline_src["executionCapabilityBasis"], "protocol_agent_baseline");
    assert_eq!(baseline_src["executionCapabilities"]["toolCalls"], true);

    // Unknown catalog capabilities, imported tool defaults, sparse chat controls, and anthropic efforts.
    let mut caps_mem = NolaStore::open(":memory:").unwrap();
    let c_saved = save_conn(&mut caps_mem, make_connection("caps"), None);
    let mut unknown = catalog_model("unknown", ModelProtocol::Responses);
    unknown.execution_capabilities = None;
    let mut effort_m = catalog_model("effort", ModelProtocol::Responses);
    effort_m.execution_capabilities = None;
    effort_m.reasoning_levels = vec!["low".into(), "high".into()];
    let mut sparse_chat = catalog_model("sparse", ModelProtocol::ChatCompletions);
    sparse_chat.execution_capabilities = None;
    sparse_chat.reasoning_levels = ["default", "none", "low", "high", "max", "ultra"]
        .map(str::to_owned)
        .to_vec();
    let c_cat = refresh_conn(
        &mut caps_mem,
        "caps",
        c_saved.revision,
        vec![
            catalog_model("known", ModelProtocol::Responses),
            unknown,
            effort_m,
            sparse_chat.clone(),
        ],
        2,
    );
    select_conn(
        &mut caps_mem,
        "caps",
        c_cat.revision,
        &[
            "responses:known",
            "responses:unknown",
            "responses:effort",
            "chat_completions:sparse",
        ],
        Some("responses:known"),
    );
    let c_profs = caps_mem.list_model_profiles().unwrap();
    let unk_src: Value = serde_json::from_str(
        &c_profs
            .iter()
            .find(|p| p.model_name == "unknown")
            .unwrap()
            .source_json,
    )
    .unwrap();
    assert_eq!(unk_src["executionCapabilityBasis"], "protocol_agent_baseline");
    assert_eq!(unk_src["executionCapabilities"]["toolCalls"], true);

    // Imported tools default on, then provider false overrides on refresh + configure.
    let cur_caps = caps_mem.model_connection("caps").unwrap().unwrap();
    let mut updated_entries = cur_caps.document.catalog.unwrap().models;
    updated_entries
        .iter_mut()
        .find(|m| m.model_id == "unknown")
        .unwrap()
        .execution_capabilities =
        Some(nola_domain::model_capabilities::ModelExecutionCapabilities::for_import(Some(false)));
    let c_cat2 = refresh_conn(&mut caps_mem, "caps", cur_caps.revision, updated_entries, 4);
    select_conn(
        &mut caps_mem,
        "caps",
        c_cat2.revision,
        &[
            "responses:known",
            "responses:unknown",
            "responses:effort",
            "chat_completions:sparse",
        ],
        Some("responses:known"),
    );
    let unk_src2: Value = serde_json::from_str(
        &caps_mem
            .list_model_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.model_name == "unknown")
            .unwrap()
            .source_json,
    )
    .unwrap();
    assert_eq!(unk_src2["executionCapabilities"]["toolCalls"], false);
    assert_eq!(unk_src2["executionCapabilityBasis"], "catalog_declaration");

    // Sparse chat efforts resolution.
    let sparse_prof = caps_mem
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.model_name == "sparse")
        .unwrap();
    let sparse_src: Value = serde_json::from_str(&sparse_prof.source_json).unwrap();
    assert_eq!(sparse_src["reasoning"]["efforts"], json!(["none", "low", "high", "max"]));
    for eff in ["none", "low", "high", "max"] {
        let res = execute_application_command(
            &mut caps_mem,
            ApplicationCommand::ResolveRemoteModelProfile {
                profile_id: sparse_prof.id.clone(),
                expected_revision: sparse_prof.revision,
                request: json!({"reasoning_selection":{"kind":"effort","value":eff}}),
            },
        )
        .unwrap();
        assert_eq!(
            res["resolved"]["runtime"]["reasoning_selection"],
            json!({"kind":"effort","value":eff})
        );
    }
    for unsupported in ["medium", "ultra"] {
        assert!(execute_application_command(
            &mut caps_mem,
            ApplicationCommand::ResolveRemoteModelProfile {
                profile_id: sparse_prof.id.clone(),
                expected_revision: sparse_prof.revision,
                request: json!({"reasoning_selection":{"kind":"effort","value":unsupported}}),
            }
        )
        .is_err());
    }

    // Observed Anthropic efforts admitted without changing provider defaults.
    let mut anth_route = make_connection("anthropic");
    anth_route.protocols = vec![ModelProtocol::Anthropic];
    let a_saved = save_conn(&mut caps_mem, anth_route, None);
    let mut a_disc = catalog_model("adaptive-observed", ModelProtocol::Anthropic);
    a_disc.execution_capabilities = None;
    a_disc.reasoning_levels = vec!["low".into(), "high".into()];
    let a_cat = refresh_conn(&mut caps_mem, "anthropic", a_saved.revision, vec![a_disc], 5);
    select_conn(
        &mut caps_mem,
        "anthropic",
        a_cat.revision,
        &["anthropic:adaptive-observed"],
        Some("anthropic:adaptive-observed"),
    );
    let a_prof = caps_mem
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.model_name == "adaptive-observed")
        .unwrap();
    assert!(
        execute_application_command(
            &mut caps_mem,
            ApplicationCommand::ResolveRemoteModelProfile {
                profile_id: a_prof.id.clone(),
                expected_revision: a_prof.revision,
                request: json!({"reasoning_selection":null}),
            }
        )
        .unwrap()["resolved"]["runtime"]["reasoning_selection"]
            .is_null()
    );
    let a_high = execute_application_command(
        &mut caps_mem,
        ApplicationCommand::ResolveRemoteModelProfile {
            profile_id: a_prof.id.clone(),
            expected_revision: a_prof.revision,
            request: json!({"reasoning_selection":{"kind":"effort","value":"high"}}),
        },
    )
    .unwrap();
    assert_eq!(
        a_high["resolved"]["runtime"]["reasoning_selection"],
        json!({"kind":"effort","value":"high"})
    );
    assert!(execute_application_command(
        &mut caps_mem,
        ApplicationCommand::ResolveRemoteModelProfile {
            profile_id: a_prof.id,
            expected_revision: a_prof.revision,
            request: json!({"reasoning_selection":{"kind":"effort","value":"none"}}),
        }
    )
    .is_err());
}
