// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Lightweight due-work probe used by the bundled macOS LaunchAgent.

#[cfg(test)]
use chrono::{Local, TimeZone};
use nola_application::{
    ApplicationCommand, AutomationCommand, AutomationDefinition, AutomationDefinitionCommand,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TICK_ARGUMENT: &str = "--background-scheduler-tick";
const MAX_CATCH_UP_SECONDS: i64 = 8 * 24 * 60 * 60;

pub(crate) fn run_if_requested() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(TICK_ARGUMENT) {
        return None;
    }
    Some(match run_tick() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("background scheduler: {error}");
            1
        }
    })
}

fn run_tick() -> Result<(), String> {
    let support_root = support_root()?;
    let state_path = support_root.join("Scheduler/background-agent-state.json");
    let now = unix_now()?;
    let previous = read_last_check(&state_path)
        .unwrap_or(now - 70)
        .max(now - MAX_CATCH_UP_SECONDS);
    let automations = read_automations()?;
    let web_sources = read_web_sources()?;
    let mut due = due_work(&automations, &web_sources, previous, now);
    due.goal_session_ids.extend(
        crate::session::due_goal_sessions(now.saturating_mul(1_000))?
            .into_iter()
            .map(|goal| goal.session_id),
    );
    let trigger_directory = support_root.join("Automations/TriggerState");
    let trigger_markers = read_trigger_markers(&trigger_directory);
    let triggered_automation_ids = triggered_automation_ids(&automations, &trigger_markers);
    due.automation_occurrences.extend(
        triggered_automation_ids
            .iter()
            .map(|id| format!("{id}@{now}")),
    );

    let valid_trigger_markers = triggered_automation_ids
        .iter()
        .map(|id| id.to_lowercase())
        .collect::<BTreeSet<_>>();
    let stale_trigger_markers = trigger_markers
        .difference(&valid_trigger_markers)
        .cloned()
        .collect::<BTreeSet<_>>();
    remove_trigger_markers(&trigger_directory, &stale_trigger_markers);

    if due.automation_occurrences.is_empty()
        && due.goal_session_ids.is_empty()
        && !due.refresh_web_sources
    {
        write_last_check(&state_path, now)?;
        return Ok(());
    }

    let executable = find_mac_app_executable()?;
    let mut command = Command::new(executable);
    command.arg("--nola-background-scheduler-run");
    if !due.automation_occurrences.is_empty() {
        command.arg("--automation-occurrences").arg(
            due.automation_occurrences
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if due.refresh_web_sources {
        command.arg("--refresh-web-sources");
    }
    if !due.goal_session_ids.is_empty() {
        command.arg("--goal-session-ids").arg(
            due.goal_session_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    let status = command
        .status()
        .map_err(|error| format!("cannot start the background app entry: {error}"))?;

    // The platform entry is only a wake transport. Core owns occurrence identity,
    // claim leases, retry eligibility and terminal state, so a non-zero child exit
    // must not make this probe manufacture a second occurrence.
    remove_trigger_markers(&trigger_directory, &valid_trigger_markers);
    write_last_check(&state_path, now)?;
    if !status.success() {
        eprintln!("background scheduler: scheduled work completed with {status}");
    }
    Ok(())
}

#[derive(Default)]
struct DueWork {
    automation_occurrences: BTreeSet<String>,
    goal_session_ids: BTreeSet<String>,
    refresh_web_sources: bool,
}

fn due_work(automations: &[Value], web_sources: &[Value], from: i64, through: i64) -> DueWork {
    let mut due = DueWork::default();
    for automation in automations {
        if automation.get("isEnabled").and_then(Value::as_bool)!=Some(true) {continue;}
        if let Some(input)=occurrence_input(automation,false,from,through) {
            if let Ok(page)=nola_application::automation_occurrence::query(&input) {
                for occurrence in page.occurrences {
                    due.automation_occurrences.insert(format!("{}@{}",input.automation_id,occurrence.scheduled_for_ms/1000));
                }
            }
        }
    }
    due.refresh_web_sources=web_sources.iter().filter(|value|value["isEnabled"]==true).any(|value|
        occurrence_input(value,true,from,through).and_then(|input|nola_application::automation_occurrence::query(&input).ok())
            .is_some_and(|page|!page.occurrences.is_empty()));
    due
}

/// Existing JSON schedule formats are adapters, not recurrence implementations.
fn occurrence_input(value:&Value,web:bool,from:i64,through:i64)->Option<nola_application::automation_occurrence::OccurrenceQuery> {
    let frequency=value[if web {"scheduleFrequency"}else{"frequency"}].as_str()?.to_string();
    let (hour,minute)=if web {(integer(value,"scheduledHour",9),integer(value,"scheduledMinute",0))}
        else {(integer(value,"hour",9),integer(value,"minute",0))};
    let sunday=integer(value,if web {"scheduledWeekday"}else{"weekday"},if web {2}else{0})-if web {1}else{0};
    Some(nola_application::automation_occurrence::OccurrenceQuery {
        automation_id:value["id"].as_str()?.into(),timezone:value["timezone"].as_str().unwrap_or("system").into(),frequency,
        time:Some(format!("{hour:02}:{minute:02}")),weekday:Some(((sunday+6)%7+1) as u32),
        once_at_ms:value["scheduledDate"].as_str().and_then(|raw|chrono::DateTime::parse_from_rfc3339(raw).ok()).map(|date|date.timestamp_millis()),
        after_ms:from.saturating_mul(1000),through_ms:through.saturating_mul(1000),limit:64,
    })
}

fn integer(value: &Value, key: &str, fallback: i64) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(fallback)
}

fn support_root() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("NOLA_MAC_APP_SUPPORT") {
        return Ok(PathBuf::from(configured));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is unavailable".to_string())?;
    Ok(PathBuf::from(home).join("Library/Application Support/Nola"))
}

fn read_automations() -> Result<Vec<Value>, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let value = application.execute(ApplicationCommand::Automation {
        command: AutomationCommand::Definition {
            command: AutomationDefinitionCommand::List,
        },
    })?;
    serde_json::from_value::<Vec<AutomationDefinition>>(value)
        .map_err(|error| format!("decode automation projection failed: {error}"))?
        .into_iter()
        .map(|definition| {
            serde_json::from_str::<Value>(&definition.metadata_json)
                .map_err(|error| format!("decode automation {} failed: {error}", definition.id))
        })
        .collect()
}

fn read_web_sources() -> Result<Vec<Value>, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let records: Vec<nola_application::DataSource> = serde_json::from_value(application.execute(
        nola_application::ApplicationCommand::DataSource {
            command: nola_application::DataSourceCommand::List,
        },
    )?)
    .map_err(|error| format!("decode canonical data sources failed: {error}"))?;
    records
        .into_iter()
        .filter(|record| record.kind == nola_application::DataSourceKind::Web)
        .map(|record| {
            let metadata = record
                .configuration
                .parameters
                .get("metadataJSON")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("data source {} has no metadata projection", record.id))
                .and_then(|raw| {
                    serde_json::from_str::<Value>(raw).map_err(|error| {
                        format!("decode data source {} metadata failed: {error}", record.id)
                    })
                })?;
            let source = metadata
                .get("webSource")
                .or_else(|| metadata.get("web_source"))
                .or_else(|| metadata.get("source"))
                .cloned()
                .ok_or_else(|| {
                    format!("data source {} has no schedulable projection", record.id)
                })?;
            let mut source = source
                .as_object()
                .cloned()
                .ok_or_else(|| format!("data source {} projection is not an object", record.id))?;
            source.insert("isEnabled".to_string(), Value::Bool(record.default_enabled));
            Ok(Value::Object(source))
        })
        .collect()
}

fn read_trigger_markers(directory: &Path) -> BTreeSet<String> {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("trigger") {
                return None;
            }
            path.file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_lowercase)
        })
        .collect()
}

fn triggered_automation_ids(automations: &[Value], markers: &BTreeSet<String>) -> BTreeSet<String> {
    automations
        .iter()
        .filter(|automation| automation.get("isEnabled").and_then(Value::as_bool) == Some(true))
        .filter_map(|automation| automation.get("id").and_then(Value::as_str))
        .filter(|id| markers.contains(&id.to_lowercase()))
        .map(str::to_owned)
        .collect()
}

fn remove_trigger_markers(directory: &Path, markers: &BTreeSet<String>) {
    for marker in markers {
        let _ = fs::remove_file(directory.join(format!("{marker}.trigger")));
    }
}

fn read_last_check(path: &Path) -> Option<i64> {
    fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
        .and_then(|value| value.get("lastCheckedAt").and_then(Value::as_i64))
}

fn write_last_check(path: &Path, timestamp: i64) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "invalid scheduler state path".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let data = serde_json::to_vec(&json!({ "lastCheckedAt": timestamp }))
        .map_err(|error| error.to_string())?;
    fs::write(&temporary, data).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn unix_now() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .map_err(|error| error.to_string())
}

fn find_mac_app_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = current
        .parent()
        .ok_or_else(|| "agent host has no executable directory".to_string())?;
    for preferred in ["NolaMac", "Nola"] {
        let candidate = directory.join(preferred);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    let entries = fs::read_dir(directory).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let candidate = entry.path();
        let name = candidate
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !name.starts_with("nola-") && is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err("cannot locate the Nola Mac executable beside nola-agent-host".to_string())
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_automation_is_selected_once_in_crossed_minute() {
        let date = Local.with_ymd_and_hms(2026, 7, 16, 9, 30, 0).unwrap();
        let input = vec![json!({
            "id": "A",
            "isEnabled": true,
            "frequency": "daily",
            "hour": 9,
            "minute": 30
        })];
        let due = due_work(&input, &[], date.timestamp() - 60, date.timestamp());
        assert_eq!(
            due.automation_occurrences.into_iter().collect::<Vec<_>>(),
            vec![format!("A@{}", date.timestamp())]
        );
    }

    #[test]
    fn disabled_and_nonmatching_work_is_ignored() {
        let date = Local.with_ymd_and_hms(2026, 7, 16, 9, 30, 0).unwrap();
        let input = vec![json!({
            "id": "A",
            "isEnabled": false,
            "frequency": "hourly",
            "minute": 30
        })];
        let due = due_work(&input, &[], date.timestamp() - 60, date.timestamp());
        assert!(due.automation_occurrences.is_empty());
        assert!(!due.refresh_web_sources);
    }

    #[test]
    fn catch_up_preserves_each_distinct_automation_occurrence() {
        let end = Local.with_ymd_and_hms(2026, 7, 16, 9, 32, 0).unwrap();
        let input = vec![json!({
            "id": "hourly",
            "isEnabled": true,
            "frequency": "hourly",
            "minute": 30
        })];
        let due = due_work(
            &input,
            &[],
            end.timestamp() - (2 * 60 * 60) - 60,
            end.timestamp(),
        );
        assert_eq!(due.automation_occurrences.len(), 2);
        assert!(due
            .automation_occurrences
            .iter()
            .all(|occurrence| occurrence.starts_with("hourly@")));
    }

    #[test]
    fn data_change_marker_selects_only_enabled_automation() {
        let automations = vec![
            json!({
                "id": "A1B2",
                "isEnabled": true,
                "frequency": "daily",
                "hour": 9,
                "minute": 30
            }),
            json!({
                "id": "C3D4",
                "isEnabled": false,
                "frequency": "daily",
                "hour": 9,
                "minute": 30
            }),
        ];
        let markers = BTreeSet::from(["a1b2".to_string(), "c3d4".to_string()]);
        let triggered = triggered_automation_ids(&automations, &markers);

        assert_eq!(triggered.into_iter().collect::<Vec<_>>(), vec!["A1B2"]);
    }

    #[test]
    fn canonical_web_source_projection_drives_scheduler_after_reopen() {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-background-web-source-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("State")).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("State/nola.db"));
        {
            let mut store = nola_store::NolaStore::open(root.join("State/nola.db")).unwrap();
            let created: nola_application::DataSource = serde_json::from_value(
                nola_application::execute_application_command(
                    &mut store,
                    nola_application::ApplicationCommand::DataSource {
                        command: nola_application::DataSourceCommand::CreateDraft {
                            input: nola_application::DataSourceDraft {
                                id: "web:fixture".into(),
                                name: "Fixture".into(),
                                kind: nola_application::DataSourceKind::Web,
                                configuration: nola_application::DataSourceConfiguration {
                                    connector_id: "test.web".into(),
                                    location_kind:
                                        nola_application::DataSourceLocationKind::HttpUrl,
                                    location: "https://example.test".into(),
                                    parameters: std::collections::BTreeMap::from([(
                                        "metadataJSON".into(),
                                        Value::String(
                                            json!({
                                                "version": 1,
                                                "webSource": {
                                                    "id": "7df7f0c0-dadd-4bb3-9820-192bf778aec0",
                                                    "scheduleFrequency": "daily",
                                                    "scheduledHour": 9,
                                                    "scheduledMinute": 30
                                                }
                                            })
                                            .to_string(),
                                        ),
                                    )]),
                                    secret_references: Vec::new(),
                                },
                                access_policy:
                                    nola_core::data_source::DataSourceAccessPolicy::DefaultEnabled,
                                authorization: nola_application::DataSourceAuthorization {
                                    state:
                                        nola_application::DataSourceAuthorizationState::NotRequired,
                                    capability: None,
                                    observed_at_ms: 1,
                                },
                                freshness_policy: nola_application::DataSourceFreshnessPolicy {
                                    maximum_age_minutes: 60,
                                    allow_stale_fallback: false,
                                },
                                capabilities: vec!["read".into(), "context".into()],
                                privacy: "cloud_reference".into(),
                                created_at_ms: 1,
                                updated_at_ms: 1,
                            },
                        },
                    },
                )
                .unwrap(),
            )
            .unwrap();
            let pending: nola_application::DataSource = serde_json::from_value(
                nola_application::execute_application_command(
                    &mut store,
                    nola_application::ApplicationCommand::DataSource {
                        command: nola_application::DataSourceCommand::RequestValidation {
                            id: created.id.clone(),
                            expected_revision: created.revision,
                            requested_at_ms: 2,
                        },
                    },
                )
                .unwrap(),
            )
            .unwrap();
            let mut contract: nola_core::data_source::DataSourceContract =
                serde_json::from_value(json!({
                    "sourceId": "web:fixture",
                    "connectorId": "test.web",
                    "connectorVersion": 1,
                    "readerDescription": "Fixture reader",
                    "readRecipe": {
                        "method": "httpGet",
                        "operationId": "test.web.read",
                        "configurationKeys": ["url"],
                        "outputSchema": {"type": "object", "properties": {"title": {"type": "string"}}}
                    },
                    "schemaFields": ["title"],
                    "redactedSample": "fixture",
                    "recordCount": 1,
                    "maximumAgeMinutes": 60,
                    "validationState": "verified",
                    "validatedAt": "2026-07-16T09:30:00Z",
                    "observedAt": "2026-07-16T09:30:00Z",
                    "validationDigest": ""
                })).unwrap();
            contract.seal_validation();
            let validated: nola_application::DataSource = serde_json::from_value(
                nola_application::execute_application_command(
                    &mut store,
                    nola_application::ApplicationCommand::DataSource {
                        command: nola_application::DataSourceCommand::RecordValidation {
                            id: created.id.clone(),
                            validation_token: pending.validation.validation_token.unwrap(),
                            result: nola_application::DataSourceValidationResult::Succeeded {
                                contract,
                            },
                            recorded_at_ms: 3,
                        },
                    },
                )
                .unwrap(),
            )
            .unwrap();
            nola_application::execute_application_command(
                &mut store,
                nola_application::ApplicationCommand::DataSource {
                    command: nola_application::DataSourceCommand::SetDefaultEnabled {
                        id: validated.id,
                        expected_revision: validated.revision,
                        enabled: true,
                        updated_at_ms: 4,
                    },
                },
            )
            .unwrap();
        }
        let sources = read_web_sources().unwrap();
        let date = Local.with_ymd_and_hms(2026, 7, 16, 9, 30, 0).unwrap();
        let due = due_work(&[], &sources, date.timestamp() - 60, date.timestamp());
        assert!(due.refresh_web_sources);
        std::env::remove_var("NOLA_DB_PATH");
        fs::remove_dir_all(root).unwrap();
    }
}
