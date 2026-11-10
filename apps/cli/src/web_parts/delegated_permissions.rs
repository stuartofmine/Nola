// SPDX-License-Identifier: Apache-2.0
// Remote transport for canonical child permission requests. The parent owns
// presentation; only the child request can authorize the child effect.
#[derive(Clone)]
struct DelegatedPermissionChallenge {
    owner: ObservedWorkerOwner,
    challenge: HostPermissionChallenge,
    task: crate::task_runtime::ParentTaskPermission,
    event: Value,
}
static DELEGATED_PERMISSIONS: OnceLock<Mutex<HashMap<String, DelegatedPermissionChallenge>>> =
    OnceLock::new();

fn observed_delegated_permissions(
    owner: &ObservedWorkerOwner,
    turn: &str,
) -> Result<Vec<Value>, String> {
    let Some(projection) = crate::session::load_session_task_projection(&owner.conversation_id)?
    else {
        return Ok(Vec::new());
    };
    let mut candidates = Vec::new();
    for task in projection.tasks.values() {
        if task.owner_turn_id.as_str() != turn {
            continue;
        }
        if let Some(permission) = crate::task_runtime::permission_for_parent_task(
            &owner.conversation_id,
            turn,
            &task.id.to_string(),
        )? {
            if permission.owner_run_id == owner.run_id {
                candidates.push(permission);
            }
        }
        if candidates.len() >= 16 {
            break;
        }
    }
    let now = unix_time_seconds();
    let mut registry = DELEGATED_PERMISSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "delegated permission registry unavailable")?;
    registry.retain(|_, value| value.challenge.expires_at > now);
    let mut events = Vec::new();
    for task in candidates {
        if let Some(existing) = registry.values().find(|item| {
            item.owner == *owner
                && item.task.request() == task.request()
                && item.task.source == task.source
        }) {
            events.push(existing.event.clone());
            continue;
        }
        if registry.len() >= 4096 {
            return Err("delegated permission registry capacity reached".into());
        }
        let request = ToolRequest {
            id: task.request().request_id.clone(),
            name: task.source.capability_id.clone(),
            args: task
                .source
                .arguments
                .as_object()
                .cloned()
                .unwrap_or_default(),
        };
        let nonce = nola_core::mobile_crypto::generate_protocol_nonce("permission");
        let challenge = HostPermissionChallenge {
            allow_conversation: task.source.allow_conversation,
            protocol_version: HOST_PERMISSION_PROTOCOL_VERSION,
            nonce: nonce.clone(),
            request_id: request.id.clone(),
            host_id: host_identity().to_owned(),
            client_id: owner.client_id.clone(),
            session_id: owner.conversation_id.clone(),
            tool_name: request.name.clone(),
            expires_at: now.saturating_add(300),
            permission_instructions: task.source.permission_instructions.clone(),
        };
        let event = json!({"type":"permission_request", "allow_conversation":challenge.allow_conversation,
            "protocol_version":challenge.protocol_version,"id":request.id,"request_id":request.id,
            "call_id":task.source.call_id,"nonce":nonce,"host_id":challenge.host_id,
            "client_id":owner.client_id,"session_id":owner.conversation_id,"canonical_run_id":owner.run_id,
            "tool_name":request.name,"safe_review_text":format!("{}\n{}", task.prompt.prompt.title, host_permission_safe_review_text(&request).unwrap_or_default()),
            "task_id":task.request().task_id,"match":request.name,
            "reason":"A delegated task requires your approval.","message":task.prompt.prompt.title,
            "expires_at":challenge.expires_at,"permission_instructions":challenge.permission_instructions});
        events.push(event.clone());
        registry.insert(
            nonce,
            DelegatedPermissionChallenge {
                owner: owner.clone(),
                challenge,
                task,
                event,
            },
        );
    }
    Ok(events)
}

fn resolve_delegated_permission(
    decision: &HostPermissionResponse,
) -> Result<Option<HostPermissionChallenge>, String> {
    let candidate = DELEGATED_PERMISSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "delegated permission registry unavailable")?
        .get(&decision.nonce)
        .cloned();
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    validate_host_permission_decision(&candidate.challenge, decision, unix_time_seconds())?;
    bind_observed_worker(&candidate.owner)?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let projection = observation_command(
        &mut application,
        json!({"type":"load_conversation_permission_projection_v1",
        "conversation_id":candidate.owner.conversation_id,"run_id":candidate.owner.run_id,"scope":{"type":"current_run"}}),
    )?;
    if projection.get("active_turn_id").and_then(Value::as_str)
        != Some(candidate.task.owner_turn_id.as_str())
    {
        return Err("delegated permission parent turn changed".into());
    }
    use crate::runtime_interaction::PermissionChoice;
    let choice = match &decision.resolution {
        WireHostPermissionResolution::Authorization {
            decision: HostPermissionAuthorizationDecision::AllowOnce,
            ..
        } => PermissionChoice::Once,
        WireHostPermissionResolution::Authorization {
            decision: HostPermissionAuthorizationDecision::AllowConversation,
            ..
        } => PermissionChoice::Session(candidate.task.prompt.prompt.session_value.clone()),
        WireHostPermissionResolution::Authorization {
            decision: HostPermissionAuthorizationDecision::Deny,
            ..
        } => PermissionChoice::Deny,
        WireHostPermissionResolution::Intervention {
            intervention: HostPermissionIntervention::ProvideInstruction { instruction },
        } => PermissionChoice::Custom(instruction.clone()),
        WireHostPermissionResolution::Intervention {
            intervention: HostPermissionIntervention::ReturnToComposer,
        } => PermissionChoice::ReturnToComposer,
    };
    crate::task_runtime::resolve_parent_task_permission(&candidate.task, choice)?;
    DELEGATED_PERMISSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "delegated permission registry unavailable")?
        .remove(&decision.nonce);
    Ok(Some(candidate.challenge))
}

// Keep delegated approvals observable while the parent is blocked inside a
// task wait. Runtime output callbacks alone cannot wake that wait's transport.
struct DelegatedPermissionStream {
    stop: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for DelegatedPermissionStream {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}
fn stream_delegated_permissions(
    owner: ObservedWorkerOwner,
    tx: mpsc::Sender<Vec<u8>>,
    token: Option<String>,
    receipt: Option<Arc<SubmissionReceiptFence>>,
) -> DelegatedPermissionStream {
    let (stop, stopped) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut sent = std::collections::HashSet::new();
        loop {
            match stopped.recv_timeout(StdDuration::from_millis(500)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {},
                _ => break,
            }
            let pending = (|| -> Result<Vec<Value>, String> {
                bind_observed_worker(&owner)?;
                let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
                let active = observation_command(&mut application, json!({"type":"active_run",
                    "conversation_id":owner.conversation_id}))?;
                if active.get("run_id").and_then(Value::as_str) != Some(owner.run_id.as_str()) {
                    return Ok(Vec::new());
                }
                let projection = observation_command(&mut application, json!({"type":"load_conversation_permission_projection_v1",
                    "conversation_id":owner.conversation_id,"run_id":owner.run_id,"scope":{"type":"current_run"}}))?;
                let Some(turn) = projection.get("active_turn_id").and_then(Value::as_str) else { return Ok(Vec::new()) };
                observed_delegated_permissions(&owner, turn)
            })();
            let Ok(mut events) = pending else { continue };
            let live: std::collections::HashSet<String> = events.iter()
                .filter_map(|event|event.get("nonce").and_then(Value::as_str).map(str::to_owned)).collect();
            sent.retain(|nonce|live.contains(nonce));
            // Remote clients present one decision card. Keep its request stable
            // until resolved instead of replacing it with a sibling's request.
            events.sort_by_key(|event| {
                let nonce = event.get("nonce").and_then(Value::as_str).unwrap_or("");
                (!sent.contains(nonce), nonce.to_owned())
            });
            for event in events.into_iter().take(1) {
                let Some(nonce) = event.get("nonce").and_then(Value::as_str).map(str::to_owned) else { continue };
                if sent.contains(&nonce) { continue; }
                if receipt.as_ref().is_some_and(|fence| fence.deliver(&tx, token.as_deref()).is_err()) { continue; }
                if tx.send(mobile_sse_event(event, token.as_deref())).is_err() { return; }
                sent.insert(nonce);
            }
        }
    });
    DelegatedPermissionStream { stop, worker: Some(worker) }
}
