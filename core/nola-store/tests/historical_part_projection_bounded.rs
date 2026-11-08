// SPDX-License-Identifier: Apache-2.0
//! Allocation evidence for the dormant semantic token seam. Input is generated
//! lazily, so neither the fixture nor validator holds the ten-million-byte body.
use nola_domain::{
    CanonicalBodyReference, CanonicalPartOwner, HistoricalProjectionStatus, PartBodyField,
    SemanticValue,
};
use nola_store::NolaStore;
use nola_store_port::canonical_body::CanonicalBodyRepository;
use rusqlite::{params, Connection};
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
// Only the test's validation thread is measured. The source and parser are
// created/dropped inside that scope; the test harness is outside it.
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

#[test]
fn ten_megabyte_historical_window_and_projection_do_not_materialize_original_json() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.db");
    let mut store = NolaStore::open(&path).unwrap();
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);INSERT INTO message_parts VALUES('p','m','text',1,'completed','',NULL,NULL,1,1);").unwrap();
    let prefix = "{\"kind\":\"text\",\"part_id\":\"p\",\"revision\":1,\"markdown\":\"";
    let suffix = "\",\"is_final\":true}";
    let body = 10_000_000u64;
    let total = prefix.len() as u64 + body + suffix.len() as u64;
    let reference = CanonicalBodyReference::for_field(
        "large-json".into(),
        "c".into(),
        "m".into(),
        "p".into(),
        PartBodyField::ContentJson,
        total,
        total.div_ceil(32768) as u32,
    )
    .unwrap();
    let mut position = 0;
    let mut input = std::iter::from_fn(|| {
        if position == total {
            return None;
        }
        let offset = position;
        let length = (total - position).min(32768) as usize;
        let mut text = String::with_capacity(length);
        for i in 0..length {
            let at = offset + i as u64;
            let byte = if at < prefix.len() as u64 {
                prefix.as_bytes()[at as usize]
            } else if at >= prefix.len() as u64 + body {
                suffix.as_bytes()[(at - prefix.len() as u64 - body) as usize]
            } else {
                b'x'
            };
            text.push(char::from(byte));
        }
        position += length as u64;
        Some(Ok((offset, text)))
    });
    store
        .prepare_body(&reference, &mut input, 20_000_000)
        .unwrap();
    drop(input);
    sql.execute(
        "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
        params![serde_json::to_string(&reference).unwrap()],
    )
    .unwrap();
    let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
    // The SQL fixture above stages a new backing. Publish its disclosure fact
    // through the production write boundary before measuring read allocations.
    // Reader behavior must not silently repair an unpublished generation.
    assert!(store.read_historical_part(&owner, 1, 8192).is_err());
    store.begin_write_unit_of_work().unwrap();
    store.commit_write_unit_of_work().unwrap();
    MAXIMUM_ALLOCATION.with(|value| value.set(0));
    LIVE_BYTES.with(|value| value.set(0));
    PEAK_BYTES.with(|value| value.set(0));
    TRACKING.with(|value| value.set(true));
    let window = store.load_conversation_message_window("c", None, 10, 4096);
    let projection = store.read_historical_part(&owner, 1, 8192);
    TRACKING.with(|value| value.set(false));
    let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
    let peak = PEAK_BYTES.with(Cell::get);
    eprintln!("history total={total} maximum={maximum} peak={peak}");
    let window = window.unwrap().unwrap();
    assert_eq!(window.conversation.messages.len(), 1);
    assert!(window.conversation.messages[0].parts[0]
        .content_json
        .is_none());
    let projection = projection.unwrap().unwrap();
    assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
    assert!(matches!(
        projection.fields["markdown"],
        SemanticValue::Deferred { .. }
    ));
    assert!(serde_json::to_vec(&projection).unwrap().len() <= 8192);
    assert!(maximum <= 32768, "maximum allocation={maximum}");
    assert!(peak <= 256 * 1024, "peak live bytes={peak}");
}
