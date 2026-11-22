// SPDX-License-Identifier: Apache-2.0
//! Consolidated Conversation Profile, Metadata & Effective Model Parameters suite:
//! non-SQLite identity repository, configuration CAS, active-policy notification,
//! metadata query/patch, seed/connectedHost/zero-value generation contracts, and
//! effective model parameter resolution.

use base64::Engine;
use nola_application::{
    conversation::{create_conversation, rename_conversation, CreateConversation},
    effective_model_parameters::{self as parameters, *},
    execute_application_command, ApplicationCommand,
};
use nola_domain::{
    conversation_configuration::{ConfigurationCommitResult, ConversationTitleSource},
    generation::GenerationOptions,
};
use nola_host_protocol::HostModelProfileReference;
use nola_store::NolaStore;
use nola_store_port::{
    conversation_profile::{
        ConversationProfileRepository, CreateConversationInput, CreateConversationResult,
        PatchConversationConfigurationInput, PatchConversationMetadataInput,
    },
    model::StoredModelProfile,
};
use serde_json::{json, Value};

#[derive(Default)]
struct RestrictedProfileRepo {
    writes: Vec<String>,
}
impl ConversationProfileRepository for RestrictedProfileRepo {
    type Error = &'static str;
    fn patch_conversation_metadata(
        &mut self,
        _: &PatchConversationMetadataInput,
    ) -> Result<Option<i64>, Self::Error> {
        unreachable!()
    }
    fn patch_conversation_configuration(
        &mut self,
        _: &PatchConversationConfigurationInput,
    ) -> Result<ConfigurationCommitResult, Self::Error> {
        unreachable!()
    }
    fn create_conversation(
        &mut self,
        input: &CreateConversationInput,
    ) -> Result<CreateConversationResult, Self::Error> {
        assert_eq!(input.revision, 1);
        self.writes.push(format!("create:{}", input.id));
        Ok(CreateConversationResult {
            created: true,
            initial_configuration_revision: Some(1),
        })
    }
    fn rename_conversation(
        &mut self,
        id: &str,
        title: &str,
        _: Option<ConversationTitleSource>,
        _: i64,
    ) -> Result<bool, Self::Error> {
        self.writes.push(format!("rename:{id}:{title}"));
        Ok(true)
    }
}

#[test]
fn conversation_profile_metadata_and_generation_configuration_lifecycle() {
    // 1. Restricted non-SQLite repository contract & removed whole-profile wire aliases.
    let mut repository = RestrictedProfileRepo::default();
    assert!(create_conversation(
        &mut repository,
        CreateConversation {
            id: "c".into(),
            title: "Original".into(),
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    )
    .unwrap()
    .created);
    assert!(rename_conversation(&mut repository, "c", "  New title  ", None, 2).unwrap());
    assert!(rename_conversation(&mut repository, "c", " ", None, 3).is_err());
    assert_eq!(repository.writes, ["create:c", "rename:c:New title"]);

    for command in ["update_conversation_settings", "update_conversation_profile"] {
        assert!(serde_json::from_value::<ApplicationCommand>(json!({
            "type": command,
            "input": {"conversation_id":"c","settings_json":"{}","title":"stale","updated_at_ms":1}
        }))
        .is_err());
    }

    // 2. Shared SQLite database exercising configuration CAS, active-policy notification,
    //    metadata read/patch, seed intent, connectedHost isolation, and zero-value generation options.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("profile-metadata.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();

    for (id, title, settings) in [
        ("cas", "Original", "{}"),
        ("policy", "policy", "{}"),
        ("policy-other", "policy-other", "{}"),
        (
            "shared",
            "Not part of metadata response",
            r#"{"isPinned":false,"knowledge":{"wikiMarkdown":"keep"}}"#,
        ),
        ("seed", "Seed", "{}"),
        ("remote", "Remote intent", "{}"),
        ("gen", "Generation contract", "{}"),
    ] {
        store
            .create_conversation(&CreateConversationInput {
                id: id.into(),
                title: title.into(),
                revision: 1,
                settings_json: Some(settings.into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }

    // Configuration CAS independent of title & transcript ("cas").
    let cas_cmd = |revision, value| ApplicationCommand::PatchConversationConfiguration {
        input: PatchConversationConfigurationInput {
            conversation_id: "cas".into(),
            expected_revision: revision,
            patch_json: json!({"permissionProfile": value})
                .to_string()
                .parse()
                .unwrap(),
            updated_at_ms: 2,
        },
    };
    store
        .rename_conversation("cas", "New title", None, 2)
        .unwrap();
    let committed = execute_application_command(&mut store, cas_cmd(1, "cautious")).unwrap();
    assert_eq!(committed["status"], "committed");
    assert_eq!(committed["snapshot"]["configurationRevision"], 2);
    let stale = execute_application_command(&mut store, cas_cmd(1, "allow_all")).unwrap();
    assert_eq!(
        stale,
        json!({"status":"conflict","conversation_id":"cas","expected_revision":1,"actual_revision":2})
    );

    // Active policy notification owner-bound & atomic with configuration CAS ("policy").
    db.execute_batch(
        "INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
         VALUES('r','policy','conversation','running',1,1,1),('other-r','policy-other','conversation','running',1,1,1);",
    )
    .unwrap();
    let policy_patch = |revision, mode| ApplicationCommand::PatchConversationConfiguration {
        input: PatchConversationConfigurationInput {
            conversation_id: "policy".into(),
            expected_revision: revision,
            patch_json: json!({"permissionProfile": mode})
                .to_string()
                .parse()
                .unwrap(),
            updated_at_ms: 2,
        },
    };
    let policy_committed =
        execute_application_command(&mut store, policy_patch(1, "allow_all")).unwrap();
    assert_eq!(policy_committed["snapshot"]["configurationRevision"], 2);
    let (owner, cmd_id, payload): (String, String, String) = db
        .query_row(
            "SELECT session_id,command_id,payload_json FROM agent_commands",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!((owner.as_str(), cmd_id.as_str()), ("policy", "permission-policy:r:2"));
    assert_eq!(
        serde_json::from_str::<Value>(&payload).unwrap(),
        json!({"type":"permission_policy_available","policy":{"run_id":"r","configuration_revision":2,"profile":"allow_all"}})
    );
    assert_eq!(
        execute_application_command(&mut store, policy_patch(1, "cautious")).unwrap()["status"],
        "conflict"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_commands", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch(
        "CREATE TRIGGER reject_policy_notice BEFORE INSERT ON agent_commands
         BEGIN SELECT RAISE(ABORT,'policy notice fault'); END;",
    )
    .unwrap();
    assert!(execute_application_command(&mut store, policy_patch(2, "cautious"))
        .unwrap_err()
        .contains("policy notice fault"));
    assert_eq!(
        store
            .load_conversation("policy")
            .unwrap()
            .unwrap()
            .configuration_revision,
        2
    );
    db.execute_batch("DROP TRIGGER reject_policy_notice")
        .unwrap();

    // Metadata read without message bodies + revision-checked metadata mutation ("shared").
    db.execute(
        "INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms)
         VALUES('message','shared','user','completed',1,1)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
         VALUES('part','message','text',1,'completed',?1,1,1)",
        ["PRIVATE TRANSCRIPT ".repeat(100_000)],
    )
    .unwrap();
    let load_meta: ApplicationCommand = serde_json::from_value(json!({
        "type": "load_conversation_metadata", "conversation_id": "shared"
    }))
    .unwrap();
    assert!(load_meta.is_read_only_query());
    let metadata = execute_application_command(&mut store, load_meta).unwrap();
    assert_eq!(
        metadata,
        json!({
            "id": "shared",
            "revision": 1,
            "configuration_revision": 1,
            "settings_json": r#"{"isPinned":false,"knowledge":{"wikiMarkdown":"keep"}}"#
        })
    );
    assert_eq!(
        execute_application_command(
            &mut store,
            ApplicationCommand::LoadConversationMetadata {
                conversation_id: "missing".into()
            }
        )
        .unwrap(),
        Value::Null
    );
    assert_eq!(
        db.query_row(
            "SELECT length(content_text) FROM message_parts WHERE id='part'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1_900_000
    );

    let meta_wire = json!({
        "type": "patch_conversation_metadata",
        "input": {
            "conversation_id": "shared",
            "expected_revision": 1,
            "patch_json": "{\"isPinned\":true}",
            "updated_at_ms": 2
        }
    });
    let meta_cmd: ApplicationCommand = serde_json::from_value(meta_wire.clone()).unwrap();
    assert!(!meta_cmd.is_read_only_query());
    assert_eq!(execute_application_command(&mut store, meta_cmd).unwrap(), json!(2));
    assert_eq!(
        execute_application_command(&mut store, serde_json::from_value(meta_wire).unwrap())
            .unwrap(),
        Value::Null
    );
    let saved_meta: Value =
        serde_json::from_str(&store.load_conversation_settings("shared").unwrap().unwrap())
            .unwrap();
    assert_eq!(saved_meta["knowledge"]["wikiMarkdown"], "keep");
    assert_eq!(saved_meta["isPinned"], true);
    for invalid_seed in [json!(0), Value::Null, json!({"kind":"value", "value":"01"})] {
        let invalid = serde_json::from_value(json!({
            "type": "patch_conversation_metadata",
            "input": {
                "conversation_id": "shared",
                "expected_revision": 2,
                "patch_json": json!({"modelOverrides":{"configured:model":{"seed":invalid_seed}}}).to_string(),
                "updated_at_ms": 3
            }
        }))
        .unwrap();
        assert!(execute_application_command(&mut store, invalid).is_err());
    }
    let remove_overrides = serde_json::from_value(json!({
        "type": "patch_conversation_metadata",
        "input": {
            "conversation_id": "shared",
            "expected_revision": 2,
            "patch_json": "{\"modelOverrides\":null}",
            "updated_at_ms": 3
        }
    }))
    .unwrap();
    assert!(execute_application_command(&mut store, remove_overrides).is_err());

    // Seed intent exact values and null inheritance ("seed").
    for (index, seed_val) in [
        json!({"kind":"clear"}),
        json!({"kind":"value","value":"0"}),
        json!({"kind":"value","value":i64::MIN.to_string()}),
        json!({"kind":"value","value":i64::MAX.to_string()}),
        Value::Null,
    ]
    .iter()
    .enumerate()
    {
        execute_application_command(
            &mut store,
            ApplicationCommand::PatchConversationConfiguration {
                input: PatchConversationConfigurationInput {
                    conversation_id: "seed".into(),
                    expected_revision: index as i64 + 1,
                    patch_json: json!({"modelOverrides":{"configured:model":{"seed":seed_val}}})
                        .to_string()
                        .parse()
                        .unwrap(),
                    updated_at_ms: index as i64 + 2,
                },
            },
        )
        .unwrap();
        let raw: String = db
            .query_row(
                "SELECT settings_json FROM conversation_settings WHERE conversation_id='seed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        if seed_val.is_null() {
            assert!(saved["modelOverrides"]["configured:model"]
                .get("seed")
                .is_none());
        } else {
            assert_eq!(&saved["modelOverrides"]["configured:model"]["seed"], seed_val);
        }
    }

    // Connected-host override isolation ("remote").
    let host_key = |host: &str| {
        format!(
            "connectedHost:{}:{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(host),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("same-profile")
        )
    };
    let first_key = host_key("host:first");
    let second_key = host_key("host:second");
    let mut host_models = serde_json::Map::new();
    host_models.insert(
        first_key.clone(),
        json!({"temperature":0,"seed":{"kind":"value","value":"0"}}),
    );
    host_models.insert(second_key.clone(), json!({"temperature":1}));
    execute_application_command(
        &mut store,
        ApplicationCommand::PatchConversationConfiguration {
            input: PatchConversationConfigurationInput {
                conversation_id: "remote".into(),
                expected_revision: 1,
                patch_json: json!({"modelOverrides": host_models})
                    .to_string()
                    .parse()
                    .unwrap(),
                updated_at_ms: 2,
            },
        },
    )
    .unwrap();

    // Invalid generation rejection & valid zero-values ("gen").
    let original_gen: Value = serde_json::from_str(
        &db.query_row::<String, _, _>(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id='gen'",
            [],
            |r| r.get(0),
        )
        .unwrap(),
    )
    .unwrap();
    let sync_before = store.pending_sync_changes(100).unwrap().len();
    for bad_fields in [
        json!({"topK":0}),
        json!({"topK":1.5}),
        json!({"seed":"0"}),
        json!({"frequencyPenalty":2.1}),
        json!({"repetitionPenalty":0.5}),
    ] {
        assert!(json!({"modelOverrides":{"configured:model":bad_fields}})
            .to_string()
            .parse::<nola_domain::configuration_patch::ConversationConfigurationPatch>()
            .is_err());
        assert_eq!(store.pending_sync_changes(100).unwrap().len(), sync_before);
    }
    let unchanged_gen: Value = serde_json::from_str(
        &db.query_row::<String, _, _>(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id='gen'",
            [],
            |r| r.get(0),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(unchanged_gen, original_gen);

    execute_application_command(
        &mut store,
        ApplicationCommand::PatchConversationConfiguration {
            input: PatchConversationConfigurationInput {
                conversation_id: "gen".into(),
                expected_revision: 1,
                patch_json: json!({"modelOverrides":{"configured:model":{"topK":1,"seed":{"kind":"value","value":"0"},"presencePenalty":0}}})
                    .to_string()
                    .parse()
                    .unwrap(),
                updated_at_ms: 3,
            },
        },
    )
    .unwrap();

    // Cold reopen verification across all mutated conversations.
    drop(store);
    let reopened = NolaStore::open(&path).unwrap();
    let cas_conv = reopened.load_conversation("cas").unwrap().unwrap();
    assert_eq!(cas_conv.title, "New title");
    assert_eq!(cas_conv.configuration_revision, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&cas_conv.settings_json.unwrap()).unwrap()
            ["permissionModeRawValue"],
        "cautious"
    );
    assert_eq!(
        reopened
            .load_conversation("policy")
            .unwrap()
            .unwrap()
            .configuration_revision,
        2
    );
    let remote_saved: Value = serde_json::from_str(
        &db.query_row::<String, _, _>(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id='remote'",
            [],
            |r| r.get(0),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        remote_saved["modelOverrides"][&first_key],
        json!({"temperature":0,"seed":{"kind":"value","value":"0"}})
    );
    assert_eq!(remote_saved["modelOverrides"][&second_key], json!({"temperature":1}));
    assert!(remote_saved["modelOverrides"]
        .get("configured:same-profile")
        .is_none());

    let gen_saved: Value = serde_json::from_str(
        &db.query_row::<String, _, _>(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id='gen'",
            [],
            |r| r.get(0),
        )
        .unwrap(),
    )
    .unwrap();
    let fields = &gen_saved["modelOverrides"]["configured:model"];
    assert_eq!(fields["seed"], json!({"kind":"value","value":"0"}));
    assert_eq!(fields["topK"], 1);
    let execution = GenerationOptions {
        top_k: fields["topK"].as_u64(),
        seed: serde_json::from_value::<nola_domain::generation_override::SeedOverride>(
            fields["seed"].clone(),
        )
        .unwrap()
        .resolved(),
        presence_penalty: fields["presencePenalty"].as_f64(),
        ..Default::default()
    };
    assert!(execution.validate().is_ok());
}

fn base_profile() -> StoredModelProfile {
    StoredModelProfile {
        id: "model".into(),
        display_name: "Private configured name".into(),
        provider: "fixture".into(),
        model_name: "model-name".into(),
        runtime: "remote".into(),
        source_json: json!({
            "baseURL": "https://example.invalid/private/v1",
            "credentialRef": "never-in-query",
            "wireProtocol": "chat_completions",
            "capabilities": ["chat"],
            "reasoning": null,
            "executionCapabilities": {
                "toolCalls": true,
                "imageInput": false,
                "reasoning": false,
                "samplingParameters": true
            }
        })
        .to_string(),
        parameters_json: json!({
            "temperature": 0.7,
            "topP": 0.9,
            "maxOutputTokens": 8192,
            "maxContextLength": 64000,
            "seed": 17,
            "presencePenalty": 0.4
        })
        .to_string(),
        selected: true,
        enabled: true,
        revision: 1,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    }
}

fn current_input(store: &NolaStore, conversation_id: &str) -> ResolveEffectiveModelParameters {
    ResolveEffectiveModelParameters::CurrentConversation {
        conversation_id: conversation_id.into(),
        expected_configuration_revision: store
            .load_conversation(conversation_id)
            .unwrap()
            .unwrap()
            .configuration_revision,
        expected_model_profile: HostModelProfileReference {
            profile_id: "model".into(),
            admitted_revision: store.model_profile("model").unwrap().unwrap().revision,
        },
    }
}

fn resolve_json(store: &mut NolaStore, conversation_id: &str) -> Value {
    let input = current_input(store, conversation_id);
    serde_json::to_value(parameters::resolve(store, &input).unwrap()).unwrap()
}

#[test]
fn effective_model_parameters_resolution_pins_and_previews() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("parameters.db");
    let mut store = NolaStore::open(&path).unwrap();
    let profile = base_profile();
    store.upsert_model_profile(&profile).unwrap();
    store
        .put_product_setting(
            nola_application::model_selection::DEFAULT_CHAT_MODEL_KEY,
            &json!({"version":1,"selection":{"kind":"profile","profile_id":"model"}}).to_string(),
            1,
        )
        .unwrap();

    let create_sparse = |store: &mut NolaStore, id: &str, overrides: &Value| {
        store
            .create_conversation(&CreateConversationInput {
                id: id.into(),
                title: "Sparse".into(),
                revision: 1,
                settings_json: Some(
                    json!({
                        "modelOverrides": {"configured:model": overrides},
                        "clientModelPresentation": {"configured:model": {"responseStyle": "concise"}}
                    })
                    .to_string(),
                ),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    };

    // 1. Sparse & nullable scalars inherit without writes or private route material.
    for (idx, overrides) in [
        json!({"temperature":0.0}),
        json!({"temperature":0.0,"topP":null,"maxOutputTokens":null,"wireProtocol":null,"reasoning":null,"reasoningPreference":null}),
    ]
    .iter()
    .enumerate()
    {
        let cid = format!("sparse-{idx}");
        create_sparse(&mut store, &cid, overrides);
        let before_conv = store.load_conversation(&cid).unwrap().unwrap();
        let before_prof = store.model_profile("model").unwrap();
        let output = resolve_json(&mut store, &cid);
        assert_eq!(output["resolution"]["status"], "available");
        assert_eq!(output["resolution"]["temperature"], 0.0);
        assert_eq!(output["resolution"]["top_p"], 0.9);
        assert_eq!(output["resolution"]["max_output_tokens"], 8192);
        assert_eq!(&output["sparse_overrides"], overrides);
        assert_eq!(output["client_model_presentation"]["responseStyle"], "concise");
        assert_eq!(store.load_conversation(&cid).unwrap().unwrap(), before_conv);
        assert_eq!(store.model_profile("model").unwrap(), before_prof);
        let encoded = output.to_string();
        for forbidden in ["never-in-query", "example.invalid", "source_json", "credentialRef"] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }

    // 2. Seed clear/value/inherit & optional option clearing.
    for (idx, (overrides, expected_seed)) in [
        (json!({}), json!(17)),
        (json!({"seed":{"kind":"clear"}}), Value::Null),
        (json!({"seed":{"kind":"value","value":"0"}}), json!(0)),
    ]
    .iter()
    .enumerate()
    {
        let cid = format!("seed-{idx}");
        create_sparse(&mut store, &cid, overrides);
        let output = resolve_json(&mut store, &cid);
        assert_eq!(output["resolution"]["generation_options"]["seed"], *expected_seed);
    }
    create_sparse(
        &mut store,
        "clear-penalties",
        &json!({"presencePenalty":null,"stopSequences":""}),
    );
    let cleared = resolve_json(&mut store, "clear-penalties");
    assert!(cleared["resolution"]["generation_options"]["presence_penalty"].is_null());
    assert!(cleared["resolution"]["generation_options"]["stop"].is_null());

    // 3. Stale configuration & profile revision pins.
    create_sparse(
        &mut store,
        "pins",
        &json!({"maxOutputTokens":1024,"maxContextLength":8192}),
    );
    let mut stale_cfg = current_input(&store, "pins");
    if let ResolveEffectiveModelParameters::CurrentConversation {
        expected_configuration_revision,
        ..
    } = &mut stale_cfg
    {
        *expected_configuration_revision += 1;
    }
    assert!(parameters::resolve(&mut store, &stale_cfg)
        .unwrap_err()
        .contains("configuration_changed"));
    let mut stale_prof = current_input(&store, "pins");
    if let ResolveEffectiveModelParameters::CurrentConversation {
        expected_model_profile,
        ..
    } = &mut stale_prof
    {
        expected_model_profile.admitted_revision += 1;
    }
    assert!(parameters::resolve(&mut store, &stale_prof)
        .unwrap_err()
        .contains("profile_changed"));
    let valid_pins = resolve_json(&mut store, "pins");
    assert_eq!(valid_pins["resolution"]["max_output_tokens"], 1024);
    assert_eq!(valid_pins["resolution"]["context_window_tokens"], 8192);

    // Device runtimes use the live local baseline for candidate previews.
    for runtime in ["mlx", "llama.cpp", "vllm"] {
        let mut local = profile.clone();
        local.id = format!("local-{runtime}");
        local.runtime = runtime.into();
        local.selected = false;
        store.upsert_model_profile(&local).unwrap();
        let stored = store.model_profile(&local.id).unwrap().unwrap();
        let preview = ResolveEffectiveModelParameters::CandidatePreview {
            model_profile: HostModelProfileReference {
                profile_id: stored.id,
                admitted_revision: stored.revision,
            },
            conversation: None,
        };
        let result = parameters::resolve(&mut store, &preview).unwrap();
        assert!(matches!(
            result.resolution,
            EffectiveModelResolution::Unavailable {
                reason: EffectiveModelUnavailableReason::LocalBaselineUnavailable
            }
        ));
        assert!(result.verified_model_profile.is_some() && result.conversation.is_none());
        let command = ApplicationCommand::ResolveEffectiveModelParameters { input: preview };
        assert!(command.is_read_only_query());
        assert!(
            serde_json::from_value::<ApplicationCommand>(serde_json::to_value(command).unwrap())
                .unwrap()
                .is_read_only_query()
        );
    }

    let mut other = profile.clone();
    other.id = "other".into();
    other.selected = false;
    store.upsert_model_profile(&other).unwrap();
    let stored_other = store.model_profile("other").unwrap().unwrap();
    let mut wrong_pin = current_input(&store, "pins");
    if let ResolveEffectiveModelParameters::CurrentConversation {
        expected_model_profile,
        ..
    } = &mut wrong_pin
    {
        *expected_model_profile = HostModelProfileReference {
            profile_id: stored_other.id.clone(),
            admitted_revision: stored_other.revision,
        };
    }
    assert!(parameters::resolve(&mut store, &wrong_pin)
        .unwrap_err()
        .contains("selection_changed"));
    let other_preview = ResolveEffectiveModelParameters::CandidatePreview {
        model_profile: HostModelProfileReference {
            profile_id: stored_other.id,
            admitted_revision: stored_other.revision,
        },
        conversation: None,
    };
    assert!(matches!(
        parameters::resolve(&mut store, &other_preview)
            .unwrap()
            .resolution,
        EffectiveModelResolution::Available { .. }
    ));

    let mut agent_host = profile.clone();
    agent_host.id = "host-candidate".into();
    agent_host.runtime = "agent-host".into();
    agent_host.selected = false;
    store.upsert_model_profile(&agent_host).unwrap();
    let stored_host = store.model_profile("host-candidate").unwrap().unwrap();
    let host_ref = HostModelProfileReference {
        profile_id: stored_host.id,
        admitted_revision: stored_host.revision,
    };
    let host_current = ResolveEffectiveModelParameters::CurrentConversation {
        conversation_id: "pins".into(),
        expected_configuration_revision: store
            .load_conversation("pins")
            .unwrap()
            .unwrap()
            .configuration_revision,
        expected_model_profile: host_ref.clone(),
    };
    assert!(parameters::resolve(&mut store, &host_current)
        .unwrap_err()
        .contains("selection_changed"));
    let host_out = parameters::resolve(
        &mut store,
        &ResolveEffectiveModelParameters::CandidatePreview {
            model_profile: host_ref,
            conversation: None,
        },
    )
    .unwrap();
    assert!(host_out.verified_model_profile.is_none() && host_out.model_key.is_none());
    assert!(matches!(
        host_out.resolution,
        EffectiveModelResolution::Unavailable {
            reason: EffectiveModelUnavailableReason::SourceHostRequired
        }
    ));

    let connection = rusqlite::Connection::open(&path).unwrap();
    let counts = || {
        ["command_receipts", "runs", "conversations"].map(|table| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        })
    };
    let before_counts = counts();
    let pin = store.model_profile("model").unwrap().unwrap();
    let dispatch_out = execute_application_command(
        &mut store,
        ApplicationCommand::ResolveEffectiveModelParameters {
            input: ResolveEffectiveModelParameters::CandidatePreview {
                model_profile: HostModelProfileReference {
                    profile_id: pin.id,
                    admitted_revision: pin.revision,
                },
                conversation: None,
            },
        },
    )
    .unwrap();
    assert_eq!(dispatch_out["resolution"]["status"], "available");
    assert_eq!(counts(), before_counts);

    // 5. Execution-host binding does not claim local profile authority.
    create_sparse(&mut store, "remote-bound", &json!({}));
    let rev = store
        .load_conversation("remote-bound")
        .unwrap()
        .unwrap()
        .configuration_revision;
    assert!(store
        .patch_conversation_configuration(&PatchConversationConfigurationInput {
            conversation_id: "remote-bound".into(),
            expected_revision: rev,
            patch_json: json!({
                "modelSelectionID": null,
                "executionHostRawValue": "remote",
                "executionHostBinding": {"baseURL": "http://127.0.0.1:5001"}
            })
            .to_string()
            .parse()
            .unwrap(),
            updated_at_ms: 5,
        })
        .unwrap()
        .is_committed());
    let remote_out = resolve_json(&mut store, "remote-bound");
    assert_eq!(remote_out["resolution"]["reason"], "source_host_required");
    assert!(remote_out["verified_model_profile"].is_null() && remote_out["model_key"].is_null());

    // 6. Reasoning & wireProtocol overrides, raw profile null rejection, and nonobject source_json rejection.
    create_sparse(
        &mut store,
        "proto-override",
        &json!({"wireProtocol":"responses","seed":{"kind":"clear"},"presencePenalty":null}),
    );
    let proto_out = resolve_json(&mut store, "proto-override");
    assert_eq!(proto_out["resolution"]["protocol"], "responses");
    assert!(proto_out["resolution"]["generation_options"]
        .as_object()
        .unwrap()
        .is_empty());

    create_sparse(
        &mut store,
        "reasoning-override",
        &json!({"reasoningPreference":"concise","wireProtocol":null,"reasoning":null}),
    );
    let mut reasoning_prof = profile.clone();
    let mut source: Value = serde_json::from_str(&reasoning_prof.source_json).unwrap();
    source["reasoning"] =
        json!({"control":"effort","efforts":["low","medium","high"],"defaultPreference":"detailed"});
    source["executionCapabilities"]["reasoning"] = json!(true);
    reasoning_prof.source_json = source.to_string();
    reasoning_prof.revision = store.model_profile("model").unwrap().unwrap().revision;
    store.upsert_model_profile(&reasoning_prof).unwrap();
    let reasoning_out = resolve_json(&mut store, "reasoning-override");
    assert_eq!(reasoning_out["resolution"]["protocol"], "chat_completions");
    assert_eq!(
        reasoning_out["resolution"]["reasoning_selection"],
        json!({"kind":"effort","value":"low"})
    );

    let mut null_temp = reasoning_prof.clone();
    null_temp.revision = store.model_profile("model").unwrap().unwrap().revision;
    null_temp.parameters_json =
        json!({"temperature":null,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000})
            .to_string();
    store.upsert_model_profile(&null_temp).unwrap();
    let null_input = current_input(&store, "pins");
    assert!(parameters::resolve(&mut store, &null_input)
        .unwrap_err()
        .contains("temperature"));

    create_sparse(
        &mut store,
        "nonobj-reasoning",
        &json!({"reasoning":{"control":"automatic"}}),
    );
    for bad_source in [json!([]), Value::Null, json!("invalid-source"), json!(42)] {
        let mut corrupt = profile.clone();
        corrupt.revision = store.model_profile("model").unwrap().unwrap().revision;
        corrupt.source_json = bad_source.to_string();
        store.upsert_model_profile(&corrupt).unwrap();
        for cid in ["proto-override", "nonobj-reasoning"] {
            let inp = current_input(&store, cid);
            assert!(parameters::resolve(&mut store, &inp)
                .unwrap_err()
                .contains("model source must be an object"));
            assert_eq!(
                store.model_profile("model").unwrap().unwrap().source_json,
                bad_source.to_string()
            );
        }
    }
}
