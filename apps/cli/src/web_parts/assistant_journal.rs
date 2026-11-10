// SPDX-License-Identifier: Apache-2.0
// Browser and paired-host read transports. Never accept arbitrary Application commands.
#[derive(serde::Deserialize)]
#[serde(tag="operation",rename_all="snake_case",deny_unknown_fields)]
enum NativeJournalRequest {
    TaskChildWindow { parent_conversation_id: String, parent_run_id: String, task_id: String,
        child_conversation_id: String, before_ordinal: Option<i64>, limit: usize, content_byte_limit: usize },
    ContextUsage { identity: nola_domain::assistant_journal::AssistantJournalIdentity },
    AdmissionHead { conversation_id: String, run_id: String },
    Admissions {
        after: Option<nola_domain::run_message_admission::RunMessageAdmissionCursor>,
        through: nola_domain::run_message_admission::RunMessageAdmissionCursor,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    TerminalSeal { conversation_id: String, run_id: String },
    TerminalManifest {
        seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
        after_entry: nola_domain::assistant_journal_operations::JournalNumber,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    Heads { conversation_id: String, run_id: String, after_message_id: Option<String>, limit: usize },
    Head {identity:nola_domain::assistant_journal::AssistantJournalIdentity},
    Read {after:nola_domain::assistant_journal::AssistantJournalCursor,
        through:nola_domain::assistant_journal::AssistantJournalCursor,max_records:usize,max_encoded_bytes:usize},
    OpenSnapshot {at:nola_domain::assistant_journal::AssistantJournalCursor},
    ReleaseSnapshot {snapshot:nola_application::AssistantJournalSnapshotHandle},
    Parts {snapshot:nola_application::AssistantJournalSnapshotHandle,after_part_id:Option<String>,max_records:usize,max_encoded_bytes:usize},
    Changes {snapshot:nola_application::AssistantJournalSnapshotHandle,base:nola_domain::assistant_journal::AssistantJournalCursor,
        after_part_id:Option<String>,max_records:usize,max_encoded_bytes:usize},
    Manifest {snapshot:nola_application::AssistantJournalSnapshotHandle,part_id:String,
        content:nola_domain::assistant_journal_operations::JournalContentRef,
        after_chunk:nola_domain::assistant_journal_operations::JournalNumber,max_records:usize},
    Chunk {snapshot:nola_application::AssistantJournalSnapshotHandle,part_id:String,
        content:nola_domain::assistant_journal_operations::JournalContentRef,
        chunk_index:nola_domain::assistant_journal_operations::JournalNumber},
    RecordChunk {snapshot:nola_application::AssistantJournalSnapshotHandle,part_id:String,
        content:nola_domain::assistant_journal_operations::JournalContentRef,
        chunk_index:nola_domain::assistant_journal_operations::JournalNumber},
    Range {snapshot:nola_application::AssistantJournalSnapshotHandle,part_id:String,
        content:nola_domain::assistant_journal_operations::JournalContentRef,
        offset_bytes:nola_domain::assistant_journal_operations::JournalNumber,limit_bytes:u32},
}

impl NativeJournalRequest {
    fn identity(&self)->Option<&nola_domain::assistant_journal::AssistantJournalIdentity> {
        Some(match self {
            Self::Heads {..}|Self::TerminalSeal {..}|Self::AdmissionHead {..}|Self::TaskChildWindow {..}=>return None,
            Self::Admissions {through,..}=>&through.identity,
            Self::TerminalManifest {seal,..}=>&seal.identity,
            Self::Head {identity}|Self::ContextUsage {identity}=>identity,
            Self::Read {after,..}=>&after.identity,
            Self::OpenSnapshot {at}=>&at.identity,
            Self::ReleaseSnapshot {snapshot}|Self::Parts {snapshot,..}|Self::Changes {snapshot,..}|Self::Manifest {snapshot,..}
                |Self::Chunk {snapshot,..}|Self::RecordChunk {snapshot,..}|Self::Range {snapshot,..}=>&snapshot.at.identity,
        })
    }
    fn into_command(self)->Result<nola_application::ApplicationCommand,String> {
        use nola_application::ApplicationCommand as C;
        Ok(match self {
            Self::TaskChildWindow {parent_conversation_id,parent_run_id,task_id,child_conversation_id,before_ordinal,limit,content_byte_limit} => C::LoadTaskChildConversationWindow {parent_conversation_id,parent_run_id,task_id,child_conversation_id,before_ordinal,limit,content_byte_limit},
            Self::AdmissionHead {conversation_id,run_id}=>C::LocalRunMessageAdmissionHead {conversation_id,run_id},
            Self::Admissions {after,through,max_records,max_encoded_bytes}=>C::ReadRunMessageAdmissions {
                after: match after {Some(after)=>after,None=>nola_domain::run_message_admission::RunMessageAdmissionCursor::initial(through.identity.clone())?},
                through,max_records,max_encoded_bytes,
            },
            Self::TerminalSeal {conversation_id,run_id}=>C::LocalAssistantRunTerminalSeal {conversation_id,run_id},
            Self::TerminalManifest {seal,after_entry,max_records,max_encoded_bytes}=>C::ReadLocalAssistantRunTerminalManifest {seal,after_entry,max_records,max_encoded_bytes},
            Self::Heads {conversation_id,run_id,after_message_id,limit}=>C::LocalRunAssistantJournalHeads {conversation_id,run_id,after_message_id,limit},
            Self::Head {identity}=>C::LocalAssistantJournalHead {identity},
            Self::ContextUsage {identity}=>C::LocalRunContextUsage {identity},
            Self::Read {after,through,max_records,max_encoded_bytes}=>C::ReadAssistantJournal {after,through,max_records,max_encoded_bytes},
            Self::OpenSnapshot {at}=>C::OpenAssistantJournalSnapshot {at},
            Self::ReleaseSnapshot {snapshot}=>C::ReleaseAssistantJournalSnapshot {snapshot},
            Self::Parts {snapshot,after_part_id,max_records,max_encoded_bytes}=>C::ReadAssistantJournalParts {snapshot,after_part_id,max_records,max_encoded_bytes},
            Self::Changes {snapshot,base,after_part_id,max_records,max_encoded_bytes}=>C::ReadAssistantJournalPartChanges {snapshot,base,after_part_id,max_records,max_encoded_bytes},
            Self::Manifest {snapshot,part_id,content,after_chunk,max_records}=>C::ReadAssistantJournalManifest {snapshot,part_id,content,after_chunk,max_records},
            Self::Chunk {snapshot,part_id,content,chunk_index}=>C::ReadAssistantJournalChunk {snapshot,part_id,content,chunk_index},
            Self::RecordChunk {snapshot,part_id,content,chunk_index}=>C::ReadAssistantJournalRecordChunk {snapshot,part_id,content,chunk_index},
            Self::Range {snapshot,part_id,content,offset_bytes,limit_bytes}=>C::ReadAssistantJournalRange {snapshot,part_id,content,offset_bytes,limit_bytes},
        })
    }
}

fn mobile_assistant_journal_response(request:&mut Request)->Response<Box<dyn Read+Send>> {
    mobile_assistant_journal_response_with(request, || {
        Ok((crate::application_port::CliApplicationPort::persistent_default()?, host_identity().to_owned()))
    })
}

// The browser route remains behind handle_request's ordinary Web trust gate:
// loopback is local; remote peers need a session and same-origin CSRF proof.
// It is deliberately absent from the public-remote and CSRF-exempt routes.
fn browser_assistant_journal_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    browser_assistant_journal_response_with(request, || {
        Ok((crate::application_port::CliApplicationPort::persistent_default()?, host_identity().to_owned()))
    })
}

fn browser_assistant_journal_response_with(
    request: &mut Request,
    open: impl FnOnce() -> Result<(crate::application_port::CliApplicationPort, String), String>,
) -> Response<Box<dyn Read + Send>> {
    // Enforce the journal-specific bound while reading, including chunked HTTP
    // bodies; the general Web body limit is intentionally much larger.
    const MAX_BYTES: usize = 64 * 1024;
    if request.body_length().is_some_and(|length| length > MAX_BYTES) {
        return json_response(StatusCode(413), json!({"error":"journal request exceeds budget"}));
    }
    let mut bytes = Vec::new();
    if request.as_reader().take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).is_err() {
        return json_response(StatusCode(400), json!({"error":"cannot read journal request"}));
    }
    if bytes.len() > MAX_BYTES {
        return json_response(StatusCode(413), json!({"error":"journal request exceeds budget"}));
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(_) => return json_response(StatusCode(400), json!({"error":"invalid journal JSON"})),
    };
    let input: NativeJournalRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(error) => return json_response(StatusCode(422), json!({"error":error.to_string()})),
    };
    match dispatch_local_assistant_journal(input, open) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

// Both transports use the same exact source/Run fence. Snapshot allocation and
// release only manage read handles; no conversation or execution mutation is
// accepted. The Host, never request JSON, selects the database and source ID.
fn dispatch_local_assistant_journal(
    input: NativeJournalRequest,
    open: impl FnOnce() -> Result<(crate::application_port::CliApplicationPort, String), String>,
) -> Result<Value, String> {
    let (mut application, source_host_id) = open()?;
    // Local readability of an imported journal does not authorize this Host
    // to impersonate its original source.
    if let Some(identity) = input.identity() {
        let _: nola_domain::assistant_journal::AssistantJournalCursor = application.execute(
            nola_application::ApplicationCommand::LocalAssistantJournalHead { identity: identity.clone() },
        )?;
    }
    // Heads and TerminalSeal perform their own exact local Run gate. A terminal
    // manifest is also checked against the immutable source seal by Application;
    // passing the journal identity gate alone never authenticates caller fields.
    let result: Value = application.execute(input.into_command()?)?;
    Ok(json!({"source_host_id":source_host_id,"result":result}))
}

fn local_browser_journal_heads(
    application: &mut crate::application_port::CliApplicationPort,
    heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
) -> Vec<nola_domain::assistant_journal::AssistantJournalCursor> {
    heads.into_iter().filter(|head| {
        // History may contain imported remote journals. Publish a local-source
        // watermark only after exact author proof; any unverified owner is
        // omitted, never relabelled. Preserve the window's original cursor
        // rather than substituting a later head from this separate read.
        application.execute::<nola_domain::assistant_journal::AssistantJournalCursor>(
            nola_application::ApplicationCommand::LocalAssistantJournalHead {
                identity: head.identity.clone(),
            },
        ).is_ok()
    }).collect()
}

// The adapter supplies the database and authenticated source identity. Neither
// can be selected by request JSON. Tests use isolated stores through this same
// authentication/decryption/dispatch path, without changing process globals.
fn mobile_assistant_journal_response_with(
    request: &mut Request,
    open: impl FnOnce() -> Result<(crate::application_port::CliApplicationPort, String), String>,
) -> Response<Box<dyn Read+Send>> {
    let Some(token)=mobile_bearer_token(request) else {
        return json_response(StatusCode(401),json!({"error":"unauthorized"}));
    };
    let body=match read_mobile_json_value(request,&token) {
        Ok(body)=>body,Err(response)=>return response,
    };
    if serde_json::to_vec(&body).map_or(true,|bytes|bytes.len()>64*1024) {
        return mobile_encrypted_json_response(StatusCode(413),json!({"error":"journal request exceeds budget"}),&token);
    }
    let input:NativeJournalRequest=match serde_json::from_value(body) {
        Ok(input)=>input,Err(error)=>return mobile_encrypted_json_response(StatusCode(422),json!({"error":error.to_string()}),&token),
    };
    let result = dispatch_local_assistant_journal(input, open);
    match result {
        Ok(value)=>mobile_encrypted_json_response(StatusCode(200),value,&token),
        Err(error)=>mobile_encrypted_json_response(StatusCode(409),json!({"error":error}),&token),
    }
}

#[cfg(test)]
mod native_journal_contract_tests {
    use super::*;

    #[test]
    fn task_child_window_uses_exact_core_scope_and_rejects_uri_authority() {
        let query = json!({"operation":"task_child_window", "parent_conversation_id":"parent",
            "parent_run_id":"run", "task_id":"task", "child_conversation_id":"child",
            "before_ordinal":null, "limit":30, "content_byte_limit":65536});
        let input: NativeJournalRequest = serde_json::from_value(query.clone()).unwrap();
        assert!(matches!(input.into_command().unwrap(), nola_application::ApplicationCommand::LoadTaskChildConversationWindow {
            parent_conversation_id, parent_run_id, task_id, child_conversation_id, before_ordinal: None,
            limit: 30, content_byte_limit: 65536,
        } if parent_conversation_id == "parent" && parent_run_id == "run" && task_id == "task" && child_conversation_id == "child"));
        let mut forged = query;
        forged["result_ref"] = json!("nola-journal://foreign/run");
        assert!(serde_json::from_value::<NativeJournalRequest>(forged).is_err());
    }

    fn browser_exchange(database: &Path, body: String) -> (u16, Value) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let database = database.to_path_buf();
        let worker = std::thread::spawn(move || {
            let mut request = server.recv_timeout(std::time::Duration::from_secs(10)).unwrap().expect("browser HTTP request");
            assert_eq!(request.url(), "/api/chat/agent/v2/journal");
            let response = browser_assistant_journal_response_with(&mut request, || {
                Ok((crate::application_port::CliApplicationPort::persistent(&database)?, "fixture-source".into()))
            });
            request.respond(response).unwrap();
        });
        let response = reqwest::blocking::Client::builder().no_proxy()
            .timeout(std::time::Duration::from_secs(15)).build().unwrap()
            .post(format!("http://{address}/api/chat/agent/v2/journal"))
            .header("Content-Type", "application/json").body(body).send().unwrap();
        let status = response.status().as_u16();
        let value = response.json().unwrap();
        worker.join().unwrap();
        (status, value)
    }

    #[test]
    fn browser_journal_requires_normal_web_gate_and_rejects_non_read_commands_before_open() {
        let path = "/api/chat/agent/v2/journal";
        assert!(!remote_web_host_route_allowed(&Method::Post, path));
        assert!(web_method_requires_csrf(&Method::Post));
        assert!(!web_csrf_exempt_path(path));
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("must-not-open.sqlite");
        for operation in ["receive_assistant_journal_record", "stage_assistant_journal_content", "begin_assistant_turn", "apply_task_guidance"] {
            assert_eq!(browser_exchange(&database, json!({"operation":operation}).to_string()).0, 422);
        }
        assert_eq!(browser_exchange(&database, json!({"operation":"heads", "conversation_id":"c", "run_id":"r", "limit":1,
            "database":"/foreign.sqlite"}).to_string()).0, 422);
        assert_eq!(browser_exchange(&database, "{".into()).0, 400);
        assert_eq!(browser_exchange(&database, " ".repeat(64 * 1024 + 1)).0, 413);
        assert!(!database.exists(), "invalid browser requests must not open a database");
    }

    fn exchange(database: &Path, token: &str, authorized: bool, payload: Value) -> (u16, Value) {
        let observing = matches!(payload.get("operation").and_then(Value::as_str), Some("discover" | "observe"));
        let path = if observing { "/api/mobile/chat/agent/v2/observe" } else { "/api/mobile/chat/agent/v2/journal" };
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let database = database.to_path_buf();
        let worker = std::thread::spawn(move || {
            let mut request = server.recv_timeout(std::time::Duration::from_secs(10)).unwrap().expect("HTTP request");
            assert_eq!(request.url(), path);
            let open = || {
                Ok((crate::application_port::CliApplicationPort::persistent(&database)?, "fixture-source".into()))
            };
            let response = if observing { run_observation_response_with(&mut request, true, open) }
                else { mobile_assistant_journal_response_with(&mut request, open) };
            request.respond(response).unwrap();
        });
        let body = nola_core::mobile_crypto::encrypt_mobile_json(&payload, token).unwrap();
        let client = reqwest::blocking::Client::builder().no_proxy()
            .timeout(std::time::Duration::from_secs(15)).build().unwrap();
        let mut request = client.post(format!("http://{address}{path}")).json(&body);
        if authorized { request = request.bearer_auth(token); }
        let response = request.send().unwrap();
        let status = response.status().as_u16();
        let value: Value = response.json().unwrap();
        worker.join().unwrap();
        let value = if value.get("type").and_then(Value::as_str) == Some("encrypted") {
            let envelope = serde_json::from_value(value).unwrap();
            nola_core::mobile_crypto::decrypt_mobile_json(&envelope, token).unwrap()
        } else { value };
        (status, value)
    }

    #[test]
    fn encrypted_http_reads_exact_committed_journal_and_rejects_forged_identity() {
        use nola_application::BeginAssistantTurn;
        use nola_core::agent::{AgentCoreConfig, AgentSubmission, AgentItem, ItemId, SessionId};
        use nola_core::agent::host::*;
        use nola_core::agent::protocol::AgentInputDelivery;
        use nola_domain::assistant_journal::AssistantJournalCursor;
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("source.sqlite");
        let mut application = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let capability = HostCapabilityDescriptor {
            id: "model.stream".into(), title: "fixture".into(), route: HostCapabilityRoute::Model,
            availability: CapabilityAvailability::Available, exposure: HostCapabilityExposure::Internal,
            permission_domain: None, input_schema: json!({"type":"object"}), input_normalization: Default::default(),
            output_policy: HostOutputPolicy {preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true},
            supports_streaming: true, supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial, cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        };
        let registered:nola_domain::remote_submission::RegisteredSourceSubmission=application.execute(
            nola_application::ApplicationCommand::RegisterSourceSubmission {
                identity:nola_domain::remote_submission::RemoteSubmissionIdentity {
                    source_host_id:"fixture-source".into(),client_id:"fixture-client".into(),conversation_id:"http-conversation".into(),
                    submission_id:"http-run".into(),user_message_id:"u".into(),assistant_message_id:"assistant".into(),
                },request:json!({"input_items":[{"text":"Work"}]}),
            }).unwrap();
        assert!(registered.created);
        let accepted = application.begin_assistant_turn(BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: "http-conversation".into(), conversation_title: "Journal".into(),
            user_message_id: "u".into(), user_part_id: "u-body".into(), user_text: "Work".into(),
            user_content_json: None, assistant_message_id: "assistant".into(), context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: crate::test_model_identity::identity(),
                session_id: Some(SessionId::from_string("http-conversation".into())), model: Some("fixture".into()),
                host_catalog: Some(HostCapabilityCatalog {host_id: HostId("fixture-source".into()), revision: 1,
                    availability_epoch: 1, lease: None, capabilities: [("model.stream".into(),capability)].into_iter().collect()}),
                ..Default::default()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("u".into()), "Work")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("http-run".into()), submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let initial = accepted.assistant_publications.last().unwrap().clone();
        assert_eq!(initial.sequence, 0, "admission is a read origin, not published content");
        let empty_window = crate::store::NolaStore::open(&database).unwrap()
            .load_web_message_window("http-conversation", None, 24).unwrap();
        assert_eq!(empty_window.assistant_journal_heads, vec![initial.clone()],
            "a real admission exposes its exact genesis owner before content arrives");
        let genesis_snapshot: nola_application::AssistantJournalSnapshotHandle = application.execute(
            nola_application::ApplicationCommand::OpenAssistantJournalSnapshot { at: initial.clone() },
        ).unwrap();
        let genesis_parts: nola_application::AssistantJournalPartPage = application.execute(
            nola_application::ApplicationCommand::ReadAssistantJournalParts {
                snapshot: genesis_snapshot.clone(), after_part_id: None,
                max_records: 24, max_encoded_bytes: 16_384,
            },
        ).unwrap();
        assert!(genesis_parts.parts.is_empty(), "genesis must not fabricate visible content");
        let _: Value = application.execute(
            nola_application::ApplicationCommand::ReleaseAssistantJournalSnapshot { snapshot: genesis_snapshot },
        ).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let request = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: "http-run".into(), worker_generation: "journal-fixture".into(),
            lease_owner: "journal-fixture".into(), claimed_at_ms: now,
            lease_expires_at_ms: now + 60_000, limit: 32,
        }).unwrap().requests.into_iter().find(|request|
            matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
        let part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        };
        for (index, payload) in [
            HostResponse::ExecutionStarted,
            HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture".into(), reason: "initial".into() },
            HostResponse::ModelDelta { channel: ModelDeltaChannel::Text, part_id,
                delta: "Committed HTTP journal content".into() },
        ].into_iter().enumerate() {
            application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: "http-conversation".into(), run_id: "http-run".into(),
                invocation_config_digest: accepted.invocation_config_digest.clone(),
                received_at_ms: now + index as i64,
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(), attempt_id: request.attempt_id.clone(),
                    delegated_invocation_id: request.delegated_invocation_id.clone(),
                    effect_id: request.effect_id.clone(), host_id: request.host_id.clone(),
                    lease_id: request.lease_id.clone(), session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(), catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch, delivery: request.delivery.clone(),
                    sequence: index as u64 + 1, payload,
                },
            }).unwrap();
        }
        let through: AssistantJournalCursor = application.execute(
            nola_application::ApplicationCommand::LocalAssistantJournalHead { identity: initial.identity },
        ).unwrap();
        assert!(through.sequence > 0, "transport must test real committed records");
        let receipt:Option<nola_domain::remote_submission::SourceSubmissionReceipt>=application.execute(
            nola_application::ApplicationCommand::ReadSourceSubmissionReceipt {
                identity:registered.receipt.identity,request_digest:registered.receipt.request_digest,
            }).unwrap();
        let receipt=receipt.unwrap();
        assert_eq!(receipt.phase,nola_domain::remote_submission::RemoteSubmissionPhase::Admitted);
        let observation = read_run_observation(&mut application,"fixture-source", RunObservationRequest::Discover {
            conversation_id:"http-conversation".into(),expected_host_id:"fixture-source".into(),client_id:"fixture-client".into(),
        }).unwrap();
        assert_eq!(observation["result"]["run_id"],"http-run");
        assert_eq!(observation["result"]["assistant_message_id"],"assistant");
        assert_eq!(observation["result"]["worker"]["state"],"absent");
        assert_eq!(observation["result"]["recovery"],"resume_required");
        assert_eq!(observation["result"]["can_submit"],false);
        assert_eq!(observation["result"]["permissions"],json!([]));
        assert!(observation["result"].get("checkpoint").is_none());
        let starting = try_register_web_agent_run("http-conversation",RuntimeExecutionControl::default()).unwrap();
        let observed_start = read_run_observation(&mut application,"fixture-source",RunObservationRequest::Discover {
            conversation_id:"http-conversation".into(),expected_host_id:"fixture-source".into(),client_id:"fixture-client".into(),
        }).unwrap();
        assert_eq!(observed_start["result"]["worker"]["state"],"starting",
            "A conversation-only registration must never prove a running owned Run");
        assert_eq!(observed_start["result"]["permissions"],json!([]));
        assert_eq!(observed_start["result"]["can_submit"],false);
        drop(starting);
        assert!(read_run_observation(&mut application,"fixture-source",RunObservationRequest::Observe {
            conversation_id:"http-conversation".into(),expected_host_id:"fixture-source".into(),client_id:"fixture-client".into(),
            expected_run_id:"http-run".into(),assistant_message_id:"foreign-root".into(),
        }).is_err());
        drop(application); // Every HTTP read reopens the actual committed database.
        let token = format!("journal-test-{}", uuid::Uuid::new_v4());
        mobile_tokens().lock().unwrap().insert(token.clone(), Instant::now());
        struct TokenGuard(String);
        impl Drop for TokenGuard {
            fn drop(&mut self) { mobile_tokens().lock().unwrap().remove(&self.0); }
        }
        let _token_guard = TokenGuard(token.clone());
        let observe = json!({"operation":"discover","conversation_id":"http-conversation",
            "expected_host_id":"fixture-source","client_id":"fixture-client"});
        assert_eq!(exchange(&database,&token,false,observe.clone()).0,401);
        let (status, observed) = exchange(&database,&token,true,observe.clone());
        assert_eq!(status,200,"{observed}");
        assert_eq!(observed["result"]["run_id"],"http-run");
        assert_eq!(observed["result"]["client_id"],"fixture-client");
        assert_eq!(observed["result"]["can_submit"],false);
        assert_eq!(observed["result"]["permissions"],json!([]));
        assert!(observed["result"].get("checkpoint").is_none());
        let mut foreign = observe;
        foreign["expected_host_id"] = json!("foreign-host");
        assert_eq!(exchange(&database,&token,true,foreign).0,409);
        let head = json!({"operation":"head","identity":through.identity});
        assert_eq!(exchange(&database,&token,false,head.clone()).0,401);
        let (status, result) = exchange(&database,&token,true,head);
        assert_eq!(status,200,"{result}");
        assert_eq!(result["source_host_id"],"fixture-source");
        assert_eq!(result["result"],json!(through));
        let (browser_status, browser_head) = browser_exchange(&database, json!({"operation":"head","identity":through.identity}).to_string());
        assert_eq!(browser_status, 200, "{browser_head}");
        assert_eq!(browser_head, result, "browser and mobile observe the same authenticated source facts");
        let admission_query = json!({"operation":"admission_head",
            "conversation_id":"http-conversation","run_id":"http-run"});
        assert_eq!(exchange(&database,&token,false,admission_query.clone()).0,401);
        let (admission_status, admission_head) = exchange(&database,&token,true,admission_query.clone());
        assert_eq!(admission_status,200,"{admission_head}");
        assert_eq!(browser_exchange(&database,admission_query.to_string()),(200,admission_head.clone()));
        let admission_read = json!({"operation":"admissions","after":null,
            "through":admission_head["result"],"max_records":64,"max_encoded_bytes":262144});
        let (admission_status, first_admissions) = browser_exchange(&database,admission_read.to_string());
        assert_eq!(admission_status,200,"{first_admissions}");
        assert_eq!(first_admissions["result"]["through"],admission_head["result"]);
        assert!(!first_admissions["result"]["records"].as_array().unwrap().is_empty());
        assert_eq!(browser_exchange(&database,json!({"operation":"admission_head",
            "conversation_id":"foreign","run_id":"http-run"}).to_string()).0,409);
        let seal_query = json!({"operation":"terminal_seal",
            "conversation_id":"http-conversation","run_id":"http-run"});
        assert_eq!(exchange(&database,&token,false,seal_query.clone()).0,401);
        let (status, unsealed) = exchange(&database,&token,true,seal_query.clone());
        assert_eq!(status,200,"{unsealed}");
        assert_eq!(unsealed,json!({"source_host_id":"fixture-source","result":null}),
            "a committed assistant watermark is not a terminal seal");
        assert_eq!(browser_exchange(&database,seal_query.to_string()),(200,unsealed));
        assert_eq!(browser_exchange(&database,json!({"operation":"terminal_seal",
            "conversation_id":"foreign","run_id":"http-run"}).to_string()).0,409);
        let source_store = crate::store::NolaStore::open(&database).unwrap();
        let history_window = source_store.load_web_message_window("http-conversation", None, 24).unwrap();
        assert_eq!(history_window.assistant_journal_heads, vec![through.clone()]);
        assert_eq!(local_browser_journal_heads(&mut source_store.application().unwrap(), history_window.assistant_journal_heads), vec![through.clone()]);
        let (status, discovered) = exchange(&database,&token,true,json!({"operation":"heads",
            "conversation_id":"http-conversation","run_id":"http-run","limit":64}));
        assert_eq!(status,200,"{discovered}");
        assert_eq!(discovered["result"],json!([through.clone()]));
        assert_eq!(exchange(&database,&token,true,json!({"operation":"heads",
            "conversation_id":"foreign","run_id":"http-run","limit":64})).0,409);
        let after = AssistantJournalCursor::initial(through.identity.clone()).unwrap();
        let (browser_status, browser_page) = browser_exchange(&database, json!({"operation":"read","after":after,
            "through":through,"max_records":128,"max_encoded_bytes":262144}).to_string());
        assert_eq!(browser_status, 200, "{browser_page}");
        let (status, page) = exchange(&database,&token,true,json!({"operation":"read","after":after,
            "through":through,"max_records":128,"max_encoded_bytes":262144}));
        assert_eq!(status,200,"{page}");
        assert_eq!(browser_page, page);
        let page: nola_application::AssistantJournalReadResult = serde_json::from_value(page["result"].clone()).unwrap();
        let nola_application::AssistantJournalReadResult::Records {records,next,complete,..} = page else {panic!()};
        assert!(complete); assert_eq!(next,through); assert!(!records.is_empty());
        let target_database = directory.path().join("receiver.sqlite");
        let mut target = crate::application_port::CliApplicationPort::persistent(&target_database).unwrap();
        let _: Value = target.execute(nola_application::ApplicationCommand::AdoptObservedRemoteRun {
            receipt,
        }).unwrap();
        let first_page:nola_domain::run_message_admission::RunMessageAdmissionPage=serde_json::from_value(first_admissions["result"].clone()).unwrap();
        let _:nola_domain::run_message_admission::RunMessageAdmissionCursor=target.execute(
            nola_application::ApplicationCommand::ReceiveRunMessageAdmissions {
                source_host_id:"fixture-source".into(),page:first_page,
            }).unwrap();
        let (status, opened) = exchange(&database, &token, true,
            json!({"operation":"open_snapshot","at":through}));
        assert_eq!(status, 200, "{opened}");
        let transfer_snapshot = opened["result"].clone();
        let mut transferred_content = Vec::new();
        for record in &records {
            record.batch.verify_header(&record.header).unwrap();
            // A record header is not its content. Download through the same
            // authenticated transport and persist preparation before applying it.
            let mut preparations = 0;
            loop {
                let need: Option<nola_store_port::assistant_journal::AssistantJournalContentNeed> =
                    target.execute(nola_application::ApplicationCommand::NextAssistantJournalContent {
                        source_host_id: "fixture-source".into(), record: record.clone(),
                    }).unwrap();
                let Some(need) = need else { break };
                preparations += 1;
                assert!(preparations <= 128, "content preparation made no bounded progress");
                let (status, chunk) = exchange(&database, &token, true, json!({
                    "operation":"record_chunk", "snapshot":transfer_snapshot,
                    "part_id":need.part_id, "content":need.content,
                    "chunk_index":need.expected_prefix.chunk_count,
                }));
                assert_eq!(status, 200, "{chunk}");
                let chunk: nola_application::AssistantJournalChunk =
                    serde_json::from_value(chunk["result"].clone()).unwrap();
                let prepared: nola_domain::assistant_journal_operations::JournalContentRef =
                    target.execute(nola_application::ApplicationCommand::StageAssistantJournalContent {
                        input: nola_application::StageAssistantJournalContent {
                            source_host_id: "fixture-source".into(), record: record.clone(),
                            part_id: need.part_id.clone(), content: need.content.clone(),
                            expected_prefix: need.expected_prefix.clone(), chunks: vec![chunk], restart: false,
                        },
                    }).unwrap();
                assert!(prepared.chunk_count.0 > need.expected_prefix.chunk_count.0);
                if prepared == need.content {
                    transferred_content.push((need.part_id, need.content));
                }
                // Restart between preparation and publication: no in-memory
                // download state may be required to finish the transfer.
                drop(target);
                target = crate::application_port::CliApplicationPort::persistent(&target_database).unwrap();
            }
            let received: nola_application::AssistantJournalReceiveResult = target.execute(
                nola_application::ApplicationCommand::ReceiveAssistantJournalRecord {
                    source_host_id: "fixture-source".into(), record: record.clone(), chunks: vec![],
                }).unwrap();
            assert!(received.applied);
        }
        drop(target);
        let mut target = crate::application_port::CliApplicationPort::persistent(&target_database).unwrap();
        let replay: nola_application::AssistantJournalReceiveResult = target.execute(
            nola_application::ApplicationCommand::ReceiveAssistantJournalRecord {
                source_host_id: "fixture-source".into(), record: records.last().unwrap().clone(), chunks: vec![],
            }).unwrap();
        assert!(!replay.applied); assert_eq!(replay.cursor,through);
        assert!(!transferred_content.is_empty());
        let snapshot: nola_application::AssistantJournalSnapshotHandle = target.execute(
            nola_application::ApplicationCommand::OpenAssistantJournalSnapshot {at: through.clone()},
        ).unwrap();
        let mut text = String::new();
        for (part_id, content) in transferred_content {
            let range: nola_application::AssistantJournalTextRange = target.execute(
                nola_application::ApplicationCommand::ReadAssistantJournalRange {
                    snapshot: snapshot.clone(), part_id, content,
                    offset_bytes: nola_domain::assistant_journal_operations::JournalNumber(0),
                    limit_bytes: 32768,
                },
            ).unwrap();
            text.push_str(&range.text);
        }
        assert!(text.contains("Committed HTTP journal content"));
        let _: Value = target.execute(
            nola_application::ApplicationCommand::ReleaseAssistantJournalSnapshot {snapshot},
        ).unwrap();
        assert_eq!(exchange(&database, &token, true, json!({
            "operation":"release_snapshot", "snapshot":transfer_snapshot,
        })).0, 200);
        assert!(local_browser_journal_heads(&mut target, vec![through.clone()]).is_empty(),
            "history must not announce an imported watermark as a local source");
        // Local readability does not authorize this adapter to relabel an
        // imported journal as its own source, even with valid pairing auth.
        assert_eq!(exchange(&target_database,&token,true,json!({"operation":"head","identity":through.identity})).0,409);
        assert_eq!(browser_exchange(&target_database,json!({"operation":"head","identity":through.identity}).to_string()).0,409);
        let (browser_status, browser_opened) = browser_exchange(&database, json!({"operation":"open_snapshot","at":through}).to_string());
        assert_eq!(browser_status, 200, "{browser_opened}");
        let browser_snapshot = browser_opened["result"].clone();
        let (browser_status, browser_changes) = browser_exchange(&database, json!({"operation":"changes","snapshot":browser_snapshot,
            "base":after,"after_part_id":null,"max_records":128,"max_encoded_bytes":262144}).to_string());
        assert_eq!(browser_status, 200, "{browser_changes}");
        assert!(!browser_changes["result"]["changes"].as_array().unwrap().is_empty());
        let (browser_status, unchanged) = browser_exchange(&database, json!({"operation":"changes","snapshot":browser_snapshot,
            "base":through,"after_part_id":null,"max_records":128,"max_encoded_bytes":262144}).to_string());
        assert_eq!(browser_status, 200, "{unchanged}");
        assert_eq!(unchanged["result"]["changes"], json!([]));
        assert_eq!(browser_exchange(&database, json!({"operation":"release_snapshot","snapshot":browser_snapshot}).to_string()).0,200);
        let (status, opened) = exchange(&database,&token,true,json!({"operation":"open_snapshot","at":through}));
        assert_eq!(status,200,"{opened}");
        let snapshot = opened["result"].clone();
        let parts = json!({"operation":"parts","snapshot":snapshot,"after_part_id":null,"max_records":128,"max_encoded_bytes":262144});
        let (status, page) = exchange(&database,&token,true,parts.clone());
        assert_eq!(status,200,"{page}");
        assert!(!page["result"]["parts"].as_array().unwrap().is_empty());
        let release = json!({"operation":"release_snapshot","snapshot":snapshot});
        assert_eq!(exchange(&database,&token,true,release.clone()).0,200);
        assert_eq!(exchange(&database,&token,true,release).0,200);
        assert_eq!(exchange(&database,&token,true,parts).0,409);
        let mut forged = through.identity.clone(); forged.conversation_id = "another-conversation".into();
        assert_eq!(exchange(&database,&token,true,json!({"operation":"head","identity":forged})).0,409);
        assert_eq!(exchange(&database,&token,true,json!({"operation":"head","identity":through.identity,"source_host_id":"forged"})).0,422);
    }

    #[test]
    fn journal_transport_cannot_dispatch_mutations_or_accept_claimed_host() {
        for operation in ["receive_run_message_admissions","receive_assistant_journal_record","stage_assistant_journal_content","begin_assistant_turn","apply_task_guidance"] {
            assert!(serde_json::from_value::<NativeJournalRequest>(json!({"operation":operation})).is_err());
        }
        let identity=json!({"source_store_epoch":"epoch","conversation_id":"conversation","run_id":"run","assistant_message_id":"assistant"});
        assert!(serde_json::from_value::<NativeJournalRequest>(json!({"operation":"head","identity":identity,"source_host_id":"forged"})).is_err());
        let command=serde_json::from_value::<NativeJournalRequest>(json!({"operation":"head","identity":identity})).unwrap().into_command().unwrap();
        assert!(command.is_read_only_query());
    }
}
