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

pub(crate) fn expert_task_host_configuration(
    project_root: &std::path::Path,
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
    let own = config.host_invocation_request()?;
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
