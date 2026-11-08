// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, SkillCommand,
    SkillInput, SkillSource,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use support::ApplicationRuntime;

fn input(source: SkillSource, version: &str) -> SkillInput {
    let instructions = "Review the supplied text.";
    SkillInput {
        id: "review".into(),
        source,
        version: version.into(),
        display_name: "Review".into(),
        summary: "Read-only review".into(),
        category: "custom".into(),
        instructions_ref: "skill://review/SKILL.md".into(),
        instructions_digest: format!("{:x}", Sha256::digest(instructions.as_bytes())),
        instructions_text: instructions.into(),
        required_tools: vec![],
        default_mode: None,
        enabled: true,
        created_at_ms: 10,
        updated_at_ms: 20,
    }
}

fn submit(runtime: &mut ApplicationRuntime, id: &str, command: SkillCommand) -> Option<Value> {
    let command = ApplicationCommand::Skill { command };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "skill-editor-test".into(),
                request_id: id.into(),
                command_type: "skill".into(),
                command_json,
                expected_revision: None,
                received_at: "2026-09-07T00:00:00Z".into(),
            },
            command,
        )
        .unwrap();
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => Some(result),
        outcome => {
            eprintln!("Skill request {id} was not committed: {outcome:?}");
            None
        }
    }
}

#[test]
fn builtin_sync_preserves_latest_activation_and_replays_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("builtin-sync.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let first = submit(
        &mut runtime,
        "install",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn, "v1")],
        },
    )
    .unwrap();
    let prepared = input(SkillSource::BuiltIn, "v2");
    submit(
        &mut runtime,
        "toggle",
        SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled: false,
            expected_revision: first[0]["revision"].as_i64().unwrap(),
            updated_at_ms: 50,
        },
    )
    .unwrap();
    let command = SkillCommand::SynchronizeBuiltIns {
        inputs: vec![prepared],
    };
    let updated = submit(&mut runtime, "upgrade", command.clone()).unwrap();
    assert_eq!(updated[0]["record"]["version"], "v2");
    assert_eq!(updated[0]["record"]["enabled"], false);
    assert_eq!(updated[0]["record"]["created_at_ms"], 10);
    assert_eq!(updated[0]["record"]["updated_at_ms"], 50);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(submit(&mut runtime, "upgrade", command).unwrap(), updated);
    assert_eq!(
        runtime
            .execute(ApplicationCommand::Skill {
                command: SkillCommand::GetEditor {
                    id: "review".into()
                }
            })
            .unwrap(),
        updated[0]
    );
}

#[test]
fn builtin_sync_rolls_back_earlier_entries_and_rejects_user_identity_collision() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("builtin-rollback.db")).unwrap();
    let mut valid = input(SkillSource::BuiltIn, "v1");
    valid.id = "new-builtin".into();
    let mut invalid = input(SkillSource::BuiltIn, "v1");
    invalid.instructions_digest = "invalid".into();
    assert!(submit(
        &mut runtime,
        "invalid-batch",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![valid.clone(), invalid],
        }
    )
    .is_none());
    assert_eq!(
        runtime
            .execute(ApplicationCommand::Skill {
                command: SkillCommand::ListEditors
            })
            .unwrap(),
        serde_json::json!([])
    );
    let user = submit(
        &mut runtime,
        "user",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v1"),
        },
    )
    .unwrap();
    assert!(submit(
        &mut runtime,
        "collision",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![valid, input(SkillSource::BuiltIn, "v2")],
        }
    )
    .is_none());
    assert_eq!(
        runtime
            .execute(ApplicationCommand::Skill {
                command: SkillCommand::ListEditors
            })
            .unwrap(),
        serde_json::json!([user])
    );
    assert!(runtime
        .execute(ApplicationCommand::Skill {
            command: SkillCommand::SynchronizeBuiltIns { inputs: vec![] }
        })
        .is_err());
}

#[test]
fn identical_package_on_new_startup_preserves_editor_revision_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("builtin-startup.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let installed = submit(
        &mut runtime,
        "install",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn, "v1")],
        },
    )
    .unwrap();
    let disabled = submit(
        &mut runtime,
        "disable",
        SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled: false,
            expected_revision: installed[0]["revision"].as_i64().unwrap(),
            updated_at_ms: 50,
        },
    )
    .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let mut same_package = input(SkillSource::BuiltIn, "v1");
    same_package.created_at_ms = 100;
    same_package.updated_at_ms = 100;
    let unchanged = submit(
        &mut runtime,
        "new-startup",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![same_package.clone()],
        },
    )
    .unwrap();
    assert_eq!(unchanged, serde_json::json!([disabled.clone()]));
    assert_eq!(editor(&mut runtime, "review"), disabled);

    // Same instructions do not make metadata changes a no-op.
    same_package.summary = "Updated package description".into();
    let changed = submit(
        &mut runtime,
        "metadata-update",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![same_package],
        },
    )
    .unwrap();
    assert!(changed[0]["revision"].as_i64().unwrap() > disabled["revision"].as_i64().unwrap());
    assert_eq!(
        changed[0]["record"]["summary"],
        "Updated package description"
    );
    assert_eq!(changed[0]["record"]["enabled"], false);
    assert_eq!(changed[0]["record"]["updated_at_ms"], 100);
}

#[test]
fn old_editor_cannot_revert_version_delete_or_resurrect_skill() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("skills.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let original = submit(
        &mut runtime,
        "create",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v1"),
        },
    )
    .unwrap();
    let revision = original["revision"].as_i64().unwrap();
    let mut edited = input(SkillSource::User, "v2");
    edited.enabled = false;
    let updated = submit(
        &mut runtime,
        "edit",
        SkillCommand::ConfigureUser {
            input: edited,
            expected_revision: revision,
        },
    )
    .unwrap();
    assert_eq!(updated["record"]["version"], "v2");
    assert_eq!(updated["record"]["enabled"], false);
    assert!(updated["revision"].as_i64().unwrap() > revision);
    let stale = || SkillCommand::ConfigureUser {
        input: input(SkillSource::User, "v1"),
        expected_revision: revision,
    };
    assert!(submit(&mut runtime, "stale", stale()).is_none());
    assert!(submit(
        &mut runtime,
        "stale-delete",
        SkillCommand::DeleteUserChecked {
            id: "review".into(),
            expected_revision: revision,
            deleted_at_ms: 30,
        }
    )
    .is_none());
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert!(submit(&mut runtime, "stale", stale()).is_none());
    assert_eq!(
        runtime
            .execute(ApplicationCommand::Skill {
                command: SkillCommand::GetEditor {
                    id: "review".into()
                }
            })
            .unwrap(),
        updated
    );
    assert!(submit(
        &mut runtime,
        "delete",
        SkillCommand::DeleteUserChecked {
            id: "review".into(),
            expected_revision: updated["revision"].as_i64().unwrap(),
            deleted_at_ms: 40,
        }
    )
    .is_some());
    assert!(submit(
        &mut runtime,
        "resurrect",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v3")
        }
    )
    .is_none());
}

#[test]
fn builtin_instruction_edit_checks_revision_and_preserves_user_state_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("builtin-edit.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let installed = submit(
        &mut runtime,
        "install",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn, "v1")],
        },
    )
    .unwrap();
    let disabled = submit(
        &mut runtime,
        "disable",
        SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled: false,
            expected_revision: installed[0]["revision"].as_i64().unwrap(),
            updated_at_ms: 50,
        },
    )
    .unwrap();
    let edit = |revision, text: &str| SkillCommand::SetBuiltInInstructionsChecked {
        id: "review".into(),
        expected_revision: revision,
        instructions_ref: "skill://review/immutable-version/SKILL.md".into(),
        instructions_digest: format!("{:x}", Sha256::digest(text.as_bytes())),
        instructions_text: text.into(),
        updated_at_ms: 60,
    };
    let revision = disabled["revision"].as_i64().unwrap();
    let command = edit(revision, "Updated instructions.");
    assert!(runtime
        .execute(ApplicationCommand::Skill {
            command: command.clone()
        })
        .is_err());
    let saved = submit(&mut runtime, "save", command.clone()).unwrap();
    assert_eq!(saved["record"]["enabled"], false);
    for key in [
        "display_name",
        "summary",
        "category",
        "required_tools",
        "default_mode",
        "created_at_ms",
    ] {
        assert_eq!(saved["record"][key], disabled["record"][key]);
    }
    assert!(submit(&mut runtime, "stale-save", edit(revision, "Lost update")).is_none());
    let mut invalid = edit(saved["revision"].as_i64().unwrap(), "Another update");
    if let SkillCommand::SetBuiltInInstructionsChecked {
        instructions_digest,
        ..
    } = &mut invalid
    {
        *instructions_digest = "invalid".into();
    }
    assert!(submit(&mut runtime, "invalid-save", invalid).is_none());
    assert_eq!(editor(&mut runtime, "review"), saved);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(submit(&mut runtime, "save", command).unwrap(), saved);
    assert_eq!(editor(&mut runtime, "review"), saved);
    let configured = submit(&mut runtime, "combined-edit", SkillCommand::ApplyChanges { commands: vec![SkillCommand::ConfigureBuiltIn {
        id: "review".into(), expected_revision: saved["revision"].as_i64().unwrap(),
        instructions_text: "Combined user instructions.".into(), enabled: true, updated_at_ms: 70,
    }] }).unwrap();
    assert_eq!(configured[0]["record"]["enabled"], true);
    assert_eq!(configured[0]["record"]["instructions_text"], "Combined user instructions.");
    assert!(submit(&mut runtime, "stale-combined", SkillCommand::ConfigureBuiltIn {
        id: "review".into(), expected_revision: saved["revision"].as_i64().unwrap(),
        instructions_text: "Lost update".into(), enabled: false, updated_at_ms: 80,
    }).is_none());
}

#[test]
fn builtin_package_refresh_must_not_replace_user_instruction_override() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("builtin-override-refresh.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let installed = submit(
        &mut runtime,
        "install",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn, "v1")],
        },
    )
    .unwrap();
    let text = "User-authored review instructions.";
    let saved = submit(
        &mut runtime,
        "edit",
        SkillCommand::SetBuiltInInstructionsChecked {
            id: "review".into(),
            expected_revision: installed[0]["revision"].as_i64().unwrap(),
            instructions_ref: "skill://review/user-version/SKILL.md".into(),
            instructions_digest: format!("{:x}", Sha256::digest(text.as_bytes())),
            instructions_text: text.into(),
            updated_at_ms: 50,
        },
    )
    .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    submit(
        &mut runtime,
        "package-upgrade",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn, "v2")],
        },
    )
    .unwrap();
    let active = editor(&mut runtime, "review");
    assert_eq!(
        active["record"]["instructions_text"], saved["record"]["instructions_text"],
        "package synchronization replaced user-owned instructions after restart"
    );
    assert_eq!(
        active["record"]["instructions_ref"],
        saved["record"]["instructions_ref"]
    );
}

#[test]
fn built_in_skill_can_be_toggled_but_not_user_edited_or_deleted() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("built-in.db")).unwrap();
    assert!(submit(
        &mut runtime,
        "invalid-create",
        SkillCommand::CreateUser {
            input: input(SkillSource::BuiltIn, "v1")
        }
    )
    .is_none());
    submit(
        &mut runtime,
        "registry",
        SkillCommand::Upsert {
            input: input(SkillSource::BuiltIn, "v1"),
        },
    )
    .unwrap();
    let toggled = submit(
        &mut runtime,
        "disable",
        SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled: false,
            expected_revision: 1,
            updated_at_ms: 30,
        },
    )
    .unwrap();
    assert_eq!(toggled["record"]["enabled"], false);
    let revision = toggled["revision"].as_i64().unwrap();
    assert!(submit(
        &mut runtime,
        "edit",
        SkillCommand::ConfigureUser {
            input: input(SkillSource::BuiltIn, "v2"),
            expected_revision: revision
        }
    )
    .is_none());
    assert!(submit(
        &mut runtime,
        "delete",
        SkillCommand::DeleteUserChecked {
            id: "review".into(),
            expected_revision: revision,
            deleted_at_ms: 40
        }
    )
    .is_none());
}

fn editor(runtime: &mut ApplicationRuntime, id: &str) -> Value {
    runtime
        .execute(ApplicationCommand::Skill {
            command: SkillCommand::GetEditor { id: id.into() },
        })
        .unwrap()
}

#[test]
fn batch_conflict_rolls_back_earlier_writes_and_replays_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("batch-conflict.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let original = submit(
        &mut runtime,
        "create",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v1"),
        },
    )
    .unwrap();
    let revision = original["revision"].as_i64().unwrap();
    let batch = || {
        let mut second = input(SkillSource::User, "v1");
        second.id = "second".into();
        SkillCommand::ApplyChanges {
            commands: vec![
                SkillCommand::CreateUser { input: second },
                SkillCommand::SetEnabledChecked {
                    id: "review".into(),
                    enabled: false,
                    expected_revision: revision + 1,
                    updated_at_ms: 30,
                },
            ],
        }
    };
    assert!(submit(&mut runtime, "batch", batch()).is_none());
    assert_eq!(editor(&mut runtime, "second"), Value::Null);
    assert_eq!(editor(&mut runtime, "review"), original);
    // Make the original precondition valid. Replaying the rejected request must
    // still return its original receipt, not execute the batch now.
    let updated = submit(
        &mut runtime,
        "advance",
        SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled: true,
            expected_revision: revision,
            updated_at_ms: 40,
        },
    )
    .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert!(submit(&mut runtime, "batch", batch()).is_none());
    assert_eq!(editor(&mut runtime, "second"), Value::Null);
    assert_eq!(editor(&mut runtime, "review"), updated);
}

#[test]
fn committed_batch_replay_does_not_increment_revisions_again() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("batch-commit.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let batch = || SkillCommand::ApplyChanges {
        commands: vec![
            SkillCommand::CreateUser {
                input: input(SkillSource::User, "v1"),
            },
            SkillCommand::SetEnabledChecked {
                id: "review".into(),
                enabled: false,
                expected_revision: 1,
                updated_at_ms: 30,
            },
        ],
    };
    let result = submit(&mut runtime, "batch", batch()).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert_eq!(result[1]["record"]["enabled"], false);
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert_eq!(submit(&mut runtime, "batch", batch()).unwrap(), result);
    assert_eq!(editor(&mut runtime, "review"), result[1]);
}

#[test]
fn batch_rejects_unadmitted_nested_unchecked_and_oversized_commands() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("batch-invalid.db")).unwrap();
    assert!(runtime
        .execute(ApplicationCommand::Skill {
            command: SkillCommand::ApplyChanges { commands: vec![] },
        })
        .is_err());
    for (index, invalid) in [
        SkillCommand::ApplyChanges { commands: vec![] },
        SkillCommand::Upsert {
            input: input(SkillSource::User, "v1"),
        },
        SkillCommand::SetEnabled {
            id: "review".into(),
            enabled: false,
            updated_at_ms: 30,
        },
        SkillCommand::DeleteUser {
            id: "review".into(),
            deleted_at_ms: 30,
        },
        SkillCommand::List,
    ]
    .into_iter()
    .enumerate()
    {
        assert!(submit(
            &mut runtime,
            &format!("invalid-{index}"),
            SkillCommand::ApplyChanges {
                commands: vec![
                    SkillCommand::CreateUser {
                        input: input(SkillSource::User, "v1")
                    },
                    invalid
                ],
            }
        )
        .is_none());
        assert_eq!(editor(&mut runtime, "review"), Value::Null);
    }
    assert!(submit(
        &mut runtime,
        "oversized",
        SkillCommand::ApplyChanges {
            commands: vec![
                SkillCommand::CreateUser {
                    input: input(SkillSource::User, "v1")
                };
                129
            ],
        }
    )
    .is_none());
    assert_eq!(editor(&mut runtime, "review"), Value::Null);
}

#[test]
fn repeated_activation_with_new_receipt_preserves_editor_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("batch-no-op.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let original = submit(
        &mut runtime,
        "create",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v1"),
        },
    )
    .unwrap();
    let revision = original["revision"].as_i64().unwrap();
    let command = |enabled, expected_revision| SkillCommand::ApplyChanges {
        commands: vec![SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled,
            expected_revision,
            updated_at_ms: 100,
        }],
    };
    let unchanged = submit(&mut runtime, "enable-again", command(true, revision)).unwrap();
    assert_eq!(unchanged[0], original);
    let changed = submit(&mut runtime, "disable", command(false, revision)).unwrap();
    assert_eq!(changed[0]["revision"], revision + 1);
    assert_eq!(changed[0]["record"]["updated_at_ms"], 100);
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    let unchanged = submit(&mut runtime, "disable-again", command(false, revision + 1)).unwrap();
    assert_eq!(unchanged, changed);
    assert_eq!(editor(&mut runtime, "review"), changed[0]);
    // A no-op must still enforce its revision precondition.
    assert!(submit(&mut runtime, "stale-no-op", command(false, revision)).is_none());
    assert_eq!(editor(&mut runtime, "review"), changed[0]);
}

#[test]
fn restore_builtin_visibility_and_activation_share_one_receipt() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("restore.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let mut first = input(SkillSource::BuiltIn, "v1");
    first.enabled = false;
    submit(
        &mut runtime,
        "first",
        SkillCommand::Upsert {
            input: first.clone(),
        },
    )
    .unwrap();
    let mut second = first;
    second.id = "second".into();
    submit(
        &mut runtime,
        "second",
        SkillCommand::Upsert { input: second },
    )
    .unwrap();
    let restore = |second_revision, visibility_revision| SkillCommand::RestoreBuiltIns {
        expected_revisions: [("review".into(), 1), ("second".into(), second_revision)].into(),
        expected_visibility_revision: visibility_revision,
        updated_at_ms: 100,
    };
    assert!(submit(&mut runtime, "bad-second", restore(2, None)).is_none());
    assert_eq!(editor(&mut runtime, "review")["revision"], 1);
    assert_eq!(editor(&mut runtime, "review")["record"]["enabled"], false);
    assert!(submit(&mut runtime, "bad-setting", restore(1, Some(1))).is_none());
    assert!(submit(
        &mut runtime,
        "incomplete",
        SkillCommand::RestoreBuiltIns {
            expected_revisions: [("review".into(), 1)].into(),
            expected_visibility_revision: None,
            updated_at_ms: 100,
        }
    )
    .is_none());
    let result = submit(&mut runtime, "restore", restore(1, None)).unwrap();
    assert!(result
        .as_array()
        .unwrap()
        .iter()
        .all(|value| value["record"]["enabled"] == true));
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert_eq!(
        submit(&mut runtime, "restore", restore(1, None)).unwrap(),
        result
    );
    // The settings write must also have persisted; a new command expecting
    // absence is rejected even though both Skills are already enabled.
    assert!(submit(
        &mut runtime,
        "stale-visibility",
        SkillCommand::RestoreBuiltIns {
            expected_revisions: [("review".into(), 2), ("second".into(), 2)].into(),
            expected_visibility_revision: None,
            updated_at_ms: 101,
        }
    )
    .is_none());
    assert!(submit(
        &mut runtime,
        "fresh-visibility",
        SkillCommand::RestoreBuiltIns {
            expected_revisions: [("review".into(), 2), ("second".into(), 2)].into(),
            expected_visibility_revision: Some(1),
            updated_at_ms: 102,
        }
    )
    .is_some());
}

fn visibility(runtime: &mut ApplicationRuntime) -> Value {
    runtime
        .execute(ApplicationCommand::GetProductSetting {
            key: "skill.builtin.hidden".into(),
        })
        .unwrap()
}

#[test]
fn builtin_removal_preserves_existing_hidden_ids_and_replays_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("remove.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    submit(
        &mut runtime,
        "first",
        SkillCommand::Upsert {
            input: input(SkillSource::BuiltIn, "v1"),
        },
    )
    .unwrap();
    let mut second = input(SkillSource::BuiltIn, "v1");
    second.id = "second".into();
    submit(
        &mut runtime,
        "second",
        SkillCommand::Upsert { input: second },
    )
    .unwrap();
    let remove = |id: &str, expected_visibility_revision| SkillCommand::RemoveBuiltIn {
        id: id.into(),
        expected_revision: 1,
        expected_visibility_revision,
        updated_at_ms: 100,
    };
    let first = submit(&mut runtime, "remove-first", remove("review", None)).unwrap();
    assert_eq!(first["record"]["enabled"], false);
    let before = visibility(&mut runtime);
    assert!(submit(&mut runtime, "stale-second", remove("second", None)).is_none());
    assert_eq!(editor(&mut runtime, "second")["record"]["enabled"], true);
    assert_eq!(visibility(&mut runtime), before);
    let second = submit(&mut runtime, "remove-second", remove("second", Some(1))).unwrap();
    let saved = visibility(&mut runtime);
    let content: Value = serde_json::from_str(saved["value_json"].as_str().unwrap()).unwrap();
    assert_eq!(content["ids"], serde_json::json!(["review", "second"]));
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert_eq!(
        submit(&mut runtime, "remove-second", remove("second", Some(1))).unwrap(),
        second
    );
    assert_eq!(visibility(&mut runtime), saved);
    assert_eq!(editor(&mut runtime, "review"), first);
}

#[test]
fn builtin_removal_rejects_user_skills_and_corrupt_visibility_without_disabling() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("invalid-remove.db")).unwrap();
    submit(
        &mut runtime,
        "user",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v1"),
        },
    )
    .unwrap();
    let remove = |id: &str, expected_visibility_revision| SkillCommand::RemoveBuiltIn {
        id: id.into(),
        expected_revision: 1,
        expected_visibility_revision,
        updated_at_ms: 100,
    };
    assert!(submit(&mut runtime, "remove-user", remove("review", None)).is_none());
    assert_eq!(visibility(&mut runtime), Value::Null);
    let mut builtin = input(SkillSource::BuiltIn, "v1");
    builtin.id = "builtin".into();
    submit(
        &mut runtime,
        "builtin",
        SkillCommand::Upsert { input: builtin },
    )
    .unwrap();
    // Seed a valid JSON value with an invalid feature schema in this isolated DB.
    runtime
        .execute(ApplicationCommand::PutProductSetting {
            key: "skill.builtin.hidden".into(),
            value_json: "{}".into(),
            updated_at_ms: 99,
        })
        .unwrap();
    let before = visibility(&mut runtime);
    assert!(submit(&mut runtime, "invalid-setting", remove("builtin", Some(1))).is_none());
    assert_eq!(editor(&mut runtime, "builtin")["record"]["enabled"], true);
    assert_eq!(visibility(&mut runtime), before);
}

#[test]
fn editor_catalog_contains_current_snapshots_and_excludes_deleted_skills() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("editor-list.db")).unwrap();
    let created = submit(
        &mut runtime,
        "create",
        SkillCommand::CreateUser {
            input: input(SkillSource::User, "v1"),
        },
    )
    .unwrap();
    let query = || ApplicationCommand::Skill {
        command: SkillCommand::ListEditors,
    };
    assert_eq!(
        runtime.execute(query()).unwrap(),
        serde_json::json!([created])
    );
    let updated = submit(
        &mut runtime,
        "disable",
        SkillCommand::SetEnabledChecked {
            id: "review".into(),
            enabled: false,
            expected_revision: 1,
            updated_at_ms: 100,
        },
    )
    .unwrap();
    assert_eq!(
        runtime.execute(query()).unwrap(),
        serde_json::json!([updated])
    );
    submit(
        &mut runtime,
        "delete",
        SkillCommand::DeleteUserChecked {
            id: "review".into(),
            expected_revision: 2,
            deleted_at_ms: 101,
        },
    )
    .unwrap();
    assert_eq!(runtime.execute(query()).unwrap(), serde_json::json!([]));
}

#[test]
fn skill_port_sync_failure_rolls_back_bundle_selection_and_editor_with_receipt() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("skill-port-sync.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let installed = submit(
        &mut runtime,
        "install",
        SkillCommand::SynchronizeBuiltIns {
            inputs: vec![input(SkillSource::BuiltIn, "v1")],
        },
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_skill_port_sync BEFORE INSERT ON sync_outbox WHEN NEW.entity_type='skill' BEGIN SELECT RAISE(ABORT,'skill sync failed'); END").unwrap();
    let text = "new immutable body";
    let command = SkillCommand::SetBuiltInInstructionsChecked {
        id: "review".into(),
        expected_revision: installed[0]["revision"].as_i64().unwrap(),
        instructions_ref: "skill://override".into(),
        instructions_digest: format!("{:x}", Sha256::digest(text.as_bytes())),
        instructions_text: text.into(),
        updated_at_ms: 50,
    };
    assert!(submit(&mut runtime, "failed-override", command.clone()).is_none());
    assert_eq!(editor(&mut runtime, "review"), installed[0]);
    let selection: (String, Option<String>) = connection.query_row(
        "SELECT package_version, override_version FROM skill_instruction_selections WHERE skill_id='review'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(selection, ("v1".into(), None));
    let versions: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM skill_versions WHERE skill_id='review'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        versions, 1,
        "failed admitted override must not leave its immutable bundle"
    );
    connection
        .execute_batch("DROP TRIGGER fail_skill_port_sync")
        .unwrap();
    let committed = submit(&mut runtime, "override", command.clone()).unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(
        submit(&mut runtime, "override", command).unwrap(),
        committed
    );
    assert_eq!(editor(&mut runtime, "review"), committed);
}
