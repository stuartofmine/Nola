//! Shared Mac/Web memory preference resolution. Preferences never grant Host authority.
use crate::application_port::CliApplicationPort;
use nola_application::ApplicationCommand;
use serde_json::Value;

pub(crate) const MEMORY_DEFAULT_KEY: &str = "assistant.memory.enabled";

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
            let settings: Value =
                serde_json::from_str(&raw).map_err(|_| "Invalid conversation settings")?;
            let fields = settings
                .as_object()
                .ok_or("Invalid conversation settings")?;
            if let Some(value) = fields
                .get("memoryEnabledOverride")
                .filter(|value| !value.is_null())
            {
                return value
                    .as_bool()
                    .ok_or_else(|| "Invalid conversation memory preference".into());
            }
        }
    }
    read_default(app)
}

pub(crate) fn read_default(app: &mut CliApplicationPort) -> Result<bool, String> {
    let stored: Value = app.execute(ApplicationCommand::GetProductSetting {
        key: MEMORY_DEFAULT_KEY.into(),
    })?;
    if stored.is_null() {
        return Ok(false); // Same initial default as MacAssistantPreferences.
    }
    decode_setting(&stored)
}

fn decode_setting(stored: &Value) -> Result<bool, String> {
    let raw = stored
        .get("value_json")
        .and_then(Value::as_str)
        .ok_or("Invalid memory preference record")?;
    let setting: Value = serde_json::from_str(raw).map_err(|_| "Invalid memory preference JSON")?;
    if setting.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported memory preference schema".into());
    }
    setting
        .get("value")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Invalid memory preference value".into())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryDefaultInput {
    pub enabled: bool,
}

pub(crate) fn save_default(
    app: &mut CliApplicationPort,
    enabled: bool,
    host_allowed: bool,
) -> Result<(), String> {
    if enabled && !host_allowed {
        return Err("Long-term memory is disabled by the Host".into());
    }
    let stored: Value = app.execute(ApplicationCommand::PutProductSetting {
        key: MEMORY_DEFAULT_KEY.into(),
        value_json: serde_json::json!({"schemaVersion":1,"value":enabled}).to_string(),
        updated_at_ms: chrono::Utc::now().timestamp_millis(),
    })?;
    if stored.get("key").and_then(Value::as_str) != Some(MEMORY_DEFAULT_KEY)
        || decode_setting(&stored)? != enabled
    {
        return Err("Memory preference write was not confirmed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn memory_default_write_is_shared_durable_and_cannot_enable_revoked_host_authority() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("write.sqlite");
        let mut app = CliApplicationPort::persistent(&database).unwrap();
        let _: Value = app
            .execute(ApplicationCommand::PutProductSetting {
                key: "assistant.retrieval.enabled".into(),
                value_json: json!({"schemaVersion":1,"value":false}).to_string(),
                updated_at_ms: 1,
            })
            .unwrap();
        save_default(&mut app, true, true).unwrap();
        drop(app);
        let mut app = CliApplicationPort::persistent(&database).unwrap();
        assert!(read_default(&mut app).unwrap());
        save_default(&mut app, false, false).unwrap();
        assert!(save_default(&mut app, true, false).is_err());
        assert!(!read_default(&mut app).unwrap());
        let other: Value = app
            .execute(ApplicationCommand::GetProductSetting {
                key: "assistant.retrieval.enabled".into(),
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(other["value_json"].as_str().unwrap()).unwrap()["value"],
            false
        );
        for invalid in [
            json!({}),
            json!({"enabled":"true"}),
            json!({"enabled":null}),
            json!({"enabled":true,"memory_enabled":false}),
        ] {
            assert!(serde_json::from_value::<MemoryDefaultInput>(invalid).is_err());
        }
    }

    #[test]
    fn shared_default_is_false_and_native_settings_are_read_strictly() {
        let root = tempfile::tempdir().unwrap();
        let mut app =
            CliApplicationPort::persistent(&root.path().join("preferences.sqlite")).unwrap();
        assert!(!resolve(&mut app, None, None).unwrap());
        for enabled in [true, false] {
            let _: Value = app
                .execute(ApplicationCommand::PutProductSetting {
                    key: MEMORY_DEFAULT_KEY.into(),
                    value_json: json!({"schemaVersion":1,"value":enabled}).to_string(),
                    updated_at_ms: 1,
                })
                .unwrap();
            assert_eq!(resolve(&mut app, None, None).unwrap(), enabled);
            assert_eq!(resolve(&mut app, None, Some(!enabled)).unwrap(), !enabled);
        }
        for value in [
            json!(true),
            json!({"schemaVersion":2,"value":true}),
            json!({"schemaVersion":1,"value":"true"}),
        ] {
            let _: Value = app
                .execute(ApplicationCommand::PutProductSetting {
                    key: MEMORY_DEFAULT_KEY.into(),
                    value_json: value.to_string(),
                    updated_at_ms: 2,
                })
                .unwrap();
            assert!(resolve(&mut app, None, None).is_err());
            assert!(!resolve(&mut app, None, Some(false)).unwrap());
        }
    }

    #[test]
    fn canonical_conversation_override_precedes_shared_default_but_not_explicit_intent() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("overrides.sqlite");
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        store
            .put_product_setting(
                MEMORY_DEFAULT_KEY,
                &json!({"schemaVersion":1,"value":true}).to_string(),
                1,
            )
            .unwrap();
        for (id, value) in [
            ("off", json!(false)),
            ("on", json!(true)),
            ("inherit", Value::Null),
            ("invalid", json!("false")),
        ] {
            store
                .create_conversation(
                    &nola_store_port::conversation_profile::CreateConversationInput {
                        id: id.into(),
                        title: id.into(),
                        revision: 1,
                        settings_json: Some(json!({"memoryEnabledOverride":value}).to_string()),
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                )
                .unwrap();
        }
        let mut app = CliApplicationPort::persistent(&database).unwrap();
        assert!(!resolve(&mut app, Some("off"), None).unwrap());
        assert!(resolve(&mut app, Some("on"), None).unwrap());
        assert!(resolve(&mut app, Some("inherit"), None).unwrap());
        assert!(resolve(&mut app, Some("off"), Some(true)).unwrap());
        assert!(!resolve(&mut app, Some("on"), Some(false)).unwrap());
        assert!(resolve(&mut app, Some("invalid"), None).is_err());
        let host_disabled = nola_core::memory::MemoryPolicy {
            long_term_enabled: false,
            ..Default::default()
        };
        assert!(crate::host_memory_admission::constrain_memory_policy(
            host_disabled,
            Some(resolve(&mut app, Some("on"), None).unwrap())
        )
        .is_err());
    }
}
