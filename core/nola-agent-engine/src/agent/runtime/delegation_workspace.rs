// SPDX-License-Identifier: Apache-2.0
//! Durable workspace preparation before the original delegation is authorized.
//! No IO, permission grant, cleanup, or alternative task engine.
use super::*;
use crate::agent::protocol::ModelToolCall;
use crate::agent::tool_lifecycle::DelegationWorkspacePreparation as Preparation;
use nola_domain::task_workspace::{TaskRepositoryIdentity, TaskWorktreeSelection};
use nola_host_protocol::delegation::DelegationParentSnapshot;
use sha2::{Digest, Sha256};
use std::path::Path;

pub(super) fn source_effect_requires_worker<'a>(
    root: &Path,
    tool: &str,
    resources: impl Iterator<Item = &'a nola_domain::permission_scope::PermissionResource>,
) -> bool {
    use nola_domain::permission_scope::PermissionResource;
    if tool == "git_worktree_integrate" {
        return false;
    }
    let source = root
        .components()
        .filter_map(|part| {
            if let std::path::Component::Normal(value) = part {
                value.to_str().map(str::to_owned)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    resources.into_iter().any(|resource| match resource {
        PermissionResource::File {
            components, access, ..
        } => {
            matches!(access.as_str(), "write" | "overwrite" | "remove")
                && components.starts_with(&source)
        }
        // An invocation's cwd establishes neither a write effect nor a read
        // guarantee. Keep its normal Host risk/permission/sandbox admission;
        // declared source-file effects above still require a worker. Inferring
        // edits from cwd alone prevents verification of the integrated result.
        _ => false,
    })
}

fn eligible(parent: &DelegationParentSnapshot, arguments: &Value) -> bool {
    if parent.project_repository_identity.is_none()
        || parent.task_worktree.is_some()
        || arguments.get("task_id").is_some()
        || arguments.get("worktree").is_some()
        || arguments.get("permission_mode").and_then(Value::as_str) == Some("read_only")
    {
        return false;
    }
    let Some(profile) = parent.profiles.get(
        arguments
            .get("agent_type")
            .and_then(Value::as_str)
            .unwrap_or("explore"),
    ) else {
        return false;
    };
    profile.tools.is_empty()
        || profile
            .tools
            .iter()
            .any(|tool| !parent.read_only_tools.contains(tool))
}

fn destination(parent: &DelegationParentSnapshot, identity: &str) -> Result<String, String> {
    let root = Path::new(&parent.project_root);
    // Prefer the narrowest independent Host scope. A project root alone does
    // not authorize its parent; the Host prepares a private scratch container.
    let directory = parent.authorized_workspace_roots.iter()
        .map(Path::new)
        .filter(|allowed| allowed.is_absolute() && !allowed.starts_with(root))
        .max_by_key(|allowed| allowed.components().count())
        .ok_or("No independent Host-authorized workspace directory is available. Configure a private managed workspace scope outside the project before retrying; OS directory access alone cannot expand frozen write authority.")?;
    let directory = if root.parent().is_some_and(|parent| parent.starts_with(directory)) {
        root.parent().unwrap()
    } else { directory };
    let suffix = format!("{:x}", Sha256::digest(identity.as_bytes()));
    Ok(directory
        .join(format!(".nola-task-{}", &suffix[..24]))
        .to_string_lossy()
        .into_owned())
}

enum WorkspacePlan {
    Wait,
    Run(Preparation, ModelToolCall),
    Ready(TaskWorktreeSelection),
}

fn preparation_call_id(identity: &str, stage: &str, sequence: u64) -> CallId {
    // Owners include session/Run/provider IDs; concatenating them exceeded the
    // canonical Part-body identity limit. Retain deterministic, bounded IDs.
    let bytes = serde_json::to_vec(&(identity, stage, sequence))
        .expect("workspace preparation identity serializes");
    CallId::from_string(format!("{:x}", Sha256::digest(bytes)))
}

fn plan_workspace_preparation(
    parent: &DelegationParentSnapshot,
    catalog: &crate::agent::host::HostCapabilityCatalog,
    preparation: Option<Preparation>,
    tools: &[crate::agent::tool_lifecycle::ToolCallState],
    identity: &str,
    next_id_sequence: &mut u64,
) -> Result<WorkspacePlan, String> {
    match preparation {
        None => {
            if !["git_worktree_list", "git_worktree_create"]
                .iter()
                .all(|name| {
                    catalog
                        .resolve_available(name)
                        .is_some_and(|capability| capability.route == HostCapabilityRoute::Tool)
                })
            {
                return Err("This Host cannot prepare an isolated Git workspace; no task was started in the source directory".into());
            }
            let target = destination(&parent, identity)?;
            *next_id_sequence = next_id_sequence
                .checked_add(1)
                .ok_or("Core ID sequence exhausted")?;
            let call_id = preparation_call_id(identity, "inspect", *next_id_sequence);
            Ok(WorkspacePlan::Run(
                Preparation::Inspect {
                    call_id: call_id.clone(),
                    destination: target,
                },
                ModelToolCall {
                    call_id,
                    name: "git_worktree_list".into(),
                    arguments: json!({"root":parent.project_root}),
                    permission_scope: None,
                    permission_rationale: None,
                },
            ))
        }
        Some(preparation) => {
            let (id, _) = preparation.call();
            let tool = tools
                .iter()
                .find(|tool| &tool.call_id == id)
                .ok_or("Workspace preparation lost its exact call")?;
            if !tool.status.is_terminal() {
                return Ok(WorkspacePlan::Wait);
            }
            if tool.is_error || tool.status != ToolCallStatus::Completed {
                return Err("Workspace preparation failed or was denied; inspect the original preparation result. No source fallback, directory adoption or retry was performed".into());
            }
            let output = tool
                .output
                .as_ref()
                .ok_or("Workspace preparation has no result")?;
            match preparation {
                Preparation::Inspect { destination, .. } => {
                    if output
                        .get("root_is_repository_root")
                        .and_then(Value::as_bool)
                        != Some(true)
                    {
                        return Err("Automatic preparation requires the project Git checkout root, not a nested source directory".into());
                    }
                    let source_snapshot = match output
                        .get("source_has_uncommitted_changes")
                        .and_then(Value::as_bool)
                    {
                        Some(false) => None,
                        Some(true) => {
                            Some(
                                serde_json::from_value::<
                                    nola_domain::task_workspace::SourceSnapshotPlan,
                                >(
                                    output
                                        .get("source_snapshot_plan")
                                        .filter(|value| !value.is_null())
                                        .cloned()
                                        .ok_or_else(|| {
                                            format!(
                                                "Source cannot be snapshotted safely: {}",
                                                output
                                                    .get("snapshot_unavailable_reason")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or(
                                                        "untracked or unsupported source changes"
                                                    )
                                            )
                                        })?,
                                )
                                .map_err(|_| "Invalid source snapshot witness")?,
                            )
                        }
                        None => {
                            return Err("Source state is unknown; no checkout substituted".into())
                        }
                    };
                    let repository: TaskRepositoryIdentity = serde_json::from_value(
                        output
                            .get("repository_identity")
                            .cloned()
                            .ok_or("Missing repository witness")?,
                    )
                    .map_err(|_| "Invalid repository witness")?;
                    if Some(&repository) != parent.project_repository_identity.as_ref() {
                        return Err("Parent repository changed since invocation admission".into());
                    }
                    let selection = TaskWorktreeSelection {
                        source_snapshot: None,
                        root: destination.clone(),
                        expected_head: output
                            .get("head")
                            .and_then(Value::as_str)
                            .ok_or("Missing inspected HEAD")?
                            .into(),
                        expected_common_directory: repository.common_directory,
                        expected_directory_identity: None,
                    };
                    selection.validate()?;
                    if let Some(plan) = &source_snapshot {
                        plan.validate()?;
                        if plan.source_head != selection.expected_head {
                            return Err("Snapshot source HEAD differs from inspection".into());
                        }
                    }
                    let source_root = output
                        .get("root")
                        .and_then(Value::as_str)
                        .ok_or("Missing source checkout root witness")?
                        .to_owned();
                    *next_id_sequence = next_id_sequence
                        .checked_add(1)
                        .ok_or("Core ID sequence exhausted")?;
                    let call_id = preparation_call_id(identity, "create", *next_id_sequence);
                    let mut arguments = json!({"root":parent.project_root,"destination":destination,
                                    "expected_head":selection.expected_head,"expected_common_directory":selection.expected_common_directory});
                    if let Some(plan) = &source_snapshot {
                        arguments["source_snapshot"] =
                            serde_json::to_value(plan).map_err(|error| error.to_string())?;
                    }
                    Ok(WorkspacePlan::Run(
                        Preparation::Create {
                            call_id: call_id.clone(),
                            selection,
                            source_root,
                            source_snapshot,
                        },
                        ModelToolCall {
                            call_id,
                            name: "git_worktree_create".into(),
                            arguments,
                            permission_scope: None,
                            permission_rationale: None,
                        },
                    ))
                }
                Preparation::Create {
                    selection,
                    source_root,
                    source_snapshot,
                    ..
                } => {
                    let actual: TaskWorktreeSelection = serde_json::from_value(
                        output
                            .get("task_worktree_selection")
                            .cloned()
                            .ok_or("Missing created workspace identity")?,
                    )
                    .map_err(|_| "Invalid created workspace identity")?;
                    actual.validate()?;
                    let snapshot_matches = match (&source_snapshot, &actual.source_snapshot) {
                        (None, None) => actual.expected_head == selection.expected_head,
                        (Some(plan), Some(binding)) => {
                            binding.plan == *plan && binding.source_root == source_root
                        }
                        _ => false,
                    };
                    if !snapshot_matches {
                        return Err(
                            "Created workspace does not match its source snapshot plan".into()
                        );
                    }
                    use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState};
                    let receipt: ExecutionReceipt = serde_json::from_value(
                        output
                            .get("_nola_execution")
                            .cloned()
                            .ok_or("Workspace creation lacks an execution receipt")?,
                    )
                    .map_err(|_| "Invalid workspace receipt")?;
                    receipt.validate()?;
                    if receipt.state != ExecutionState::Completed
                        || !receipt.effects.iter().any(|effect| {
                            effect.operation == "git.worktree.create"
                                && effect.after_version.as_deref()
                                    == Some(actual.expected_head.as_str())
                        })
                        || ![
                            "git.worktree.head_matches",
                            "git.worktree.common_directory_matches",
                            "git.worktree.detached",
                            "git.worktree.root_matches",
                        ]
                        .iter()
                        .all(|predicate| {
                            receipt.checks.iter().any(|check| {
                                check.passed
                                    && check.predicate == *predicate
                                    && check.version == actual.expected_head
                            })
                        })
                        || (source_snapshot.is_some()
                            && !receipt.checks.iter().any(|check| {
                                check.passed
                                    && check.predicate == "git.worktree.source_snapshot_matches"
                                    && check.version == actual.expected_head
                            }))
                    {
                        return Err(
                            "Workspace creation has no completed verified checkout receipt".into(),
                        );
                    }
                    if source_snapshot.is_none()
                        && output
                            .get("source_has_uncommitted_changes")
                            .and_then(Value::as_bool)
                            != Some(false)
                    {
                        return Err("Source changed before checkout; created worktree retained, delegation blocked pending baseline review".into());
                    }
                    if actual.root != selection.root
                        || actual.expected_common_directory != selection.expected_common_directory
                        || actual
                            .expected_directory_identity
                            .as_ref()
                            .map(|ids| ids.common_directory.as_str())
                            != parent
                                .project_repository_identity
                                .as_ref()
                                .map(|repo| repo.common_directory_identity.as_str())
                    {
                        return Err("Created worktree does not match the frozen preparation".into());
                    }
                    Ok(WorkspacePlan::Ready(actual))
                }
            }
        }
    }
}

impl AgentRuntime {
    /// Returns true while normal approved tools are preparing the primary checkout.
    /// No model request or harness session may start until this returns false.
    pub(super) fn prepare_primary_workspace(&mut self, turn_id: &TurnId) -> Result<bool, String> {
        let turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or("no primary Turn")?;
        if &turn.turn_id != turn_id {
            return Err("primary workspace Turn differs".into());
        }
        if turn.context.runtime_snapshot.execution_backend.backend_id == "nola" {
            return Ok(false);
        }
        if turn.cancellation_reason.is_some()
            || turn.status == AgentTurnStatus::Cancelling
            || turn.status.is_terminal()
        {
            return Ok(false);
        }
        if let Some(error) = &turn.primary_workspace_failure {
            return Err(error.clone());
        }
        let parent = turn
            .context
            .configuration
            .turn_identity
            .delegation
            .as_ref()
            .ok_or("External execution has no admitted workspace preparation authority")?
            .snapshot()?;
        if parent.process_sandbox.is_some() {
            return Err(
                "Required native shell sandbox cannot cover the external execution unit".into(),
            );
        }
        if parent.task_worktree.is_some() || parent.project_repository_identity.is_none() {
            return Ok(false);
        }
        if let Some(binding) = self.session.execution.primary_worktree.clone() {
            binding.validate()?;
            if binding.source_root != parent.project_root
                || Some(&binding.source_repository) != parent.project_repository_identity.as_ref()
            {
                return Err("Primary workspace belongs to a different source project; it was not moved or replaced".into());
            }
            if [
                &binding.selection.root,
                &binding.selection.expected_common_directory,
            ]
            .iter()
            .any(|path| {
                !parent
                    .authorized_workspace_roots
                    .iter()
                    .any(|root| Path::new(path).starts_with(root))
            }) {
                return Err("Primary execution workspace authority was revoked".into());
            }
            self.set_primary_workspace_identity(&binding)?;
            return Ok(false);
        }
        let catalog = turn.context.capability_projection.frozen_host_catalog();
        let mut sequence = self.session.journal.next_id_sequence;
        let identity = format!(
            "{}:{}:primary",
            self.session.session_id.as_str(),
            turn_id.as_str()
        );
        let plan = plan_workspace_preparation(
            &parent,
            &catalog,
            turn.primary_workspace_preparation.clone(),
            &turn.tools,
            &identity,
            &mut sequence,
        );
        self.session.journal.next_id_sequence = sequence;
        match plan {
            Ok(WorkspacePlan::Run(preparation, call)) => {
                let turn = self
                    .session
                    .execution
                    .active_turn
                    .as_mut()
                    .ok_or("no primary Turn")?;
                let admitted = turn.admit_tool_calls(vec![call.clone()]);
                turn.tools.extend(admitted);
                turn.primary_workspace_preparation = Some(preparation);
                self.session.record_turn_items(
                    turn_id,
                    vec![AgentItem::ToolCall {
                        id: ItemId::from_string(format!("{}:item", call.call_id.as_str())),
                        call_id: call.call_id,
                        name: call.name,
                        arguments: call.arguments,
                    }],
                );
                Ok(true)
            }
            Ok(WorkspacePlan::Wait) => Ok(true),
            Ok(WorkspacePlan::Ready(selection)) => {
                let binding = nola_domain::task_workspace::PrimaryWorktreeBinding {
                    source_root: parent.project_root,
                    source_repository: parent
                        .project_repository_identity
                        .ok_or("Missing primary repository")?,
                    selection,
                };
                binding.validate()?;
                self.session
                    .execution
                    .active_turn
                    .as_mut()
                    .ok_or("no primary Turn")?
                    .primary_workspace_preparation = None;
                self.session.execution.primary_worktree = Some(binding.clone());
                self.set_primary_workspace_identity(&binding)?;
                Ok(false)
            }
            Err(error) => {
                self.session
                    .execution
                    .active_turn
                    .as_mut()
                    .ok_or("no primary Turn")?
                    .primary_workspace_failure = Some(error.clone());
                Err(error)
            }
        }
    }

    fn set_primary_workspace_identity(
        &mut self,
        binding: &nola_domain::task_workspace::PrimaryWorktreeBinding,
    ) -> Result<(), String> {
        let source_identity = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or("no primary Turn")?
            .context
            .configuration
            .turn_identity
            .workspace
            .as_ref()
            .map(|identity| &identity.root_digest);
        let digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(binding, source_identity))
                    .map_err(|error| error.to_string())?
            )
        );
        self.session
            .execution
            .active_turn
            .as_mut()
            .ok_or("no primary Turn")?
            .context
            .runtime_snapshot
            .workspace = Some(crate::agent::AgentWorkspaceIdentity {
            workspace_id: digest.clone(),
            root_digest: digest,
            summary: None,
        });
        Ok(())
    }

    /// Native Git conversations coordinate edits through one managed worker chain.
    /// This does not migrate a live Run or attest process sandbox enforcement.
    pub(super) fn primary_native_workspace_root(&self) -> Result<Option<String>, String> {
        let Some(turn) = self.session.execution.active_turn.as_ref() else {
            return Ok(None);
        };
        let Some(parent) = turn.context.configuration.turn_identity.delegation.as_ref() else {
            return Ok(None);
        };
        let parent = parent.snapshot()?;
        Ok((parent.execution_backend == "nola"
            && parent.task_worktree.is_none()
            && parent.project_repository_identity.is_some())
        .then_some(parent.project_root))
    }

    /// Preparation consumes exact Host metadata, never the bounded model preview.
    /// Keep only the fields needed by this Core contract, not an entire worktree list.
    pub(super) fn workspace_preparation_observation(
        &self,
        call_id: &CallId,
        output: &Value,
    ) -> Option<Value> {
        let turn = self.session.execution.active_turn.as_ref()?;
        if !turn
            .tools
            .iter()
            .filter_map(|tool| tool.workspace_preparation.as_ref())
            .chain(turn.primary_workspace_preparation.as_ref())
            .any(|preparation| preparation.call().0 == call_id)
        {
            return None;
        }
        let fields = [
            "root",
            "head",
            "repository_identity",
            "root_is_repository_root",
            "source_has_uncommitted_changes",
            "source_snapshot_plan",
            "snapshot_unavailable_reason",
            "task_worktree_selection",
            "_nola_execution",
        ];
        Some(Value::Object(
            fields
                .into_iter()
                .filter_map(|key| output.get(key).map(|value| (key.into(), value.clone())))
                .collect(),
        ))
    }

    pub(super) fn prepare_delegation_workspaces(&mut self, turn_id: &TurnId) -> Result<(), String> {
        let execution_target_admission = self.execution_target_admission();
        let turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or("no active turn")?;
        if &turn.turn_id != turn_id {
            return Err("workspace preparation Turn differs".into());
        }
        if turn.cancellation_reason.is_some()
            || matches!(
                turn.status,
                AgentTurnStatus::Cancelling
                    | AgentTurnStatus::Cancelled
                    | AgentTurnStatus::Completed
                    | AgentTurnStatus::Failed
            )
        {
            return Ok(());
        }
        let Some(parent) = turn.context.configuration.turn_identity.delegation.as_ref() else {
            return Ok(());
        };
        let parent = parent.snapshot()?;
        let catalog = turn.context.capability_projection.frozen_host_catalog();
        let pending = turn
            .tools
            .iter()
            .filter(|tool| {
                tool.name == "spawn_agent"
                    && tool.status == ToolCallStatus::Pending
                    && tool.admission_error.is_none()
                    && tool.invocation_context_rejection.is_none()
            })
            .map(|tool| tool.call_id.clone())
            .collect::<Vec<_>>();
        let session_id = self.session.session_id.as_str().to_owned();
        let mut next_id_sequence = self.session.journal.next_id_sequence;
        let mut items = Vec::new();
        for original_id in pending {
            let turn = self
                .session
                .execution
                .active_turn
                .as_mut()
                .ok_or("no active turn")?;
            let index = turn
                .tools
                .iter()
                .position(|tool| tool.call_id == original_id)
                .ok_or("missing delegation")?;
            if let Some(capability) = catalog.resolve_available("spawn_agent") {
                crate::agent::tool_lifecycle::canonicalize_tool_arguments(
                    &mut turn.tools[index].arguments,
                    Some(&capability),
                );
            }
            let original = turn.tools[index].clone();
            let exposed = turn.user_delegation_call.as_ref() == Some(&original_id)
                || turn
                    .last_model_tool_projection
                    .as_ref()
                    .is_some_and(|projection| projection.capability_ids.contains(&original.name))
                || (turn
                    .context
                    .capability_projection
                    .available_capabilities
                    .get("spawn_agent")
                    .is_some_and(|capability| {
                        capability.route == HostCapabilityRoute::Tool
                            && capability.availability == CapabilityAvailability::Available
                            && capability.exposure.is_model_discoverable()
                    })
                    && catalog
                        .resolve_available("spawn_agent")
                        .is_some_and(|capability| capability.exposure.is_model_discoverable())
                    && turn.code_cells.values().any(|cell| {
                        cell.pending_calls().any(|call| {
                            cell.nested_call_id(call.sequence) == original_id.as_str()
                                && call.tool_name == "spawn_agent"
                        })
                    }));
            if !exposed
                || !catalog
                    .resolve_available("spawn_agent")
                    .is_some_and(|capability| {
                        capability.route == HostCapabilityRoute::Tool
                            && capability.validate_input(&original.arguments).is_ok()
                            && execution_target_admission
                                .validate(
                                    capability.input_normalization.execution_target.as_ref(),
                                    &original.arguments,
                                )
                                .is_ok()
                    })
            {
                continue;
            }
            if original.workspace_preparation.is_none() && !eligible(&parent, &original.arguments) {
                continue;
            }
            let planned = plan_workspace_preparation(
                &parent,
                &catalog,
                original.workspace_preparation.clone(),
                &turn.tools,
                &format!(
                    "{}:{}:{}",
                    session_id,
                    turn_id.as_str(),
                    original_id.as_str()
                ),
                &mut next_id_sequence,
            );
            match planned {
                Ok(WorkspacePlan::Run(preparation, call)) => {
                    let mut admitted = turn.admit_tool_calls(vec![call.clone()]);
                    for tool in &mut admitted {
                        tool.invocation_context = original.invocation_context.clone();
                    }
                    turn.tools[index].workspace_preparation = Some(preparation);
                    turn.tools.extend(admitted);
                    items.push(AgentItem::ToolCall {
                        id: ItemId::from_string(format!("{}:item", call.call_id.as_str())),
                        call_id: call.call_id,
                        name: call.name,
                        arguments: call.arguments,
                    });
                }
                Ok(WorkspacePlan::Wait) => {}
                Ok(WorkspacePlan::Ready(actual)) => {
                    turn.tools[index]
                        .arguments
                        .as_object_mut()
                        .ok_or("Invalid delegation arguments")?
                        .insert(
                            "worktree".into(),
                            serde_json::to_value(actual).map_err(|error| error.to_string())?,
                        );
                    turn.tools[index].workspace_preparation = None;
                }
                Err(message) => {
                    turn.tools[index].workspace_preparation = None;
                    turn.tools[index].admission_error = Some(
                        json!({"error":"task_workspace_preparation_blocked",
                        "message":message,"tool_execution_skipped":true,"source_fallback_performed":false}),
                    );
                }
            }
        }
        self.session.journal.next_id_sequence = next_id_sequence;
        if !items.is_empty() {
            self.session.record_turn_items(turn_id, items);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::host::HostToolOutcome;
    use nola_host_protocol::delegation::ValidatedDelegationParent;

    #[test]
    fn primary_code_guard_uses_resolved_resources_without_granting_delivery_authority() {
        use nola_domain::permission_scope::PermissionResource;
        let root = std::env::temp_dir().join("source-project");
        let components = root
            .components()
            .filter_map(|part| {
                if let std::path::Component::Normal(value) = part {
                    Some(value.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .chain(["code.rs".into()])
            .collect();
        let write = PermissionResource::File {
            filesystem_id: "fixture".into(),
            components,
            access: "overwrite".into(),
        };
        assert!(source_effect_requires_worker(
            &root,
            "edit_file",
            [&write].into_iter()
        ));
        assert!(!source_effect_requires_worker(
            &root,
            "git_worktree_integrate",
            [&write].into_iter()
        )); // Its normal overwrite approval still applies.
        let process = |cwd: std::path::PathBuf| PermissionResource::ProcessInvocation {
            invocation_digest: nola_domain::assistant_journal::AssistantDigest::parse_hex(
                &"a".repeat(64),
            )
            .unwrap(),
            command: "synthetic command".into(),
            cwd: cwd.to_string_lossy().into_owned(),
        };
        assert!(!source_effect_requires_worker(
            &root,
            "bash",
            [&process(root.clone())].into_iter()
        ));
        assert!(!source_effect_requires_worker(
            &root,
            "bash",
            [&process(std::env::temp_dir().join("managed-child"))].into_iter()
        ));
        assert!(source_effect_requires_worker(&root, "bash",
            [&process(root.clone()), &write].into_iter())); // Concrete writes still coordinate.
        let mut runtime = runtime();
        let original = runtime
            .session
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .context
            .configuration
            .turn_identity
            .delegation
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap();
        for swe_mode in [false, true] {
            let mut parent = original.clone();
            parent.model.swe_mode = swe_mode;
            runtime
                .session
                .execution
                .active_turn
                .as_mut()
                .unwrap()
                .context
                .configuration
                .turn_identity
                .delegation = Some(ValidatedDelegationParent::new(parent.clone()).unwrap());
            assert_eq!(
                runtime.primary_native_workspace_root().unwrap(),
                Some(parent.project_root)
            );
        }
        let mut non_git = original;
        non_git.project_repository_identity = None;
        runtime
            .session
            .execution
            .active_turn
            .as_mut()
            .unwrap()
            .context
            .configuration
            .turn_identity
            .delegation = Some(ValidatedDelegationParent::new(non_git).unwrap());
        assert!(runtime.primary_native_workspace_root().unwrap().is_none());
    }

    #[test]
    fn external_worker_profiles_share_preparation_without_claiming_a_sandbox() {
        let runtime = runtime();
        let mut parent = runtime
            .session
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .context
            .configuration
            .turn_identity
            .delegation
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap();
        let builtins = nola_domain::delegation::effective_agent_profiles(&Default::default());
        for name in ["codex", "claude_code", "opencode"] {
            parent.profiles.insert(name.into(), builtins[name].clone());
            let mut args = json!({"agent_type":name,"prompt":"Synthetic engineering task"});
            assert!(
                eligible(&parent, &args),
                "{name} must use the same preparation chain"
            );
            args["task_id"] = json!("existing-owner-task");
            assert!(
                !eligible(&parent, &args),
                "continuation must retain its frozen workspace"
            );
            assert!(args.get("sandbox_requirement").is_none());
        }
    }

    fn runtime() -> AgentRuntime {
        let base = super::super::user_delegation::tests::runtime(true);
        let mut config = base.config.clone();
        let mut parent = config
            .turn_identity
            .delegation
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap();
        let root = std::env::temp_dir().join("nola-core-workspace-fixture");
        parent.project_root = root.to_string_lossy().into_owned();
        parent.authorized_workspace_roots =
            vec![root.parent().unwrap().to_string_lossy().into_owned()];
        parent.project_repository_identity = Some(TaskRepositoryIdentity {
            common_directory: root.join(".git").to_string_lossy().into_owned(),
            common_directory_identity: "fixture-common".into(),
        });
        config.turn_identity.delegation = Some(ValidatedDelegationParent::new(parent).unwrap());
        let catalog = config.host_catalog.as_mut().unwrap();
        let descriptor = catalog.capabilities["spawn_agent"].clone();
        catalog
            .capabilities
            .get_mut("spawn_agent")
            .unwrap()
            .input_schema["properties"]["worktree"] = json!({"type":"object"});
        catalog
            .capabilities
            .get_mut("spawn_agent")
            .unwrap()
            .input_schema["properties"]["max_turns"] = json!({"type":"integer"});
        for name in ["git_worktree_list", "git_worktree_create"] {
            let mut capability = descriptor.clone();
            capability.id = name.into();
            capability.input_schema = json!({"type":"object"});
            catalog.capabilities.insert(name.into(), capability);
        }
        let mut runtime = AgentRuntime::try_new(config).unwrap();
        runtime
            .submit(
                AgentSubmission::UserDelegation {
                    agent_type: "general".into(),
                    prompt: "Synthetic implementation".into(),
                    submission_id: "workspace-command".into(),
                    submitted_at_epoch_ms: 1000,
                },
                1000,
            )
            .unwrap();
        runtime
    }

    #[test]
    fn worktree_destination_requires_host_scope_not_os_directory_access() {
        let runtime = runtime();
        let mut parent = runtime.session.execution.active_turn.as_ref().unwrap()
            .context.configuration.turn_identity.delegation.as_ref().unwrap().snapshot().unwrap();
        parent.authorized_workspace_roots = vec![parent.project_root.clone()];
        let error = destination(&parent, "scope-fixture").unwrap_err();
        assert!(error.contains("No independent Host-authorized workspace directory"));
        let scratch = std::env::temp_dir().join("private-host-workspaces");
        parent.authorized_workspace_roots.push(scratch.to_string_lossy().into_owned());
        assert_eq!(Path::new(&destination(&parent, "scope-fixture").unwrap()).parent(), Some(scratch.as_path()));
        let directory = Path::new(&parent.project_root).parent().unwrap().to_owned();
        parent.authorized_workspace_roots = vec![directory.to_string_lossy().into_owned()];
        let root = destination(&parent, "scope-fixture").unwrap();
        assert_eq!(Path::new(&root).parent(), Some(directory.as_path()));
        assert_ne!(root, parent.project_root);
    }

    #[test]
    fn preparation_identity_is_bounded_stable_and_keeps_owner_stage_and_sequence() {
        let owner = "session:run:provider-call:".repeat(20);
        let id = preparation_call_id(&owner, "inspect", u64::MAX);
        assert!(id.as_str().len() <=64);
        assert_eq!(id, preparation_call_id(&owner, "inspect", u64::MAX));
        assert_ne!(id, preparation_call_id(&owner, "create", u64::MAX));
        assert_ne!(id, preparation_call_id(&owner, "inspect", u64::MAX - 1));
        assert_ne!(id, preparation_call_id("other-owner", "inspect", u64::MAX));
        nola_domain::CanonicalBodyReference::for_field(
            "body".into(), "conversation".into(), "message".into(),
            format!("{}:item", id.as_str()), nola_domain::PartBodyField::ContentJson, 1, 1,
        ).unwrap();
    }

    fn inspected() -> Value {
        let root = std::env::temp_dir().join("nola-core-workspace-fixture");
        json!({"head":"a".repeat(40),"root":root,"repository_root":root,"root_is_repository_root":true, "source_has_uncommitted_changes":false,
            "repository_identity":{"common_directory":root.join(".git"),"common_directory_identity":"fixture-common"}})
    }

    #[test]
    fn external_primary_preparation_is_checkpointed_and_reuses_its_checkout() {
        let base = runtime();
        let mut config = base.config.clone();
        let mut parent = config
            .turn_identity
            .delegation
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap();
        parent.execution_backend = "codex".into();
        parent.credential = None;
        parent.fallback_credentials.clear();
        parent.model.endpoint.clear();
        parent.runtime.llm_fallbacks.clear();
        config.turn_identity.delegation = Some(ValidatedDelegationParent::new(parent).unwrap());
        config.turn_identity.execution_backend.backend_id = "codex".into();
        let catalog = config.host_catalog.as_mut().unwrap();
        let mut descriptor = catalog.capabilities["spawn_agent"].clone();
        descriptor.id = "agent.delegate".into();
        descriptor.route = HostCapabilityRoute::Model;
        catalog
            .capabilities
            .insert(descriptor.id.clone(), descriptor);
        let mut runtime = AgentRuntime::try_new(config).unwrap();
        let first = runtime
            .submit(
                AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("primary-input".into()),
                        "Synthetic code task",
                    )],
                    turn_context_items: Vec::new(),
                    turn_context_platform: Default::default(),
                    turn_context_components: Vec::new(),
                    submission_id: Some("primary-input".into()),
                    submitted_at_epoch_ms: Some(1000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                1000,
            )
            .unwrap();
        assert!(first.host_requests.iter().all(|request| !matches!(
            request.payload,
            crate::agent::host::HostRequest::DelegatedAgent { .. }
        )));
        let turn = runtime.session.execution.active_turn.as_mut().unwrap();
        let turn_id = turn.turn_id.clone();
        assert_eq!(turn.model_request_count, 0);
        assert_eq!(turn.tools.len(), 1);
        turn.tools[0].status = ToolCallStatus::Completed;
        turn.tools[0].output = Some(inspected());
        turn.status = AgentTurnStatus::Cancelling;
        assert!(!runtime.prepare_primary_workspace(&turn_id).unwrap());
        assert_eq!(
            runtime
                .session
                .execution
                .active_turn
                .as_ref()
                .unwrap()
                .tools
                .len(),
            1
        );
        let mut effects = Vec::new();
        runtime
            .request_model(&turn_id, &mut Vec::new(), &mut effects)
            .unwrap();
        assert!(
            effects.is_empty(),
            "cancellation cannot start a model or checkout"
        );
        runtime
            .session
            .execution
            .active_turn
            .as_mut()
            .unwrap()
            .status = AgentTurnStatus::AwaitingTools;
        assert!(runtime.prepare_primary_workspace(&turn_id).unwrap());
        let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
        let mut runtime = AgentRuntime::restore(
            runtime.config.clone(),
            serde_json::from_value(serde_json::to_value(checkpoint).unwrap()).unwrap(),
        )
        .unwrap();
        let turn = runtime.session.execution.active_turn.as_mut().unwrap();
        let Preparation::Create { mut selection, .. } =
            turn.primary_workspace_preparation.clone().unwrap()
        else {
            panic!("primary create")
        };
        selection.expected_directory_identity =
            Some(nola_domain::task_workspace::TaskWorktreeDirectoryIdentity {
                root: "primary-root".into(),
                git_directory: "primary-index".into(),
                common_directory: "fixture-common".into(),
            });
        let checks=["git.worktree.head_matches","git.worktree.common_directory_matches","git.worktree.detached","git.worktree.root_matches"]
            .map(|predicate|json!({"kind":"postcondition","resource_uri":"file:///fixture/primary/","version":selection.expected_head,"predicate":predicate,"passed":true}));
        turn.tools[1].status = ToolCallStatus::Completed;
        turn.tools[1].output = Some(
            json!({"task_worktree_selection":selection,"source_has_uncommitted_changes":false,
            "_nola_execution":{"state":"completed","local_stop":null,"effects":[{"resource_uri":"file:///fixture/primary/","operation":"git.worktree.create","after_version":selection.expected_head}],"checks":checks}}),
        );
        let mut stale_source = AgentRuntime::restore(runtime.config.clone(), runtime.checkpoint(CheckpointReason::Manual)).unwrap();
        stale_source.session.execution.active_turn.as_mut().unwrap().tools[1].output.as_mut().unwrap()["source_has_uncommitted_changes"] = json!(true);
        assert!(stale_source.prepare_primary_workspace(&turn_id).unwrap_err().contains("Source changed"));
        assert!(stale_source.session.execution.primary_worktree.is_none());
        assert!(!runtime.prepare_primary_workspace(&turn_id).unwrap());
        let binding = runtime.session.execution.primary_worktree.clone().unwrap();
        assert_eq!(binding.selection, selection);
        let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
        let mut restored = AgentRuntime::restore(
            runtime.config.clone(),
            serde_json::from_value(serde_json::to_value(checkpoint).unwrap()).unwrap(),
        )
        .unwrap();
        assert!(!restored.prepare_primary_workspace(&turn_id).unwrap());
        assert_eq!(
            restored
                .session
                .execution
                .active_turn
                .as_ref()
                .unwrap()
                .tools
                .len(),
            2
        );
        let mut parent = restored
            .session
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .context
            .configuration
            .turn_identity
            .delegation
            .as_ref()
            .unwrap()
            .snapshot()
            .unwrap();
        parent.authorized_workspace_roots = vec![parent.project_root.clone()];
        restored
            .session
            .execution
            .active_turn
            .as_mut()
            .unwrap()
            .context
            .configuration
            .turn_identity
            .delegation = Some(ValidatedDelegationParent::new(parent).unwrap());
        assert!(restored
            .prepare_primary_workspace(&turn_id)
            .unwrap_err()
            .contains("revoked"));
    }

    // Deterministic Core protocol transitions, not a real Host/model run.
    #[test]
    fn exact_preparation_survives_checkpoint_and_does_not_repeat_checkout() {
        for snapshot in [false, true] {
            let mut runtime = runtime();
            let turn = runtime.session.execution.active_turn.as_mut().unwrap();
            let turn_id = turn.turn_id.clone();
            assert_eq!(turn.tools.len(), 2);
            assert!(turn.tools[0].workspace_preparation.is_some());
            assert!(!turn
                .schedulable_tools()
                .iter()
                .any(|tool| tool.name == "spawn_agent"));
            turn.tools[1].status = ToolCallStatus::Completed;
            let mut info = inspected();
            if snapshot {
                info["source_has_uncommitted_changes"] = json!(true);
                info["source_snapshot_plan"] =
                    json!({"source_head":"a".repeat(40),"state_sha256":"b".repeat(64)});
            }
            turn.tools[1].output = Some(info);
            turn.status = AgentTurnStatus::Cancelling;
            runtime.prepare_delegation_workspaces(&turn_id).unwrap();
            assert_eq!(
                runtime
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .tools
                    .len(),
                2
            );
            runtime
                .session
                .execution
                .active_turn
                .as_mut()
                .unwrap()
                .status = AgentTurnStatus::AwaitingTools;
            // Round-trip precisely the state persisted by the normal checkpoint.
            let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
            let checkpoint =
                serde_json::from_value(serde_json::to_value(checkpoint).unwrap()).unwrap();
            let mut runtime = AgentRuntime::restore(runtime.config.clone(), checkpoint).unwrap();
            runtime.prepare_delegation_workspaces(&turn_id).unwrap();
            let turn = runtime.session.execution.active_turn.as_mut().unwrap();
            assert_eq!(turn.tools.len(), 3);
            let Preparation::Create { selection, .. } =
                turn.tools[0].workspace_preparation.clone().unwrap()
            else {
                panic!("create stage")
            };
            let mut actual = selection.clone();
            if snapshot {
                actual.expected_head = "c".repeat(40);
                actual.source_snapshot = Some(nola_domain::task_workspace::SourceSnapshotBinding {
                    source_root: std::env::temp_dir()
                        .join("nola-core-workspace-fixture")
                        .to_string_lossy()
                        .into_owned(),
                    tree: "d".repeat(40),
                    plan: nola_domain::task_workspace::SourceSnapshotPlan {
                        source_head: "a".repeat(40),
                        state_sha256: "b".repeat(64),
                    },
                });
            }
            actual.expected_directory_identity =
                Some(nola_domain::task_workspace::TaskWorktreeDirectoryIdentity {
                    root: "fixture-root".into(),
                    git_directory: "fixture-index".into(),
                    common_directory: "fixture-common".into(),
                });
            let checks = ["git.worktree.head_matches","git.worktree.common_directory_matches","git.worktree.detached","git.worktree.root_matches","git.worktree.source_snapshot_matches"]
            .map(|predicate|json!({"kind":"postcondition","resource_uri":"file:///fixture/worktree/","version":actual.expected_head,"predicate":predicate,"passed":true}));
            turn.tools[2].status = ToolCallStatus::Completed;
            turn.tools[2].output = Some(
                json!({"task_worktree_selection":actual,"source_has_uncommitted_changes":snapshot,
            "_nola_execution":{"state":"completed","local_stop":null,"effects":[{"resource_uri":"file:///fixture/worktree/","operation":"git.worktree.create","after_version":actual.expected_head}],"checks":checks}}),
            );
            runtime.prepare_delegation_workspaces(&turn_id).unwrap();
            runtime.prepare_delegation_workspaces(&turn_id).unwrap();
            let turn = runtime.session.execution.active_turn.as_ref().unwrap();
            assert_eq!(turn.tools.len(), 3);
            assert!(turn.tools[0].workspace_preparation.is_none());
            assert_eq!(
                turn.tools[0].arguments["worktree"]["root"], selection.root,
                "{:?}",
                turn.tools[0].admission_error
            );
            assert_eq!(turn.tools[0].status, ToolCallStatus::Pending);
        }
    }

    #[test]
    fn dirty_source_and_denied_checkout_never_fall_back_to_source() {
        for denied in [false, true] {
            let mut runtime = runtime();
            let turn = runtime.session.execution.active_turn.as_mut().unwrap();
            let id = turn.turn_id.clone();
            turn.tools[1].status = ToolCallStatus::Completed;
            let mut info = inspected();
            info["source_has_uncommitted_changes"] = json!(!denied);
            turn.tools[1].output = Some(info);
            runtime.prepare_delegation_workspaces(&id).unwrap();
            if denied {
                let turn = runtime.session.execution.active_turn.as_mut().unwrap();
                turn.tools[2].status = ToolCallStatus::Denied;
                turn.tools[2].is_error = true;
                runtime.prepare_delegation_workspaces(&id).unwrap();
            }
            let turn = runtime.session.execution.active_turn.as_ref().unwrap();
            assert!(turn.tools[0].admission_error.is_some());
            assert!(turn.tools[0].arguments.get("worktree").is_none());
            assert_eq!(turn.tools.len(), if denied { 3 } else { 2 });
        }
    }

    #[test]
    fn canonical_arguments_cannot_skip_workspace_preparation() {
        let mut runtime = runtime();
        let turn = runtime.session.execution.active_turn.as_mut().unwrap();
        let id = turn.turn_id.clone();
        turn.tools.truncate(1);
        turn.tools[0].workspace_preparation = None;
        turn.tools[0].arguments["max_turns"] = json!("4");
        runtime.prepare_delegation_workspaces(&id).unwrap();
        let turn = runtime.session.execution.active_turn.as_ref().unwrap();
        assert_eq!(turn.tools[0].arguments["max_turns"], 4);
        assert!(turn.tools[0].workspace_preparation.is_some());
        assert_eq!(turn.tools.len(), 2);
    }

    #[test]
    fn host_observation_survives_model_preview_truncation() {
        let mut runtime = runtime();
        let turn = runtime.session.execution.active_turn.as_mut().unwrap();
        let id = turn.turn_id.clone();
        let call = turn.tools[1].call_id.clone();
        turn.tools[1].status = ToolCallStatus::Running;
        let mut raw = inspected();
        raw["worktrees"] = json!(["metadata unrelated to the preparation decision".repeat(2000)]);
        let mut events = Vec::new();
        let mut effects = Vec::new();
        runtime
            .complete_tool(
                &id,
                &call,
                None,
                raw,
                HostToolOutcome::Succeeded,
                None,
                None,
                Vec::new(),
                None,
                false,
                &mut events,
                &mut effects,
            )
            .unwrap();
        let turn = runtime.session.execution.active_turn.as_ref().unwrap();
        assert!(
            matches!(
                turn.tools[0].workspace_preparation,
                Some(Preparation::Create { .. })
            ),
            "{:?}",
            turn.tools[0].admission_error
        );
        assert!(turn.tools[1]
            .output
            .as_ref()
            .unwrap()
            .get("worktrees")
            .is_none());
        assert!(turn.tools[1]
            .output
            .as_ref()
            .unwrap()
            .get("repository_identity")
            .is_some());
    }
}
