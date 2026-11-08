// SPDX-License-Identifier: Apache-2.0
//! The presentation ACK is typed; stale observations are not successful ACKs.
use nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement as Ack;

#[test]
fn notification_acknowledgement_has_three_distinct_current_wire_values() {
    for (value, wire) in [
        (Ack::Delivered, "delivered"),
        (Ack::AlreadyDelivered, "already_delivered"),
        (Ack::Superseded, "superseded"),
    ] {
        assert_eq!(serde_json::to_value(value).unwrap(), serde_json::json!(wire));
        assert_eq!(serde_json::from_value::<Ack>(serde_json::json!(wire)).unwrap(), value);
    }
    for invalid in [serde_json::json!(true), serde_json::json!(false),
        serde_json::json!(null), serde_json::json!("success"), serde_json::json!({"delivered":true})] {
        assert!(serde_json::from_value::<Ack>(invalid).is_err());
    }
}
