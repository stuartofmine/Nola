// SPDX-License-Identifier: Apache-2.0
//! A leased head must not let another consumer reorder parent Task commands.
use nola_store::{EnqueueAgentCommandInput, NolaStore};

#[test]
fn leased_head_blocks_successors_across_connections_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fifo.sqlite");
    let mut first = NolaStore::open(&path).unwrap();
    for (session, id, time) in [("parent", "waiting", 100), ("parent", "resume", 90),
        ("parent", "recover", 80), ("other", "independent", 70)] {
        first.enqueue_agent_command(&EnqueueAgentCommandInput {
            session_id:session.into(),command_id:id.into(),payload_json:"{}".into(),created_at_ms:time,
        }).unwrap();
    }
    let head = first.claim_next_agent_command("parent","worker-a",200,300).unwrap().unwrap();
    assert_eq!(head.command_id,"waiting");
    let mut second = NolaStore::open(&path).unwrap();
    assert!(second.claim_next_agent_command("parent","worker-b",210,310).unwrap().is_none());
    assert!(second.claim_next_agent_command("other","worker-b",210,310).unwrap().is_some());
    drop(first);
    drop(second);
    let mut reopened = NolaStore::open(&path).unwrap();
    assert!(reopened.claim_next_agent_command("parent","worker-b",299,399).unwrap().is_none());
    let replay = reopened.claim_next_agent_command("parent","worker-b",300,400).unwrap().unwrap();
    assert_eq!(replay.command_id,"waiting");
    assert!(!reopened.acknowledge_agent_command("parent","waiting","worker-a").unwrap());
    assert!(reopened.claim_next_agent_command("parent","worker-c",301,401).unwrap().is_none());
    assert!(reopened.acknowledge_agent_command("parent","waiting","worker-b").unwrap());
    let resume = reopened.claim_next_agent_command("parent","worker-b",302,402).unwrap().unwrap();
    assert_eq!(resume.command_id,"resume");
    assert!(reopened.release_agent_command_lease("parent","resume","worker-b").unwrap());
    let resume = reopened.claim_next_agent_command("parent","worker-c",303,403).unwrap().unwrap();
    assert_eq!(resume.command_id,"resume", "released head was skipped");
    assert!(reopened.acknowledge_agent_command("parent","resume","worker-c").unwrap());
    assert_eq!(reopened.claim_next_agent_command("parent","worker-c",304,404).unwrap().unwrap().command_id,"recover");
}
