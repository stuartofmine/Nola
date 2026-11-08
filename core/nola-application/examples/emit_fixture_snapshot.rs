// SPDX-License-Identifier: Apache-2.0

use nola_agent_engine::agent::protocol::{AgentCoreConfig, SessionId};
use nola_agent_engine::agent::AgentCore;
#[path = "../tests/support/model_identity.rs"]
mod fixture_model_identity;

fn main() {
    let session_id = std::env::args()
        .nth(1)
        .expect("usage: emit_fixture_snapshot <session-id>");
    let snapshot = AgentCore::try_new(AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string(session_id)),
        ..AgentCoreConfig::default()
    })
    .expect("create Core")
    .snapshot();
    println!(
        "{}",
        serde_json::to_string_pretty(&snapshot).expect("encode snapshot")
    );
}
