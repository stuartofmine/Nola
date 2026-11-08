// SPDX-License-Identifier: Apache-2.0
//! Actual Store reads: sparse resolution, exact pins, and no persistence side effects.
use nola_application::{
    effective_model_parameters::{self as parameters, *},
    ApplicationCommand,
};
use nola_host_protocol::HostModelProfileReference;
use nola_store::NolaStore;
use nola_store_port::model::StoredModelProfile;
use serde_json::{json, Value};

fn setup(overrides: Value) -> (NolaStore, StoredModelProfile) {
    let mut store = NolaStore::open(":memory:").unwrap();
    let profile=StoredModelProfile {
        id:"model".into(),display_name:"Private configured name".into(),provider:"fixture".into(),model_name:"model-name".into(),runtime:"remote".into(),
        source_json:json!({"baseURL":"https://example.invalid/private/v1","credentialRef":"never-in-query","wireProtocol":"chat_completions","capabilities":["chat"],"reasoning":null,"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}).to_string(),
        parameters_json:json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000,"seed":17,"presencePenalty":0.4}).to_string(),
        selected:true,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
    };
    store.upsert_model_profile(&profile).unwrap();
    // Inherit an exact Host preference. Explicit model selection deliberately
    // initializes a full conversation snapshot and cannot create this sparse
    // fixture; inheritance leaves the supplied override intent unchanged.
    store
        .put_product_setting(
            nola_application::model_selection::DEFAULT_CHAT_MODEL_KEY,
            &json!({"version":1,"selection":{"kind":"profile","profile_id":"model"}}).to_string(),
            1,
        )
        .unwrap();
    store.create_conversation(&nola_store_port::conversation_profile::CreateConversationInput {
        id:"conversation".into(),title:"Sparse".into(),revision:1,
        settings_json:Some(json!({"modelOverrides":{"configured:model":overrides},"clientModelPresentation":{"configured:model":{"responseStyle":"concise"}}}).to_string()),created_at_ms:1,updated_at_ms:1,
    }).unwrap();
    (store, profile)
}
fn input(store: &NolaStore) -> ResolveEffectiveModelParameters {
    ResolveEffectiveModelParameters::CurrentConversation {
        conversation_id: "conversation".into(),
        expected_configuration_revision: store
            .load_conversation("conversation")
            .unwrap()
            .unwrap()
            .configuration_revision,
        expected_model_profile: HostModelProfileReference {
            profile_id: "model".into(),
            admitted_revision: store.model_profile("model").unwrap().unwrap().revision,
        },
    }
}
fn resolved(store: &mut NolaStore) -> Value {
    let input = input(store);
    serde_json::to_value(parameters::resolve(store, &input).unwrap()).unwrap()
}
#[test]
fn sparse_and_nullable_scalars_inherit_without_writes_or_private_route_material() {
    for overrides in [
        json!({"temperature":0.0}),
        json!({"temperature":0.0,"topP":null,"maxOutputTokens":null,"wireProtocol":null,"reasoning":null,"reasoningPreference":null}),
    ] {
        let (mut store, _) = setup(overrides.clone());
        let before = store.load_conversation("conversation").unwrap().unwrap();
        let profile = store.model_profile("model").unwrap();
        let output = resolved(&mut store);
        assert_eq!(output["resolution"]["status"], "available");
        assert_eq!(output["resolution"]["temperature"], 0.0);
        assert_eq!(output["resolution"]["top_p"], 0.9);
        assert_eq!(output["resolution"]["max_output_tokens"], 8192);
        assert_eq!(output["sparse_overrides"], overrides);
        assert_eq!(
            output["client_model_presentation"]["responseStyle"],
            "concise"
        );
        assert_eq!(
            store.load_conversation("conversation").unwrap().unwrap(),
            before
        );
        assert_eq!(store.model_profile("model").unwrap(), profile);
        let encoded = output.to_string();
        for forbidden in [
            "never-in-query",
            "example.invalid",
            "source_json",
            "credentialRef",
        ] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }
}
#[test]
fn seed_clear_value_inherit_and_optional_option_clear_keep_current_typed_semantics() {
    for (overrides, seed) in [
        (json!({}), json!(17)),
        (json!({"seed":{"kind":"clear"}}), Value::Null),
        (json!({"seed":{"kind":"value","value":"0"}}), json!(0)),
    ] {
        let (mut store, _) = setup(overrides);
        let output = resolved(&mut store);
        assert_eq!(output["resolution"]["generation_options"]["seed"], seed);
    }
    let (mut store, _) = setup(json!({"presencePenalty":null,"stopSequences":""}));
    let output = resolved(&mut store);
    assert!(output["resolution"]["generation_options"]["presence_penalty"].is_null());
    assert!(output["resolution"]["generation_options"]["stop"].is_null());
}
#[test]
fn stale_profile_configuration_and_selected_identity_are_rejected() {
    let (mut store, _) = setup(json!({"maxOutputTokens":1024,"maxContextLength":8192}));
    let mut stale = input(&store);
    if let ResolveEffectiveModelParameters::CurrentConversation {
        expected_configuration_revision,
        ..
    } = &mut stale
    {
        *expected_configuration_revision += 1;
    }
    assert!(parameters::resolve(&mut store, &stale)
        .unwrap_err()
        .contains("configuration_changed"));
    let mut stale = input(&store);
    if let ResolveEffectiveModelParameters::CurrentConversation {
        expected_model_profile,
        ..
    } = &mut stale
    {
        expected_model_profile.admitted_revision += 1;
    }
    assert!(parameters::resolve(&mut store, &stale)
        .unwrap_err()
        .contains("profile_changed"));
    let result = resolved(&mut store);
    assert_eq!(result["resolution"]["max_output_tokens"], 1024);
    assert_eq!(result["resolution"]["context_window_tokens"], 8192);
}
#[test]
fn candidate_preview_is_read_only_and_local_has_no_invented_runtime_defaults() {
    let (mut store, mut profile) = setup(json!({"temperature":0.0}));
    profile.id = "local-candidate".into();
    profile.runtime = "mlx".into();
    profile.selected = false;
    store.upsert_model_profile(&profile).unwrap();
    let stored = store.model_profile(&profile.id).unwrap().unwrap();
    let input = ResolveEffectiveModelParameters::CandidatePreview {
        model_profile: HostModelProfileReference {
            profile_id: stored.id,
            admitted_revision: stored.revision,
        },
        conversation: None,
    };
    let result = parameters::resolve(&mut store, &input).unwrap();
    assert!(matches!(
        result.resolution,
        EffectiveModelResolution::Unavailable {
            reason: EffectiveModelUnavailableReason::LocalBaselineUnavailable
        }
    ));
    assert!(result.verified_model_profile.is_some());
    assert!(result.conversation.is_none());
    let query = ApplicationCommand::ResolveEffectiveModelParameters { input };
    assert!(query.is_read_only_query());
    let encoded = serde_json::to_value(query).unwrap();
    assert!(serde_json::from_value::<ApplicationCommand>(encoded)
        .unwrap()
        .is_read_only_query());
}
#[test]
fn raw_profile_null_is_rejected_instead_of_becoming_a_new_default_contract() {
    let (mut store, mut profile) = setup(json!({}));
    profile.revision = store.model_profile("model").unwrap().unwrap().revision;
    profile.parameters_json =
        json!({"temperature":null,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000})
            .to_string();
    store.upsert_model_profile(&profile).unwrap();
    let input = input(&store);
    assert!(parameters::resolve(&mut store, &input)
        .unwrap_err()
        .contains("temperature"));
}

#[test]
fn local_conversation_with_execution_host_binding_does_not_claim_local_profile_authority() {
    let (mut store, _) = setup(json!({}));
    let revision = store
        .load_conversation("conversation")
        .unwrap()
        .unwrap()
        .configuration_revision;
    let outcome=store.patch_conversation_configuration(&nola_store_port::conversation_profile::PatchConversationConfigurationInput {
        conversation_id:"conversation".into(),expected_revision:revision,
        patch_json:json!({"modelSelectionID":null,"executionHostRawValue":"remote","executionHostBinding":{"baseURL":"http://127.0.0.1:5001"}}).to_string().parse().unwrap(),updated_at_ms:5,
    }).unwrap();
    assert!(outcome.is_committed());
    let output = resolved(&mut store);
    assert_eq!(output["resolution"]["reason"], "source_host_required");
    assert!(output["verified_model_profile"].is_null());
    assert!(output["model_key"].is_null());
}
#[test]
fn current_model_pin_rejects_another_enabled_candidate_but_preview_can_inspect_it() {
    let (mut store, mut profile) = setup(json!({}));
    profile.id = "other".into();
    profile.selected = false;
    store.upsert_model_profile(&profile).unwrap();
    let profile = store.model_profile("other").unwrap().unwrap();
    let mut current = input(&store);
    if let ResolveEffectiveModelParameters::CurrentConversation {
        expected_model_profile,
        ..
    } = &mut current
    {
        *expected_model_profile = HostModelProfileReference {
            profile_id: profile.id.clone(),
            admitted_revision: profile.revision,
        };
    }
    assert!(parameters::resolve(&mut store, &current)
        .unwrap_err()
        .contains("selection_changed"));
    let candidate = ResolveEffectiveModelParameters::CandidatePreview {
        model_profile: HostModelProfileReference {
            profile_id: profile.id,
            admitted_revision: profile.revision,
        },
        conversation: None,
    };
    assert!(matches!(
        parameters::resolve(&mut store, &candidate)
            .unwrap()
            .resolution,
        EffectiveModelResolution::Available { .. }
    ));
}

#[test]
fn current_reasoning_and_protocol_override_use_one_contract_before_resolution() {
    let (mut store, mut profile) =
        setup(json!({"reasoningPreference":"concise","wireProtocol":null,"reasoning":null}));
    let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
    source["reasoning"] =
        json!({"control":"effort","efforts":["low","medium","high"],"defaultPreference":"detailed"});
    source["executionCapabilities"]["reasoning"] = json!(true);
    profile.source_json = source.to_string();
    profile.revision = store.model_profile("model").unwrap().unwrap().revision;
    store.upsert_model_profile(&profile).unwrap();
    let output = resolved(&mut store);
    assert_eq!(output["resolution"]["protocol"], "chat_completions");
    assert_eq!(
        output["resolution"]["reasoning_selection"],
        json!({"kind":"effort","value":"low"})
    );
    let (mut store, _) =
        setup(json!({"wireProtocol":"responses","seed":{"kind":"clear"},"presencePenalty":null}));
    let output = resolved(&mut store);
    assert_eq!(output["resolution"]["protocol"], "responses");
    assert!(output["resolution"]["generation_options"]
        .as_object()
        .unwrap()
        .is_empty());
}

#[test]
fn application_dispatch_is_read_only_at_the_actual_store_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("parameters.db");
    let mut store = NolaStore::open(&path).unwrap();
    let (_fixture, profile) = setup(json!({}));
    store.upsert_model_profile(&profile).unwrap();
    let pin = store.model_profile("model").unwrap().unwrap();
    let query = ApplicationCommand::ResolveEffectiveModelParameters {
        input: ResolveEffectiveModelParameters::CandidatePreview {
            model_profile: HostModelProfileReference {
                profile_id: pin.id,
                admitted_revision: pin.revision,
            },
            conversation: None,
        },
    };
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
    let before = counts();
    let output = nola_application::execute_application_command(&mut store, query).unwrap();
    assert_eq!(output["resolution"]["status"], "available");
    assert_eq!(counts(), before);
}

#[test]
fn agent_host_candidate_does_not_invent_a_local_model_key_or_verify_source_profile() {
    let (mut store, mut profile) = setup(json!({}));
    profile.id = "host-candidate".into();
    profile.runtime = "agent-host".into();
    profile.selected = false;
    store.upsert_model_profile(&profile).unwrap();
    let profile = store.model_profile(&profile.id).unwrap().unwrap();
    let query = ResolveEffectiveModelParameters::CandidatePreview {
        model_profile: HostModelProfileReference {
            profile_id: profile.id,
            admitted_revision: profile.revision,
        },
        conversation: None,
    };
    let current = ResolveEffectiveModelParameters::CurrentConversation {
        conversation_id: "conversation".into(),
        expected_configuration_revision: store
            .load_conversation("conversation")
            .unwrap()
            .unwrap()
            .configuration_revision,
        expected_model_profile: match &query {
            ResolveEffectiveModelParameters::CandidatePreview { model_profile, .. } => {
                model_profile.clone()
            }
            _ => unreachable!(),
        },
    };
    assert!(parameters::resolve(&mut store, &current)
        .unwrap_err()
        .contains("selection_changed"));
    let output = parameters::resolve(&mut store, &query).unwrap();
    assert!(output.verified_model_profile.is_none());
    assert!(output.model_key.is_none());
    assert!(matches!(
        output.resolution,
        EffectiveModelResolution::Unavailable {
            reason: EffectiveModelUnavailableReason::SourceHostRequired
        }
    ));
}

#[test]
fn nonobject_profile_source_is_rejected_without_panicking_or_rebuilding_it() {
    for source in [json!([]), Value::Null, json!("invalid-source"), json!(42)] {
        for overrides in [
            json!({"wireProtocol":"responses"}),
            json!({"reasoning":{"control":"automatic"}}),
        ] {
            let (mut store, mut profile) = setup(overrides);
            profile.source_json = source.to_string();
            store.upsert_model_profile(&profile).unwrap();
            let input = input(&store);
            assert!(parameters::resolve(&mut store, &input)
                .unwrap_err()
                .contains("model source must be an object"));
            assert_eq!(
                store.model_profile("model").unwrap().unwrap().source_json,
                source.to_string()
            );
        }
    }
}
