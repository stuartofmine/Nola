// SPDX-License-Identifier: Apache-2.0
use nola_store::{NolaStore,EnqueueAgentCommandInput};
#[test]
fn repeated_agent_command_preserves_original_time_but_rejects_changed_payload() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("inbox.sqlite");
    let mut store=NolaStore::open(&path).unwrap();
    let mut input=EnqueueAgentCommandInput {session_id:"parent".into(),command_id:"stable".into(),payload_json:"{\"operation\":1}".into(),created_at_ms:100};
    assert!(store.enqueue_agent_command(&input).unwrap());input.created_at_ms=200;
    assert!(!store.enqueue_agent_command(&input).unwrap());
    input.payload_json="{\"operation\":2}".into();assert!(store.enqueue_agent_command(&input).is_err());
    let command=store.claim_next_agent_command("parent","consumer",300,400).unwrap().unwrap();
    assert_eq!(command.created_at_ms,100);assert_eq!(command.payload_json,"{\"operation\":1}");
}
