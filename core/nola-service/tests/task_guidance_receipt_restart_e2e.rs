// SPDX-License-Identifier: Apache-2.0
//! Lost-ACK receipts are immutable outcomes, not current Host adoption views.
#[path = "../../nola-application/tests/support/model_identity.rs"]
mod fixture_model_identity;
mod support;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use nola_application::{ApplicationCommand, BeginAssistantTurn, RuntimeTaskCommand, RuntimeTaskLeaseWitness, RuntimeTaskRunBinding, TaskGuidanceApplyResult};
use nola_core::agent::host::*;
use nola_core::agent::{AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId};
use nola_service_client::ApplicationServiceClient;
use serde_json::{json, Value};
fn begin() -> BeginAssistantTurn {
    let capability = HostCapabilityDescriptor {
        id:"model.stream".into(),title:"fixture model".into(),route:HostCapabilityRoute::Model,
        availability:CapabilityAvailability::Available,exposure:HostCapabilityExposure::Internal,
        permission_domain:None,input_schema:json!({"type":"object"}),input_normalization:Default::default(),
        output_policy:HostOutputPolicy { preview_char_limit:4000,persist_full_output:true,allow_artifacts:true },
        supports_streaming:true,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,
        cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable,
    };
    BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id:"child".into(),conversation_title:"Child task".into(),user_message_id:"user".into(),
        user_part_id:"user-body".into(),user_text:"Execute this task once".into(),user_content_json:None,
        assistant_message_id:"assistant".into(),context_checkpoint_id:None,
        core_config:AgentCoreConfig {
            turn_identity: fixture_model_identity::identity(),
            session_id:Some(SessionId::from_string("actual-child-core".into())), model:Some("fixture-model".into()),
            host_catalog:Some(HostCapabilityCatalog { host_id:HostId("fixture-host".into()),revision:1,availability_epoch:1,
                lease:None,capabilities:[("model.stream".into(),capability)].into_iter().collect() }),
            ..Default::default()
        },
        submission:AgentSubmission::UserInput {
            items:vec![AgentItem::user_message(ItemId::from_string("user".into()),"Execute this task once")],
            turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
            submission_id:Some("task-admission".into()),submitted_at_epoch_ms:Some(1000),delivery:AgentInputDelivery::CurrentTurn,
        },created_at_ms:1000,
    }
}

fn seed(path: &std::path::Path) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    let mut runtime = support::open_runtime(path).unwrap();
    support::agent_task_admission::draft(&mut runtime, None);
    support::agent_task_admission::source_fact(&mut runtime)
}

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Service {
    Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .args(["--database"])
            .arg(database)
            .args(["--socket"])
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start real Application Service"),
    )
}

fn ready(client: &ApplicationServiceClient, child: &mut Service) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.0.try_wait().expect("inspect service").is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn read_publication(
    client: &ApplicationServiceClient,
    through: &nola_domain::assistant_journal::AssistantJournalCursor,
) -> Vec<Value> {
    use nola_application::AssistantJournalReadResult;
    let mut after=nola_domain::assistant_journal::AssistantJournalCursor::initial(through.identity.clone()).unwrap();
    let mut pages=Vec::new();
    for _ in 0..128 {
        let command=ApplicationCommand::ReadAssistantJournal {
            after:after.clone(),through:through.clone(),max_records:1,max_encoded_bytes:65536,
        };
        assert!(command.is_read_only_query(), "reads must not write receipts or flush Core deltas");
        let page:AssistantJournalReadResult=client.execute(command).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len()<=65536);
        pages.push(serde_json::to_value(&page).unwrap());
        match page {
            AssistantJournalReadResult::Records { after:observed,through:fixed,records,next,complete } => {
                assert_eq!(observed,after);assert_eq!(fixed,*through);
                assert!(records.len()<=1);
                for record in records {
                    record.batch.verify_header(&record.header).unwrap();
                    assert_eq!(record.header.sequence,after.sequence+1);
                    assert_eq!(record.header.previous_record_digest,after.record_digest);
                }
                if complete {assert_eq!(next,*through);return pages;}
                assert!(next.sequence>after.sequence);
                after=next;
            }
            AssistantJournalReadResult::SnapshotRequired => panic!("newly published fixture cannot be pruned"),
        }
    }
    panic!("journal pagination failed to finish");
}


#[test]
fn guidance_receipt_survives_service_restart_but_adoption_requires_fresh_observation() {
    let directory=tempfile::tempdir().unwrap();
    let database=directory.path().join("nola.db");
    let socket=directory.path().join("nola.sock");
    let task_admission=seed(&database);
    let client=ApplicationServiceClient::with_socket(&database,&socket);
    let mut first=start_service(&database,&socket);ready(&client,&mut first);
    let authority = client.import_agent_task_admission_with_request_id("task-admission", &task_admission).unwrap();
    assert_eq!(authority.admission.as_ref(), Some(&task_admission));
    let attempt: nola_application::RuntimeTaskAttempt = client.execute(ApplicationCommand::RuntimeTask {
        command:RuntimeTaskCommand::Claim {task_id:"task".into(),worker_id:"worker".into(),worker_generation:"generation".into(),
            lease_token:"token".into(),now_ms:900,lease_until_ms:10000}
    }).unwrap();
    let lease=RuntimeTaskLeaseWitness::from(&attempt);
    let admission:nola_application::AcceptedAssistantTurn=client.execute(ApplicationCommand::BeginTaskAssistantTurn {lease:lease.clone(),input:begin()}).unwrap();
    assert!(!admission.assistant_publications.is_empty(), "Service must carry committed admission watermarks");
    let admission_cursor=admission.assistant_publications[0].clone();
    let admission_snapshot:nola_application::AssistantJournalSnapshotHandle=client.execute(
        ApplicationCommand::OpenAssistantJournalSnapshot {at:admission_cursor.clone()}).unwrap();
    let admission_parts:nola_application::AssistantJournalPartPage=client.execute(
        ApplicationCommand::ReadAssistantJournalParts {
            snapshot:admission_snapshot.clone(),after_part_id:None,max_records:128,max_encoded_bytes:65536,
        }).unwrap();
    assert!(admission_parts.parts.iter().all(|part|part.bodies.is_empty()),
        "admission must not invent assistant text before a provider response");
    let _:Value=client.execute(ApplicationCommand::ReleaseAssistantJournalSnapshot {
        snapshot:admission_snapshot,
    }).unwrap();

    // Publish a real streamed body before pinning a text range. Admission alone
    // is an empty assistant projection, not the submitted user prompt.
    let claimed:nola_application::CommittedHostRequests=client.execute(
        ApplicationCommand::ClaimCommittedHostRequests {
            input:nola_application::ClaimCommittedHostRequests {
                run_id:admission.begin.run_id.clone(),worker_generation:attempt.worker_generation.clone(),
                lease_owner:attempt.worker_id.clone(),claimed_at_ms:1001,lease_expires_at_ms:10000,limit:1,
            },
        }).unwrap();
    let request=claimed.requests.into_iter()
        .find(|request|matches!(request.payload,HostRequest::ModelStream {..})).unwrap();
    let text_part_id=match &request.payload {
        HostRequest::ModelStream {text_part_id,..}=>text_part_id.clone(),
        _=>unreachable!(),
    };
    // A real IPC wait must wake on the source commit, without a poll or
    // forwarding a different Run's notification into this stream.
    let waiting_client = client.clone();
    let waiting_after = admission_cursor.clone();
    let (head_tx, head_rx) = std::sync::mpsc::channel();
    let waiting = thread::spawn(move || {
        let result = waiting_client.assistant_journal_wait(&nola_protocol::AssistantJournalWaitRequest {
            after: waiting_after, wait_ms: 30_000,
        }, &nola_service_client::ApplicationServiceRequestCancellation::new());
        head_tx.send(result).unwrap();
    });
    assert!(head_rx.recv_timeout(Duration::from_millis(100)).is_err());
    let mut initial_cursor=admission_cursor;
    for (index,payload) in [
        HostResponse::ExecutionStarted,
        HostResponse::ModelAttemptStarted {
            provider:"fixture".into(),model:"fixture-model".into(),reason:"initial".into(),
        },
        HostResponse::ModelDelta {
            channel:ModelDeltaChannel::Text,part_id:text_part_id,delta:"Original streamed evidence".into(),
        },
    ].into_iter().enumerate() {
        let accepted:nola_application::AcceptedHostResponse=client.execute(
            ApplicationCommand::ReduceAndAcceptHostResponse {
                input:nola_application::ReduceAndAcceptHostResponse {
                    conversation_id:"child".into(),run_id:admission.begin.run_id.clone(),
                    invocation_config_digest:admission.invocation_config_digest.clone(),
                    response:HostResponseEnvelope {
                        request_id:request.request_id.clone(),effect_id:request.effect_id.clone(),
                        host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),
                        session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),
                        catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,
                        attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
                        delivery:request.delivery.clone(),sequence:request.resume_after_sequence.unwrap_or(0)+index as u64+1,payload,
                    },received_at_ms:1002+index as i64,
                },
            }).unwrap();
        if let Some(cursor)=accepted.commit.assistant_publications.last() {
            initial_cursor=cursor.clone();
        }
    }
    assert!(initial_cursor.sequence>admission.assistant_publications[0].sequence);
    let woke = head_rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    waiting.join().unwrap();
    assert_eq!(woke.identity, initial_cursor.identity);
    assert!(woke.sequence > admission.assistant_publications[0].sequence);
    assert!(woke.sequence <= initial_cursor.sequence);
    let mut foreign = initial_cursor.clone(); foreign.identity.run_id = "foreign-run".into();
    assert!(client.assistant_journal_wait(&nola_protocol::AssistantJournalWaitRequest {
        after: foreign, wait_ms: 0,
    }, &nola_service_client::ApplicationServiceRequestCancellation::new()).is_err());
    let stop = nola_service_client::ApplicationServiceRequestCancellation::new();
    let stop_wait = stop.clone(); let cancel_client = client.clone(); let cancel_after = initial_cursor.clone();
    let cancelling = thread::spawn(move || cancel_client.assistant_journal_wait(
        &nola_protocol::AssistantJournalWaitRequest { after: cancel_after, wait_ms: 30_000 }, &stop_wait));
    thread::sleep(Duration::from_millis(50));
    let cancelled_at = Instant::now(); stop.cancel();
    assert!(cancelling.join().unwrap().is_err());
    assert!(cancelled_at.elapsed() < Duration::from_secs(2));
    let initial_pages=read_publication(&client,&initial_cursor);
    let pinned:nola_application::AssistantJournalSnapshotHandle=client.execute(
        ApplicationCommand::OpenAssistantJournalSnapshot {at:initial_cursor.clone()}).unwrap();
    let (pinned_part,pinned_content)={
        let parts:nola_application::AssistantJournalPartPage=client.execute(
            ApplicationCommand::ReadAssistantJournalParts {
                snapshot:pinned.clone(),after_part_id:None,max_records:128,max_encoded_bytes:65536,
            }).unwrap();
        assert_eq!(parts.at,pinned.at);
        assert!(parts.next_part_id.is_none());
        assert!(serde_json::to_vec(&parts).unwrap().len()<=65536);
        let part=parts.parts.into_iter().find(|part|!part.bodies.is_empty()).unwrap();
        (part.part_id,part.bodies[0].clone())
    };
    let range_command=||ApplicationCommand::ReadAssistantJournalRange {
        snapshot:pinned.clone(),part_id:pinned_part.clone(),content:pinned_content.clone(),
        offset_bytes:nola_domain::assistant_journal_operations::JournalNumber(0),limit_bytes:32768,
    };
    let original_range:nola_application::AssistantJournalTextRange=client.execute(range_command()).unwrap();
    assert_eq!(original_range.text,"Original streamed evidence");
    let mut forged=initial_cursor.clone();
    forged.sequence+=1;
    let bad:Result<Value,_>=client.execute(ApplicationCommand::ReadAssistantJournal {
        after:nola_domain::assistant_journal::AssistantJournalCursor::initial(initial_cursor.identity.clone()).unwrap(),
        through:forged,max_records:1,max_encoded_bytes:65536,
    });
    assert!(bad.is_err(), "a client cannot advance a committed watermark");
    {
        use nola_store_port::assistant_journal::AssistantJournalRepository;
        let store=nola_store::NolaStore::open(&database).unwrap();
        assert_eq!(store.assistant_journal_head(&initial_cursor.identity).unwrap(), initial_cursor);
        let admission_window=read_publication(&client,&admission.assistant_publications[0]);
        assert!(!admission_window.is_empty(),
            "the admitted empty baseline stays readable after real streamed content");
    }
    let binding:Option<RuntimeTaskRunBinding>=client.execute(ApplicationCommand::RuntimeTask {
        command:RuntimeTaskCommand::RunBinding {task_id:"task".into()}
    }).unwrap();
    let run=binding.unwrap().run_id;
    let _:Value=client.execute(ApplicationCommand::SubmitTaskGuidance {
        task_id:"task".into(),message_id:"g1".into(),parent_turn_id:task_admission.parent_turn_id.clone(),
        content:"Preserve the original Run".into(),submitted_at_ms:1050,
    }).unwrap();
    {
        let mut store=nola_store::NolaStore::open(&database).unwrap();
        let queued=store.claim_next_agent_command("parent","fixture-parent",1100,2100).unwrap().unwrap();
        let notification:nola_core::agent::host::AgentBoundaryInput=serde_json::from_str(&queued.payload_json).unwrap();
        let nola_core::agent::host::AgentBoundaryInput::TaskGuidanceAvailable {task_id,message_id,settlement}=notification else {panic!("expected typed guidance notice")};
        assert_eq!(queued.command_id,format!("task-guidance-{}:{}:{}",if settlement{"result"}else{"admit"},task_id,message_id));
        let projection:nola_domain::agent_task_admission::AgentTaskGuidanceProjection=client.execute(
            ApplicationCommand::ExportAgentTaskGuidanceProjection {task_id,message_id,settlement}
        ).unwrap();
        use nola_store_port::assistant_journal::AssistantJournalRepository;
        let identity=nola_domain::assistant_journal::AssistantJournalIdentity {
            source_store_epoch:task_admission.source_store_epoch.clone(),conversation_id:"parent".into(),
            run_id:task_admission.parent_run_id.clone(),assistant_message_id:"parent-assistant".into(),
        };
        let before=store.assistant_journal_head(&identity).unwrap();
        let load_parent=|| -> Value {
            let stored:Value=client.execute(ApplicationCommand::LoadCheckpoint {conversation_id:"parent".into()}).unwrap();
            serde_json::from_str(stored["checkpoint_json"].as_str().unwrap()).unwrap()
        };
        let parent_before=load_parent();
        let applied=client.import_agent_task_guidance_projection_with_request_id(&queued.command_id,&projection).unwrap();
        assert_eq!(applied.conversation_id,"parent");
        assert_eq!(applied.task_id,"task");assert_eq!(applied.message_id,"g1");assert!(!applied.settlement);
        let parent_after=load_parent();
        assert!(applied.core_revision>parent_before["revision"].as_u64().unwrap());
        assert_eq!(parent_after["revision"].as_u64(),Some(applied.core_revision));
        assert!(parent_after["state"]["task_projection"]["revision"].as_u64().unwrap()
            >parent_before["state"]["task_projection"]["revision"].as_u64().unwrap());
        let pending=parent_after["state"]["task_projection"]["guidance"]["task"]["pending"].as_array().unwrap();
        assert_eq!(pending.len(),1);
        let guidance=&pending[0];
        assert_eq!(guidance["message_id"],"g1");
        assert_eq!(guidance["content"],projection.guidance.content);
        assert_eq!(guidance["parent_session_id"],projection.guidance.parent_session_id);
        assert_eq!(guidance["parent_turn_id"],projection.guidance.parent_turn_id);
        assert_eq!(guidance["input_watermark"],json!(projection.guidance.input_watermark));
        assert_eq!(guidance["enqueued_at_epoch_ms"],json!(projection.guidance.submitted_at_ms));
        let after=store.assistant_journal_head(&identity).unwrap();
        // Guidance admission changes the private queue, not the public Task
        // card's state/progress/result. Do not manufacture a journal record for
        // a projection revision with no visible semantic change.
        assert_eq!(after,before,"private guidance admission must not fabricate public changes");
        assert_eq!(after.identity.conversation_id,"parent");
        assert_eq!(after.identity.run_id,task_admission.parent_run_id);
        assert!(!read_publication(&client,&after).is_empty());
        assert!(store.acknowledge_agent_command("parent",&queued.command_id,"fixture-parent").unwrap());
    }
    let apply=|lease,now_ms|ApplicationCommand::ApplyTaskGuidance {
        task_id:"task".into(),message_id:"g1".into(),expected_run_id:run.clone(),lease,now_ms,
    };
    // Deliberately do not acknowledge the child FIFO. The transport response is
    // captured only by this test; model a Host that lost it before adoption.
    let first_result:Value=client.execute_with_request_id("lost-guidance-ack",apply(Some(lease.clone()),1150)).unwrap();
    let first_view:TaskGuidanceApplyResult=serde_json::from_value(first_result.clone()).unwrap();
    assert_eq!(first_view.accepted.unwrap().control.queued_input_count,1);
    let original_receipt=client.command_receipt("lost-guidance-ack").unwrap().unwrap();
    let digest=nola_store::NolaStore::open(&database).unwrap().active_run("child").unwrap().unwrap().invocation_config_digest.unwrap();
    let newer:nola_application::AcceptedActiveSubmission=client.execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
        input:nola_application::ReduceAndCommitActiveSubmission {
            conversation_id:"child".into(),run_id:run.clone(),transition_id:"later-input".into(),
            invocation_config_digest:digest,submission:AgentSubmission::UserInput {
                items:vec![AgentItem::user_message(ItemId::from_string("later-input".into()),"Later input")],
                turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                submission_id:Some("later-input".into()),submitted_at_epoch_ms:Some(1200),
                delivery:AgentInputDelivery::CurrentTurn,
            },committed_at_ms:1200,
        },
    }).unwrap();
    assert_eq!(newer.control.queued_input_count,2);
    drop(first);
    let mut second=start_service(&database,&socket);ready(&client,&mut second);
    assert_eq!(read_publication(&client,&initial_cursor),initial_pages,
        "a fixed journal window must survive later commits and Service restart");
    let reopened_range:nola_application::AssistantJournalTextRange=client.execute(range_command()).unwrap();
    assert_eq!(serde_json::to_value(&reopened_range).unwrap(),serde_json::to_value(&original_range).unwrap());
    let before=nola_store::NolaStore::open(&database).unwrap().load_checkpoint("child").unwrap().unwrap();
    let retry:Value=client.execute_with_request_id("lost-guidance-ack",apply(Some(lease),1150)).unwrap();
    assert_eq!(retry,first_result,"same request replays immutable result");
    let looked_up=client.command_receipt("lost-guidance-ack").unwrap().unwrap();
    assert_eq!(serde_json::to_value(&looked_up).unwrap(),serde_json::to_value(&original_receipt).unwrap());
    let after=nola_store::NolaStore::open(&database).unwrap().load_checkpoint("child").unwrap().unwrap();
    assert_eq!(before.checkpoint_json,after.checkpoint_json,"receipt lookup/retry cannot flush or inject");
    // Only this new explicit command supplies an adoptable CURRENT checkpoint.
    let observation:TaskGuidanceApplyResult=client.execute_with_request_id("observe-after-lost-ack",apply(None,1300)).unwrap();
    assert_eq!(observation.guidance.status,"delivered");
    let accepted=observation.accepted.unwrap();
    assert_eq!(accepted.control.queued_input_count,2);
    assert!(accepted.events.iter().all(|event|!matches!(event.event,nola_core::agent::AgentEvent::InputQueued {..})));
    drop(second);
    let mut third=start_service(&database,&socket);ready(&client,&mut third);
    let again:TaskGuidanceApplyResult=client.execute_with_request_id("observe-after-second-restart",apply(None,1400)).unwrap();
    assert_eq!(again.accepted.unwrap().control.queued_input_count,2);
    let db=rusqlite::Connection::open(&database).unwrap();
    let runs:i64=db.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id='child'",[],|row|row.get(0)).unwrap();
    assert_eq!(runs,1);
    let stored:String=db.query_row("SELECT result_json FROM command_receipts WHERE request_id='lost-guidance-ack'",[],|row|row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(),first_result,"observations must not rewrite original receipt");
    for _ in 0..2 {
        let _:Value=client.execute(ApplicationCommand::ReleaseAssistantJournalSnapshot {snapshot:pinned.clone()}).unwrap();
    }
    let released:Result<Value,_>=client.execute(range_command());
    assert!(released.unwrap_err().contains("released"));
}
