// SPDX-License-Identifier: Apache-2.0
//! Pending human-in-the-loop approvals, questions, device-tool callbacks, and streaming channel reader.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use nola_tools::{ToolContext, ToolHandler};
use nola_types::{new_id, now_ms, sha256_hex, ToolExecutionResult, ToolSpec};
use serde_json::{json, Value};

use crate::web_api::ms_to_rfc3339;

use super::auth::{encode_sse_frame, generate_hex_token_32};

pub(super) struct ChannelReader {
    pub(super) rx: mpsc::Receiver<Vec<u8>>,
    pub(super) buf: Vec<u8>,
    pub(super) pos: usize,
    pub(super) cancel: Option<Arc<AtomicBool>>,
    pub(super) finished_eof: bool,
}

impl ChannelReader {
    pub(super) fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
            cancel: None,
            finished_eof: false,
        }
    }

    pub(super) fn with_cancel(rx: mpsc::Receiver<Vec<u8>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
            cancel: Some(cancel),
            finished_eof: false,
        }
    }
}

impl Drop for ChannelReader {
    fn drop(&mut self) {
        if !self.finished_eof {
            if let Some(ref c) = self.cancel {
                c.store(true, Ordering::SeqCst);
            }
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) if chunk.is_empty() => continue,
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => {
                    self.finished_eof = true;
                    return Ok(0);
                }
            }
        }
        let avail = &self.buf[self.pos..];
        let n = avail.len().min(out.len());
        out[..n].copy_from_slice(&avail[..n]);
        self.pos += n;
        Ok(n)
    }
}

pub(super) struct PendingApprovalEntry {
    pub(super) conversation_id: String,
    pub(super) tool_name: String,
    pub(super) target_key: String,
    pub(super) reason: String,
    pub(super) arguments: Value,
    pub(super) tx: mpsc::Sender<bool>,
}

pub(super) struct PendingQuestionEntry {
    pub(super) conversation_id: String,
    pub(super) question: String,
    pub(super) options: Vec<String>,
    pub(super) tx: mpsc::Sender<Option<String>>,
}

pub(super) struct PendingDeviceToolEntry {
    pub(super) conversation_id: String,
    pub(super) tx: mpsc::Sender<Value>,
}

/// Pending interactive approval, user-question, & hybrid device-tool waiters bridged
/// between SSE turns and REST, plus narrowly scoped `(conv_id, tool_name, target_key)` session grants.
#[derive(Default)]
pub(super) struct PendingInteractions {
    pub(super) approvals: Mutex<BTreeMap<String, PendingApprovalEntry>>,
    pub(super) questions: Mutex<BTreeMap<String, PendingQuestionEntry>>,
    pub(super) device_tools: Mutex<BTreeMap<String, PendingDeviceToolEntry>>,
    pub(super) session_grants: Mutex<HashSet<(String, String, String)>>,
}

impl PendingInteractions {
    pub(super) fn has_session_grant(&self, conv_id: &str, tool_name: &str, target_key: &str) -> bool {
        self.session_grants
            .lock()
            .unwrap()
            .contains(&(conv_id.to_string(), tool_name.to_string(), target_key.to_string()))
    }

    pub(super) fn resolve_approval(&self, request_id: &str, allow: bool, scope: &str) -> bool {
        if let Some(entry) = self.approvals.lock().unwrap().remove(request_id) {
            if allow && scope.eq_ignore_ascii_case("session") {
                self.session_grants.lock().unwrap().insert((
                    entry.conversation_id,
                    entry.tool_name,
                    entry.target_key,
                ));
            }
            let _ = entry.tx.send(allow);
            true
        } else {
            false
        }
    }

    pub(super) fn resolve_question(&self, request_id: &str, answer: Option<String>) -> bool {
        if let Some(entry) = self.questions.lock().unwrap().remove(request_id) {
            let _ = entry.tx.send(answer);
            true
        } else {
            false
        }
    }

    pub(super) fn register_device_tool(
        &self,
        request_id: String,
        conversation_id: String,
    ) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel::<Value>();
        self.device_tools.lock().unwrap().insert(
            request_id,
            PendingDeviceToolEntry {
                conversation_id,
                tx,
            },
        );
        rx
    }

    pub(super) fn resolve_device_tool(&self, request_id: &str, payload: Value) -> bool {
        if let Some(entry) = self.device_tools.lock().unwrap().remove(request_id) {
            let _ = entry.tx.send(payload);
            true
        } else {
            false
        }
    }

    pub(super) fn conversation_needs_response(&self, conv_id: &str) -> bool {
        let has_approval = self
            .approvals
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id);
        if has_approval {
            return true;
        }
        let has_question = self
            .questions
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id);
        if has_question {
            return true;
        }
        self.device_tools
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id)
    }

    pub(super) fn any_needs_response(&self) -> bool {
        !self.approvals.lock().unwrap().is_empty()
            || !self.questions.lock().unwrap().is_empty()
            || !self.device_tools.lock().unwrap().is_empty()
    }

    pub(super) fn pending_permission_for(&self, conv_id: &str) -> Value {
        self.approvals
            .lock()
            .unwrap()
            .iter()
            .find(|(_, e)| e.conversation_id == conv_id)
            .map(|(req_id, e)| {
                json!({
                    "requestId": req_id,
                    "request_id": req_id,
                    "tool": e.tool_name,
                    "reason": e.reason,
                    "arguments": e.arguments
                })
            })
            .unwrap_or(Value::Null)
    }

    pub(super) fn pending_permissions_list(&self, conv_id: &str) -> Vec<Value> {
        self.approvals
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| e.conversation_id == conv_id)
            .take(64)
            .map(|(req_id, e)| {
                json!({
                    "request_id": req_id,
                    "tool": e.tool_name,
                    "reason": e.reason,
                    "arguments": e.arguments
                })
            })
            .collect()
    }

    pub(super) fn pending_question_for(&self, conv_id: &str) -> Value {
        self.questions
            .lock()
            .unwrap()
            .iter()
            .find(|(_, e)| e.conversation_id == conv_id)
            .map(|(req_id, e)| {
                json!({
                    "requestId": req_id,
                    "request_id": req_id,
                    "question": e.question,
                    "options": e.options
                })
            })
            .unwrap_or(Value::Null)
    }

    pub(super) fn clear_conversation(&self, conv_id: &str) {
        self.session_grants
            .lock()
            .unwrap()
            .retain(|(cid, _, _)| cid != conv_id);
        self.approvals
            .lock()
            .unwrap()
            .retain(|_, entry| entry.conversation_id != conv_id);
        self.questions
            .lock()
            .unwrap()
            .retain(|_, entry| entry.conversation_id != conv_id);
        self.device_tools
            .lock()
            .unwrap()
            .retain(|_, entry| entry.conversation_id != conv_id);
    }
}

/// Hybrid device-tool proxy registered on a turn when a mobile/desktop client declares
/// client-executed tools. Emits SSE `device_tool_request` and awaits REST callback resolution.
pub(super) struct DeviceToolProxy {
    pub(super) spec: ToolSpec,
    pub(super) conversation_id: String,
    pub(super) host_id: String,
    pub(super) client_id: String,
    pub(super) capability: String,
    pub(super) tx: mpsc::Sender<Vec<u8>>,
    pub(super) pending: Arc<PendingInteractions>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) crypto_tok: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_device_tool_request_frame(
    req_id: &str,
    conversation_id: &str,
    host_id: &str,
    client_id: &str,
    tool_name: &str,
    capability: &str,
    arguments: &Value,
    nonce: &str,
    expires_at_ms: i64,
) -> Value {
    let args_obj = if arguments.is_object() {
        arguments.clone()
    } else {
        json!({})
    };
    json!({
        "type": "device_tool_request",
        "id": req_id,
        "request_id": req_id,
        "call_id": req_id,
        "conversation_id": conversation_id,
        "session_id": conversation_id,
        "host_id": host_id,
        "client_id": client_id,
        "tool_name": tool_name,
        "tool": tool_name,
        "capability": capability,
        "arguments": args_obj,
        "args": args_obj,
        "nonce": nonce,
        "expires_at": expires_at_ms,
        "expires_at_iso": ms_to_rfc3339(expires_at_ms),
        "protocol_version": 1,
        "trace": {
            "type": "device_tool_request",
            "id": req_id,
            "request_id": req_id,
            "tool_name": tool_name,
            "session_id": conversation_id
        }
    })
}

pub(super) fn device_tool_callback_is_ok(payload: &Value) -> bool {
    payload
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| match payload.get("error") {
            Some(Value::Bool(b)) => !b,
            Some(Value::Null) | None => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            Some(_) => false,
        })
}

impl ToolHandler for DeviceToolProxy {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let req_id = new_id();
        let nonce = generate_hex_token_32();
        let expires_at_ms = now_ms().saturating_add(120_000);
        let rx = self
            .pending
            .register_device_tool(req_id.clone(), self.conversation_id.clone());
        let frame = build_device_tool_request_frame(
            &req_id,
            &self.conversation_id,
            &self.host_id,
            &self.client_id,
            &self.spec.schema.name,
            &self.capability,
            &args,
            &nonce,
            expires_at_ms,
        );
        let _ = self
            .tx
            .send(encode_sse_frame(&frame, self.crypto_tok.as_deref()));

        let deadline = Instant::now() + Duration::from_secs(120);
        let resolved = loop {
            if self.cancel.load(Ordering::SeqCst) {
                break None;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(val) => break Some(val),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break None;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            }
        };
        self.pending.device_tools.lock().unwrap().remove(&req_id);
        match resolved {
            Some(payload) => {
                let is_ok = device_tool_callback_is_ok(&payload);
                if is_ok {
                    let result_val = payload
                        .get("result")
                        .or_else(|| payload.get("output"))
                        .or_else(|| payload.get("payload"))
                        .cloned()
                        .unwrap_or(payload);
                    let summary = result_val
                        .as_str()
                        .map(|s| s.chars().take(240).collect::<String>())
                        .unwrap_or_else(|| format!("Device tool '{}' completed", self.spec.schema.name));
                    ToolExecutionResult::ok_structured(result_val, summary)
                } else {
                    let msg = payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| {
                            payload
                                .get("output")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                        })
                        .unwrap_or("device tool execution failed");
                    ToolExecutionResult::err_structured("device_tool_error", msg, None)
                }
            }
            None => ToolExecutionResult::err_structured(
                "device_tool_timeout",
                format!(
                    "device tool '{}' timed out or was cancelled waiting for client callback",
                    self.spec.schema.name
                ),
                None,
            ),
        }
    }
}

pub(super) fn approval_target_key(args: &Value) -> String {
    for key in ["path", "file_path", "target_path", "command", "url", "endpoint"] {
        if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return format!("{key}:{trimmed}");
            }
        }
    }
    let raw = serde_json::to_string(args).unwrap_or_default();
    format!("args:{}", &sha256_hex(raw.as_bytes())[..16])
}

