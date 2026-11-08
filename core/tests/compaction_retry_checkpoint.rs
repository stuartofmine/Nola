use nola_core::agent::context_window::{compaction_source_fingerprint, PendingCompactionPlan};
use nola_core::agent::{AgentItem, AgentRole, ContextWindowConfig, ContextWindowState, ItemId};
use sha2::{Digest, Sha256};

fn archive(content: &str) -> Vec<AgentItem> {
    vec![AgentItem::Message {
        id: ItemId::from_string("source".to_string()),
        role: AgentRole::User,
        content: content.to_string(),
    }]
}

fn plan(content: &str) -> PendingCompactionPlan {
    PendingCompactionPlan {
        reason: nola_core::agent::CompactionReason::Manual,
        working_set_item_count: archive(content).len(),
        working_set_fingerprint: compaction_source_fingerprint(&archive(content)).unwrap(),
        source_fingerprint: compaction_source_fingerprint(&archive(content)).unwrap(),
        source_item_ids: vec![ItemId::from_string("source".to_string())],
        original_token_count: archive(content)[0].estimated_tokens(),
        replaced_item_count: 1,
        summary_token_limit: 100,
    }
}

fn reopen(state: &ContextWindowState) -> ContextWindowState {
    serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap()
}

#[test]
fn retry_guard_survives_cold_restore_and_tracks_the_actual_archive() {
    let mut state = ContextWindowState::new(ContextWindowConfig::default());
    for expected in 1..=3 {
        state.begin_compaction(plan("same archive")).unwrap();
        state = reopen(&state);
        state.fail_compaction();
        state = reopen(&state);
        assert_eq!(state.compaction_retry.failed_attempts, expected);
    }
    assert!(state.begin_compaction(plan("same archive")).is_err());
    assert!(state.pending_compaction.is_none());
    state.begin_compaction(plan("changed archive")).unwrap();
    assert_eq!(state.compaction_retry.failed_attempts, 0);
    state.fail_compaction();
    assert_eq!(state.compaction_retry.failed_attempts, 1);
}

#[test]
fn repeated_failure_without_a_pending_attempt_does_not_increase_the_guard() {
    let mut state = ContextWindowState::new(ContextWindowConfig::default());
    state.begin_compaction(plan("source")).unwrap();
    state.fail_compaction();
    state = reopen(&state);
    state.fail_compaction();
    assert_eq!(state.compaction_retry.failed_attempts, 1);
    assert_eq!(state.consecutive_compaction_failures, 1);
    assert_eq!(state.compaction_generation, 0);
}

#[test]
fn a_second_start_cannot_replace_the_pending_archive() {
    let mut state = ContextWindowState::new(ContextWindowConfig::default());
    let original = plan("original");
    state.begin_compaction(original.clone()).unwrap();
    assert!(state.begin_compaction(plan("replacement")).is_err());
    assert_eq!(state.pending_compaction.as_ref(), Some(&original));
}

#[test]
fn streaming_fingerprint_matches_canonical_serialization_for_large_unicode_content() {
    let items = archive(&"长期记忆/工具结果\n".repeat(100_000));
    let expected = format!("{:x}", Sha256::digest(serde_json::to_vec(&items).unwrap()));
    assert_eq!(compaction_source_fingerprint(&items).unwrap(), expected);
    assert_ne!(
        compaction_source_fingerprint(&items).unwrap(),
        compaction_source_fingerprint(&archive("different")).unwrap()
    );
}

#[test]
fn restored_compaction_plan_replaces_only_its_archive_and_preserves_new_inputs() {
    let original = archive(&"canonical source ".repeat(100));
    let plan = plan(&"canonical source ".repeat(100));
    let restored: PendingCompactionPlan =
        serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    let newest = AgentItem::user_message(ItemId::from_string("new-user".into()),
        "Do not change the database; inspect only.");
    let mut current = original.clone();
    current.push(newest.clone());
    assert_eq!(restored.retained_items(&current).unwrap(), vec![&newest]);
    assert!(restored.validate_summary_for_prompt("Source reviewed.", 100, &current).is_ok());
    // The same original plan must budget the newly admitted tail too.
    current.push(AgentItem::user_message(ItemId::from_string("large-input".into()), "x".repeat(1000)));
    assert!(restored.validate_summary_for_prompt("Source reviewed.", 100, &current).is_err());
    // A rewritten prefix cannot be merged by matching IDs alone.
    current[0] = archive("rewritten source").remove(0);
    let before = current.clone();
    assert!(restored.retained_items(&current).unwrap_err().contains("prefix changed"));
    assert_eq!(current, before);
    assert_eq!(restored, plan);
}
