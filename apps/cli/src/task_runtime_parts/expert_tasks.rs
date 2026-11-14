// SPDX-License-Identifier: Apache-2.0
// Same durable Task promotion and worker as temporary subagents. Only target
// configuration and persistent-context selection differ.

pub(crate) fn is_expert_context(context: &str) -> bool {
    let read =
        crate::application_port::CliApplicationPort::persistent_default().and_then(|mut app| {
            app.execute::<Value>(
                nola_application::ApplicationCommand::LoadConversationMetadata {
                    conversation_id: context.to_owned(),
                },
            )
        });
    read.ok()
        .and_then(|value| value["settings_json"].as_str().map(str::to_owned))
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .is_some_and(|settings| settings.get("agentRoomOwner").is_some_and(Value::is_object))
}

/// Advertising never grants dispatch authority. Core rechecks the actual Main
/// Run, exact Room association and target when the concrete effect is prepared.
pub(crate) fn can_dispatch_experts(context: &str) -> bool {
    let read=(|| -> Result<bool,String> {
        let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
        let metadata:Value=app.execute(nola_application::ApplicationCommand::LoadConversationMetadata {conversation_id:context.into()})?;
        // A fresh explicit Main session is created by Core in the same user
        // admission transaction. Advertising is not execution authority.
        if metadata.is_null() {return Ok(true);}
        let settings:Value=serde_json::from_str(metadata["settings_json"].as_str().unwrap_or("{}"))
            .map_err(|_|"invalid main conversation settings")?;
        if settings.get("agentRoomOwner").is_some() {return Ok(false);}
        Ok(true)
    })();
    matches!(read,Ok(true))
}

enum ExpertTaskHostError {
    Rejected(String),
    Pending(String),
}
impl From<String> for ExpertTaskHostError {
    fn from(message: String) -> Self {
        if nola_service_client::ApplicationServiceCommandError::unresolved_from_boundary_message(
            &message,
        )
        .is_some()
        {
            Self::Pending(message)
        } else {
            Self::Rejected(message)
        }
    }
}
impl From<&str> for ExpertTaskHostError {
    fn from(message: &str) -> Self {
        Self::Rejected(message.into())
    }
}
impl From<nola_application::host_expert_template_operation::HostExpertOperationError>
    for ExpertTaskHostError
{
    fn from(
        error: nola_application::host_expert_template_operation::HostExpertOperationError,
    ) -> Self {
        match error {
            nola_application::host_expert_template_operation::HostExpertOperationError::OutcomePending{..}=>Self::Pending(error.to_string()),
            _=>Self::Rejected(error.to_string()),
        }
    }
}
fn expert_host_operation<T: serde::de::DeserializeOwned>(
    operation: Value,
) -> Result<T, ExpertTaskHostError> {
    let operation = serde_json::from_value(operation)
        .map_err(|error| format!("Invalid expert Host operation: {error}"))?;
    crate::application_port::CliApplicationPort::persistent_default()?
        .execute_expert_host_operation(&uuid::Uuid::new_v4().to_string(), operation)
        .map_err(ExpertTaskHostError::from)
}

pub(crate) fn prepare_expert_task_admission(
    task_id: &str,
    begin: &mut nola_application::BeginAssistantTurn,
) -> Result<(), String> {
    let store = NolaStore::open_default()?;
    let task = store
        .runtime_task(task_id)?
        .ok_or("Task admission record missing")?;
    let metadata: Value = serde_json::from_str(&task.metadata_json).map_err(|e| e.to_string())?;
    if metadata.get("expert_target").is_none() {
        return Ok(());
    }
    let mut frozen: nola_core::agent::AgentCoreConfig =
        crate::application_port::CliApplicationPort::persistent_default()?.execute(
            nola_application::ApplicationCommand::LoadMobileTaskChildConfiguration {
                task_id: task_id.into(),
            },
        )?;
    let expected = frozen
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Expert frozen Host binding missing")?;
    let actual = begin
        .core_config
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Expert actual Host binding missing")?;
    expected.require_exact(actual).map_err(str::to_owned)?;
    // History is read from the existing target conversation at this actual
    // admission boundary. Security policy and model facts remain the frozen
    // target's; no source transcript or current defaults enter this Run.
    frozen.initial_items = begin.core_config.initial_items.clone();
    begin.core_config = frozen;
    Ok(())
}

fn expert_runtime_response(
    response: nola_core::agent::host::HostResponse,
) -> Result<RuntimeToolResult, String> {
    use nola_core::agent::host::HostResponse;
    Ok(match response {
        HostResponse::TaskPromoted {
            task_id,
            input_watermark,
            kind,
            state,
            progress,
            current_step,
            lineage,
        } => RuntimeToolResult {
            outcome: task_ok(
                json!({"task_id":task_id,"exchange_id":task_id,"status":"queued"}).to_string(),
                "Expert task queued".into(),
            ),
            task_promotion: Some(RuntimeTaskPromotion {
                task_id,
                input_watermark,
                kind,
                state,
                progress,
                current_step,
                lineage,
            }),
            task_update: None,
            pending_error: None,
        },
        HostResponse::TaskUpdated {
            task_id,
            state,
            progress,
            current_step,
            result_ref,
            cancel_requested,
            ownership_token,
            completion_watermark,
        } => RuntimeToolResult::task_updated(
            task_ok(
                json!({"task_id":task_id,"replayed":true}).to_string(),
                "Expert task restored".into(),
            ),
            RuntimeTaskUpdate {
                task_id,
                state,
                progress,
                current_step,
                result_ref,
                cancel_requested,
                ownership_token,
                completion_watermark,
            },
        ),
        HostResponse::ToolCompleted {
            output,
            outcome,
            activation,
            full_output_ref,
            artifacts,
            ..
        } => {
            let error = matches!(outcome, nola_domain::host_effect::HostToolOutcome::Failed);
            let text = serde_json::to_string(&output).map_err(|e| e.to_string())?;
            RuntimeToolResult::completed(ToolOutcome {
                text,
                display: "Expert cooperation".into(),
                error,
                data: Some(output),
                activation,
                full_output_ref,
                artifacts,
                execution: Some(nola_host_protocol::tool_result::ExecutionReceipt {
                    state: nola_host_protocol::tool_result::ExecutionState::Completed,
                    ..Default::default()
                }),
            })
        }
        _ => return Err("Unsupported expert Task response".into()),
    })
}

fn execute_expert_task_tool(
    config: &HostInvocationRequest,
    request: &HostRequestEnvelope,
    context: &ToolContext,
) -> RuntimeToolResult {
    let mut preparation = None;
    let result = (|| -> Result<RuntimeToolResult, ExpertTaskHostError> {
        if config.session.desktop_resume.is_some() && config.runtime.expert_template_authority.is_none() {
            return Err(ExpertTaskHostError::Pending("Native Main dispatch recovery requires the current native Host identity".into()));
        }
        let trusted_authority=config.runtime.expert_template_authority.clone()
            .unwrap_or_else(crate::web::expert_host_authority);
        let authority: nola_application::expert_template_capture::HostExpertTemplateAuthority =
            expert_host_operation(json!({"kind":"derive_peer_authority","request":request,"authority":trusted_authority}))?;
        let nola_core::agent::host::HostRequest::ExecuteTool {
            call_id,
            input: arguments,
            ..
        } = &request.payload
        else {
            return Err("Expert cooperation requires an admitted tool request".into());
        };
        if arguments["action"] == "request" {
            let source_run_id = request
                .turn_id
                .clone()
                .ok_or("Expert source Run is missing")?;
            let input = nola_application::expert_peer::PeerExchangeInput {
                source_context_id: request.session_id.clone(),
                source_run_id,
                call_id: call_id.clone(),
                target_participant_id: arguments["target_participant_id"]
                    .as_str()
                    .ok_or("Expert target missing")?
                    .into(),
                kind: "request".into(),
                text: arguments["text"]
                    .as_str()
                    .ok_or("Expert brief missing")?
                    .into(),
                in_reply_to: arguments
                    .get("in_reply_to")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or("Invalid expert reply identity")
                    })
                    .transpose()?,
            };
            let material = crate::web::capture_expert_peer_material(request, &input, &authority)?;
            preparation = Some((material.view.exchange_id.clone(), authority.clone()));
            // Redelivery uses the exact frozen original target, including after
            // settings changes; do not resolve current defaults for an old call.
            let task = NolaStore::open_default()?.runtime_task(&material.view.exchange_id)?;
            let target = if let Some(task) = task {
                let metadata: Value =
                    serde_json::from_str(&task.metadata_json).map_err(|e| e.to_string())?;
                crate::web::ExpertTaskHostConfiguration {
                    configuration: serde_json::from_value(
                        metadata["expert_target_configuration"].clone(),
                    )
                    .map_err(|e| e.to_string())?,
                    invocation: Some(
                        serde_json::from_value(metadata["delegated_invocation"].clone())
                            .map_err(|e| e.to_string())?,
                    ),
                    project_root: task.project_root,
                }
            } else {
                crate::web::expert_task_host_configuration(
                    &config.tool_settings.project_root,
                    &material,
                )?
            };
            let prepared: Value = expert_host_operation(
                json!({"kind":"prepare_peer_task","request":request,
                "exchange_id":material.view.exchange_id,"target_configuration":target.configuration,
                "target_invocation":target.invocation,"project_root":target.project_root,"authority":authority}),
            )?;
            let response =
                serde_json::from_value(prepared["response"].clone()).map_err(|e| e.to_string())?;
            tools::wake_background_task_notification();
            return expert_runtime_response(response).map_err(ExpertTaskHostError::from);
        }
        // The same supervisor owns wake/recovery; observing never creates a
        // second execution owner, including after a bounded wait times out.
        reconcile_supervised_tasks(config);
        let mut deadline = None;
        loop {
            if context.cancellation.is_requested() {
                return Err("Expert wait cancelled; the separate task was not stopped".into());
            }
            let observation: Value = expert_host_operation(
                json!({"kind":"peer_task_action","request":request,"authority":authority}),
            )?;
            let wait_ms = observation["wait_ms"].as_u64().unwrap_or(0).min(30_000);
            let until =
                *deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(wait_ms));
            if observation["terminal"] == true || wait_ms == 0 || Instant::now() >= until {
                return expert_runtime_response(
                    serde_json::from_value(observation["response"].clone())
                        .map_err(|e| e.to_string())?,
                )
                .map_err(ExpertTaskHostError::from);
            }
            std::thread::sleep(
                Duration::from_millis(100).min(until.saturating_duration_since(Instant::now())),
            );
        }
    })();
    match result {
        Ok(value) => value,
        Err(ExpertTaskHostError::Pending(message)) => RuntimeToolResult::pending(message),
        Err(ExpertTaskHostError::Rejected(message)) => {
            if let Some((exchange, authority)) = preparation {
                let aborted: Result<Value, ExpertTaskHostError> =
                    expert_host_operation(json!({"kind":"abort_peer_task_preparation",
                    "request":request,"exchange_id":exchange,"authority":authority}));
                if !matches!(aborted,Ok(ref value) if value["preparation_aborted"]==true) {
                    return RuntimeToolResult::pending(message);
                }
            }
            RuntimeToolResult::completed(task_error(message))
        }
    }
}
