// SPDX-License-Identifier: Apache-2.0
//! Application-owned product setting projection.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductSetting {
    pub key: String,
    pub value_json: String,
    pub revision: i64,
    pub updated_at_ms: i64,
}

impl From<nola_store_port::product_setting::StoredProductSetting> for ProductSetting {
    fn from(setting: nola_store_port::product_setting::StoredProductSetting) -> Self {
        Self {
            key: setting.key,
            value_json: setting.value_json,
            revision: setting.revision,
            updated_at_ms: setting.updated_at_ms,
        }
    }
}

/// The admitted receipt owns the read/CAS/write unit; callers cannot edit a
/// setting using an unchecked read from a different snapshot.
pub(crate) fn put_checked<
    R: nola_store_port::product_setting::ProductSettingRepository + ?Sized,
>(
    store: &mut R,
    key: &str,
    value_json: &str,
    expected_revision: Option<i64>,
    updated_at_ms: i64,
) -> Result<ProductSetting, String> {
    if nola_domain::model_connection::owns_setting(key) {
        return Err("model connections and accounts require their owning model command".into());
    }
    if key == crate::persistent_agent::DIRECTORY_KEY
        || key.starts_with(crate::agent_room::ROOM_KEY_PREFIX)
    {
        return Err("persistent Agent directory must use its owning configuration command".into());
    }
    if key.starts_with(crate::agent_room::CONVERSATION_ROOM_KEY_PREFIX)
        || key.starts_with(crate::agent_room::ROOM_CONVERSATION_KEY_PREFIX)
    {
        return Err("conversation Room associations require their owning Agent Room command".into());
    }
    if key == crate::model_selection::DEFAULT_CHAT_MODEL_KEY {
        crate::model_selection::parse_default_chat_configuration(Some(value_json))?;
    }
    if !store.has_write_unit_of_work() {
        return Err("product setting editing requires durable command admission".into());
    }
    if expected_revision.is_some_and(|revision| revision < 1) {
        return Err("product setting editing requires a positive revision or absence".into());
    }
    let current = store
        .product_setting(key)
        .map_err(|error| error.to_string())?;
    if current.as_ref().map(|setting| setting.revision) != expected_revision {
        return Err(format!("product_setting_revision_conflict: {key}"));
    }
    store
        .put_product_setting(key, value_json, updated_at_ms)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod port_tests {
    use super::*;
    use nola_store_port::product_setting::{ProductSettingRepository, StoredProductSetting};
    struct Facts {
        admitted: bool,
        writes: usize,
    }
    impl ProductSettingRepository for Facts {
        type Error = &'static str;
        fn list_room_submissions(&self, _: &str, _: usize, _: usize) -> Result<Vec<StoredProductSetting>, Self::Error> {
            panic!("editor must not list room submissions")
        }
        fn list_product_settings(&self, _: &str, _: usize, _: usize) -> Result<Vec<StoredProductSetting>, Self::Error> {
            panic!("editor must not list settings")
        }
        fn has_write_unit_of_work(&self) -> bool {
            self.admitted
        }
        fn product_setting(&self, key: &str) -> Result<Option<StoredProductSetting>, Self::Error> {
            Ok(Some(StoredProductSetting {
                key: key.into(),
                value_json: "{}".into(),
                revision: 3,
                updated_at_ms: 1,
            }))
        }
        fn put_product_setting(
            &mut self,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<StoredProductSetting, Self::Error> {
            self.writes += 1;
            Err("storage unavailable")
        }
        fn delete_product_setting(&mut self, _: &str) -> Result<bool, Self::Error> {
            panic!("editor must not delete")
        }
    }
    #[test]
    fn actual_checked_editor_requires_admission_and_cas_before_write() {
        let mut facts = Facts {
            admitted: false,
            writes: 0,
        };
        assert!(put_checked(&mut facts, "agent_chat_conversation_room/c", "{}", Some(3), 5)
            .unwrap_err().contains("owning Agent Room command"));
        assert!(put_checked(&mut facts, "agent_chat_room_conversation/r", "{}", Some(3), 5)
            .unwrap_err().contains("owning Agent Room command"));
        assert_eq!(facts.writes, 0);
        assert!(put_checked(&mut facts, "key", "{}", Some(3), 5)
            .unwrap_err()
            .contains("admission"));
        facts.admitted = true;
        assert!(put_checked(&mut facts, "key", "{}", Some(2), 5)
            .unwrap_err()
            .contains("revision_conflict"));
        assert_eq!(facts.writes, 0);
        assert!(put_checked(&mut facts, "key", "{}", Some(3), 5)
            .unwrap_err()
            .contains("storage unavailable"));
        assert_eq!(facts.writes, 1);
    }
}
