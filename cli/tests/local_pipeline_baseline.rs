// SPDX-License-Identifier: Apache-2.0
//! Opt-in local persistence baseline. Synthetic Host packets, real Application and SQLite.
use nola_application::{ApplicationCommand as Command, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand};
use nola_application_bootstrap::{open, ApplicationRuntime};
use nola_core::agent::{host::*, protocol::AgentInputDelivery, AgentCoreConfig, AgentItem, AgentSubmission, ItemId, SessionId};
use nola_protocol::CommandMetadata;
use serde_json::{json, Value};
use std::{path::Path, time::Instant};

fn received(app: &mut ApplicationRuntime, command: Command, request: &str) -> Result<Value, String> {
    let receipt = app.execute_received_command(ExecuteReceivedCommand {
        client_id: "local-baseline".into(), request_id: request.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(&command).unwrap(), expected_revision: None,
        received_at: "2026-10-11T00:00:00Z".into(),
    }, command).map_err(|error|error.to_string())?;
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => Ok(result),
        other => Err(format!("non-committed receipt: {other:?}")),
    }
}

fn measure<T>(samples: &mut Vec<Value>, condition: &str, index: usize, stage: &str,
    f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let capture=nola_store::commit_diagnostics::begin_capture().expect("non-nested baseline capture");
    let start = Instant::now();
    let result = f();
    let elapsed_ns=start.elapsed().as_nanos().to_string();
    let diagnostics=capture.finish();
    samples.push(json!({"condition":condition,"sample":index,"stage":stage,
        "elapsed_ns":elapsed_ns,"commit_diagnostics":diagnostics,
        "outcome":if result.is_ok(){"succeeded"}else{"failed"},
        "error":result.as_ref().err()}));
    result
}

fn run_sample(app: &mut ApplicationRuntime, database: &Path, samples: &mut Vec<Value>, condition: &str, index: usize) -> Result<(), String> {
    let id = format!("baseline-{condition}-{index}");
    let capability = HostCapabilityDescriptor {
        id:"model.stream".into(),title:"synthetic packet source".into(),route:HostCapabilityRoute::Model,
        availability:CapabilityAvailability::Available,exposure:HostCapabilityExposure::Internal,
        permission_domain:None,input_schema:json!({"type":"object"}),input_normalization:Default::default(),
        output_policy:HostOutputPolicy {preview_char_limit:4000,persist_full_output:true,allow_artifacts:true},
        supports_streaming:true,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,
        cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable,
    };
    let command = Command::BeginAssistantTurn { input:nola_application::BeginAssistantTurn {
        expected_parent_message_id:None,expected_configuration_revision:None,
        conversation_id:id.clone(),conversation_title:"Baseline".into(),user_message_id:format!("{id}-u"),
        user_part_id:format!("{id}-up"),user_text:"Local pipeline baseline".into(),user_content_json:None,
        assistant_message_id:format!("{id}-a"),context_checkpoint_id:None,
        core_config:AgentCoreConfig {session_id:Some(SessionId::from_string(id.clone())),model:Some("synthetic".into()),
            host_catalog:Some(HostCapabilityCatalog {host_id:HostId("baseline-host".into()),revision:1,
                availability_epoch:1,lease:None,capabilities:[("model.stream".into(),capability)].into_iter().collect()}),
            ..Default::default()},
        submission:AgentSubmission::UserInput {
            items:vec![AgentItem::user_message(ItemId::from_string(format!("{id}-u")),"Local pipeline baseline")],
            turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
            submission_id:Some(id.clone()),submitted_at_epoch_ms:Some(1000),delivery:AgentInputDelivery::CurrentTurn,
        },created_at_ms:1000,
    }};
    let accepted: nola_application::AcceptedAssistantTurn = serde_json::from_value(measure(samples,condition,index,"admission_receipt",
        ||received(app,command,&format!("{id}-admit")))?).map_err(|e|e.to_string())?;
    let now=nola_runtime_driver::monotonic_epoch_millis();
    let requests: nola_application::CommittedHostRequests=serde_json::from_value(measure(samples,condition,index,"claim_receipt",||
        received(app,Command::ClaimCommittedHostRequests {input:nola_application::ClaimCommittedHostRequests {
            run_id:id.clone(),worker_generation:"baseline".into(),lease_owner:"baseline".into(),
            claimed_at_ms:now,lease_expires_at_ms:now+60_000,limit:32,
        }},&format!("{id}-claim")))?).map_err(|e|e.to_string())?;
    let request=requests.requests.into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).ok_or("missing model request")?;
    let part_id=match &request.payload {HostRequest::ModelStream{text_part_id,..}=>text_part_id.clone(),_=>unreachable!()};
    for (sequence,payload) in [HostResponse::ExecutionStarted,
        HostResponse::ModelAttemptStarted{provider:"synthetic".into(),model:"synthetic".into(),reason:"initial".into()},
        HostResponse::ModelDelta{channel:ModelDeltaChannel::Text,part_id:part_id.clone(),delta:"x".repeat(1024)},
    ].into_iter().enumerate() {
        let stage=["execution_started_receipt","attempt_started_receipt","delta_1024b_receipt"][sequence];
        measure(samples,condition,index,stage,||received(app,Command::ReduceAndAcceptHostResponse {
            input:nola_application::ReduceAndAcceptHostResponse {
                conversation_id:id.clone(),run_id:id.clone(),invocation_config_digest:accepted.invocation_config_digest.clone(),
                received_at_ms:now+sequence as i64,response:HostResponseEnvelope {
                    request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
                    effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),
                    session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,
                    availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence:sequence as u64+1,payload,
                },
            },
        },&format!("{id}-response-{sequence}")))?;
    }
    let identity=accepted.assistant_publications.last().ok_or("missing read origin")?.identity.clone();
    let head=app.execute(Command::LocalAssistantJournalHead {identity:identity.clone()})?;
    let frontier: nola_domain::assistant_journal::AssistantJournalCursor = serde_json::from_value(head.clone()).map_err(|e|e.to_string())?;
    if frontier.sequence==0 {return Err("no committed journal frontier".into());}
    let mut reopened=measure(samples,condition,index,"reopen_connection",||open(database))?;
    let recovered=measure(samples,condition,index,"recovered_journal_head",||reopened.execute(Command::LocalAssistantJournalHead{identity}))?;
    if head!=recovered {return Err("ACK frontier lost across connection reopen".into());}
    let projection=measure(samples,condition,index,"recovered_session_projection",||reopened.execute(Command::LoadSessionProjection{conversation_id:id.clone()}))?;
    if projection["conversation_id"]!=id || projection["conversation_projection"]["parts"][&part_id]["text"]!="x".repeat(1024) {
        return Err("recovered session identity or exact 1024-byte first body mismatched".into());
    }
    Ok(())
}

#[test]
#[ignore = "explicit local persistence performance experiment; set NOLA_BASELINE_OUTPUT"]
fn local_application_sqlite_baseline() {
    let output=std::env::var("NOLA_BASELINE_OUTPUT").expect("explicit output path required");
    let directory=tempfile::tempdir().unwrap();
    let mut samples=vec![];
    let mut failures=vec![];
    for condition in ["fresh_database","reused_database"] {
        let warm_path=directory.path().join("warm.sqlite");
        let mut warm=if condition=="reused_database" {
            match open(&warm_path) {
                Ok(app)=>Some(app),
                Err(error)=>{failures.push(json!({"condition":condition,"stage":"condition_setup","error":error}));continue;}
            }
        }else{None};
        for index in 0..30 {
            let path=if warm.is_some(){warm_path.clone()}else{directory.path().join(format!("cold-{index}.sqlite"))};
            let mut cold=None;
            if warm.is_none(){
                match measure(&mut samples,condition,index,"fresh_database_open",||open(&path)) {
                    Ok(app)=>cold=Some(app),
                    Err(error)=>{failures.push(json!({"condition":condition,"sample":index,"error":error}));continue;}
                }
            }
            let app=warm.as_mut().or(cold.as_mut()).unwrap();
            if let Err(error)=run_sample(app,&path,&mut samples,condition,index){failures.push(json!({"condition":condition,"sample":index,"error":error}));}
        }
    }
    let report=json!({"schema_version":1,"scenario":"local-application-sqlite-v1","profile":"cargo-test-dev",
        "commit_path":"database_only_ack",
        "samples_per_condition":30,"provider":"not called; injected Host envelopes","concurrency":1,
        "clock":"std::time::Instant; same process","recovery":"second connection, not process restart",
        "samples":samples,"failures":failures});
    std::fs::write(output,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(failures.is_empty(),"{failures:?}");
}
