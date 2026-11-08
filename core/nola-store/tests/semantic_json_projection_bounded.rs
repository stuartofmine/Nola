// SPDX-License-Identifier: Apache-2.0
//! Allocation evidence for the dormant semantic token seam. Input is generated
//! lazily, so neither the fixture nor validator holds the ten-million-byte body.
use nola_store::semantic_json_projection::{extract_top_level, ExtractedValue, ScalarValue};
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
fn ten_million_byte_keys_strings_arrays_and_escapes_keep_buffers_bounded() {
    for mode in 0..4 {
        let (prefix, suffix, body_bytes): (&str, &str, u64) = match mode {
            0 => ("{\"huge\":\"", "\",\"kind\":\"tool\"}", 10_000_000),
            1 => ("{\"", "\":null,\"kind\":\"tool\"}", 10_000_000),
            2 => ("{\"huge\":[", "],\"kind\":\"tool\"}", 10_000_001),
            _ => ("{\"huge\":\"", "\",\"kind\":\"tool\"}", 10_000_008),
        };
        let total = prefix.len() as u64 + body_bytes + suffix.len() as u64;
        MAXIMUM_ALLOCATION.with(|value| value.set(0));
        LIVE_BYTES.with(|value| value.set(0));
        PEAK_BYTES.with(|value| value.set(0));
        TRACKING.with(|value| value.set(true));
        let result = extract_top_level(
            || {
                let mut position = 0u64;
                std::iter::from_fn(move || {
                    if position == total {
                        return None;
                    }
                    let offset = position;
                    let length = (total - position).min(32768) as usize;
                    let mut text = String::with_capacity(length);
                    for index in 0..length {
                        let at = offset + index as u64;
                        let byte = if at < prefix.len() as u64 {
                            prefix.as_bytes()[at as usize]
                        } else if at >= prefix.len() as u64 + body_bytes {
                            suffix.as_bytes()[(at - prefix.len() as u64 - body_bytes) as usize]
                        } else {
                            let body = at - prefix.len() as u64;
                            match mode {
                                2 => {
                                    if body % 2 == 0 {
                                        b'0'
                                    } else {
                                        b','
                                    }
                                }
                                3 => b"\\uD83D\\uDE00"[(body % 12) as usize],
                                _ => b'x',
                            }
                        };
                        text.push(char::from(byte));
                    }
                    position += length as u64;
                    Some(Ok((offset, text)))
                })
            },
            total,
            &["huge", "kind"],
            4096,
        );
        TRACKING.with(|value| value.set(false));
        let largest = MAXIMUM_ALLOCATION.with(Cell::get);
        let peak = PEAK_BYTES.with(Cell::get);
        eprintln!("semantic mode={mode} total={total} largest={largest} peak={peak}");
        let result = result.unwrap();
        let kind = result
            .fields
            .iter()
            .find(|entry| entry.field_index == 1)
            .unwrap();
        assert_eq!(
            kind.value,
            ExtractedValue::Complete(ScalarValue::String("tool".into()))
        );
        if mode != 1 {
            let huge = result
                .fields
                .iter()
                .find(|entry| entry.field_index == 0)
                .unwrap();
            assert_eq!(huge.value, ExtractedValue::Deferred);
            assert_eq!(huge.span.end_bytes - huge.span.start_bytes, body_bytes + 2);
        } else {
            assert_eq!(result.fields.len(), 1);
        }
        assert!(largest <= 32768, "largest allocation={largest}");
        assert!(peak <= 32768 + 3 * 4096, "peak live bytes={peak}");
    }
}
