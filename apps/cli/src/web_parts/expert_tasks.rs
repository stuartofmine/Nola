// SPDX-License-Identifier: Apache-2.0
// Expert configuration is prepared by the owning Host; Task execution stays in
// the existing Task supervisor, lease and child worker.

pub(crate) struct ExpertTaskHostConfiguration {
    pub configuration: nola_core::agent::AgentCoreConfig,
    pub invocation: Option<nola_host_protocol::delegation::DelegatedInvocationSnapshot>,
    pub project_root: String,
}

pub(crate) fn capture_expert_peer_material(
    request: &nola_core::agent::host::HostRequestEnvelope,
    input: &nola_application::expert_peer::PeerExchangeInput,
    authority: &nola_application::expert_template_capture::HostExpertTemplateAuthority,
) -> Result<nola_application::expert_peer::PeerMaterial, ExpertHostFailure> {
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let prior: Option<nola_application::expert_peer::PeerMaterial> = app
        .execute_expert_host_operation(
            &uuid::Uuid::new_v4().to_string(),
            HostExpertTemplateOperation::LoadPeerForEffect {
                request: Box::new(request.clone()),
                authority: authority.clone(),
            },
        )?;
    if let Some(prior) = prior {
        return Ok(prior);
    }
    let preview = app.execute_expert_host_operation(
        &uuid::Uuid::new_v4().to_string(),
        HostExpertTemplateOperation::PreviewPeer {
            input: input.clone(),
            authority: authority.clone(),
        },
    )?;
    let context = if crate::desktop_host_control::is_active() {
        let context=crate::desktop_host_control::capture_expert_templates(request)?;
        if context.authority()!=*authority {return Err("native_expert_template_authority_changed".into());}
        context
    } else {
        // An unavailable native provider is not permission to borrow the Web
        // principal, default template or instruction producer.
        if *authority!=expert_host_authority() {return Err("native_expert_template_provider_unavailable".into());}
        expert_capture_from_preview(preview,authority.clone())?
    };
    app.execute_expert_host_operation(
        &uuid::Uuid::new_v4().to_string(),
        HostExpertTemplateOperation::AdmitPeer {
            input: input.clone(),
            context,
        },
    )
}

fn apply_desktop_host_tool_environment(
    config: &mut Config,
    project_root: &std::path::Path,
    host_tools: &crate::host_invocation_request::HostToolRequest,
) {
    config.skill_dirs = host_tools.skill_dirs.clone();
    let mut allowed_dirs = vec![project_root.to_path_buf()];
    for directory in &host_tools.allowed_dirs {
        if !allowed_dirs.contains(directory) {
            allowed_dirs.push(directory.clone());
        }
    }
    config.allowed_dirs = allowed_dirs;
    if config.bash_prefixes.is_empty() && !host_tools.bash_prefixes.is_empty() {
        config.bash_prefixes = host_tools.bash_prefixes.clone();
    }
}

pub(crate) fn expert_task_host_configuration(
    project_root: &std::path::Path,
    host_tool_settings: Option<&crate::host_invocation_request::HostToolRequest>,
    material: &nola_application::expert_peer::PeerMaterial,
) -> Result<ExpertTaskHostConfiguration, String> {
    let desktop_host =
        crate::desktop_host_control::is_active() || std::env::var_os("NOLA_DESKTOP_HOST").is_some();
    expert_task_host_configuration_with_host_mode(
        project_root,
        host_tool_settings,
        desktop_host,
        material,
    )
}

fn expert_task_host_configuration_with_host_mode(
    project_root: &std::path::Path,
    host_tool_settings: Option<&crate::host_invocation_request::HostToolRequest>,
    desktop_host: bool,
    material: &nola_application::expert_peer::PeerMaterial,
) -> Result<ExpertTaskHostConfiguration, String> {
    let store = NolaStore::open_default()?;
    let entry = &material.entry;
    let target = &entry.participant.context_conversation_id;
    let (revision, _) = conversation_configuration_state(target)?
        .ok_or("Expert target configuration is unavailable")?;
    let profile = store
        .model_profile(&entry.model_declaration.profile_id)?
        .ok_or("Expert model profile is unavailable")?;
    validate_reserved_room_model(entry, &profile, revision)?;
    // Fresh application configuration, never the sender's invocation or its
    // one-turn permission grants. Canonical target settings replace the model.
    let mut config = Config::from_args([
        "web".to_owned(),
        "--project".to_owned(),
        project_root.to_string_lossy().into_owned(),
    ])?;
    let settings = load_agent_settings_from_store(&store)?;
    apply_agent_tool_settings(&mut config, &settings);
    if desktop_host {
        if let Some(host_tools) = host_tool_settings {
            apply_desktop_host_tool_environment(&mut config, project_root, host_tools);
        }
    }
    let mut captured = entry.defaults.conversation_configuration();
    if captured
        .get("permissionModeRawValue")
        .is_none_or(Value::is_null)
    {
        captured["permissionModeRawValue"] = json!(settings.permission_profile);
    }
    crate::conversation_invocation::apply_expert_settings(&mut config, &captured, &store)?;
    config.session_id = Some(target.clone());
    config.permission_scope_id = Some(format!("host:task:{}", material.view.exchange_id));
    config.continue_last = false;
    config.save_session = true;
    config.json_output = true;
    config.remote_tool_plan = None;
    config.host_context_components.clear();
    config.host_turn_context = None;
    config.host_attachments.clear();
    config.context_files.clear();
    config.host_submission_id = None;
    config.host_user_message_id = None;
    config.host_assistant_message_id = None;
    config.expected_configuration_revision = Some(revision);
    let skills = web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &settings)?;
    config.skill_dirs = skills.configured_dirs().map(<[PathBuf]>::to_vec);
    let (tools, _) = capture_current_host_tool_plan_tools(&config, target)?;
    config.tool_allowlist = Some(tools.into_iter().map(|tool| tool.tool_id).collect());
    config.tool_allowlist_is_authorization_only = true;
    let raw_mcp_servers = if !settings.mcp_servers.is_empty() {
        crate::resource_access::mcp_runtime_resources(&settings.mcp_servers)
    } else if desktop_host {
        host_tool_settings
            .map(|host_tools| host_tools.mcp_server_registry.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    bind_and_catalog_web_mcp(&mut config, &raw_mcp_servers, target)?;
    let mut own = config.host_invocation_request()?;
    if !own.tool_settings.mcp_server_registry.is_empty() {
        if let Some(allowlist) = own.tool_settings.tool_allowlist.as_mut() {
            if own
                .tool_settings
                .mcp_server_registry
                .iter()
                .any(|server| server["enabled"] != false)
                && !allowlist
                    .iter()
                    .any(|id| id == crate::mcp_connection_authority::CONNECT_CAPABILITY)
            {
                allowlist.push(crate::mcp_connection_authority::CONNECT_CAPABILITY.to_string());
            }
            if let Some(catalog) = own.tool_settings.mcp_capability_catalog.as_ref() {
                for schema in crate::agent_host_driver::remote_mcp_tool_schemas(catalog) {
                    if let Some(name) = schema.pointer("/function/name").and_then(Value::as_str) {
                        if !allowlist.iter().any(|id| id == name) {
                            allowlist.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    let configuration = crate::core_agent_runtime::capture_task_target_configuration(&own)?;
    Ok(ExpertTaskHostConfiguration {
        configuration,
        invocation: None,
        project_root: own
            .tool_settings
            .project_root
            .to_string_lossy()
            .into_owned(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MobileExpertManifestRead {
    expected_host_id: String,
    client_id: String,
    conversation_id: String,
    message_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MobileExpertParentPrepare {
    expected_host_id: String,
    client_id: String,
    conversation_id: String,
    title: String,
}

fn mobile_expert_parent_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let result = (|| -> Result<Value, String> {
        let input: MobileExpertParentPrepare =
            serde_json::from_value(body).map_err(|_| "Invalid expert parent intent")?;
        validate_edited_resend_web_owner(&input.expected_host_id, &input.client_id)?;
        uuid::Uuid::parse_str(&input.conversation_id)
            .map_err(|_| "A stable conversation UUID is required")?;
        if input.title.len() > 2000 || input.title.chars().any(char::is_control) {
            return Err("Invalid conversation title".into());
        }
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let mut parent: Option<Value> = app.execute(
            nola_application::ApplicationCommand::LoadConversationMetadata {
                conversation_id: input.conversation_id.clone(),
            },
        )?;
        if parent.is_none() {
            let now = now_unix_millis();
            let _: Value =
                app.execute(nola_application::ApplicationCommand::CreateConversation {
                    input: nola_application::CreateConversation {
                        id: input.conversation_id.clone(),
                        title: input.title,
                        settings_json: None,
                        created_at_ms: now,
                        updated_at_ms: now,
                    },
                })?;
            parent = app.execute(
                nola_application::ApplicationCommand::LoadConversationMetadata {
                    conversation_id: input.conversation_id.clone(),
                },
            )?;
        }
        let room: Value = app.execute(nola_application::ApplicationCommand::AgentRoom {
            command: nola_application::agent_room::AgentRoomCommand::EnsureForConversation {
                conversation_id: input.conversation_id.clone(),
                at_ms: now_unix_millis(),
            },
        })?;
        let epoch: String = app.execute(nola_application::ApplicationCommand::ReadUiAuthority)?;
        Ok(
            json!({"host_id":host_identity(),"client_id":input.client_id,"source_store_epoch":epoch,"parent":parent,"room":room}),
        )
    })();
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(_) => mobile_encrypted_json_response(
            StatusCode(409),
            json!({"code":"expert_parent_unconfirmed"}),
            &token,
        ),
    }
}

fn export_expert_parent_admission(conversation: &str, message: &str) -> Result<Value, String> {
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    // UI authority and journal source use the same durable Store epoch; this
    // read is never an app version or a new per-request identity.
    let epoch: String = app.execute(nola_application::ApplicationCommand::ReadUiAuthority)?;
    app.execute_expert_host_operation(&uuid::Uuid::new_v4().to_string(),
        HostExpertTemplateOperation::ExportRemoteParentAdmission {
            conversation_id: conversation.into(),message_id: message.into(),
            source_store_epoch: epoch,authority:expert_host_authority(),
        },
    ).map_err(|error|error.to_string())
}

fn expert_remote_child_sources(
    manifest: &nola_domain::expert_message::ExpertMessageManifest,
) -> Result<Vec<Value>, String> {
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let client = format!("room:{}", manifest.input.room_id);
    manifest.entries.iter().map(|entry| {
        let receipt:Option<nola_domain::remote_submission::SourceSubmissionReceipt>=app.execute(
            nola_application::ApplicationCommand::ObservedSourceSubmissionReceipt {
                source_host_id:host_identity().into(),client_id:client.clone(),
                conversation_id:entry.participant.context_conversation_id.clone(),run_id:entry.child_submission_id.clone(),
                assistant_message_id:entry.child_assistant_message_id.clone(),
            })?;
        Ok(json!({"conversation_id":entry.participant.context_conversation_id,"run_id":entry.child_submission_id,
            "user_message_id":entry.child_user_message_id,"assistant_message_id":entry.child_assistant_message_id,
            "client_id":client,"source_receipt":receipt}))
    }).collect()
}

fn mobile_expert_admit_response(
    request: &mut Request,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut definitive_rejection = None;
    let result = (|| -> Result<Value, String> {
        let input: WebExpertMessageAdmit =
            serde_json::from_value(body).map_err(|_| "Invalid expert admission intent")?;
        let parent = input.input.conversation_id.clone();
        let message = input.input.user_message_id.clone();
        let rejection_identity = json!({"host_id":host_identity(),"client_id":input.client_id,
            "conversation_id":parent,"submission_id":input.input.submission_id,"user_message_id":message});
        let mut value = execute_expert_admit(input).map_err(|error| {
            if matches!(&error,ExpertHostFailure::Rejected{code,..} if code=="expert_admission_rejected") {
                // Only the actual stable Core admission command can return
                // this classification. Preview/transport/export failures stay
                // unconfirmed, and the caller must retain its original intent.
                definitive_rejection=Some(rejection_identity);
            }
            error.to_string()
        })?;
        value["parent_admission"] = export_expert_parent_admission(&parent, &message)?;
        let manifest: nola_domain::expert_message::ExpertMessageManifest =
            serde_json::from_value(value["manifest"].clone())
                .map_err(|_| "Expert admission response is invalid")?;
        // Only exact already-admitted child submissions are dispatched. The
        // shared guards reconcile duplicate requests and canonical Run seals.
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        for entry in &manifest.entries {
            let receipt: Option<nola_application::agent_room::RoomSubmission> =
                app.execute(nola_application::ApplicationCommand::AgentRoom {
                    command: nola_application::agent_room::AgentRoomCommand::Submission {
                        room_id: manifest.input.room_id.clone(),
                        submission_id: entry.child_submission_id.clone(),
                    },
                })?;
            start_room_dispatch(
                config.clone(),
                receipt.ok_or("Expert child receipt missing")?,
            );
        }
        value["children"] = serde_json::to_value(expert_remote_child_sources(&manifest)?)
            .map_err(|e| e.to_string())?;
        Ok(value)
    })();
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(_) if definitive_rejection.is_some() => {
            let mut response = definitive_rejection.unwrap();
            response["code"] = json!("expert_admission_rejected");
            response["definitive_rejection"] = json!(true);
            response["error"]=json!("The original expert input was rejected. Refresh the expert selection and resend your draft.");
            mobile_encrypted_json_response(StatusCode(409), response, &token)
        }
        Err(_) => mobile_encrypted_json_response(
            StatusCode(409),
            json!({"code":"expert_admission_unconfirmed",
            "definitive_rejection":false,"outcome_pending":true,
            "error":"Refresh the original expert submission before retrying"}),
            &token,
        ),
    }
}

fn mobile_expert_manifest_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let result = (|| -> Result<Value, String> {
        let input: MobileExpertManifestRead =
            serde_json::from_value(body).map_err(|_| "Invalid expert receipt read")?;
        validate_edited_resend_web_owner(&input.expected_host_id, &input.client_id)?;
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let manifest: Option<nola_domain::expert_message::ExpertMessageManifest> = app
            .execute_expert_host_operation(
                &uuid::Uuid::new_v4().to_string(),
                HostExpertTemplateOperation::LoadManifest {
                    conversation_id: input.conversation_id.clone(),
                    message_id: input.message_id.clone(),
                    authority: expert_host_authority(),
                },
            )
            .map_err(|error| error.to_string())?;
        let admission = if manifest.is_some() {
            export_expert_parent_admission(&input.conversation_id, &input.message_id)?
        } else {
            Value::Null
        };
        let children = manifest
            .as_ref()
            .map(expert_remote_child_sources)
            .transpose()?
            .unwrap_or_default();
        Ok(
            json!({"host_id":host_identity(),"client_id":input.client_id,"manifest":manifest,"parent_admission":admission,"children":children}),
        )
    })();
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(_) => mobile_encrypted_json_response(
            StatusCode(409),
            json!({"code":"expert_receipt_unavailable"}),
            &token,
        ),
    }
}

#[cfg(test)]
mod expert_tasks_tests {
    use super::*;

    struct EnvironmentGuard {
        previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
        conversation: Option<String>,
        _root: tempfile::TempDir,
    }

    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            if let Some(conversation) = &self.conversation {
                let _ = crate::core_agent_runtime::invalidate_core_agent_session(conversation);
            }
            for (key, value) in &self.previous {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn expert_task_host_configuration_inherits_desktop_skill_and_allowed_dirs_without_clobbering_file_refs() {
        std::thread::Builder::new()
            .name("expert-task-host-config-test".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let _lock = crate::store::test_env_lock();
                let root = tempfile::tempdir().unwrap();
                let canonical_root = std::fs::canonicalize(root.path()).unwrap();
                let previous = ["HOME", "NOLA_HOME", "NOLA_DB_PATH"]
                    .into_iter()
                    .map(|key| (key, std::env::var_os(key)))
                    .collect();
                let database = canonical_root.join("nola.sqlite");
                std::env::set_var("HOME", &canonical_root);
                std::env::set_var("NOLA_HOME", &canonical_root);
                std::env::set_var("NOLA_DB_PATH", &database);
                let mut guard = EnvironmentGuard {
                    previous,
                    conversation: None,
                    _root: root,
                };

                let project_root = canonical_root.join("workspace");
                let extra_authorized_dir = canonical_root.join("authorized-extra");
                let bundle_skill_dir = canonical_root.join("NolaBuiltInSkills.bundle/create-visual-art");
                std::fs::create_dir_all(&project_root).unwrap();
                std::fs::create_dir_all(&extra_authorized_dir).unwrap();
                std::fs::create_dir_all(&bundle_skill_dir).unwrap();
                let skill_file = bundle_skill_dir.join("SKILL.md");
                std::fs::write(
                    &skill_file,
                    "---\nname: create-visual-art\ndescription: Visual art creation\n---\nBundle visual art instructions.\n",
                )
                .unwrap();
                let file_uri = url::Url::from_file_path(&skill_file).unwrap().to_string();

                let mut store = nola_store::NolaStore::open(&database).unwrap();
                let source_json = json!({
                    "schemaVersion": 1,
                    "endpointID": "ep-1",
                    "endpointName": "Fixture",
                    "baseURL": "https://fixture.invalid/v1",
                    "endpointOrder": 0,
                    "endpointEnabled": true,
                    "modelEnabled": true,
                    "modelOrder": 0,
                    "priority": 100,
                    "capabilities": ["chat"],
                    "executionCapabilities": {
                        "toolCalls": true,
                        "imageInput": false,
                        "reasoning": false,
                        "samplingParameters": true
                    },
                    "wireProtocol": "responses"
                })
                .to_string();
                store
                    .upsert_model_profile(&nola_store_port::model::StoredModelProfile {
                        id: "expert-profile".into(),
                        display_name: "Expert Fixture".into(),
                        provider: "openai-compatible".into(),
                        model_name: "gpt-test".into(),
                        runtime: "remote".into(),
                        source_json,
                        parameters_json: "{}".into(),
                        selected: true,
                        enabled: true,
                        revision: 1,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                        deleted_at_ms: None,
                    })
                    .unwrap();
                let identity =
                    crate::host_credentials::identity("https://fixture.invalid/v1", "responses")
                        .unwrap();
                crate::host_credentials::HostCredentialVault::open(&database)
                    .unwrap()
                    .write("ep-1", &identity, 0, Some("fixture-secret"), || Ok(()))
                    .unwrap();

                use nola_application::{agent_room as rooms, persistent_agent as agents};
                use nola_domain::persistent_agent::{AgentAvailability, AgentDefaults};
                store.begin_write_unit_of_work().unwrap();
                agents::configure(
                    &mut store.product_setting_repository(),
                    agents::ConfigureAgent {
                        id: "leijun".to_string().try_into().unwrap(),
                        expected_directory_revision: None,
                        expected_agent_revision: None,
                        name: "雷军".into(),
                        description: "代码编写".into(),
                        defaults: AgentDefaults::try_from(
                            json!({"modelSelectionID": "configured:expert-profile"}),
                        )
                        .unwrap(),
                        availability: AgentAvailability::Active,
                        updated_at_ms: 1,
                    },
                )
                .unwrap();
                let room = rooms::create(
                    &mut store.product_setting_repository(),
                    "Expert Room".into(),
                    1,
                )
                .unwrap();
                let joined = rooms::join(
                    &mut store,
                    rooms::JoinAgent {
                        room_id: room.room.id().into(),
                        expected_room_revision: room.revision,
                        agent_id: "leijun".to_string().try_into().unwrap(),
                        expected_agent_revision: 1,
                        at_ms: 2,
                    },
                )
                .unwrap();
                store.commit_write_unit_of_work().unwrap();
                let participant = &joined.room.participants()[0];
                let context_conversation_id = participant.context_conversation_id.clone();
                guard.conversation = Some(context_conversation_id.clone());

                let mut app =
                    crate::application_port::CliApplicationPort::persistent(&database).unwrap();
                let inputs = crate::builtin_skills::catalog(&[])
                    .unwrap()
                    .into_iter()
                    .map(|item| {
                        let id = item["id"].as_str().unwrap().to_owned();
                        let content = item["content"].as_str().unwrap().to_owned();
                        let instructions_ref = if id == "create-visual-art" {
                            file_uri.clone()
                        } else {
                            format!("file:///Applications/Nola.app/Contents/Resources/NolaBuiltInSkills.bundle/{id}/SKILL.md")
                        };
                        nola_application::SkillInput {
                            id: id.clone(),
                            source: nola_application::SkillSource::BuiltIn,
                            version: format!("ver-{id}"),
                            display_name: id.clone(),
                            summary: item["description"].as_str().unwrap().to_owned(),
                            category: item["category_id"].as_str().unwrap().to_owned(),
                            instructions_ref,
                            instructions_text: content,
                            required_tools: Vec::new(),
                            default_mode: Some(nola_core::agent::capability::SkillExecutionMode::Inline),
                            enabled: id == "create-visual-art",
                            created_at_ms: 10,
                            updated_at_ms: 10,
                        }
                    })
                    .collect::<Vec<_>>();
                let _: Value = app
                    .execute(nola_application::ApplicationCommand::Skill {
                        command: nola_application::SkillCommand::SynchronizeBuiltIns { inputs },
                    })
                    .unwrap();
                drop(app);
                drop(store);

                assert!(
                    !canonical_root
                        .join(".nola/cli/skills/built-in/create-visual-art")
                        .exists(),
                    "managed CLI directory must remain absent to prove bundle skill_dirs is used"
                );

                let exchange_id = uuid::Uuid::new_v4().to_string();
                let material: nola_application::expert_peer::PeerMaterial =
                    serde_json::from_value(json!({
                        "view": {
                            "version": 1,
                            "exchange_id": exchange_id,
                            "room_id": joined.room.id(),
                            "parent_conversation_id": "main-conv",
                            "source": {
                                "kind": "main_agent",
                                "context_conversation_id": "main-conv"
                            },
                            "target": {
                                "participant_id": participant.id,
                                "agent_id": participant.agent_id,
                                "agent_revision": participant.agent_revision,
                                "context_conversation_id": context_conversation_id
                            },
                            "source_run_id": "main-run",
                            "source_call_id": "main-call",
                            "source_display_name": "Nola",
                            "target_display_name": "雷军",
                            "kind": "request",
                            "text": "你好",
                            "in_reply_to": null,
                            "root_exchange_id": exchange_id,
                            "hop": 1,
                            "created_at_ms": 100,
                            "clarifications": []
                        },
                        "entry": {
                            "participant": {
                                "participant_id": participant.id,
                                "agent_id": participant.agent_id,
                                "agent_revision": participant.agent_revision,
                                "context_conversation_id": context_conversation_id
                            },
                            "display_name": "雷军",
                            "description": "代码编写",
                            "defaults": {
                                "modelSelectionID": "configured:expert-profile"
                            },
                            "model_declaration": {
                                "profile_id": "expert-profile",
                                "profile_revision": 1,
                                "provider": "openai-compatible",
                                "model_name": "gpt-test",
                                "runtime": "remote",
                                "parameters": {}
                            },
                            "context_configuration_revision": 1,
                            "child_submission_id": exchange_id,
                            "child_user_message_id": format!("{exchange_id}:user"),
                            "child_assistant_message_id": format!("{exchange_id}:assistant"),
                            "expected_prompt_definition": null,
                            "model_witness": null,
                            "instruction_witness": null
                        },
                        "prompt_definitions": []
                    }))
                    .unwrap();

                let mut host_tools =
                    crate::host_invocation_request::HostInvocationRequest::for_test()
                        .unwrap()
                        .tool_settings;
                host_tools.project_root = project_root.clone();
                host_tools.allowed_dirs = vec![
                    project_root.clone(),
                    extra_authorized_dir.clone(),
                    bundle_skill_dir.clone(),
                ];
                host_tools.skill_dirs = Some(vec![bundle_skill_dir.clone()]);

                let configured = expert_task_host_configuration_with_host_mode(
                    &project_root,
                    Some(&host_tools),
                    true,
                    &material,
                )
                .unwrap();

                let store = nola_store::NolaStore::open(&database).unwrap();
                let stored_skill = store.skill("create-visual-art").unwrap().unwrap();
                assert_eq!(stored_skill.instructions_ref, file_uri);

                let restored = crate::core_agent_runtime::restore_task_target_configuration(
                    &configured.configuration,
                    &context_conversation_id,
                )
                .unwrap();
                assert_eq!(
                    restored.tool_settings.skill_dirs,
                    Some(vec![bundle_skill_dir])
                );
                assert!(restored
                    .tool_settings
                    .allowed_dirs
                    .contains(&extra_authorized_dir));

                for forbidden in [
                    "delegate_expert",
                    "expert_exchange",
                    "spawn_agent",
                    "request_user_input",
                ] {
                    assert!(
                        configured.configuration.tools.iter().all(|schema| {
                            nola_domain::expert_task_target::schema_tool_name(schema)
                                != Some(forbidden)
                        }),
                        "leaf configuration must exclude {forbidden} tool schema"
                    );
                    assert!(
                        !configured
                            .configuration
                            .host_catalog
                            .as_ref()
                            .unwrap()
                            .capabilities
                            .contains_key(forbidden),
                        "leaf configuration must exclude {forbidden} capability"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    }
}
