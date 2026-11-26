// SPDX-License-Identifier: Apache-2.0
//! Scheduled Automations & External Messaging Channels module
//! (Slack, Discord, Telegram, WeCom/WeChat, Feishu/Lark, DingTalk with HMAC-SHA256 signing,
//! WhatsApp Cloud API, Email webhook/relay, inbound AgentEngine turn execution, and background
//! messaging channel worker).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use nola_engine::{
    AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, ModelCaller,
};
use nola_llm::{LlmClient, LlmError};
use nola_store::{Store, Vault, DEFAULT_MAX_EVENTS_PER_SCOPE};
use nola_tools::{SandboxMode, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, AutomationRecord, CapabilityDomain, FrozenSystemBlock, HostCapabilities,
    MessagingChannel, ModelRouteConfig, ModelStreamDelta, NormalizedModelResponse,
    StructuredModelRequest, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::ModulesSuite;

pub const KIND_AUTOMATION: &str = "automation";
pub const KIND_MESSAGING: &str = "messaging_channel";

const WORKER_KV_KEY: &str = "messaging_channel_worker";
const OUTBOUND_QUEUE_KV_KEY: &str = "messaging_outbound_queue";

/// Constant-time, fail-closed inbound webhook token verification.
/// Returns `false` whenever `expected_secret` is blank/whitespace-only or differs from `provided`.
pub fn verify_inbound_webhook_token(expected_secret: &str, provided: &str) -> bool {
    let expected = expected_secret.trim();
    if expected.is_empty() || provided.is_empty() {
        return false;
    }
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    let max_len = a.len().max(b.len());
    let mut diff: u8 = if a.len() == b.len() { 0 } else { 1 };
    for i in 0..max_len {
        let ba = a.get(i).copied().unwrap_or(0);
        let bb = b.get(i).copied().unwrap_or(0);
        diff |= ba ^ bb;
    }
    diff == 0
}

/// Pure RFC 2104 HMAC-SHA256 using `sha2::Sha256`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out = outer.finalize();
    let mut res = [0u8; 32];
    res.copy_from_slice(&out);
    res
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Sign a DingTalk webhook URL using HMAC-SHA256(`"{timestamp_ms}\n{secret}"`),
/// appending `timestamp` and URL-encoded `sign` query parameters.
pub fn sign_dingtalk_webhook_url(
    base_url: &str,
    secret: &str,
    timestamp_ms: i64,
) -> Result<String, String> {
    let mut parsed = url::Url::parse(base_url).map_err(|e| format!("invalid DingTalk URL: {e}"))?;
    let string_to_sign = format!("{timestamp_ms}\n{secret}");
    let mac = hmac_sha256(secret.as_bytes(), string_to_sign.as_bytes());
    let b64 = base64_encode(&mac);
    parsed
        .query_pairs_mut()
        .append_pair("timestamp", &timestamp_ms.to_string())
        .append_pair("sign", &b64);
    Ok(parsed.to_string())
}

/// Build an outbound webhook JSON payload formatted for Slack, Discord, Telegram,
/// WeCom/WeChat, Feishu/Lark, DingTalk, WhatsApp Cloud API, Email relay, or generic webhooks.
pub fn build_outbound_webhook_payload(kind: &str, text: &str, chat_id: Option<&str>) -> Value {
    match kind.trim().to_ascii_lowercase().as_str() {
        "slack" => json!({ "text": text }),
        "discord" => json!({ "content": text }),
        "telegram" => json!({
            "chat_id": chat_id.unwrap_or(""),
            "text": text
        }),
        "wechat" | "wecom" => json!({
            "msgtype": "text",
            "text": {
                "content": text
            }
        }),
        "feishu" | "lark" => json!({
            "msg_type": "text",
            "content": {
                "text": text
            }
        }),
        "dingtalk" => json!({
            "msgtype": "text",
            "text": {
                "content": text
            }
        }),
        "whatsapp" => json!({
            "messaging_product": "whatsapp",
            "to": chat_id.unwrap_or(""),
            "type": "text",
            "text": {
                "body": text
            }
        }),
        "email" => json!({
            "to": chat_id.unwrap_or(""),
            "subject": "Nola Notification",
            "text": text
        }),
        _ => json!({ "text": text, "content": text }),
    }
}

/// Parse an inbound webhook payload from Slack, Discord, Telegram, WeCom/WeChat,
/// Feishu/Lark, DingTalk, WhatsApp Cloud API, Email relay, or generic JSON webhook.
pub fn parse_inbound_webhook_payload(kind: &str, payload: &Value) -> Result<Value, String> {
    let norm_kind = kind.trim().to_ascii_lowercase();
    let (text, chat_id, sender) = match norm_kind.as_str() {
        "slack" => {
            let txt = payload
                .get("text")
                .or_else(|| payload.pointer("/event/text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("channel")
                .or_else(|| payload.pointer("/event/channel"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .get("user_name")
                .or_else(|| payload.get("user"))
                .or_else(|| payload.pointer("/event/user"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        "discord" => {
            let txt = payload
                .get("content")
                .or_else(|| payload.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("channel_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .pointer("/author/username")
                .or_else(|| payload.get("username"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        "telegram" => {
            let txt = payload
                .get("text")
                .or_else(|| payload.pointer("/message/text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("chat_id")
                .or_else(|| payload.pointer("/message/chat/id"))
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty());
            let user = payload
                .pointer("/message/from/username")
                .or_else(|| payload.get("from"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        "wechat" | "wecom" => {
            let txt = payload
                .pointer("/text/content")
                .or_else(|| payload.get("Content"))
                .or_else(|| payload.get("content"))
                .or_else(|| payload.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("ChatId")
                .or_else(|| payload.get("chat_id"))
                .or_else(|| payload.get("FromUserName"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .get("FromUserName")
                .or_else(|| payload.get("sender"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        "feishu" | "lark" => {
            let raw_content = payload
                .pointer("/event/message/content")
                .or_else(|| payload.pointer("/content/text"))
                .or_else(|| payload.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let txt = serde_json::from_str::<Value>(raw_content)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(ToString::to_string))
                .unwrap_or_else(|| raw_content.to_string());
            let ch = payload
                .pointer("/event/message/chat_id")
                .or_else(|| payload.get("chat_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .pointer("/event/sender/sender_id/open_id")
                .or_else(|| payload.get("sender"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt, ch, user)
        }
        "dingtalk" => {
            let txt = payload
                .pointer("/text/content")
                .or_else(|| payload.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("conversationId")
                .or_else(|| payload.get("chat_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .get("senderNick")
                .or_else(|| payload.get("senderId"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        "whatsapp" => {
            let txt = payload
                .pointer("/entry/0/changes/0/value/messages/0/text/body")
                .or_else(|| payload.pointer("/text/body"))
                .or_else(|| payload.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .pointer("/entry/0/changes/0/value/messages/0/from")
                .or_else(|| payload.get("from"))
                .or_else(|| payload.get("chat_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .pointer("/entry/0/changes/0/value/contacts/0/profile/name")
                .or_else(|| payload.get("from"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        "email" => {
            let txt = payload
                .get("text")
                .or_else(|| payload.get("body"))
                .or_else(|| payload.get("subject"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("from")
                .or_else(|| payload.get("reply_to"))
                .or_else(|| payload.get("chat_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .get("from")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
        _ => {
            let txt = payload
                .get("text")
                .or_else(|| payload.get("content"))
                .or_else(|| payload.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ch = payload
                .get("chat_id")
                .or_else(|| payload.get("channel"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let user = payload
                .get("sender")
                .or_else(|| payload.get("user"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            (txt.to_string(), ch, user)
        }
    };

    if text.trim().is_empty() {
        return Err(format!(
            "inbound '{norm_kind}' webhook payload missing text/content"
        ));
    }

    Ok(json!({
        "kind": if norm_kind.is_empty() { "webhook" } else { norm_kind.as_str() },
        "text": text,
        "chat_id": chat_id,
        "sender": sender,
        "raw": payload
    }))
}

/// Send an outbound message through any configured `MessagingChannel` protocol
/// with SSRF validation, DNS pinning, DingTalk HMAC-SHA256 signing, and WhatsApp/Email Bearer auth.
pub fn send_channel_message(
    channel: &MessagingChannel,
    text: &str,
    chat_id: Option<&str>,
    secret_or_token: Option<&str>,
) -> Result<Value, String> {
    if !channel.enabled {
        return Err(format!("messaging channel '{}' is disabled", channel.name));
    }
    let kind_norm = channel.kind.trim().to_ascii_lowercase();
    let target_url = if kind_norm == "dingtalk" {
        if let Some(secret) = secret_or_token.map(str::trim).filter(|s| !s.is_empty()) {
            sign_dingtalk_webhook_url(&channel.endpoint, secret, now_ms())?
        } else {
            channel.endpoint.clone()
        }
    } else {
        channel.endpoint.clone()
    };

    let (parsed, host, addrs) = nola_tools::web::resolve_and_validate_public_url(&target_url)?;
    let body = build_outbound_webhook_payload(&channel.kind, text, chat_id);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &addrs)
        .user_agent("Nola/2.0")
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.post(parsed).json(&body);
    if matches!(kind_norm.as_str(), "whatsapp" | "email" | "feishu" | "lark") {
        if let Some(token) = secret_or_token.map(str::trim).filter(|s| !s.is_empty()) {
            req = req.bearer_auth(token);
        }
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(json!({
            "delivered": true,
            "channel": channel.name,
            "kind": channel.kind,
            "status": status.as_u16()
        }))
    } else {
        Err(format!("channel '{}' returned HTTP {}", channel.name, status))
    }
}

struct MessagingLlmCaller {
    client: Arc<LlmClient>,
}

impl ModelCaller for MessagingLlmCaller {
    fn call(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        self.client.complete(route, req, stream, on_delta)
    }
}

fn sanitize_thread_segment(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if clean.is_empty() {
        "default".into()
    } else {
        clean
    }
}

/// Normalize an inbound messaging webhook payload, log the event, execute an `AgentEngine`
/// turn in session `channel_{channel_id}_{thread}`, and deliver the assistant reply back
/// through the channel's outbound adapter.
pub fn handle_inbound_webhook_turn(
    store: &Store,
    route: ModelRouteConfig,
    workspace_root: &Path,
    channel_id: &str,
    payload: &Value,
) -> Result<Value, String> {
    handle_inbound_webhook_turn_with_secret(store, route, workspace_root, channel_id, payload, None)
}

/// Variant of `handle_inbound_webhook_turn` that forwards the resolved Vault secret/token
/// (for DingTalk HMAC-SHA256 signing, WhatsApp/Feishu/Email bearer auth, etc.) to outbound replies.
pub fn handle_inbound_webhook_turn_with_secret(
    store: &Store,
    route: ModelRouteConfig,
    workspace_root: &Path,
    channel_id: &str,
    payload: &Value,
    secret_or_token: Option<&str>,
) -> Result<Value, String> {
    let client = LlmClient::new(120).map_err(|e| e.to_string())?;
    let caller: Arc<dyn ModelCaller> = Arc::new(MessagingLlmCaller {
        client: Arc::new(client),
    });
    handle_inbound_webhook_turn_with_caller(
        store,
        caller,
        route,
        workspace_root,
        channel_id,
        payload,
        secret_or_token,
    )
}

/// Testable variant of `handle_inbound_webhook_turn` accepting a custom `ModelCaller` and optional secret/token.
pub fn handle_inbound_webhook_turn_with_caller(
    store: &Store,
    caller: Arc<dyn ModelCaller>,
    route: ModelRouteConfig,
    workspace_root: &Path,
    channel_id: &str,
    payload: &Value,
    secret_or_token: Option<&str>,
) -> Result<Value, String> {
    let channel = store
        .get_module_record::<MessagingChannel>(KIND_MESSAGING, channel_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("messaging channel '{channel_id}' not found"))?;
    if !channel.enabled {
        return Err(format!("messaging channel '{}' is disabled", channel.name));
    }

    let normalized = parse_inbound_webhook_payload(&channel.kind, payload)?;
    let user_text = normalized["text"].as_str().unwrap_or("").to_string();
    let chat_id = normalized
        .get("chat_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let thread = sanitize_thread_segment(chat_id.as_deref().unwrap_or("default"));
    let session_id = format!("channel_{}_{}", sanitize_thread_segment(channel_id), thread);

    let _ = store.append_event(
        &session_id,
        "messaging_inbound",
        json!({
            "channel_id": channel.id,
            "channel_kind": channel.kind,
            "chat_id": chat_id,
            "sender": normalized.get("sender"),
            "text": user_text
        }),
        DEFAULT_MAX_EVENTS_PER_SCOPE,
    );

    let suite = Arc::new(ModulesSuite::new(
        store.clone(),
        workspace_root.to_path_buf(),
        None,
    ));
    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);
    let engine = AgentEngine::new(store.clone(), Arc::new(registry), caller);

    let policy = EffectivePolicy::default();
    let mut ctx = ToolContext::new(workspace_root).with_session_id(&session_id);
    ctx.permission_profile = policy.profile;
    ctx.network_allowed = policy.network_allowed;
    ctx.sandbox_mode = SandboxMode::from_env();

    let hooks = HookRunner::load(workspace_root, None, policy.workspace_trusted);
    let run_cfg = AgentRunConfig {
        session_id: Some(session_id.clone()),
        resume: true,
        route,
        frozen_system_blocks: vec![FrozenSystemBlock {
            id: "messaging_channel".into(),
            text: format!(
                "You are replying to a user message received via Nola Messaging Channel '{}' ({}). Be concise and helpful.",
                channel.name, channel.kind
            ),
        }],
        host_capabilities: HostCapabilities {
            interactive_user_input: false,
            network_allowed: policy.network_allowed,
        },
        policy,
        hooks,
        approval: None,
        max_steps: 16,
        force_tool_set: ForceToolSet::Auto,
        stream: false,
        json_schema: None,
        cancel: Arc::new(AtomicBool::new(false)),
        auto_recall_memory: true,
        inbox: None,
    };

    let run = engine
        .run(&ctx, &user_text, run_cfg, |_| {})
        .map_err(|e| e.to_string())?;
    let reply_text = run.final_text;

    let delivery = if channel.endpoint.trim().is_empty() {
        json!({ "delivered": false, "skipped": true, "reason": "channel has no outbound endpoint" })
    } else {
        match send_channel_message(&channel, &reply_text, chat_id.as_deref(), secret_or_token) {
            Ok(v) => v,
            Err(err) => json!({ "delivered": false, "error": err }),
        }
    };

    let _ = store.append_event(
        &session_id,
        "messaging_outbound",
        json!({
            "channel_id": channel.id,
            "channel_kind": channel.kind,
            "chat_id": chat_id,
            "text": reply_text,
            "delivery": delivery
        }),
        DEFAULT_MAX_EVENTS_PER_SCOPE,
    );

    Ok(json!({
        "ok": true,
        "channel_id": channel.id,
        "session_id": session_id,
        "inbound": normalized,
        "reply": reply_text,
        "delivery": delivery
    }))
}

fn resolve_channel_secret(store: &Store, vault: Option<&Arc<Vault>>, channel_id: &str) -> Option<String> {
    let key = format!("messaging_secret:{channel_id}");
    if let Some(v) = vault {
        if let Ok(Some(secret)) = v.get_secret(&key) {
            if !secret.trim().is_empty() {
                return Some(secret);
            }
        }
    }
    store
        .get_kv(&key)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(ToString::to_string))
        .filter(|s| !s.trim().is_empty())
}

/// Spawn a real background messaging channel worker that reads `kv["messaging_channel_worker"]`,
/// polls configured Telegram `getUpdates` / queued outbound jobs, runs inbound turns via
/// `handle_inbound_webhook_turn`, and updates worker heartbeat/stats in KV.
pub fn spawn_messaging_channel_worker<F>(
    store: Store,
    vault: Option<Arc<Vault>>,
    route_provider: F,
    workspace_root: PathBuf,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    F: Fn() -> Option<ModelRouteConfig> + Send + 'static,
{
    std::thread::Builder::new()
        .name("nola-messaging-worker".into())
        .spawn(move || {
            let mut polls: u64 = 0;
            let mut messages_processed: u64 = 0;
            let mut outbound_sent: u64 = 0;
            let mut last_error: Option<String> = None;

            while !shutdown.load(Ordering::Relaxed) {
                polls = polls.saturating_add(1);
                let mut cfg = store
                    .get_kv(WORKER_KV_KEY)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({}));
                let enabled = cfg
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let poll_secs = cfg
                    .get("poll_interval_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5)
                    .clamp(1, 300);

                if enabled {
                    // 1. Drain any queued outbound jobs in kv["messaging_outbound_queue"]
                    if let Ok(Some(Value::Array(queue))) = store.get_kv(OUTBOUND_QUEUE_KV_KEY) {
                        if !queue.is_empty() {
                            let mut remaining = Vec::new();
                            for job in queue {
                                let ch_id = job
                                    .get("channel_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let text = job.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                let chat_id = job.get("chat_id").and_then(|v| v.as_str());
                                if let Ok(Some(ch)) =
                                    store.get_module_record::<MessagingChannel>(KIND_MESSAGING, ch_id)
                                {
                                    let secret =
                                        resolve_channel_secret(&store, vault.as_ref(), &ch.id);
                                    match send_channel_message(
                                        &ch,
                                        text,
                                        chat_id,
                                        secret.as_deref(),
                                    ) {
                                        Ok(_) => {
                                            outbound_sent = outbound_sent.saturating_add(1);
                                        }
                                        Err(e) => {
                                            last_error = Some(e);
                                            let attempts = job
                                                .get("attempts")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0)
                                                + 1;
                                            if attempts < 3 {
                                                let mut retry_job = job.clone();
                                                if let Some(obj) = retry_job.as_object_mut() {
                                                    obj.insert("attempts".into(), json!(attempts));
                                                }
                                                remaining.push(retry_job);
                                            }
                                        }
                                    }
                                }
                            }
                            let _ = store.set_kv(OUTBOUND_QUEUE_KV_KEY, &Value::Array(remaining));
                        }
                    }

                    // 2. Poll Telegram `getUpdates` for enabled telegram channels with a bot token
                    let channels: Vec<MessagingChannel> = store
                        .list_module_records(KIND_MESSAGING)
                        .unwrap_or_default();
                    for ch in channels {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        if !ch.enabled || !ch.kind.eq_ignore_ascii_case("telegram") {
                            continue;
                        }
                        let Some(bot_token) = resolve_channel_secret(&store, vault.as_ref(), &ch.id)
                        else {
                            continue;
                        };
                        let offset_key = format!("telegram_offset:{}", ch.id);
                        let next_offset = store
                            .get_kv(&offset_key)
                            .ok()
                            .flatten()
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let poll_url = format!(
                            "https://api.telegram.org/bot{bot_token}/getUpdates?offset={next_offset}&timeout=0&limit=10"
                        );
                        let resp = reqwest::blocking::Client::builder()
                            .timeout(Duration::from_secs(10))
                            .redirect(reqwest::redirect::Policy::none())
                            .build()
                            .and_then(|c| c.get(&poll_url).send());
                        match resp {
                            Ok(r) if r.status().is_success() => {
                                if let Ok(body) = r.json::<Value>() {
                                    if let Some(updates) =
                                        body.get("result").and_then(|v| v.as_array())
                                    {
                                        let mut max_id = next_offset;
                                        for upd in updates {
                                            if let Some(uid) =
                                                upd.get("update_id").and_then(|v| v.as_i64())
                                            {
                                                max_id = max_id.max(uid + 1);
                                            }
                                            if let Some(route) = route_provider() {
                                                match handle_inbound_webhook_turn_with_secret(
                                                    &store,
                                                    route,
                                                    &workspace_root,
                                                    &ch.id,
                                                    upd,
                                                    Some(bot_token.as_str()),
                                                ) {
                                                    Ok(_) => {
                                                        messages_processed =
                                                            messages_processed.saturating_add(1);
                                                    }
                                                    Err(e) => {
                                                        last_error = Some(e);
                                                    }
                                                }
                                            }
                                        }
                                        if max_id > next_offset {
                                            let _ = store.set_kv(&offset_key, &json!(max_id));
                                        }
                                    }
                                }
                            }
                            Ok(r) => {
                                last_error = Some(format!(
                                    "telegram poll HTTP {} for '{}'",
                                    r.status(),
                                    ch.id
                                ));
                            }
                            Err(e) => {
                                last_error = Some(e.to_string());
                            }
                        }
                    }
                }

                // Update worker heartbeat/stats in KV
                if !cfg.is_object() {
                    cfg = json!({});
                }
                if let Some(obj) = cfg.as_object_mut() {
                    obj.insert("running".into(), json!(true));
                    obj.insert("last_heartbeat_ms".into(), json!(now_ms()));
                    obj.insert("polls".into(), json!(polls));
                    obj.insert("messages_processed".into(), json!(messages_processed));
                    obj.insert("outbound_sent".into(), json!(outbound_sent));
                    obj.insert("last_error".into(), json!(last_error));
                }
                let _ = store.set_kv(WORKER_KV_KEY, &cfg);

                // Sleep poll_secs in 100ms slices for fast shutdown
                for _ in 0..(poll_secs * 10) {
                    if shutdown.load(Ordering::Relaxed) {
                        if let Some(obj) = cfg.as_object_mut() {
                            obj.insert("running".into(), json!(false));
                            obj.insert("last_heartbeat_ms".into(), json!(now_ms()));
                        }
                        let _ = store.set_kv(WORKER_KV_KEY, &cfg);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        })
        .expect("spawn nola-messaging-worker thread")
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(AutomationManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(MessagingManageTool {
        suite: Arc::clone(suite),
    }));
}

struct AutomationManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for AutomationManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "automation_manage".into(),
                description: "Create, list, enable/disable, or delete scheduled Automations.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "schedule": { "type": "string" },
                        "prompt": { "type": "string" },
                        "enabled": { "type": "boolean" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "create" => {
                let rec = AutomationRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Automation")
                        .to_string(),
                    schedule: args
                        .get("schedule")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0 * * * *")
                        .to_string(),
                    prompt: args
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    workspace_root: self.suite.workspace_root.display().to_string(),
                    enabled: args.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    last_run_ms: None,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_AUTOMATION, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Saved automation '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("automation_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<AutomationRecord>(KIND_AUTOMATION) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "automations": items}),
                        format!("Listed {count} automation(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("automation_error", e.to_string(), None),
            },
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_AUTOMATION, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted automation '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("automation_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported automation action '{other}'"),
                None,
            ),
        }
    }
}

struct MessagingManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for MessagingManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "messaging_manage".into(),
                description: "Configure, list, or send notifications through Messaging Channels (webhook/Slack/Discord/Telegram/WeCom/Feishu/DingTalk/WhatsApp/Email).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "send", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "endpoint": { "type": "string" },
                        "chat_id": { "type": "string" },
                        "secret": { "type": "string", "description": "Optional DingTalk HMAC secret or WhatsApp/Email Bearer token" },
                        "text": { "type": "string" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Network,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "create" => {
                let ch = MessagingChannel {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    kind: args
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("webhook")
                        .to_string(),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Channel")
                        .to_string(),
                    endpoint: args
                        .get("endpoint")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    enabled: true,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_MESSAGING, &ch.id, &ch) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&ch).unwrap_or(json!({})),
                        format!("Saved messaging channel '{}'", ch.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("messaging_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<MessagingChannel>(KIND_MESSAGING) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "channels": items}),
                        format!("Listed {count} messaging channel(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("messaging_error", e.to_string(), None),
            },
            "send" => {
                if !ctx.network_allowed {
                    return ToolExecutionResult::err_structured(
                        "network_denied",
                        "outbound messaging blocked by network policy",
                        None,
                    );
                }
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let chat_id = args.get("chat_id").and_then(|v| v.as_str());
                let secret = args.get("secret").and_then(|v| v.as_str());
                let Ok(Some(ch)) = self.suite.get_record::<MessagingChannel>(KIND_MESSAGING, id) else {
                    return ToolExecutionResult::err_structured("not_found", format!("channel '{id}' not found"), None);
                };
                match send_channel_message(&ch, text, chat_id, secret) {
                    Ok(res) => ToolExecutionResult::ok_structured(
                        res,
                        format!("Delivered message to '{}'", ch.name),
                    ),
                    Err(e) => {
                        let code = if e.contains("disabled") {
                            "channel_disabled"
                        } else if e.contains("private") || e.contains("loopback") || e.contains("SSRF") {
                            "ssrf_blocked"
                        } else {
                            "delivery_failed"
                        };
                        ToolExecutionResult::err_structured(code, e, None)
                    }
                }
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_MESSAGING, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted channel '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("messaging_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported messaging action '{other}'"),
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_types::{ModelCapabilities, ModelProtocol, ModelStopReason, TokenUsage};

    struct StaticReplyCaller(String);

    impl ModelCaller for StaticReplyCaller {
        fn call(
            &self,
            _route: &ModelRouteConfig,
            _req: &StructuredModelRequest,
            _stream: bool,
            _on_delta: &mut dyn FnMut(ModelStreamDelta),
        ) -> Result<NormalizedModelResponse, LlmError> {
            Ok(NormalizedModelResponse {
                response_id: Some(new_id()),
                response_model: Some("mock".into()),
                content: self.0.clone(),
                reasoning: None,
                tool_calls: Vec::new(),
                stop_reason: ModelStopReason::Stop,
                usage: TokenUsage::default(),
                continuation: None,
            })
        }
    }

    #[test]
    fn webhook_token_verification_is_fail_closed_and_payloads_normalize() {
        // Fail-closed on empty/whitespace secret
        assert!(!verify_inbound_webhook_token("", ""));
        assert!(!verify_inbound_webhook_token("   ", "anything"));
        assert!(!verify_inbound_webhook_token("secret-123", ""));
        assert!(!verify_inbound_webhook_token("secret-123", "secret-124"));
        assert!(!verify_inbound_webhook_token("secret-123", "secret-1234"));
        assert!(verify_inbound_webhook_token("secret-123", "secret-123"));

        // RFC 4231 Test Case 2 for HMAC-SHA256: key = "Jefe", data = "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );

        let signed = sign_dingtalk_webhook_url(
            "https://oapi.dingtalk.com/robot/send?access_token=abc",
            "SECsecret",
            1_700_000_000_000,
        )
        .unwrap();
        assert!(signed.contains("timestamp=1700000000000"));
        assert!(signed.contains("sign="));

        assert_eq!(
            build_outbound_webhook_payload("slack", "deploy ok", None),
            json!({"text": "deploy ok"})
        );
        assert_eq!(
            build_outbound_webhook_payload("discord", "deploy ok", None),
            json!({"content": "deploy ok"})
        );
        assert_eq!(
            build_outbound_webhook_payload("telegram", "deploy ok", Some("9988")),
            json!({"chat_id": "9988", "text": "deploy ok"})
        );
        assert_eq!(
            build_outbound_webhook_payload("wecom", "hello wecom", None),
            json!({"msgtype": "text", "text": {"content": "hello wecom"}})
        );
        assert_eq!(
            build_outbound_webhook_payload("feishu", "hello lark", None),
            json!({"msg_type": "text", "content": {"text": "hello lark"}})
        );
        assert_eq!(
            build_outbound_webhook_payload("dingtalk", "hello dt", None),
            json!({"msgtype": "text", "text": {"content": "hello dt"}})
        );
        assert_eq!(
            build_outbound_webhook_payload("whatsapp", "hi wa", Some("+15551234567")),
            json!({
                "messaging_product": "whatsapp",
                "to": "+15551234567",
                "type": "text",
                "text": {"body": "hi wa"}
            })
        );
        assert_eq!(
            build_outbound_webhook_payload("email", "report body", Some("ops@example.com")),
            json!({"to": "ops@example.com", "subject": "Nola Notification", "text": "report body"})
        );

        let parsed_tg = parse_inbound_webhook_payload(
            "telegram",
            &json!({"chat_id": 12345, "text": "status?"}),
        )
        .unwrap();
        assert_eq!(parsed_tg["chat_id"], "12345");
        assert_eq!(parsed_tg["text"], "status?");

        let parsed_dc =
            parse_inbound_webhook_payload("discord", &json!({"content": "hello bot"})).unwrap();
        assert_eq!(parsed_dc["text"], "hello bot");

        let parsed_lark = parse_inbound_webhook_payload(
            "feishu",
            &json!({"event": {"message": {"chat_id": "oc_1", "content": "{\"text\":\"ping lark\"}"}}}),
        )
        .unwrap();
        assert_eq!(parsed_lark["text"], "ping lark");
        assert_eq!(parsed_lark["chat_id"], "oc_1");
    }

    #[test]
    fn inbound_webhook_turn_and_background_worker_operate_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        store
            .put_module_record(
                KIND_MESSAGING,
                "ch-feishu",
                &MessagingChannel {
                    id: "ch-feishu".into(),
                    kind: "feishu".into(),
                    name: "Ops Lark".into(),
                    endpoint: String::new(), // skip real outbound HTTP in unit test
                    enabled: true,
                    updated_at_ms: now_ms(),
                },
            )
            .unwrap();

        let route = ModelRouteConfig {
            connection_id: "mock".into(),
            endpoint: "http://127.0.0.1:1".into(),
            protocol: ModelProtocol::OpenAiChat,
            model: "mock-model".into(),
            api_key: String::new(),
            context_window: 32_768,
            max_output_tokens: Some(512),
            temperature: None,
            top_p: None,
            capabilities: ModelCapabilities::default(),
            reasoning_effort: None,
            prompt_cache_key: None,
        };
        let res = handle_inbound_webhook_turn_with_caller(
            &store,
            Arc::new(StaticReplyCaller("All systems green.".into())),
            route,
            dir.path(),
            "ch-feishu",
            &json!({"chat_id": "thread-7", "text": "status report?"}),
            None,
        )
        .unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["session_id"], "channel_ch-feishu_thread-7");
        assert_eq!(res["reply"], "All systems green.");

        let events = store
            .read_event_range("channel_ch-feishu_thread-7", 1, 20)
            .unwrap();
        assert!(events.iter().any(|e| e.kind == "messaging_inbound"));
        assert!(events.iter().any(|e| e.kind == "messaging_outbound"));

        // Spawn background worker, wait for heartbeat in KV, then shut down cleanly
        let shutdown = Arc::new(AtomicBool::new(false));
        let handle = spawn_messaging_channel_worker(
            store.clone(),
            None,
            || None,
            dir.path().to_path_buf(),
            Arc::clone(&shutdown),
        );
        std::thread::sleep(Duration::from_millis(150));
        shutdown.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        let kv = store.get_kv(WORKER_KV_KEY).unwrap().unwrap();
        assert!(kv["polls"].as_u64().unwrap() >= 1);
        assert_eq!(kv["running"], false);
    }
}
