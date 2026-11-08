// SPDX-License-Identifier: Apache-2.0
//! Allocation evidence for the dormant JSON preparation seam. Input is generated
//! lazily, so neither the fixture nor validator holds the ten-million-byte body.
use nola_store::structured_json_validation::validate_json_chunks;
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
fn ten_million_byte_strings_keys_and_numbers_have_constant_live_buffers() {
    // Both huge values and huge object keys normally populate serde's String
    // scratch. Escapes exercise the bounded token validator repeatedly.
    for mode in 0..4 {
        let object_key = mode == 1;
        let escaped = mode == 2;
        let total = if escaped {
            10_000_010_u64
        } else {
            10_000_000_u64
        };
        let mut position = 0u64;
        MAXIMUM_ALLOCATION.with(|value| value.set(0));
        LIVE_BYTES.with(|value| value.set(0));
        PEAK_BYTES.with(|value| value.set(0));
        TRACKING.with(|value| value.set(true));
        let source = std::iter::from_fn(move || {
            if position == total {
                return None;
            }
            let offset = position;
            let length = (total - position).min(32768) as usize;
            let mut chunk = String::with_capacity(length);
            for index in 0..length {
                let at = offset + index as u64;
                let byte = if mode == 3 {
                    if at == 1 {
                        b'.'
                    } else {
                        b'0'
                    }
                } else if object_key {
                    match at {
                        0 => b'{',
                        1 => b'"',
                        v if v == total - 4 => b'"',
                        v if v == total - 3 => b':',
                        v if v == total - 2 => b'0',
                        v if v == total - 1 => b'}',
                        _ => b'x',
                    }
                } else {
                    match at {
                        0 => b'"',
                        v if v == total - 1 => b'"',
                        _ if escaped => b"\\uD83D\\uDE00"[((at - 1) % 12) as usize],
                        _ => b'x',
                    }
                };
                chunk.push(char::from(byte));
            }
            position += length as u64;
            Some(Ok((offset, chunk)))
        });
        let result = validate_json_chunks(source, total);
        TRACKING.with(|value| value.set(false));
        let largest = MAXIMUM_ALLOCATION.with(Cell::get);
        let peak = PEAK_BYTES.with(Cell::get);
        eprintln!("JSON validator mode={mode} total={total} largest_allocation={largest} peak_live_bytes={peak}");
        let stats = result.unwrap();
        assert_eq!(stats.total_utf8_bytes, total);
        assert_eq!(stats.maximum_chunk_bytes, 32768);
        assert_eq!(stats.chunk_count, total.div_ceil(32768));
        assert!(largest <= 32768, "largest allocation={largest}");
        assert!(peak <= 32768 + 4096, "peak live allocated bytes={peak}");
    }
}
