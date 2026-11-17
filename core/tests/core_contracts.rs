// SPDX-License-Identifier: Apache-2.0
//! Consolidated integration contract binary for `nola_core`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub mod support;

struct AllocationCounter;

thread_local! {
    static ALLOCATED: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count_alloc(bytes: usize) {
    let _ = ALLOCATED.try_with(|total| {
        if let Some(value) = total.get() {
            total.set(Some(value + bytes));
        }
    });
}

unsafe impl GlobalAlloc for AllocationCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_alloc(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_alloc(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_alloc(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: AllocationCounter = AllocationCounter;

pub fn measure_allocated_bytes<T>(f: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATED.with(|total| total.set(Some(0)));
    let out = f();
    let bytes = ALLOCATED.with(|total| total.replace(None).unwrap());
    (out, bytes)
}

// --- Preserved exact benchmark & CI filter modules ---
#[path = "suites/agent_prompt_sop_contract.rs"]
pub mod agent_prompt_sop_contract;

#[path = "suites/agent_goal_runtime_checkpoint_contract.rs"]
pub mod agent_goal_runtime_checkpoint_contract;

#[path = "suites/agent_task_handle_contract.rs"]
pub mod agent_task_handle_contract;

#[path = "suites/capability_skill_contract.rs"]
pub mod capability_skill_contract;

#[path = "suites/agent_trace_contract.rs"]
pub mod agent_trace_contract;

#[path = "suites/agent_core_host_contract/mod.rs"]
pub mod agent_core_host_contract;

// --- Consolidated multi-scenario contract modules ---
#[path = "suites/abi_instruction_and_boundary_contracts.rs"]
pub mod abi_instruction_and_boundary_contracts;

#[path = "suites/compaction_and_context_contracts.rs"]
pub mod compaction_and_context_contracts;

#[path = "suites/memory_contracts.rs"]
pub mod memory_contracts;

#[path = "suites/permission_and_trust_contracts.rs"]
pub mod permission_and_trust_contracts;

#[path = "suites/agent_durability_memory_contract.rs"]
pub mod agent_durability_memory_contract;

#[path = "suites/agent_input_origin_contract.rs"]
pub mod agent_input_origin_contract;

#[path = "suites/agent_task_terminal_receipt_contract.rs"]
pub mod agent_task_terminal_receipt_contract;

#[path = "suites/agent_turn_context_snapshot_contract.rs"]
pub mod agent_turn_context_snapshot_contract;

#[path = "suites/canonical_part_lifecycle_fixture.rs"]
pub mod canonical_part_lifecycle_fixture;

#[path = "suites/model_capability_contract.rs"]
pub mod model_capability_contract;

#[path = "suites/provider_continuation_contract.rs"]
pub mod provider_continuation_contract;

#[path = "suites/skill_goal_lifecycle_contract.rs"]
pub mod skill_goal_lifecycle_contract;
