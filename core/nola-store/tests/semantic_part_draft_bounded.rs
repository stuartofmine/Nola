// SPDX-License-Identifier: Apache-2.0
//! Allocation evidence for the dormant semantic token seam. Input is generated
//! lazily, so neither the fixture nor validator holds the ten-million-byte body.
use nola_store::{
    semantic_json_projection::{ExtractedValue, TokenKind},
    semantic_part_draft::{extract_draft, DraftStatus, HistoricalSchema},
};
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
fn actual_agent_and_view_tool_shapes_keep_ten_million_byte_inputs_deferred() {
    for schema in [
        HistoricalSchema::AgentPart,
        HistoricalSchema::MessagePartView,
    ] {
        let prefix = "{\"execution_source\":{\"kind\":\"model\"},\"input\":\"";
        let suffix = if schema == HistoricalSchema::AgentPart {
            "\",\"kind\":\"tool\",\"id\":\"p\",\"message_id\":\"m\",\"turn_id\":\"r\",\"call_id\":\"call\",\"capability_id\":\"read\",\"title\":\"Read\",\"state\":\"completed\",\"artifacts\":[]}"
        } else {
            "\",\"kind\":\"tool\",\"part_id\":\"p\",\"revision\":1,\"tool_call_id\":\"call\",\"capability_id\":\"read\",\"title\":\"Read\",\"origin\":{\"kind\":\"unknown\"},\"state\":\"completed\",\"artifacts\":[],\"affected_resources\":[]}"
        };
        let body_bytes = 10_000_000u64;
        let total = prefix.len() as u64 + body_bytes + suffix.len() as u64;
        MAXIMUM_ALLOCATION.with(|value| value.set(0));
        LIVE_BYTES.with(|value| value.set(0));
        PEAK_BYTES.with(|value| value.set(0));
        TRACKING.with(|value| value.set(true));
        let draft = extract_draft(
            || {
                let mut position = 0u64;
                std::iter::from_fn(move || {
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
                        } else if at >= prefix.len() as u64 + body_bytes {
                            suffix.as_bytes()[(at - prefix.len() as u64 - body_bytes) as usize]
                        } else {
                            b'x'
                        };
                        text.push(char::from(byte));
                    }
                    position += length as u64;
                    Some(Ok((offset, text)))
                })
            },
            total,
            "tool",
            8192,
        );
        TRACKING.with(|value| value.set(false));
        let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
        let peak = PEAK_BYTES.with(Cell::get);
        eprintln!("draft schema={schema:?} total={total} maximum={maximum} peak={peak}");
        let draft = draft.unwrap();
        assert_eq!(draft.status, DraftStatus::Recognized);
        assert_eq!(draft.schema, Some(schema));
        if schema == HistoricalSchema::MessagePartView {
            for (name, kind, raw) in [
                ("title", TokenKind::String, "\"Read\""),
                ("origin", TokenKind::Object, "{\"kind\":\"unknown\"}"),
                ("artifacts", TokenKind::Array, "[]"),
            ] {
                let field = draft.field(name).unwrap();
                assert_eq!(field.span.kind, kind);
                let suffix_offset = prefix.len() as u64 + body_bytes;
                let start = usize::try_from(field.span.start_bytes - suffix_offset).unwrap();
                let end = usize::try_from(field.span.end_bytes - suffix_offset).unwrap();
                assert_eq!(&suffix[start..end], raw);
            }
            for field in ["origin", "artifacts"] {
                assert!(draft.pending_nested_validation.contains(&field));
            }
        }
        let input = draft.field("input").unwrap();
        assert_eq!(input.value, ExtractedValue::Deferred);
        assert_eq!(
            input.span.end_bytes - input.span.start_bytes,
            body_bytes + 2
        );
        assert!(!draft.pending_nested_validation.is_empty());
        assert!(maximum <= 32768, "maximum={maximum}");
        assert!(peak <= 32768 + 3 * 8192, "peak={peak}");
    }
}
