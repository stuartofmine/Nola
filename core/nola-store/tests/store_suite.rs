// SPDX-License-Identifier: Apache-2.0
//! Consolidated integration test binary for `nola-store`.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

pub mod support;

pub mod allocator_probe {
    use super::*;

    thread_local! {
        pub(super) static TRACKING: Cell<bool> = const { Cell::new(false) };
        static MAXIMUM_ALLOCATION: Cell<usize> = const { Cell::new(0) };
        pub(super) static LIVE_BYTES: Cell<usize> = const { Cell::new(0) };
        static PEAK_BYTES: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn record(bytes: usize, allocating: bool) {
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

    pub fn measure_allocations<T>(f: impl FnOnce() -> T) -> (T, usize, usize) {
        MAXIMUM_ALLOCATION.with(|value| value.set(0));
        LIVE_BYTES.with(|value| value.set(0));
        PEAK_BYTES.with(|value| value.set(0));
        TRACKING.with(|value| value.set(true));
        let output = f();
        TRACKING.with(|value| value.set(false));
        (
            output,
            MAXIMUM_ALLOCATION.with(Cell::get),
            PEAK_BYTES.with(Cell::get),
        )
    }
}

struct MeasuredAllocator;

unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocator_probe::record(layout.size(), true);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        allocator_probe::record(layout.size(), false);
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, old, size) };
        if !result.is_null() {
            allocator_probe::record(old.size(), false);
            allocator_probe::record(size, true);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

#[test]
#[ignore = "child process fixture"]
fn writer_process_fixture() {
    let db = std::env::var("NOLA_ARCHIVE_FIXTURE_DB").expect("fixture database");
    let mut store = nola_store::NolaStore::open(db).unwrap();
    store
        .create_conversation(&nola_store::CreateConversationInput {
            id: "async-fixture".into(),
            title: "archive".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .save_checkpoint(&nola_store::SaveCheckpointInput {
            conversation_id: "async-fixture".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 1,
            updated_at_ms: 1,
            checkpoint_json:
                serde_json::json!({"revision":1,"state":{"conversation":{"transcript":[]}}})
                    .to_string(),
        })
        .unwrap();
    std::process::exit(0);
}

#[path = "suites/append_messages_e2e.rs"]
pub mod append_messages_e2e;

#[path = "suites/automation_lifecycle_e2e.rs"]
pub mod automation_lifecycle_e2e;

#[path = "suites/bounded_semantic_allocation_e2e.rs"]
pub mod bounded_semantic_allocation_e2e;

#[path = "suites/catalog_and_store_invariants_e2e.rs"]
pub mod catalog_and_store_invariants_e2e;

#[path = "suites/conversation_lifecycle_e2e.rs"]
pub mod conversation_lifecycle_e2e;

#[path = "suites/conversation_metadata_sync_e2e.rs"]
pub mod conversation_metadata_sync_e2e;

#[path = "suites/delegated_invocation_ledger_e2e.rs"]
pub mod delegated_invocation_ledger_e2e;

#[path = "suites/delegated_permission_ledger_e2e.rs"]
pub mod delegated_permission_ledger_e2e;

#[path = "suites/effect_delivery_attempt_e2e.rs"]
pub mod effect_delivery_attempt_e2e;

#[path = "suites/ledger_e2e.rs"]
pub mod ledger_e2e;

#[path = "suites/memory_e2e.rs"]
pub mod memory_e2e;

#[path = "suites/model_attempt_ledger_e2e.rs"]
pub mod model_attempt_ledger_e2e;

#[path = "suites/orphaned_run_recovery_e2e.rs"]
pub mod orphaned_run_recovery_e2e;

#[path = "suites/permission_successor_e2e.rs"]
pub mod permission_successor_e2e;

#[path = "suites/remote_sync_apply_e2e.rs"]
pub mod remote_sync_apply_e2e;

#[path = "suites/runtime_task_e2e.rs"]
pub mod runtime_task_e2e;

#[path = "suites/structured_field_export_e2e.rs"]
pub mod structured_field_export_e2e;

#[path = "suites/turn_append_sync_e2e.rs"]
pub mod turn_append_sync_e2e;

#[path = "suites/uploaded_transcript_publication_e2e.rs"]
pub mod uploaded_transcript_publication_e2e;

#[path = "suites/work_archive_lifecycle_e2e.rs"]
pub mod work_archive_lifecycle_e2e;

