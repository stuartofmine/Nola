// SPDX-License-Identifier: Apache-2.0
//! Plugin installation intent. Host materializes/verifies package bytes and
//! owns live connections; enabled intent alone never authorizes a tool call.
use nola_domain::plugin::{PluginInstallation, PluginManifest};
use nola_store_port::plugin::PluginRepository;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginCommand {
    Get { id: String },
    List { after: Option<String>, limit: usize },
    Install {
        manifest: PluginManifest,
        package_digest: String,
        expected_revision: Option<i64>,
        now_ms: i64,
    },
    SetEnabled { id: String, expected_revision: i64, enabled: bool, now_ms: i64 },
    Remove { id: String, expected_revision: i64, now_ms: i64 },
}

pub(crate) fn execute(store: &mut impl PluginRepository, command: PluginCommand) -> Result<Value, String> {
    match command {
        PluginCommand::Get { id } => {
            // Keep the tombstone revision observable for explicit reinstall.
            let record = store.get(&id).map_err(|e| e.to_string())?;
            serde_json::to_value(record).map_err(|e| e.to_string())
        }
        PluginCommand::List { after, limit } => serde_json::to_value(
            store.list(after.as_deref(), limit).map_err(|e| e.to_string())?
        ).map_err(|e| e.to_string()),
        command => {
            if !store.has_write_unit_of_work() { return Err("plugin changes require durable command admission".into()); }
            let (record, expected) = match command {
                PluginCommand::Install { manifest, package_digest, expected_revision, now_ms } => {
                    manifest.validate()?;
                    if package_digest.len() != 64 || !package_digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                        return Err("invalid plugin package digest".into());
                    }
                    let previous = store.get(&manifest.id).map_err(|e| e.to_string())?;
                    if previous.as_ref().map(|r| r.revision) != expected_revision {
                        return Err("plugin_revision_conflict".into());
                    }
                    if previous.as_ref().is_some_and(|r| r.enabled) {
                        return Err("disable the installed plugin before replacing its package".into());
                    }
                    validate_time(now_ms, previous.as_ref())?;
                    (PluginInstallation { manifest, package_digest, enabled: false,
                        revision: next_revision(expected_revision.unwrap_or(0))?,
                        installed_at_ms: now_ms, updated_at_ms: now_ms, removed_at_ms: None }, expected_revision)
                }
                PluginCommand::SetEnabled { id, expected_revision, enabled, now_ms } => {
                    let mut record = editable(store, &id, expected_revision, now_ms)?;
                    record.enabled = enabled;
                    (record, Some(expected_revision))
                }
                PluginCommand::Remove { id, expected_revision, now_ms } => {
                    let mut record = editable(store, &id, expected_revision, now_ms)?;
                    record.enabled = false;
                    record.removed_at_ms = Some(now_ms);
                    (record, Some(expected_revision))
                }
                _ => unreachable!(),
            };
            store.save(&record, expected).map_err(|e| e.to_string())?;
            serde_json::to_value(record).map_err(|e| e.to_string())
        }
    }
}

fn next_revision(value: i64) -> Result<i64, String> {
    value.checked_add(1).filter(|v| *v > 0).ok_or_else(|| "invalid plugin revision".into())
}
fn validate_time(now: i64, previous: Option<&PluginInstallation>) -> Result<(), String> {
    if now < 0 || previous.is_some_and(|r| now < r.updated_at_ms) {
        return Err("invalid plugin update time".into());
    }
    Ok(())
}
fn editable(store: &impl PluginRepository, id: &str, revision: i64, now: i64) -> Result<PluginInstallation, String> {
    let mut record = store.get(id).map_err(|e| e.to_string())?
        .filter(|r| r.removed_at_ms.is_none()).ok_or("plugin not installed")?;
    if record.revision != revision { return Err("plugin_revision_conflict".into()); }
    validate_time(now, Some(&record))?;
    record.revision = next_revision(revision)?;
    record.updated_at_ms = now;
    Ok(record)
}
