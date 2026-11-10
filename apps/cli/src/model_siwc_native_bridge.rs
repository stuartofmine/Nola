// SPDX-License-Identifier: Apache-2.0
//! Private same-owner native lifecycle. No TTY parsing or token-bearing frames.
use crate::{
    model_siwc::{AuthorizationCancellation, SiwcHost, Snapshot},
    model_siwc_cli as lifecycle,
    store::NolaStore,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read, Write},
    sync::{Arc, Mutex},
};

const MAX_FRAME: u64 = 64 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlowIdentity {
    owner_id: String,
    flow_id: String,
    account_id: String,
    expected_account_revision: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Start {
    owner_id: String,
    account_id: String,
    expected_account_revision: Option<i64>,
    display_name: String,
    acknowledge_authorization: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountRequest {
    owner_id: String,
    account_id: String,
    expected_account_revision: i64,
    expected_session_revision: Option<i64>,
    acknowledge_sign_out: bool,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Hello {
        request_id: String,
    },
    Start {
        request_id: String,
        request: Start,
    },
    Status {
        request_id: String,
        request: FlowIdentity,
    },
    Cancel {
        request_id: String,
        request: FlowIdentity,
    },
    Finish {
        request_id: String,
        request: FlowIdentity,
    },
    AccountStatus {
        request_id: String,
        request: AccountRequest,
    },
    Models {
        request_id: String,
        request: AccountRequest,
    },
    Refresh {
        request_id: String,
        request: AccountRequest,
    },
    SignOut {
        request_id: String,
        request: AccountRequest,
    },
    Close {
        request_id: String,
    },
}

struct Flow {
    id: String,
    start_request_id: String,
    account: String,
    revision: Option<i64>,
    label: String,
    state: Mutex<FlowState>,
}
#[derive(Default)]
struct FlowState {
    cancellation: Option<AuthorizationCancellation>,
    cancelled: bool,
    authorization_url: Option<String>,
    result: Option<Result<Snapshot, String>>,
    committed: Option<Value>,
}
pub(crate) struct NativeSiwcService {
    owner: String,
    store: NolaStore,
    host: Arc<SiwcHost>,
    flows: BTreeMap<String, Arc<Flow>>,
}
impl NativeSiwcService {
    pub(crate) fn owner_id(&self) -> &str {
        &self.owner
    }
    pub(crate) fn new(store: NolaStore, host: SiwcHost) -> Self {
        Self {
            owner: uuid::Uuid::new_v4().to_string(),
            store,
            host: Arc::new(host),
            flows: BTreeMap::new(),
        }
    }
    fn owner(&self, owner: &str) -> Result<(), String> {
        if owner != self.owner {
            return Err("native_siwc_owner_changed".into());
        }
        Ok(())
    }
    fn current(&self, account: &str, revision: Option<i64>) -> Result<(), String> {
        let current = lifecycle::load(&self.store, account)?;
        match (current, revision) {
            (None, None) => Ok(()),
            (Some(value), Some(revision)) => lifecycle::owned(&value, Some(revision)),
            _ => Err("native_siwc_account_changed".into()),
        }
    }
    fn selected(&self, identity: &FlowIdentity) -> Result<Arc<Flow>, String> {
        self.owner(&identity.owner_id)?;
        let flow = self
            .flows
            .get(&identity.flow_id)
            .ok_or("native_siwc_flow_unknown")?;
        if flow.account != identity.account_id
            || flow.revision != identity.expected_account_revision
        {
            return Err("native_siwc_flow_identity_mismatch".into());
        }
        Ok(flow.clone())
    }
    fn project(&self, flow: &Flow) -> Result<Value, String> {
        let state = flow
            .state
            .lock()
            .map_err(|_| "native_siwc_state_unavailable")?;
        let phase = if state.committed.is_some() {
            "committed"
        } else if state.cancelled {
            "cancelled"
        } else {
            match &state.result {
                Some(Ok(_)) => "authorized",
                Some(Err(_)) => "failed",
                None if state.authorization_url.is_some() => "awaiting_authorization",
                None => "preparing",
            }
        };
        Ok(
            json!({"owner_id":self.owner,"flow_id":flow.id,"account_id":flow.account,
            "expected_account_revision":flow.revision,"phase":phase,
            "authorization_url":if phase=="awaiting_authorization" {state.authorization_url.as_deref()} else {None},
            "error":state.result.as_ref().and_then(|result|result.as_ref().err()),
            "private_session_may_be_retained":state.cancelled || matches!(&state.result,Some(Err(error))
                if flow.revision.is_some() || !matches!(error.as_str(),"native_siwc_authorization_declined" | "native_siwc_authorization_expired")),
            "account":state.committed,"inference_performed":false}),
        )
    }
    pub(crate) fn start(&mut self, request_id: &str, request: Start) -> Result<Value, String> {
        self.owner(&request.owner_id)?;
        if !request.acknowledge_authorization {
            return Err("native_siwc_confirmation_required".into());
        }
        let id = uuid::Uuid::parse_str(&request.account_id)
            .map_err(|_| "native_siwc_account_invalid")?;
        if id.to_string() != request.account_id
            || request.display_name.trim().is_empty()
            || request.display_name.len() > 256
        {
            return Err("native_siwc_account_invalid".into());
        }
        if let Some(existing) = self
            .flows
            .values()
            .find(|flow| flow.start_request_id == request_id)
        {
            if existing.account != request.account_id
                || existing.revision != request.expected_account_revision
                || existing.label != request.display_name
            {
                return Err("native_siwc_request_reused".into());
            }
            return self.project(existing);
        }
        if self.flows.len() >= 16
            || self.flows.values().any(|flow| {
                flow.state
                    .lock()
                    .map(|s| {
                        s.result.is_none()
                            || (!s.cancelled
                                && s.committed.is_none()
                                && matches!(s.result, Some(Ok(_))))
                    })
                    .unwrap_or(true)
            })
        {
            return Err("native_siwc_flow_busy".into());
        }
        self.current(&request.account_id, request.expected_account_revision)?;
        let revision = self
            .host
            .snapshot(&request.account_id)?
            .map(|s| s.session_revision)
            .unwrap_or(0);
        let flow = Arc::new(Flow {
            id: uuid::Uuid::new_v4().to_string(),
            start_request_id: request_id.into(),
            account: request.account_id,
            revision: request.expected_account_revision,
            label: request.display_name,
            state: Mutex::new(FlowState::default()),
        });
        self.flows.insert(flow.id.clone(), flow.clone());
        let pending = flow.clone();
        let host = self.host.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let attempt = host.begin_sign_in(&pending.account, revision)?;
                {
                    let mut state = pending
                        .state
                        .lock()
                        .map_err(|_| "native_siwc_state_unavailable")?;
                    if state.cancelled {
                        attempt.cancel();
                        return Err("siwc_cancelled".into());
                    }
                    state.authorization_url = Some(attempt.authorization_url().into());
                    state.cancellation = Some(attempt.cancellation());
                }
                host.finish_sign_in(attempt)
            })();
            if let Ok(mut state) = pending.state.lock() {
                // Never expose provider body/errors or callback URL diagnostics.
                state.result = Some(result.map_err(|error| match error.as_str() {
                    "siwc_authorization_denied" => "native_siwc_authorization_declined",
                    "siwc_callback_expired" => "native_siwc_authorization_expired",
                    "siwc_network_unavailable" => "native_siwc_authorization_network",
                    _ => "native_siwc_authorization_failed",
                }.into()));
                state.authorization_url = None;
                state.cancellation = None;
            }
        });
        self.project(&flow)
    }
    pub(crate) fn status(&self, identity: &FlowIdentity) -> Result<Value, String> {
        let flow = self.selected(identity)?;
        self.project(&flow)
    }
    pub(crate) fn cancel(&self, identity: &FlowIdentity) -> Result<Value, String> {
        let flow = self.selected(identity)?;
        {
            let mut state = flow
                .state
                .lock()
                .map_err(|_| "native_siwc_state_unavailable")?;
            if state.committed.is_none() {
                state.cancelled = true;
                state.authorization_url = None;
                if let Some(cancellation) = &state.cancellation {
                    cancellation.cancel();
                }
            }
        }
        self.project(&flow)
    }
    pub(crate) fn finish(&self, identity: &FlowIdentity) -> Result<Value, String> {
        let flow = self.selected(identity)?;
        {
            let mut state = flow
                .state
                .lock()
                .map_err(|_| "native_siwc_state_unavailable")?;
            if state.cancelled {
                return Err("native_siwc_cancelled".into());
            }
            if state.committed.is_none() {
                self.current(&flow.account, flow.revision)?;
                let session = state
                    .result
                    .as_ref()
                    .ok_or("native_siwc_authorization_pending")?
                    .as_ref()
                    .map_err(|_| "native_siwc_authorization_failed")?;
                let account = self.host.with_session_guard(session, || {
                    lifecycle::save_account(
                        &self.store,
                        &flow.account,
                        &flow.label,
                        flow.revision,
                        session,
                    )
                    .map_err(|_| "native_siwc_core_commit_conflict_private_session_retained".into())
                })?;
                state.committed = Some(
                    serde_json::to_value(account).map_err(|_| "native_siwc_response_invalid")?,
                );
            }
        }
        self.project(&flow)
    }

    pub(crate) fn manage(&self, operation: &str, request: AccountRequest) -> Result<Value, String> {
        self.owner(&request.owner_id)?;
        let mut account = lifecycle::load(&self.store, &request.account_id)?
            .ok_or("native_siwc_account_changed")?;
        lifecycle::owned(&account, Some(request.expected_account_revision))?;
        let mut session = self.host.snapshot(&request.account_id)?;
        let mut connection = None;
        if operation != "account_status" {
            if self.flows.values().any(|flow| {
                flow.state
                    .lock()
                    .map(|state| {
                        state.result.is_none()
                            || (!state.cancelled
                                && state.committed.is_none()
                                && matches!(state.result, Some(Ok(_))))
                    })
                    .unwrap_or(true)
            }) {
                return Err("native_siwc_flow_busy".into());
            }
            let expected = request
                .expected_session_revision
                .ok_or("native_siwc_private_session_changed")?;
            let baseline = session
                .as_ref()
                .ok_or("native_siwc_private_session_changed")?;
            if expected != baseline.session_revision {
                return Err("native_siwc_private_session_changed".into());
            }
            if operation == "models" {
                let models = self.host.models(&request.account_id, expected)?;
                // Catalog access may renew this same registration before its GET.
                let current_session = self.host.snapshot(&request.account_id)?
                    .ok_or("native_siwc_private_session_changed")?;
                if current_session.client_id != baseline.client_id || current_session.subject != baseline.subject {
                    return Err("native_siwc_private_session_changed".into());
                }
                connection = Some(self.host.with_session_guard(&current_session, || {
                    self.current(&request.account_id, Some(request.expected_account_revision))?;
                    lifecycle::sync_catalog(&self.store, &account, models)
                })?);
                session = Some(current_session);
            } else {
                let changed = match operation {
                    "refresh" => self.host.refresh(&request.account_id, expected)?,
                    "sign_out" if request.acknowledge_sign_out => {
                        self.host.sign_out(&request.account_id, expected)?
                    }
                    "sign_out" => return Err("native_siwc_confirmation_required".into()),
                    _ => return Err("native_siwc_request_invalid".into()),
                };
                account = self.host.with_session_guard(&changed, || {
                    lifecycle::save_account(
                        &self.store,
                        &request.account_id,
                        &account.account.display_name,
                        Some(request.expected_account_revision),
                        &changed,
                    )
                    .map_err(|_| "native_siwc_core_commit_conflict_private_session_retained".into())
                })?;
                session = Some(changed);
            }
        }
        Ok(
            json!({"owner_id":self.owner,"account_id":request.account_id,"account":account,
            "session":session,"connection":connection,"inference_performed":false}),
        )
    }
}
impl Drop for NativeSiwcService {
    fn drop(&mut self) {
        for flow in self.flows.values() {
            if let Ok(mut state) = flow.state.lock() {
                state.cancelled = true;
                if let Some(cancellation) = &state.cancellation {
                    cancellation.cancel();
                }
            }
        }
    }
}

fn serve(
    mut service: NativeSiwcService,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    loop {
        let mut bytes = Vec::new();
        input
            .take(MAX_FRAME + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| "native_siwc_pipe_failed")?;
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() as u64 > MAX_FRAME || !bytes.ends_with(b"\n") {
            return Err("native_siwc_frame_invalid".into());
        }
        let request: Request =
            serde_json::from_slice(&bytes).map_err(|_| "native_siwc_request_invalid")?;
        let request_id = match &request {
            Request::Hello { request_id }
            | Request::Start { request_id, .. }
            | Request::Status { request_id, .. }
            | Request::Cancel { request_id, .. }
            | Request::Finish { request_id, .. }
            | Request::AccountStatus { request_id, .. }
            | Request::Models { request_id, .. }
            | Request::Refresh { request_id, .. }
            | Request::SignOut { request_id, .. }
            | Request::Close { request_id } => request_id.clone(),
        };
        if uuid::Uuid::parse_str(&request_id).is_err() {
            return Err("native_siwc_request_id_invalid".into());
        }
        let result = match request {
            Request::Hello { .. } => {
                Ok(json!({"type":"hello","owner_id":service.owner_id(),"protocol":1}))
            }
            Request::Start { request, .. } => service
                .start(&request_id, request)
                .map(|flow| json!({"type":"flow","flow":flow})),
            Request::Status { request, .. } => service
                .status(&request)
                .map(|flow| json!({"type":"flow","flow":flow})),
            Request::Cancel { request, .. } => service
                .cancel(&request)
                .map(|flow| json!({"type":"flow","flow":flow})),
            Request::Finish { request, .. } => service
                .finish(&request)
                .map(|flow| json!({"type":"flow","flow":flow})),
            Request::AccountStatus { request, .. } => service
                .manage("account_status", request)
                .map(|result| json!({"type":"account","result":result})),
            Request::Models { request, .. } => service
                .manage("models", request)
                .map(|result| json!({"type":"account","result":result})),
            Request::Refresh { request, .. } => service
                .manage("refresh", request)
                .map(|result| json!({"type":"account","result":result})),
            Request::SignOut { request, .. } => service
                .manage("sign_out", request)
                .map(|result| json!({"type":"account","result":result})),
            Request::Close { .. } => return Ok(()),
        };
        let mut response =
            result.unwrap_or_else(|error| json!({"type":"error","error":safe_error(&error)}));
        response["request_id"] = json!(request_id);
        let bytes = serde_json::to_vec(&response).map_err(|_| "native_siwc_response_invalid")?;
        if bytes.len() as u64 >= MAX_FRAME {
            return Err("native_siwc_frame_invalid".into());
        }
        output
            .write_all(&bytes)
            .and_then(|_| output.write_all(b"\n"))
            .and_then(|_| output.flush())
            .map_err(|_| "native_siwc_pipe_failed")?;
    }
}

fn safe_error(error: &str) -> &'static str {
    match error {
        "native_siwc_owner_changed" => "native_siwc_owner_changed",
        "native_siwc_account_changed"
        | "siwc_core_revision_conflict"
        | "siwc_core_owner_mismatch" => "native_siwc_account_changed",
        "native_siwc_flow_identity_mismatch" => "native_siwc_flow_identity_mismatch",
        "native_siwc_flow_unknown" => "native_siwc_flow_unknown",
        "native_siwc_confirmation_required" => "native_siwc_confirmation_required",
        "native_siwc_account_invalid" => "native_siwc_account_invalid",
        "native_siwc_request_reused" => "native_siwc_request_reused",
        "native_siwc_flow_busy" => "native_siwc_flow_busy",
        "native_siwc_authorization_pending" => "native_siwc_authorization_pending",
        "native_siwc_authorization_failed" => "native_siwc_authorization_failed",
        "native_siwc_cancelled" => "native_siwc_cancelled",
        "native_siwc_private_session_changed" => "native_siwc_private_session_changed",
        "siwc_revision_conflict" => "native_siwc_private_session_changed",
        "siwc_refresh_result_unknown" => "native_siwc_refresh_result_unknown",
        "siwc_sign_in_required" | "siwc_reauthorization_required" => {
            "native_siwc_reauthorization_required"
        }
        "siwc_plan_consent_required" => "native_siwc_plan_consent_required",
        "native_siwc_core_commit_conflict_private_session_retained" => {
            "native_siwc_core_commit_conflict_private_session_retained"
        }
        _ => "native_siwc_operation_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_service() -> (tempfile::TempDir, NativeSiwcService) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("owner.sqlite");
        let store = NolaStore::open(&path).unwrap();
        let _: Value = store
            .application()
            .unwrap()
            .execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::ListAccounts,
            })
            .unwrap();
        let host = SiwcHost::open(&path).unwrap();
        (root, NativeSiwcService::new(store, host))
    }
    #[test]
    fn model_siwc_native_pipe_hello_is_private_bounded_and_eof_cancels_pending() {
        let (_root, mut service) = fixture_service();
        let flow = Arc::new(Flow {
            id: uuid::Uuid::new_v4().to_string(),
            start_request_id: uuid::Uuid::new_v4().to_string(),
            account: uuid::Uuid::new_v4().to_string(),
            revision: None,
            label: "Fixture".into(),
            state: Mutex::new(FlowState::default()),
        });
        service.flows.insert(flow.id.clone(), flow.clone());
        let id = uuid::Uuid::new_v4().to_string();
        let mut input =
            std::io::Cursor::new(format!("{}\n", json!({"type":"hello","request_id":id})));
        let mut output = Vec::new();
        serve(service, &mut input, &mut output).unwrap();
        let reply: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(reply["request_id"], id);
        assert_eq!(reply["protocol"], 1);
        assert!(flow.state.lock().unwrap().cancelled);
        assert!(!String::from_utf8(output).unwrap().contains("sqlite"));
        let (_root, service) = fixture_service();
        assert!(serve(
            service,
            &mut std::io::Cursor::new(vec![b'x'; MAX_FRAME as usize + 1]),
            &mut Vec::new()
        )
        .is_err());
        assert_eq!(
            safe_error("token=private-fixture"),
            "native_siwc_operation_failed"
        );
    }
}
pub(crate) fn run_if_requested() -> Option<Result<(), String>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) != Some("model-siwc-bridge") {
        return None;
    }
    Some((|| {
        if arguments.len() != 3
            || arguments[1] != "--database"
            || !std::path::Path::new(&arguments[2]).is_absolute()
        {
            return Err("native_siwc_database_required".into());
        }
        let store = NolaStore::open(std::path::Path::new(&arguments[2]))?;
        let _: Value =
            store
                .application()?
                .execute(nola_application::ApplicationCommand::Model {
                    command: nola_application::ModelCommand::ListAccounts,
                })?;
        let host = SiwcHost::open(store.path())?;
        serve(
            NativeSiwcService::new(store, host),
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
        )
    })())
}
