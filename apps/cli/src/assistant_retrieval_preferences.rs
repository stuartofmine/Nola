//! Shared Mac/Web preference for automatic web-source context, not tool authorization.
use crate::application_port::CliApplicationPort;
use nola_application::ApplicationCommand;
use serde_json::Value;

pub(crate) const RETRIEVAL_DEFAULT_KEY: &str = "assistant.retrieval.enabled";

pub(crate) fn requested(body: &Value) -> Result<Option<bool>, String> {
    body.get("retrieval_enabled")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| "retrieval_enabled must be boolean".into())
        })
        .transpose()
}

pub(crate) fn resolve(
    app: &mut CliApplicationPort,
    conversation_id: Option<&str>,
    explicit: Option<bool>,
) -> Result<bool, String> {
    if let Some(enabled) = explicit {
        return Ok(enabled);
    }
    if let Some(id) = conversation_id {
        if let Some(raw) = app.load_conversation_settings(id)? {
            let value: Value =
                serde_json::from_str(&raw).map_err(|_| "Invalid conversation settings")?;
            let fields = value.as_object().ok_or("Invalid conversation settings")?;
            if let Some(value) = fields
                .get("retrievalEnabledOverride")
                .filter(|value| !value.is_null())
            {
                return value
                    .as_bool()
                    .ok_or_else(|| "Invalid retrieval override".into());
            }
        }
    }
    read_default(app)
}

pub(crate) fn read_default(app: &mut CliApplicationPort) -> Result<bool, String> {
    let stored: Value = app.execute(ApplicationCommand::GetProductSetting {
        key: RETRIEVAL_DEFAULT_KEY.into(),
    })?;
    if stored.is_null() {
        return Ok(true);
    } // MacAssistantPreferences initial default.
    decode_setting(&stored)
}

fn decode_setting(stored: &Value) -> Result<bool, String> {
    let raw = stored
        .get("value_json")
        .and_then(Value::as_str)
        .ok_or("Invalid retrieval preference record")?;
    let setting: Value =
        serde_json::from_str(raw).map_err(|_| "Invalid retrieval preference JSON")?;
    if setting.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported retrieval preference schema".into());
    }
    setting
        .get("value")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Invalid retrieval preference value".into())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrievalDefaultInput {
    pub enabled: bool,
}

pub(crate) fn save_default(app: &mut CliApplicationPort, enabled: bool) -> Result<(), String> {
    let stored: Value = app.execute(ApplicationCommand::PutProductSetting {
        key: RETRIEVAL_DEFAULT_KEY.into(),
        value_json: serde_json::json!({"schemaVersion":1,"value":enabled}).to_string(),
        updated_at_ms: chrono::Utc::now().timestamp_millis(),
    })?;
    if stored.get("key").and_then(Value::as_str) != Some(RETRIEVAL_DEFAULT_KEY)
        || decode_setting(&stored)? != enabled
    {
        return Err("Retrieval preference write was not confirmed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn retrieval_default_write_is_durable_and_does_not_change_memory() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("defaults.sqlite");
        let mut app = CliApplicationPort::persistent(&database).unwrap();
        crate::assistant_memory_preferences::save_default(&mut app, false, true).unwrap();
        save_default(&mut app, false).unwrap();
        drop(app);
        let mut app = CliApplicationPort::persistent(&database).unwrap();
        assert!(!resolve(&mut app, None, None).unwrap());
        save_default(&mut app, true).unwrap();
        assert!(resolve(&mut app, None, None).unwrap());
        assert!(!crate::assistant_memory_preferences::read_default(&mut app).unwrap());
        for invalid in [
            json!({}),
            json!({"enabled":"false"}),
            json!({"enabled":null}),
            json!({"enabled":true,"retrieval_enabled":false}),
        ] {
            assert!(serde_json::from_value::<RetrievalDefaultInput>(invalid).is_err());
        }
    }
    #[test]
    fn retrieval_preferences_share_native_defaults_and_exact_override_precedence() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("retrieval.sqlite");
        let mut app = CliApplicationPort::persistent(&database).unwrap();
        assert!(resolve(&mut app, None, None).unwrap());
        let _: Value = app
            .execute(ApplicationCommand::PutProductSetting {
                key: RETRIEVAL_DEFAULT_KEY.into(),
                value_json: json!({"schemaVersion":1,"value":false}).to_string(),
                updated_at_ms: 1,
            })
            .unwrap();
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        for (id, value) in [
            ("on", json!(true)),
            ("inherit", Value::Null),
            ("invalid", json!("true")),
        ] {
            store
                .create_conversation(
                    &nola_store_port::conversation_profile::CreateConversationInput {
                        id: id.into(),
                        title: id.into(),
                        revision: 1,
                        settings_json: Some(json!({"retrievalEnabledOverride":value}).to_string()),
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                )
                .unwrap();
        }
        assert!(!resolve(&mut app, Some("inherit"), None).unwrap());
        assert!(resolve(&mut app, Some("on"), None).unwrap());
        assert!(!resolve(&mut app, Some("on"), Some(false)).unwrap());
        assert!(resolve(&mut app, Some("invalid"), None).is_err());
        for invalid in [json!(null), json!("false"), json!(0)] {
            assert!(requested(&json!({"retrieval_enabled":invalid})).is_err());
        }
        assert_eq!(requested(&json!({})).unwrap(), None);
        assert_eq!(
            requested(&json!({"retrieval_enabled":false})).unwrap(),
            Some(false)
        );
        let _: Value = app
            .execute(ApplicationCommand::PutProductSetting {
                key: RETRIEVAL_DEFAULT_KEY.into(),
                value_json: json!({"schemaVersion":2,"value":true}).to_string(),
                updated_at_ms: 2,
            })
            .unwrap();
        assert!(resolve(&mut app, None, None).is_err());
    }
}
