// SPDX-License-Identifier: Apache-2.0
//! Native stdin request validation and assembly over the typed Host contract.
//! No product Config, command dispatcher or terminal dependency in production.
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use crate::host_invocation_request::HostInvocationRequest;
use crate::runtime_config::RuntimeModelCapabilitiesConfig;

/// Resume has no user input channel. Its nested object contains only the
/// original launch configuration; the checkpoint remains the work authority.
fn decode_desktop_operation(request: Value, session: Option<&str>)
    -> Result<(Value, Option<crate::host_invocation_request::DesktopResumeIdentity>), String>
{
    let operation = request.get("operation").map(|value| value.as_str()
        .ok_or("desktop operation must be a string")).transpose()?;
    match operation {
        None | Some("submit") => return Ok((request, None)),
        Some("resume") => {},
        Some(_) => return Err("unsupported desktop operation".into()),
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ResumeRequest {
        operation: String,
        session_id: String,
        expected_run_id: String,
        assistant_message_id: String,
        configuration: serde_json::Map<String, Value>,
        #[serde(default)]
        expert_template_authority: Option<nola_application::expert_template_capture::HostExpertTemplateAuthority>,
    }
    let resume: ResumeRequest = serde_json::from_value(request)
        .map_err(|error| format!("invalid desktop resume request: {error}"))?;
    debug_assert_eq!(resume.operation, "resume");
    for id in [&resume.session_id, &resume.expected_run_id, &resume.assistant_message_id] {
        if id.trim().is_empty() || id.trim() != id || id.len() > 256 || id.chars().any(char::is_control) {
            return Err("invalid desktop resume identity".into());
        }
    }
    if session != Some(resume.session_id.as_str()) {
        return Err("desktop resume session does not match --session".into());
    }
    // Explicit allowlist, not a blacklist: a future submission field must not
    // silently become a recovery input. Route/project roots remain frozen CLI
    // launch arguments and are independently checked by HostInvocationBinding.
    const CONFIG_FIELDS: &[&str] = &[
        "permission_profile", "api_key", "context_window_tokens", "max_iterations",
        "model_purpose", "model_test_mode", "llm_fallbacks", "mcp_servers",
        "permission_rules", "execution_backend", "external_session_id",
        "execution_model", "execution_model_provider", "model_capabilities",
        "vision_input", "reasoning_selection", "reasoning_tier", "max_output_tokens", "temperature",
        "top_p", "sampling_override", "generation_options", "model_protocol",
        "chat_output_token_field", "host_attachments", "mac_launch", "native_model_route",
        "max_retries", "parallel_tool_calls", "thinking_enabled", "reasoning_summary_supported", "prompt_cache", "prompt_cache_retention", "frozen_output_admission",
        "model_profile_id", "expected_profile_revision",
    ];
    if let Some(key) = resume.configuration.keys().find(|key| !CONFIG_FIELDS.contains(&key.as_str())) {
        return Err(format!("desktop resume configuration contains forbidden field {key}"));
    }
    if resume.configuration.get("model_test_mode").is_some_and(|value| value != &Value::Bool(false)) {
        return Err("model test mode cannot resume a Run".into());
    }
    let mut configuration=resume.configuration;
    if let Some(authority)=resume.expert_template_authority {
        authority.validate()?;
        if authority.actor.kind!=nola_core::agent::resource_access::AgentResourcePrincipalKind::LocalUser {
            return Err("native Main recovery requires the current local Host principal".into());
        }
        configuration.insert("expert_template_authority".into(),serde_json::to_value(authority).map_err(|e|e.to_string())?);
    }
    Ok((Value::Object(configuration), Some(crate::host_invocation_request::DesktopResumeIdentity {
        session_id: resume.session_id, expected_run_id: resume.expected_run_id,
        assistant_message_id: resume.assistant_message_id,
    })))
}

pub(crate) fn decode_desktop_cancel_operation(request: &Value, session: Option<&str>)
    -> Result<Option<crate::host_invocation_request::DesktopCancelIdentity>, String>
{
    if request.get("operation").and_then(Value::as_str) != Some("cancel") { return Ok(None); }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CancelRequest {
        operation: String,
        session_id: String,
        expected_run_id: String,
        assistant_message_id: String,
        cancellation_id: String,
    }
    let request: CancelRequest = serde_json::from_value(request.clone())
        .map_err(|error| format!("invalid desktop cancel request: {error}"))?;
    debug_assert_eq!(request.operation, "cancel");
    for id in [&request.session_id, &request.expected_run_id, &request.assistant_message_id, &request.cancellation_id] {
        if id.trim().is_empty() || id.len() > 1024 {
            return Err("invalid desktop cancel identity".into());
        }
    }
    if session != Some(request.session_id.as_str()) {
        return Err("desktop cancel session does not match --session".into());
    }
    Ok(Some(crate::host_invocation_request::DesktopCancelIdentity {
        session_id: request.session_id, expected_run_id: request.expected_run_id,
        assistant_message_id: request.assistant_message_id, cancellation_id: request.cancellation_id,
    }))
}

pub(crate) fn apply_desktop_host_request(config: &mut HostInvocationRequest, payload: &str) -> Result<(), String> {
    apply_desktop_host_request_with_dependencies(config, payload,
        |identity| validate_desktop_resume_identity(
            &mut crate::application_port::CliApplicationPort::persistent_default()?, identity),
        |servers| crate::mcp_host::describe_configured_servers(servers)
            .map_err(|error| format!("{}: {}", error.code, error.message)))
}

/// Decode caller witnesses without consulting a current conversation. Null is
/// the observed absence of a configuration/parent, not permission to refresh.
pub(crate) fn submission_preconditions(request: &Value) -> Result<(Option<i64>, Option<String>), String> {
    let revision = match request.get("expected_configuration_revision") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_i64().filter(|revision| *revision >= 0)
            .ok_or("expected_configuration_revision must be a nonnegative integer or null")?),
    };
    let parent = match request.get("expected_parent_message_id") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let value = value.as_str().ok_or("expected_parent_message_id must be a canonical string or null")?;
            if value.trim().is_empty() || value.trim() != value || value.len() > 256 || value.chars().any(char::is_control) {
                return Err("expected_parent_message_id is invalid".into());
            }
            Some(value.to_owned())
        }
    };
    Ok((revision, parent))
}

pub(crate) fn validate_desktop_resume_identity(application: &mut crate::application_port::CliApplicationPort,
    identity: &crate::host_invocation_request::DesktopResumeIdentity) -> Result<(), String>
{
    let active = application.active_run(&identity.session_id)?
        .ok_or("expected Run is no longer active")?;
    if active.kind != "conversation" || active.run_id != identity.expected_run_id
        || active.assistant_message_id.as_deref() != Some(identity.assistant_message_id.as_str()) {
        return Err("desktop resume canonical Run/assistant identity mismatch".into());
    }
    Ok(())
}

fn apply_desktop_host_request_with_dependencies(
    config: &mut HostInvocationRequest, payload: &str,
    validate_resume: impl FnOnce(&crate::host_invocation_request::DesktopResumeIdentity) -> Result<(), String>,
    describe_mcp: impl FnOnce(&[Value]) -> Result<Vec<Value>, String>,
) -> Result<(), String> {
    const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
    if payload.len() > MAX_REQUEST_BYTES {
        return Err("desktop Agent Host request exceeds the 4 MB limit".to_string());
    }
    let envelope: Value = serde_json::from_str(payload)
        .map_err(|err| format!("invalid desktop Agent Host request: {err}"))?;
    if envelope.get("edited_resend_operation_id").is_some()
        && (envelope.get("operation").and_then(Value::as_str).is_some_and(|operation|operation!="submit")
            || envelope.get("goal_control").is_some() || envelope.get("goal_wake_id").is_some()) {
        return Err("edited resend cannot resume, cancel, or control an existing execution".into());
    }
    if let Some(value) = envelope.get("goal_wake_id") {
        if envelope.get("goal_control").is_some() { return Err("Goal wake cannot also control a Goal".into()); }
        let goal = value.as_str().filter(|v| !v.is_empty() && v.trim() == *v)
            .ok_or("Goal wake requires an exact Goal identity")?;
        config.session.desktop_goal_wake = Some(goal.into());
        return Ok(());
    }
    if let Some(value) = envelope.get("goal_control") {
        let control: crate::goal_control_port::NativeGoalControl = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid native Goal control: {error}"))?;
        let session = config.session.session_id.clone().ok_or("Goal requires a conversation")?;
        let input = control.input(session.clone());
        if !matches!(input.action, nola_application::goal_control_intent::GoalControlAction::Start) {
            // Controls need no current route; execution restores the committed
            // Goal binding only after the exact control has been accepted.
            config.session.desktop_goal_control = Some(input);
            return Ok(());
        }
        config.session.desktop_goal_control = Some(input);
    }
    if let Some(cancel) = decode_desktop_cancel_operation(&envelope, config.session.session_id.as_deref())? {
        config.session.desktop_cancel = Some(cancel);
        config.session.desktop_resume = None;
        config.session.queued_user_command_id = None;

        config.input.message.clear();
        config.input.host_turn_context = None;
        config.input.host_context_components.clear();
        config.session.main_expert_selection=None;
        config.input.host_attachments.clear();
        config.session.host_submission_id = None;
        config.session.host_user_message_id = None;
        config.session.host_assistant_message_id = None;
        config.session.expected_configuration_revision = None;
        config.session.expected_parent_message_id = None;
        config.session.automation_execution_lease = None;
        config.session.automation_execution_spec = None;
        config.session.automation_artifacts.clear();


        config.session.save_session = true;
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        config.tool_settings.mcp_server_registry.clear();
        config.tool_settings.mcp_capability_catalog = None;
        return Ok(());
    }
    let (request, resume) = decode_desktop_operation(envelope, config.session.session_id.as_deref())?;
    // Reject stale ownership before hydrating configuration metadata. This
    // stage never connects MCP; execution rechecks the owner independently.
    if let Some(identity) = &resume { validate_resume(identity)?; }
    let (expected_configuration_revision, expected_parent_message_id) = submission_preconditions(&request)?;
    let prompt = if resume.is_some() { None } else {
        let prompt = request.get("prompt").and_then(Value::as_str)
            .ok_or("desktop Agent Host request is missing prompt")?;
        if prompt.trim().is_empty() { return Err("desktop Agent Host request prompt is empty".into()); }
        Some(prompt)
    };
    let permission_profile = request
        .get("permission_profile")
        .map(|value| {
            let value = value.as_str().ok_or_else(|| {
                "desktop Agent Host permission_profile must be a string".to_string()
            })?;
            crate::permissions::PermissionProfile::from_canonical_name(value).ok_or_else(|| {
                "desktop Agent Host permission_profile must be cautious or allow_all"
                    .to_string()
            })
        })
        .transpose()?
        .unwrap_or(crate::permissions::PermissionProfile::Cautious);
    let stable_identity = |key: &str| -> Result<Option<String>, String> {
        let Some(value) = request.get(key) else {
            return Ok(None);
        };
        let value = value
            .as_str()
            .ok_or_else(|| format!("desktop Agent Host {key} must be a string"))?;
        if value.is_empty() || value.trim() != value || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(format!("desktop Agent Host {key} is invalid"));
        }
        Ok(Some(value.to_string()))
    };
    let submission_id = stable_identity("submission_id")?;
    let user_message_id = stable_identity("user_message_id")?;
    let assistant_message_id = stable_identity("assistant_message_id")?;
    let edited_resend_operation_id=stable_identity("edited_resend_operation_id")?;
    if let Some(operation)=&edited_resend_operation_id {
        uuid::Uuid::parse_str(operation).map_err(|_|"edited resend operation must be a stable UUID")?;
        if resume.is_some() || config.session.session_id.is_none()
            || submission_id.as_deref()!=Some(operation.as_str()) || request.get("goal_control").is_some()
            || request.get("automation_execution_lease").is_some() || request.get("host_task_execution").is_some() {
            return Err("edited resend requires its fresh target and exact submission identities".into());
        }
    }
    config.session.edited_resend_operation_id=edited_resend_operation_id;
    if let Some(raw) = request.get("expert_template_authority") {
        if config.session.session_id.is_none() || config.session.host_task_execution.is_some() {
            return Err("Expert template Host identity requires a native Main session".into());
        }
        let authority: nola_application::expert_template_capture::HostExpertTemplateAuthority =
            serde_json::from_value(raw.clone()).map_err(|_| "invalid native Expert template Host identity")?;
        authority.validate()?;
        config.runtime.expert_template_authority = Some(authority);
    }
    let supplied_identity_count = [
        submission_id.as_ref(),
        user_message_id.as_ref(),
        assistant_message_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    .count();
    if supplied_identity_count != 0 && supplied_identity_count != 3 {
        return Err(
            "desktop Agent Host requires submission_id, user_message_id, and assistant_message_id together"
                .to_string(),
        );
    }
    let queued_user_command_id = stable_identity("queued_user_command_id")?;
    if let Some(command_id) = queued_user_command_id.as_deref() {
        nola_domain::generative_ui::validate_ui_submission_id(command_id).map_err(|error| error.to_string())?;
        if resume.is_some()
            || submission_id.as_deref() != Some(command_id)
            || user_message_id.as_deref() != Some(format!("ui:{command_id}:user").as_str())
            || assistant_message_id.as_deref() != Some(format!("{command_id}:assistant").as_str())
            || request.get("automation_execution_lease").is_some()
        {
            return Err("queued UI input requires its canonical submission identity and a user admission".into());
        }
    }
    config.session.queued_user_command_id = queued_user_command_id;
    let automation_execution_lease = request.get("automation_execution_lease")
        .map(|value| serde_json::from_value::<nola_domain::automation_execution::AutomationExecutionLease>(value.clone())
            .map_err(|error| format!("invalid automation execution lease: {error}")))
        .transpose()?;
    let automation_execution_spec = request.get("automation_execution_spec")
        .map(|value| serde_json::from_value::<nola_domain::automation_execution::AutomationExecutionSpec>(value.clone())
            .map_err(|error| format!("invalid automation execution spec: {error}")))
        .transpose()?;
    if let Some(lease) = &automation_execution_lease {
        if resume.is_some() || supplied_identity_count != 3 || config.session.host_task_execution.is_some() {
            return Err("automation admission requires a new, fully identified submission without a Task owner".into());
        }
        let identity = nola_domain::automation_execution::AutomationExecutionIdentity::for_occurrence(&lease.occurrence_id);
        if config.session.session_id.as_deref() != Some(identity.conversation_id.as_str())
            || submission_id.as_deref() != Some(identity.submission_id.as_str())
            || user_message_id.as_deref() != Some(identity.user_message_id.as_str())
            || assistant_message_id.as_deref() != Some(identity.assistant_message_id.as_str()) {
            return Err("automation submission does not match its occurrence identity".into());
        }
        if !config.session.save_session {
            return Err("automation execution requires durable session storage".into());
        }
    }
    if automation_execution_lease.is_some() != automation_execution_spec.is_some() {
        return Err("automation lease and execution spec must be supplied together".into());
    }
    if request.get("context_messages").is_some() {
        return Err(
            "desktop Agent Host request contains unknown field context_messages".to_string(),
        );
    }
    let requested_api_key = request
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|api_key| !api_key.is_empty());
    let requested_context_window_tokens = request
        .get("context_window_tokens")
        .map(|value| {
            let tokens = value.as_u64().ok_or_else(|| {
                "desktop Agent Host context_window_tokens must be an integer".to_string()
            })?;
            if !(2_000..=4_000_000).contains(&tokens) {
                return Err(
                    "desktop Agent Host context_window_tokens must be between 2000 and 4000000"
                        .to_string(),
                );
            }
            usize::try_from(tokens).map_err(|_| {
                "desktop Agent Host context_window_tokens is unsupported on this platform"
                    .to_string()
            })
        })
        .transpose()?;
    let requested_max_iterations = request
        .get("max_iterations")
        .map(|value| {
            let turns = value.as_u64().ok_or_else(|| {
                "desktop Agent Host max_iterations must be an integer".to_string()
            })?;
            usize::try_from(turns).map_err(|_| {
                "desktop Agent Host max_iterations is unsupported on this platform".to_string()
            })
        })
        .transpose()?;
    let model_purpose = request
        .get("model_purpose")
        .map(|value| {
            let purpose = value
                .as_str()
                .map(str::trim)
                .ok_or_else(|| "desktop Agent Host model_purpose must be a string".to_string())?;
            matches!(
                purpose,
                "chat" | "retrieval" | "issue_repair" | "sensitive_data"
            )
                .then_some(purpose)
                .ok_or_else(|| "desktop Agent Host model_purpose is invalid".to_string())
        })
        .transpose()?;
    let model_test_mode = request
        .get("model_test_mode")
        .map(|value| value.as_bool().ok_or_else(|| {
            "desktop Agent Host model_test_mode must be a boolean".to_string()
        }))
        .transpose()?
        .unwrap_or(false);
    let requested_llm_fallbacks = request
        .get("llm_fallbacks")
        .map(|value| {
            let values = value
                .as_array()
                .ok_or_else(|| "desktop Agent Host llm_fallbacks must be an array".to_string())?;
            if values.len() > 16 {
                return Err("desktop Agent Host has too many LLM fallbacks".to_string());
            }
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let object = value.as_object().ok_or_else(|| {
                        format!("desktop Agent Host llm_fallbacks[{index}] must be an object")
                    })?;
                    let required = |key: &str| {
                        object
                            .get(key)
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .ok_or_else(|| {
                                format!(
                                    "desktop Agent Host llm_fallbacks[{index}].{key} is required"
                                )
                            })
                    };
                    let endpoint = required("endpoint")?;
                    let endpoint_url = url::Url::parse(&endpoint).map_err(|_| {
                        format!("desktop Agent Host llm_fallbacks[{index}].endpoint is invalid")
                    })?;
                    if !matches!(endpoint_url.scheme(), "http" | "https") {
                        return Err(format!(
                            "desktop Agent Host llm_fallbacks[{index}].endpoint is invalid"
                        ));
                    }
                    Ok(crate::runtime_config::RuntimeLlmProvider {
                        provider: object
                            .get("provider")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                        endpoint,
                        api_key: object
                            .get("api_key")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                        model: Some(required("model")?),
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let context_components = request
        .get("context_components")
        .and_then(Value::as_array)
        .map(|components| {
            if components.len() > 64 {
                return Err(
                    "desktop Agent Host request has too many context components".to_string()
                );
            }
            components
                .iter()
                .map(|component| {
                    serde_json::from_value::<nola_core::context_engine::ContextComponent>(
                        component.clone(),
                    )
                    .map_err(|err| {
                        format!("desktop Agent Host context component is invalid: {err}")
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let host_attachments = request
        .get(if resume.is_some() { "host_attachments" } else { "attachments" })
        .map(|value| {
            let attachments = value
                .as_array()
                .ok_or_else(|| "desktop Agent Host attachments must be an array".to_string())?;
            if attachments.len() > 32 {
                return Err("desktop Agent Host has too many attachments".to_string());
            }
            attachments
                .iter()
                .map(|attachment| {
                    let parsed =
                        serde_json::from_value::<crate::external_execution_settings::HostAttachment>(attachment.clone())
                            .map_err(|error| {
                                format!("desktop Agent Host attachment is invalid: {error}")
                            })?;
                    if !parsed.path.is_absolute()
                        || parsed.name.trim().is_empty()
                        || parsed.name.len() > 512
                        || parsed.mime_type.trim().is_empty()
                        || parsed.mime_type.len() > 255
                        || parsed.byte_count > 30 * 1024 * 1024
                    {
                        return Err("desktop Agent Host attachment metadata is invalid".to_string());
                    }
                    let metadata = std::fs::metadata(&parsed.path).map_err(|error| {
                        format!("desktop Agent Host attachment is unavailable: {error}")
                    })?;
                    if !metadata.is_file() || metadata.len() != parsed.byte_count {
                        return Err(
                            "desktop Agent Host attachment size or file type changed".to_string()
                        );
                    }
                    Ok(parsed)
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let input_origin = request
        .get("input_origin")
        .map(|value| {
            serde_json::from_value::<nola_core::agent::AgentInputOrigin>(value.clone())
                .map_err(|_| "desktop Agent Host input_origin is invalid".to_string())
        })
        .transpose()?
        .unwrap_or(nola_core::agent::AgentInputOrigin::InteractiveUser);
    if matches!(input_origin, nola_core::agent::AgentInputOrigin::Unknown) {
        return Err("desktop Agent Host input_origin must be explicit or omitted".to_string());
    }
    let requested_mcp_servers = request
        .get("mcp_servers")
        .map(|value| {
            let servers = value
                .as_array()
                .ok_or_else(|| "desktop Agent Host mcp_servers must be an array".to_string())?;
            if servers.len() > 32 {
                return Err("desktop Agent Host has too many MCP servers".to_string());
            }
            Ok(servers.clone())
        })
        .transpose()?;
    let requested_permission_rules = request
        .get("permission_rules")
        .map(|value| {
            serde_json::from_value::<Vec<nola_core::agent::permissions::PermissionRule>>(
                value.clone(),
            )
            .map_err(|error| format!("desktop Agent Host permission_rules are invalid: {error}"))
        })
        .transpose()?
        .unwrap_or_default();
    if requested_permission_rules.iter().any(|rule| {
        matches!(
            rule.source,
            nola_core::agent::permissions::PermissionRuleSource::ProductDefault
                | nola_core::agent::permissions::PermissionRuleSource::Turn
        )
    }) {
        return Err(
            "desktop Agent Host permission_rules may contain only durable user, Host, or conversation grants"
                .to_string(),
        );
    }

    config.input.message = prompt.unwrap_or_default().to_string();
    config.session.desktop_cancel = None;
    config.session.desktop_resume = resume;
    if config.session.desktop_resume.is_some() {


        config.session.save_session = true;
        config.input.host_turn_context = None;
    }
    config.tool_settings.permission_profile = Some(permission_profile);
    config.tool_settings.auto_approve = permission_profile.auto_approve();
    config.tool_settings.network_policy = permission_profile.network_policy().to_string();
    // `model_purpose` records why the Mac selected this model. It is model
    // routing metadata, not tool authorization. The concrete tool catalog,
    // workspace grants, and permission profile remain the only authorities
    // that can bound what an Agent Run may do.
    let _ = model_purpose;
    config.runtime.llm_fallbacks = requested_llm_fallbacks;
    if let Some(backend) = request.get("execution_backend").and_then(Value::as_str) {
        let backend = backend.trim();
        let descriptor = crate::external_executor::discover_execution_backend(backend, false)
            .ok_or_else(|| format!("unknown execution backend: {backend}"))?;
        if descriptor.availability
            != nola_domain::execution_backend::ExecutionBackendAvailability::Available
        {
            return Err(format!("execution backend is unavailable: {backend}"));
        }
        config.execution.execution_backend = backend.to_string();
    }
    config.execution.external_execution_session_id = request
        .get("external_session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    config.execution.external_execution_model = request
        .get("execution_model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    config.execution.external_execution_model_provider = request
        .get("execution_model_provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    config.input.host_input_origin = input_origin;
    config.session.automation_execution_lease = automation_execution_lease;
    config.session.automation_execution_spec = automation_execution_spec;
    config.session.automation_artifacts = request.get("automation_artifacts")
        .map(|value| serde_json::from_value(value.clone()).map_err(|error| format!("invalid automation artifacts: {error}")))
        .transpose()?.unwrap_or_default();
    config.session.host_submission_id = submission_id;
    config.session.expected_configuration_revision = expected_configuration_revision;
    config.session.expected_parent_message_id = expected_parent_message_id;
    config.session.host_user_message_id = user_message_id;
    config.session.host_assistant_message_id = assistant_message_id;
    config.session.main_expert_selection = decode_main_expert_selection(&request)?;
    if let Some(api_key) = requested_api_key {
        config.model_settings.api_key = api_key.to_string();
    }
    if let Some(context_window_tokens) = requested_context_window_tokens {
        config.runtime.compaction.context_window_tokens = Some(context_window_tokens);
    }
    if let Some(max_iterations) = requested_max_iterations {
        config.execution.max_iterations = max_iterations;
    }
    if (config.session.desktop_resume.is_some() && request.get("mac_launch").is_some())
        || config.session.edited_resend_operation_id.is_some() {
        if let Some(value) = request.get("frozen_output_admission") {
            let admission: nola_host_protocol::HostOutputAdmission = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid frozen output admission: {error}"))?;
            admission.validate().map_err(str::to_string)?;
            config.runtime.max_output_tokens = match admission.preference {
                nola_domain::generation::OutputTokenPreference::Default => None,
                nola_domain::generation::OutputTokenPreference::Explicit(value) => Some(value),
            };
            config.runtime.output_admission = Some(admission);
        }
    }
    config.runtime = config.runtime.with_model_request(&request)?;
    if (config.session.desktop_resume.is_some() && request.get("mac_launch").is_some())
        || config.session.edited_resend_operation_id.is_some() {
        if let Some(value) = request.get("max_retries") {
            config.runtime.admitted_max_retries = Some(value.as_u64().filter(|value| *value <= 6)
                .ok_or("invalid frozen retry budget")? as u32);
        }
        if request.get("parallel_tool_calls").is_some_and(|value| !value.is_null()) {
            return Err("unsupported frozen parallel-tool preference".into());
        }
        if let Some(value) = request.get("prompt_cache") {
            config.runtime.openai_prompt_cache = Some(value.as_bool().ok_or("invalid frozen cache preference")?);
            config.runtime.openai_prompt_cache_key = None;
            config.runtime.openai_prompt_cache_key_prefix = None;
        }
        if let Some(value) = request.get("prompt_cache_retention") {
            config.runtime.openai_prompt_cache_retention = if value.is_null() { None } else {
                Some(value.as_str().filter(|value| value.len() <= 128).ok_or("invalid frozen cache retention")?.to_string())
            };
        }
    }
    if let Some(value) = request.get("model_capabilities") {
        let declared = value.as_object().ok_or("model_capabilities must be an object")?;
        config.runtime.model_capabilities.merge(declared)?;
    }
    if let Some(value) = request.get("vision_input") {
        config.runtime.model_capabilities.vision_input = Some(value.as_bool()
            .ok_or("vision_input must be boolean")?);
    }
    if (config.session.desktop_resume.is_some() && request.get("mac_launch").is_some())
        || config.session.edited_resend_operation_id.is_some() {
        if let Some(value) = request.get("reasoning_summary_supported") {
            config.runtime.model_capabilities.reasoning_summary_supported = serde_json::from_value(value.clone()).map_err(|_| "invalid frozen reasoning summary capability")?;
        }
        if let Some(value) = request.get("thinking_enabled") {
            config.runtime.model_capabilities.reasoning = Some(value.as_bool().ok_or("invalid frozen reasoning availability")?);
        }
    }
    if config.runtime.reasoning_selection.is_some()
        && config.runtime.model_capabilities.reasoning == Some(false) {
        return Err("selected reasoning is unsupported by this model".into());
    }
    if let Some(declaration) = &config.runtime.model_capabilities.reasoning_declaration {
        let protocol = crate::llm::resolved_model_protocol(config.runtime.provider.as_deref(), &config.model_settings.endpoint);
        declaration.validate(protocol).map_err(|error| error.to_string())?;
        let declared = crate::model_profile_resolution::reasoning_configuration(declaration);
        let selected = serde_json::to_value(&config.runtime.reasoning_selection).map_err(|error| error.to_string())?;
        nola_application::resolve_model_reasoning(&declared, protocol,
            config.runtime.max_output_tokens.unwrap_or(16384),
            config.runtime.model_capabilities.reasoning.unwrap_or(true), Some(&selected))?;
    }
    if model_test_mode {
        // The model test is an inference check, not an Agent Turn. Remove
        // capabilities at the Host boundary regardless of CLI defaults.
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        config.tool_settings.mcp_server_registry = Default::default();
        config.tool_settings.mcp_capability_catalog = None;
    }
    config.input.host_context_components = context_components;
    config.runtime.native_model_route = request.get("native_model_route").map(|value| {
        if std::env::var("NOLA_CLIENT_SURFACE").ok().as_deref() != Some("mac")
            || std::env::var_os("NOLA_DESKTOP_HOST").is_none() {
            return Err("native model route requires the Mac stdin Host boundary".to_string());
        }
        let route = serde_json::from_value::<nola_host_protocol::HostNativeModelRouteBinding>(value.clone())
            .map_err(|error| format!("invalid native model route: {error}"))?;
        crate::model_route::validate_native_transport(&route, &config.model_settings.endpoint, &config.model_settings.model, &config.model_settings.api_key)?;
        Ok(route)
    }).transpose()?;
    config.runtime.mac_launch = request.get("mac_launch").map(|value| {
        let descriptor: nola_host_protocol::HostMacLaunchDescriptor = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid Mac launch descriptor: {error}"))?;
        descriptor.validate().map_err(str::to_string)?;
        // A fresh submission supplies input provenance. Resume deliberately has
        // no input channel: its descriptor is checked against the original
        // Run's frozen HostInvocationBinding at the execution boundary instead.
        // Never reinterpret the parser's absent-input default as new authority.
        if config.session.desktop_resume.is_none() {
            validate_mac_input_origin(descriptor.input_origin, config.input.host_input_origin)?;
        }
        if std::env::var("NOLA_CLIENT_SURFACE").ok().as_deref() != Some("mac") {
            return Err("Mac launch descriptor requires the Mac Host boundary".into());
        }
        Ok::<_, String>(descriptor)
    }).transpose()?;
    config.input.host_attachments = host_attachments;
    config.runtime.core_permission_rules = requested_permission_rules;
    if request.get("model_profile_id").is_some() || request.get("expected_profile_revision").is_some()
        || config.runtime.provider.as_deref()==Some("siwc_responses") {
        if std::env::var("NOLA_CLIENT_SURFACE").ok().as_deref()!=Some("mac")
            || std::env::var_os("NOLA_DESKTOP_HOST").is_none() {
            return Err("SIWC profile selection requires the Mac stdin Host boundary".into());
        }
        apply_siwc_profile(config,&request,&crate::store::NolaStore::open_default()?)?;
    }
    if config.runtime.provider.as_deref() != Some("siwc_responses") {
        // Freeze the exact selected profile read used to validate this launch;
        // never look up a later default when constructing a Goal binding.
        if let Some(id) = config.runtime.mac_launch.as_ref().and_then(|launch| launch.model_profile_id.as_deref()) {
            if config.runtime.native_model_route.is_none() {
                let launch = config.runtime.mac_launch.as_ref().unwrap();
                let frozen = config.session.desktop_resume.is_some()
                    || config.session.edited_resend_operation_id.is_some();
                if frozen {
                    // Exact immutable Host binding is checked at recovery admission.
                    // Current profile edits must not relabel an existing Run.
                    config.runtime.selected_profile = launch.model_profile_revision.map(|revision|
                        nola_host_protocol::HostModelProfileReference {
                            profile_id: id.to_owned(), admitted_revision: revision,
                        });
                } else {
                    let store = crate::store::NolaStore::open_default()?;
                    let profile = store.model_profile(id)?.ok_or("Mac selected profile is unavailable")?;
                    if launch.model_profile_revision != Some(profile.revision) {
                        return Err("Mac selected profile changed before submission; refresh the model selection".into());
                    }
                    let resolved = crate::model_profile_resolution::resolve_remote_profile_settings(
                        &profile, &serde_json::json!({}), &Default::default(), &[])?;
                    if resolved.model != config.model_settings.model
                        || crate::model_profile_resolution::model_endpoint_identity(&resolved.endpoint)
                            != crate::model_profile_resolution::model_endpoint_identity(&config.model_settings.endpoint) {
                        return Err("Mac selected profile differs from the explicit launch route".into());
                    }
                    config.runtime.selected_profile = resolved.runtime.selected_profile;
                }
            }
        }
    }
    crate::core_agent_runtime::prepare_edited_resend_host_request(config)?;
    crate::core_agent_runtime::admit_automation_artifacts(config)?;
    if let Some(servers) = requested_mcp_servers.filter(|_| !model_test_mode) {
        let reconciled = crate::resource_access::reconcile_mcp_resources(&[], &servers)?;
        let runtime_servers = crate::resource_access::mcp_runtime_resources(&reconciled);
        let discovered = describe_mcp(&runtime_servers)?;
        let revision =
            crate::agent_host_driver::configured_mcp_capability_catalog_revision(&discovered);
        config.tool_settings.mcp_capability_catalog = Some(
            crate::agent_host_driver::configured_mcp_capability_catalog(&discovered, revision)?,
        );
        config.tool_settings.mcp_server_registry = discovered;
    }

    Ok(())
}

pub(crate) fn apply_siwc_profile(config:&mut HostInvocationRequest,request:&Value,store:&crate::store::NolaStore)->Result<(),String> {
    let id=request["model_profile_id"].as_str().filter(|v|!v.is_empty()).ok_or("SIWC model profile missing")?;
    let revision=request["expected_profile_revision"].as_i64().filter(|v|*v>0).ok_or("SIWC model profile revision missing")?;
    let launch=config.runtime.mac_launch.as_ref().ok_or("SIWC requires frozen Mac launch identity")?;
    if launch.model_profile_id.as_deref()!=Some(id) || launch.model_profile_revision!=Some(revision)
        || config.execution.execution_backend!="nola" || request.get("api_key").is_some()
        || !config.runtime.llm_fallbacks.is_empty() || config.runtime.native_model_route.is_some()
        || config.runtime.provider.as_deref()!=Some("siwc_responses") {
        return Err("SIWC profile identity or credential route mismatch".into());
    }
    let profile=store.model_profile(id)?.ok_or("SIWC selected profile unavailable")?;
    if profile.revision!=revision || !profile.enabled || profile.deleted_at_ms.is_some() {return Err("SIWC selected profile revision changed".into());}
    let source:Value=serde_json::from_str(&profile.source_json).map_err(|_|"SIWC profile source invalid")?;
    if source["wireProtocol"]!="siwc_responses" || source["sourceKind"]!="nola_chatgpt" {return Err("Selected profile is not Nola SIWC".into());}
    // Resend carries an already resolved provider selection (including null).
    // Its saved agent tier controls the tool/context budget; resolving that
    // tier again can invent an effort on a model that never requested one.
    // prepare_edited_resend_host_request subsequently restores and verifies
    // the authoritative model binding from the durable Prepared record.
    let mut profile_request = request.clone();
    if config.session.edited_resend_operation_id.is_some() {
        if profile_request.get("reasoning_selection").is_none() {
            return Err("edited resend requires its frozen reasoning selection".into());
        }
        profile_request.as_object_mut().ok_or("SIWC request must be an object")?
            .remove("reasoning_tier");
    }
    let mut resolved=crate::model_profile_resolution::resolve_remote_profile_settings(&profile,&profile_request,&Default::default(),&[])?;
    crate::model_profile_resolution::bind_siwc_profile(&mut resolved,&profile,store)?;
    if request.get("endpoint").is_some_and(|v|v.as_str().and_then(crate::model_profile_resolution::model_endpoint_identity)!=crate::model_profile_resolution::model_endpoint_identity(&resolved.endpoint))
        || request.get("model").is_some_and(|v|v.as_str()!=Some(resolved.model.as_str())) {
        return Err("SIWC launch route does not match the selected profile".into());
    }
    // Desktop SIWC carries a Core selector, never ordinary Config defaults or a key.
    config.model_settings.endpoint=resolved.endpoint.clone();
    config.model_settings.model=resolved.model.clone();
    config.model_settings.api_key.clear();
    let selected=resolved.runtime.host_runtime_options();
    config.runtime.selected_profile=selected.selected_profile;
    config.runtime.siwc=selected.siwc;
    config.runtime.provider=selected.provider;
    config.runtime.model_capabilities=selected.model_capabilities;
    config.runtime.temperature=selected.temperature;
    config.runtime.top_p=selected.top_p;
    config.runtime.max_output_tokens=selected.max_output_tokens;
    config.runtime.sampling_override=selected.sampling_override;
    config.runtime.generation_options=selected.generation_options;
    config.runtime.reasoning_selection=selected.reasoning_selection;
    config.runtime.reasoning_tier=selected.reasoning_tier;
    config.runtime.compaction.context_window_tokens=selected.compaction.context_window_tokens;
    config.runtime.chat_output_token_field=selected.chat_output_token_field;
    Ok(())
}

fn validate_mac_input_origin(
    frozen: nola_host_protocol::HostMacInputOrigin,
    actual: nola_core::agent::AgentInputOrigin,
) -> Result<(), String> {
    use nola_core::agent::AgentInputOrigin;
    use nola_host_protocol::HostMacInputOrigin;
    if matches!((frozen, actual),
        (HostMacInputOrigin::InteractiveUser, AgentInputOrigin::InteractiveUser)
        | (HostMacInputOrigin::ScheduledAction, AgentInputOrigin::ScheduledAction)) {
        Ok(())
    } else {
        Err("Mac launch input origin differs from the admitted Host input".into())
    }
}

#[cfg(test)]
mod submission_precondition_tests {
    use super::*;

    #[test]
    fn queued_ui_admission_is_identity_bound_and_not_frozen() {
        let wire = json!({"prompt":"form action", "queued_user_command_id":"action-1",
            "submission_id":"action-1", "user_message_id":"ui:action-1:user",
            "assistant_message_id":"action-1:assistant", "expected_configuration_revision":7});
        let mut request = HostInvocationRequest::for_test().unwrap();
        apply_desktop_host_request_with_dependencies(&mut request, &wire.to_string(),
            |_| panic!("not recovery"), |_| panic!("no MCP discovery")).unwrap();
        assert_eq!(request.session.queued_user_command_id.as_deref(), Some("action-1"));
        let frozen = serde_json::to_value(&request).unwrap();
        assert!(frozen["session"].get("queued_user_command_id").is_none());
        let restored: HostInvocationRequest = serde_json::from_value(frozen).unwrap();
        assert!(restored.session.queued_user_command_id.is_none());
        for field in ["submission_id", "user_message_id", "assistant_message_id", "queued_user_command_id"] {
            let mut invalid = wire.clone();
            invalid[field] = json!("different-owner");
            assert!(apply_desktop_host_request_with_dependencies(
                &mut HostInvocationRequest::for_test().unwrap(), &invalid.to_string(),
                |_| panic!("not recovery"), |_| panic!("no MCP discovery")).is_err());
        }
    }

    #[test]
    fn native_model_input_validates_complete_reasoning_declaration_and_choice() {
        let mut request = HostInvocationRequest::for_test().unwrap();
        let wire = json!({"prompt":"fixture", "model_protocol":"anthropic", "max_output_tokens":4096,
            "sampling_override":false, "reasoning_selection":{"kind":"budget","tokens":1024},
            "model_capabilities":{"reasoning":true,"reasoning_declaration":{"control":"budget","tokens":[1024,4096,8192]}}});
        apply_desktop_host_request_with_dependencies(&mut request, &wire.to_string(),
            |_| panic!("send has no recovery"), |_| panic!("no MCP discovery")).unwrap();
        assert!(matches!(request.runtime.model_capabilities.reasoning_declaration,
            Some(nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Budget { ref tokens })
            if tokens == &vec![1024,4096,8192]));
        for selected in [2048,8192] {
            let mut invalid = wire.clone();
            invalid["reasoning_selection"]["tokens"] = json!(selected);
            assert!(apply_desktop_host_request_with_dependencies(&mut HostInvocationRequest::for_test().unwrap(), &invalid.to_string(),
                |_| panic!("send has no recovery"), |_| panic!("no MCP discovery")).is_err());
        }
    }

    #[test]
    fn native_caller_witnesses_are_preserved_without_a_store_lookup() {
        let mut request = HostInvocationRequest::for_test().unwrap();
        apply_desktop_host_request_with_dependencies(&mut request,
            r#"{"prompt":"search current files","expected_configuration_revision":7,"expected_parent_message_id":"observed-parent"}"#,
            |_| panic!("send must not use recovery lookup"), |_| panic!("no MCP discovery requested")).unwrap();
        assert_eq!(request.session.expected_configuration_revision, Some(7));
        assert_eq!(request.session.expected_parent_message_id.as_deref(), Some("observed-parent"));
        let wire = serde_json::to_string(&request).unwrap();
        let restored: HostInvocationRequest = serde_json::from_str(&wire).unwrap();
        assert_eq!(restored.session.expected_configuration_revision, Some(7));
        assert_eq!(restored.session.expected_parent_message_id.as_deref(), Some("observed-parent"));
    }

    #[test]
    fn malformed_preconditions_do_not_become_absence_or_current_versions() {
        for revision in [serde_json::json!(-1), serde_json::json!(1.5), serde_json::json!("7"), serde_json::json!(true)] {
            assert!(submission_preconditions(&serde_json::json!({"expected_configuration_revision":revision})).is_err());
        }
        for parent in [serde_json::json!(""), serde_json::json!(" parent"), serde_json::json!(42), serde_json::json!("line\nbreak")] {
            assert!(submission_preconditions(&serde_json::json!({"expected_parent_message_id":parent})).is_err());
        }
        assert_eq!(submission_preconditions(&serde_json::json!({})).unwrap(), (None, None));
        assert_eq!(submission_preconditions(&serde_json::json!({"expected_configuration_revision":null,"expected_parent_message_id":null})).unwrap(), (None, None));
    }

    #[test]
    fn resume_rejects_new_submission_witnesses() {
        let request = serde_json::json!({"operation":"resume","session_id":"c","expected_run_id":"r","assistant_message_id":"a",
            "configuration":{"expected_configuration_revision":7}});
        assert!(decode_desktop_operation(request, Some("c")).is_err());
    }
}

#[cfg(test)]
mod mac_launch_origin_contract_tests {
    use super::*;

    #[test]
    fn scheduled_resume_hydrates_frozen_descriptor_without_a_new_input_channel() {
        let _lock = crate::store::test_env_lock();
        struct Surface(Option<std::ffi::OsString>);
        impl Drop for Surface {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("NOLA_CLIENT_SURFACE", value),
                    None => std::env::remove_var("NOLA_CLIENT_SURFACE"),
                }
            }
        }
        let _surface = Surface(std::env::var_os("NOLA_CLIENT_SURFACE"));
        std::env::set_var("NOLA_CLIENT_SURFACE", "mac");
        let descriptor = json!({"version":1,"input_origin":"scheduled_action","project_path":"/workspace",
            "authorized_workspace_paths":["/workspace"],"allowed_directory_arguments":["--allow-dir","/workspace"],
            "skill_directory_arguments":["--no-skills"],"active_skill_ids":[],"model_profile_id":"profile","model_profile_revision":7,
            "fallback_profile_ids":[],"permission_mode":"cautious","memory_enabled":false,
            "context_window_tokens":4096,"max_iterations":0,"model_capabilities":{},"vision_input":false,
            "permission_rules":[],"host_attachments":[]});
        let request = json!({"operation":"resume","session_id":"session","expected_run_id":"run",
            "assistant_message_id":"run:assistant","configuration":{"mac_launch":descriptor}});
        let mut config = HostInvocationRequest::for_test().unwrap();
        config.session.session_id = Some("session".into());
        let validated = std::cell::Cell::new(false);
        apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
            |identity| {
                assert_eq!(identity.expected_run_id, "run");
                assert_eq!(identity.assistant_message_id, "run:assistant");
                validated.set(true);
                Ok(())
            }, |_| panic!("no MCP discovery")).unwrap();
        assert!(validated.get());
        assert_eq!(serde_json::to_value(config.runtime.mac_launch.as_ref().unwrap()).unwrap(), descriptor);
        assert!(config.session.desktop_resume.is_some());
        assert_eq!(config.runtime.selected_profile.as_ref().unwrap().admitted_revision, 7);
        assert!(config.input.message.is_empty());
        assert!(config.session.host_submission_id.is_none());
        // No provenance is injected into the input channel by recovery.
        assert_eq!(config.input.host_input_origin, nola_core::agent::AgentInputOrigin::InteractiveUser);
        for origin in ["interactive_user", "scheduled_action"] {
            let mut forbidden = request.clone();
            forbidden["configuration"]["input_origin"] = json!(origin);
            assert!(apply_desktop_host_request_with_dependencies(&mut config, &forbidden.to_string(),
                |_| panic!("invalid wire cannot reach owner validation"), |_| panic!("no discovery")).is_err());
        }
        let mismatched_submit = json!({"operation":"submit","prompt":"new input",
            "input_origin":"interactive_user","mac_launch":descriptor});
        assert!(apply_desktop_host_request_with_dependencies(&mut config, &mismatched_submit.to_string(),
            |_| panic!("submit is not resume"), |_| panic!("no discovery"))
            .unwrap_err().contains("differs from the admitted Host input"));
    }

    #[test]
    fn frozen_launch_origin_must_match_actual_input_without_granting_recovery_authority() {
        use nola_core::agent::AgentInputOrigin as Actual;
        use nola_host_protocol::HostMacInputOrigin as Frozen;
        for frozen in [Frozen::InteractiveUser, Frozen::ScheduledAction] {
            for actual in [Actual::Unknown, Actual::InteractiveUser, Actual::GoalInteractiveUser,
                Actual::ScheduledAction, Actual::Replay, Actual::SystemRecovery,
                Actual::GoalContinuation, Actual::GoalAudit, Actual::DelegatedAgent, Actual::Maintenance] {
                let allowed = matches!((frozen, actual),
                    (Frozen::InteractiveUser, Actual::InteractiveUser)
                    | (Frozen::ScheduledAction, Actual::ScheduledAction));
                assert_eq!(validate_mac_input_origin(frozen, actual).is_ok(), allowed);
            }
        }
    }
}

#[cfg(test)]
mod desktop_cancel_request_tests {
    use super::{apply_desktop_host_request_with_dependencies, decode_desktop_cancel_operation};
    use crate::host_invocation_request::HostInvocationRequest;
    use serde_json::{json, Value};

    fn frame() -> Value {
        json!({"operation":"cancel", "session_id":"session", "expected_run_id":"run",
            "assistant_message_id":"run:assistant", "cancellation_id":"cancel-id"})
    }
    fn config() -> HostInvocationRequest {
        crate::config::Config::from_args(
            ["run", "--json", "--host-request-stdin", "--session", "session"]
                .into_iter().map(str::to_owned)).and_then(|config| config.host_invocation_request()).unwrap()
    }

    #[test]
    fn cancel_hydration_never_enters_resume_validation_or_mcp_discovery() {
        let mut config = config();
        config.input.host_context_components.clear();
        config.input.message = "must not be submitted".into();
        config.tool_settings.mcp_server_registry = vec![json!({"invalid":"must not discover"})];
        apply_desktop_host_request_with_dependencies(&mut config, &frame().to_string(),
            |_| panic!("cancel is not model-bound resume"),
            |_| panic!("cancel cannot discover or spawn MCP")).unwrap();
        let cancel = config.session.desktop_cancel.as_ref().unwrap();
        assert_eq!(cancel.session_id, "session");
        assert_eq!(cancel.expected_run_id, "run");
        assert_eq!(cancel.assistant_message_id, "run:assistant");
        assert_eq!(cancel.cancellation_id, "cancel-id");
        assert!(config.session.desktop_resume.is_none());
        assert!(config.input.message.is_empty());

        assert!(!config.tool_settings.tools_enabled);
        assert_eq!(config.tool_settings.tool_allowlist, Some(Vec::new()));
        assert!(config.tool_settings.mcp_server_registry.is_empty());
        assert!(config.session.host_submission_id.is_none());
    }

    #[test]
    fn cancel_rejects_every_input_or_configuration_field_before_io() {
        for key in ["configuration", "prompt", "submission_id", "user_message_id", "attachments",
            "context_components", "input_origin", "mcp_servers", "permission_profile", "endpoint",
            "model", "reason", "requested_at_ms", "unexpected"] {
            let mut request = frame(); request[key] = Value::Null;
            assert!(apply_desktop_host_request_with_dependencies(&mut config(), &request.to_string(),
                |_| panic!("unexpected resume validation"),
                |_| panic!("unexpected MCP discovery")).is_err(), "{key}");
        }
    }

    #[test]
    fn cancel_requires_complete_bounded_exact_session_and_identity() {
        for key in ["session_id", "expected_run_id", "assistant_message_id", "cancellation_id"] {
            for value in [Value::Null, json!(""), json!(" \n"), json!("界".repeat(342))] {
                let mut request = frame(); request[key] = value;
                assert!(decode_desktop_cancel_operation(&request, Some("session")).is_err(), "{key}");
            }
            let mut request = frame(); request.as_object_mut().unwrap().remove(key);
            assert!(decode_desktop_cancel_operation(&request, Some("session")).is_err());
        }
        assert!(decode_desktop_cancel_operation(&frame(), Some("other")).is_err());
        assert!(decode_desktop_cancel_operation(&frame(), None).is_err());
        let mut request = frame(); request["cancellation_id"] = json!("x".repeat(1024));
        assert!(decode_desktop_cancel_operation(&request, Some("session")).unwrap().is_some());
    }

    #[test]
    fn unknown_cancel_alias_is_not_a_submit_or_resume_operation() {
        let mut request = frame(); request["operation"] = json!("cancel_run");
        assert!(apply_desktop_host_request_with_dependencies(&mut config(), &request.to_string(),
            |_| panic!("unexpected resume validation"), |_| panic!("unexpected MCP discovery"))
            .unwrap_err().contains("unsupported desktop operation"));
    }
}

#[cfg(test)]
mod desktop_automation_admission_tests {
    use super::*;

    #[test]
    fn automation_claim_cannot_enter_an_unrelated_or_unidentified_run() {
        let _environment = crate::store::test_env_lock();
        for identified in [false, true] {
            let mut config = HostInvocationRequest::for_test().unwrap();
            config.session.session_id = Some("unrelated".into());
            let mut request = json!({"operation":"submit", "prompt":"scheduled work",
                "automation_execution_lease":{"occurrence_id":"occurrence", "definition_revision":1,
                    "worker_id":"worker", "lease_token":"lease"}});
            if identified {
                request["submission_id"] = json!("other-submission");
                request["user_message_id"] = json!("user");
                request["assistant_message_id"] = json!("assistant");
            }
            let error = apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
                |_| panic!("submit must not validate resume"),
                |_| panic!("rejected occurrence must not initialize MCP")).unwrap_err();
            assert!(error.contains("automation"), "{error}");
            assert!(config.session.automation_execution_lease.is_none());
        }
    }

    #[test]
    fn automation_admission_checks_every_stable_message_identity() {
        let _environment = crate::store::test_env_lock();
        let identity = nola_domain::automation_execution::AutomationExecutionIdentity::for_occurrence("occurrence");
        for field in ["submission_id", "user_message_id", "assistant_message_id"] {
            let mut config = HostInvocationRequest::for_test().unwrap();
            config.session.save_session = true;
            config.session.session_id = Some(identity.conversation_id.clone());
            let mut request = json!({"operation":"submit", "prompt":"scheduled work",
                "submission_id":identity.submission_id, "user_message_id":identity.user_message_id,
                "assistant_message_id":identity.assistant_message_id,
                "automation_execution_lease":{"occurrence_id":"occurrence", "definition_revision":1,
                    "worker_id":"worker", "lease_token":"lease"}});
            request[field] = json!("foreign-identity");
            let error = apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
                |_| panic!("submit must not validate resume"),
                |_| panic!("rejected identity must not initialize MCP")).unwrap_err();
            assert!(error.contains("occurrence identity"), "{field}: {error}");
        }
    }

    #[test]
    fn automation_admission_never_normalizes_identity_or_disables_storage() {
        let _environment = crate::store::test_env_lock();
        let identity = nola_domain::automation_execution::AutomationExecutionIdentity::for_occurrence("occurrence");
        for invalid_field in [Some("submission_id"), Some("user_message_id"), Some("assistant_message_id"), None] {
            let mut config = HostInvocationRequest::for_test().unwrap();
            config.session.session_id = Some(identity.conversation_id.clone());
            config.session.save_session = invalid_field.is_some();
            let mut request = json!({"operation":"submit", "prompt":"scheduled work",
                "submission_id":identity.submission_id, "user_message_id":identity.user_message_id,
                "assistant_message_id":identity.assistant_message_id,
                "automation_execution_lease":{"occurrence_id":"occurrence", "definition_revision":1,
                    "worker_id":"worker", "lease_token":"lease"}});
            if let Some(field) = invalid_field {
                request[field] = json!(format!(" {} ", request[field].as_str().unwrap()));
            }
            let error = apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
                |_| panic!("submit must not validate resume"),
                |_| panic!("invalid admission must not initialize MCP")).unwrap_err();
            match invalid_field {
                Some(field) => assert!(error.contains(field) && error.contains("invalid"), "{error}"),
                None => assert!(error.contains("durable session storage"), "{error}"),
            }
            assert!(config.session.automation_execution_lease.is_none());
        }
    }

    #[test]
    fn malformed_automation_lease_is_rejected_before_host_initialization() {
        let _environment = crate::store::test_env_lock();
        let identity = nola_domain::automation_execution::AutomationExecutionIdentity::for_occurrence("occurrence");
        let lease = json!({"occurrence_id":"occurrence", "definition_revision":1,
            "worker_id":"worker", "lease_token":"lease"});
        let mut missing_token = lease.clone();
        missing_token.as_object_mut().unwrap().remove("lease_token");
        let mut unknown_authority = lease.clone();
        unknown_authority["allow_all"] = json!(true);
        let mut wrong_revision_type = lease;
        wrong_revision_type["definition_revision"] = json!("1");
        for invalid in [Value::Null, missing_token, unknown_authority, wrong_revision_type] {
            let mut config = HostInvocationRequest::for_test().unwrap();
            config.session.save_session = true;
            config.session.session_id = Some(identity.conversation_id.clone());
            let request = json!({"operation":"submit", "prompt":"scheduled work",
                "submission_id":identity.submission_id, "user_message_id":identity.user_message_id,
                "assistant_message_id":identity.assistant_message_id,
                "automation_execution_lease":invalid});
            let error = apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
                |_| panic!("submit must not validate resume"),
                |_| panic!("malformed lease must not initialize MCP")).unwrap_err();
            assert!(error.contains("invalid automation execution lease"), "{error}");
            assert!(config.session.automation_execution_lease.is_none());
        }
    }
}

#[cfg(test)]
mod desktop_resume_request_tests {
    use super::{apply_desktop_host_request, apply_desktop_host_request_with_dependencies,
        decode_desktop_operation, validate_desktop_resume_identity};
    use serde_json::{json, Value};

    fn frame() -> Value {
        json!({"operation":"resume", "session_id":"session", "expected_run_id":"run",
            "assistant_message_id":"run:assistant", "configuration": {
                "permission_profile":"cautious", "max_output_tokens":64000,
                "model_protocol":"responses", "temperature":0.2,
                "permission_rules":[], "host_attachments":[]}})
    }

    #[test]
    fn native_resume_frame_preserves_route_without_admitting_unknown_configuration() {
        let mut request = frame();
        let route = json!({"adapter":"mlx", "adapter_contract_revision":1, "model_id":"local-model",
            "profile_ref":{"profile_id":"local-model", "admitted_revision":4}});
        request["configuration"]["native_model_route"] = route.clone();
        request["configuration"]["model_protocol"] = json!("chat_completions");
        let (configuration, identity) = decode_desktop_operation(request.clone(), Some("session")).unwrap();
        assert_eq!(configuration["native_model_route"], route);
        assert_eq!(identity.unwrap().expected_run_id, "run");
        // Decoding only preserves the descriptor; the trusted stdin hydration
        // still validates Mac provenance and its concrete native channel.
        request["configuration"]["native_channel_authorized"] = json!(true);
        assert!(decode_desktop_operation(request, Some("session")).unwrap_err()
            .contains("forbidden field native_channel_authorized"));
    }

    #[test]
    fn native_main_resume_reacquires_local_principal_outside_frozen_configuration() {
        let mut request=frame();
        request["expert_template_authority"]=json!({"actor":{"kind":"local_user","id":"current-native-user"},"request_host_id":"native-host"});
        let mut config=crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        config.session.session_id=Some("session".into());
        apply_desktop_host_request_with_dependencies(&mut config,&request.to_string(),|_|Ok(()),|_|Ok(Vec::new())).unwrap();
        assert_eq!(config.runtime.expert_template_authority.unwrap().actor.id,"current-native-user");
        assert!(config.session.desktop_resume.is_some());
        assert!(config.session.main_expert_selection.is_none());
        assert!(config.input.message.is_empty());
        let authority=request.as_object_mut().unwrap().remove("expert_template_authority").unwrap();
        request["configuration"]["expert_template_authority"]=authority;
        assert!(decode_desktop_operation(request,Some("session")).unwrap_err().contains("forbidden field"));
    }

    #[test]
    fn resume_is_not_a_submission_and_preserves_original_configuration() {
        let mut config = crate::config::Config::from_args(
            ["run", "--host-request-stdin", "--session", "session"].into_iter().map(str::to_owned)
        ).and_then(|config| config.host_invocation_request()).unwrap();
        apply_desktop_host_request_with_dependencies(&mut config, &frame().to_string(),
            |_| Ok(()), |_| panic!("fixture has no MCP configuration")).unwrap();
        let identity = config.session.desktop_resume.as_ref().unwrap();
        assert_eq!(identity.expected_run_id, "run");
        assert_eq!(identity.assistant_message_id, "run:assistant");
        assert!(config.input.message.is_empty());
        assert!(config.session.host_submission_id.is_none());
        assert!(config.session.host_user_message_id.is_none());
        assert!(config.session.host_assistant_message_id.is_none());
        assert!(config.input.host_context_components.is_empty());
        assert_eq!(config.runtime.max_output_tokens, Some(64000));
        assert_eq!(config.runtime.provider.as_deref(), Some("openai-responses"));
        assert_eq!(config.tool_settings.permission_profile, Some(crate::permissions::PermissionProfile::Cautious));

    }

    #[test]
    fn resume_rejects_input_fields_in_both_envelope_and_configuration() {
        for key in ["prompt", "submission_id", "user_message_id", "context_components",
            "context_messages", "attachments", "input_origin", "replace_active_run", "selected_experts", "expected_source_store_epoch"] {
            let mut request = frame();
            request[key] = Value::Null;
            assert!(decode_desktop_operation(request, Some("session")).is_err(), "top-level {key}");
            let mut request = frame();
            request["configuration"][key] = Value::Null;
            assert!(decode_desktop_operation(request, Some("session")).is_err(), "config {key}");
        }
    }

    #[test]
    fn resume_requires_exact_complete_identity_and_explicit_operation() {
        for key in ["session_id", "expected_run_id", "assistant_message_id"] {
            for value in [Value::Null, json!(""), json!(" trailing "), json!("a\nb")] {
                let mut request = frame(); request[key] = value;
                assert!(decode_desktop_operation(request, Some("session")).is_err());
            }
        }
        assert!(decode_desktop_operation(frame(), Some("other")).is_err());
        assert!(decode_desktop_operation(frame(), None).is_err());
        let mut request = frame(); request["operation"] = json!("resume_session");
        assert!(decode_desktop_operation(request, Some("session")).is_err());
        let mut request = frame(); request["configuration"]["model_test_mode"] = json!(true);
        assert!(decode_desktop_operation(request, Some("session")).is_err());
    }

    #[test]
    fn submit_keeps_existing_nonempty_prompt_contract() {
        let mut config = crate::config::Config::from_args(
            ["run", "--host-request-stdin"].into_iter().map(str::to_owned)).and_then(|config| config.host_invocation_request()).unwrap();
        assert!(apply_desktop_host_request(&mut config, r#"{"operation":"submit"}"#).is_err());
        apply_desktop_host_request(&mut config, r#"{"operation":"submit","prompt":"hello"}"#).unwrap();
        assert!(config.session.desktop_resume.is_none());
        assert_eq!(config.input.message, "hello");
    }

    #[test]
    fn wrong_persisted_run_or_assistant_never_reaches_mcp_discovery() {
        let root = tempfile::tempdir().unwrap();
        let mut application = crate::application_port::CliApplicationPort::persistent(
            &root.path().join("identity.sqlite3")).unwrap();
        application.begin_assistant_turn(nola_application::BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: "session".into(),
            conversation_title: "Identity fixture".into(),
            user_message_id: "user".into(), user_part_id: "user:text".into(), user_text: "original".into(),
            user_content_json: None, assistant_message_id: "run:assistant".into(),
            context_checkpoint_id: None, created_at_ms: 1,
            core_config:nola_core::agent::AgentCoreConfig {
                turn_identity: crate::test_model_identity::identity(),
                session_id:Some(nola_core::agent::SessionId::from_string("session".into())),
                model: Some("fixture-model".into()),
                host_catalog: Some(serde_json::from_value(serde_json::json!({
                    "host_id":"fixture-host", "revision":1,"availability_epoch":1,"lease":null,
                    "capabilities":{"model.stream":{
                        "id":"model.stream","title":"Fixture model","route":"model",
                        "availability":"available","exposure":"internal","permission_domain":null,
                        "input_schema":{"type":"object"},"input_normalization":{},
                        "output_policy":{"preview_char_limit":4000,"persist_full_output":true,"allow_artifacts":true},
                        "supports_streaming":true,"supports_argument_streaming":false,
                        "parallelism":"serial","cancellation":"cooperative","replay_safety":"recoverable"
                    }}
                })).unwrap()),
                ..Default::default()
            },
            submission:nola_core::agent::AgentSubmission::UserInput {
                items:vec![nola_core::agent::AgentItem::user_message(nola_core::agent::ItemId::from_string("user".into()),"original")],
                turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                submission_id:Some("run".into()),submitted_at_epoch_ms:Some(1),
                delivery:nola_core::agent::protocol::AgentInputDelivery::CurrentTurn,
            },
        }).unwrap();
        let original = application.active_run("session").unwrap().unwrap();
        let discoveries = std::cell::Cell::new(0);
        for key in ["expected_run_id", "assistant_message_id"] {
            let mut config = crate::config::Config::from_args(
                ["run", "--host-request-stdin", "--session", "session"].into_iter().map(str::to_owned)).and_then(|config| config.host_invocation_request()).unwrap();
            let mut request = frame();
            request[key] = json!("stale");
            request["configuration"]["mcp_servers"] = json!([]);
            let error = apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
                |identity| validate_desktop_resume_identity(&mut application, identity),
                |_| { discoveries.set(discoveries.get() + 1); Ok(Vec::new()) }).unwrap_err();
            assert!(error.contains("identity mismatch"), "{error}");
            assert_eq!(discoveries.get(), 0);
            assert!(config.session.desktop_resume.is_none());
        }
        // A positive control proves the spy is connected to the actual parser
        // discovery boundary, not a separate test-only validation path.
        let mut config = crate::config::Config::from_args(
            ["run", "--host-request-stdin", "--session", "session"].into_iter().map(str::to_owned)).and_then(|config| config.host_invocation_request()).unwrap();
        let mut request = frame(); request["configuration"]["mcp_servers"] = json!([]);
        apply_desktop_host_request_with_dependencies(&mut config, &request.to_string(),
            |identity| validate_desktop_resume_identity(&mut application, identity),
            |_| { discoveries.set(discoveries.get() + 1); Ok(Vec::new()) }).unwrap();
        assert_eq!(discoveries.get(), 1);
        let after = application.active_run("session").unwrap().unwrap();
        assert_eq!(after.run_id, original.run_id);
        assert_eq!(after.next_event_sequence, original.next_event_sequence);
    }
}

#[cfg(test)]
mod desktop_model_capability_tests {
    use super::apply_desktop_host_request;
    use crate::config::Config;
    use crate::host_invocation_request::HostInvocationRequest;

    fn config() -> HostInvocationRequest {
        Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        ).and_then(|config| config.host_invocation_request())
        .unwrap()
    }

    #[test]
    fn model_test_removes_tool_and_mcp_capabilities_without_reclassifying_chat() {
        let mut test = config();
        apply_desktop_host_request(
            &mut test,
            r#"{"prompt":"hello","model_purpose":"chat","model_test_mode":true,"mcp_servers":[{"id":"broken"}]}"#,
        )
        .unwrap();
        assert!(!test.tool_settings.tools_enabled);
        assert_eq!(test.tool_settings.tool_allowlist, Some(Vec::new()));
        assert!(test.tool_settings.mcp_server_registry.is_empty());
        assert!(test.tool_settings.mcp_capability_catalog.is_none());

        let mut invalid = config();
        assert!(apply_desktop_host_request(
            &mut invalid,
            r#"{"prompt":"hello","model_test_mode":"true"}"#,
        )
        .unwrap_err()
        .contains("model_test_mode must be a boolean"));
    }

    #[test]
    fn model_purpose_does_not_change_the_desktop_tool_surface_and_preserves_fallback_order() {
        let mut chat = config();
        apply_desktop_host_request(
            &mut chat,
            r#"{"prompt":"hello","model_purpose":"chat","llm_fallbacks":[{"endpoint":"https://second.example/v1","model":"second"},{"endpoint":"https://third.example/v1","api_key":"secret","model":"third"}]}"#,
        )
        .unwrap();
        assert!(chat.tool_settings.tools_enabled);
        assert_eq!(chat.tool_settings.tool_allowlist, None);
        assert_eq!(chat.runtime.llm_fallbacks.len(), 2);
        assert_eq!(
            chat.runtime.llm_fallbacks[0].model.as_deref(),
            Some("second")
        );
        assert_eq!(
            chat.runtime.llm_fallbacks[1].model.as_deref(),
            Some("third")
        );

        let mut retrieval = config();
        apply_desktop_host_request(
            &mut retrieval,
            r#"{"prompt":"find it","model_purpose":"retrieval"}"#,
        )
        .unwrap();
        assert!(retrieval.tool_settings.tools_enabled);
        assert_eq!(retrieval.tool_settings.tool_allowlist, None);

        let mut issue_repair = config();
        apply_desktop_host_request(
            &mut issue_repair,
            r#"{"prompt":"fix it","model_purpose":"issue_repair"}"#,
        )
        .unwrap();
        assert!(issue_repair.tool_settings.tools_enabled);
        assert_eq!(issue_repair.tool_settings.tool_allowlist, None);

        let mut sensitive_data = config();
        apply_desktop_host_request(
            &mut sensitive_data,
            r#"{"prompt":"inspect private notes","model_purpose":"sensitive_data"}"#,
        )
        .unwrap();
        assert!(sensitive_data.tool_settings.tools_enabled);
        assert_eq!(sensitive_data.tool_settings.tool_allowlist, None);

        // Natural multilingual follow-ups do not select or suppress tools.
        // Core performs semantic capability selection from the active catalog.
        let mut natural_followup = config();
        apply_desktop_host_request(
            &mut natural_followup,
            r#"{"prompt":"对，查看当前目录","model_purpose":"chat"}"#,
        )
        .unwrap();
        assert!(natural_followup.tool_settings.tools_enabled);
        assert_eq!(natural_followup.tool_settings.tool_allowlist, None);
    }

    #[test]
    fn desktop_host_applies_a_bounded_model_iteration_budget() {
        let mut bounded = config();
        apply_desktop_host_request(
            &mut bounded,
            r#"{"prompt":"commit the reviewed changes","max_iterations":24}"#,
        )
        .unwrap();
        assert_eq!(bounded.execution.max_iterations, 24);

        let mut invalid = config();
        apply_desktop_host_request(
            &mut invalid,
            r#"{"prompt":"hello","max_iterations":0}"#,
        )
        .unwrap();
        assert_eq!(invalid.execution.max_iterations, 0);
    }
}

/// Shared native/Web transport projection. Selection data is validated again
/// and resolved atomically by Core; it is not a prompt or a permission grant.
pub(crate) fn decode_main_expert_selection(request: &Value) -> Result<Option<nola_domain::expert_task_target::MainExpertSelectionContextV1>,String> {
    let Some(selected)=request.get("selected_experts") else {return Ok(None);};
    let selection:nola_domain::expert_task_target::MainExpertSelectionContextV1=serde_json::from_value(
        serde_json::json!({"version":1,"selected_experts":selected,"expected_source_store_epoch":request.get("expected_source_store_epoch")})).map_err(|_|"invalid_main_expert_selection")?;
    if selection.selected_experts.is_empty() {return Ok(None);}
    Ok(Some(selection))
}

#[cfg(test)]
mod main_expert_selection_transport_tests {
    use super::*;
    #[test]
    fn main_selection_uses_typed_metadata_without_changing_user_text() {
        let input=serde_json::json!({"prompt":"请帮我检查这个改动","selected_experts":[{
            "agent_id":"reviewer","agent_revision":3,"participant_id":null,"context_conversation_id":null
        }]});
        let before=input.clone();
        let selection=decode_main_expert_selection(&input).unwrap().unwrap();
        assert_eq!(selection.version,1);
        assert_eq!(selection.selected_experts[0].agent_id,"reviewer");
        assert_eq!(selection.selected_experts[0].agent_revision,3);
        assert_eq!(input,before);
        assert!(decode_main_expert_selection(&serde_json::json!({"selected_experts":[]})).unwrap().is_none());
    }
    #[test]
    fn selection_transport_rejects_unknown_fields_and_bad_revision_type() {
        for row in [serde_json::json!({"agent_id":"x","agent_revision":"three"}),
            serde_json::json!({"agent_id":"x","agent_revision":1,"actor":"user-supplied"})] {
            assert!(decode_main_expert_selection(&serde_json::json!({"selected_experts":[row]})).is_err());
        }
    }
}
