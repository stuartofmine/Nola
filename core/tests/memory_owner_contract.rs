use nola_core::memory::{MemoryNamespace, MemoryScope};
use nola_domain::memory_owner::{MemoryExecutionBinding, MemoryOwner};

fn binding() -> MemoryExecutionBinding {
    MemoryExecutionBinding {
        profile_id: "profile-a".into(),
        workspace_id: Some("workspace-a".into()),
        conversation_id: Some("conversation-a".into()),
        turn_id: Some("turn-a".into()),
    }
}

#[test]
fn owners_isolate_profiles_workspaces_conversations_and_turns() {
    let original = binding();
    let owners = [
        original
            .owner_for(&MemoryNamespace::Personal, MemoryScope::Global)
            .unwrap(),
        original
            .owner_for(&MemoryNamespace::Workspace, MemoryScope::Global)
            .unwrap(),
        original
            .owner_for(&MemoryNamespace::Conversation, MemoryScope::Conversation)
            .unwrap(),
        original
            .owner_for(&MemoryNamespace::Conversation, MemoryScope::Turn)
            .unwrap(),
    ];
    let mut other_profile = original.clone();
    other_profile.profile_id = "profile-b".into();
    let mut other_workspace = original.clone();
    other_workspace.workspace_id = Some("workspace-b".into());
    let mut other_conversation = original.clone();
    other_conversation.conversation_id = Some("conversation-b".into());
    let mut other_turn = original.clone();
    other_turn.turn_id = Some("turn-b".into());
    for (context, expected) in [
        (original, [true, true, true, true]),
        (other_profile, [false, false, false, false]),
        (other_workspace, [true, false, true, true]),
        (other_conversation, [true, true, false, false]),
        (other_turn, [true, true, true, false]),
    ] {
        assert_eq!(
            owners.each_ref().map(|owner| owner.is_visible_to(&context)),
            expected
        );
    }
}

#[test]
fn missing_local_identity_never_falls_back_to_global() {
    let empty = MemoryExecutionBinding {
        profile_id: "profile-a".into(),
        workspace_id: None,
        conversation_id: None,
        turn_id: None,
    };
    for (namespace, scope) in [
        (MemoryNamespace::Workspace, MemoryScope::Global),
        (MemoryNamespace::Conversation, MemoryScope::Conversation),
        (MemoryNamespace::Conversation, MemoryScope::Turn),
        (MemoryNamespace::Conversation, MemoryScope::Global),
    ] {
        assert!(empty.owner_for(&namespace, scope).is_err());
    }
    let orphan_turn = MemoryExecutionBinding {
        turn_id: Some("turn".into()),
        ..empty
    };
    assert!(orphan_turn.validate().is_err());
}

#[test]
fn ownership_round_trip_is_strict_and_does_not_accept_provenance_as_identity() {
    let owner = binding()
        .owner_for(&MemoryNamespace::Conversation, MemoryScope::Turn)
        .unwrap();
    let encoded = serde_json::to_value(&owner).unwrap();
    assert_eq!(
        serde_json::from_value::<MemoryOwner>(encoded.clone()).unwrap(),
        owner
    );
    let mut extra = encoded;
    extra["source"] = "conversation-b".into();
    assert!(serde_json::from_value::<MemoryOwner>(extra).is_err());
    assert!(serde_json::from_str::<MemoryOwner>(
        r#"{"kind":"turn","profile_id":"a","turn_id":"b"}"#
    )
    .is_err());
    assert!(!MemoryOwner::Profile {
        profile_id: " ".into()
    }
    .is_visible_to(&binding()));
}
