fn has_unapplied_web_skill_settings(value: &Value) -> bool {
    value.get("builtin_skill_overrides").is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))
}

// Explicit configuration retirement, not a second Skill runtime or automatic import.
// The previous document is retained verbatim before the current settings CAS.
fn confirm_core_skill_settings(application: &mut crate::application_port::CliApplicationPort, expected_revision: i64) -> Result<String, String> {
    use nola_application::{ApplicationCommand, ProductSetting};
    use sha2::{Digest, Sha256};
    let current: ProductSetting = application.execute::<Option<ProductSetting>>(ApplicationCommand::GetProductSetting {
        key: "web.agent.settings".into(),
    })?.ok_or("Web settings disappeared")?;
    if current.revision != expected_revision { return Err("Web settings changed; review the current settings before continuing".into()); }
    let mut value: Value = serde_json::from_str(&current.value_json).map_err(|error| error.to_string())?;
    if !has_unapplied_web_skill_settings(&value) { return Err("No unapplied Skill settings require review".into()); }
    let previous = serde_json::to_string(&value["builtin_skill_overrides"]).map_err(|error|error.to_string())?;
    let backup_key = format!("web.skill-settings.unapplied.{:x}", Sha256::digest(previous.as_bytes()));
    let existing: Option<ProductSetting> = application.execute(ApplicationCommand::GetProductSetting { key: backup_key.clone() })?;
    if let Some(existing) = existing {
        if existing.value_json != previous { return Err("Retained Skill settings identity conflict".into()); }
    } else {
        let _: ProductSetting = application.execute(ApplicationCommand::PutProductSettingChecked {
            key: backup_key.clone(), value_json: previous, expected_revision: None,
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        })?;
    }
    value.as_object_mut().ok_or("Stored Web settings are not an object")?.remove("builtin_skill_overrides");
    value["skill_settings_backup_key"] = json!(backup_key);
    let _: ProductSetting = application.execute(ApplicationCommand::PutProductSettingChecked {
        key: "web.agent.settings".into(), value_json: value.to_string(), expected_revision: Some(expected_revision),
        updated_at_ms: chrono::Utc::now().timestamp_millis(),
    })?;
    Ok(backup_key)
}

fn load_agent_settings_from_store(store: &NolaStore) -> Result<AgentSettings, String> {
    let mut settings = agent_settings_from_stored_value(store.load_web_agent_settings()?)?;
    let profile =
        crate::permissions::PermissionProfile::from_canonical_name(&settings.permission_profile)
            .ok_or_else(|| "Invalid stored permission_profile".to_string())?;
    settings.permission_profile = profile.canonical_name().to_string();
    settings.network_policy = profile.network_policy().to_string();
    settings.uploaded_documents = store.load_web_uploaded_documents()?;
    settings.uploaded_skills = crate::skill_catalog_port::load_web_uploaded_skills()?;
    settings.builtin_skill_overrides = crate::skill_catalog_port::load_web_builtin_skills()?;
    settings.mcp_servers = crate::mcp_catalog_port::load_web_mcp_servers()?;
    Ok(settings)
}

/// Missing settings initialize defaults; an existing invalid setting is an error.
fn agent_settings_from_stored_value(value: Option<Value>) -> Result<AgentSettings, String> {
    let Some(value) = value else {
        return Ok(AgentSettings::default());
    };
    let Some(object) = value.as_object() else {
        return Err("Stored agent settings must be an object".into());
    };
    if has_unapplied_web_skill_settings(&value) {
        return Err("Previous Web Skill settings require review. Open Skills and MCP to retain them and explicitly choose the Core configuration. No previous instructions have been applied or deleted.".into());
    }
    if let Some(profile) = object.get("permission_profile") {
        profile.as_str().and_then(crate::permissions::PermissionProfile::from_canonical_name)
            .ok_or_else(|| "Stored permission_profile must be cautious or allow_all".to_string())?;
    }
    serde_json::from_value(value).map_err(|error| format!("Invalid stored agent settings: {error}"))
}

fn list_permission_rules_response() -> Response<Box<dyn Read + Send>> {
    match crate::permission_grants::load_web_permission_rules() {
        Ok(rules) => json_response(
            StatusCode(200),
            json!({"rules": rules, "localFirst": true, "backend": "cli-rust"}),
        ),
        Err(err) => json_response(StatusCode(500), json!({"error": err})),
    }
}

fn create_permission_rule_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<PermissionRuleInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let tool_name = input.tool_name.unwrap_or_default().trim().to_string();
    if tool_name.is_empty() {
        return json_response(StatusCode(400), json!({"error": "tool_name required"}));
    }
    let behavior = input
        .behavior
        .unwrap_or_else(|| "allow".to_string())
        .trim()
        .to_ascii_lowercase();
    if !matches!(behavior.as_str(), "allow" | "deny" | "ask") {
        return json_response(
            StatusCode(400),
            json!({"error": "behavior must be 'allow', 'deny', or 'ask'"}),
        );
    }
    let match_pattern = input
        .match_pattern
        .unwrap_or_else(|| "*".to_string())
        .trim()
        .to_string();
    let match_pattern = if match_pattern.is_empty() {
        "*".to_string()
    } else {
        match_pattern
    };
    let mut rules = match crate::permission_grants::load_web_permission_rules() {
        Ok(values) => values,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let next_id = rules
        .iter()
        .filter_map(|rule| rule.get("id").and_then(Value::as_u64))
        .max()
        .unwrap_or(0)
        + 1;
    let rule = WebPermissionRule {
        id: next_id,
        tool_name,
        behavior,
        match_pattern,
    };
    rules.push(json!(rule.clone()));
    if let Err(err) = crate::permission_grants::replace_web_permission_rules(&rules) {
        return json_response(StatusCode(500), json!({"error": err}));
    }
    json_response(StatusCode(200), json!(rule))
}

fn delete_permission_rule_response(rule_id: u64) -> Response<Box<dyn Read + Send>> {
    let mut rules = match crate::permission_grants::load_web_permission_rules() {
        Ok(values) => values,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let before = rules.len();
    rules.retain(|rule| rule.get("id").and_then(Value::as_u64) != Some(rule_id));
    let deleted = rules.len() != before;
    if deleted {
        if let Err(err) = crate::permission_grants::replace_web_permission_rules(&rules) {
            return json_response(StatusCode(500), json!({"error": err}));
        }
    }
    json_response(
        if deleted {
            StatusCode(200)
        } else {
            StatusCode(404)
        },
        json!({"deleted": deleted, "localFirst": true}),
    )
}

fn web_permission_rules_for_core(
    rules: &[WebPermissionRule],
) -> Vec<nola_core::agent::permissions::PermissionRule> {
    rules
        .iter()
        .filter_map(|rule| {
            let tool_name = rule.tool_name.trim();
            if tool_name.is_empty() {
                return None;
            }
            let capability = if tool_name == "*" {
                "*".to_string()
            } else {
                nola_core::agent::policy::permission_name_for_tool(tool_name)
            };
            let decision = match rule.behavior.trim().to_ascii_lowercase().as_str() {
                "allow" => nola_core::agent::permissions::PermissionPolicyDecision::Allow,
                "ask" => nola_core::agent::permissions::PermissionPolicyDecision::Ask,
                "deny" => nola_core::agent::permissions::PermissionPolicyDecision::Deny,
                _ => return None,
            };
            Some(nola_core::agent::permissions::PermissionRule {
                resource_pattern:
                    nola_core::agent::permissions::permission_resource_pattern_for_rule(
                        &capability,
                        rule.match_pattern.trim(),
                    ),
                capability_pattern: capability,
                decision,
                source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
            })
        })
        .collect()
}

fn agent_tools_response(config: &Config) -> Response<Box<dyn Read + Send>> {
    let settings =
        match NolaStore::open_default().and_then(|store| load_agent_settings_from_store(&store)) {
            Ok(settings) => settings,
            Err(error) => {
                return json_response(
                    StatusCode(500),
                    json!({"error":error,"code":"agent_settings_snapshot_failed"}),
                )
            }
        };
    let skills = match web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &settings) {
        Ok(skills) => skills,
        Err(error) => {
            return json_response(
                StatusCode(502),
                json!({"error":error,"code":"skill_catalog_unavailable"}),
            )
        }
    };
    let mut tools = web_host_tool_catalog(&skills)
        .into_iter()
        .filter_map(|schema| {
            let function = schema.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?.to_string();
            Some(json!({
                "name": name,
                "description": function.get("description").and_then(Value::as_str).unwrap_or(""),
                "category": "cli",
                "is_read_only": nola_core::code_intelligence::operation_for_capability_id(&name)
                    .map(|operation| operation.is_read_only()).unwrap_or_else(|| tool_is_read_only(&name)),
                "is_concurrency_safe": nola_core::code_intelligence::operation_for_capability_id(&name)
                    .map(|operation| operation.is_read_only()).unwrap_or_else(|| tool_is_read_only(&name)),
                "parameters": function.get("parameters").cloned().unwrap_or_else(|| json!({})),
            }))
        })
        .collect::<Vec<_>>();
    tools.sort_by(|a, b| {
        let a_name = a.get("name").and_then(Value::as_str).unwrap_or("");
        let b_name = b.get("name").and_then(Value::as_str).unwrap_or("");
        a_name.cmp(b_name)
    });
    json_response(StatusCode(200), json!({"tools": tools}))
}

fn mobile_login_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let loopback = request
        .remote_addr()
        .is_some_and(|address| address.ip().is_loopback());
    let trusted_https_proxy = std::env::var("NOLA_TRUST_HTTPS_PROXY")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1")
        && request_header(request, "X-Forwarded-Proto")
            .is_some_and(|value| value.eq_ignore_ascii_case("https"));
    if !loopback && !trusted_https_proxy {
        return json_response(
            StatusCode(403),
            json!({"error": "password pairing requires loopback or a configured trusted HTTPS proxy"}),
        );
    }
    let login_key = request
        .remote_addr()
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if mobile_login_is_rate_limited(&login_key) {
        return json_response(
            StatusCode(429),
            json!({"error": "too many pairing attempts; try again later"}),
        );
    }
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let email = body
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("local@nola.cli")
        .trim()
        .to_ascii_lowercase();
    let password = body
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if email.is_empty() || password.is_empty() {
        return json_response(
            StatusCode(400),
            json!({"error": "email and password required"}),
        );
    }
    let pairing_secret = match std::env::var("NOLA_MOBILE_PAIRING_SECRET") {
        Ok(value) if value.trim().chars().count() >= 16 => value,
        _ => {
            return json_response(
                StatusCode(503),
                json!({"error": "mobile pairing is disabled; configure NOLA_MOBILE_PAIRING_SECRET with at least 16 characters"}),
            )
        }
    };
    if !nola_core::mobile_crypto::constant_time_secret_eq(password, pairing_secret.trim()) {
        record_mobile_login_failure(&login_key);
        return json_response(
            StatusCode(401),
            json!({"error": "invalid mobile pairing credentials"}),
        );
    }
    clear_mobile_login_failures(&login_key);
    let token = nola_core::mobile_crypto::generate_mobile_access_token();
    {
        let mut tokens = mobile_tokens().lock().unwrap();
        mobile_cleanup_locked(&mut tokens);
        if tokens.len() >= 32 {
            if let Some(oldest) = tokens
                .iter()
                .min_by_key(|(_, seen)| *seen)
                .map(|(token, _)| token.clone())
            {
                tokens.remove(&oldest);
            }
        }
        tokens.insert(token.clone(), Instant::now());
    }
    json_response(
        StatusCode(200),
        json!({
            "token": token,
            "token_type": "Bearer",
            "expires_in": MOBILE_TOKEN_TTL_SEC,
            "local_user_id": "cli-local-user",
            "display_name": "Nola CLI",
            "email": email,
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn mobile_login_is_rate_limited(key: &str) -> bool {
    let Ok(mut failures) = MOBILE_LOGIN_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    else {
        return true;
    };
    failures.retain(|_, (_, started)| started.elapsed() <= StdDuration::from_secs(300));
    failures.get(key).is_some_and(|(count, _)| *count >= 5)
}

fn record_mobile_login_failure(key: &str) {
    if let Ok(mut failures) = MOBILE_LOGIN_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        let entry = failures
            .entry(key.to_string())
            .or_insert((0, Instant::now()));
        if entry.1.elapsed() > StdDuration::from_secs(300) {
            *entry = (0, Instant::now());
        }
        entry.0 = entry.0.saturating_add(1);
    }
}

fn clear_mobile_login_failures(key: &str) {
    if let Ok(mut failures) = MOBILE_LOGIN_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        failures.remove(key);
    }
}

fn mobile_me_response(request: &Request) -> Response<Box<dyn Read + Send>> {
    if mobile_bearer_token(request).is_none() {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    }
    json_response(
        StatusCode(200),
        json!({
            "local_user_id": "cli-local-user",
            "display_name": "Nola CLI",
            "email": "local@nola.cli",
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn mobile_agent_settings_response(request: &Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let (model_profiles, default_chat_model_resolution, default_chat_model_revision) = match NolaStore::open_default()
        .and_then(|store| {
            let profiles = mobile_model_profile_catalog(&store)?;
            let (resolution, revision) = mobile_default_chat_model_projection(&store)?;
            Ok((profiles, resolution, revision))
        })
    {
        Ok(profiles) => profiles,
        Err(_) => {
            return mobile_encrypted_json_response(
                StatusCode(500),
                json!({"code":"model_catalog_unavailable","error":"Cannot read Host model catalog"}),
                &token,
            )
        }
    };
    let browser_status = native_status_value(NativeBridgeKind::Browser);
    let browser_tools = native_browser_tool_names_from_status(&browser_status);
    let messaging_worker = crate::messaging_channel_worker::config_projection()
        .unwrap_or_else(|error| json!({"enabled": false, "error": error}));
    let messaging_channels = crate::messaging_channel_worker::list_connections()
        .unwrap_or_else(|error| json!({"channels": [], "error": error}));
    let settings = match NolaStore::open_default().and_then(|store| load_agent_settings(&store)) {
        Ok(settings) => settings,
        Err(error) => return json_response(StatusCode(500), json!({"error":error})),
    };
    {
        // This status-only projection does not request any Skill capability.
        let computer_tools = crate::tools::tool_schemas_with_skills(
            &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(Vec::new())),
        )
        .into_iter()
        .filter_map(|schema| {
            schema
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .filter(|name| name.starts_with("computer_"))
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
        mobile_encrypted_json_response(
            StatusCode(200),
            json!({
                "skills": builtin_skills::merged_catalog(
                    &settings.builtin_skill_overrides,
                    &settings.uploaded_skills,
                ).unwrap_or_else(|_| settings.uploaded_skills.clone()),
                "uploaded_skills": settings.uploaded_skills,
                "uploaded_documents": crate::resource_access::data_source_api_projections(&settings.uploaded_documents),
                "mcp_servers": settings.mcp_servers.iter()
                    .filter_map(crate::resource_access::mcp_api_projection)
                    .collect::<Vec<_>>(),
                "enable_local_dev_tools": settings.enable_local_dev_tools,
                "network_policy": settings.network_policy,
                "permission_profile": settings.permission_profile,
                "custom_system_prompt": settings.custom_system_prompt,
                "computer_use": {
                    "feature_enabled": false,
                    "token_configured": false,
                    "connected": false,
                    "hosts": [],
                    "tools": computer_tools,
                    "transport": "agent-host",
                },
                "native_browser": {
                    "token_configured": browser_status.get("connected").and_then(Value::as_bool).unwrap_or(false),
                    "connected": browser_status.get("connected").and_then(Value::as_bool).unwrap_or(false),
                    "structured_search": native_browser_supports_structured_search(&browser_status),
                    "hosts": browser_status.get("hosts").cloned().unwrap_or_else(|| json!([])),
                    "tools": browser_tools,
                    "pending_commands": browser_status.get("pending_commands").and_then(Value::as_u64).unwrap_or(0),
                    "in_flight_commands": browser_status.get("in_flight_commands").and_then(Value::as_u64).unwrap_or(0),
                },
                "host_identity": {
                    "id": host_identity(),
                    "permission_protocol_version": HOST_PERMISSION_PROTOCOL_VERSION,
                },
                "execution_backends": crate::external_executor::discover_execution_backends(),
                "model_profiles": model_profiles,
                "default_chat_model_resolution": default_chat_model_resolution,
                "default_chat_model_revision": default_chat_model_revision,
                "execution_backend": settings.execution_backend,
                "messaging": {
                    "worker": messaging_worker,
                    "channels": messaging_channels.get("channels").cloned().unwrap_or_else(|| json!([])),
                    "error": messaging_channels.get("error").cloned().unwrap_or(Value::Null),
                    "management_path": "/settings#channels",
                },
                "localFirst": true,
                "backend": "cli-rust",
            }),
            &token,
        )
    }
}

fn native_browser_tool_names_from_status(status: &Value) -> Vec<String> {
    let mut actions = status
        .get("hosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|host| host.get("capabilities").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|action| native_browser_action_allowed(action))
        .filter(|action| !native_browser_action_is_internal(action))
        .map(|action| format!("chrome_native_{action}"))
        .collect::<Vec<_>>();
    actions.sort();
    actions.dedup();
    actions
}

fn native_browser_supports_structured_search(status: &Value) -> bool {
    status
        .get("hosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|host| host.get("capabilities").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .any(|capability| capability.trim().eq_ignore_ascii_case("search_results"))
}

fn mobile_encrypted_json_response(
    status: StatusCode,
    value: Value,
    token: &str,
) -> Response<Box<dyn Read + Send>> {
    match nola_core::mobile_crypto::encrypt_mobile_json(&value, token).and_then(|envelope| {
        serde_json::to_value(envelope)
            .map_err(|_| "failed to encode encrypted mobile response".to_string())
    }) {
        Ok(envelope) => json_response(status, envelope),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn mobile_attachment_parse_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let filename = body
        .get("filename")
        .and_then(Value::as_str)
        .unwrap_or("attachment")
        .trim();
    let filename = if filename.is_empty() {
        "attachment"
    } else {
        filename
    };
    let mime_type = body
        .get("mime_type")
        .or_else(|| body.get("mimeType"))
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .trim();
    let encoded = body
        .get("data_base64")
        .or_else(|| body.get("dataBase64"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if encoded.is_empty() {
        return json_response(StatusCode(400), json!({"error": "data_base64 required"}));
    }
    let bytes = match general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => bytes,
        Err(_) => return json_response(StatusCode(400), json!({"error": "invalid data_base64"})),
    };
    if bytes.len() > MOBILE_ATTACHMENT_MAX_BYTES {
        return json_response(
            StatusCode(413),
            json!({"error": format!("attachment too large; max {} bytes", MOBILE_ATTACHMENT_MAX_BYTES)}),
        );
    }
    let parsed = match extract_mobile_attachment_text(&bytes, filename, mime_type) {
        Ok(parsed) => parsed,
        Err(error) => {
            return json_response(
                StatusCode(422),
                json!({
                    "error": error.message,
                    "code": error.code,
                    "format": error.format,
                    "recoverable": true,
                    "recovery": "Extract text locally with the platform document parser or OCR, then attach the extracted text."
                }),
            )
        }
    };
    let text = truncate_chars(parsed.text.trim(), MOBILE_ATTACHMENT_MAX_CHARS);
    if text.is_empty() {
        return json_response(
            StatusCode(422),
            json!({"error": "no readable text extracted"}),
        );
    }
    mobile_encrypted_json_response(
        StatusCode(200),
        json!({
            "filename": filename,
            "parser": parsed.parser,
            "text": text,
            "truncated": parsed.text.chars().count() > MOBILE_ATTACHMENT_MAX_CHARS,
            "localFirst": true,
            "backend": "cli-rust",
        }),
        &token,
    )
}

struct MobileAttachmentText {
    parser: &'static str,
    text: String,
}

#[derive(Debug)]
struct MobileAttachmentTextError {
    code: &'static str,
    format: &'static str,
    message: &'static str,
}

/// Decode only formats for which this fallback can produce trustworthy text.
/// Binary documents must go through a native parser or OCR; interpreting their
/// container bytes as Latin-1 produces convincing-looking garbage for models.
fn extract_mobile_attachment_text(
    bytes: &[u8],
    filename: &str,
    mime_type: &str,
) -> Result<MobileAttachmentText, MobileAttachmentTextError> {
    let lower_name = filename.to_ascii_lowercase();
    let extension = lower_name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or("");
    if let Some(format) = binary_attachment_format(bytes, extension, mime_type) {
        return Err(MobileAttachmentTextError {
            code: "unsupported_binary_format",
            format,
            message: "This attachment requires document extraction or OCR; raw binary bytes were not used as text.",
        });
    }
    let decoded = decode_attachment_text(bytes).ok_or(MobileAttachmentTextError {
        code: "unreadable_text",
        format: "text",
        message: "No trustworthy text encoding was detected.",
    })?;
    let trimmed_lower = decoded.trim_start().to_ascii_lowercase();
    let is_html = mime_type.eq_ignore_ascii_case("text/html")
        || matches!(extension, "html" | "htm")
        || trimmed_lower.starts_with("<!doctype html")
        || trimmed_lower.starts_with("<html");
    let text = if is_html {
        clean_html_text(&decoded)
    } else {
        decoded
    };
    if text.trim().is_empty() {
        return Err(MobileAttachmentTextError {
            code: "no_readable_text",
            format: "text",
            message: "No readable text was extracted.",
        });
    }
    Ok(MobileAttachmentText {
        parser: if is_html {
            "cli_html_text"
        } else {
            "cli_plain_text"
        },
        text,
    })
}

fn binary_attachment_format(
    bytes: &[u8],
    extension: &str,
    mime_type: &str,
) -> Option<&'static str> {
    if bytes.starts_with(b"%PDF-")
        || extension == "pdf"
        || mime_type.eq_ignore_ascii_case("application/pdf")
    {
        return Some("pdf");
    }
    if bytes.starts_with(b"PK\x03\x04")
        || matches!(extension, "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp")
    {
        return Some("archive_document");
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) || matches!(extension, "doc" | "xls" | "ppt") {
        return Some("legacy_office");
    }
    if mime_type.starts_with("image/")
        || matches!(
            extension,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "tif" | "tiff" | "bmp"
        )
    {
        return Some("image");
    }
    None
}

fn decode_attachment_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let decoded = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8(bytes[3..].to_vec()).ok()?
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        decode_utf16_bytes(&bytes[2..], true)?
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        decode_utf16_bytes(&bytes[2..], false)?
    } else {
        String::from_utf8(bytes.to_vec()).ok()?
    };
    let meaningful = decoded
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<Vec<_>>();
    if meaningful.is_empty() {
        return None;
    }
    let control_count = meaningful
        .iter()
        .filter(|character| character.is_control())
        .count();
    if control_count.saturating_mul(100) > meaningful.len() {
        return None;
    }
    Some(decoded)
}

fn decode_utf16_bytes(bytes: &[u8], little_endian: bool) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
include!("native_bridge_tests.rs");

fn mobile_chat_agent_stream_response(
    request: &mut Request,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    chat_agent_stream_response_body(body, config, Some(token))
}

fn mobile_chat_agent_trace_replay_response(
    request: &mut Request,
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(conversation_id) = trimmed_string_field(&body, "conversationId")
    else {
        return mobile_encrypted_json_response(
            StatusCode(422),
            json!({"error": "conversationId is required"}),
            &token,
        );
    };
    let trace_id =
        trimmed_string_field(&body, "traceId").or_else(|| trimmed_string_field(&body, "trace_id"));
    let after_sequence = body
        .get("afterSequence")
        .or_else(|| body.get("after_sequence"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            return mobile_encrypted_json_response(StatusCode(500), json!({"error": error}), &token)
        }
    };
    let runs = match store.trace_runs_for_session(&conversation_id) {
        Ok(runs) => runs,
        Err(error) => {
            return mobile_encrypted_json_response(StatusCode(500), json!({"error": error}), &token)
        }
    };
    let run = if let Some(trace_id) = trace_id.as_deref() {
        runs.into_iter().find(|run| run.trace_id == trace_id)
    } else {
        runs.into_iter().max_by(|left, right| {
            left.started_at
                .cmp(&right.started_at)
                .then_with(|| left.trace_id.cmp(&right.trace_id))
        })
    };
    let Some(run) = run else {
        return mobile_encrypted_json_response(
            StatusCode(404),
            json!({"error": "trace not found"}),
            &token,
        );
    };
    let mut events = match store.trace_event_envelopes_after(&run.trace_id, after_sequence) {
        Ok(events) => events,
        Err(error) => {
            return mobile_encrypted_json_response(StatusCode(500), json!({"error": error}), &token)
        }
    };
    if run.status == "interrupted" {
        match interrupted_run_terminal_envelope(&store, &run, after_sequence) {
            Ok(Some(event)) => events.push(event),
            Ok(None) => {}
            Err(error) => {
                return mobile_encrypted_json_response(
                    StatusCode(500),
                    json!({"error": error}),
                    &token,
                )
            }
        }
    }
    let next_sequence = events
        .last()
        .and_then(|event| event.get("trace"))
        .and_then(|trace| trace.get("sequence"))
        .and_then(Value::as_u64)
        .unwrap_or(after_sequence);
    // Trace history carries audit events only. Journal and terminal recovery
    // use their own exact canonical Run reads, never a trace-derived identity.
    mobile_encrypted_json_response(
        StatusCode(200),
        json!({
            "conversationId": conversation_id,
            "traceId": run.trace_id,
            "runId": run.run_id,
            "messageId": run.message_id,
            "status": run.status,
            "lastSequence": if run.status == "interrupted" {
                run.last_sequence.saturating_add(1)
            } else {
                run.last_sequence
            },
            "nextSequence": next_sequence,
            "terminalReason": run.terminal_reason,
            "events": events,
        }),
        &token,
    )
}

fn interrupted_run_terminal_envelope(
    store: &NolaStore,
    run: &crate::store::AgentTraceRunRecord,
    after_sequence: u64,
) -> Result<Option<Value>, String> {
    let terminal_sequence = run.last_sequence.saturating_add(1);
    if after_sequence >= terminal_sequence {
        return Ok(None);
    }
    let Some(mut envelope) = store
        .trace_event_envelopes_after(&run.trace_id, 0)?
        .into_iter()
        .last()
    else {
        return Ok(None);
    };
    let Some(object) = envelope.as_object_mut() else {
        return Ok(None);
    };
    let Some(trace) = object.get_mut("trace").and_then(Value::as_object_mut) else {
        return Ok(None);
    };
    let event_id = format!("{}:server-restarted", run.trace_id);
    let emitted_at = run
        .finished_at
        .as_deref()
        .unwrap_or(run.started_at.as_str())
        .to_string();
    trace.insert("event_id".to_string(), Value::String(event_id.clone()));
    trace.insert("sequence".to_string(), Value::from(terminal_sequence));
    trace.insert("emitted_at".to_string(), Value::String(emitted_at));
    trace.insert("kind".to_string(), Value::String("run.failed".to_string()));
    trace.insert("status".to_string(), Value::String("failed".to_string()));
    trace.insert(
        "visibility".to_string(),
        Value::String("summary".to_string()),
    );
    trace.remove("span_id");
    trace.remove("parent_span_id");
    trace.insert(
        "payload".to_string(),
        json!({
            "reason": "server_restarted",
            "code": "server_restarted",
            "detail": "The Nola Server restarted before this run completed."
        }),
    );
    object.insert("type".to_string(), Value::String("error".to_string()));
    object.insert("event_id".to_string(), Value::String(event_id));
    object.insert("sequence".to_string(), Value::from(terminal_sequence));
    object.insert(
        "message".to_string(),
        Value::String("The Nola Server restarted before this run completed.".to_string()),
    );
    object.insert("error".to_string(), Value::Bool(true));
    Ok(Some(envelope))
}

fn mobile_host_permission_decision_response(
    request: &mut Request,
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let decision = match serde_json::from_value::<HostPermissionResponse>(body) {
        Ok(value) => value,
        Err(_) => {
            return json_response(
                StatusCode(400),
                json!({"error": "invalid host permission decision"}),
            )
        }
    };
    if let Err(error) = submit_host_permission_decision(decision.clone()) {
        return json_response(StatusCode(409), json!({"error": error}));
    }
    mobile_encrypted_json_response(
        StatusCode(200),
        json!({"accepted": true, "nonce": decision.nonce}),
        &token,
    )
}

fn mobile_chat_agent_cancel_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (status, payload) = chat_agent_cancel_result(&body);
    mobile_encrypted_json_response(status, payload, &token)
}

fn mobile_chat_agent_task_cancel_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (status, payload) = chat_agent_task_cancel_result(&body);
    mobile_encrypted_json_response(status, payload, &token)
}

fn mobile_chat_agent_task_message_response(
    request: &mut Request,
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (status, payload) = chat_agent_task_message_result(&body);
    mobile_encrypted_json_response(status, payload, &token)
}

fn mobile_chat_agent_task_events_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(conversation_id) = trimmed_string_field(&body, "conversationId")
    else {
        return json_response(
            StatusCode(422),
            json!({"error": "conversationId is required"}),
        );
    };
    let mut cursor = body
        .get("afterCursor")
        .or_else(|| body.get("after_cursor"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let timeout_ms = body
        .get("timeoutMs")
        .or_else(|| body.get("timeout_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(25_000)
        .clamp(1_000, 30_000);
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let started = Instant::now();
        let mut last_projection_revision: Option<u64> = None;
        let mut first = true;
        while started.elapsed() < StdDuration::from_millis(timeout_ms) {
            let events = NolaStore::open_default()
                .and_then(|store| {
                    store.runtime_task_events_after_session_cursor(&conversation_id, cursor, 128)
                })
                .unwrap_or_default();
            if let Some(last) = events.last() {
                cursor = cursor.max(last.id);
            }
            let task_projection = crate::session::load_session_task_projection(&conversation_id)
                .ok()
                .flatten();
            let projection_revision = task_projection.as_ref().map(|value| value.revision);
            let projection_changed = projection_revision != last_projection_revision;
            if first || !events.is_empty() || projection_changed {
                let event_values = events
                    .iter()
                    .map(|event| {
                        json!({
                            "cursor": event.id,
                            "taskId": event.task_id,
                            "kind": event.kind,
                            "status": event.status,
                            "summary": event.summary,
                            "detail": serde_json::from_str::<Value>(&event.detail_json)
                                .unwrap_or_else(|_| json!({})),
                            "createdAtMs": event.created_at_ms,
                        })
                    })
                    .collect::<Vec<_>>();
                if tx
                    .send(mobile_sse_event(
                        json!({
                            "type": "task_projection",
                            "conversationId": conversation_id,
                            "cursor": cursor,
                            "events": event_values,
                            "taskProjection": task_projection,
                        }),
                        Some(&token),
                    ))
                    .is_err()
                {
                    return;
                }
                first = false;
                last_projection_revision = projection_revision;
            }
            thread::sleep(StdDuration::from_millis(250));
        }
        let _ = tx.send(mobile_sse_event(
            json!({
                "type": "task_stream_checkpoint",
                "conversationId": conversation_id,
                "cursor": cursor,
            }),
            Some(&token),
        ));
    });
    sse_response(rx)
}

fn mobile_device_tool_result_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let result = match serde_json::from_value::<nola_core::device_bridge::DeviceToolResult>(body) {
        Ok(value) => value,
        Err(_) => {
            return json_response(
                StatusCode(400),
                json!({"error": "invalid device tool result"}),
            )
        }
    };
    let challenge = match HOST_DEVICE_TOOL_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|challenges| challenges.get(&result.nonce).cloned())
    {
        Some(value) => value,
        None => {
            return json_response(
                StatusCode(409),
                json!({"error": "device tool challenge is missing or already resolved"}),
            )
        }
    };
    if let Err(error) = challenge.validate_at(device_bridge_now_ms()) {
        return json_response(StatusCode(409), json!({"error": error}));
    }
    if let Err(error) = nola_core::device_bridge::validate_device_tool_result(
        &challenge,
        &result,
        unix_time_seconds(),
    ) {
        return json_response(StatusCode(409), json!({"error": error}));
    }
    let sender = HOST_DEVICE_TOOL_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|mut waiters| waiters.remove(&result.nonce));
    let Some(sender) = sender else {
        return json_response(
            StatusCode(409),
            json!({"error": "device tool execution is no longer waiting"}),
        );
    };
    if let Ok(mut challenges) = HOST_DEVICE_TOOL_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        challenges.remove(&result.nonce);
    }
    if sender.send(result.clone()).is_err() {
        return json_response(
            StatusCode(409),
            json!({"error": "device tool execution has ended"}),
        );
    }
    mobile_encrypted_json_response(
        StatusCode(200),
        json!({"accepted": true, "nonce": result.nonce}),
        &token,
    )
}

fn web_host_permission_decision_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let decision = match serde_json::from_value::<HostPermissionResponse>(body) {
        Ok(value) => value,
        Err(_) => {
            return json_response(
                StatusCode(400),
                json!({"error": "invalid host permission decision"}),
            )
        }
    };
    if let Err(error) = submit_host_permission_decision(decision.clone()) {
        return json_response(StatusCode(409), json!({"error": error}));
    }
    json_response(
        StatusCode(200),
        json!({"accepted": true, "nonce": decision.nonce}),
    )
}

fn submit_host_permission_decision(decision: HostPermissionResponse) -> Result<(), String> {
    let resolution = match &decision.resolution {
        WireHostPermissionResolution::Authorization {
            decision: HostPermissionAuthorizationDecision::AllowOnce,
            ..
        } => ToolPermissionResolution::Allow,
        WireHostPermissionResolution::Authorization {
            decision: HostPermissionAuthorizationDecision::AllowConversation,
            ..
        } => ToolPermissionResolution::AllowConversation,
        WireHostPermissionResolution::Authorization {
            decision: HostPermissionAuthorizationDecision::Deny,
            ..
        } => ToolPermissionResolution::Deny,
        WireHostPermissionResolution::Intervention {
            intervention: HostPermissionIntervention::ProvideInstruction { instruction },
        } => ToolPermissionResolution::Custom(instruction.clone()),
        WireHostPermissionResolution::Intervention {
            intervention: HostPermissionIntervention::ReturnToComposer,
        } => ToolPermissionResolution::ReturnToComposer,
    };
    submit_host_permission_decision_with_resolution(decision, resolution)
}

fn submit_host_permission_decision_with_resolution(
    decision: HostPermissionResponse,
    resolution: ToolPermissionResolution,
) -> Result<(), String> {
    // This acknowledges transport delivery only. Canonical completion still
    // arrives through Application's committed projection. Retain the exact
    // accepted response until challenge expiry so a lost HTTP 200 can be
    // retried without delivering a second decision or accepting a different one.
    static ACCEPTED_RESPONSES: OnceLock<Mutex<HashMap<String, (HostPermissionChallenge, Value)>>> =
        OnceLock::new();
    let mut accepted = ACCEPTED_RESPONSES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "host permission acknowledgements unavailable".to_string())?;
    let now = unix_time_seconds();
    accepted.retain(|_, (challenge, _)| challenge.expires_at > now);
    let wire = serde_json::to_value(&decision).map_err(|error| error.to_string())?;
    if let Some((challenge, original)) = accepted.get(&decision.nonce) {
        validate_host_permission_decision(challenge, &decision, now)?;
        return if original == &wire {
            Ok(())
        } else {
            Err("host permission request already accepted a different response".into())
        };
    }
    if accepted.len() >= 4096 {
        return Err("host permission acknowledgement capacity reached; retry after expiry".into());
    }
    if let Some(challenge) = resolve_delegated_permission(&decision)? {
        accepted.insert(decision.nonce.clone(), (challenge, wire));
        return Ok(());
    }
    let challenge = HOST_PERMISSION_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "host permission state unavailable".to_string())?
        .get(&decision.nonce)
        .cloned()
        .ok_or_else(|| "host permission challenge is missing or already resolved".to_string())?;
    validate_host_permission_decision(&challenge, &decision, unix_time_seconds())?;
    let sender = HOST_PERMISSION_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "host permission wait state unavailable".to_string())?
        .remove(&decision.nonce)
        .ok_or_else(|| "host permission request is no longer waiting".to_string())?;
    if let Ok(mut challenges) = HOST_PERMISSION_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        challenges.remove(&decision.nonce);
    }
    sender
        .send(resolution)
        .map_err(|_| "host permission execution has ended".to_string())?;
    accepted.insert(decision.nonce.clone(), (challenge, wire));
    Ok(())
}

fn mobile_tokens() -> &'static Mutex<HashMap<String, Instant>> {
    MOBILE_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mobile_bearer_token(request: &Request) -> Option<String> {
    if let Some(token) = mobile_proof_token(request) {
        return Some(token);
    }
    let token = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))?
        .value
        .as_str()
        .trim()
        .strip_prefix("Bearer ")?
        .trim()
        .to_string();
    if token.is_empty() {
        return None;
    }
    let mut tokens = mobile_tokens().lock().unwrap();
    mobile_cleanup_locked(&mut tokens);
    if tokens.contains_key(&token) {
        tokens.insert(token.clone(), Instant::now());
        Some(token)
    } else {
        None
    }
}

fn mobile_proof_token(request: &Request) -> Option<String> {
    let timestamp = request_header(request, "X-Nola-Mobile-Timestamp")?
        .parse::<u64>()
        .ok()?;
    let nonce = request_header(request, "X-Nola-Mobile-Nonce")?;
    let proof = request_header(request, "X-Nola-Mobile-Auth")?;
    let body_sha256 = request_header(request, "X-Nola-Mobile-Body-SHA256")?;
    if nonce.len() < 16
        || nonce.len() > 128
        || proof.len() < 32
        || proof.len() > 128
        || body_sha256.len() != 64
        || !body_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let now = unix_time_seconds();
    if now.abs_diff(timestamp) > 60 {
        return None;
    }
    {
        let mut nonces = MOBILE_AUTH_NONCES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .ok()?;
        nonces.retain(|_, seen| seen.elapsed() <= StdDuration::from_secs(120));
        if nonces.contains_key(nonce) {
            return None;
        }
    }
    let method = request.method().to_string();
    let path = request.url();
    let mut tokens = mobile_tokens().lock().ok()?;
    mobile_cleanup_locked(&mut tokens);
    let matched = tokens
        .keys()
        .find(|token| {
            nola_core::mobile_crypto::verify_mobile_auth_proof(
                token,
                &method,
                path,
                timestamp,
                nonce,
                body_sha256,
                proof,
            )
        })?
        .clone();
    tokens.insert(matched.clone(), Instant::now());
    drop(tokens);
    let mut nonces = MOBILE_AUTH_NONCES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    if nonces.insert(nonce.to_string(), Instant::now()).is_some() {
        return None;
    }
    Some(matched)
}

fn request_header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    let value = request
        .headers()
        .iter()
        .find(|header| header.field.to_string().eq_ignore_ascii_case(name))?
        .value
        .as_str()
        .trim();
    (!value.is_empty()).then_some(value)
}

fn request_language(request: &Request) -> crate::i18n::Language {
    preferred_request_language(
        request_header(request, "Cookie"),
        request_header(request, "Accept-Language"),
    )
    .unwrap_or_else(crate::i18n::current_language)
}

fn preferred_request_language(
    cookie: Option<&str>,
    accept_language: Option<&str>,
) -> Option<crate::i18n::Language> {
    cookie
        .and_then(parse_ui_locale_cookie)
        .or_else(|| accept_language.and_then(parse_accept_language))
}

fn parse_ui_locale_cookie(cookie: &str) -> Option<crate::i18n::Language> {
    use crate::i18n::Language;
    let mut values = cookie.split(';').filter_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "nola_ui_locale").then_some(value)
    });
    let value = values.next()?;
    // Ambiguous duplicate cookies must not pick a preference by header order.
    if values.next().is_some() {
        return None;
    }
    // Only the Svelte preference enum is accepted. Never decode or echo cookie text.
    match value {
        "zh-Hans" => Some(Language::ZhHans),
        "en" => Some(Language::En),
        "de" => Some(Language::De),
        "es" => Some(Language::Es),
        "fr" => Some(Language::Fr),
        "ja" => Some(Language::Ja),
        "ru" => Some(Language::Ru),
        _ => None,
    }
}

fn parse_accept_language(value: &str) -> Option<crate::i18n::Language> {
    let mut best = None;
    let mut best_quality = -1.0_f32;
    for item in value.split(',') {
        let mut parts = item.trim().split(';');
        let tag = parts.next()?.trim();
        if tag == "*" || tag.is_empty() {
            continue;
        }
        let quality = parts
            .find_map(|part| part.trim().strip_prefix("q="))
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.0);
        if quality <= 0.0 || quality <= best_quality {
            continue;
        }
        if let Some(language) = crate::i18n::parse_language_tag(tag) {
            best = Some(language);
            best_quality = quality;
        }
    }
    best
}

fn mobile_cleanup_locked(tokens: &mut HashMap<String, Instant>) {
    let now = Instant::now();
    let configured = WEB_HOST_ACCESS_TOKEN.get();
    tokens.retain(|token, issued_at| {
        configured.is_some_and(|configured| configured == token)
            || now.duration_since(*issued_at).as_secs() <= MOBILE_TOKEN_TTL_SEC
    });
    if let Some(configured) = configured {
        tokens.entry(configured.clone()).or_insert(now);
    }
}

fn native_status_response(kind: NativeBridgeKind) -> Response<Box<dyn Read + Send>> {
    json_response(StatusCode(200), native_status_value(kind))
}

fn native_status_value(kind: NativeBridgeKind) -> Value {
    let mut store = native_bridge_store().lock().unwrap();
    let queue = native_queue_mut(&mut store, kind);
    native_cleanup_locked(queue);
    let now = Instant::now();
    let hosts = native_connected_hosts_locked(queue, now);
    let pending_commands = queue
        .commands
        .values()
        .filter(|command| command.status == NativeCommandStatus::Pending)
        .count();
    let in_flight_commands = queue
        .commands
        .values()
        .filter(|command| command.status == NativeCommandStatus::InFlight)
        .count();
    json!({
        "connected": !hosts.is_empty(),
        "hosts": hosts,
        "pending_commands": pending_commands,
        "in_flight_commands": in_flight_commands,
        "configured": true,
        "localFirst": true,
        "backend": "cli-rust",
    })
}

fn native_browser_command_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let action = match native_required_string(&body, &["action"]) {
        Ok(value) => value.to_ascii_lowercase(),
        Err(err) => return json_response(StatusCode(400), json!({"ok": false, "error": err})),
    };
    if !native_browser_action_allowed(&action) {
        return json_response(
            StatusCode(400),
            json!({"ok": false, "error": format!("Unsupported native browser action: {action}")}),
        );
    }
    let args = native_object_field(&body, &["args", "params"]);
    let preferred_host_id = match native_optional_host_id(&body, &["host_id", "hostId"]) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"ok": false, "error": err})),
    };
    let timeout_sec = native_timeout_sec(&body, 30.0);
    let result = native_submit_command(
        NativeCommand {
            id: new_id("nb"),
            kind: NativeBridgeKind::Browser,
            action: action.clone(),
            args,
            preferred_host_id,
            created_at: Instant::now(),
            status: NativeCommandStatus::Pending,
            host_id: None,
            host_instance_id: None,
            host_manifest_id: None,
            host_manifest_expires_at_ms: None,
            claimed_at: None,
            result: None,
        },
        timeout_sec,
    );
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    json_response(
        StatusCode(200),
        json!({"ok": ok, "action": action, "result": result}),
    )
}

fn native_unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn native_required_u64(body: &Value, keys: &[&str]) -> Result<u64, String> {
    for key in keys {
        if let Some(value) = body.get(*key) {
            if let Some(number) = value.as_u64() {
                return Ok(number);
            }
            if let Some(number) = value
                .as_str()
                .and_then(|raw| raw.trim().parse::<u64>().ok())
            {
                return Ok(number);
            }
            return Err(format!("{key} must be an unsigned integer"));
        }
    }
    Err(format!("{} required", keys[0]))
}

fn native_host_manifest_from_poll(body: &Value) -> Result<NativeHost, String> {
    let mut capabilities = native_string_array_field(body, &["capabilities", "caps"])
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    capabilities.sort();
    if capabilities.is_empty() {
        return Err("capabilities must contain at least one explicit capability".to_string());
    }
    let manifest_id = native_required_string(body, &["manifest_id", "manifestId"])?;
    let instance_id = native_required_string(body, &["instance_id", "instanceId"])?;
    let manifest_version = native_required_u64(body, &["manifest_version", "manifestVersion"])?;
    let issued_at_ms = native_required_u64(body, &["issued_at_ms", "issuedAtMs"])?;
    let expires_at_ms = native_required_u64(body, &["expires_at_ms", "expiresAtMs"])?;
    let now_ms = native_unix_time_ms();
    if manifest_version != 1 {
        return Err("unsupported native host manifest version".to_string());
    }
    if issued_at_ms == 0
        || expires_at_ms <= issued_at_ms
        || expires_at_ms.saturating_sub(issued_at_ms) > 120_000
        || issued_at_ms > now_ms.saturating_add(30_000)
        || expires_at_ms <= now_ms
    {
        return Err("native host manifest is stale or has invalid freshness".to_string());
    }
    Ok(NativeHost {
        last_seen: Instant::now(),
        capabilities,
        manifest_id,
        manifest_version,
        instance_id,
        issued_at_ms,
        expires_at_ms,
    })
}

fn native_host_poll_response(
    request: &mut Request,
    kind: NativeBridgeKind,
) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let host_id = match native_required_string(&body, &["host_id", "hostId"]) {
        Ok(value) if native_host_id_is_valid(&value) => value,
        Ok(_) => {
            return json_response(
                StatusCode(400),
                json!({"error": "host_id contains unsupported characters"}),
            )
        }
        Err(err) => return json_response(StatusCode(400), json!({"error": err})),
    };
    let host = match native_host_manifest_from_poll(&body) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"error": err})),
    };
    let wait_sec = native_wait_sec(&body, 25.0);
    let deadline = Instant::now() + StdDuration::from_millis((wait_sec * 1000.0) as u64);

    loop {
        if let Some(command) = native_claim_command(kind, &host_id, &host) {
            return json_response(
                StatusCode(200),
                json!({"type": "command", "command": command}),
            );
        }
        if Instant::now() >= deadline {
            return json_response(StatusCode(200), json!({"type": "noop", "retry_ms": 500}));
        }
        thread::sleep(StdDuration::from_millis(100));
    }
}

fn native_host_result_response(
    request: &mut Request,
    kind: NativeBridgeKind,
) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let command_id = match native_required_string(&body, &["command_id", "commandId"]) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"error": err})),
    };
    let host_id = match native_required_string(&body, &["host_id", "hostId"]) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"error": err})),
    };
    let instance_id = match native_required_string(&body, &["instance_id", "instanceId"]) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"error": err})),
    };
    let manifest_id = match native_required_string(&body, &["manifest_id", "manifestId"]) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"error": err})),
    };
    let result = native_object_field(&body, &["result"]);
    let mut store = native_bridge_store().lock().unwrap();
    let queue = native_queue_mut(&mut store, kind);
    let Some(command) = queue.commands.get_mut(&command_id) else {
        return json_response(StatusCode(404), json!({"accepted": false}));
    };
    if !native_result_identity_matches(
        command,
        &host_id,
        &instance_id,
        &manifest_id,
        native_unix_time_ms(),
    ) {
        return json_response(
            StatusCode(409),
            json!({"accepted": false, "error": "native command claim identity mismatch"}),
        );
    }
    command.status = NativeCommandStatus::Done;
    command.result = Some(result);
    json_response(StatusCode(200), json!({"accepted": true}))
}

fn native_result_identity_matches(
    command: &NativeCommand,
    host_id: &str,
    instance_id: &str,
    manifest_id: &str,
    now_ms: u64,
) -> bool {
    command.status == NativeCommandStatus::InFlight
        && command.host_id.as_deref() == Some(host_id)
        && command.host_instance_id.as_deref() == Some(instance_id)
        && command.host_manifest_id.as_deref() == Some(manifest_id)
        && command
            .host_manifest_expires_at_ms
            .map(|expires_at_ms| expires_at_ms > now_ms)
            .unwrap_or(false)
}

fn native_submit_command(command: NativeCommand, timeout_sec: f64) -> Value {
    let timeout = timeout_sec.clamp(1.0, 120.0);
    let command_id = command.id.clone();
    let kind = command.kind;
    {
        let wait_deadline = Instant::now() + StdDuration::from_secs_f64(timeout.min(3.0));
        loop {
            let has_matching_host = {
                let mut store = native_bridge_store().lock().unwrap();
                let queue = native_queue_mut(&mut store, kind);
                native_cleanup_locked(queue);
                native_has_matching_host_locked(queue, &command)
            };
            if has_matching_host || Instant::now() >= wait_deadline {
                break;
            }
            thread::sleep(StdDuration::from_millis(50));
        }
        let mut store = native_bridge_store().lock().unwrap();
        let queue = native_queue_mut(&mut store, kind);
        native_cleanup_locked(queue);
        if !native_has_matching_host_locked(queue, &command) {
            return json!({"ok": false, "error": native_no_matching_host_error_locked(queue, &command)});
        }
        queue.pending.push_back(command_id.clone());
        queue.commands.insert(command_id.clone(), command);
    }

    let deadline = Instant::now() + StdDuration::from_secs_f64(timeout);
    loop {
        {
            let mut store = native_bridge_store().lock().unwrap();
            let queue = native_queue_mut(&mut store, kind);
            native_cleanup_locked(queue);
            if let Some(command) = queue.commands.get(&command_id) {
                if command.status == NativeCommandStatus::Done {
                    let result = command
                        .result
                        .clone()
                        .unwrap_or_else(|| json!({"ok": false, "error": "Native command finished without a result."}));
                    queue.commands.remove(&command_id);
                    queue.pending.retain(|id| id != &command_id);
                    return result;
                }
            } else {
                return json!({"ok": false, "error": "Native command disappeared before completion."});
            }
        }
        if Instant::now() >= deadline {
            let mut store = native_bridge_store().lock().unwrap();
            let queue = native_queue_mut(&mut store, kind);
            queue.commands.remove(&command_id);
            queue.pending.retain(|id| id != &command_id);
            return json!({"ok": false, "error": format!("Native {} command timed out after {timeout}s.", native_kind_label(kind))});
        }
        thread::sleep(StdDuration::from_millis(50));
    }
}

fn native_claim_command(
    kind: NativeBridgeKind,
    host_id: &str,
    advertised_host: &NativeHost,
) -> Option<Value> {
    let mut store = native_bridge_store().lock().unwrap();
    let queue = native_queue_mut(&mut store, kind);
    native_cleanup_locked(queue);
    let now = Instant::now();
    let mut current_host = advertised_host.clone();
    current_host.last_seen = now;
    queue
        .hosts
        .insert(host_id.to_string(), current_host.clone());
    let pending_count = queue.pending.len();
    for _ in 0..pending_count {
        let Some(command_id) = queue.pending.pop_front() else {
            break;
        };
        let Some(command) = queue.commands.get_mut(&command_id) else {
            continue;
        };
        if command.status != NativeCommandStatus::Pending {
            continue;
        }
        if !native_host_can_claim_command(host_id, &current_host, command) {
            queue.pending.push_back(command_id);
            continue;
        }
        command.status = NativeCommandStatus::InFlight;
        command.host_id = Some(host_id.to_string());
        command.host_instance_id = Some(current_host.instance_id.clone());
        command.host_manifest_id = Some(current_host.manifest_id.clone());
        command.host_manifest_expires_at_ms = Some(current_host.expires_at_ms);
        command.claimed_at = Some(now);
        return Some(native_command_payload(command));
    }
    None
}

fn native_bridge_store() -> &'static Mutex<NativeBridgeStore> {
    NATIVE_BRIDGES.get_or_init(|| Mutex::new(NativeBridgeStore::default()))
}

fn native_queue_mut(
    store: &mut NativeBridgeStore,
    kind: NativeBridgeKind,
) -> &mut NativeBridgeQueue {
    match kind {
        NativeBridgeKind::Browser => &mut store.browser,
    }
}

fn native_cleanup_locked(queue: &mut NativeBridgeQueue) {
    let now = Instant::now();
    let now_ms = native_unix_time_ms();
    queue.hosts.retain(|_, host| {
        now.duration_since(host.last_seen).as_secs() <= NATIVE_HOST_STALE_SEC
            && host.expires_at_ms > now_ms
    });
    let mut requeue = Vec::new();
    queue.commands.retain(|id, command| {
        if command.status == NativeCommandStatus::InFlight {
            if command
                .claimed_at
                .map(|claimed| now.duration_since(claimed).as_secs() > NATIVE_COMMAND_STALE_SEC)
                .unwrap_or(false)
            {
                command.status = NativeCommandStatus::Pending;
                command.host_id = None;
                command.host_instance_id = None;
                command.host_manifest_id = None;
                command.host_manifest_expires_at_ms = None;
                command.claimed_at = None;
                requeue.push(id.clone());
            }
            return true;
        }
        now.duration_since(command.created_at).as_secs() <= NATIVE_COMMAND_STALE_SEC * 2
    });
    for id in requeue.into_iter().rev() {
        if !queue.pending.iter().any(|pending| pending == &id) {
            queue.pending.push_front(id);
        }
    }
    queue.pending.retain(|id| {
        queue
            .commands
            .get(id)
            .map(|command| command.status == NativeCommandStatus::Pending)
            .unwrap_or(false)
    });
}

fn native_connected_hosts_locked(queue: &NativeBridgeQueue, now: Instant) -> Vec<Value> {
    let mut hosts = queue
        .hosts
        .iter()
        .map(|(host_id, host)| {
            json!({
                "host_id": host_id,
                "last_seen_age_sec": (now.duration_since(host.last_seen).as_millis() as f64) / 1000.0,
                "capabilities": host.capabilities,
                "manifest_id": host.manifest_id,
                "manifest_version": host.manifest_version,
                "instance_id": host.instance_id,
                "issued_at_ms": host.issued_at_ms,
                "expires_at_ms": host.expires_at_ms,
            })
        })
        .collect::<Vec<_>>();
    hosts.sort_by(|a, b| {
        let a_id = a.get("host_id").and_then(Value::as_str).unwrap_or("");
        let b_id = b.get("host_id").and_then(Value::as_str).unwrap_or("");
        a_id.cmp(b_id)
    });
    hosts
}

fn native_has_matching_host_locked(queue: &NativeBridgeQueue, command: &NativeCommand) -> bool {
    queue
        .hosts
        .iter()
        .any(|(host_id, host)| native_host_can_claim_command(host_id, host, command))
}

fn native_host_can_claim_command(
    host_id: &str,
    host: &NativeHost,
    command: &NativeCommand,
) -> bool {
    if let Some(preferred) = command.preferred_host_id.as_deref() {
        if !preferred.is_empty() && preferred != host_id {
            return false;
        }
    }
    if host.expires_at_ms <= native_unix_time_ms() || host.capabilities.is_empty() {
        return false;
    }
    let action = command.action.trim().to_ascii_lowercase();
    match command.kind {
        NativeBridgeKind::Browser => host
            .capabilities
            .iter()
            .any(|capability| capability.trim().eq_ignore_ascii_case(&action)),
    }
}

fn native_no_matching_host_error_locked(
    queue: &NativeBridgeQueue,
    command: &NativeCommand,
) -> String {
    if queue.hosts.is_empty() {
        return match command.kind {
            NativeBridgeKind::Browser => "No connected Chrome native browser host. Install/enable the Nola Chrome extension and native messaging host, then keep Chrome running.".to_string(),
        };
    }
    if let Some(preferred) = command
        .preferred_host_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let Some(host) = queue.hosts.get(preferred) else {
            return format!(
                "Preferred native {} host '{preferred}' is not connected.",
                native_kind_label(command.kind)
            );
        };
        if !native_host_can_claim_command(preferred, host, command) {
            return format!(
                "Preferred native {} host '{preferred}' does not support this command.",
                native_kind_label(command.kind)
            );
        }
    }
    format!(
        "No connected native {} host supports the requested command.",
        native_kind_label(command.kind)
    )
}

fn native_command_payload(command: &NativeCommand) -> Value {
    let mut value = json!({
        "id": command.id,
        "command_id": command.id,
        "commandId": command.id,
        "local_user_id": "cli-local-user",
        "action": command.action,
        "preferred_host_id": command.preferred_host_id.clone().unwrap_or_default(),
        "created_at": 0.0,
    });
    if let Some(map) = value.as_object_mut() {
        match command.kind {
            NativeBridgeKind::Browser => {
                map.insert("args".to_string(), command.args.clone());
            }
        }
    }
    value
}

fn native_browser_action_allowed(action: &str) -> bool {
    matches!(
        action,
        "accessibility_snapshot"
            | "assert"
            | "click"
            | "console_logs"
            | "dom_snapshot"
            | "drag"
            | "hover"
            | "inspect"
            | "list_pages"
            | "network_events"
            | "open_page"
            | "permission_resource"
            | "press_key"
            | "screenshot"
            | "search_results"
            | "scroll"
            | "select_option"
            | "set_checked"
            | "status"
            | "storage_state"
            | "switch_page"
            | "type_text"
            | "wait_for"
    )
}

fn native_browser_action_is_internal(action: &str) -> bool {
    matches!(action, "search_results" | "permission_resource")
}

fn native_required_string(body: &Value, keys: &[&str]) -> Result<String, String> {
    for key in keys {
        if let Some(value) = body.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                if native_simple_value_is_valid(trimmed, 240) {
                    return Ok(trimmed.to_string());
                }
                return Err(format!("{key} contains unsupported characters"));
            }
        }
    }
    Err(format!("{} required", keys[0]))
}

fn native_optional_host_id(body: &Value, keys: &[&str]) -> Result<Option<String>, String> {
    for key in keys {
        if let Some(value) = body.get(*key) {
            let host_id = value.as_str().unwrap_or("").trim();
            if host_id.is_empty() {
                continue;
            }
            if native_host_id_is_valid(host_id) {
                return Ok(Some(host_id.to_string()));
            }
            return Err(format!(
                "{key} must contain only A-Z, a-z, 0-9, dot, underscore, colon, or dash"
            ));
        }
    }
    Ok(None)
}

fn native_object_field(body: &Value, keys: &[&str]) -> Value {
    for key in keys {
        if let Some(value) = body.get(*key).filter(|value| value.is_object()) {
            return value.clone();
        }
    }
    json!({})
}

fn native_string_array_field(body: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(items) = body.get(*key).and_then(Value::as_array) {
            return items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
    }
    Vec::new()
}

fn native_timeout_sec(body: &Value, default: f64) -> f64 {
    native_timeout_sec_with_keys(body, &["timeout_sec", "timeoutSec"], default)
}

fn native_wait_sec(body: &Value, default: f64) -> f64 {
    for key in ["wait_sec", "waitSec", "timeout_sec", "timeoutSec"] {
        if let Some(value) = body.get(key) {
            if let Some(number) = value.as_f64() {
                return number.clamp(0.0, 30.0);
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.trim().parse::<f64>() {
                    return number.clamp(0.0, 30.0);
                }
            }
        }
    }
    default
}

fn native_timeout_sec_with_keys(body: &Value, keys: &[&str], default: f64) -> f64 {
    for key in keys {
        if let Some(value) = body.get(*key) {
            if let Some(number) = value.as_f64() {
                return number.clamp(1.0, 120.0);
            }
            if let Some(text) = value.as_str() {
                if let Ok(number) = text.trim().parse::<f64>() {
                    return number.clamp(1.0, 120.0);
                }
            }
        }
    }
    default
}

fn native_host_id_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
}

fn native_simple_value_is_valid(value: &str, max_len: usize) -> bool {
    !value.is_empty() && value.len() <= max_len && !value.chars().any(char::is_control)
}

fn native_kind_label(kind: NativeBridgeKind) -> &'static str {
    match kind {
        NativeBridgeKind::Browser => "browser",
    }
}

use crate::model_profile_resolution::ResolvedLlmSettings;

fn permission_profile_for_request(
    body: &Value,
    settings: &AgentSettings,
) -> Result<crate::permissions::PermissionProfile, String> {
    let requested = body
        .get("permission_profile")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| "permission_profile must be a string".to_string())
        })
        .transpose()?;
    let value = requested.unwrap_or(settings.permission_profile.as_str());
    crate::permissions::PermissionProfile::from_canonical_name(value).ok_or_else(|| {
        "permission_profile must be 'cautious' or 'allow_all'".to_string()
    })
}

fn chat_agent_stream_response(
    request: &mut Request,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    chat_agent_stream_response_body(body, config, None)
}

fn active_web_agent_runs() -> &'static Mutex<HashMap<String, RuntimeExecutionControl>> {
    ACTIVE_WEB_AGENT_RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug)]
struct WebAgentRunRegistration {
    conversation_id: String,
    execution_id: u64,
}

impl Drop for WebAgentRunRegistration {
    fn drop(&mut self) {
        let mut runs = active_web_agent_runs()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if runs
            .get(&self.conversation_id)
            .is_some_and(|run| run.execution_id() == self.execution_id)
        {
            runs.remove(&self.conversation_id);
        }
        if let Ok(mut owners) = OBSERVED_WORKERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            owners.remove(&self.execution_id);
        }
    }
}

fn try_register_web_agent_run(
    conversation_id: &str,
    cancellation: RuntimeExecutionControl,
) -> Result<WebAgentRunRegistration, u64> {
    let execution_id = cancellation.execution_id();
    let mut runs = active_web_agent_runs()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = runs.get(conversation_id) {
        return Err(previous.execution_id());
    }
    runs.insert(conversation_id.to_string(), cancellation);
    Ok(WebAgentRunRegistration {
        conversation_id: conversation_id.to_string(),
        execution_id,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WebAgentCancellationStatus {
    cancel_requested: bool,
    termination_confirmed: bool,
}

fn request_exact_web_agent_cancel(
    identity: &RemoteSubmissionIdentity,
) -> Result<WebAgentCancellationStatus, String> {
    let runs = active_web_agent_runs()
        .lock()
        .map_err(|_| "execution registry unavailable")?;
    let Some(cancellation) = runs.get(&identity.conversation_id) else {
        return Ok(WebAgentCancellationStatus {
            cancel_requested: false,
            termination_confirmed: false,
        });
    };
    let owners = OBSERVED_WORKERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "execution ownership unavailable")?;
    let owner = owners
        .get(&cancellation.execution_id())
        .ok_or("execution is starting; observe before cancellation")?;
    if owner.conversation_id != identity.conversation_id
        || owner.run_id != identity.submission_id
        || owner.assistant_message_id != identity.assistant_message_id
        || owner.client_id != identity.client_id
    {
        return Err("execution cancellation owner mismatch".into());
    }
    // Hold both generation and owner locks until the exact token is signalled.
    // A late callback must not cancel a newer Run in the same conversation.
    cancellation.request();
    Ok(WebAgentCancellationStatus {
        cancel_requested: true,
        termination_confirmed: false,
    })
}

fn chat_agent_cancel_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    chat_agent_cancel_response_body(body)
}

fn chat_agent_cancel_response_body(body: Value) -> Response<Box<dyn Read + Send>> {
    let (status, payload) = chat_agent_cancel_result(&body);
    json_response(status, payload)
}

fn chat_agent_cancel_result(body: &Value) -> (StatusCode, Value) {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ExactCancel {
        identity: RemoteSubmissionIdentity,
        request_digest: String,
    }
    let input: ExactCancel = match serde_json::from_value(body.clone()) {
        Ok(input) => input,
        Err(error) => {
            return (
                StatusCode(422),
                json!({"error":error.to_string(),"code":"invalid_cancellation_identity"}),
            )
        }
    };
    let result = (|| -> Result<WebAgentCancellationStatus, String> {
        input.identity.validate()?;
        if input.identity.source_host_id != host_identity() {
            return Err("cancellation source changed".into());
        }
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        let receipt: SourceSubmissionReceipt = serde_json::from_value(observation_command(&mut application, json!({
            "type":"read_source_submission_receipt","identity":input.identity,"request_digest":input.request_digest
        }))?).map_err(|e| e.to_string())?;
        receipt.validate()?;
        if receipt.phase != RemoteSubmissionPhase::Admitted {
            return Err("submission not admitted; cancel the submission intent".into());
        }
        let signalled = request_exact_web_agent_cancel(&input.identity)?;
        if signalled.cancel_requested {
            return Ok(signalled);
        }
        // No live transport worker: use the existing cancel-only driver, not a
        // normal model/tool resume and not a fabricated terminal transition.
        let token = RuntimeExecutionControl::default();
        let registration = try_register_web_agent_run(&input.identity.conversation_id, token)
            .map_err(|_| "execution changed before cancellation; observe again")?;
        let owner = ObservedWorkerOwner {
            conversation_id: input.identity.conversation_id.clone(),
            run_id: input.identity.submission_id.clone(),
            assistant_message_id: input.identity.assistant_message_id.clone(),
            client_id: input.identity.client_id.clone(),
            generation: registration.execution_id,
        };
        bind_observed_worker(&owner)?;
        let cancellation_id = format!("remote-cancel:{}", input.identity.submission_id);
        let committed: nola_application::RunCancellationReceipt = application.execute(
            nola_application::ApplicationCommand::RequestRunCancellation {
                input: nola_application::RequestRunCancellation {
                    conversation_id: input.identity.conversation_id.clone(),
                    expected_run_id: input.identity.submission_id.clone(),
                    assistant_message_id: input.identity.assistant_message_id.clone(),
                    cancellation_id: cancellation_id.clone(),
                    reason: Some("cancelled by user".into()),
                    requested_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            },
        )?;
        if committed.terminal_result.is_some() {
            return Ok(WebAgentCancellationStatus {
                cancel_requested: true,
                termination_confirmed: true,
            });
        }
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        let delivery = crate::core_agent_runtime::cancel_agent_core_expected_for_conversation(
            &input.identity.conversation_id,
            &mut emitter,
            &input.identity.submission_id,
            &input.identity.assistant_message_id,
            &cancellation_id,
        );
        if let Err(error) = delivery {
            eprintln!("Exact Web cancellation remains pending: run={}; error={error}", input.identity.submission_id);
            // Cancellation is durable but an external effect may still need
            // reconciliation. Keep the true Run active for exact observation.
            return Ok(WebAgentCancellationStatus {
                cancel_requested: true,
                termination_confirmed: false,
            });
        }
        let seal = observation_command(
            &mut application,
            json!({"type":"local_assistant_run_terminal_seal",
            "conversation_id":input.identity.conversation_id,"run_id":input.identity.submission_id}),
        )?;
        Ok(WebAgentCancellationStatus {
            cancel_requested: true,
            termination_confirmed: !seal.is_null(),
        })
    })();
    match result {
        Ok(cancellation) => (
            StatusCode(202),
            json!({"conversationId":input.identity.conversation_id,
            "cancelRequested":cancellation.cancel_requested,"terminationConfirmed":cancellation.termination_confirmed}),
        ),
        Err(error) => (
            StatusCode(409),
            json!({"code":"cancellation_owner_changed","error":error}),
        ),
    }
}

fn chat_agent_task_cancel_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (status, payload) = chat_agent_task_cancel_result(&body);
    json_response(status, payload)
}

fn chat_agent_task_cancel_result(body: &Value) -> (StatusCode, Value) {
    owned_task_cancellation_result(body, false)
}

fn owned_task_cancellation_result(body: &Value, receipt_only: bool) -> (StatusCode, Value) {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Input { host_id: String, conversation_id: String, parent_run_id: String, task_id: String }
    let result = (|| {
        let input: Input = serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        if input.host_id != host_identity() { return Err("Task cancellation Host changed".into()); }
        let owner = nola_application::owned_task_cancellation::OwnedTaskCancellation {
            parent_conversation_id: input.conversation_id.clone(), parent_run_id: input.parent_run_id.clone(),
            task_id: input.task_id.clone(),
        };
        let command = if receipt_only {
            nola_application::RuntimeTaskCommand::ReadOwnedCancellation { input: owner.clone() }
        } else {
            nola_application::RuntimeTaskCommand::RequestOwnedCancellation { input: owner.clone(),
                now_ms: crate::goal_runtime::goal_now_epoch_ms() }
        };
        let mut receipt: nola_application::owned_task_cancellation::OwnedTaskCancellationReceipt =
            crate::application_port::CliApplicationPort::persistent_default()?.execute(
                nola_application::ApplicationCommand::RuntimeTask { command })?;
        if !receipt_only && receipt.cancel_requested && !receipt.terminal {
            if let Err(error) = crate::task_runtime::reconcile_persisted_task_cancellation(&input.task_id) {
                crate::runtime_task_port::task_recovery_diagnostic(&input.task_id, "owned_cancel_reconciliation", error);
                return Err("Cancellation intent is saved, but stopping is unconfirmed. The Host could not reconcile the exact execution.".into());
            }
            receipt = crate::application_port::CliApplicationPort::persistent_default()?.execute(
                nola_application::ApplicationCommand::RuntimeTask {
                    command: nola_application::RuntimeTaskCommand::ReadOwnedCancellation { input: owner },
                })?;
        }
        Ok::<_, String>(json!({"hostId":host_identity(),"conversationId":receipt.parent_conversation_id,
            "parentRunId":receipt.parent_run_id,"taskId":receipt.task_id,
            "cancelRequested":receipt.cancel_requested,"terminal":receipt.terminal,
            "state":receipt.state,"applicationPending":false}))
    })();
    match result { Ok(receipt) => (StatusCode(200), receipt), Err(error) => (StatusCode(409), json!({"error":error})) }
}

fn chat_agent_task_cancel_receipt_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let (status, payload) = owned_task_cancellation_result(&body, true);
    json_response(status, payload)
}
fn mobile_chat_agent_task_cancel_receipt_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) { Ok(value) => value, Err(response) => return response };
    let (status, payload) = owned_task_cancellation_result(&body, true);
    mobile_encrypted_json_response(status, payload, &token)
}

fn chat_agent_task_message_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (status, payload) = chat_agent_task_message_result(&body);
    json_response(status, payload)
}

fn task_guidance_identity(body: &Value, key: &str) -> Result<String, String> {
    body.get(key).and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.trim() == *value && value.len() <= 512)
        .map(str::to_string).ok_or_else(|| format!("{key} must be a bounded exact identity"))
}

fn task_guidance_receipt(body: &Value) -> Result<Option<nola_application::RuntimeTaskGuidance>, String> {
    let field = |key| task_guidance_identity(body, key);
    if field("hostId")? != host_identity() { return Err("Task guidance Host changed".into()); }
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        nola_application::ApplicationCommand::LoadTaskGuidanceReceipt {
            task_id: field("taskId")?, message_id: field("messageId")?,
            parent_conversation_id: field("conversationId")?, parent_run_id: field("parentRunId")?,
        })
}

fn task_guidance_receipt_payload(body: &Value, guidance: Option<nola_application::RuntimeTaskGuidance>) -> Value {
    json!({"hostId":host_identity(), "conversationId":body["conversationId"],
        "taskId":body["taskId"], "messageId":body["messageId"], "guidance":guidance})
}

fn chat_agent_task_message_receipt_result(body: &Value) -> (StatusCode, Value) {
    match task_guidance_receipt(body) {
        Ok(record) => (StatusCode(200), task_guidance_receipt_payload(body, record)),
        Err(error) => (StatusCode(409), json!({"error":error})),
    }
}

fn chat_agent_task_message_receipt_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let (status, value) = chat_agent_task_message_receipt_result(&body);
    json_response(status, value)
}

fn mobile_chat_agent_task_message_receipt_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else { return json_response(StatusCode(401), json!({"error":"unauthorized"})); };
    let body = match read_mobile_json_value(request, &token) { Ok(value) => value, Err(response) => return response };
    let (status, value) = chat_agent_task_message_receipt_result(&body);
    mobile_encrypted_json_response(status, value, &token)
}

fn chat_agent_task_message_result(body: &Value) -> (StatusCode, Value) {
    let Some(message) = trimmed_string_field(body, "message") else { return (StatusCode(422), json!({"error":"message is required"})); };
    let Some(at) = body.get("submittedAtMs").and_then(Value::as_i64).filter(|at| *at >= 0) else {
        return (StatusCode(422), json!({"error":"submittedAtMs is required"}));
    };
    let existing = match task_guidance_receipt(body) { Ok(value) => value, Err(error) => return (StatusCode(409), json!({"error":error})) };
    if let Some(record) = existing {
        if record.content != message || record.submitted_at_ms != at {
            return (StatusCode(409), json!({"error":"guidance message identity conflicts"}));
        }
        return (StatusCode(200), task_guidance_receipt_payload(body, Some(record)));
    }
    let conversation_id = body["conversationId"].as_str().unwrap();
    let task_id = body["taskId"].as_str().unwrap();
    let message_id = body["messageId"].as_str().unwrap();
    let projection = match crate::session::load_session_task_projection(conversation_id) {
        Ok(Some(value)) => value, Ok(None) => return (StatusCode(404), json!({"error":"task projection not found"})),
        Err(error) => return (StatusCode(500), json!({"error":error})),
    };
    let Some(task) = projection.tasks.get(task_id) else { return (StatusCode(404), json!({"error":"task not found"})); };
    match crate::task_runtime::queue_agent_guidance_intent(conversation_id, task.owner_turn_id.as_str(), task_id, &message, message_id, at) {
        Ok(record) => (StatusCode(202), task_guidance_receipt_payload(body, Some(record))),
        Err(error) => (StatusCode(409), json!({"error":error})),
    }
}

fn canonical_host_identity_from_request(
    body: &Value,
) -> Result<(Option<String>, Option<String>, Option<String>), &'static str> {
    let field = |key: &str| -> Result<Option<String>, &'static str> {
        let Some(value) = body.get(key) else {
            return Ok(None);
        };
        let Some(raw) = value.as_str() else {
            return Err("canonical identity is invalid");
        };
        if raw.len() > 256 || raw.chars().any(char::is_control) {
            return Err("canonical identity is invalid");
        }
        let normalized = raw.trim();
        if normalized.is_empty() {
            return Err("canonical identity is invalid");
        }
        Ok(Some(normalized.to_string()))
    };
    let submission_id = field("submission_id")?;
    let user_message_id = field("user_message_id")?;
    let assistant_message_id = field("assistant_message_id")?;
    let values = [
        submission_id.as_ref(),
        user_message_id.as_ref(),
        assistant_message_id.as_ref(),
    ];
    let supplied_identity_count = values.into_iter().flatten().count();
    if supplied_identity_count != 3 {
        return Err(
            "submission_id, user_message_id and assistant_message_id must be supplied together",
        );
    }
    Ok((submission_id, user_message_id, assistant_message_id))
}

fn chat_agent_stream_response_body(body:Value,config:&Config,mobile_crypto_token:Option<String>)->Response<Box<dyn Read+Send>> {
    chat_agent_stream_response_body_with_room_retry(body,config,mobile_crypto_token,None)
}
fn chat_agent_stream_response_body_with_room_retry(
    mut body: Value,
    config: &Config,
    mobile_crypto_token: Option<String>,
    room_retry:Option<RoomSourceAdmissionRetry>,
) -> Response<Box<dyn Read + Send>> {
    let mut semantic=body.clone();
    let (source_identity,source_digest)=match validate_stream_submission(&mut semantic,host_identity()) {
        Ok(value)=>value,Err(error)=>return json_response(StatusCode(409),json!({"code":"invalid_submission","error":error})),
    };
    if room_retry.as_ref().is_some_and(|retry|retry.identity!=source_identity||retry.digest!=source_digest) {
        return json_response(StatusCode(409),json!({"code":"room_exact_source_owner_changed"}));
    }
    let preflight=match SourceAdmissionFence::acquire(&source_identity,&source_digest) {
        Ok(Some(guard))=>guard,
        Ok(None)=>return json_response(StatusCode(409),json!({"code":"source_admission_pending","error":"The original source admission remains owned; observe its exact receipt."})),
        Err(_)=>return json_response(StatusCode(409),json!({"code":"source_admission_unknown","error":"Source admission ownership is unavailable; no request was dispatched."})),
    };
    let edited_prepared=match web_prepared_edited_resend(&body) {
        Ok(value)=>value,
        Err(error)=>return json_response(StatusCode(409),json!({"code":"edited_resend_rejected","error":error})),
    };
    let edited_operation=edited_prepared.as_ref().map(|prepared|prepared.input.operation_id.clone());
    let queued_user_command_id = match body.get("queued_user_command_id") {
        None => None,
        Some(Value::String(id)) if nola_domain::generative_ui::validate_ui_submission_id(id).is_ok()
            && body.get("submission_id").and_then(Value::as_str) == Some(id.as_str())
            && body.get("user_message_id").and_then(Value::as_str) == Some(format!("ui:{id}:user").as_str())
            && body.get("assistant_message_id").and_then(Value::as_str) == Some(format!("{id}:assistant").as_str()) => Some(id.clone()),
        _ => return json_response(StatusCode(400), json!({"code":"invalid_queued_input_identity"})),
    };
    match admitted_stream_submission_receipt(&body) {
        Ok(Some(receipt)) => {
            let (tx,rx)=mpsc::channel();
            let _=tx.send(mobile_sse_event(json!({"type":"submission_receipt","receipt":receipt}),mobile_crypto_token.as_deref()));
            let _=tx.send(b"data: [DONE]\n\n".to_vec());
            return sse_response(rx);
        },
        Ok(None) => {},
        Err(error) => return json_response(StatusCode(409),json!({"code":"invalid_submission","error":error})),
    }
    let remote_tool_plan = match admit_host_tool_plan(&body, config) {
        Ok(plan) => plan,
        Err(error) => {
            // A verified original request rejected before admission must leave
            // a durable receipt for its remote owner to reconcile. Registration
            // validates exact identity/digest; Store settlement never regresses
            // an admitted or already-cancelled request during a process race.
            let receipt = register_stream_submission(&mut body).and_then(|(mut dispatch,_,_)|{
                dispatch._preflight=Some(preflight);
                dispatch.settle()
            });
            match receipt {
                Ok(receipt) if receipt.phase == RemoteSubmissionPhase::Admitted => {
                    let (tx,rx)=mpsc::channel();
                    let _=tx.send(mobile_sse_event(json!({"type":"submission_receipt","receipt":receipt}),mobile_crypto_token.as_deref()));
                    let _=tx.send(b"data: [DONE]\n\n".to_vec());
                    return sse_response(rx);
                },
                Ok(receipt) => return json_response(StatusCode(409),json!({"code":"host_tool_plan_rejected","error":error,"receipt":receipt})),
                Err(_) => return json_response(StatusCode(409),json!({"code":"host_tool_plan_rejected","error":error})),
            }
        },
    };
    let (expected_configuration_revision, expected_parent_message_id) =
        match crate::native_host_request::submission_preconditions(&body) {
            Ok(preconditions) => preconditions,
            Err(error) => return json_response(StatusCode(422), json!({"code":"invalid_submission_preconditions","error":error})),
        };
    // Bind the original semantic request before resolving mutable Host defaults.
    let (mut dispatch, receipt, created) = match register_stream_submission(&mut body) {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                StatusCode(409),
                json!({"code":"invalid_submission","error":error}),
            )
        }
    };
    let exact_room_retry=room_retry.is_some() && receipt.phase==RemoteSubmissionPhase::Prepared;
    if !created && !exact_room_retry {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(mobile_sse_event(
            json!({"type":"submission_receipt","receipt":receipt}),
            mobile_crypto_token.as_deref(),
        ));
        let _ = tx.send(b"data: [DONE]\n\n".to_vec());
        return sse_response(rx);
    }
    // Retain the same fence across registration, actual Begin and the worker
    // pump. A duplicate can observe, never race another physical preflight.
    dispatch.owns_dispatch=true;
    dispatch._preflight=Some(preflight);
    let dispatch = Arc::new(dispatch);
    let body = match if edited_prepared.is_some() {Ok(body)} else {apply_conversation_request_configuration(body)} {
        Ok(body) => body,
        Err(error) => {
            return json_response(
                StatusCode(422),
                json!({"code":"invalid_conversation_configuration","error":error}),
            )
        }
    };
    let state = load_state().unwrap_or_default();
    // Persisted settings may have changed independently of the server state.
    // Capture them before composing any catalog, context, or execution policy.
    let mut turn_agent_settings =
        match NolaStore::open_default().and_then(|store| load_agent_settings_from_store(&store)) {
            Ok(settings) => settings,
            Err(error) => {
                return json_response(
                    StatusCode(500),
                    json!({"error": error, "code": "agent_settings_snapshot_failed"}),
                )
            }
        };
    let permission_profile = match if edited_prepared.is_some() {Ok(crate::permissions::PermissionProfile::Cautious)}
        else {permission_profile_for_request(&body, &turn_agent_settings)} {
        Ok(profile) => profile,
        Err(error) => {
            return json_response(
                StatusCode(422),
                json!({"error": error, "code": "invalid_permission_profile"}),
            )
        }
    };
    let safe_web_only = body.get("agentMode").and_then(Value::as_bool) == Some(false);
    // A model test is a chat-only inference check, not an authorization to
    // exercise the Host, MCP or device catalog. The flag can only remove tools.
    let model_test_mode = body.get("model_test_mode").and_then(Value::as_bool) == Some(true);
    if model_test_mode && body.get("device_bridge").is_some() {
        return json_response(
            StatusCode(400),
            json!({"error":"model tests cannot use device tools"}),
        );
    }
    let skills = if safe_web_only || model_test_mode {
        crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(Vec::new()))
    } else {
        match web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &turn_agent_settings) {
            Ok(skills) => skills,
            Err(error) => {
                return json_response(
                    StatusCode(502),
                    json!({"error":error,"code":"skill_catalog_unavailable"}),
                )
            }
        }
    };
    let host_id = host_identity().to_string();
    if let Some(expected_host_id) = trimmed_string_field(&body, "expected_host_id") {
        if expected_host_id != host_id {
            return json_response(
                StatusCode(409),
                json!({
                    "error": "host identity changed; explicit re-pairing is required",
                    "code": "host_identity_changed",
                    "expected_host_id": expected_host_id,
                    "actual_host_id": host_id,
                }),
            );
        }
    }
    let execution_backend = if let Some(prepared)=&edited_prepared {
        if prepared.source.model.primary["kind"]=="external" {
            prepared.source.model.primary["backend"].as_str().unwrap_or("").to_string()
        } else {"nola".to_string()}
    } else {trimmed_string_field(&body, "execution_backend")
        .unwrap_or_else(|| state.agent_settings.execution_backend.clone())};
    if body.get("model_intent").is_some() && execution_backend != "nola" {
        return json_response(
            StatusCode(422),
            json!({"code":"invalid_model_intent",
            "error":"model_intent is only supported by the Nola execution backend"}),
        );
    }
    let execution_backend_catalog = crate::external_executor::discover_execution_backends();
    let execution_backend_available = execution_backend_catalog.iter().any(|backend| {
        backend.id == execution_backend
            && backend.availability
                == nola_domain::execution_backend::ExecutionBackendAvailability::Available
    });
    if !execution_backend_available {
        return json_response(
            StatusCode(422),
            json!({
                "error": format!("execution backend is unavailable: {execution_backend}"),
                "code": "execution_backend_unavailable",
                "execution_backend": execution_backend,
                "execution_backends": execution_backend_catalog,
            }),
        );
    }
    // A prepared target intentionally has no invented saved profile. Rebuild
    // its original model privately before the first Host/model validation.
    let external_memory = if execution_backend != "nola" {
        match resolve_turn_memory_policy(&body, config, Some(&dispatch.identity.conversation_id)) {
            Ok(value) => value, Err(response) => return response,
        }
    } else {None};
    let resolved = if edited_prepared.is_some() || execution_backend != "nola" {None} else {Some(match resolve_llm_settings(&body, config, Some(&turn_agent_settings), Some(&dispatch.identity.conversation_id)) {
        Ok(settings) => settings,
        Err(response) => return response,
    })};
    let client_id = dispatch.identity.client_id.clone();
    let host_session_id = dispatch.identity.conversation_id.clone();
    turn_agent_settings.mcp_servers = match crate::mcp_connection_authority::bind_configurations(
        &crate::resource_access::mcp_runtime_resources(&turn_agent_settings.mcp_servers),
        "nola-cli-host", &host_session_id,
    ) {
        Ok(servers) => servers,
        Err(error) => return json_response(StatusCode(409),json!({"code":"host_tool_plan_rejected","error":error})),
    };
    let (submission_id, user_message_id, assistant_message_id) =
        match canonical_host_identity_from_request(&body) {
            Ok(identity) => identity,
            Err(error) => {
                return json_response(
                    StatusCode(400),
                    json!({"error": error, "code": "invalid_canonical_identity"}),
                )
            }
        };
    let should_persist_trace = true;
    let device_bridge_runtime_tools = if let Some(value) = body.get("device_bridge") {
        if mobile_crypto_token.is_none() {
            return json_response(
                StatusCode(401),
                json!({"error": "device bridge requires an authenticated mobile request"}),
            );
        }
        let advertisement = match serde_json::from_value::<DeviceBridgeAdvertisement>(value.clone())
        {
            Ok(value) => value,
            Err(_) => {
                return json_response(
                    StatusCode(400),
                    json!({"error": "invalid device bridge advertisement"}),
                )
            }
        };
        if let Err(error) = nola_core::device_bridge::validate_advertisement(
            &advertisement,
            &host_id,
            &client_id,
            &host_session_id,
        ) {
            return json_response(StatusCode(400), json!({"error": error}));
        }
        let issued_at_ms = unix_time_seconds().saturating_mul(1_000);
        let expires_at_ms = issued_at_ms.saturating_add(300_000);
        let manifest_id = format!(
            "device-bridge:{}:{}:{}:{}",
            host_id, client_id, host_session_id, issued_at_ms
        );
        advertisement
            .tools
            .into_iter()
            .map(|tool| {
                let tool_id = nola_core::device_bridge::device_tool_name(&tool.schema)
                    .expect("validated device bridge tool")
                    .to_string();
                crate::tools::DeviceBridgeRuntimeTool {
                    schema: tool.schema,
                    capability: tool.capability,
                    result_egress_receipt: nola_core::tool_policy::RuntimeResultEgressReceipt {
                        tool_id: tool_id.clone(),
                        route: ToolRoute::Device,
                        manifest_id: manifest_id.clone(),
                        host_id: host_id.clone(),
                        client_id: client_id.clone(),
                        session_id: host_session_id.clone(),
                        issued_at_ms,
                        expires_at_ms,
                        allow_result_egress: tool.allow_result_egress,
                    },
                    result_egress_grant: nola_core::tool_policy::ResultEgressGrant {
                        tool_id,
                        route: ToolRoute::Device,
                        manifest_id: manifest_id.clone(),
                        host_id: host_id.clone(),
                        client_id: client_id.clone(),
                        session_id: host_session_id.clone(),
                        expires_at_ms,
                    },
                }
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let base_catalog = if model_test_mode {
        Vec::new()
    } else {
        web_host_tool_catalog(&skills)
    };
    let mut local_candidates =
        web_local_candidate_schemas_from_catalog(&turn_agent_settings, &base_catalog);
    if safe_web_only {
        local_candidates.retain(|schema| {
            schema
                .pointer("/function/name")
                .and_then(Value::as_str)
                .is_some_and(|name| tool_spec_category(name) == "web_safe")
        });
    }
    let host_candidates =
        if !model_test_mode && !safe_web_only && turn_agent_settings.host_tools_enabled {
            base_catalog.as_slice()
        } else {
            &[]
        };
    let mut candidates = candidates_from_schemas(&local_candidates, ToolRoute::AgentHost);
    candidates.extend(candidates_from_schemas(
        host_candidates,
        ToolRoute::AgentHost,
    ));
    // The bound Core publisher is a Host capability even when filesystem and
    // device tools are unavailable. Include it before policy resolution so the
    // model schema and the driver's explicit allowlist agree.
    if !model_test_mode {
        candidates.extend(candidates_from_schemas(
            &[nola_core::agent::generative_ui::tool_schema()],
            ToolRoute::AgentHost,
        ));
    }
    candidates.extend(
        device_bridge_runtime_tools
            .iter()
            .cloned()
            .map(|runtime_tool| {
                crate::tool_policy::candidate_with_result_egress(
                    runtime_tool.schema,
                    ToolRoute::Device,
                    runtime_tool.result_egress_receipt,
                    runtime_tool.result_egress_grant,
                )
            }),
    );
    restrict_web_tool_candidates(&mut candidates, config.tools_enabled, config.tool_allowlist.as_deref());
    let (context_tools, tool_plan) = resolve_tool_schemas(
        candidates,
        &ToolPlanOptions {
            platform: ToolPlatform::Web,
            conversation_mode: if !safe_web_only && turn_agent_settings.agent_mode_enabled {
                "agent"
            } else {
                "chat"
            }
            .to_string(),
            network_policy: turn_agent_settings.network_policy.clone(),
            // Availability and authorization are separate contracts. Keep the
            // complete Host catalog visible to the model; Core evaluates the
            // concrete call against the normalized permission rules below.
            denied_tools: Vec::new(),
            allow_result_egress_routes: if device_bridge_runtime_tools.is_empty() {
                Vec::new()
            } else {
                vec![ToolRoute::Device]
            },
        },
    );
    let tools_enabled = !context_tools.is_empty();
    llm::log_local_debug_payload(
        "core_tool_policy",
        &json!({
            "type": "core_tool_policy",
            "surface": "web",
            "enabled": tool_plan.enabled_tools.len(),
            "enabled_tool_ids": tool_plan
                .enabled_tools
                .iter()
                .map(|tool| tool.tool_id.as_str())
                .collect::<Vec<_>>(),
            "disabled": tool_plan.disabled_tools,
        }),
    );
    if let Err(error) = prepare_turn_mcp_servers(
        &mut turn_agent_settings,
        model_test_mode,
        crate::mcp_host::refresh_changed_servers,
    ) {
        return json_response(
            StatusCode(502),
            json!({
                "error": error.message,
                "code": error.code,
                "message": "The MCP catalog changed, but the next-Turn refresh failed."
            }),
        );
    }
    // Refresh may advance a connected MCP generation after preparation. Do not
    // silently admit a different initial catalog under the prepared reference.
    match capture_current_host_tool_plan_tools(config, &host_session_id) {
        Ok((_, revision)) if revision == remote_tool_plan.reference.catalog_revision => {},
        Ok(_) => return json_response(StatusCode(409),json!({"code":"host_tool_plan_rejected","error":"Host catalog changed during admission; prepare again"})),
        Err(error) => return json_response(StatusCode(409),json!({"code":"host_tool_plan_rejected","error":error})),
    }
    let context_messages = if let Some(prepared)=&edited_prepared {
        vec![json!({"id":prepared.user_message_id,"role":"user","content":prepared.input.edited_text})]
    } else {agent_context_messages_from_request(&body, &turn_agent_settings)};
    let mut context_components = if edited_prepared.is_some() {Vec::new()} else {
        match web_context_components_from_request_with_skills(&body, &turn_agent_settings, &skills)
        {
            Ok(components) => components,
            Err(error) => {
                return json_response(
                    StatusCode(502),
                    json!({"error": error, "code": "context_source_unavailable"}),
                )
            }
        }};
    if let Some(captured)=room_retry.as_ref().and_then(|retry|retry.prompt_definitions.as_ref()) {
        let definitions=match crate::core_agent_runtime::frozen_expert_prompt_components(captured) {
            Ok(value)=>value,
            Err(error)=>return json_response(StatusCode(409),json!({"code":"room_prompt_definition_rejected","error":error})),
        };
        context_components.retain(|component| component.kind != ContextComponentKind::PromptTemplate);
        context_components.extend(definitions);
    }
    let (initial_items, input_items) = split_web_core_items(&context_messages);
    if input_items.is_empty() {
        return json_response(
            StatusCode(422),
            json!({"error": "chat request has no current user input"}),
        );
    }
    let max_iterations = positive_usize_field(&body, "max_iterations").unwrap_or(0);
    let external_prompt = input_items
        .iter()
        .rev()
        .find_map(|item| match item {
            AgentItem::Message {
                role: AgentRole::User,
                content,
                ..
            } => Some(content.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if should_persist_trace && edited_prepared.is_none() {
        let visible_user_text = body
            .get("current_user_display_text")
            .and_then(Value::as_str)
            .unwrap_or(external_prompt.as_str());
        if let Err(error) =
            adopt_first_message_conversation_title(&host_session_id, visible_user_text)
        {
            return json_response(
                StatusCode(500),
                json!({
                    "error": error,
                    "code": "conversation_metadata_commit_failed"
                }),
            );
        }
    }

    let mut agent_config = config.clone();
    if execution_backend != "nola" { agent_config.runtime.memory_enabled = external_memory; }
    agent_config.skill_dirs = skills.configured_dirs().map(<[PathBuf]>::to_vec);
    if let Some(resolved)=resolved {
        agent_config.runtime = resolved.runtime;
        agent_config.endpoint = resolved.endpoint;
        agent_config.api_key = resolved.api_key;
        agent_config.model = resolved.model;
        agent_config.runtime.temperature = Some(resolved.temperature);
        agent_config.runtime.top_p = Some(resolved.top_p);
        agent_config.runtime.max_output_tokens = Some(resolved.max_output_tokens);
        agent_config.runtime.compaction.context_window_tokens =
            Some(usize::try_from(resolved.max_context_tokens).unwrap_or(usize::MAX));
    }
    if edited_prepared.is_some() {
        agent_config.message=external_prompt.clone();
        agent_config.context_files.clear();
        agent_config.host_attachments.clear();
        agent_config.host_turn_context=None;
    }
    agent_config.max_iterations = max_iterations;
    agent_config.json_output = true;
    // Every source submission has durable identity and a receipt, including
    // utility requests. No identity-free ephemeral execution bypass remains.
    agent_config.save_session = should_persist_trace;
    agent_config.continue_last = false;
    agent_config.session_id = Some(host_session_id.clone());
    agent_config.host_submission_id = submission_id;
    agent_config.expected_configuration_revision = expected_configuration_revision;
    agent_config.expected_parent_message_id = expected_parent_message_id;
    agent_config.host_user_message_id = user_message_id;
    agent_config.host_assistant_message_id = assistant_message_id;
    agent_config.execution_backend = execution_backend;
    agent_config.external_execution_session_id = trimmed_string_field(&body, "external_session_id");
    agent_config.external_execution_model = trimmed_string_field(&body, "execution_model");
    agent_config.external_execution_model_provider =
        trimmed_string_field(&body, "execution_model_provider");
    // The shared Host policy builder consumes these typed components from the
    // request. Do not send a second copy through the per-turn argument channel.
    agent_config.host_context_components = context_components;
    agent_config
        .runtime
        .core_permission_rules
        .extend(web_permission_rules_for_core(&state.permission_rules));
    agent_config.tools_enabled = tools_enabled;
    agent_config.remote_tool_plan = Some(remote_tool_plan);
    // A generated Web route catalog is an authorization boundary, not a user
    // request to put every schema into the model prompt. Reuse the desktop
    // Host's tiered initial surface and typed discovery for deferred tools.
    // Preserve an explicitly configured direct surface when one was supplied.
    agent_config.tool_allowlist_is_authorization_only =
        config.tool_allowlist.is_none() || config.tool_allowlist_is_authorization_only;
    // An unrestricted local Host owns its live catalog, including tools learned
    // after an approved MCP connection. Freezing the initial Web schema names
    // into an allowlist hides mcp_connect and every subsequently discovered tool.
    // Keep explicit user restrictions and limited Web/device surfaces bounded.
    agent_config.tool_allowlist = if turn_agent_settings.host_tools_enabled
        && !safe_web_only && !model_test_mode && config.tool_allowlist.is_none() {
        None
    } else { Some(
        context_tools
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
    ) };
    agent_config.auto_approve = safe_web_only
        && normalize_network_policy_value(&turn_agent_settings.network_policy) == "allow";
    apply_agent_tool_settings(&mut agent_config, &turn_agent_settings);
    agent_config.permission_profile = Some(permission_profile);
    agent_config.auto_approve = permission_profile.auto_approve();
    agent_config.network_policy = permission_profile.network_policy().to_string();
    if model_test_mode {
        agent_config.mcp_server_registry = Default::default();
        agent_config.mcp_capability_catalog = None;
    } else {
        let mcp_catalog_revision =
            crate::agent_host_driver::configured_mcp_capability_catalog_revision(
                &turn_agent_settings.mcp_servers,
            );
        agent_config.mcp_server_registry = turn_agent_settings.mcp_servers.clone();
        agent_config.mcp_capability_catalog =
            match crate::agent_host_driver::configured_mcp_capability_catalog(
                &turn_agent_settings.mcp_servers,
                mcp_catalog_revision,
            ) {
                Ok(catalog) => Some(catalog),
                Err(error) => {
                    return json_response(
                        StatusCode(422),
                        json!({"error": error, "code": "invalid_mcp_capability_catalog"}),
                    )
                }
            };
    }

    let execution_cancellation = RuntimeExecutionControl::default();
    let run_registration =
        match try_register_web_agent_run(&host_session_id, execution_cancellation.clone()) {
            Ok(registration) => registration,
            Err(active_execution_id) => {
                return json_response(
                    StatusCode(409),
                    json!({
                        "error": "this conversation already has an active run",
                        "code": "run_in_progress",
                        "conversationId": host_session_id,
                        "activeExecutionId": active_execution_id,
                        "recovery": "observe"
                    }),
                );
            }
        };
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _ = tx.send(mobile_sse_event(
        json!({"type":"submission_receipt","receipt":receipt}),
        mobile_crypto_token.as_deref(),
    ));
    let receipt_fence = Arc::new(SubmissionReceiptFence {
        dispatch: Arc::clone(&dispatch),
        sent_admission: Mutex::new(false),
    });
    let (heartbeat_stop_tx, heartbeat_stop_rx) = mpsc::channel::<()>();
    let heartbeat_handle =
        spawn_sse_heartbeat(tx.clone(), heartbeat_stop_rx, StdDuration::from_secs(15));
    thread::spawn(move || {
        // Dropping this sender wakes and stops the heartbeat thread on every
        // return path, including permission/device bridge failures.
        let _heartbeat_stop = heartbeat_stop_tx;
        let _heartbeat_handle = heartbeat_handle;
        let observation_owner = agent_config
            .host_submission_id
            .as_ref()
            .zip(agent_config.host_assistant_message_id.as_ref())
            .map(|(run, assistant)| ObservedWorkerOwner {
                conversation_id: host_session_id.clone(),
                run_id: run.clone(),
                assistant_message_id: assistant.clone(),
                client_id: client_id.clone(),
                generation: run_registration.execution_id,
            });
        let _run_registration = run_registration;
        // The runtime nests this same canonical conversation owner. Client
        // identity remains frozen in the resolver/observation registration,
        // not encoded into a second, incompatible session identifier.
        let _host_permission_session = enter_host_permission_session(&host_session_id);
        let resolver_host_id = host_id.clone();
        let resolver_client_id = client_id.clone();
        let resolver_session_id = host_session_id.clone();
        let resolver_tx = tx.clone();
        let resolver_token = mobile_crypto_token.clone();
        let resolver_owner = observation_owner.clone();
        let resolver_receipt = Arc::clone(&receipt_fence);
        let _host_permission_resolver =
            enter_host_permission_resolver(Arc::new(move |request, instructions, allow_conversation| {
                if resolver_receipt
                    .deliver(&resolver_tx, resolver_token.as_deref())
                    .is_err()
                {
                    return ToolPermissionResolution::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
                    );
                }
                if let Some(owner) = &resolver_owner {
                    if bind_observed_worker(owner).is_err() {
                        return ToolPermissionResolution::TransportInterrupted(
                            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
                        );
                    }
                }
                wait_for_host_permission_in_run(
                    request,
                    &resolver_host_id,
                    &resolver_client_id,
                    &resolver_session_id,
                    &resolver_tx,
                    resolver_token.as_deref(),
                    instructions,
                    allow_conversation,
                    StdDuration::from_secs(300),
                    resolver_owner.as_ref().map(|o| o.run_id.as_str()),
                )
            }));
        let _device_bridge_tools = match enter_device_bridge_tools(device_bridge_runtime_tools) {
            Ok(guard) => guard,
            Err(error) => {
                let _ = tx.send(mobile_sse_event(
                    json!({"type": "error", "message": error, "error": error}),
                    mobile_crypto_token.as_deref(),
                ));
                dispatch.finish_stream(&tx, mobile_crypto_token.as_deref());
                return;
            }
        };
        let device_resolver_host_id = host_id.clone();
        let device_resolver_client_id = client_id.clone();
        let device_resolver_session_id = host_session_id.clone();
        let device_resolver_tx = tx.clone();
        let device_resolver_token = mobile_crypto_token.clone();
        let device_receipt = Arc::clone(&receipt_fence);
        let _device_bridge_resolver = match enter_device_bridge_resolver(
            nola_core::tool_policy::RuntimeIdentity {
                host_id: Some(host_id.clone()),
                client_id: Some(client_id.clone()),
                session_id: Some(host_session_id.clone()),
            },
            Arc::new(move |request, runtime_tool| {
                if device_receipt
                    .deliver(&device_resolver_tx, device_resolver_token.as_deref())
                    .is_err()
                {
                    return ToolOutcome {
                        full_output_ref: None,
                        data: None,
                        execution: None,
                        text: "Source submission admission is unavailable".into(),
                        display: "Device dispatch requires source admission".into(),
                        error: true,
                        artifacts: Vec::new(),
                        activation: None,
                    };
                }
                wait_for_device_tool(
                    request,
                    runtime_tool,
                    &device_resolver_host_id,
                    &device_resolver_client_id,
                    &device_resolver_session_id,
                    &device_resolver_tx,
                    device_resolver_token.as_deref(),
                )
            }),
        ) {
            Ok(guard) => guard,
            Err(error) => {
                let _ = tx.send(mobile_sse_event(
                    json!({"type":"error", "message":error}),
                    mobile_crypto_token.as_deref(),
                ));
                dispatch.finish_stream(&tx, mobile_crypto_token.as_deref());
                return;
            }
        };
        let delegated_permissions = observation_owner.clone().map(|owner|
            stream_delegated_permissions(owner, tx.clone(), mobile_crypto_token.clone(), Some(Arc::clone(&receipt_fence))));
        let event_tx = tx.clone();
        let event_crypto_token = mobile_crypto_token.clone();
        let usage_conversation_id = host_session_id.clone();
        let event_source_host_id = host_id.clone();
        let event_receipt = Arc::clone(&receipt_fence);
        let mut journal_notifications = None;
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(move |mut event| {
            if event_receipt
                .deliver(&event_tx, event_crypto_token.as_deref())
                .is_err()
            {
                return;
            }
            if let Some(owner) = &observation_owner {
                let _ = bind_observed_worker(owner);
            }
            let event_type = event.get("type").and_then(Value::as_str).map(str::to_owned);
            if matches!(
                event_type.as_deref(),
                Some("assistant_journal_committed_v1" | "assistant_run_terminal_committed_v1")
            ) {
                event["source_host_id"] = Value::String(event_source_host_id.clone());
            }
            if let Some(owner) = &observation_owner {
                if observe_journal_for_stream(&mut journal_notifications, owner, &event,
                    &event_tx, &event_crypto_token, &event_source_host_id) { return; }
            }
            // The emitter-level usage observation is shared with the terminal
            // CLI and therefore has no transport identity of its own. Mobile
            // scopes it here before forwarding so native clients never guess
            // which conversation owns the measurement.
            if event_type.as_deref() == Some("context_usage") {
                event["protocol_version"] = Value::from(2);
                event["session_id"] = Value::String(usage_conversation_id.clone());
                event["source_host_id"] = Value::String(event_source_host_id.clone());
            }
            // Mobile is a canonical Core transport. Presenter-only events are
            // intentionally not part of this protocol and must never be
            // converted into user-visible failures.
            if !mobile_event_is_canonical(&event) {
                llm::log_local_debug_payload(
                    "mobile_non_canonical_event_dropped",
                    &json!({
                        "type": "mobile_non_canonical_event_dropped",
                        "event_type": event.get("type").and_then(Value::as_str),
                    }),
                );
                return;
            }
            let trace = event
                .get("trace")
                .cloned()
                .and_then(|value| serde_json::from_value::<AgentTraceEvent>(value).ok());
            // Core projections are already the canonical presentation
            // contract. They intentionally do not require an append-only
            // Trace wrapper.
            if !mobile_event_requires_trace(&event) {
                let _ = event_tx.send(mobile_sse_event(event, event_crypto_token.as_deref()));
                return;
            }
            let Some(trace) = trace else {
                let _ = event_tx.send(mobile_sse_event(
                    json!({
                        "type": "error",
                        "message": "Agent Core emitted a presentation event without a trace envelope.",
                        "code": "missing_core_trace"
                    }),
                    event_crypto_token.as_deref(),
                ));
                return;
            };
            // Permission requests need transport metadata (nonce, digest and
            // expiry) that only the synchronous permission bridge can attach.
            // The bridge sends this same canonical trace inside the
            // `permission_request` envelope. Sending the plain trace here as
            // well produces two UI requests for one operation.
            if trace.kind == AgentTraceKind::ToolPermissionRequested {
                return;
            }
            let _ = event_tx.send(mobile_sse_event(event, event_crypto_token.as_deref()));
        });
        llm::log_local_debug_payload("mobile_host_dispatch_started", &json!({
            "conversation_id": dispatch.identity.conversation_id,
            "run_id": dispatch.identity.submission_id,
        }));
        let dispatch_started = std::time::Instant::now();
        let run_result = agent_config.host_invocation_request().and_then(|mut request| {
            if let Some(operation)=&edited_operation {
                request.session.edited_resend_operation_id=Some(operation.clone());
                request.runtime.expert_template_authority=Some(expert_host_authority());
                crate::core_agent_runtime::prepare_edited_resend_host_request(&mut request)?;
            }
            if let Some(command_id) = queued_user_command_id {
                let now = nola_runtime_driver::monotonic_epoch_millis();
                let lease_owner = format!("web-ui:{}", uuid::Uuid::new_v4());
                let lease = crate::agent_command_port::claim_next_agent_command(
                    &host_session_id, &lease_owner, now, now.saturating_add(300_000))?
                    .ok_or("queued UI input is already claimed or absent; observe its Run")?;
                if lease.command_id != command_id {
                    crate::agent_command_port::release_agent_command_lease(&host_session_id, &lease.command_id, &lease_owner)?;
                    return Err("an earlier queued input must be handled first".into());
                }
                let admission_request_id = format!("queued-admission:{}:{}", command_id, lease.delivery_attempt);
                crate::core_agent_runtime::run_queued_user_input(&request, lease, admission_request_id,
                    &mut emitter, execution_cancellation, ContextPlatform::Web, Vec::new())
            } else {
                run_agent_core_turn_with_cancellation(&request, initial_items, input_items,
                    ContextPlatform::Web, Vec::new(), execution_cancellation, &mut emitter)
            }
        });
        // Keep the exact dispatch owner and outcome before transport settlement.
        // This is diagnostic evidence, never proof of a canonical terminal.
        llm::log_local_debug_payload("mobile_host_dispatch_returned", &json!({
            "conversation_id": dispatch.identity.conversation_id,
            "run_id": dispatch.identity.submission_id,
            "elapsed_ms": dispatch_started.elapsed().as_millis(),
            "ok": run_result.is_ok(),
            "error": run_result.as_ref().err().map(|error| crate::session::redact_sensitive_text(error)),
        }));
        // A successful pump may leave a canonical Run awaiting permission.
        // Its emitted Core snapshot is authoritative; HTTP [DONE] below only
        // closes this transport and must never synthesize a Run terminal.
        if let Err(err) = run_result {
            // A Core/projection failure must close the durable trace as well
            // as the transport. Otherwise journal followers see a permanently
            // running assistant bubble after the SSE producer has exited.
            emitter.error(&err);
            let _ = tx.send(mobile_sse_event(
                json!({
                    "type": "error",
                    "message": err,
                    "error": err,
                    "code": "agent_core_failed"
                }),
                mobile_crypto_token.as_deref(),
            ));
        }
        drop(delegated_permissions);
        let _ = receipt_fence.deliver(&tx, mobile_crypto_token.as_deref());
        dispatch.finish_stream(&tx, mobile_crypto_token.as_deref());
    });

    sse_response(rx)
}

fn web_event_is_core_projection(event: &Value) -> bool {
    matches!(
        event.get("type").and_then(Value::as_str),
        Some(
            "core_task_updated"
                | "core_skill_runtime_updated"
                | "core_capabilities_activated"
                | "core_permission_projection_updated"
                | "core_memory_runtime_projection_updated"
                | "assistant_journal_committed_v1"
                | "assistant_run_terminal_committed_v1"
                | "context_usage"
        )
    )
}

fn mobile_event_is_canonical(event: &Value) -> bool {
    web_event_is_core_projection(event) || event.get("trace").is_some_and(Value::is_object)
}

fn mobile_event_requires_trace(event: &Value) -> bool {
    !web_event_is_core_projection(event)
}

fn mobile_sse_event(value: Value, token: Option<&str>) -> Vec<u8> {
    let Some(token) = token else {
        return sse_event(value);
    };
    match nola_core::mobile_crypto::encrypt_mobile_json(&value, token).and_then(|envelope| {
        serde_json::to_value(envelope)
            .map_err(|_| "failed to encode encrypted SSE event".to_string())
    }) {
        Ok(envelope) => sse_event(envelope),
        Err(error) => sse_event(json!({"type":"error","message":error})),
    }
}

fn host_identity() -> &'static str {
    initialize_host_identity().expect("Web Host requires durable identity before serving requests")
}

fn initialize_host_identity() -> Result<&'static str, String> {
    HOST_IDENTITY
        .get_or_init(load_or_create_host_identity)
        .as_ref()
        .map(String::as_str)
        .map_err(Clone::clone)
}

fn load_or_create_web_host_access_token() -> Result<(String, PathBuf), String> {
    let path = web_runtime_directory()?.join("mobile-access-token");
    let token = load_or_create_web_host_access_token_at(&path)?;
    Ok((token, path))
}

fn load_or_create_web_host_access_token_at(path: &Path) -> Result<String, String> {
    if let Ok(value) = fs::read_to_string(path) {
        let token = value.trim();
        if valid_mobile_access_token(token) {
            harden_private_file(path)?;
            return Ok(token.to_string());
        }
        return Err(format!(
            "existing Web Host access token {} is invalid",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Web Host access token path has no parent directory".to_string())?;
    fs::create_dir_all(&parent).map_err(|error| {
        format!(
            "failed to create Web Host credential directory {}: {error}",
            parent.display()
        )
    })?;
    let token = nola_core::mobile_crypto::generate_mobile_access_token();
    match write_private_new_file(path, &format!("{token}\n")) {
        Ok(()) => Ok(token),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read_to_string(path).map_err(|read_error| {
                format!(
                    "failed to read existing Web Host access token {}: {read_error}",
                    path.display()
                )
            })?;
            let existing = existing.trim();
            if !valid_mobile_access_token(existing) {
                return Err(format!(
                    "existing Web Host access token {} is invalid",
                    path.display()
                ));
            }
            harden_private_file(path)?;
            Ok(existing.to_string())
        }
        Err(error) => Err(format!(
            "failed to create Web Host access token {}: {error}",
            path.display()
        )),
    }
}

fn valid_mobile_access_token(token: &str) -> bool {
    (32..=256).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn load_or_create_host_identity() -> Result<String, String> {
    let parent = web_runtime_directory()?;
    load_or_create_host_identity_at(&parent)
}

fn load_or_create_host_identity_at(parent: &Path) -> Result<String, String> {
    let path = parent.join("host-identity");
    if let Ok(value) = fs::read_to_string(&path) {
        let identity = value.trim();
        if valid_host_identity(identity) {
            harden_private_file(&path)?;
            return Ok(identity.to_string());
        }
    }
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create host identity directory {}: {error}",
            parent.display()
        )
    })?;
    let identity = nola_core::mobile_crypto::generate_protocol_nonce("cli-host");
    match write_private_new_file(&path, &format!("{identity}\n")) {
        Ok(()) => Ok(identity),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if let Ok(value) = fs::read_to_string(&path) {
                let existing = value.trim();
                if valid_host_identity(existing) {
                    return Ok(existing.to_string());
                }
            }
            Err(format!(
                "existing host identity {} is invalid",
                path.display()
            ))
        }
        Err(error) => Err(format!(
            "failed to create host identity {}: {error}",
            path.display()
        )),
    }
}

fn valid_host_identity(identity: &str) -> bool {
    (16..=128).contains(&identity.len())
        && identity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn write_private_new_file(path: &Path, content: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = std::io::Write::write_all(&mut file, content.as_bytes()) {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    file.sync_all()
}

#[cfg(unix)]
fn harden_private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to secure private file {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn harden_private_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn unix_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct HostPermissionRegistryCleanup(String);

impl Drop for HostPermissionRegistryCleanup {
    fn drop(&mut self) {
        if let Ok(mut events) = OBSERVED_PERMISSION_EVENTS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            events.remove(&self.0);
        }
        if let Ok(mut challenges) = HOST_PERMISSION_CHALLENGES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            challenges.remove(&self.0);
        }
        if let Ok(mut waiters) = HOST_PERMISSION_WAITERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
        {
            waiters.remove(&self.0);
        }
    }
}

fn wait_for_host_permission(
    request: &ToolRequest,
    host_id: &str,
    client_id: &str,
    session_id: &str,
    event_tx: &mpsc::Sender<Vec<u8>>,
    mobile_crypto_token: Option<&str>,
    instructions: &[nola_core::agent::host::PermissionInstruction],
    allow_conversation: bool,
) -> ToolPermissionResolution {
    wait_for_host_permission_with_timeout(
        request,
        host_id,
        client_id,
        session_id,
        event_tx,
        mobile_crypto_token,
        instructions,
        allow_conversation,
        StdDuration::from_secs(300),
    )
}

fn wait_for_host_permission_with_timeout(
    request: &ToolRequest,
    host_id: &str,
    client_id: &str,
    session_id: &str,
    event_tx: &mpsc::Sender<Vec<u8>>,
    mobile_crypto_token: Option<&str>,
    instructions: &[nola_core::agent::host::PermissionInstruction],
    allow_conversation: bool,
    wait_duration: StdDuration,
) -> ToolPermissionResolution {
    wait_for_host_permission_in_run(
        request,
        host_id,
        client_id,
        session_id,
        event_tx,
        mobile_crypto_token,
        instructions,
        allow_conversation,
        wait_duration,
        None,
    )
}

// A resumed delivery is bound to the canonical Store Run. Core Turn and
// diagnostic trace run identities remain unchanged inside their own events.
fn wait_for_host_permission_in_run(
    request: &ToolRequest,
    host_id: &str,
    client_id: &str,
    session_id: &str,
    event_tx: &mpsc::Sender<Vec<u8>>,
    mobile_crypto_token: Option<&str>,
    instructions: &[nola_core::agent::host::PermissionInstruction],
    allow_conversation: bool,
    wait_duration: StdDuration,
    canonical_run_id: Option<&str>,
) -> ToolPermissionResolution {
    let Some(tool_catalog_evidence) = crate::host_tool_permission_evidence::current() else {
        return ToolPermissionResolution::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    };
    let safe_review_text = host_permission_safe_review_text(request);
    // This envelope is delivered to browser clients and persisted for replay.
    // Core has the immutable ToolRequest; Web presentation does not need raw
    // args. Encrypted mobile transports retain their existing preview contract.
    let trace_args = if mobile_crypto_token.is_some() {
        Value::Object(request.args.clone())
    } else {
        Value::Object(serde_json::Map::new())
    };
    let nonce = nola_core::mobile_crypto::generate_protocol_nonce("permission");
    let expires_at = unix_time_seconds().saturating_add(wait_duration.as_secs().max(1));
    let challenge = HostPermissionChallenge {
        allow_conversation,
        protocol_version: HOST_PERMISSION_PROTOCOL_VERSION,
        nonce: nonce.clone(),
        request_id: request.id.clone(),
        host_id: host_id.to_string(),
        client_id: client_id.to_string(),
        session_id: session_id.to_string(),
        tool_name: request.name.clone(),
        expires_at,
        permission_instructions: instructions.to_vec(),
    };
    let _registry_cleanup = HostPermissionRegistryCleanup(nonce.clone());
    let (decision_tx, decision_rx) = mpsc::sync_channel(1);
    if let Ok(mut challenges) = HOST_PERMISSION_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        let now = unix_time_seconds();
        challenges.retain(|_, item| item.expires_at > now);
        challenges.insert(nonce.clone(), challenge);
    } else {
        if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
            eprintln!("[permission-route] challenge registry unavailable");
        }
        return ToolPermissionResolution::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
        );
    }
    if let Ok(mut waiters) = HOST_PERMISSION_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        waiters.insert(nonce.clone(), decision_tx);
    } else {
        if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
            eprintln!("[permission-route] permission waiter registry unavailable");
        }
        return ToolPermissionResolution::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
        );
    }
    let reason = "This operation can make an irreversible change.";
    let Some(trace) = take_pending_host_permission_trace(session_id, request, &trace_args, reason)
    else {
        if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
            eprintln!("[permission-route] Core permission trace unavailable");
        }
        return ToolPermissionResolution::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
        );
    };
    let Some(call_id) = trace
        .pointer("/payload/call_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
    else {
        return ToolPermissionResolution::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
        );
    };
    let mut event = json!({
        "type": "permission_request",
        "allow_conversation": allow_conversation,
        "protocol_version": HOST_PERMISSION_PROTOCOL_VERSION,
        "id": request.id,
        "request_id": request.id,
        "call_id": call_id,
        "nonce": nonce,
        "host_id": host_id,
        "client_id": client_id,
        "session_id": session_id,
        "tool_name": request.name,
        "tool_catalog_evidence": tool_catalog_evidence,
        "safe_review_text": safe_review_text,
        "match": request.name,
        "reason": reason,
        "message": "Choose whether to allow this operation once or for this conversation.",
        "expires_at": expires_at,
        "permission_instructions": instructions,
        "trace": trace,
    });
    if mobile_crypto_token.is_some() {
        event["args"] = Value::Object(request.args.clone());
    }
    if let Some(run_id) = canonical_run_id {
        event["canonical_run_id"] = json!(run_id);
        let owner = OBSERVED_WORKERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .ok()
            .and_then(|owners| {
                owners
                    .values()
                    .find(|o| {
                        o.conversation_id == session_id
                            && o.run_id == run_id
                            && o.client_id == client_id
                    })
                    .cloned()
            });
        if let Some(owner) = owner {
            if let Ok(mut events) = OBSERVED_PERMISSION_EVENTS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                let mut observed = event.clone();
                if let Some(fields) = observed.as_object_mut() {
                    // Re-observation needs the challenge, not transcript/tool
                    // bodies. Core pending facts independently prove ownership.
                    fields.remove("trace");
                    fields.remove("args");
                }
                events.insert(nonce.clone(), (owner, observed));
            }
        }
    }
    if let Some(trace_event) = event
        .get("trace")
        .cloned()
        .and_then(|value| serde_json::from_value::<AgentTraceEvent>(value).ok())
    {
        if let Ok(store) = NolaStore::open_default() {
            if let Err(error) = store.record_trace_event_envelope(&trace_event, &event) {
                if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
                    eprintln!("[permission-route] failed to persist permission envelope: {error}");
                }
            }
        }
    }
    if event_tx
        .send(mobile_sse_event(event, mobile_crypto_token))
        .is_err()
    {
        if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
            eprintln!(
                "[permission-route] permission stream disconnected; preserving the canonical ask for recovery"
            );
        }
        return ToolPermissionResolution::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
        );
    }
    if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
        eprintln!("[permission-route] permission challenge sent; awaiting decision");
    }
    let wait_deadline = Instant::now() + wait_duration;
    let resolution = loop {
        let remaining = wait_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break ToolPermissionResolution::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Timeout);
        }
        match decision_rx.recv_timeout(remaining.min(StdDuration::from_millis(250))) {
            Ok(resolution) => break resolution,
            Err(mpsc::RecvTimeoutError::Disconnected) => break ToolPermissionResolution::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Wake this physical permission wait only for a committed
                // cancellation of its exact Run. The Host must still return
                // and settle its actual target; this is not a stop receipt.
                if canonical_run_id.is_some_and(|run| web_permission_stop_committed(session_id, run).unwrap_or(false)) {
                    break ToolPermissionResolution::StopRequested;
                }
            }
        }
    };
    if std::env::var("NOLA_PERMISSION_DEBUG").ok().as_deref() == Some("1") {
        eprintln!("[permission-route] permission challenge resolved: {resolution:?}");
    }
    resolution
}

fn web_permission_stop_committed(session: &str, run: &str) -> Result<bool, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    permission_wait_stop_committed(&mut application, session, run)
}

pub(crate) fn permission_wait_stop_committed(
    application: &mut crate::application_port::CliApplicationPort,
    session: &str,
    run: &str,
) -> Result<bool, String> {
    if application.active_run(session)?.as_ref().map(|active| active.run_id.as_str()) != Some(run) {
        return Ok(false);
    }
    let execution: nola_core::agent::HostExecutionState = application.execute(
        nola_application::ApplicationCommand::LoadHostExecutionState { conversation_id: session.into() })?;
    Ok(execution.control.turn_status == nola_core::agent::AgentTurnStatus::Cancelling)
}

/// A non-empty review is an approval affordance, not an authorization grant.
/// Unknown or value-bearing operations intentionally have no preview: a label
/// such as "write file" without its hidden bytes cannot justify approval.
fn host_permission_safe_review_text(request: &ToolRequest) -> Option<String> {
    fn safe_target(value: &Value) -> Option<String> {
        let text = value.as_str()?.trim();
        if text.is_empty()
            || text.len() > 512
            || text.chars().any(char::is_control)
            || crate::session::redact_sensitive_text(text) != text
            || [
                "password",
                "passwd",
                "token",
                "secret",
                "credential",
                "api_key",
                "apikey",
            ]
            .iter()
            .any(|word| text.to_ascii_lowercase().contains(word))
        {
            return None;
        }
        Some(text.to_string())
    }
    match request.name.as_str() {
        "code_mode_wait" => {
            let input: nola_domain::code_mode::CodeCellWaitRequest =
                serde_json::from_value(Value::Object(request.args.clone())).ok()?;
            safe_target(&json!(input.cell_id))?;
            Some(format!("code_mode_wait\n{}", serde_json::to_string(&input).ok()?))
        }
        "code_mode_exec" => {
            let input: nola_domain::code_mode::CodeCellExecRequest =
                serde_json::from_value(Value::Object(request.args.clone())).ok()?;
            input.initial_wait_ms().ok()?;
            let source = &input.source;
            if source.trim().is_empty() || source.len() > 16 * 1024
                || source.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
                || crate::session::redact_sensitive_text(source) != *source { return None; }
            Some(format!("code_mode_exec\nComplete source:\n{source}\nRequested budget: {}\nNested tool calls still require their own Core permission checks.",
                serde_json::to_string(&input.budget).ok()?))
        }
        "mcp_connect" => {
            if request.args.len() != 1 { return None; }
            let reference = request.args.get("connection_ref")?.as_str()?;
            let resource = crate::mcp_host::connection_resource(reference).ok()?;
            let target = safe_target(&json!(resource.target))?;
            Some(format!("mcp_connect: {}\nTarget: {target}\nConfiguration SHA-256: {}\nConnection: {reference}\nConnect this configured server and discover its capabilities. Tool calls require their own permission decision.",
                resource.transport, resource.config_digest))
        }
        "render_ui" => {
            // Review the exact bounded publication request without disclosing
            // its form values. Core still validates every protocol message and
            // applies ownership/revision fences before committing any change.
            let input: nola_domain::generative_ui_tool::UiToolInput =
                serde_json::from_value(Value::Object(request.args.clone())).ok()?;
            input.validate().ok()?;
            let bytes = serde_json::to_vec(&input).ok()?;
            use sha2::{Digest, Sha256};
            Some(format!(
                "render_ui: publish UI changes in this conversation\nExpected revision: {}\nProtocol messages: {}\nRequest UTF-8 bytes: {}\nRequest SHA-256: {:x}\nCore validates the protocol before applying changes. This approval does not authorize form actions or business operations.",
                input.expected_revision, input.messages.len(), bytes.len(), Sha256::digest(&bytes)
            ))
        }
        "bash" | "bash_command_review" => {
            if crate::tools::validate_tool_request(request).is_some() { return None; }
            let command = safe_target(request.args.get("command").or_else(|| request.args.get("cmd"))?)?;
            let cwd = match request.args.get("cwd") { Some(value) => safe_target(value)?, None => "Project directory".into() };
            Some(format!("{}\nCommand: {command}\nWorking directory: {cwd}", request.name))
        }
        "agent_plan_file" => {
            if crate::tools::validate_tool_request(request).is_some() { return None; }
            let root = safe_target(request.args.get("root")?)?;
            let action = request.args.get("action").and_then(Value::as_str).unwrap_or("list");
            let slug = match request.args.get("slug") { Some(value) => safe_target(value)?, None => String::new() };
            let content = request.args.get("content").and_then(Value::as_str).unwrap_or("");
            use sha2::{Digest, Sha256};
            Some(format!("agent_plan_file: {action}\nProject: {root}\nPlan: {}\nPreview only: {}\nContent UTF-8 bytes: {}\nContent SHA-256: {:x}",
                slug, request.args.get("dry_run").and_then(Value::as_bool).unwrap_or(true),
                content.len(), Sha256::digest(content.as_bytes())))
        }
        "write_file" => {
            // Present only the already-bound operation, never its possibly
            // secret-bearing body. Unknown fields cannot silently add effects.
            if !request.args.keys().all(|key| matches!(key.as_str(),
                "path" | "content" | "dry_run" | "ephemeral" | "expected_sha256"))
                || ["dry_run", "ephemeral"].iter().any(|key|
                    request.args.get(*key).is_some_and(|value| !value.is_boolean())) {
                return None;
            }
            let raw_path = request.args.get("path")?.as_str()?;
            let path = safe_target(request.args.get("path")?)?;
            if path != raw_path { return None; }
            let content = request.args.get("content")?.as_str()?;
            let expected = match request.args.get("expected_sha256") {
                None => None,
                Some(value) => {
                    let digest = value.as_str()?;
                    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
                    Some(digest)
                }
            };
            use sha2::{Digest, Sha256};
            let mode = if request.args.get("dry_run") == Some(&Value::Bool(true)) { "preview only; no write" }
                else if expected.is_some() { "replace full file if expected version matches" }
                else { "create; changed existing content requires an expected version" };
            Some(format!("write_file: {}\nMode: {mode}\nUTF-8 bytes: {}\nContent SHA-256: {:x}{}",
                serde_json::to_string(&path).ok()?, content.len(), Sha256::digest(content.as_bytes()),
                expected.map(|digest| format!("\nExpected existing SHA-256: {digest}")).unwrap_or_default()))
        }
        "delete_file" => {
            if !request
                .args
                .keys()
                .all(|key| matches!(key.as_str(), "path" | "dry_run" | "expected_sha256"))
                || request
                    .args
                    .get("dry_run")
                    .is_some_and(|value| !value.is_boolean())
                || request.args.get("expected_sha256").is_some_and(|value| {
                    !value.as_str().is_some_and(|digest| {
                        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                })
            {
                return None;
            }
            let path = safe_target(request.args.get("path")?)?;
            let operation = if request.args.get("dry_run") == Some(&Value::Bool(true)) {
                "preview delete_file"
            } else {
                "delete_file"
            };
            Some(format!(
                "{operation}: {}",
                serde_json::to_string(&path).ok()?
            ))
        }
        "web_fetch" => {
            if !request
                .args
                .keys()
                .all(|key| matches!(key.as_str(), "url" | "max_chars"))
                || request.args.get("max_chars").is_some_and(|value| {
                    !value
                        .as_u64()
                        .is_some_and(|count| (1..=80_000).contains(&count))
                })
            {
                return None;
            }
            let target = safe_target(request.args.get("url")?)?;
            let url = reqwest::Url::parse(&target).ok()?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return None;
            }
            Some(format!(
                "web_fetch: {}",
                serde_json::to_string(&target).ok()?
            ))
        }
        _ => {
            if let Some(review) = crate::tools::builtin_permission_review_text(request) { return Some(review); }
            // Only Host-bound MCP routes qualify; a model-supplied tool name
            // or origin cannot turn an unknown operation into an approvable one.
            let evidence = crate::host_tool_permission_evidence::current()?;
            if evidence["tool"]["tool_id"] != request.name { return None; }
            let route: nola_core::agent::host::HostMcpToolRoute =
                serde_json::from_value(evidence["tool"]["mcp_route"].clone()).ok()?;
            let resource = crate::mcp_host::connection_resource(&route.connection_ref).ok()?;
            if resource.config_digest != route.config_digest || resource.generation.is_none() { return None; }
            // Preview simple safe arguments only. Nested bodies, scripts and
            // sensitive values retain the existing fail-closed presentation.
            for (key, value) in &request.args {
                safe_target(&json!(key))?;
                match value {
                    Value::String(_) => { safe_target(value)?; },
                    Value::Null | Value::Bool(_) | Value::Number(_) => {},
                    _ => return None,
                }
            }
            let arguments = serde_json::to_string_pretty(&request.args).ok()?;
            if arguments.len() > 4096 { return None; }
            Some(format!("MCP: {} / {}\nTarget: {}\nArguments: {arguments}\nConfiguration SHA-256: {}",
                safe_target(&json!(route.server_id))?, safe_target(&json!(route.tool_name))?,
                safe_target(&json!(resource.target))?, route.config_digest))
        },
    }
}

#[derive(Clone)]
struct PendingDeviceToolChallenge {
    request: nola_core::device_bridge::DeviceToolRequest,
    receipt: nola_core::tool_policy::RuntimeResultEgressReceipt,
    grant: nola_core::tool_policy::ResultEgressGrant,
}

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

    if local_models::contains_model(&model) {
        let Some(local_endpoint) = local_models::running_endpoint(&model) else {
            return Err(json_response(
                StatusCode(503),
                json!({
                    "error": "The selected local model is not running. Start it in Model center first.",
                    "modelId": model,
                    "backend": "llama.cpp",
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
    let resolved_context = if local_models::contains_model(&model) {
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
                if item.get("source").and_then(Value::as_str) == Some("custom") {
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
                }
                let directory = builtin_skills::managed_directory_for_id(id)?
                    .filter(|directory| directory.join("SKILL.md").is_file())
                    .ok_or_else(|| format!("Selected Skill {id:?} is not materialized; save its settings to activate it"))?;
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
