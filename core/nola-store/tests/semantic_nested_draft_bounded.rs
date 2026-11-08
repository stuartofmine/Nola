// SPDX-License-Identifier: Apache-2.0
//! Allocation evidence for the dormant semantic token seam. Input is generated
//! lazily, so neither the fixture nor validator holds the ten-million-byte body.
use nola_store::semantic_nested_draft::{validate_nested, NestedKind, NestedStatus};
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
fn ten_million_byte_nested_fields_and_large_resource_arrays_are_explicitly_deferred() {
    for mode in 0..5 {
        let (kind, prefix, suffix, pattern, repeated_bytes): (NestedKind, &str, &str, &str, u64) =
            match mode {
                0 => (
                    NestedKind::Origin,
                    "{\"kind\":\"mcp\",\"provider_title\":\"",
                    "\"}",
                    "x",
                    10_000_000,
                ),
                1 => (
                    NestedKind::Permission,
                    "{\"request_id\":\"r\",\"permission_domain\":\"fs\",\"rationale\":\"",
                    "\"}",
                    "x",
                    10_000_000,
                ),
                2 => (
                    NestedKind::AffectedResources,
                    "[{\"resource_type\":\"file\",\"uri\":\"",
                    "\",\"operation\":\"read\"}]",
                    "x",
                    10_000_000,
                ),
                3 => (
                    NestedKind::Artifact,
                    "{\"uri\":\"file:///a\",\"name\":\"",
                    "\"}",
                    "x",
                    10_000_000,
                ),
                _ => {
                    let resource =
                        "{\"resource_type\":\"file\",\"uri\":\"a\",\"operation\":\"read\"},";
                    (
                        NestedKind::AffectedResources,
                        "[",
                        "]",
                        resource,
                        resource.len() as u64 * 200_000 - 1,
                    )
                }
            };
        let total = prefix.len() as u64 + repeated_bytes + suffix.len() as u64;
        MAXIMUM_ALLOCATION.with(|value| value.set(0));
        LIVE_BYTES.with(|value| value.set(0));
        PEAK_BYTES.with(|value| value.set(0));
        TRACKING.with(|value| value.set(true));
        let result = validate_nested(
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
                        } else if at >= prefix.len() as u64 + repeated_bytes {
                            suffix.as_bytes()[(at - prefix.len() as u64 - repeated_bytes) as usize]
                        } else {
                            pattern.as_bytes()
                                [((at - prefix.len() as u64) % pattern.len() as u64) as usize]
                        };
                        text.push(char::from(byte));
                    }
                    position += length as u64;
                    Some(Ok((offset, text)))
                })
            },
            total,
            kind,
            8192,
        );
        TRACKING.with(|value| value.set(false));
        let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
        let peak = PEAK_BYTES.with(Cell::get);
        eprintln!("nested mode={mode} total={total} maximum={maximum} peak={peak}");
        let draft = result.unwrap();
        assert_eq!(draft.status, NestedStatus::Deferred);
        assert!(draft.resources.is_empty());
        if mode == 4 {
            assert_eq!(draft.total_items, Some(200_000));
        }
        assert!(maximum <= 32768, "maximum={maximum}");
        assert!(peak <= 32768 + 3 * 8192, "peak={peak}");
    }
}

#[test]
fn evidence_location_range_and_nested_memory_collections_keep_ten_megabyte_sources_bounded() {
    for mode in 0..5 {
        let (kind,prefix,suffix,pattern,body):(NestedKind,&str,&str,&str,u64)=match mode {
            0=>(NestedKind::Evidence,"{\"kind\":\"memory\",\"usage\":{\"record_id\":\"r\",\"component_id\":\"c\",\"title\":\"","\"}}","x",10_000_000),
            1=>(NestedKind::Location,"{\"page\":1,\"section\":\"","\"}","x",10_000_000),
            2=>(NestedKind::Range,"{\"start\":0.",",\"end\":1}","0",10_000_000),
            3=>(NestedKind::Evidence,"{\"kind\":\"memory\",\"usage\":{\"record_id\":\"r\",\"component_id\":\"c\",\"namespace\":{\"kind\":\"custom\",\"name\":\"","\"}}}","x",10_000_000),
            _=>(NestedKind::Evidence,"{\"kind\":\"memory\",\"usage\":{\"record_id\":\"r\",\"component_id\":\"c\",\"supersedes\":[","]}}","\"old\",",10_000_001),
        };
        let total = prefix.len() as u64 + body + suffix.len() as u64;
        MAXIMUM_ALLOCATION.with(|value| value.set(0));
        LIVE_BYTES.with(|value| value.set(0));
        PEAK_BYTES.with(|value| value.set(0));
        TRACKING.with(|value| value.set(true));
        let result = validate_nested(
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
                        } else if at >= prefix.len() as u64 + body {
                            suffix.as_bytes()[(at - prefix.len() as u64 - body) as usize]
                        } else {
                            pattern.as_bytes()
                                [((at - prefix.len() as u64) % pattern.len() as u64) as usize]
                        };
                        text.push(char::from(byte));
                    }
                    position += length as u64;
                    Some(Ok((offset, text)))
                })
            },
            total,
            kind,
            8192,
        );
        TRACKING.with(|value| value.set(false));
        let maximum = MAXIMUM_ALLOCATION.with(Cell::get);
        let peak = PEAK_BYTES.with(Cell::get);
        eprintln!("evidence mode={mode} total={total} maximum={maximum} peak={peak}");
        assert_eq!(result.unwrap().status, NestedStatus::Deferred);
        assert!(maximum <= 32768, "maximum={maximum}");
        assert!(peak <= 32768 + 3 * 8192, "peak={peak}");
    }
}
