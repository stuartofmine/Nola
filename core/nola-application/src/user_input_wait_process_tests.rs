// SPDX-License-Identifier: Apache-2.0
//! Real process death after committed wait/answer; no live provider or user DB.
use super::*;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const WORKER: &str = "user_input_wait_tests::process_recovery::committed_wait_process_worker";

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn killed_worker(path: &Path, stage: &str) -> serde_json::Value {
    let ready = path.with_extension(format!("{stage}.ready"));
    let log = path.with_extension(format!("{stage}.stderr"));
    let mut child = ChildGuard(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", WORKER, "--ignored", "--nocapture", "--test-threads=1"])
        .env("NOLA_WAIT_PROCESS_STAGE", stage)
        .env("NOLA_WAIT_PROCESS_DB", path)
        .env("NOLA_WAIT_PROCESS_READY", &ready)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(30);
    let committed = loop {
        if let Ok(bytes) = std::fs::read(&ready) {
            if let Ok(value) = serde_json::from_slice(&bytes) { break value; }
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("worker exited before commit acknowledgement: {status}; {}", std::fs::read_to_string(&log).unwrap_or_default());
        }
        assert!(Instant::now() < deadline, "worker readiness timeout: {}", std::fs::read_to_string(&log).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(10));
    };
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert!(!status.success(), "worker must not exit normally");
    #[cfg(unix)] {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "expected actual SIGKILL");
    }
    committed
}

#[test]
#[ignore = "worker entrypoint invoked only by the process recovery test"]
fn committed_wait_process_worker() {
    let Ok(stage) = std::env::var("NOLA_WAIT_PROCESS_STAGE") else { return; };
    let path = std::path::PathBuf::from(std::env::var_os("NOLA_WAIT_PROCESS_DB").unwrap());
    let ready = std::path::PathBuf::from(std::env::var_os("NOLA_WAIT_PROCESS_READY").unwrap());
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    if stage == "open" {
        let accepted = crate::assistant_turn::admit_with_core(&mut runtime.store, BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id:"conversation".into(),conversation_title:"Process recovery".into(),
            user_message_id:"origin".into(),user_part_id:"body".into(),user_text:"Ask for scope".into(),
            user_content_json:None,assistant_message_id:"assistant".into(),context_checkpoint_id:None,
            core_config:config(),created_at_ms:1000,
            submission:AgentSubmission::UserInput {
                items:vec![AgentItem::user_message(ItemId::from_string("origin".into()),"Ask for scope")],
                turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                submission_id:Some("origin".into()),submitted_at_epoch_ms:Some(1000),delivery:AgentInputDelivery::CurrentTurn,
            },
        },None).unwrap().0;
        complete_model(&mut runtime.store,&accepted.begin.run_id,&accepted.invocation_config_digest,true,1001);
    }
    let waits: Vec<StoredUserInputWait> = serde_json::from_value(runtime.execute(
        ApplicationCommand::ListOpenUserInputWaits { conversation_id: "conversation".into(), run_id: None }
    ).unwrap()).unwrap();
    assert_eq!(waits.len(), 1);
    let wait = &waits[0];
    if stage == "answer" {
        assert_eq!(runtime.execute(ApplicationCommand::RespondToConversationWait { input: answer(wait) }).unwrap()["type"], "applied");
    } else { assert_eq!(stage, "open"); }
    let requests = load_committed_host_requests(&runtime.store, &wait.run_id).unwrap().requests;
    let request_ids: Vec<_> = requests.iter().map(|request| request.request_id.clone()).collect();
    std::fs::write(ready, serde_json::to_vec(&serde_json::json!({"wait":wait,"request_ids":request_ids})).unwrap()).unwrap();
    // Keep the actual Application/SQLite handles alive: the parent kills us,
    // rather than accepting clean function-return/drop as crash evidence.
    loop { std::thread::park_timeout(Duration::from_secs(30)); }
}

#[test]
fn killed_process_preserves_committed_question_and_exact_answer_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("process-wait.sqlite");
    let opened = killed_worker(&path, "open");
    let original: StoredUserInputWait = serde_json::from_value(opened["wait"].clone()).unwrap();
    {
        let mut runtime = ApplicationRuntime::open(&path).unwrap();
        let waits: Vec<StoredUserInputWait> = serde_json::from_value(runtime.execute(
            ApplicationCommand::ListOpenUserInputWaits {conversation_id:"conversation".into(),run_id:Some(original.run_id.clone())}
        ).unwrap()).unwrap();
        assert_eq!(waits.len(),1);
        assert_eq!(waits[0].wait, original.wait);
        assert_eq!(waits[0].run_id, original.run_id);
        assert!(load_committed_host_requests(&runtime.store,&original.run_id).unwrap().requests.is_empty());
    }
    let answered = killed_worker(&path, "answer");
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    assert!(runtime.store.open_user_input_waits("conversation",None).unwrap().is_empty());
    let replay = runtime.execute(ApplicationCommand::RespondToConversationWait { input: answer(&original) }).unwrap();
    assert_eq!(replay["type"],"replay");
    assert_eq!(replay["resume_policy"],"conversation_run");
    let requests = load_committed_host_requests(&runtime.store,&original.run_id).unwrap().requests;
    let ids: Vec<_> = requests.iter().map(|request| request.request_id.clone()).collect();
    assert_eq!(serde_json::to_value(&ids).unwrap(),answered["request_ids"]);
    assert_eq!(ids.len(),1,"answer replay must not append another model continuation");
}
