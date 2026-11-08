use nola_core::agent::parts::{AgentPart, ConversationProjection, PartUpdate, ToolPartState};
use serde::Deserialize;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/normalized_part_lifecycle_contract_v1.json");

#[derive(Deserialize)]
struct ContractFixture {
    schema_version: u32,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    id: String,
    terminal_event: String,
    expected_final_text: Option<String>,
    events: Vec<Value>,
}

#[test]
fn normalized_part_lifecycle_contract_is_ordered_and_authoritative() {
    let fixture: ContractFixture = serde_json::from_str(FIXTURE).expect("decode lifecycle fixture");
    assert_eq!(fixture.schema_version, 1);

    for scenario in fixture.scenarios {
        let mut projection = ConversationProjection::default();
        let mut terminal_seen = false;
        let mut expected_sequence = 1_u64;
        for event in &scenario.events {
            assert_eq!(event["sequence"].as_u64(), Some(expected_sequence));
            expected_sequence += 1;
            let event_type = event["type"].as_str().expect("event type");
            if terminal_seen {
                panic!("{} emitted an event after its terminal event", scenario.id);
            }
            if event_type == "part_updated" {
                let update: PartUpdate = serde_json::from_value(event["update"].clone())
                    .expect("decode normalized Part update");
                projection
                    .apply(update)
                    .expect("apply normalized Part update");
                let expected: AgentPart = serde_json::from_value(event["part"].clone())
                    .expect("decode authoritative Part snapshot");
                assert_eq!(projection.parts.get(expected.id()), Some(&expected));
            } else {
                assert_eq!(event_type, scenario.terminal_event);
                terminal_seen = true;
            }
        }
        assert!(terminal_seen, "{} has no terminal event", scenario.id);

        let text_parts = projection
            .parts
            .values()
            .filter_map(|part| match part {
                AgentPart::Text(part) if !part.text.is_empty() => Some(part),
                _ => None,
            })
            .collect::<Vec<_>>();
        match scenario.expected_final_text.as_deref() {
            Some(expected) => {
                assert_eq!(text_parts.len(), 1, "{} duplicated final body", scenario.id);
                assert_eq!(text_parts[0].text.as_str(), expected);
                assert!(text_parts[0].is_complete);
            }
            None => assert!(text_parts.is_empty()),
        }

        let tool_states = projection
            .parts
            .values()
            .filter_map(|part| match part {
                AgentPart::Tool(part) => Some(part.state),
                _ => None,
            })
            .collect::<Vec<_>>();
        match scenario.id.as_str() {
            "completed" => assert_eq!(tool_states, [ToolPartState::Completed]),
            "failed" => assert_eq!(tool_states, [ToolPartState::Failed]),
            "cancelled" => assert_eq!(tool_states, [ToolPartState::Cancelled]),
            other => panic!("unexpected lifecycle scenario {other}"),
        }
    }
}
