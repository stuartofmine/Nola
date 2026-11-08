// SPDX-License-Identifier: Apache-2.0
//! Actual large-JSON estimator allocation probe; input creation and oracle
//! serialization are outside the measured interval.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct MeasuredAllocator;
thread_local! {
    static MEASURE: Cell<Option<(usize, usize, usize)>> = const { Cell::new(None) };
}
fn record(bytes: usize) {
    let _ = MEASURE.try_with(|measure| {
        if let Some((calls, total, largest)) = measure.get() {
            measure.set(Some((
                calls + 1,
                total.saturating_add(bytes),
                largest.max(bytes),
            )));
        }
    });
}
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

#[test]
fn ten_megabyte_nested_json_estimates_without_body_sized_or_accumulated_allocations() {
    let unit = "\\🦀\0\n\"é";
    let body = unit.repeat(10 * 1024 * 1024 / unit.len() + 1);
    let input_bytes = body.len();
    let value = serde_json::json!({"nested":[{"正文":body}],"numbers":[-3,1.25,null,true]});
    MEASURE.with(|measure| measure.set(Some((0, 0, 0))));
    let tokens = nola_kernel::item_token_estimate::estimated_item_tokens(
        nola_domain::item_token_estimate::ItemTokenEstimateFacts::NamedJson {
            name: "工具",
            value: &value,
        },
    );
    let (calls, total, largest) = MEASURE.with(|measure| measure.replace(None).unwrap());
    let json_characters = value.to_string().chars().count();
    assert_eq!(tokens, (2 + json_characters + 3) / 4 + 4);
    assert_eq!(
        calls, 0,
        "estimator must not allocate while streaming JSON to the counter"
    );
    assert_eq!(total, 0);
    assert_eq!(largest, 0);
    println!("input_utf8_bytes={input_bytes} json_characters={json_characters} estimated_tokens={tokens} allocations={calls} total_allocated_bytes={total} largest_allocation={largest}");
}
