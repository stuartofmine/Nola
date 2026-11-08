use chrono::{Duration, SecondsFormat, Utc};
use nola_core::context_engine::{
    prepare_context, ContextComponent, ContextComponentKind, ContextComponentSource,
    ContextPrepareInput,
};
use serde_json::Value;

fn prepared_payload(authorization: Option<&str>, observed_at: &str) -> String {
    let input = ContextPrepareInput {
        components: vec![ContextComponent {
            kind: ContextComponentKind::PlatformData,
            id: "health".to_string(),
            content: Value::String("private-health-payload".to_string()),
            source: Some(ContextComponentSource {
                authorization: authorization.map(str::to_string),
                observed_at: Some(observed_at.to_string()),
                maximum_age_minutes: Some(60),
                ..ContextComponentSource::default()
            }),
        }],
        ..ContextPrepareInput::default()
    };
    serde_json::to_string(&prepare_context(&input)).expect("serialize prepared context")
}

#[test]
fn core_omits_unauthorized_platform_data() {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let payload = prepared_payload(Some("denied"), &now);
    assert!(!payload.contains("private-health-payload"));
    assert!(payload.contains("authorization is denied"));
}

#[test]
fn core_omits_stale_platform_data() {
    let stale = (Utc::now() - Duration::minutes(61)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let payload = prepared_payload(Some("granted"), &stale);
    assert!(!payload.contains("private-health-payload"));
    assert!(payload.contains("older than 60 minutes"));
}

#[test]
fn core_keeps_fresh_authorized_platform_data() {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let payload = prepared_payload(Some("granted"), &now);
    assert!(payload.contains("private-health-payload"));
}

#[test]
fn core_omits_protected_data_without_authorization_snapshot() {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let payload = prepared_payload(None, &now);
    assert!(!payload.contains("private-health-payload"));
    assert!(payload.contains("authorization is unknown"));
}

#[test]
fn core_keeps_fresh_data_that_requires_no_os_authorization() {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let payload = prepared_payload(Some("not_required"), &now);
    assert!(payload.contains("private-health-payload"));
}

#[test]
fn omitted_data_is_not_reported_as_injected_by_either_preparation_path() {
    for authorization in [None, Some("denied"), Some("not_determined")] {
        let input = ContextPrepareInput {
            components: vec![ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "protected-source".into(),
                content: Value::String("private-source-body".into()),
                source: Some(ContextComponentSource {
                    authorization: authorization.map(str::to_string),
                    observed_at: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
                    maximum_age_minutes: Some(60),
                    ..ContextComponentSource::default()
                }),
            }],
            ..ContextPrepareInput::default()
        };
        let prepared = prepare_context(&input);
        let turn = nola_core::context_engine::prepare_context_components(
            input.platform,
            &input.components,
            4096,
        );
        for metrics in [&prepared.component_metrics, &turn.component_metrics] {
            let metric = metrics
                .iter()
                .find(|metric| metric.kind == ContextComponentKind::PlatformData)
                .unwrap();
            assert!(metric.used_ids.is_empty());
            assert!(metric.truncated_ids.is_empty());
            assert_eq!(metric.dropped_ids, vec!["protected-source"]);
            assert!(
                metric.used_tokens > 0,
                "The safe omission notice still has a token cost"
            );
        }
        assert!(!serde_json::to_string(&prepared.messages)
            .unwrap()
            .contains("private-source-body"));
        assert!(!serde_json::to_string(&turn.messages)
            .unwrap()
            .contains("private-source-body"));
    }
}

#[test]
fn missing_source_metadata_cannot_bypass_platform_authorization() {
    for source in [None, Some(ContextComponentSource::default())] {
        let input = ContextPrepareInput {
            components: vec![ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "unattributed-source".into(),
                content: Value::String("must-not-be-disclosed".into()),
                source,
            }],
            ..ContextPrepareInput::default()
        };
        let prepared = prepare_context(&input);
        let turn = nola_core::context_engine::prepare_context_components(
            input.platform,
            &input.components,
            4096,
        );
        for messages in [&prepared.messages, &turn.messages] {
            let json = serde_json::to_string(messages).unwrap();
            assert!(!json.contains("must-not-be-disclosed"));
            assert!(json.contains("authorization is unknown"));
        }
        for metrics in [&prepared.component_metrics, &turn.component_metrics] {
            let metric = metrics
                .iter()
                .find(|metric| metric.kind == ContextComponentKind::PlatformData)
                .unwrap();
            assert!(metric.used_ids.is_empty());
            assert_eq!(metric.dropped_ids, vec!["unattributed-source"]);
        }
    }
}
