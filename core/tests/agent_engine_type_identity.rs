// SPDX-License-Identifier: Apache-2.0
//! Core ABI adapters must never grow a second reducer/checkpoint/trace model.
#[test]
fn core_adapters_use_the_exact_engine_owned_types() {
    fn owns_reducer_port<T: nola_agent_engine::AgentReducerPort>() {}
    owns_reducer_port::<nola_core::agent::AgentCore>();
    let _: fn(nola_agent_engine::agent::AgentCore) -> nola_core::agent::AgentCore = |value| value;
    let _: fn(nola_agent_engine::agent_trace::AgentTraceEvent)
        -> nola_core::agent_trace::AgentTraceEvent = |value| value;
    let _: fn(nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState>)
        -> nola_core::agent::recovery::CoreCheckpoint<nola_core::agent::AgentThreadState> = |value| value;
}
