use nola_core::agent_trace::{
    reduce_agent_trace, AgentTraceEvent, AgentTraceResourceOperation, AgentTraceStatus,
};

#[test]
fn shared_long_task_fixture_reduces_without_loss_or_duplication() {
    let events: Vec<AgentTraceEvent> =
        serde_json::from_str(include_str!("fixtures/long_task_trace_v1.json"))
            .expect("shared Agent Trace fixture must decode");
    let projection = reduce_agent_trace(&events).unwrap();

    assert_eq!(projection.last_sequence, 9);
    assert_eq!(projection.status, AgentTraceStatus::Succeeded);
    assert!(events.iter().all(|event| serde_json::to_value(event)
        .expect("control trace serializes")["payload"]
        .get("delta")
        .is_none()));
    assert_eq!(projection.activities.len(), 2);
    assert_eq!(projection.activities[0].span_id, "tool_search_1");
    assert_eq!(projection.activities[1].span_id, "tool_read_1");
    assert_eq!(
        projection.activities[1].resources[0].operation,
        AgentTraceResourceOperation::Read
    );
    assert_eq!(
        projection.activities[1].resources[0].uri,
        "Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md"
    );
    assert_eq!(
        projection
            .usage
            .expect("usage snapshot must survive projection")
            .context_window_tokens,
        131_072
    );
}
