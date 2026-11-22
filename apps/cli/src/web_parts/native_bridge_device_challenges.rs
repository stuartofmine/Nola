// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
#[path = "device_egress_tests.rs"]
mod device_egress_tests;

impl std::ops::Deref for PendingDeviceToolChallenge {
    type Target = nola_core::device_bridge::DeviceToolRequest;
    fn deref(&self) -> &Self::Target {
        &self.request
    }
}

struct DeviceToolChallengeGuard {
    nonce: String,
}

fn register_device_tool_challenge(
    challenge: &PendingDeviceToolChallenge,
    sender: mpsc::SyncSender<nola_core::device_bridge::DeviceToolResult>,
) -> Result<(), ()> {
    HOST_DEVICE_TOOL_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| ())?
        .insert(challenge.nonce.clone(), challenge.clone());
    HOST_DEVICE_TOOL_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| ())?
        .insert(challenge.nonce.clone(), sender);
    Ok(())
}

impl Drop for DeviceToolChallengeGuard {
    fn drop(&mut self) {
        if let Some(challenges) = HOST_DEVICE_TOOL_CHALLENGES.get() {
            challenges
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.nonce);
        }
        if let Some(waiters) = HOST_DEVICE_TOOL_WAITERS.get() {
            waiters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.nonce);
        }
    }
}

impl PendingDeviceToolChallenge {
    fn validate_at(&self, now_ms: u64) -> Result<u64, &'static str> {
        nola_core::tool_policy::validate_result_egress_evidence(
            &self.receipt,
            &self.grant,
            &nola_core::tool_policy::RuntimeIdentity {
                host_id: Some(self.host_id.clone()),
                client_id: Some(self.client_id.clone()),
                session_id: Some(self.session_id.clone()),
            },
            &self.tool_name,
            ToolRoute::Device,
            now_ms,
        )
    }
}

fn device_bridge_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn device_egress_error() -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: "Device result egress authorization is invalid or expired".into(),
        display: "Device authorization must be refreshed".into(),
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn wait_for_device_tool(
    request: &ToolRequest,
    runtime_tool: &DeviceBridgeRuntimeTool,
    host_id: &str,
    client_id: &str,
    session_id: &str,
    event_tx: &mpsc::Sender<Vec<u8>>,
    mobile_crypto_token: Option<&str>,
) -> ToolOutcome {
    let now_ms = device_bridge_now_ms();
    let deadline_ms = match nola_core::tool_policy::validate_result_egress_evidence(
        &runtime_tool.result_egress_receipt,
        &runtime_tool.result_egress_grant,
        &nola_core::tool_policy::RuntimeIdentity {
            host_id: Some(host_id.into()),
            client_id: Some(client_id.into()),
            session_id: Some(session_id.into()),
        },
        &request.name,
        ToolRoute::Device,
        now_ms,
    ) {
        Ok(deadline) => deadline,
        Err(_) => return device_egress_error(),
    };
    let arguments = Value::Object(request.args.clone());
    let nonce = nola_core::mobile_crypto::generate_protocol_nonce("device-tool");
    let expires_at = deadline_ms.min(now_ms.saturating_add(300_000)) / 1_000;
    let challenge = nola_core::device_bridge::DeviceToolRequest {
        protocol_version: nola_core::device_bridge::DEVICE_BRIDGE_PROTOCOL_VERSION,
        nonce: nonce.clone(),
        request_id: request.id.clone(),
        host_id: host_id.to_string(),
        client_id: client_id.to_string(),
        session_id: session_id.to_string(),
        tool_name: request.name.clone(),
        capability: runtime_tool.capability.clone(),
        arguments: arguments.clone(),
        expires_at,
    };
    let challenge = PendingDeviceToolChallenge {
        request: challenge,
        receipt: runtime_tool.result_egress_receipt.clone(),
        grant: runtime_tool.result_egress_grant.clone(),
    };
    let challenge_guard = DeviceToolChallengeGuard {
        nonce: nonce.clone(),
    };
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    // Registration returns before RAII cleanup, so even a poisoned mutex's
    // temporary guard has been dropped before cleanup tries to acquire it.
    if register_device_tool_challenge(&challenge, result_tx).is_err() {
        return ToolOutcome {
            full_output_ref: None,
            data: None,
            execution: None,
            text: "Device tool challenge state is unavailable".to_string(),
            display: "Device tool failed".to_string(),
            error: true,
            artifacts: Vec::new(),
            activation: None,
        };
    }
    let event = json!({
        "type": "device_tool_request",
        "protocol_version": nola_core::device_bridge::DEVICE_BRIDGE_PROTOCOL_VERSION,
        "id": request.id,
        "nonce": nonce,
        "host_id": host_id,
        "client_id": client_id,
        "session_id": session_id,
        "tool_name": request.name,
        "capability": runtime_tool.capability,
        "arguments": arguments,
        "args": request.args,
        "expires_at": expires_at,
    });
    if challenge.validate_at(device_bridge_now_ms()).is_err() {
        return device_egress_error();
    }
    if event_tx
        .send(mobile_sse_event(event, mobile_crypto_token))
        .is_err()
    {
        return ToolOutcome {
            full_output_ref: None,
            data: None,
            execution: None,
            text: "Device tool client disconnected".to_string(),
            display: "Device tool failed".to_string(),
            error: true,
            artifacts: Vec::new(),
            activation: None,
        };
    }
    let result = result_rx.recv_timeout(StdDuration::from_millis(
        deadline_ms
            .saturating_sub(device_bridge_now_ms())
            .min(300_000),
    ));
    drop(challenge_guard);
    // Even a result accepted by the HTTP handler can become stale while queued.
    // Never publish its body/artifacts after proof expiry or with another nonce.
    if challenge.validate_at(device_bridge_now_ms()).is_err() {
        return device_egress_error();
    }
    if result.as_ref().is_ok_and(|result| {
        nola_core::device_bridge::validate_device_tool_result(
            &challenge,
            result,
            unix_time_seconds(),
        )
        .is_err()
    }) {
        return device_egress_error();
    }
    match result {
        Ok(result) => match crate::tools::explicit_host_artifacts_from_value(&result.output) {
            Ok(artifacts) => ToolOutcome {
                full_output_ref: None,
                data: None,
                execution: None,
                text: serde_json::to_string(&result.output).unwrap_or_else(|_| "null".to_string()),
                display: if result.error {
                    "Device tool failed"
                } else {
                    "Device tool completed"
                }
                .to_string(),
                error: result.error,
                artifacts,
                activation: None,
            },
            Err(error) => ToolOutcome {
                full_output_ref: None,
                data: None,
                execution: None,
                text: error,
                display: "Device tool returned an invalid artifact contract".to_string(),
                error: true,
                artifacts: Vec::new(),
                activation: None,
            },
        },
        Err(_) => ToolOutcome {
            full_output_ref: None,
            data: None,
            execution: None,
            text: "Timed out waiting for the connected device".to_string(),
            display: "Device tool timed out".to_string(),
            error: true,
            artifacts: Vec::new(),
            activation: None,
        },
    }
}

fn sse_response(rx: mpsc::Receiver<Vec<u8>>) -> Response<Box<dyn Read + Send>> {
    Response::new(
        StatusCode(200),
        vec![
            content_type("text/event-stream; charset=utf-8"),
            Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap(),
            Header::from_bytes(&b"X-Accel-Buffering"[..], &b"no"[..]).unwrap(),
        ],
        ChannelReader::new(rx),
        None,
        None,
    )
    .boxed()
}

fn spawn_sse_heartbeat(
    tx: mpsc::Sender<Vec<u8>>,
    stop_rx: mpsc::Receiver<()>,
    interval: StdDuration,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        match stop_rx.recv_timeout(interval) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // SSE comments keep intermediaries and URLSession streams
                // alive without becoming product events or entering history.
                if tx.send(b": keepalive\n\n".to_vec()).is_err() {
                    break;
                }
            }
        }
    })
}

fn respond_web_response(
    request: Request,
    response: Response<Box<dyn Read + Send>>,
) -> Result<(), String> {
    let is_event_stream = response.data_length().is_none();
    if !is_event_stream {
        return request
            .respond(response)
            .map_err(|err| format!("failed to write response: {err}"));
    }

    let status = response.status_code().0;
    let reason = if status == 200 { "OK" } else { "Event Stream" };
    let headers = response.headers().to_vec();
    let mut reader = response.into_reader();
    let mut writer = request.into_writer();
    write!(writer, "HTTP/1.1 {status} {reason}\r\n")
        .map_err(|err| format!("failed to write SSE status: {err}"))?;
    for header in headers {
        if header.field.equiv("Content-Length")
            || header.field.equiv("Transfer-Encoding")
            || header.field.equiv("Connection")
        {
            continue;
        }
        write!(writer, "{}: {}\r\n", header.field, header.value)
            .map_err(|err| format!("failed to write SSE header: {err}"))?;
    }
    writer
        .write_all(b"Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
        .map_err(|err| format!("failed to finish SSE headers: {err}"))?;
    writer
        .flush()
        .map_err(|err| format!("failed to flush SSE headers: {err}"))?;

    let mut buffer = vec![0_u8; 8 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|err| format!("failed to read SSE event: {err}"))?;
        if count == 0 {
            break;
        }
        write!(writer, "{count:x}\r\n")
            .map_err(|err| format!("failed to write SSE chunk size: {err}"))?;
        writer
            .write_all(&buffer[..count])
            .and_then(|_| writer.write_all(b"\r\n"))
            .and_then(|_| writer.flush())
            .map_err(|err| format!("failed to flush SSE event: {err}"))?;
    }
    writer
        .write_all(b"0\r\n\r\n")
        .and_then(|_| writer.flush())
        .map_err(|err| format!("failed to finish SSE response: {err}"))
}

fn resolve_turn_memory_policy(
    body: &Value, config: &Config, conversation_id: Option<&str>,
) -> Result<Option<bool>, Response<Box<dyn Read + Send>>> {
    let requested =
        crate::runtime_config::RuntimeConfig::requested_memory_enabled(body).map_err(|error| {
            json_response(
                StatusCode(422),
                json!({"code":"invalid_memory_policy","error":error}),
            )
        })?;
    let requested = requested.or_else(|| conversation_id.is_none().then_some(config.runtime.memory_enabled).flatten());
    let requested = match requested {
        Some(value) => Some(value),
        None => {
            let preference = (|| -> Result<bool, String> {
                let path = crate::platform_paths::application_database_path()?;
                let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
                crate::assistant_memory_preferences::resolve(&mut app, conversation_id, None)
            })().map_err(|error| json_response(StatusCode(500), json!({"code":"memory_preference_unavailable","error":error})))?;
            Some(preference)
        }
    };
    crate::host_memory_admission::resolve_host_memory_policy(requested).map_err(|error| {
        json_response(
            StatusCode(422),
            json!({"code":"memory_unavailable","error":error}),
        )
    })?;
    Ok(requested)
}

fn resolve_llm_settings(
    body: &Value,
    config: &Config,
    agent_settings: Option<&AgentSettings>,
    conversation_id: Option<&str>,
) -> Result<ResolvedLlmSettings, Response<Box<dyn Read + Send>>> {
    let requested = resolve_turn_memory_policy(body, config, conversation_id)?;
    let mut resolved = resolve_llm_settings_inner(body, config, agent_settings)?;
    // Apply after every model branch, including bound intent and configured profiles.
    resolved.runtime.memory_enabled = requested;
    Ok(resolved)
}

fn resolve_llm_settings_inner(
    body: &Value,
    config: &Config,
    agent_settings: Option<&AgentSettings>,
) -> Result<ResolvedLlmSettings, Response<Box<dyn Read + Send>>> {
    crate::model_profile_resolution::validate_model_request_fields(body).map_err(|error| {
        json_response(StatusCode(422), json!({"code":"invalid_model_request","error":error}))
    })?;
    if let Some(intent) = decode_run_model_intent(body).map_err(|error| {
        json_response(
            StatusCode(422),
            json!({"code":"invalid_model_intent","error":error}),
        )
    })? {
        let store = NolaStore::open_default().map_err(|_| {
            json_response(
                StatusCode(500),
                json!({"error":"Model credential host is unavailable"}),
            )
        })?;
        let session_overrides = match body
            .get("conversationId")
            .and_then(Value::as_str)
        {
            Some(id) => conversation_configuration_state(id)
                .map_err(|error| {
                    json_response(
                        StatusCode(422),
                        json!({"code":"invalid_conversation_configuration","error":error}),
                    )
                })?
                .map(|(_, settings)| {
                    settings["modelOverrides"][format!("configured:{}", intent.profile_id)].clone()
                })
                .unwrap_or(Value::Null),
            None => Value::Null,
        };
        // Read the exact Host-owned profile after the session preferences. No
        // name matching, client declarations or legacy-default fallback.
        let profile = load_bound_intent_profile(&store, &intent)?;
        let source: Value = serde_json::from_str(&profile.source_json)
            .map_err(|_| json_response(StatusCode(422), json!({"error":"Invalid model source"})))?;
        let secret =
            crate::host_credentials::for_profile(&store, &profile, None).map_err(|_| {
                json_response(
                    StatusCode(500),
                    json!({"error":"Cannot read the selected endpoint credential"}),
                )
            })?;
        // Host credential lookup may deliberately yield an unauthenticated
        // endpoint. Never borrow a key from the global process configuration.
        return resolve_bound_model_intent_settings(
            &profile,
            &intent,
            &session_overrides,
            &config.runtime,
            &[(
                source["baseURL"].as_str().unwrap_or(""),
                secret.as_deref().unwrap_or(""),
            )],
        )
        .and_then(|mut settings| {
            crate::model_profile_resolution::bind_siwc_profile(&mut settings, &profile, &store)?;
            Ok(settings)
        })
        .map_err(|error| {
            json_response(
                StatusCode(422),
                json!({"code":"invalid_model_intent","error":error}),
            )
        });
    }
    if let Some(id) = body.get("modelProfileId") {
        let Some(id) = id.as_str().filter(|value| !value.trim().is_empty()) else {
            return Err(json_response(
                StatusCode(422),
                json!({"error":"modelProfileId must be a nonempty string"}),
            ));
        };
        let store = NolaStore::open_default().map_err(|_| {
            json_response(
                StatusCode(500),
                json!({"error":"Model credential host is unavailable"}),
            )
        })?;
        let profile = store
            .model_profile(id)
            .map_err(|error| json_response(StatusCode(500), json!({"error":error})))?
            .ok_or_else(|| {
                json_response(
                    StatusCode(404),
                    json!({"error":"Selected model profile no longer exists"}),
                )
            })?;
        let profile =
            profile_with_session_contract(&profile, &body["__nola_session_model_contract"])
                .map_err(|error| json_response(StatusCode(422), json!({"error":error})))?;
        let secret = crate::host_credentials::for_profile(
            &store,
            &profile,
            body.get("model_protocol").and_then(Value::as_str),
        )
        .map_err(|_| {
            json_response(
                StatusCode(500),
                json!({"error":"Cannot read the selected endpoint credential"}),
            )
        })?;
        let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| {
                json_response(
                    StatusCode(422),
                    json!({"error":"Invalid model endpoint configuration"}),
                )
            })?;
        return resolve_remote_profile_settings(
                &profile,
                body,
                &config.runtime,
                &[(
                    source["baseURL"].as_str().unwrap_or(""),
                    secret.as_deref().unwrap_or(""),
                )],
            )
            .and_then(|mut settings| {
                crate::model_profile_resolution::bind_siwc_profile(&mut settings, &profile, &store)?;
                Ok(settings)
            })
            .map_err(|error| {
                json_response(
                    StatusCode(422),
                    json!({"error":error,"code":"invalid_model_profile"}),
                )
            });
    }
    let request_endpoint = trimmed_string_field(body, "customEndpoint");
    let request_api_key = trimmed_string_field(body, "customApiKey");
    let request_model = trimmed_string_field(body, "model");
    let settings_endpoint = agent_settings.and_then(|settings| {
        non_empty_string(settings.agent_llm_endpoint.trim()).map(ToOwned::to_owned)
    });
    let settings_api_key = agent_settings.and_then(|settings| {
        non_empty_string(settings.agent_llm_api_key.trim()).map(ToOwned::to_owned)
    });
    let settings_model = agent_settings.and_then(|settings| {
        non_empty_string(settings.agent_llm_model.trim()).map(ToOwned::to_owned)
    });
    let temperature = body
        .get("temperature")
        .and_then(Value::as_f64)
        .or_else(|| agent_settings.map(|settings| settings.temperature))
        .unwrap_or(0.7)
        .clamp(0.0, 2.0);
    let top_p = body
        .get("top_p")
        .and_then(Value::as_f64)
        .or_else(|| agent_settings.map(|settings| settings.top_p))
        .unwrap_or(0.9)
        .clamp(0.0, 1.0);
    let max_output_tokens = body
        .get("max_output_tokens")
        .and_then(Value::as_u64)
        .or_else(|| agent_settings.map(|settings| settings.max_output_tokens))
        .unwrap_or(4096)
        .clamp(64, 1_000_000);
    let max_context_tokens = body
        .get("max_context_tokens")
        .and_then(Value::as_u64)
        .or_else(|| agent_settings.map(|settings| settings.max_context_tokens))
        .unwrap_or(32768)
        .clamp(512, 4_000_000);

    let mut endpoint = request_endpoint
        .or(settings_endpoint)
        .unwrap_or_else(|| config.endpoint.trim().to_string());
    let mut api_key = request_api_key
        .or(settings_api_key)
        .unwrap_or_else(|| config.api_key.trim().to_string());
    let model = request_model
        .or(settings_model)
        .unwrap_or_else(|| config.model.trim().to_string());
    let mut max_context_tokens = max_context_tokens;

    let local_engine = local_models::model_runtime(&model);
    if let Some(engine) = &local_engine {
        let Some(local_endpoint) = local_models::running_endpoint(&model) else {
            return Err(json_response(
                StatusCode(503),
                json!({
                    "error": "The selected local model is not running. Start it in Model center first.",
                    "modelId": model,
                    "backend": engine,
                    "action": "start",
                }),
            ));
        };
        endpoint = local_endpoint.base_url;
        api_key = local_endpoint.api_key;
        max_context_tokens = local_endpoint.context_window_tokens as u64;
    }

    if endpoint.is_empty() {
        return Err(json_response(
            StatusCode(400),
            json!({"error": "缺少 LLM endpoint"}),
        ));
    }
    if model.is_empty() {
        return Err(json_response(
            StatusCode(400),
            json!({"error": "model 不能为空，请填写模型名称。"}),
        ));
    }

    let mut runtime = config.runtime.clone();
    if let Some(engine) = &local_engine {
        runtime.provider = Some(if engine == "llama.cpp" { "llamacpp".into() } else { engine.clone() });
        runtime.siwc = None;
        runtime.native_model_route = None;
        runtime.model_binding = None;
    }
    runtime.temperature = Some(temperature);
    runtime.top_p = Some(top_p);
    runtime.max_output_tokens = Some(max_output_tokens);
    runtime.compaction.context_window_tokens = Some(max_context_tokens as usize);
    // This branch resolves an explicit route, not a catalog profile. Do not
    // inherit selection evidence from the process's unrelated default model.
    runtime.selected_profile = None;
    let runtime = runtime.with_model_request(body).map_err(|error| {
        json_response(
            StatusCode(422),
            json!({"error": error, "code": "invalid_model_parameters"}),
        )
    })?;
    let resolved_context = runtime
        .compaction
        .context_window_tokens
        .unwrap_or(max_context_tokens as usize) as u64;
    // Local engine allocation is a hard ceiling, even for explicit overrides.
    let resolved_context = if local_engine.is_some() {
        resolved_context.min(max_context_tokens)
    } else {
        resolved_context
    };
    Ok(ResolvedLlmSettings {
        selected_profile: None,
        endpoint,
        api_key,
        model,
        temperature: runtime.temperature.unwrap_or(temperature),
        top_p: runtime.top_p.unwrap_or(top_p),
        max_output_tokens: runtime.max_output_tokens.unwrap_or(max_output_tokens),
        max_context_tokens: resolved_context,
        runtime,
    })
}

fn prepare_turn_mcp_servers<F>(
    settings: &mut AgentSettings,
    model_test_mode: bool,
    refresh: F,
) -> Result<(), crate::mcp_host::McpHostError>
where
    F: FnOnce(&[Value]) -> Result<Option<Vec<Value>>, crate::mcp_host::McpHostError>,
{
    if model_test_mode {
        // A chat-only model test has no MCP candidates. Its inference must
        // not depend on the health of unrelated MCP connections.
        settings.mcp_servers.clear();
        return Ok(());
    }
    settings.mcp_servers = crate::resource_access::mcp_runtime_resources(&settings.mcp_servers);
    if let Some(refreshed) = refresh(&settings.mcp_servers)? {
        settings.mcp_servers = refreshed;
    }
    Ok(())
}

fn restrict_web_tool_candidates(
    candidates: &mut Vec<crate::tool_policy::ToolCandidate>,
    tools_enabled: bool,
    allowlist: Option<&[String]>,
) {
    candidates.retain(|candidate| tools_enabled && allowlist.is_none_or(|allowed| {
        candidate.schema.pointer("/function/name").and_then(Value::as_str)
            .is_some_and(|name| allowed.iter().any(|id| id.trim().eq_ignore_ascii_case(name)))
    }));
}

fn apply_agent_tool_settings(config: &mut Config, settings: &AgentSettings) {
    let allowed_dirs = settings
        .allowed_dirs
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if !allowed_dirs.is_empty() {
        config.allowed_dirs = allowed_dirs;
    }
    // Skill source selection is not a filesystem grant. Activation may later
    // expose the actual materialized resource through the owned Tool outcome.
    if !settings.bash_prefixes.is_empty() {
        config.bash_prefixes = settings.bash_prefixes.clone();
    }
    let network_policy = normalize_network_policy_value(&settings.network_policy);
    if !network_policy.is_empty() {
        config.network_policy = network_policy;
    }
}

fn resolve_web_builtin_skill_directory(
    id: &str,
    settings: &AgentSettings,
) -> Result<PathBuf, String> {
    let override_entry = settings
        .builtin_skill_overrides
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id));
    if let Some(directory) = override_entry
        .and_then(|entry| entry.get("instructions_ref"))
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("file://"))
        .and_then(|value| url::Url::parse(value).ok())
        .and_then(|parsed| parsed.to_file_path().ok())
        .filter(|path| path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md"))
        .and_then(|path| path.parent().map(PathBuf::from))
    {
        return Ok(directory);
    }
    if let Some(directory) = builtin_skills::managed_directory_for_id(id)?
        .filter(|directory| directory.join("SKILL.md").is_file())
    {
        return Ok(directory);
    }
    let core_admitted = override_entry.is_some_and(|entry| {
        entry.get("revision").and_then(Value::as_i64).is_some()
            && entry.get("enabled").and_then(Value::as_bool) == Some(true)
            && entry.get("removed").and_then(Value::as_bool) != Some(true)
    });
    if core_admitted {
        let content_override = override_entry
            .and_then(|entry| entry.get("content"))
            .and_then(Value::as_str);
        if let Some(directory) = builtin_skills::materialize_single_builtin(id, content_override)?
            .filter(|directory| directory.join("SKILL.md").is_file())
        {
            return Ok(directory);
        }
    }
    Err(format!(
        "Selected Skill {id:?} is not materialized; save its settings to activate it"
    ))
}

fn web_skill_runtime_for_settings(
    configured_dirs: Option<&[PathBuf]>,
    settings: &AgentSettings,
) -> Result<crate::skill_runtime::SkillRuntimeContext, String> {
    let directories = match configured_dirs {
        Some(directories) => directories.to_vec(),
        None => {
            // Freeze exact enabled child directories, never their parents:
            // adding another Skill on disk must not expand an existing Turn.
            let items = builtin_skills::context_index(
                &settings.builtin_skill_overrides,
                &settings.uploaded_skills,
            )?;
            let mut directories = Vec::new();
            for item in items {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Skill catalog entry is missing its identity".to_string())?;
                let directory = if item.get("source").and_then(Value::as_str) == Some("custom") {
                    let resource = settings
                        .uploaded_skills
                        .iter()
                        .find(|resource| resource.get("id").and_then(Value::as_str) == Some(id))
                        .ok_or_else(|| format!("Skill resource {id:?} is unavailable"))?;
                    if !crate::resource_access::allows(
                        resource,
                        nola_core::agent::resource_access::AgentResourceKind::Skill,
                        nola_core::agent::resource_access::AgentResourcePermission::ReadContent,
                    ) {
                        continue;
                    }
                    builtin_skills::managed_directory_for_id(id)?
                        .filter(|directory| directory.join("SKILL.md").is_file())
                        .ok_or_else(|| format!("Selected Skill {id:?} is not materialized; save its settings to activate it"))?
                } else {
                    resolve_web_builtin_skill_directory(id, settings)?
                };
                if !directories.contains(&directory) {
                    directories.push(directory);
                }
            }
            directories
        }
    };
    let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(directories));
    // Invalid explicit roots fail before any catalog or internal rule is
    // exposed. A disabled selection performs no source read.
    if skills.enabled() {
        skills.index()?;
    }
    Ok(skills)
}

fn initialize_web_skill_sources_if_never_initialized(
    configured_dirs: Option<&[PathBuf]>,
    roots: impl FnOnce() -> Result<Vec<PathBuf>, String>,
    exists: impl Fn(&std::path::Path) -> bool,
    synchronize: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    // A lifecycle initialization, not a request-side retry or repair. Any
    // initialized root is authoritative, including missing disabled children.
    if configured_dirs.is_some() {
        return Ok(false);
    }
    let roots = roots()?;
    if roots.len() != 2 {
        return Err("Managed Skill initialization requires its exact two roots".to_string());
    }
    if roots.iter().any(|root| exists(root)) {
        return Ok(false);
    }
    synchronize()?;
    Ok(true)
}

fn web_host_tool_catalog(skills: &crate::skill_runtime::SkillRuntimeContext) -> Vec<Value> {
    crate::tools::tool_schemas_with_skills(skills).into_iter()
        .chain(crate::code_intelligence_provider::available_model_tool_schemas())
        .collect()
}

fn web_local_candidate_schemas_from_catalog(
    settings: &AgentSettings,
    catalog: &[Value],
) -> Vec<Value> {
    catalog
        .iter()
        .filter(|schema| {
            let name = schema
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            settings.enable_local_dev_tools || tool_spec_category(name) == "web_safe"
        })
        .cloned()
        .collect()
}

fn resolve_web_agent_host_tools(
    settings: &AgentSettings,
    permission_rules: &[WebPermissionRule],
    skills: &crate::skill_runtime::SkillRuntimeContext,
) -> Result<Vec<Value>, String> {
    if skills.enabled() {
        skills.index()?;
    }
    Ok(resolve_web_agent_host_tools_from_catalog(
        settings,
        permission_rules,
        &web_host_tool_catalog(skills),
    ))
}

fn resolve_web_agent_host_tools_from_catalog(
    settings: &AgentSettings,
    _permission_rules: &[WebPermissionRule],
    catalog: &[Value],
) -> Vec<Value> {
    let local_candidates = web_local_candidate_schemas_from_catalog(settings, catalog);
    let host_candidates = if settings.host_tools_enabled {
        catalog
    } else {
        &[]
    };
    let mut candidates = candidates_from_schemas(&local_candidates, ToolRoute::AgentHost);
    candidates.extend(candidates_from_schemas(
        host_candidates,
        ToolRoute::AgentHost,
    ));
    resolve_tool_schemas(
        candidates,
        &ToolPlanOptions {
            platform: ToolPlatform::Web,
            conversation_mode: "agent".to_string(),
            network_policy: settings.network_policy.clone(),
            denied_tools: Vec::new(),
            allow_result_egress_routes: Vec::new(),
        },
    )
    .0
}

#[cfg(test)]
mod task_guidance_identity_tests {
    use super::*;
    #[test]
    fn receipt_identity_is_exact_and_bounded_not_normalized() {
        assert_eq!(task_guidance_identity(&json!({"messageId":"original"}), "messageId").unwrap(), "original");
        for value in [json!(" original"), json!("original "), json!(""), json!("x".repeat(513)), json!(42), Value::Null] {
            assert!(task_guidance_identity(&json!({"messageId":value}), "messageId").is_err());
        }
    }
}
