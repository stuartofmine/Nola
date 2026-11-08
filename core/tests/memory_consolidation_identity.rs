use nola_core::agent::memory_runtime::consolidation_input_digest;
use nola_core::memory::MemoryRecord;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct AllocationCounter;
thread_local! {
    static ALLOCATED: Cell<Option<usize>> = const { Cell::new(None) };
}
fn count(bytes: usize) {
    let _ = ALLOCATED.try_with(|total| {
        if let Some(value) = total.get() {
            total.set(Some(value + bytes));
        }
    });
}
unsafe impl GlobalAlloc for AllocationCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: AllocationCounter = AllocationCounter;

fn record(id: &str) -> MemoryRecord {
    serde_json::from_value(serde_json::json!({
        "id": id, "owner": {"kind":"profile","profile_id":"local"}, "namespace": {"kind": "personal"}, "scope": "global",
        "kind": "preference", "content": "Prefer concise answers",
        "summary": "Response style", "tags": [], "confidence": 0.9,
        "importance": 0.8, "sensitivity": null,
        "retention": {"mode": "until_superseded"}, "supersedes": [],
        "provenance": {"kind": "explicit_user", "source": "identity-test",
            "session_id": null, "turn_id": null, "item_ids": [], "observed_at": null}
    }))
    .unwrap()
}

#[test]
fn semantic_edits_change_identity_but_result_order_does_not() {
    let a = record("a");
    let b = record("b");
    let initial = consolidation_input_digest(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(
        initial,
        consolidation_input_digest(&[b.clone(), a.clone()]).unwrap()
    );
    let mut changed = a.clone();
    changed.content = "Prefer detailed explanations".into();
    assert_ne!(
        initial,
        consolidation_input_digest(&[changed, b.clone()]).unwrap()
    );
    let mut changed = a.clone();
    changed.summary = Some("Corrected summary".into());
    assert_ne!(
        initial,
        consolidation_input_digest(&[changed, b.clone()]).unwrap()
    );
    let mut changed = a;
    changed.provenance.source = "another-source".into();
    assert_ne!(initial, consolidation_input_digest(&[changed, b]).unwrap());
}

#[test]
fn missing_or_duplicate_inputs_are_rejected() {
    assert!(consolidation_input_digest(&[]).is_err());
    assert!(consolidation_input_digest(&[record("same"), record("same")]).is_err());
}

#[test]
fn ten_million_character_selection_only_copies_the_selected_records() {
    use nola_core::agent::memory_runtime::MemoryRuntimeProjection;
    let records = (0..1000)
        .map(|index| {
            let mut item = record(&format!("memory-{index:04}"));
            item.content = "x".repeat(10_000);
            item
        })
        .collect::<Vec<_>>();
    let runtime = MemoryRuntimeProjection::default();
    ALLOCATED.with(|total| total.set(Some(0)));
    let candidates = runtime.consolidation_candidates(&records);
    let allocated = ALLOCATED.with(|total| total.replace(None).unwrap());
    assert_eq!(candidates.len(), 40);
    assert_eq!(candidates[0].id, "memory-0000");
    assert_eq!(candidates[39].id, "memory-0039");
    assert_eq!(
        candidates
            .iter()
            .map(|record| record.content.len())
            .sum::<usize>(),
        400_000
    );
    eprintln!("10M memory selection allocated {allocated} bytes for 40 records");
    assert!(
        allocated < 1_000_000,
        "selection copied unselected content: {allocated}"
    );
}

#[test]
fn owner_changes_invalidate_the_frozen_semantic_input_digest() {
    let original = record("owner-bound");
    let digest = consolidation_input_digest(&[original.clone()]).unwrap();
    let mut changed = original;
    changed.owner = nola_domain::memory_owner::MemoryOwner::Profile { profile_id: "foreign".into() };
    assert_ne!(digest, consolidation_input_digest(&[changed]).unwrap());
}
