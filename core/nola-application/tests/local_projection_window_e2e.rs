// SPDX-License-Identifier: Apache-2.0
// Persistence/query integration: synthetic Core state, no model or Host execution.

#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{load_conversation_projection_v1, load_conversation_projection_window_v1};
use nola_agent_engine::agent::context_window::ContextWindowConfig;
use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{EventCursor};
use nola_agent_engine::agent::{AgentSessionConfiguration, AgentThreadState, SessionId};
use nola_domain::{AssistantTurnBinding, PartMutation, TransitionMessageInput};
use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore};
use serde_json::json;

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static MAXIMUM_ALLOCATION: Cell<usize> = const { Cell::new(0) };
    static LIVE_BYTES: Cell<usize> = const { Cell::new(0) };
    static PEAK_BYTES: Cell<usize> = const { Cell::new(0) };
}
struct MeasuredAllocator;
// Measure Rust allocations only during the selected read on this test thread.
// Checkpoint construction and SQLite's C allocator are outside this evidence.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), true);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(layout.size(), false);
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, old, size) };
        if !result.is_null() {
            record(old.size(), false);
            record(size, true);
        }
        result
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;
fn record(bytes: usize, allocating: bool) {
    if !TRACKING.try_with(Cell::get).unwrap_or(false) {
        return;
    }
    LIVE_BYTES.with(|live| {
        let next = if allocating {
            live.get().saturating_add(bytes)
        } else {
            live.get().saturating_sub(bytes)
        };
        live.set(next);
        if allocating {
            MAXIMUM_ALLOCATION.with(|maximum| maximum.set(maximum.get().max(bytes)));
            PEAK_BYTES.with(|peak| peak.set(peak.get().max(next)));
        }
    });
}

fn begin(store: &mut NolaStore, conversation: &str, message: &str, part: &str) {
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            conversation_id: conversation.into(),
            conversation_title: "Canonical window fixture".into(),
            user_message_id: "fixture-user".into(),
            user_part_id: "fixture-user-part".into(),
            user_text: "Fixture input".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: message.into(),
            assistant_part_id: part.into(),
            assistant_turn_bindings: vec![
                AssistantTurnBinding::Admission {
                    turn_id: "shared-run".into(),
                    assistant_message_id: message.into(),
                },
                AssistantTurnBinding::Part {
                    turn_id: "shared-run".into(),
                    assistant_message_id: message.into(),
                    part_id: part.into(),
                },
            ],
            initial_assistant_part_mutations: vec![],
            run_id: "shared-run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 0,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        })
        .unwrap();
}
fn transition(checkpoint: String) -> CommitTransitionInput {
    CommitTransitionInput {
        transition_id: "fixture-materialization".into(),
        run_id: "shared-run".into(),
        expected_checkpoint_revision: 0,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 1,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: checkpoint },
        submission_reduction_json: None, host_event: None,
        events: vec![],
        ensured_messages: vec![],
        part_message_owners: Default::default(),
        assistant_turn_bindings: vec![],
        part_mutations: vec![],
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        effect_delivery_attempt_mutations: vec![],
        acknowledged_effect_ids: vec![],
        memory_usage_entries: vec![],
        run_status: None,
        assistant_status: Some("completed".into()),
        assistant_status_message_id: None,
        committed_at_ms: 3,
    }
}
fn bind_part(input: &mut CommitTransitionInput, message: &str, part: &str, ordinal: i64) {
    input
        .part_message_owners
        .insert(part.into(), message.into());
    input
        .assistant_turn_bindings
        .push(AssistantTurnBinding::Part {
            turn_id: "shared-run".into(),
            assistant_message_id: message.into(),
            part_id: part.into(),
        });
    input.part_mutations.push(PartMutation::Upsert {
        part_id: part.into(),
        kind: "text".into(),
        ordinal,
        state: "completed".into(),
        content_text: String::new(),
        content_json: None,
        content_digest: None,
        created_at_ms: 2,
        updated_at_ms: 3,
    });
}

#[test]
fn local_window_preserves_part_ownership_across_messages_in_one_run() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("local-window.db");
    let mut state = AgentThreadState::new(
        SessionId::from_string("local-window".into()),
        AgentSessionConfiguration {
            model: None,
            max_model_iterations: 0,
            max_strategy_recovery_attempts: 3,
            tools: vec![],
            context_window: ContextWindowConfig {
                context_window_tokens: 65_536,
                reserved_output_tokens: 8_192,
                ..ContextWindowConfig::default()
            },
            memory_policy: Default::default(),
            turn_identity: fixture_model_identity::identity(),
        },
    );
    state.conversation.context_window.used_tokens = 123;
    state.conversation.projection = serde_json::from_value(json!({
        "revision": 1,
        "active_message_segments": {"shared-run":"new"},
        "activity_message_owners": {},
        "order": ["old-part", "fallback-part", "new-part", "bound-part"],
        "parts": {
            "old-part": {"kind":"text", "id":"old-part", "message_id":"old",
                "turn_id":"shared-run", "text":"x".repeat(10_000_000), "is_complete":true},
            "fallback-part": {"kind":"text", "id":"fallback-part", "message_id":"shared-run:assistant",
                "turn_id":"shared-run", "text":"First message fallback", "is_complete":true},
            "new-part": {"kind":"text", "id":"new-part", "message_id":"new",
                "turn_id":"shared-run", "text":"Latest message", "is_complete":true},
            "bound-part": {"kind":"text", "id":"bound-part", "message_id":"shared-run:assistant",
                "turn_id":"shared-run", "text":"Explicit persisted Part binding", "is_complete":true}
        }
    })).unwrap();
    let checkpoint = CoreCheckpoint {
        schema_version: nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
        session_id: "local-window".into(),
        revision: 1,
        last_event_cursor: EventCursor { sequence: 1 },
        capability_catalog_revision: 0,
        created_at_ms: 3,
        reason: CheckpointReason::Manual,
        state,
    };
    let original_checkpoint = serde_json::to_string(&checkpoint).unwrap();
    {
        let mut store = NolaStore::open(&database).unwrap();
        begin(&mut store, "local-window", "old", "old-part");
        let mut input = transition(original_checkpoint.clone());
        input.ensured_messages.push(TransitionMessageInput {
            id: "new".into(),
            run_id: Some("shared-run".into()),
            role: "assistant".into(),
            status: "completed".into(),
            created_at_ms: 2,
            completed_at_ms: Some(3),
            initial_parts: vec![],
        });
        bind_part(&mut input, "old", "old-part", 1);
        bind_part(&mut input, "new", "new-part", 1);
        bind_part(&mut input, "new", "bound-part", 2);
        store.commit_transition(&input).unwrap();
    }
    drop(checkpoint);
    let store = NolaStore::open(&database).unwrap();
    let inputs = store
        .load_local_presentation_inputs("local-window", r#"["new"]"#, None)
        .unwrap()
        .unwrap();
    assert!(
        inputs.len() < 4096,
        "The prior ten-million-character Part must stay out of the read result."
    );
    let latest =
        load_conversation_projection_window_v1(&store, "local-window", None, 1, 4096).unwrap();
    assert_eq!(latest.total_message_count, 3); // Real admission also owns its visible user input.
    assert!(latest.has_more);
    assert_eq!(latest.projection.messages[0].message_id.0, "new");
    assert_eq!(latest.projection.messages[0].parts.len(), 2);
    // This metadata/ownership fixture has no committed ContextUpdated fact.
    // A checkpoint estimate is not a source usage publication.
    assert_eq!(latest.projection.context_usage.used_tokens, 0);
    assert!(latest.projection.context_usage.estimated);
    let parts = &latest.projection.messages[0].parts;
    assert!(
        matches!(&parts[0], nola_domain::MessagePartView::Text { part_id, markdown, .. }
        if part_id.0 == "new-part" && markdown == "Latest message")
    );
    assert!(
        matches!(&parts[1], nola_domain::MessagePartView::Text { part_id, markdown, .. }
        if part_id.0 == "bound-part" && markdown == "Explicit persisted Part binding")
    );
    // This old checkpoint-only body has no canonical original to bind a
    // Deferred locator to. Reject before returning its giant aggregate to Rust.
    MAXIMUM_ALLOCATION.with(|v| v.set(0));
    PEAK_BYTES.with(|v| v.set(0));
    LIVE_BYTES.with(|v| v.set(0));
    TRACKING.with(|v| v.set(true));
    let full = load_conversation_projection_v1(&store, "local-window");
    let older = load_conversation_projection_window_v1(
        &store,
        "local-window",
        latest.before_ordinal,
        1,
        4096,
    );
    TRACKING.with(|v| v.set(false));
    let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
    let peak = PEAK_BYTES.with(Cell::get);
    assert!(full
        .unwrap_err()
        .to_string()
        .contains("bounded read budget"));
    assert!(older
        .unwrap_err()
        .to_string()
        .contains("bounded read budget"));
    assert!(maximum <= 32768, "maximum Rust allocation={maximum}");
    assert!(peak <= 65536, "peak Rust allocations={peak}");
    eprintln!("checkpoint-only 10M rejection: Rust maximum={maximum}, peak={peak}");
    assert_eq!(
        store
            .load_checkpoint("local-window")
            .unwrap()
            .unwrap()
            .checkpoint_json,
        original_checkpoint
    );
    let canonical = store.load_conversation("local-window").unwrap().unwrap();
    let old = canonical
        .messages
        .iter()
        .find(|message| message.id == "old")
        .unwrap();
    assert_eq!(old.parts[0].content_text, "");
    assert_eq!(old.parts[0].content_json, None);
}

#[test]
fn local_presentation_admission_counts_escaped_keys_values_and_metadata() {
    for (case, id, text, session_id, admitted) in [
        (
            "bounded",
            "escaped\"\\\u{0001}界".to_string(),
            "\u{0001}\"\\界".repeat(20_000),
            "escape".to_string(),
            true,
        ),
        (
            "key",
            "\u{0001}".repeat(200_000),
            "body".to_string(),
            "escape".to_string(),
            false,
        ),
        (
            "value",
            "part".to_string(),
            "\u{0001}".repeat(400_000),
            "escape".to_string(),
            false,
        ),
        (
            "metadata",
            "part".to_string(),
            "body".to_string(),
            "\u{0001}".repeat(400_000),
            false,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("escape.db")).unwrap();
        begin(&mut store, "escape", "message", &id);
        // The Store accepts JSON checkpoints; this test isolates its selected
        // output admission, including metadata that later typed parsing rejects.
        let original = json!({"session_id": session_id, "state": {
            "session_id": "escape", "conversation": {"projection": {
                "revision": 1, "order": [&id], "parts": {
                    (&id): {"kind": "text", "id": &id, "message_id": "message",
                        "turn_id": "shared-run", "text": &text, "is_complete": true}
                }
            }}
        }})
        .to_string();
        let mut input = transition(original.clone());
        bind_part(&mut input, "message", &id, 1);
        store.commit_transition(&input).unwrap();
        MAXIMUM_ALLOCATION.with(|v| v.set(0));
        LIVE_BYTES.with(|v| v.set(0));
        PEAK_BYTES.with(|v| v.set(0));
        TRACKING.with(|v| v.set(true));
        let result = store.load_local_presentation_inputs("escape", r#"["message"]"#, None);
        TRACKING.with(|v| v.set(false));
        if admitted {
            let output = result.unwrap().unwrap();
            assert!(output.len() <= 2 * 1024 * 1024);
            let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert_eq!(decoded["projection"]["order"][0], id);
            assert_eq!(decoded["projection"]["parts"][&id]["text"], text);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("bounded read budget"),
                "{case}"
            );
            let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
            assert!(
                maximum <= 32768,
                "{case}: maximum Rust allocation={maximum}"
            );
            eprintln!("escaped {case} rejected before Rust aggregate: maximum={maximum}");
        }
        assert_eq!(
            store
                .load_checkpoint("escape")
                .unwrap()
                .unwrap()
                .checkpoint_json,
            original
        );
    }
}
