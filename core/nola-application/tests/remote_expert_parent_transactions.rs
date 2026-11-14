// SPDX-License-Identifier: Apache-2.0
//! Isolated authenticated-transport receiver facts; no Run or provider is fabricated.
mod support;
use nola_application::{remote_expert_parent::*, ApplicationCommand};
use nola_domain::{expert_message::*, remote_expert_parent::source_key, *};
use nola_store::NolaStore;
use nola_store_port::conversation_history::ConversationProjectionRepository;
use serde_json::json;
fn admission(message: &str, revision: i64) -> RemoteExpertParentAdmission {
    RemoteExpertParentAdmission {
        version: 1,
        source_host_id: "Host".into(),
        source_store_epoch: "epoch".into(),
        conversation_id: "parent".into(),
        source_revision: revision,
        source_predecessor_message_id: None,
        input: AdmitExpertConversationMessage {
            conversation_id: "parent".into(),
            expected_revision: revision - 1,
            room_id: "9f7d0000-4e6f-4c61-8000-000000000001".into(),
            expected_room_revision: 1,
            submission_id: uuid::Uuid::new_v4().to_string(),
            user_message_id: message.into(),
            text: format!("Message {message}"),
            selected_participants: vec![ExpertParticipantReference {
                participant_id: "member".into(),
                agent_id: "agent".to_string().try_into().unwrap(),
                agent_revision: 1,
                context_conversation_id: "child".into(),
            }],
            created_at_ms: 10,
        },
        message: MessageView {
            message_id: MessageId(message.into()),
            role: MessageRole::User,
            state: MessageState::Completed,
            revision: revision as u64,
            created_at_ms: 10,
            parts: vec![MessagePartView::Text {
                part_id: MessagePartId(format!("{message}:text")),
                revision: revision as u64,
                markdown: format!("Message {message}"),
                is_final: true,
                review: None,
            }],
        },
    }
}
fn setup(path: &std::path::Path) {
    let mut store = NolaStore::open(path).unwrap();
    store.create_conversation(&nola_store_port::conversation_profile::CreateConversationInput{id:"parent".into(),title:"Remote expert parent".into(),revision:1,settings_json:Some(json!({"executionHostBinding":{"baseURL":"https://fixture.invalid","tlsCertificateSHA256":null}}).to_string()),created_at_ms:1,updated_at_ms:1}).unwrap();
}
fn receive(
    runtime: &mut support::ApplicationRuntime,
    value: RemoteExpertParentAdmission,
) -> Result<serde_json::Value, String> {
    runtime.execute(ApplicationCommand::ReceiveRemoteExpertParentAdmission {
        expected_source_host_id: "Host".into(),
        expected_source_store_epoch: "epoch".into(),
        admission: value,
    })
}
fn count(path: &std::path::Path, table: &str) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn receive_user_without_run_sets_explicit_source_and_exact_retry_is_immutable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mirror.sqlite");
    setup(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let value = admission("user-one", 2);
    let first = receive(&mut runtime, value.clone()).unwrap();
    assert_eq!(first["source_revision"], 2);
    assert_eq!(receive(&mut runtime, value.clone()).unwrap(), first);
    let mut changed = value.clone();
    changed.input.text = "changed".into();
    assert!(receive(&mut runtime, changed).is_err());
    assert_eq!(count(&path, "runs"), 0);
    assert_eq!(count(&path, "messages"), 1);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store.conversation_source_owner("parent").unwrap(),
        ConversationSourceOwner::Remote {
            source_host_id: "Host".into(),
            source_store_epoch: "epoch".into()
        }
    );
    let projection = nola_application::load_conversation_projection_v1(&store, "parent").unwrap();
    assert_eq!(projection.messages.len(), 1);
    assert_eq!(projection.messages[0].message_id, value.message.message_id);
    assert_eq!(projection.messages[0].parts[0], value.message.parts[0]);
}
#[test]
fn source_revision_conflicts_and_wrong_owners_leave_no_partial_user() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rejected.sqlite");
    setup(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    receive(&mut runtime, admission("first", 4)).unwrap();
    let low = admission("unknown-old", 3);
    assert!(receive(&mut runtime, low)
        .unwrap_err()
        .contains("revision_conflict"));
    let mut foreign = admission("foreign", 5);
    foreign.source_host_id = "Other".into();
    assert!(receive(&mut runtime, foreign).is_err());
    let mut foreign = admission("foreign", 5);
    foreign.source_store_epoch = "Other".into();
    assert!(receive(&mut runtime, foreign).is_err());
    let mut assistant = admission("assistant", 5);
    assistant.message.role = MessageRole::Assistant;
    assert!(receive(&mut runtime, assistant).is_err());
    assert_eq!(count(&path, "messages"), 1);
    assert_eq!(count(&path, "runs"), 0);
    for predecessor in [None, Some("missing-source-message".into())] {
        let mut out_of_order = admission("second", 5);
        out_of_order.source_predecessor_message_id = predecessor;
        assert!(receive(&mut runtime, out_of_order).unwrap_err().contains("sync_required"));
        assert_eq!(count(&path, "messages"), 1);
        assert_eq!(count(&path, "runs"), 0);
    }
    let mut second = admission("second", 5);
    second.source_predecessor_message_id = Some("first".into());
    receive(&mut runtime, second).unwrap();
    assert_eq!(count(&path, "messages"), 2);
}
#[test]
fn owning_marker_cannot_be_written_deleted_or_exported_by_renderer_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite");
    setup(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let key = source_key("parent");
    assert!(runtime
        .execute(ApplicationCommand::PutProductSettingChecked {
            key: key.clone(),
            value_json: "{}".into(),
            expected_revision: None,
            updated_at_ms: 1
        })
        .is_err());
    receive(&mut runtime, admission("first", 2)).unwrap();
    assert!(runtime
        .execute(ApplicationCommand::DeleteProductSetting { key })
        .is_err());
    assert!(serde_json::from_value::<ApplicationCommand>(json!({"type":"export_remote_expert_parent_admission","conversation_id":"parent","message_id":"first","source_host_id":"Host","source_store_epoch":"epoch"})).is_err());
}
#[test]
fn unowned_local_history_cannot_be_relabelled_by_an_external_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("local.sqlite");
    setup(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::AdmitConversationUserMessage {
            input: nola_protocol::AdmitConversationUserMessage {
                conversation_id: "parent".into(),
                message_id: "local-user".into(),
                text: "Local user".into(),
                created_at_ms: 2,
            },
        })
        .unwrap();
    assert!(receive(&mut runtime, admission("remote-user", 3))
        .unwrap_err()
        .contains("relabel"));
    assert_eq!(count(&path, "messages"), 1);
    assert_eq!(
        NolaStore::open(&path)
            .unwrap()
            .conversation_source_owner("parent")
            .unwrap(),
        ConversationSourceOwner::Local
    );
}

#[test]
fn transport_must_explicitly_report_nullable_predecessor() {
    let value = admission("first", 2);
    let mut wire = serde_json::to_value(&value).unwrap();
    assert!(wire.get("source_predecessor_message_id").unwrap().is_null());
    assert!(serde_json::from_value::<RemoteExpertParentAdmission>(wire.clone()).is_ok());
    wire.as_object_mut().unwrap().remove("source_predecessor_message_id");
    assert!(serde_json::from_value::<RemoteExpertParentAdmission>(wire).is_err());
}
