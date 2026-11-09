fn update_agent_todos(args: &Map<String, Value>, state: &AgentPlanState) -> ToolOutcome {
    let new_todos = tool_try!(normalize_todos(args));
    let current_todos = match state.current_todos() {
        Ok(todos) => todos,
        Err(err) => return error(err),
    };
    let replan_reason = args
        .get("replan_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    if replan_reason.is_none() {
        if let Err(outcome) = validate_todo_transition(&current_todos, &new_todos) {
            return outcome;
        }
    }
    let replacement = match replan_reason {
        Some(reason) => state.replan_todos(new_todos.clone(), reason),
        None => state.replace_todos(new_todos.clone()),
    };
    let old_todos = match replacement {
        Ok(old) => old,
        Err(err) => return error(format!("persist agent todos failed: {err}")),
    };

    let todo_completed = new_todos
        .iter()
        .filter(|todo| todo.status == "completed")
        .count();
    let todo_current = new_todos
        .iter()
        .find(|todo| todo.status == "in_progress")
        .map(|todo| {
            compact_todo_content(if todo.active_form.is_empty() {
                &todo.content
            } else {
                &todo.active_form
            })
        });
    let todo_changes = summarize_todo_changes(&old_todos, &new_todos);
    let mut lines = vec![
        "agent_todos_updated: true".to_string(),
        format!("progress: {todo_completed}/{}", new_todos.len()),
        format!("current: {}", todo_current.as_deref().unwrap_or("(none)")),
        "changes:".to_string(),
    ];
    if todo_changes.is_empty() {
        lines.push("- (none)".to_string());
    } else {
        lines.extend(
            todo_changes
                .iter()
                .take(4)
                .map(|change| format!("- {change}")),
        );
        if todo_changes.len() > 4 {
            lines.push(format!("- ... and {} more", todo_changes.len() - 4));
        }
    }
    lines.extend([String::new(), format_todos(&new_todos)]);
    if new_todos.len() >= 3
        && new_todos.iter().all(|todo| todo.status == "completed")
        && !new_todos
            .iter()
            .any(|todo| contains_verify_word(&todo.content))
    {
        lines.extend([
            String::new(),
            "NOTE: You closed a 3+ item checklist without an explicit verification/test item. Before giving the final answer, verify the result or explain why verification is not applicable.".to_string(),
        ]);
    }

    let completed = new_todos
        .iter()
        .filter(|todo| todo.status == "completed")
        .count();
    let display = new_todos
        .iter()
        .find(|todo| todo.status == "in_progress")
        .map(|todo| {
            if todo.active_form.is_empty() {
                todo.content.clone()
            } else {
                todo.active_form.clone()
            }
        })
        .unwrap_or_else(|| {
            if new_todos.is_empty() {
                "Todos cleared".to_string()
            } else {
                format!("{completed}/{} completed", new_todos.len())
            }
        });
    ok(lines.join("\n"), display)
}

fn validate_todo_transition(old: &[AgentTodo], new: &[AgentTodo]) -> Result<(), ToolOutcome> {
    for previous in old {
        if (!previous.artifact_refs.is_empty() || !previous.work_step_transaction_refs.is_empty())
            && !new.iter().any(|todo| todo.id == previous.id)
        {
            return Err(error(format!(
                "plan step {} cannot discard persisted artifacts or work transactions by omitting the step; retain the step and its references",
                previous.id
            )));
        }
    }
    let old_by_id = old
        .iter()
        .map(|todo| (todo.id.as_str(), todo))
        .collect::<std::collections::BTreeMap<_, _>>();
    for todo in new {
        let Some(previous) = old_by_id.get(todo.id.as_str()) else {
            continue;
        };
        let semantics_changed = previous.content != todo.content
            || previous.success_criteria != todo.success_criteria
            || previous.dependency_ids != todo.dependency_ids
            || previous.capability_refs != todo.capability_refs
            || previous.expected_artifact_refs != todo.expected_artifact_refs;
        if semantics_changed
            && (todo.status == "completed"
                || todo.verification_status != "pending"
                || !todo.evidence_refs.is_empty()
                || todo.termination_reason.is_some())
        {
            return Err(error(format!(
                "plan step {} changed semantics; clear evidence and return it to pending verification",
                todo.id
            )));
        }
        if todo.attempt < previous.attempt {
            return Err(error(format!(
                "plan step {} cannot decrease its attempt without an explicit replan",
                todo.id
            )));
        }
        if previous
            .artifact_refs
            .iter()
            .any(|reference| !todo.artifact_refs.contains(reference))
            || previous
                .work_step_transaction_refs
                .iter()
                .any(|reference| !todo.work_step_transaction_refs.contains(reference))
        {
            return Err(error(format!(
                "plan step {} cannot discard persisted artifacts or work transactions without an explicit replan",
                todo.id
            )));
        }
    }
    Ok(())
}

fn request_user_input(_args: &Map<String, Value>) -> ToolOutcome {
    error("request_user_input must be admitted by Core as a durable user-input wait; Host cannot create or answer a question".to_string())
}

fn agent_task_status(args: &Map<String, Value>, state: &AgentPlanState) -> ToolOutcome {
    let status = match args.get("status").and_then(Value::as_str).map(str::trim) {
        Some(value)
            if matches!(
                value,
                "in_progress" | "validated" | "blocked" | "failed" | "needs_user"
            ) =>
        {
            value.to_string()
        }
        _ => {
            return error(
                "status must be one of: in_progress, validated, blocked, failed, needs_user"
                    .to_string(),
            )
        }
    };
    let summary = match args.get("summary").and_then(Value::as_str).map(str::trim) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => return error("summary is required".to_string()),
    };
    let mut evidence = match args.get("evidence") {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return error("evidence items must be non-empty strings".to_string());
                };
                out.push(text.to_string());
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("evidence must be a list of strings".to_string()),
    };
    if status == "validated" {
        let runtime_evidence = state.validation_evidence();
        if runtime_evidence.is_empty() {
            if state.mutation_pending_validation() {
                return error(
                    "validated status requires a successful validation command after the latest edit; free-form evidence is not sufficient"
                        .to_string(),
                );
            }
            if evidence.is_empty() {
                return error(
                    "validated read-only status requires concrete tool evidence".to_string(),
                );
            }
        } else {
            evidence = runtime_evidence;
        }
    }
    let next_action = args
        .get("next_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if matches!(status.as_str(), "blocked" | "failed" | "needs_user") && next_action.is_empty() {
        return error(
            "next_action is required for blocked, failed, or needs_user status".to_string(),
        );
    }

    let mut lines = vec![
        format!("agent_task_status: {status}"),
        format!("agent_task_summary: {summary}"),
    ];
    if !evidence.is_empty() {
        lines.push("agent_task_evidence:".to_string());
        for (index, item) in evidence.iter().enumerate() {
            lines.push(format!("{}. {}", index + 1, item));
        }
    }
    if !next_action.is_empty() {
        lines.push(format!("agent_task_next_action: {next_action}"));
    }
    ok(lines.join("\n"), format!("task status: {status}"))
}

fn goal_complete(args: &Map<String, Value>) -> ToolOutcome {
    let data = match nola_core::agent::goal::completion::declare(args) {
        Ok(data) => data, Err(message) => return error(message),
    };
    let status = data["goal_status"].as_str().unwrap();
    let mut lines = vec![format!("goal_complete: {status}"), format!("goal_summary: {}", data["summary"].as_str().unwrap())];
    for (field, heading) in [("evidence", "goal_evidence:"), ("blockers", "goal_blockers:")] {
        let items = data[field].as_array().unwrap();
        if !items.is_empty() { lines.push(heading.into()); }
        lines.extend(items.iter().enumerate().map(|(index, value)| format!("{}. {}", index + 1, value.as_str().unwrap())));
    }
    if let Some(action) = data["next_action"].as_str().filter(|value| !value.is_empty()) {
        lines.push(format!("goal_next_action: {action}"));
    }
    ok(lines.join("\n"), format!("goal status: {status}")).with_data(data)
}

fn normalize_todos(args: &Map<String, Value>) -> Result<Vec<AgentTodo>, ToolOutcome> {
    let todos = match args.get("todos").and_then(Value::as_array) {
        Some(value) => value,
        None => return Err(error("todos must be a list".to_string())),
    };
    let mut in_progress = 0usize;
    let mut normalized = Vec::with_capacity(todos.len());
    let mut ids = std::collections::BTreeSet::new();
    for item in todos {
        let Some(item) = item.as_object() else {
            return Err(error("each todo must be an object".to_string()));
        };
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| error("todo id is required".to_string()))?
            .to_string();
        if !ids.insert(id.clone()) {
            return Err(error("todo ids must be unique".to_string()));
        }
        let content = item
            .get("content")
            .or_else(|| item.get("title"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if content.is_empty() {
            return Err(error("todo content is required".to_string()));
        }
        let active_form = item
            .get("activeForm")
            .or_else(|| item.get("active_form"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("pending")
            .to_string();
        if !matches!(
            status.as_str(),
            "pending" | "in_progress" | "completed" | "cancelled"
        ) {
            return Err(error(
                "status must be pending, in_progress, completed, or cancelled".to_string(),
            ));
        }
        let priority = item
            .get("priority")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("medium")
            .to_string();
        if !matches!(priority.as_str(), "high" | "medium" | "low") {
            return Err(error("priority must be high, medium, or low".to_string()));
        }
        if status == "in_progress" {
            in_progress += 1;
        }
        let success_criteria = match item.get("successCriteria") {
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            error("success criteria must be non-empty strings".to_string())
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => {
                return Err(error(
                    "successCriteria must contain at least one item".to_string(),
                ))
            }
        };
        let string_refs = |key: &str| -> Result<Vec<String>, ToolOutcome> {
            match item.get(key) {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .ok_or_else(|| error(format!("{key} must contain non-empty strings")))
                    })
                    .collect(),
                _ => Err(error(format!("{key} must be an array"))),
            }
        };
        let dependency_ids = string_refs("dependencyIds")?;
        let capability_refs = string_refs("capabilityRefs")?;
        let expected_artifact_refs = string_refs("expectedArtifactRefs")?;
        let optional_string_refs = |key: &str| -> Result<Vec<String>, ToolOutcome> {
            match item.get(key) {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .ok_or_else(|| error(format!("{key} must contain non-empty strings")))
                    })
                    .collect(),
                Some(Value::Null) | None => Ok(Vec::new()),
                _ => Err(error(format!("{key} must be an array"))),
            }
        };
        let artifact_refs = optional_string_refs("artifactRefs")?;
        let work_step_transaction_refs = optional_string_refs("workStepTransactionRefs")?;
        let attempt = item.get("attempt").and_then(Value::as_u64).unwrap_or(0);
        let attempt = u32::try_from(attempt)
            .map_err(|_| error("attempt exceeds the supported range".to_string()))?;
        let optional_ref = |key: &str| -> Result<Option<String>, ToolOutcome> {
            match item.get(key) {
                Some(Value::String(value)) if !value.trim().is_empty() => {
                    Ok(Some(value.trim().to_string()))
                }
                Some(Value::Null) | None => Ok(None),
                _ => Err(error(format!("{key} must be a non-empty string"))),
            }
        };
        let executor_ref = optional_ref("executorRef")?;
        let resume_key = optional_ref("resumeKey")?;
        let approval_status = item
            .get("approvalStatus")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if !matches!(
            approval_status.as_str(),
            "not_required" | "pending" | "approved" | "rejected"
        ) {
            return Err(error("approvalStatus is invalid".to_string()));
        }
        let evidence_refs = match item.get("evidenceRefs") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            error("evidence references must be non-empty strings".to_string())
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Null) | None => Vec::new(),
            _ => return Err(error("evidenceRefs must be a list of strings".to_string())),
        };
        let verification_status = item
            .get("verificationStatus")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("pending")
            .to_string();
        if !matches!(
            verification_status.as_str(),
            "pending" | "passed" | "failed" | "not_required"
        ) {
            return Err(error("verificationStatus is invalid".to_string()));
        }
        let termination_reason = item
            .get("terminationReason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if termination_reason.as_deref().is_some_and(|reason| {
            !matches!(
                reason,
                "success" | "verification_not_applicable" | "cancelled" | "superseded"
            )
        }) {
            return Err(error("terminationReason is invalid".to_string()));
        }
        if status == "completed"
            && (!matches!(verification_status.as_str(), "passed" | "not_required")
                || termination_reason.is_none())
        {
            return Err(error(
                "completed todos require passed/not_required verification and terminationReason"
                    .to_string(),
            ));
        }
        if status == "completed" && verification_status == "passed" && evidence_refs.is_empty() {
            return Err(error(
                "verified completed todos require evidenceRefs".to_string(),
            ));
        }
        if status == "completed"
            && expected_artifact_refs.iter().any(|expected| {
                !artifact_refs.contains(expected) && !evidence_refs.contains(expected)
            })
        {
            return Err(error(
                "completed todos require evidence for every expectedArtifactRef".to_string(),
            ));
        }
        if status == "cancelled" && termination_reason.is_none() {
            return Err(error(
                "cancelled todos require terminationReason".to_string(),
            ));
        }
        if matches!(status.as_str(), "pending" | "in_progress") && termination_reason.is_some() {
            return Err(error(
                "non-terminal todos cannot have terminationReason".to_string(),
            ));
        }
        if matches!(status.as_str(), "in_progress" | "completed")
            && matches!(approval_status.as_str(), "pending" | "rejected")
        {
            return Err(error(
                "active or completed todos require approval".to_string(),
            ));
        }
        if approval_status == "rejected" && status != "cancelled" {
            return Err(error("rejected todos must be cancelled".to_string()));
        }
        normalized.push(AgentTodo {
            id,
            content,
            active_form,
            status,
            priority,
            dependency_ids,
            approval_status,
            capability_refs,
            expected_artifact_refs,
            artifact_refs,
            work_step_transaction_refs,
            attempt,
            executor_ref,
            resume_key,
            success_criteria,
            evidence_refs,
            verification_status,
            termination_reason,
        });
    }
    if in_progress > 1 {
        return Err(error("only one todo may be in_progress".to_string()));
    }
    let statuses = normalized
        .iter()
        .map(|todo| (todo.id.as_str(), todo.status.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for todo in &normalized {
        let mut unique = std::collections::BTreeSet::new();
        for dependency_id in &todo.dependency_ids {
            if dependency_id == &todo.id || !unique.insert(dependency_id.as_str()) {
                return Err(error(
                    "todo dependencies must be unique and cannot reference the same step"
                        .to_string(),
                ));
            }
            let Some(dependency_status) = statuses.get(dependency_id.as_str()) else {
                return Err(error(format!("unknown todo dependency: {dependency_id}")));
            };
            if matches!(todo.status.as_str(), "in_progress" | "completed")
                && *dependency_status != "completed"
            {
                return Err(error(
                    "active or completed todos require completed dependencies".to_string(),
                ));
            }
        }
    }
    fn visit_dependency(
        id: &str,
        dependencies: &std::collections::BTreeMap<String, Vec<String>>,
        visiting: &mut std::collections::BTreeSet<String>,
        visited: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), ToolOutcome> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.to_string()) {
            return Err(error("todo dependencies must be acyclic".to_string()));
        }
        for dependency_id in &dependencies[id] {
            visit_dependency(dependency_id, dependencies, visiting, visited)?;
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        Ok(())
    }
    let dependencies = normalized
        .iter()
        .map(|todo| (todo.id.clone(), todo.dependency_ids.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    for id in dependencies.keys() {
        visit_dependency(id, &dependencies, &mut visiting, &mut visited)?;
    }
    Ok(normalized)
}

fn format_todos(todos: &[AgentTodo]) -> String {
    if todos.is_empty() {
        return "agent_todos: []".to_string();
    }
    let mut lines = vec!["agent_todos:".to_string()];
    for (index, todo) in todos.iter().enumerate() {
        let suffix = if todo.active_form.is_empty() {
            String::new()
        } else {
            format!(" — active: {}", todo.active_form)
        };
        lines.push(format!(
            "{}. [{}] {} (priority: {}, verification: {}){}",
            index + 1,
            todo.status,
            todo.content,
            todo.priority,
            todo.verification_status,
            suffix
        ));
        lines.push(format!("   success: {}", todo.success_criteria.join("; ")));
        lines.push(format!(
            "   approval: {} · dependencies: {}",
            todo.approval_status,
            if todo.dependency_ids.is_empty() {
                "none".to_string()
            } else {
                todo.dependency_ids.join(", ")
            }
        ));
        if !todo.capability_refs.is_empty() {
            lines.push(format!(
                "   capabilities: {}",
                todo.capability_refs.join(", ")
            ));
        }
        if !todo.expected_artifact_refs.is_empty() {
            lines.push(format!(
                "   expected artifacts: {}",
                todo.expected_artifact_refs.join(", ")
            ));
        }
        if !todo.artifact_refs.is_empty() {
            lines.push(format!("   artifacts: {}", todo.artifact_refs.join(", ")));
        }
        if !todo.work_step_transaction_refs.is_empty() {
            lines.push(format!(
                "   work transactions: {}",
                todo.work_step_transaction_refs.join(", ")
            ));
        }
        if todo.attempt > 0 || todo.executor_ref.is_some() || todo.resume_key.is_some() {
            lines.push(format!(
                "   runtime: attempt {} · executor: {} · resume: {}",
                todo.attempt,
                todo.executor_ref.as_deref().unwrap_or("none"),
                todo.resume_key.as_deref().unwrap_or("none")
            ));
        }
        if !todo.evidence_refs.is_empty() {
            lines.push(format!("   evidence: {}", todo.evidence_refs.join(", ")));
        }
    }
    lines.join("\n")
}

fn contains_verify_word(content: &str) -> bool {
    let lower = content.to_lowercase();
    VERIFY_WORDS.iter().any(|word| lower.contains(word))
}

fn propose_agent_plan(args: &Map<String, Value>) -> ToolOutcome {
    let validated_plan = tool_try!(format_agent_plan(args));
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Execution plan");
    let step_count = args
        .get("steps")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    ok(
        format!(
            "{validated_plan}\n\nplan_published: true\nplan_title: {title}\nstep_count: {step_count}\npolicy: Publication is not approval; use runtime permission prompts for permission-sensitive actions.\nnext: Keep update_agent_todos current and verify before the final response."
        ),
        "Plan published".to_string(),
    )
}

fn strip_plan_step_prefix(value: &str) -> String {
    let value = value.trim();
    let value = value
        .strip_prefix("- ")
        .or_else(|| value.strip_prefix("* "))
        .unwrap_or(value);
    let digit_count = value.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return value.to_string();
    }
    let remainder = &value[digit_count..];
    if let Some(remainder) = remainder
        .strip_prefix('.')
        .or_else(|| remainder.strip_prefix(')'))
    {
        let remainder = remainder.trim_start();
        if !remainder.is_empty() {
            return remainder.to_string();
        }
    }
    value.to_string()
}

fn seed_agent_todos_from_plan(
    args: &Map<String, Value>,
    state: &AgentPlanState,
) -> Result<usize, String> {
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "steps must contain at least one item".to_string())?
        .iter()
        .filter_map(Value::as_str)
        .map(strip_plan_step_prefix)
        .filter(|step| !step.is_empty())
        .collect::<Vec<_>>();
    let todos = steps
        .iter()
        .enumerate()
        .map(|(index, content)| AgentTodo {
            id: format!("plan-step-{}", index + 1),
            content: content.clone(),
            active_form: if index == 0 {
                content.clone()
            } else {
                String::new()
            },
            status: if index == 0 {
                "in_progress".to_string()
            } else {
                "pending".to_string()
            },
            priority: if index == 0 {
                "high".to_string()
            } else {
                "medium".to_string()
            },
            dependency_ids: if index == 0 {
                Vec::new()
            } else {
                vec![format!("plan-step-{index}")]
            },
            approval_status: "not_required".to_string(),
            capability_refs: Vec::new(),
            expected_artifact_refs: Vec::new(),
            artifact_refs: Vec::new(),
            work_step_transaction_refs: Vec::new(),
            attempt: if index == 0 { 1 } else { 0 },
            executor_ref: None,
            resume_key: None,
            success_criteria: vec![format!("The planned outcome is observable: {content}")],
            evidence_refs: Vec::new(),
            verification_status: "pending".to_string(),
            termination_reason: None,
        })
        .collect::<Vec<_>>();
    let count = todos.len();
    state
        .replace_todos(todos)
        .map_err(|err| format!("persist plan checklist failed: {err}"))?;
    Ok(count)
}

fn propose_agent_plan_with_state(args: &Map<String, Value>, state: &AgentPlanState) -> ToolOutcome {
    let mut outcome = propose_agent_plan(args);
    if outcome.error {
        return outcome;
    }
    match seed_agent_todos_from_plan(args, state) {
        Ok(count) => {
            outcome.text.push_str(&format!(
                "\n\nagent_todos_seeded: true\nagent_todo_count: {count}\nnext_action: continue with the first work item; do not call update_agent_todos merely to initialize the same checklist."
            ));
            outcome.display = format!("Plan published · {count} steps");
            outcome
        }
        Err(err) => error(err),
    }
}

fn format_agent_plan(args: &Map<String, Value>) -> Result<String, ToolOutcome> {
    let title = arg_string(args, "title").unwrap_or_else(|| "Execution Plan".to_string());
    let summary = arg_string(args, "summary")
        .or_else(|| arg_string(args, "plan"))
        .ok_or_else(|| error("summary is required".to_string()))?;
    let steps_value = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| error("steps must contain at least one item".to_string()))?;
    let steps = steps_value
        .iter()
        .filter_map(Value::as_str)
        .map(strip_plan_step_prefix)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return Err(error("steps must contain at least one item".to_string()));
    }
    if steps.len() > MAX_PLAN_STEPS {
        return Err(error(format!(
            "steps must contain at most {MAX_PLAN_STEPS} items"
        )));
    }
    let risks = match args.get("risks") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| error("risks must be a list".to_string()))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    let verification = arg_string(args, "verification").unwrap_or_default();

    let mut lines = vec![format!("# {title}"), String::new(), summary];
    lines.extend([String::new(), "steps:".to_string()]);
    lines.extend(
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| format!("{}. {step}", index + 1)),
    );
    if !risks.is_empty() {
        lines.extend([String::new(), "risks:".to_string()]);
        lines.extend(risks.iter().map(|risk| format!("- {risk}")));
    }
    if !verification.is_empty() {
        lines.extend([String::new(), format!("verification: {verification}")]);
    }
    let plan = lines.join("\n").trim().to_string();
    if plan.len() > MAX_PLAN_CHARS {
        return Err(error(format!(
            "plan is too long (max {MAX_PLAN_CHARS} chars)"
        )));
    }
    Ok(plan)
}

fn agent_plan_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let action = arg_string(args, "action").unwrap_or_else(|| "list".to_string());
    if !matches!(action.as_str(), "list" | "read" | "write") {
        return error("action must be list, read, or write".to_string());
    }
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let plans_dir = root.join(".nola").join("plans");
    if action == "list" {
        let mut plans_by_slug = HashMap::new();
        for directory in [&plans_dir] {
            if directory.is_dir() {
                let entries = match fs::read_dir(directory) {
                    Ok(value) => value,
                    Err(err) => {
                        return error(format!(
                            "Read plans directory {} failed: {err}",
                            directory.display()
                        ))
                    }
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|value| value.to_str()) == Some("md") {
                        if let Some(slug) = path.file_stem().and_then(|value| value.to_str()) {
                            plans_by_slug.insert(slug.to_string(), path);
                        }
                    }
                }
            }
        }
        let mut plans = plans_by_slug.into_values().collect::<Vec<_>>();
        plans.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
        let mut lines = vec![format!("# Agent plan files: {}", plans_dir.display())];
        if plans.is_empty() {
            lines.push("(no plan files found)".to_string());
        }
        for path in plans.iter().take(100) {
            let rel = path
                .strip_prefix(&root)
                .map(path_slash)
                .unwrap_or_else(|_| path_slash(path));
            let digest = tool_try!(file_sha256_hex(path));
            let bytes = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
            let slug = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("plan");
            lines.push(format!("- {slug} path={rel} sha256={digest} bytes={bytes}"));
        }
        if plans.len() > 100 {
            lines.push("...[plan list truncated]".to_string());
        }
        return ok(lines.join("\n"), format!("Agent plans ({})", plans.len()));
    }

    let slug = tool_try!(plan_file_slug(args));
    let primary_path = plans_dir.join(format!("{slug}.md"));
    let path = primary_path;
    if !path.starts_with(&root) {
        return error("plan path escapes root".to_string());
    }
    let rel = path
        .strip_prefix(&root)
        .map(path_slash)
        .unwrap_or_else(|_| path_slash(&path));
    if action == "read" {
        if !path.is_file() {
            return error(format!("plan file not found: {rel}"));
        }
        let content = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(err) => return error(format!("Read failed: {err}")),
        };
        return ok(
            format!(
                "# Agent plan: {slug}\npath: {rel}\n\n{}",
                content.trim_end()
            ),
            format!("Read plan {slug}"),
        );
    }

    let mut content = match args.get("content").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        _ => return error("content is required for action=write".to_string()),
    };
    if content.len() > 100_000 {
        return error("content exceeds 100000 bytes".to_string());
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    let dry_run = tool_try!(bool_arg(args, "dry_run", true));
    if path.exists() {
        tool_try!(check_expected_sha256(&path, args));
    } else if let Err(outcome) = expected_sha256(args) {
        return outcome;
    }
    let before = if path.exists() {
        match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(err) => return error(format!("Read failed: {err}")),
        }
    } else {
        String::new()
    };
    let diff = format_unified_diff(&path, &before, &content, MAX_DIFF_CHARS);
    if dry_run {
        let digest = if path.exists() {
            tool_try!(file_sha256_hex(&path))
        } else {
            "(new file)".to_string()
        };
        return ok(
            format!(
                "# Agent plan write preview: {slug}\ndry_run: true\npath: {rel}\nexists: {}\nsha256: {digest}\n\n## Diff\n{diff}",
                path.exists()
            ),
            format!("Plan write preview {slug}"),
        );
    }
    if before == content {
        return ok_data(json!({
            "path": rel,
            "sha256": tool_try!(file_sha256_hex(&path)),
            "changed": false,
            "message": "Plan already contains this content; no file was written. Continue the task instead of repeating this write."
        }), format!("Plan unchanged {slug}"));
    }
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return error(format!("Create plan directory failed: {err}"));
        }
    }
    if let Err(err) = fs::write(&path, content) {
        return error(format!("Write failed: {err}"));
    }
    let digest = tool_try!(file_sha256_hex(&path));
    ok(
        format!("Wrote agent plan {slug}\npath: {rel}\nsha256: {digest}\n\n## Diff\n{diff}"),
        format!("Wrote plan {slug}"),
    )
}

fn plan_file_slug(args: &Map<String, Value>) -> Result<String, ToolOutcome> {
    let mut slug = required_string(args, "slug")?;
    if let Some(stripped) = slug.strip_suffix(".md") {
        slug = stripped.to_string();
    }
    if slug.is_empty()
        || slug.len() > 80
        || !slug.chars().enumerate().all(|(index, ch)| {
            ch.is_ascii_alphanumeric() || (index > 0 && matches!(ch, '.' | '_' | '-'))
        })
    {
        return Err(error(
            "slug must be 1-80 chars: letters, numbers, dot, underscore, or dash".to_string(),
        ));
    }
    Ok(slug)
}

// JSON Schema maxLength counts Unicode scalar values, not UTF-8 bytes.
const BASH_COMMAND_MAX_CHARS: usize = 4000;

fn bash_command_too_long(command: &str) -> bool {
    command.chars().count() > BASH_COMMAND_MAX_CHARS
}

fn bash_command_length_error(command: &str) -> ToolOutcome {
    let actual = command.chars().count();
    let message = format!("Command has {actual} Unicode characters; limit is {BASH_COMMAND_MAX_CHARS}, including wrapper and quoting. Split file writes into smaller heredoc chunks (first >, then >>), or use an authorized file-edit tool. No command was executed.");
    error_data_with_display(json!({
        "error": "command_too_long",
        "tool_execution_skipped": true,
        "actual_characters": actual,
        "max_characters": BASH_COMMAND_MAX_CHARS,
        "message": message
    }), message.clone())
}

fn bash_command_review(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let command = tool_try!(required_string(args, "command"));
    if bash_command_too_long(&command) {
        return bash_command_length_error(&command);
    }
    if let Some(cwd) = args
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        match resolve_allowed(cwd, &ctx.allowed_dirs, true) {
            Ok(path) if path.is_dir() => {}
            Ok(path) => return error(format!("cwd is not a directory: {}", path.display())),
            Err(outcome) => return outcome,
        }
    }
    let (allowed_by_prefix, head) = bash_prefix_allowed(&command, ctx.policy.bash_prefixes());
    let mut flags = Vec::new();
    let forbidden = forbidden_shell_syntax_tokens(&command);
    let permissive_sandbox = sandbox_bash_permissive_enabled();
    let parts = command.split_whitespace().collect::<Vec<_>>();
    if !forbidden.is_empty() && permissive_sandbox {
        flags.push(format!(
            "shell_syntax_allowed_by_sandbox: {}",
            forbidden.join(", ")
        ));
    } else if !forbidden.is_empty() {
        flags.push(format!(
            "approval_required: shell syntax ({})",
            forbidden.join(", ")
        ));
    }
    if permissive_sandbox && is_blocked_sandbox_git_command(&parts) {
        flags.push(
            "not_runnable: mutating or network git command is disabled in sandbox-permissive bash mode"
                .to_string(),
        );
    }
    if ctx.policy.bash_prefixes().is_empty() {
        flags.push("approval_required: no bash prefixes configured".to_string());
    } else if !allowed_by_prefix {
        flags.push(format!(
            "approval_required: '{head}' does not match configured bash prefixes"
        ));
    }
    if DESTRUCTIVE_HEADS.contains(&head.as_str()) {
        flags.push("destructive: can remove or terminate local state".to_string());
    }
    if head == "git"
        && parts
            .iter()
            .skip(1)
            .any(|part| matches!(*part, "reset" | "clean" | "checkout" | "restore"))
    {
        flags.push("destructive_git: can discard worktree changes".to_string());
    }
    if head == "git"
        && parts
            .iter()
            .skip(1)
            .any(|part| matches!(*part, "push" | "fetch" | "pull" | "clone"))
    {
        flags.push("network: git command may contact remotes".to_string());
    }
    if NETWORK_HEADS.contains(&head.as_str())
        && parts.iter().skip(1).any(|part| {
            matches!(
                *part,
                "install" | "add" | "update" | "fetch" | "pull" | "push" | "clone"
            )
        })
    {
        flags.push(
            "network_or_dependency_change: may download, upload, or alter dependencies".to_string(),
        );
    }
    if WRITE_COMMANDS.contains(&head.as_str()) {
        flags.push(
            "may_write_files: command commonly creates, modifies, or builds files".to_string(),
        );
    }
    if READ_ONLY_COMMANDS.contains(&head.as_str())
        && !flags.iter().any(|flag| {
            flag.starts_with("destructive")
                || flag.starts_with("may_write")
                || flag.starts_with("network")
        })
    {
        flags.push(
            "read_only_candidate: command is commonly read-only, subject to arguments".to_string(),
        );
    }
    let risk_level = if flags
        .iter()
        .any(|flag| flag.starts_with("blocked") || flag.starts_with("destructive"))
    {
        "high"
    } else if flags.iter().any(|flag| {
        flag.starts_with("may_write")
            || flag.starts_with("network")
            || flag.starts_with("approval_required")
    }) {
        "medium"
    } else if flags.len() == 1 && flags[0].starts_with("read_only_candidate") {
        "low"
    } else {
        "unknown"
    };
    let mut lines = vec![
        "# Bash command review".to_string(),
        format!("command: {command}"),
        format!("head: {}", if head.is_empty() { "(none)" } else { &head }),
        format!("allowed_by_prefix: {allowed_by_prefix}"),
        format!("risk_level: {risk_level}"),
        String::new(),
        "flags:".to_string(),
    ];
    if flags.is_empty() {
        lines.push("- (none)".to_string());
    } else {
        lines.extend(flags.iter().map(|flag| format!("- {flag}")));
    }
    if permissive_sandbox && is_blocked_sandbox_git_command(&parts) {
        lines.extend([
            String::new(),
            "verdict: bash will reject mutating or network git commands in sandbox-permissive mode; use read-only git commands instead."
                .to_string(),
        ]);
    } else if !forbidden.is_empty() && !permissive_sandbox {
        lines.extend([
            String::new(),
            "verdict: agent runtime will ask for approval before running shell syntax. Prefer passing cwd separately and using a simple command when possible."
                .to_string(),
        ]);
    } else if !allowed_by_prefix {
        lines.extend([
            String::new(),
            "verdict: agent runtime will ask for approval before running this command. Non-interactive runs need --yes or a matching bash_prefixes config entry."
                .to_string(),
        ]);
    } else if risk_level == "high" {
        lines.extend([
            String::new(),
            "verdict: high-risk command; prefer a dry-run or dedicated file/git tool first."
                .to_string(),
        ]);
    } else {
        lines.extend([
            String::new(),
            "verdict: review the flags, then run only if the command is necessary and scoped."
                .to_string(),
        ]);
    }
    ok(lines.join("\n"), format!("Bash review: {risk_level}"))
}

fn bash_prefix_allowed(command: &str, prefixes: &[String]) -> (bool, String) {
    let head = command.split_whitespace().next().unwrap_or("").to_string();
    let allowed = !prefixes.is_empty()
        && prefixes
            .iter()
            .any(|prefix| bash_command_matches_scope(command, prefix));
    (allowed, head)
}

fn bash_command_matches_scope(command: &str, scope: &str) -> bool {
    let Some(command_segments) = parse_read_only_shell_segments(command) else {
        return command.trim() == scope.trim();
    };
    let Some(scope_segments) = parse_read_only_shell_segments(scope) else {
        return command.trim() == scope.trim();
    };
    if command_segments.len() != 1 || scope_segments.len() != 1 {
        return command.trim() == scope.trim();
    }
    let command_tokens = &command_segments[0];
    let scope_tokens = &scope_segments[0];
    !scope_tokens.is_empty()
        && command_tokens.len() >= scope_tokens.len()
        && command_tokens[..scope_tokens.len()] == scope_tokens[..]
}

fn bash_command_segments_match_scopes(command: &str, scopes: &[String]) -> bool {
    let Some(segments) = parse_read_only_shell_segments(command) else {
        return false;
    };
    !segments.is_empty()
        && segments.iter().all(|tokens| {
            let segment = tokens.join(" ");
            scopes
                .iter()
                .any(|scope| bash_command_matches_scope(&segment, scope))
        })
}

fn bash_session_command_scope(command: &str) -> String {
    command.trim().to_owned()
}


fn is_blocked_sandbox_git_command_from_str(command: &str) -> bool {
    let Some(segments) = parse_read_only_shell_segments(command) else {
        return true;
    };
    segments.iter().any(|segment| {
        let parts = segment.iter().map(String::as_str).collect::<Vec<_>>();
        is_blocked_sandbox_git_command(&parts)
    })
}

fn is_blocked_sandbox_git_command(parts: &[&str]) -> bool {
    if parts.first().copied() != Some("git") {
        return false;
    }
    let subcommand = parts
        .iter()
        .skip(1)
        .find(|part| !part.starts_with('-'))
        .copied()
        .unwrap_or("");
    matches!(
        subcommand,
        "add"
            | "am"
            | "apply"
            | "bisect"
            | "branch"
            | "checkout"
            | "cherry-pick"
            | "clean"
            | "clone"
            | "commit"
            | "fetch"
            | "gc"
            | "init"
            | "merge"
            | "mv"
            | "pull"
            | "push"
            | "rebase"
            | "remote"
            | "reset"
            | "restore"
            | "revert"
            | "rm"
            | "stash"
            | "submodule"
            | "switch"
            | "tag"
            | "worktree"
    )
}

fn is_validation_command_like(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return false;
    }

    fn is_validation_shell_separator(token: &str) -> bool {
        matches!(token, "&&" | "||" | ";" | "|" | "&")
    }

    fn is_validation_env_prefix(token: &str) -> bool {
        let Some((name, _)) = token.split_once('=') else {
            return false;
        };
        if name.is_empty() {
            return false;
        }
        let mut name_chars = name.chars();
        let Some(first) = name_chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() && first != '_' {
            return false;
        }
        name_chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    }

    fn skip_validation_env_prefixes(tokens: &[&str], mut index: usize) -> usize {
        while index < tokens.len() {
            if !is_validation_env_prefix(tokens[index]) {
                break;
            }
            index += 1;
        }
        index
    }

    fn is_validation_command_tokens(tokens: &[&str], start: usize) -> bool {
        if start >= tokens.len() {
            return false;
        }
        let command_head = tokens[start];
        match command_head {
            "pytest" | "tox" | "nox" => true,
            "cargo" => tokens
                .get(start + 1)
                .is_some_and(|token| matches!(*token, "test" | "check" | "build")),
            "go" => tokens.get(start + 1).is_some_and(|token| *token == "test"),
            "npm" | "pnpm" | "yarn" => tokens
                .get(start + 1)
                .is_some_and(|token| matches!(*token, "test" | "run")),
            "make" => tokens
                .get(start + 1)
                .is_some_and(|token| token.contains("test") || token.contains("check")),
            "gradle" | "./gradlew" | "mvn" => tokens
                .get(start + 1)
                .is_some_and(|token| token.contains("test") || token == &"check"),
            "swift" => tokens.get(start + 1).is_some_and(|token| *token == "test"),
            "xcodebuild" => tokens[start..].contains(&"test"),
            "-m" => tokens[start..]
                .iter()
                .position(|token| *token == "pytest" || *token == "unittest")
                .is_some(),
            head if head.starts_with("python") => {
                let arguments = &tokens[start + 1..];
                arguments.contains(&"--validate-only")
                    || arguments
                        .iter()
                        .position(|token| *token == "-m")
                        .and_then(|index| arguments.get(index + 1))
                        .is_some_and(|token| *token == "pytest" || *token == "unittest")
                    || arguments.iter().any(|token| {
                        let basename = token.rsplit('/').next().unwrap_or(token);
                        basename.starts_with("validate_")
                            || basename.starts_with("verify_")
                            || basename.ends_with("_validator.py")
                    })
            }
            "poetry" | "uv" | "coverage" => {
                if start + 1 >= tokens.len() || tokens[start + 1] != "run" {
                    return false;
                }
                is_validation_command_tokens(tokens, start + 2)
            }
            _ => false,
        }
    }

    fn is_validation_segment_command(tokens: &[&str]) -> bool {
        if tokens.is_empty() {
            return false;
        }

        let mut index = 0usize;
        loop {
            index = skip_validation_env_prefixes(tokens, index);
            if index >= tokens.len() {
                return false;
            }
            match tokens[index] {
                "cd" | "source" | "." => {
                    return false;
                }
                "export" => {
                    index += 1;
                    if index >= tokens.len() {
                        return false;
                    }
                    let next = skip_validation_env_prefixes(tokens, index);
                    if next >= tokens.len() {
                        return false;
                    }
                    index = next;
                }
                "set" | "env" => {
                    index += 1;
                    if index >= tokens.len() {
                        return false;
                    }
                    index = skip_validation_env_prefixes(tokens, index);
                    if index >= tokens.len() {
                        return false;
                    }
                    return is_validation_command_tokens(tokens, index);
                }
                _ => return is_validation_command_tokens(tokens, index),
            }
        }
    }

    let mut segment_start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if !is_validation_shell_separator(token) {
            continue;
        }
        if is_validation_segment_command(&tokens[segment_start..index]) {
            return true;
        }
        segment_start = index + 1;
    }
    is_validation_segment_command(&tokens[segment_start..])
}

fn is_validation_bash_chain_command(command: &str) -> bool {
    is_validation_command_like(command)
}

fn validation_pipeline_hides_result(command: &str) -> bool {
    if !is_validation_command_like(command) {
        return false;
    }
    let pipefail_enabled = command
        .split(|character| matches!(character, ';' | '\n'))
        .take_while(|segment| !segment.contains('|'))
        .any(|segment| {
            let normalized = segment.split_whitespace().collect::<Vec<_>>().join(" ");
            normalized == "set -o pipefail"
                || normalized == "set -e -o pipefail"
                || normalized == "set -u -o pipefail"
                || normalized == "set -eu -o pipefail"
                || normalized == "set -euo pipefail"
        });
    if pipefail_enabled {
        return false;
    }
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    tokens.windows(2).any(|pair| {
        if pair[0] != "|" {
            return false;
        }
        let consumer = pair[1]
            .trim_matches(|character: char| character == '\'' || character == '"')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        matches!(consumer, "head" | "tail")
    })
}

fn allowed_operators_for_validation_chain(tokens: &[&'static str]) -> bool {
    tokens
        .iter()
        .all(|token| matches!(token.trim(), ";" | "&&" | "||" | "|" | ">" | "<"))
}

fn is_validation_bash_command_allowed(command: &str) -> bool {
    let forbidden = forbidden_shell_syntax_tokens(command);
    if forbidden.is_empty() {
        return false;
    }
    if !allowed_operators_for_validation_chain(&forbidden) {
        return false;
    }
    is_validation_bash_chain_command(command)
}

fn forbidden_shell_syntax_tokens(command: &str) -> Vec<&'static str> {
    let bytes = command.as_bytes();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut found = Vec::new();

    let mut push_once = |token: &'static str| {
        if !found.contains(&token) {
            found.push(token);
        }
    };

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match ch {
            '\'' => {
                in_single = true;
                i += 1;
            }
            '"' => {
                in_double = true;
                i += 1;
            }
            '\\' => {
                i += 2;
            }
            ';' => {
                push_once(";");
                i += 1;
            }
            '`' => {
                push_once("`");
                i += 1;
            }
            '\n' => {
                push_once("\\n");
                i += 1;
            }
            '\r' => {
                push_once("\\r");
                i += 1;
            }
            '|' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    push_once("||");
                    i += 2;
                } else {
                    push_once("|");
                    i += 1;
                }
            }
            '&' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                    push_once("&&");
                    i += 2;
                } else {
                    push_once("&");
                    i += 1;
                }
            }
            '$' => {
                if i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    if next == b'(' {
                        push_once("$(");
                        i += 2;
                        continue;
                    }
                    if next == b'{' {
                        push_once("${");
                        i += 2;
                        continue;
                    }
                    if next == b'[' {
                        push_once("$[");
                        i += 2;
                        continue;
                    }
                }
                i += 1;
            }
            '>' => {
                push_once(">");
                i += 1;
            }
            '<' => {
                push_once("<");
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    found
}

fn test_folder_permissive_enabled() -> bool {
    matches!(
        env::var("NOLA_TEST_FOLDER_PERMISSIVE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
    )
}

fn sandbox_bash_permissive_enabled() -> bool {
    matches!(
        env::var("NOLA_CLI_SANDBOX_BASH_PERMISSIVE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
    )
}

fn bash_timeout_max_sec() -> u64 {
    env::var("NOLA_CLI_BASH_TIMEOUT_MAX_SEC")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(1_200)
        .clamp(1, 86_400)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BashExecutionMode {
    Auto,
    Foreground,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DeclaredBashOutput {
    path: PathBuf,
    operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BackgroundOutputContract {
    declared: Vec<DeclaredBashOutput>,
    before: FileArtifactStates,
    settlement_transaction_id: String,
    settled: bool,
    artifacts: Vec<nola_core::agent::host::HostArtifact>,
    error: Option<String>,
}

fn prepare_background_output_contract(
    task_id: &str,
    declared: &[DeclaredBashOutput],
    before: &FileArtifactStates,
    ctx: &ToolContext,
) -> Result<Option<BackgroundOutputContract>, ToolOutcome> {
    if declared.is_empty() {
        return Ok(None);
    }
    let settlement_request = ToolRequest {
        id: format!("{task_id}:output-settlement"),
        name: "bash_background_output_settlement".to_string(),
        args: Map::from_iter([(
            "output_artifacts".to_string(),
            serde_json::to_value(declared).unwrap_or(Value::Null),
        )]),
    };
    let transaction = prepare_effect_work_step_transaction(
        &settlement_request,
        ctx,
        nola_core::agent::host::HostCapabilityEffect::MutateAndValidate,
        nola_core::agent::host::HostCapabilityTransactionBoundary::Workspace,
    )?
    .ok_or_else(|| {
        error_with_display(
            "Background Bash output settlement did not create a durable transaction".to_string(),
            "Background command was not started because output recovery is unavailable"
                .to_string(),
        )
    })?;
    Ok(Some(BackgroundOutputContract {
        declared: declared.to_vec(),
        before: before.clone(),
        settlement_transaction_id: transaction.id,
        settled: false,
        artifacts: Vec::new(),
        error: None,
    }))
}

fn persist_background_output_contract(
    task_id: &str,
    contract: &BackgroundOutputContract,
) -> Result<(), String> {
    let encoded = serde_json::to_string(contract)
        .map_err(|error| format!("encode background output contract failed: {error}"))?;
    let status = if !contract.settled {
        "pending"
    } else if contract.error.is_some() {
        "failed"
    } else {
        "completed"
    };
    crate::runtime_task_port::append_event(
        task_id,
        "output_contract",
        status,
        Some("Background output contract settled".to_string()),
        encoded,
        unix_epoch_millis(),
    )
    .map(|_| ())
}

fn settle_background_output_contract(
    task: &mut BackgroundBashTask,
    _status: &str,
) -> Option<ToolOutcome> {
    // Output settlement is a terminal effect, never a consequence of a label,
    // PID disappearance, status file, timeout or failed database observation.
    let observation = match observe_background_task(task) {
        Ok(observation) if observation.terminal => observation,
        Ok(_) => return None,
        Err(error) => return Some(foreground_task_observation_error(&task.id, "observe", error)),
    };
    let status = observation.label.as_str();
    let mut contract = task.output_contract.clone()?;
    if contract.settled {
        let mut outcome = ok(
            format!(
                "Background output contract settled for {}",
                task.id
            ),
            format!("Background outputs settled for {}", task.id),
        );
        outcome.artifacts = contract.artifacts;
        if let Some(error_text) = contract.error {
            outcome.error = true;
            outcome.text = error_text;
            outcome.display = format!("Background output contract failed for {}", task.id);
        }
        return Some(outcome);
    }

    let desired = if observation.state != "completed" {
        error_with_display(
            format!(
                "Background command {} ended with {status}; declared outputs were not committed",
                task.id
            ),
            format!("Background output contract failed for {}", task.id),
        )
    } else {
        let paths = contract
            .declared
            .iter()
            .map(|output| output.path.as_path())
            .collect::<Vec<_>>();
        match capture_file_artifact_states(&paths) {
            Err(outcome) => outcome,
            Ok(after) => {
                let mismatch = contract.declared.iter().find_map(|declared| {
                    let observed = observed_file_artifact_operation(
                        &declared.path,
                        &contract.before,
                        &after,
                    );
                    (observed != Some(declared.operation.as_str())).then(|| {
                        format!(
                            "Background Bash output artifact postcondition failed for {}: declared={}, observed={}",
                            declared.path.display(),
                            declared.operation,
                            observed.unwrap_or("unchanged")
                        )
                    })
                });
                if let Some(mismatch) = mismatch {
                    error_with_display(
                        mismatch,
                        format!("Background output contract failed for {}", task.id),
                    )
                } else {
                    ok(
                        format!("Background output contract settled for {}", task.id),
                        format!("Background outputs settled for {}", task.id),
                    )
                    .with_file_artifact_changes(&contract.before, &after)
                }
            }
        }
    };

    let transaction = match crate::work_step_port::work_step_transaction(
        &contract.settlement_transaction_id,
    ) {
        Ok(Some(transaction)) => transaction,
        Ok(None) => {
            return Some(error_with_display(
                format!(
                    "Background output settlement transaction is missing: {}",
                    contract.settlement_transaction_id
                ),
                "Background outputs require recovery".to_string(),
            ))
        }
        Err(store_error) => {
            return Some(error_with_display(
                format!("Load background output settlement failed: {store_error}"),
                "Background outputs require recovery".to_string(),
            ))
        }
    };
    let finalized = match transaction.phase {
        nola_application::WorkStepTransactionPhase::Checkpointed => {
            finish_tool_work_step_transaction(Some(transaction), desired)
        }
        nola_application::WorkStepTransactionPhase::EffectObserved => {
            let replay = transaction
                .result
                .as_ref()
                .and_then(tool_outcome_from_transaction)
                .unwrap_or_else(|| {
                    error_with_display(
                        "Observed background output settlement has no replayable result".to_string(),
                        "Background outputs require recovery".to_string(),
                    )
                });
            let phase = if replay.error {
                nola_application::WorkStepTransactionPhase::Failed
            } else {
                nola_application::WorkStepTransactionPhase::Committed
            };
            let terminal_error = replay.error.then(|| replay.text.clone());
            match crate::work_step_port::advance_work_step_transaction(
                &transaction.id,
                transaction.revision,
                phase,
                transaction.workspace_checkpoint_id,
                transaction.result,
                transaction.result_ref,
                terminal_error,
            ) {
                Ok(_) => replay,
                Err(store_error) => error_with_display(
                    format!("Commit observed background outputs failed: {store_error}"),
                    "Background outputs require recovery".to_string(),
                ),
            }
        }
        nola_application::WorkStepTransactionPhase::Committed
        | nola_application::WorkStepTransactionPhase::Failed => transaction
            .result
            .as_ref()
            .and_then(tool_outcome_from_transaction)
            .unwrap_or_else(|| {
                error_with_display(
                    "Terminal background output settlement has no replayable result".to_string(),
                    "Background outputs require recovery".to_string(),
                )
            }),
        nola_application::WorkStepTransactionPhase::Prepared => error_with_display(
            "Background output settlement was not checkpointed".to_string(),
            "Background outputs require recovery".to_string(),
        ),
    };
    contract.settled = matches!(
        crate::work_step_port::work_step_transaction(&contract.settlement_transaction_id)
            .ok()
            .flatten()
            .map(|transaction| transaction.phase),
        Some(
            nola_application::WorkStepTransactionPhase::Committed
                | nola_application::WorkStepTransactionPhase::Failed
        )
    );
    contract.artifacts = finalized.artifacts.clone();
    contract.error = finalized.error.then(|| finalized.text.clone());
    task.output_contract = Some(contract.clone());
    if let Err(store_error) = persist_background_output_contract(&task.id, &contract) {
        return Some(error_with_display(
            format!("Persist background output settlement failed: {store_error}"),
            "Background outputs were observed but require recovery".to_string(),
        ));
    }
    Some(finalized)
}

fn declared_bash_outputs(
    args: &Map<String, Value>,
    cwd: &Path,
    allowed_dirs: &[PathBuf],
) -> Result<Vec<DeclaredBashOutput>, ToolOutcome> {
    let Some(value) = args.get("output_artifacts") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(error("output_artifacts must be a list".to_string()));
    };
    if items.len() > 100 {
        return Err(error("output_artifacts exceeds 100 resources".to_string()));
    }
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut outputs = Vec::with_capacity(items.len());
    for item in items {
        let Some(item) = item.as_object() else {
            return Err(error(
                "output_artifacts entries must be objects".to_string(),
            ));
        };
        let Some(raw_path) = item
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            return Err(error(
                "output_artifacts entry path is required".to_string(),
            ));
        };
        let operation = item
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(operation, "create" | "edit" | "delete") {
            return Err(error(
                "output_artifacts operation must be create, edit, or delete".to_string(),
            ));
        }
        let requested = if Path::new(raw_path).is_absolute() {
            PathBuf::from(raw_path)
        } else {
            cwd.join(raw_path)
        };
        let path = resolve_allowed_entry(
            requested.to_string_lossy().as_ref(),
            allowed_dirs,
        )?;
        // cwd resolves relative operands; it is not the filesystem authority.
        // Each exact output was admitted above against the current resource
        // grants. Observation and recovery inventory only these declarations,
        // including authorized siblings and targets in a second granted root.
        if outputs
            .iter()
            .any(|output: &DeclaredBashOutput| output.path == path)
        {
            return Err(error(format!(
                "Duplicate output_artifacts path: {}",
                path.display()
            )));
        }
        outputs.push(DeclaredBashOutput {
            path,
            operation: operation.to_string(),
        });
    }
    Ok(outputs)
}

fn observed_file_artifact_operation(
    path: &Path,
    before: &FileArtifactStates,
    after: &FileArtifactStates,
) -> Option<&'static str> {
    match (before.get(path), after.get(path)) {
        (None, Some(_)) => Some("create"),
        (Some(_), None) => Some("delete"),
        (Some(left), Some(right)) if left != right => Some("edit"),
        _ => None,
    }
}

#[cfg(test)]
mod declared_shell_resource_tests {
    use super::*;
    include!("../test_parts/declared_shell_resources.rs");
}

fn bash_execution_mode(args: &Map<String, Value>) -> Result<BashExecutionMode, ToolOutcome> {
    match args
        .get("execution_mode")
        .and_then(Value::as_str)
        .unwrap_or("foreground")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "auto" => Ok(BashExecutionMode::Auto),
        "foreground" => Ok(BashExecutionMode::Foreground),
        "background" => Ok(BashExecutionMode::Background),
        _ => Err(error(
            "execution_mode must be auto, foreground, or background".to_string(),
        )),
    }
}

fn bash_auto_background_after() -> Duration {
    Duration::from_millis(
        env::var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(2_000)
            .clamp(100, 30_000),
    )
}

#[cfg(test)]
mod background_detection_tests {
    use super::{bash_execution_mode, BashExecutionMode};
    use serde_json::{Map, Value};

    #[test]
    fn execution_mode_is_structured_and_independent_of_command_text() {
        let mut args = Map::new();
        args.insert("command".to_string(), Value::String("anything".to_string()));
        assert_eq!(
            bash_execution_mode(&args).unwrap(),
            BashExecutionMode::Foreground
        );

        args.insert(
            "execution_mode".to_string(),
            Value::String("background".to_string()),
        );
        assert_eq!(
            bash_execution_mode(&args).unwrap(),
            BashExecutionMode::Background
        );

        args.insert(
            "execution_mode".to_string(),
            Value::String("foreground".to_string()),
        );
        assert_eq!(
            bash_execution_mode(&args).unwrap(),
            BashExecutionMode::Foreground
        );
    }
}

#[cfg(test)]
mod background_lifecycle_tests {
    use super::*;

    #[test]
    fn sop_invocation_scope_restores_nested_execution_identity() {
        let outer = nola_core::agent::ToolInvocationContext {
            sop_run_id: "run-outer".to_string(),
            sop_step_id: "step-outer".to_string(),
        };
        let inner = nola_core::agent::ToolInvocationContext {
            sop_run_id: "run-inner".to_string(),
            sop_step_id: "step-inner".to_string(),
        };
        assert!(current_sop_invocation_context().is_none());
        let outer_guard = enter_sop_invocation_scope(Some(outer.clone()));
        assert_eq!(current_sop_invocation_context(), Some(outer.clone()));
        {
            let _inner_guard = enter_sop_invocation_scope(Some(inner.clone()));
            assert_eq!(current_sop_invocation_context(), Some(inner));
        }
        assert_eq!(current_sop_invocation_context(), Some(outer));
        drop(outer_guard);
        assert!(current_sop_invocation_context().is_none());
    }

    fn isolated_context(root: &Path, cancellation: RuntimeExecutionControl) -> ToolContext {
        ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![root.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "deny".to_string(), Vec::new()),
            permission_scope: Default::default(),
            cancellation,
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        }
    }

    #[cfg(any(target_os="macos",target_os="linux"))]
    #[test]
    fn required_process_sandbox_missing_never_executes_foreground_command() {
        let root=tempfile::tempdir().unwrap();let root=root.path().canonicalize().unwrap();
        let mut context=isolated_context(&root,RuntimeExecutionControl::default());
        context.policy=context.policy.with_process_sandbox(Some(crate::process_sandbox::unavailable_process_sandbox_fixture(&root)));
        let marker=root.join("must-not-run");
        let args=json!({"command":format!("printf unsafe > '{}'",marker.display()),"cwd":root,"operation_kind":"execute","execution_mode":"foreground"});
        let outcome=run_bash_execution(args.as_object().unwrap(),&context,true);
        assert!(outcome.error,"{}",outcome.text);
        assert_eq!(outcome.data.as_ref().unwrap()["error"],"required_process_sandbox_unavailable");
        assert_eq!(outcome.data.as_ref().unwrap()["tool_execution_skipped"],true);
        assert!(!marker.exists());
        let refused=run_shell_command(&root,"printf unsafe",&context.cancellation,
            context.policy.process_sandbox()).unwrap_err();
        assert_eq!(refused.data.as_ref().unwrap()["command_not_started"],true);
        assert!(!marker.exists());
    }

    fn temporary_test_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir()
            .join("tests")
            .join(format!("nola-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn task_id_from(outcome: &ToolOutcome) -> String {
        serde_json::from_str::<Value>(&outcome.text).unwrap()["task_id"]
            .as_str()
            .expect("background outcome must expose its canonical task id")
            .to_string()
    }

    #[test]
    fn authorized_shell_can_compose_git_operations_without_a_forced_workflow() {
        let root = tempfile::tempdir().unwrap();
        git_command_output(root.path(), &["init", "--quiet"]).unwrap();
        git_command_output(root.path(), &["config", "user.name", "Fixture"]).unwrap();
        git_command_output(root.path(), &["config", "user.email", "fixture@localhost"]).unwrap();
        git_command_output(root.path(), &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(root.path().join("source.rs"), "fn main() {}\n").unwrap();
        let ctx = isolated_context(root.path(), RuntimeExecutionControl::default());
        let args = json!({"command":"git add -- source.rs && git commit -m fixture", "cwd":root.path(), "operation_kind":"mutate"});
        let outcome = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
        assert!(!outcome.error, "{}", outcome.text);
        assert_eq!(git_commit_text(root.path(), &["show", "HEAD:source.rs"]).unwrap(), "fn main() {}");
        assert!(outcome.execution.as_ref().is_none_or(|receipt| receipt.checks.is_empty()));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn staged_state_is_preserved_even_for_approved_or_mislabeled_shell_calls() {
        let root = tempfile::tempdir().unwrap();
        for args in [vec!["init", "--quiet"], vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@localhost"], vec!["config", "commit.gpgsign", "false"]] {
            git_command_output(root.path(), &args).unwrap();
        }
        fs::write(root.path().join("selected"), "before").unwrap();
        git_command_output(root.path(), &["add", "--", "selected"]).unwrap();
        git_command_output(root.path(), &["commit", "-m", "baseline"]).unwrap();
        fs::write(root.path().join("selected"), "after").unwrap();
        fs::write(root.path().join("user.rs"), "user staged content").unwrap();
        git_command_output(root.path(), &["add", "--", "user.rs"]).unwrap();
        let before = git_index_entries(root.path()).unwrap();
        let head = git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap();
        let ctx = isolated_context(root.path(), RuntimeExecutionControl::default());
        for (command, kind) in [("git commit -m wrong", "mutate"), ("git reset --quiet HEAD -- user.rs", "inspect"),
            ("bash -c 'git commit -m nested'", "execute")] {
            let args = json!({"command":command, "cwd":root.path(), "operation_kind":kind});
            let outcome = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
            assert!(outcome.error, "{}", outcome.text);
            assert!(outcome.activation.as_ref().unwrap().capability_ids.contains(&"git_commit".into()));
            assert_eq!(git_index_entries(root.path()).unwrap(), before);
            assert_eq!(git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap(), head);
        }
        let read = json!({"command":"git status --short", "cwd":root.path(), "operation_kind":"inspect"});
        let outcome = run_bash_with_permission(read.as_object().unwrap(), &ctx, true);
        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.text.contains("user.rs"));
        let deferred = json!({"command":"git commit -m wrong", "cwd":root.path(), "operation_kind":"mutate", "execution_mode":"background"});
        let outcome = run_bash_with_permission(deferred.as_object().unwrap(), &ctx, true);
        assert!(outcome.error);
        assert!(outcome.text.contains("preservation_requires_foreground"));
        let selected = json!({"root":root.path(), "paths":[root.path().join("selected")], "message":"selected only"});
        let outcome = git_commit(selected.as_object().unwrap(), &ctx);
        assert!(!outcome.error, "{}", outcome.text);
        assert_eq!(git_index_entries(root.path()).unwrap()["user.rs"], before["user.rs"]);
        assert_eq!(git_commit_paths(root.path(), &["diff-tree", "--no-commit-id", "--name-only", "-r", "-z", "HEAD"]).unwrap(), ["selected".into()].into());
    }

    #[test]
    fn validation_without_scope_is_rejected_before_execution_even_when_approved() {
        let root = tempfile::tempdir().unwrap();
        let ctx = isolated_context(root.path(), RuntimeExecutionControl::default());
        for kind in ["validate", "mutate_and_validate"] {
            for targets in [Value::Null, json!([]), json!([""]), json!([42])] {
                let args = json!({"command":"touch must-not-exist", "cwd":root.path(),
                    "operation_kind":kind, "validation_targets":targets});
                let outcome = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
                assert!(outcome.error);
                let payload: Value = serde_json::from_str(&outcome.text).unwrap();
                assert_eq!(payload["error"], "validation_scope_required");
                assert_eq!(payload["tool_execution_skipped"], true);
                assert!(!root.path().join("must-not-exist").exists());
            }
        }
    }

    #[test]
    fn auto_mode_promotes_one_process_to_a_durable_background_task() {
        let _environment = crate::store::test_env_lock();
        let root = temporary_test_root("auto-background");
        let db_path = root.join("state").join("nola.db");
        unsafe {
            env::set_var("NOLA_DB_PATH", &db_path);
            env::set_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS", "100");
        }
        let ctx = isolated_context(&root, RuntimeExecutionControl::default());
        let mut args = Map::new();
        args.insert(
            "command".to_string(),
            Value::String("printf begin; sleep 0.3; printf end".to_string()),
        );
        args.insert(
            "execution_mode".to_string(),
            Value::String("auto".to_string()),
        );
        args.insert("_nola_approved_once".to_string(), Value::Bool(true));

        let invocation = nola_core::agent::ToolInvocationContext {
            sop_run_id: "run-background".to_string(),
            sop_step_id: "step-background".to_string(),
        };
        let _sop_scope = enter_sop_invocation_scope(Some(invocation.clone()));
        let outcome = run_bash_with_permission(&args, &ctx, true);
        assert!(!outcome.error, "{}", outcome.text);
        assert_eq!(serde_json::from_str::<Value>(&outcome.text).unwrap()["background_mode"], "automatic_after_grace");
        let task_id = task_id_from(&outcome);
        let store = NolaStore::open(&db_path).unwrap();
        let persisted = store
            .background_task(&task_id)
            .unwrap()
            .expect("promoted task must be durable");
        // Promotion records the admitted task before the asynchronous worker
        // has necessarily published its physical PID (notably on Windows).
        assert_eq!(persisted.sop_invocation, Some(invocation));

        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            let status = manage_background_bash_tasks(
                "status",
                Some(&task_id),
                std::slice::from_ref(&root),
                None,
                &ctx.skills,
            );
            if status.data.as_ref().and_then(|data| data.get("terminal")).and_then(Value::as_bool) == Some(true) || Instant::now() >= deadline {
                break status;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(!status.error, "{}", status.text);
        assert_eq!(status.data.as_ref().unwrap()["state"], "completed", "{}", status.text);
        assert_eq!(status.data.as_ref().unwrap()["terminal"], true);
        assert_eq!(status.data.as_ref().unwrap()["exit_code"], 0, "{}", status.text);
        assert!(status.text.contains("beginend"), "{}", status.text);
        assert!(crate::store::background_task_from_runtime(store.runtime_task(&task_id).unwrap().unwrap()).unwrap().pid > 0,
            "the completed task must retain its actual execution PID");

        unsafe {
            env::remove_var("NOLA_DB_PATH");
            env::remove_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn short_auto_command_returns_foreground_result_from_the_supervised_worker() {
        let _environment = crate::store::test_env_lock();
        let root = temporary_test_root("auto-foreground-completion");
        let db_path = root.join("state").join("nola.db");
        unsafe {
            env::set_var("NOLA_DB_PATH", &db_path);
            // Keep the assertion about foreground completion independent of
            // scheduler contention from other test binaries. The production
            // behavior at the one-second boundary is covered separately by
            // the explicit promotion tests.
            env::set_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS", "5000");
        }
        let ctx = isolated_context(&root, RuntimeExecutionControl::default());
        let mut args = Map::new();
        args.insert(
            "command".to_string(),
            Value::String("printf supervised-quick-result".to_string()),
        );
        args.insert(
            "execution_mode".to_string(),
            Value::String("auto".to_string()),
        );
        args.insert("_nola_approved_once".to_string(), Value::Bool(true));

        let outcome = run_bash_with_permission(&args, &ctx, true);

        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.text.contains("supervised-quick-result"), "{}", outcome.text);
        assert!(!outcome.text.contains("background_mode:"), "{}", outcome.text);
        let tasks = NolaStore::open(&db_path).unwrap().list_all_runtime_tasks().unwrap();
        assert_eq!(tasks.len(), 1, "short auto command bypassed or duplicated supervision");
        assert_eq!(tasks[0].kind, "shell_process");
        assert_eq!(tasks[0].status, "completed");
        let metadata: Value = serde_json::from_str(&tasks[0].metadata_json).unwrap();
        assert!(metadata.get("pid").and_then(Value::as_u64).is_some_and(|pid| pid > 0));
        let scheduler = metadata.get("_nola_scheduler").expect("scheduler evidence");
        assert_eq!(scheduler.get("output_bytes").and_then(Value::as_u64), Some(23));
        assert!(
            scheduler
                .pointer("/resource_usage/peak_processes")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0),
            "short auto command lacks process-tree resource evidence: {metadata}"
        );

        unsafe {
            env::remove_var("NOLA_DB_PATH");
            env::remove_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn foreground_host_request_transfers_without_restarting_the_process() {
        let _environment = crate::store::test_env_lock();
        let root = temporary_test_root("foreground-transfer");
        let db_path = root.join("state").join("nola.db");
        unsafe {
            env::set_var("NOLA_DB_PATH", &db_path);
        }
        let cancellation = RuntimeExecutionControl::default().with_background_transfer();
        assert!(cancellation.request_background());
        let ctx = isolated_context(&root, cancellation);
        let mut args = Map::new();
        args.insert(
            "command".to_string(),
            Value::String("printf before; sleep 1; printf after".to_string()),
        );
        args.insert(
            "execution_mode".to_string(),
            Value::String("foreground".to_string()),
        );
        args.insert("_nola_approved_once".to_string(), Value::Bool(true));

        let outcome = run_bash_with_permission(&args, &ctx, true);
        assert!(!outcome.error, "{}", outcome.text);
        assert_eq!(serde_json::from_str::<Value>(&outcome.text).unwrap()["background_mode"], "user_transfer");
        let task_id = task_id_from(&outcome);
        let persisted = NolaStore::open(&db_path)
            .unwrap()
            .background_task(&task_id)
            .unwrap()
            .expect("transferred task must retain its identity");
        assert_eq!(serde_json::from_str::<Value>(&outcome.text).unwrap()["pid"], persisted.pid);

        let status = bash_background_wait(
            &Map::from_iter([
                ("task_id".to_string(), Value::String(task_id)),
                ("timeout_sec".to_string(), Value::from(5)),
                ("poll_interval_ms".to_string(), Value::from(100)),
            ]),
            &ctx,
        );
        assert!(!status.error, "{}", status.text);
        assert_eq!(status.data.as_ref().unwrap()["state"], "completed", "{}", status.text);
        assert_eq!(status.data.as_ref().unwrap()["terminal"], true);
        assert_eq!(status.data.as_ref().unwrap()["exit_code"], 0, "{}", status.text);
        assert!(status.text.contains("beforeafter"), "{}", status.text);

        unsafe {
            env::remove_var("NOLA_DB_PATH");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completion_notification_is_acknowledged_across_process_reconstruction() {
        let _environment = crate::store::test_env_lock();
        let root = temporary_test_root("notification-restart");
        let db_path = root.join("state").join("nola.db");
        unsafe {
            env::set_var("NOLA_DB_PATH", &db_path);
        }
        let ctx = isolated_context(&root, RuntimeExecutionControl::default());
        let mut args = Map::new();
        args.insert(
            "command".to_string(),
            Value::String("printf done".to_string()),
        );
        args.insert(
            "execution_mode".to_string(),
            Value::String("background".to_string()),
        );
        args.insert("_nola_approved_once".to_string(), Value::Bool(true));
        let outcome = run_bash_with_permission(&args, &ctx, true);
        assert!(!outcome.error, "{}", outcome.text);

        // This is a delivery/reconstruction contract, not a two-second shell
        // or service-start benchmark. Await the actual durable terminal fact
        // before asserting notification delivery; status reads must not ACK it.
        let result: Value = serde_json::from_str(&outcome.text).unwrap();
        let task_id = result["task_id"].as_str().unwrap().to_string();
        let terminal = bash_background_wait(&Map::from_iter([
            ("task_id".into(), Value::String(task_id)),
            ("timeout_sec".into(), Value::from(5)),
            ("poll_interval_ms".into(), Value::from(100)),
        ]), &ctx);
        assert!(!terminal.error, "{}", terminal.text);
        assert_eq!(terminal.data.as_ref().unwrap()["terminal"], true);
        let first = take_background_task_notifications(std::slice::from_ref(&root), None).unwrap();
        assert_eq!(first.len(), 1);
        let status_args = Map::from_iter([("task_id".into(), Value::String(first[0].id.clone()))]);
        assert!(!bash_background_status(&status_args, &ctx).error);
        assert_eq!(take_background_task_notifications(std::slice::from_ref(&root), None).unwrap().len(), 1,
            "model status reads do not acknowledge UI presentation");
        BACKGROUND_BASH_TASKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .clear();
        let after_reconstruction =
            take_background_task_notifications(std::slice::from_ref(&root), None).unwrap();
        assert_eq!(after_reconstruction.len(), 1, "fetch followed by process loss must replay");
        let notification = &after_reconstruction[0];
        assert!(notification.delivery.owner_conversation_id.is_none());
        let mut wrong_owner = notification.delivery.clone();
        wrong_owner.owner_conversation_id = Some("another-session".into());
        assert!(crate::runtime_task_port::acknowledge_notification_delivery(&wrong_owner).is_err());
        let mut old = notification.delivery.clone(); old.occurrence_id = "terminal:old-receipt".into();
        assert!(crate::runtime_task_port::acknowledge_notification_delivery(&old).is_err());
        assert!(deliver_background_task_notification(notification, || Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "output failed"))).is_err());
        assert!(!crate::runtime_task_port::notification_delivered(&notification.delivery).unwrap());
        assert_eq!(deliver_background_task_notification(notification, || Ok(crate::terminal_ui::HistoryPresentation::Deferred)).unwrap(), None);
        assert!(!crate::runtime_task_port::notification_delivered(&notification.delivery).unwrap());
        assert_eq!(deliver_background_task_notification(notification, || Ok(crate::terminal_ui::HistoryPresentation::Presented)).unwrap(), Some(nola_application::RuntimeTaskNotificationAcknowledgement::Delivered));
        assert_eq!(acknowledge_background_task_notification(notification).unwrap(), nola_application::RuntimeTaskNotificationAcknowledgement::AlreadyDelivered);
        let mut other_consumer = notification.delivery.clone(); other_consumer.consumer_id = "other-ui".into();
        assert!(!crate::runtime_task_port::notification_delivered(&other_consumer).unwrap());
        assert_eq!(crate::runtime_task_port::acknowledge_notification_delivery(&other_consumer).unwrap(), nola_application::RuntimeTaskNotificationAcknowledgement::Delivered);
        BACKGROUND_BASH_TASKS.get().unwrap().lock().unwrap().clear();
        assert!(take_background_task_notifications(std::slice::from_ref(&root), None).unwrap().is_empty());

        unsafe {
            env::remove_var("NOLA_DB_PATH");
        }
        let _ = fs::remove_dir_all(root);
    }
}

fn is_test_scope_path(path: &Path) -> bool {
    path.components().any(|component| {
        let text = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            text.as_str(),
            "test" | "tests" | "testing" | "spec" | "specs"
        )
    })
}

fn foreground_transfer_requested(cancel_observed: bool, user_requested: bool, grace_elapsed: bool) -> bool {
    !cancel_observed && (user_requested || grace_elapsed)
}

fn submit_foreground_cancellation(
    submitted: &mut bool,
    submit: impl FnOnce() -> Result<bool, String>,
) -> Result<bool, String> {
    if *submitted { return Ok(true); }
    let accepted = submit()?;
    *submitted = accepted;
    Ok(accepted)
}

// The durable queue boundary was crossed or attempted. Losing submission,
// observation or cancellation ACK cannot attest the command never executed.
fn foreground_task_observation_error(task_id: &str, phase: &str, detail: String) -> ToolOutcome {
    error(detail.clone()).with_data(json!({
        "type": "deferred_tool_task",
        "task_id": task_id,
        "state": "unknown",
        "effects_completed": false,
        "observation_error": { "phase": phase, "detail": detail },
        "recovery": { "tool": "bash_background_status", "arguments": { "task_id": task_id } },
    })).with_execution(nola_host_protocol::tool_result::ExecutionReceipt {
        state: nola_host_protocol::tool_result::ExecutionState::Unknown,
        ..Default::default()
    })
}

fn register_supervised_shell_index(
    task_id: &str,
    register: impl FnOnce() -> Result<(), String>,
) -> Result<(), ToolOutcome> {
    register().map_err(|detail| foreground_task_observation_error(task_id, "registry", detail))
}

#[cfg(test)]
#[test]
fn queued_task_observation_failure_retains_identity_and_unknown_execution() {
    for phase in ["queue_submit", "registry", "observe", "cancel_submit", "cancel_observe"] {
        let result = foreground_task_observation_error("original-task", phase, "storage unavailable".into());
        assert!(result.error);
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["task_id"], "original-task");
        assert_eq!(data["state"], "unknown");
        assert_eq!(data["observation_error"]["phase"], phase);
        assert_eq!(data["recovery"]["arguments"]["task_id"], "original-task");
        assert_eq!(result.execution.unwrap().state, nola_host_protocol::tool_result::ExecutionState::Unknown);
    }
    let failed_index = register_supervised_shell_index("original-task", || Err("poisoned index".into())).unwrap_err();
    assert_eq!(failed_index.data.as_ref().unwrap()["task_id"], "original-task");
    assert_eq!(failed_index.execution.unwrap().state, nola_host_protocol::tool_result::ExecutionState::Unknown);
}

#[cfg(test)]
#[test]
fn foreground_cancellation_submission_is_once_and_errors_are_not_acknowledgments() {
    let mut submitted = false;
    assert!(submit_foreground_cancellation(&mut submitted, || Err("storage unavailable".into())).is_err());
    assert!(!submitted);
    assert!(!submit_foreground_cancellation(&mut submitted, || Ok(false)).unwrap());
    assert!(!submitted);
    assert!(submit_foreground_cancellation(&mut submitted, || Ok(true)).unwrap());
    assert!(submit_foreground_cancellation(&mut submitted, || panic!("duplicate cancellation submission")).unwrap());
}

#[cfg(test)]
#[test]
fn cancellation_prevents_both_requested_and_automatic_foreground_transfer() {
    for user in [false, true] {
        for automatic in [false, true] {
            assert!(!foreground_transfer_requested(true, user, automatic));
            assert_eq!(foreground_transfer_requested(false, user, automatic), user || automatic);
        }
    }
    let control = RuntimeExecutionControl::default().with_background_transfer();
    assert!(control.request_background());
    control.request();
    assert!(!foreground_transfer_requested(control.is_requested(), control.take_background_request(), true));
}

fn run_bash(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    run_bash_with_permission(args, ctx, false)
}

fn run_bash_with_permission(
    args: &Map<String, Value>,
    ctx: &ToolContext,
    approved_by_permission: bool,
) -> ToolOutcome {
    let operation = args.get("operation_kind").and_then(Value::as_str);
    if args.contains_key("regression_test_files")
        && (operation != Some("validate") || args.get("execution_mode").and_then(Value::as_str) == Some("background")) {
        return error_data(json!({"error":"invalid_regression_validation", "message":"regression_test_files requires foreground operation_kind=validate; keep the original regression unchanged across repair", "tool_execution_skipped":true}));
    }
    if operation != Some("validate") {
        // Declared output settlement emits file-version effects, never
        // verification credit from an opaque combined command's label.
        return run_bash_execution(args, ctx, approved_by_permission);
    }
    let before = match validation_snapshot(args, ctx) {
        Ok(snapshot) => snapshot,
        Err(message) => return error_data(json!({"error":"validation_scope_required", "message":message, "tool_execution_skipped":true})),
    };
    let regression = match regression_check_predicate(args, ctx, &before) {
        Ok(predicate) => predicate,
        Err(message) => return error_data(json!({"error":"invalid_regression_validation", "message":message,"tool_execution_skipped":true})),
    };
    let outcome = run_bash_execution(args, ctx, approved_by_permission);
    if let Some(predicate) = regression {
        return attach_regression_evidence(outcome, before, validation_snapshot(args, ctx), predicate);
    }
    if outcome.error || outcome.execution.as_ref().is_some_and(|receipt| receipt.state == nola_host_protocol::tool_result::ExecutionState::Deferred) { return outcome; }
    attach_validation_evidence(outcome, before, validation_snapshot(args, ctx))
}

fn read_shell_capture(path: &Path) -> Result<String, ToolOutcome> {
    let bytes = fs::read(path).map_err(|err| error_data_with_display(json!({
        "error": "shell_capture_read_failed",
        "path": path,
        "message": err.to_string(),
        "output_complete": false,
        "next_action": "Inspect the existing execution and capture; do not replay the command merely to recover output."
    }), "Unable to read command output".into()))?;
    // Shell streams are bytes, not guaranteed UTF-8. Preserve readable text
    // around binary bytes; the capture file retains the original byte stream.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn shell_capture_keeps_text_around_binary_and_reports_missing_capture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stdout");
    let bytes = b"before\n\xff\xfe\nafter\n";
    fs::write(&path, bytes).unwrap();
    let output = read_shell_capture(&path).unwrap();
    assert!(output.starts_with("before\n"));
    assert!(output.ends_with("\nafter\n"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let failure = read_shell_capture(&dir.path().join("missing")).unwrap_err();
    assert_eq!(failure.data.unwrap()["error"], "shell_capture_read_failed");
}

/// A stop request is not proof that every descendant stopped. Preserve the
/// captures and report a snapshot, including read failures, without replay advice.
fn interrupted_shell_output(
    command: &str,
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    stop_observation: Result<nola_domain::execution_evidence::LocalExecutionStopScope, String>,
    reason: &str,
    detail: String,
    display: String,
) -> ToolOutcome {
    let (local_stop, local_stop_error) = match stop_observation {
        Ok(scope) => match current_local_execution_identity() {
            Some(identity) => (Some(nola_domain::execution_evidence::LocalExecutionStop { identity, scope }), None),
            None => (None, Some("No current Host-owned invocation binding is available".to_string())),
        },
        Err(error) => (None, Some(error)),
    };
    let stdout = fs::read_to_string(stdout_path);
    let stderr = fs::read_to_string(stderr_path);
    let stdout_error = stdout.as_ref().err().map(ToString::to_string);
    let stderr_error = stderr.as_ref().err().map(ToString::to_string);
    let stdout = stdout.ok();
    let stderr = stderr.ok();
    let (stdout_preview, _) = compact_head_tail_text(stdout.as_deref().unwrap_or(""), 4_000);
    let (stderr_preview, _) = compact_head_tail_text(stderr.as_deref().unwrap_or(""), 4_000);
    let mut outcome = error_with_display(format!(
        "{detail}\nprocess_tree_stop: unknown\nOutput is a captured snapshot, not a completed stream. Inspect execution state before retrying.\n--- partial stdout ---\n{stdout_preview}\n--- partial stderr ---\n{stderr_preview}"
    ), display);
    outcome.data = Some(json!({
        "type": "shell_execution", "command": command, "cwd": cwd,
        "state": "unknown", "interruption_reason": reason, "detail": detail,
        "stop_requested": true, "process_tree_stop": "unknown",
        "local_execution_stop": local_stop, "local_execution_stop_error": local_stop_error,
        "output_complete": false, "output_compacted": false,
        "stdout": stdout, "stderr": stderr,
        "stdout_read_error": stdout_error, "stderr_read_error": stderr_error,
        "capture_paths": {"stdout": stdout_path, "stderr": stderr_path},
        "recovery_hint": "Inspect execution state before retrying; interruption does not prove that side effects stopped."
    }));
    outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
        state: nola_host_protocol::tool_result::ExecutionState::Unknown,
        local_stop,
        ..Default::default()
    });
    outcome
}

#[test]
fn interrupted_shell_output_preserves_raw_capture_and_does_not_invent_stop_proof() {
    let workspace = tempfile::tempdir().unwrap();
    let stdout_path = workspace.path().join("stdout.log");
    let stderr_path = workspace.path().join("stderr.log");
    let original = "中\\\"😀\n".repeat(4_000);
    fs::write(&stdout_path, &original).unwrap();
    let outcome = interrupted_shell_output("fixture", workspace.path(), &stdout_path,
        &stderr_path, Err("No owned stop confirmation".into()),
        "timeout", "timeout".into(), "timeout".into());
    let data = outcome.data.unwrap();
    assert!(outcome.error);
    assert_eq!(outcome.execution.unwrap().state,
        nola_host_protocol::tool_result::ExecutionState::Unknown);
    assert_eq!(data["stdout"].as_str(), Some(original.as_str()));
    assert!(data["stderr"].is_null());
    assert!(data["stderr_read_error"].as_str().is_some_and(|error| !error.is_empty()));
    assert_eq!(data["state"], "unknown");
    assert_eq!(data["process_tree_stop"], "unknown");
    assert_eq!(data["output_complete"], false);
    assert_eq!(data["output_compacted"], false);
    assert!(data.get("process_group_terminated").is_none());
    assert_eq!(fs::read_to_string(stdout_path).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn interrupted_shell_owned_group_stop_does_not_promote_unknown_effects() {
    use nola_domain::execution_evidence::{LocalExecutionIdentity, LocalExecutionStopScope};
    let workspace = tempfile::tempdir().unwrap();
    let stdout = workspace.path().join("stdout");
    let stderr = workspace.path().join("stderr");
    fs::write(&stdout, "partial output").unwrap();
    fs::write(&stderr, "").unwrap();
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30"]);
    let mut child = crate::task_runtime::OwnedShellChild::spawn_owned(&mut command).unwrap();
    let pid = child.id();
    let identity = LocalExecutionIdentity {
        execution_id: "private-invocation".into(), host_id: "host".into(), session_id: "session".into(),
        turn_id: "turn".into(), effect_id: "effect".into(), request_id: "request".into(),
        call_id: "call".into(), delivery_attempt_id: Some("attempt".into()),
    };
    let _guard = enter_local_execution_identity(identity.clone());
    let observation = child.stop_and_confirm(Duration::from_secs(2));
    let outcome = interrupted_shell_output("fixture", workspace.path(), &stdout, &stderr,
        observation, "cancel_requested", "cancelled".into(), "cancelled".into());
    assert!(outcome.error);
    let receipt = outcome.execution.unwrap();
    assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Unknown);
    assert!(receipt.effects.is_empty() && receipt.checks.is_empty());
    let stopped = receipt.local_stop.unwrap();
    assert_eq!(stopped.identity, identity);
    assert_eq!(stopped.scope, LocalExecutionStopScope::OwnedProcessGroup { leader_pid: pid });
    let data = outcome.data.unwrap();
    assert_eq!(data["state"], "unknown");
    assert_eq!(data["process_tree_stop"], "unknown");
    assert_eq!(data["output_complete"], false);
    assert_eq!(data["stdout"], "partial output");
}

#[cfg(unix)]
#[test]
fn direct_shell_terminal_and_failed_artifact_observation_keep_the_owned_local_stop() {
    use nola_domain::execution_evidence::LocalExecutionIdentity;
    let workspace = tempfile::tempdir().unwrap();
    let context = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(), allowed_dirs: vec![workspace.path().to_path_buf()],
        policy: ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
        permission_scope: Default::default(), cancellation: Default::default(), plan_state: Default::default(),
        output_store: ToolOutputStore::for_test(), browser: BrowserRuntime::new(workspace.path().join("browser")),
    };
    for (nonce, command, path, expected_error) in [
        ("normal", "printf 'actual output'; printf data > result.txt", "result.txt", false),
        ("failed-observation", "printf 'actual output'; mkdir claimed-file", "claimed-file", true),
    ] {
        let identity = LocalExecutionIdentity {
            execution_id: nonce.into(), host_id: "host".into(), session_id: "session".into(),
            turn_id: "turn".into(), effect_id: format!("effect-{nonce}"), request_id: format!("request-{nonce}"),
            call_id: format!("call-{nonce}"), delivery_attempt_id: None,
        };
        let _scope = enter_local_execution_identity(identity.clone());
        let args = json!({"command":command, "cwd":workspace.path(), "operation_kind":"execute",
            "execution_mode":"foreground", "output_artifacts":[{"path":path,"operation":"create"}]});
        let outcome = run_bash_execution(args.as_object().unwrap(), &context, true);
        assert_eq!(outcome.error, expected_error, "{}", outcome.text);
        let receipt = outcome.execution.unwrap();
        assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Unknown);
        assert_eq!(receipt.local_stop.unwrap().identity, identity);
        assert!(receipt.checks.is_empty());
        if expected_error {
            assert!(receipt.effects.is_empty());
            assert_eq!(outcome.data.as_ref().unwrap()["output_contract_failure"]["code"], "artifact_state_unavailable");
        } else {
            assert_eq!(receipt.effects.len(), 1, "with_file_artifact_changes retains the independent stop");
        }
        assert_eq!(outcome.data.unwrap()["stdout"], "actual output");
    }
}

fn run_bash_execution(
    args: &Map<String, Value>,
    ctx: &ToolContext,
    approved_by_permission: bool,
) -> ToolOutcome {
    if ctx.cancellation.is_requested() {
        return error_data_with_display(json!({
            "error":"cancelled", "tool_execution_skipped":true,
            "message":"The command was cancelled before execution"
        }), "Command cancelled".to_string());
    }
    let command = tool_try!(required_string(args, "command"));
    if matches!(args.get("operation_kind").and_then(Value::as_str), Some("validate" | "mutate_and_validate"))
        && !args.get("validation_targets").and_then(Value::as_array).is_some_and(|targets| {
            !targets.is_empty() && targets.iter().all(|target| target.as_str().is_some_and(|path| !path.trim().is_empty() && path.trim() != "*"))
        })
    {
        return error_data(json!({
            "error": "validation_scope_required",
            "tool_execution_skipped": true,
            "message": "Validation requires nonempty validation_targets naming the resources actually checked. Supply exact paths or a covering root. Wildcard * is not verification evidence and cannot resolve unknown mutation scope. Do not repeat a completed mutation or relabel it as inspection to bypass verification."
        }));
    }
    if bash_command_too_long(&command) {
        return bash_command_length_error(&command);
    }
    let cwd = match args
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        Some(cwd) => match resolve_allowed(cwd, &ctx.allowed_dirs, true) {
            Ok(path) if path.is_dir() => path,
            Ok(path) => return error(format!("cwd is not a directory: {}", path.display())),
            Err(outcome) => return outcome,
        },
        None => ctx
            .allowed_dirs
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    match if unrestricted_resource_policy() { BashCommandPathPolicy::Allowed } else { bash_command_path_policy_check(
        &command,
        Some(cwd.to_string_lossy().as_ref()),
        &ctx.allowed_dirs,
    ) } {
        BashCommandPathPolicy::Allowed => {}
        BashCommandPathPolicy::UnsupportedSyntax => {
            return error_with_display(
                "Bash command uses syntax that cannot be safely analyzed. Use a short command without heredocs, command substitution, or embedded multiline source; write source with write_file or append_file before running it."
                    .to_string(),
                "Command syntax is unsupported by the workspace boundary checker".to_string(),
            );
        }
        BashCommandPathPolicy::InvalidPath => {
            return error("Command working directory or resource path cannot be resolved".to_string());
        }
        BashCommandPathPolicy::SensitiveResource if approved_by_permission => {}
        BashCommandPathPolicy::SensitiveResource => {
            return error_data(json!({"error":"sensitive_shell_resource", "tool_execution_skipped":true,
                "message":"This shell command targets a sensitive resource and requires approval for this exact invocation. Request sensitive data reading approval; a workspace directory grant is not needed."}));
        }
    }
    // Shell authorization is evaluated against its concrete command/resources.
    // A specialized tool is an alternative, not a compulsory workflow. Shell
    // success does not manufacture that tool's stronger postcondition receipt.
    let permissive_test_scope = test_folder_permissive_enabled() && is_test_scope_path(&cwd);
    let permissive_sandbox = sandbox_bash_permissive_enabled();
    let sandbox_read_only = permissive_sandbox
        && bash_command_is_safe_read_only(
            &command,
            Some(cwd.to_string_lossy().as_ref()),
            &ctx.allowed_dirs,
        );
    let approved_once = approved_by_permission;
    let forbidden = forbidden_shell_syntax_tokens(&command);
    if !(permissive_test_scope || permissive_sandbox || approved_once)
        && !forbidden.is_empty()
        && !is_validation_bash_command_allowed(&command)
    {
        return error("shell metacharacters/substitutions are not allowed".to_string());
    }
    let head = command.split_whitespace().next().unwrap_or("");
    if !unrestricted_resource_policy() && permissive_sandbox && is_blocked_sandbox_git_command_from_str(&command) {
        return error(
            "mutating or network git commands are disabled in sandbox-permissive bash mode"
                .to_string(),
        );
    }
    if !permissive_test_scope && !approved_once && ctx.policy.bash_prefixes().is_empty() {
        return error("No bash prefixes whitelisted. Use --bash-prefix.".to_string());
    }
    if !permissive_test_scope
        && !approved_once
        && !sandbox_read_only
        && !bash_command_segments_match_scopes(&command, ctx.policy.bash_prefixes())
    {
        return error(format!("'{head}' is not in the bash allow-list"));
    }
    let execution_mode = tool_try!(bash_execution_mode(args));
    let terminal = match args.get("terminal") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let size: nola_domain::terminal_input::TerminalDimensions = tool_try!(
                serde_json::from_value(value.clone()).map_err(|err| error(format!("Invalid terminal dimensions: {err}"))));
            tool_try!(size.validate().map_err(error));
            if execution_mode != BashExecutionMode::Background {
                return error("terminal requires explicit execution_mode=background".into());
            }
            if ctx.permission_scope.owner_session_id().is_none() {
                return error("terminal requires an owning conversation".into());
            }
            #[cfg(not(unix))]
            return error("PTY execution is unavailable on this platform".into());
            #[cfg(unix)]
            {
                let identity = tool_try!(current_local_execution_identity()
                    .ok_or_else(|| error("terminal requires an admitted Host execution identity".into())));
                Some(nola_domain::terminal_input::TerminalSessionConfiguration { size, host_id: identity.host_id })
            }
        }
    };
    if let Some(spec)=ctx.policy.process_sandbox() {
        if let Err(message)=spec.require_available() {
            return error_data(json!({"error":"required_process_sandbox_unavailable","message":message,"tool_execution_skipped":true,"unrestricted_fallback":false}));
        }
    }
    let verified_read_only = permission_resolution::verified_read_only_shell_command(ctx, args);
    let requested_command = command.clone();
    let command = verified_read_only.clone().unwrap_or(command);
    let preservation = tool_try!(git_process_preservation(&cwd, &ctx.allowed_dirs)
        .map_err(|message| error_data(json!({"error":"preservation_observation_failed", "tool_execution_skipped":true, "message":message}))));
    if preservation.active() && execution_mode == BashExecutionMode::Background {
        let mut outcome = error_data(json!({"error":"preservation_requires_foreground", "tool_execution_skipped":true,
            "message":"Durable background execution cannot yet carry this preservation contract. Use foreground execution; the guard cannot be dropped."}));
        preservation.annotate(&mut outcome);
        return outcome;
    }
    let mut guarded_command = tool_try!(preservation.command().map_err(|message| {
        let mut outcome = error_data(json!({"error":"preservation_backend_unavailable", "tool_execution_skipped":true, "message":message}));
        preservation.annotate(&mut outcome);
        outcome
    }));
    let timeout = args
        .get("timeout_sec")
        .and_then(Value::as_u64)
        .unwrap_or(if execution_mode == BashExecutionMode::Background {
            2 * 60 * 60
        } else {
            120
        })
        .clamp(1, bash_timeout_max_sec());
    let declared_outputs = tool_try!(declared_bash_outputs(args, &cwd, &ctx.allowed_dirs));
    if !declared_outputs.is_empty()
        && matches!(
            args.get("operation_kind").and_then(Value::as_str),
            Some("inspect" | "validate")
        )
    {
        return error(
            "output_artifacts require operation_kind=mutate, mutate_and_validate, or execute"
                .to_string(),
        );
    }
    let declared_output_paths = declared_outputs
        .iter()
        .map(|output| output.path.as_path())
        .collect::<Vec<_>>();
    let output_states_before = tool_try!(capture_file_artifact_states(&declared_output_paths));
    let background_capable = verified_read_only.is_none() && !preservation.active() && matches!(
        execution_mode,
        BashExecutionMode::Auto | BashExecutionMode::Background
    );
    // Interactive foreground commands enter the unified supervisor from the
    // outset so Ctrl+B can release the wait without restarting or replacing
    // the real worker/command. Non-terminal Hosts keep short foreground work
    // ephemeral unless their composition explicitly enables transfer capability.
    let foreground_transferable = verified_read_only.is_none() && !preservation.active() && (execution_mode == BashExecutionMode::Auto
        || (execution_mode == BashExecutionMode::Foreground
            && ctx.cancellation.supports_background_transfer()));

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let task_id = format!("bash-{}-{stamp}", std::process::id());
    // The task starts before auto mode's observation window. Persist that
    // original start time so elapsed time never drops the foreground grace.
    let started_at_ms = unix_epoch_millis();
    let durable_task = background_capable || foreground_transferable;
    let (stdout_path, stderr_path, status_path) = if durable_task {
        let database_path = match crate::platform_paths::application_database_path() {
            Ok(path) => path,
            Err(err) => return error(format!("Resolve background task state failed: {err}")),
        };
        let Some(state_root) = database_path.parent() else {
            return error("Background task store has no parent directory".to_string());
        };
        let task_dir = state_root.join("cli").join("background");
        if let Err(err) = fs::create_dir_all(&task_dir) {
            return error(format!(
                "Create background task directory {} failed: {err}",
                task_dir.display()
            ));
        }
        (
            task_dir.join(format!("{task_id}.stdout.log")),
            task_dir.join(format!("{task_id}.stderr.log")),
            task_dir.join(format!("{task_id}.status")),
        )
    } else {
        (
            std::env::temp_dir().join(format!("nola-cli-{}-{stamp}.out", std::process::id())),
            std::env::temp_dir().join(format!("nola-cli-{}-{stamp}.err", std::process::id())),
            std::env::temp_dir().join(format!("nola-cli-{}-{stamp}.status", std::process::id())),
        )
    };
    let _ = fs::remove_file(&status_path);
    if execution_mode == BashExecutionMode::Background {
        if let Err(err) = File::create(&stdout_path) {
            return error(format!("temp stdout failed: {err}"));
        }
        if let Err(err) = File::create(&stderr_path) {
            return error(format!("temp stderr failed: {err}"));
        }
        let project_root = ctx
            .allowed_dirs
            .iter()
            .find(|root| cwd.starts_with(root))
            .cloned()
            .unwrap_or_else(|| cwd.clone());
        let output_contract = tool_try!(prepare_background_output_contract(
            &task_id,
            &declared_outputs,
            &output_states_before,
            ctx,
        ));
        let output_contract_json = tool_try!(
            output_contract
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|encode_error| error(format!(
                    "Encode background output contract failed: {encode_error}"
                )))
        );
        let record = DbBackgroundTask {
            process_sandbox: ctx.policy.process_sandbox().cloned(),
            terminal,
            id: task_id.clone(),
            project_root: project_root.to_string_lossy().to_string(),
            command: command.clone(),
            cwd: cwd.to_string_lossy().to_string(),
            pid: 0,
            stdout_path: stdout_path.to_string_lossy().to_string(),
            stderr_path: stderr_path.to_string_lossy().to_string(),
            status_path: status_path.to_string_lossy().to_string(),
            started_at_ms,
            owner_session_id: ctx.permission_scope.owner_session_id(),
            output_contract_json,
            sop_invocation: current_sop_invocation_context(),
        };
        let priority = match args.get("priority").and_then(Value::as_str).unwrap_or("normal") {
            "low" => nola_application::RuntimeTaskPriority::Low,
            "normal" => nola_application::RuntimeTaskPriority::Normal,
            "high" => nola_application::RuntimeTaskPriority::High,
            value => return error(format!("invalid background task priority: {value}")),
        };
        let scheduling = nola_application::RuntimeTaskScheduling {
            priority,
            queued_at_ms: started_at_ms,
            resource_budget: nola_application::RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(timeout.saturating_mul(1_000)),
                max_output_bytes: Some(
                    args.get("max_output_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(2 * 1024 * 1024)
                        .clamp(1_024, 16 * 1024 * 1024),
                ),
                max_cpu_time_ms: args
                    .get("max_cpu_time_sec")
                    .and_then(Value::as_u64)
                    .map(|seconds| seconds.clamp(1, 86_400).saturating_mul(1_000)),
                max_memory_bytes: args
                    .get("max_memory_bytes")
                    .and_then(Value::as_u64)
                    .map(|bytes| bytes.clamp(16 * 1024 * 1024, 64 * 1024 * 1024 * 1024)),
                max_processes: args
                    .get("max_processes")
                    .and_then(Value::as_u64)
                    .map(|count| count.clamp(1, 256) as u32),
            },
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        };
        let started = match crate::task_runtime::queue_background_shell_task(
            record.clone(),
            scheduling,
        ) {
            Ok(started) => started,
            Err(err) => {
                return foreground_task_observation_error(&task_id, "queue_submit", format!("Queue background Shell task acknowledgment failed: {err}"));
            }
        };
        let task = BackgroundBashTask {
            id: task_id.clone(),
            project_root,
            command: command.clone(),
            cwd: cwd.clone(),
            child: None,
            pid: 0,
            stdout_path,
            stderr_path,
            status_path,
            started_at_ms,
            owner_session_id: record.owner_session_id,
            notify_on_completion: true,
            output_contract,
        };
        let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
        if let Err(outcome) = register_supervised_shell_index(&task_id, || {
            let mut tasks = state.lock().map_err(|_| "background task state lock poisoned after submission".to_string())?;
            tasks.insert(task_id.clone(), task);
            Ok(())
        }) {
            return outcome;
        }
        let mut submitted = json!({
                "type": "deferred_tool_task",
                "task_id": task_id,
                "summary": format!("Queued background bash task {task_id}"),
                "state": if started { "starting" } else { "queued_for_slot" },
                "resource_supervisor": "unified",
                "effects_completed": false,
            });
        if record.terminal.is_some() {
            attach_terminal_submission_guidance(&mut submitted, &task_id);
        }
        return ok_data(submitted,
            format!("Queued background bash {task_id}"),
        ).with_execution(nola_host_protocol::tool_result::ExecutionReceipt {
            state:nola_host_protocol::tool_result::ExecutionState::Deferred, ..Default::default()
        });
    }
    if foreground_transferable {
        if let Err(err) = File::create(&stdout_path) {
            return error(format!("temp stdout failed: {err}"));
        }
        if let Err(err) = File::create(&stderr_path) {
            return error(format!("temp stderr failed: {err}"));
        }
        let project_root = ctx
            .allowed_dirs
            .iter()
            .find(|root| cwd.starts_with(root))
            .cloned()
            .unwrap_or_else(|| cwd.clone());
        let output_contract = tool_try!(prepare_background_output_contract(
            &task_id,
            &declared_outputs,
            &output_states_before,
            ctx,
        ));
        let output_contract_json = tool_try!(
            output_contract
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|encode_error| error(format!(
                    "Encode background output contract failed: {encode_error}"
                )))
        );
        let record = DbBackgroundTask {
            process_sandbox: ctx.policy.process_sandbox().cloned(),
            terminal: None,
            id: task_id.clone(),
            project_root: project_root.to_string_lossy().to_string(),
            command: command.clone(),
            cwd: cwd.to_string_lossy().to_string(),
            pid: 0,
            stdout_path: stdout_path.to_string_lossy().to_string(),
            stderr_path: stderr_path.to_string_lossy().to_string(),
            status_path: status_path.to_string_lossy().to_string(),
            started_at_ms,
            owner_session_id: ctx.permission_scope.owner_session_id(),
            output_contract_json,
            sop_invocation: current_sop_invocation_context(),
        };
        let scheduling = nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: started_at_ms,
            resource_budget: nola_application::RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(timeout.saturating_mul(1_000)),
                max_output_bytes: Some(
                    args.get("max_output_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(2 * 1024 * 1024)
                        .clamp(1_024, 16 * 1024 * 1024),
                ),
                max_cpu_time_ms: args
                    .get("max_cpu_time_sec")
                    .and_then(Value::as_u64)
                    .map(|seconds| seconds.clamp(1, 86_400).saturating_mul(1_000)),
                max_memory_bytes: args
                    .get("max_memory_bytes")
                    .and_then(Value::as_u64)
                    .map(|bytes| bytes.clamp(16 * 1024 * 1024, 64 * 1024 * 1024 * 1024)),
                max_processes: args
                    .get("max_processes")
                    .and_then(Value::as_u64)
                    .map(|count| count.clamp(1, 256) as u32),
            },
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        };
        let mut started = match crate::task_runtime::queue_background_shell_task(
            record.clone(),
            scheduling,
        ) {
            Ok(started) => started,
            Err(err) => return foreground_task_observation_error(&task_id, "queue_submit", format!("Queue supervised foreground Shell acknowledgment failed: {err}")),
        };
        let task = BackgroundBashTask {
            id: task_id.clone(),
            project_root,
            command: command.clone(),
            cwd: cwd.clone(),
            child: None,
            pid: 0,
            stdout_path: stdout_path.clone(),
            stderr_path: stderr_path.clone(),
            status_path: status_path.clone(),
            started_at_ms,
            owner_session_id: record.owner_session_id.clone(),
            notify_on_completion: false,
            output_contract,
        };
        let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
        if let Err(outcome) = register_supervised_shell_index(&task_id, || {
            let mut tasks = state.lock().map_err(|_| "background task state lock poisoned after submission".to_string())?;
            tasks.insert(task_id.clone(), task);
            Ok(())
        }) {
            return outcome;
        }

        let mut command_wait_started = None::<Instant>;
        let mut cancellation_observed = false;
        let mut cancellation_submitted = false;
        let terminal_task = loop {
            cancellation_observed |= ctx.cancellation.is_requested();
            // Completion wins over promotion. Checking the grace deadline
            // first races the supervisor: a short process that has already
            // persisted its terminal state at the deadline would otherwise be
            // reported as a newly promoted background task.
            match crate::runtime_task_port::get(&task_id) {
                Ok(Some(task))
                    if matches!(task.status.as_str(), "completed" | "failed" | "cancelled") =>
                {
                    break task;
                }
                Ok(Some(task)) => {
                    cancellation_observed |= task.cancel_requested;
                    cancellation_submitted |= task.cancel_requested;
                    if command_wait_started.is_none() && task.started_at_ms.is_some() {
                        command_wait_started = Some(Instant::now());
                    }
                }
                Ok(None) => return foreground_task_observation_error(&task_id, "observe", "Supervised foreground Shell task disappeared".to_string()),
                Err(err) => return foreground_task_observation_error(&task_id, "observe", format!("Read supervised foreground Shell failed: {err}")),
            }
            cancellation_observed |= ctx.cancellation.is_requested();
            if cancellation_observed && !cancellation_submitted {
                match submit_foreground_cancellation(&mut cancellation_submitted, || {
                    crate::runtime_task_port::request_cancel(&task_id, unix_epoch_millis())
                }) {
                    Ok(true) => cancellation_submitted = true,
                    Ok(false) => {
                        // A terminal transaction may have won the race. A
                        // false submission is not evidence of cancellation.
                        match crate::runtime_task_port::get(&task_id) {
                            Ok(Some(task)) if matches!(task.status.as_str(), "completed" | "failed" | "cancelled") => break task,
                            Ok(Some(task)) if task.cancel_requested => cancellation_submitted = true,
                            Ok(_) => return foreground_task_observation_error(&task_id, "cancel_observe", "Supervised foreground Shell cancellation was not recorded; task outcome remains unconfirmed".to_string()),
                            Err(err) => return foreground_task_observation_error(&task_id, "cancel_observe", format!("Read supervised foreground Shell cancellation failed; task outcome remains unconfirmed: {err}")),
                        }
                    }
                    Err(err) => return foreground_task_observation_error(&task_id, "cancel_submit", format!("Submit supervised foreground Shell cancellation failed; task outcome remains unconfirmed: {err}")),
                }
            }
            let user_requested_background = !cancellation_observed && ctx.cancellation.take_background_request();
            let automatic_background = execution_mode == BashExecutionMode::Auto
                && command_wait_started
                    .is_some_and(|started| started.elapsed() >= bash_auto_background_after());
            if foreground_transfer_requested(cancellation_observed, user_requested_background, automatic_background) {
                // A cancellation/terminal transaction may race the grace
                // check. Never turn that observation into a "running" receipt.
                let runtime_task = match crate::runtime_task_port::get(&task_id) {
                    Ok(Some(task)) => task,
                    Ok(None) => return foreground_task_observation_error(&task_id, "observe", "Supervised foreground Shell task disappeared".to_string()),
                    Err(err) => return foreground_task_observation_error(&task_id, "observe", format!("Read supervised foreground Shell failed: {err}")),
                };
                if matches!(runtime_task.status.as_str(), "completed" | "failed" | "cancelled") {
                    break runtime_task;
                }
                cancellation_observed |= runtime_task.cancel_requested || ctx.cancellation.is_requested();
                if cancellation_observed { continue; }
                if let Ok(mut tasks) = state.lock() {
                    if let Some(task) = tasks.get_mut(&task_id) {
                        task.notify_on_completion = true;
                    }
                }
                let pid = serde_json::from_str::<Value>(&runtime_task.metadata_json).ok()
                    .and_then(|metadata| metadata.get("pid").and_then(Value::as_u64))
                    .unwrap_or(0);
                return ok_data(json!({
                        "type": "deferred_tool_task",
                        "task_id": task_id,
                        "summary": format!("Started background bash task {task_id}"),
                        "pid": pid,
                        "state": "running",
                        "effects_completed": false,
                        "background_mode": if automatic_background {
                            "automatic_after_grace"
                        } else {
                            "user_transfer"
                        },
                        "resource_supervisor": "unified",
                        "foreground_wait_released": true,
                    }),
                    if automatic_background {
                        format!("Bash continues in background {task_id}")
                    } else {
                        format!("Moved bash to background {task_id}")
                    },
                ).with_execution(nola_host_protocol::tool_result::ExecutionReceipt {
                    state:nola_host_protocol::tool_result::ExecutionState::Deferred, ..Default::default()
                });
            }
            if !started && !cancellation_observed {
                // A foreground transferable command may be waiting behind an
                // older queued Agent task. The active Core turn cannot enter
                // the interactive idle scheduler until this Tool returns, so
                // let the bound Supervisor configuration promote that task
                // before retrying this command's claim.
                let owner_scope_id = ctx.permission_scope.owner_session_id();
                crate::task_runtime::reconcile_bound_supervised_tasks(
                    owner_scope_id.as_deref(),
                    &cwd,
                );
                started = crate::task_runtime::try_start_queued_background_shell_task(
                    record.clone(),
                )
                .unwrap_or(false);
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        let stdout_full = tool_try!(read_shell_capture(&stdout_path));
        let stderr_full = tool_try!(read_shell_capture(&stderr_path));
        let (stdout, stdout_compacted) = compact_head_tail_text(&stdout_full, 8_000);
        let (stderr, stderr_compacted) = compact_head_tail_text(&stderr_full, 8_000);
        let status = fs::read_to_string(&status_path)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| terminal_task.last_activity.clone().unwrap_or_else(|| terminal_task.status.clone()));
        let execution_metadata = serde_json::from_str::<Value>(&terminal_task.metadata_json)
            .ok().and_then(|value| value.get("shell_execution").cloned()).unwrap_or(Value::Null);
        let (status_exit_code, status_signal) = shell_terminal_status_fields(&status);
        let exit_code = execution_metadata.get("exit_code").and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok()).or(status_exit_code);
        let signal = execution_metadata.get("signal").and_then(Value::as_i64)
            .and_then(|signal| i32::try_from(signal).ok()).or(status_signal);
        let mut text = format!(
            "$ {command}\n(cwd={}, status={status})\n\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            cwd.display()
        );
        if stdout_compacted || stderr_compacted {
            text.push_str("\n\n[output compacted: showing bounded head/tail]");
        }
        let mut outcome = ToolOutcome {
        full_output_ref: None,
        data: Some(json!({
            "type": "shell_execution",
            "command": command,
            "cwd": cwd,
            "state": terminal_task.status,
            "exit_code": exit_code,
            "signal": signal,
            "exit_evidence": execution_metadata.get("exit_evidence"),
            "semantic_note": execution_metadata.get("semantic_note"),
            // Preserve the original evidence before the common result writer
            // creates a bounded model preview and a navigable full-output ref.
            "stdout": stdout_full,
            "stderr": stderr_full,
            "output_compacted": false,
        })),
        execution: None,
            text,
            display: if terminal_task.status == "completed" {
                format!("$ {} (completed)", truncate(&command, 60))
            } else {
                format!("$ {} ({})", truncate(&command, 44), terminal_task.status)
            },
            error: terminal_task.status != "completed",
            artifacts: Vec::new(),
            activation: None,
        };
        if let Ok(mut tasks) = state.lock() {
            if let Some(task) = tasks.get_mut(&task_id) {
                if let Some(settlement) = settle_background_output_contract(task, &status) {
                    outcome.artifacts = settlement.artifacts;
                    if settlement.error {
                        outcome.error = true;
                        outcome.text.push_str("\n\n--- output contract ---\n");
                        outcome.text.push_str(&settlement.text);
                        outcome.display = settlement.display;
                    }
                }
            }
        }
        return outcome;
    }
    let evidence_path = PathBuf::from(format!("{}.exit-evidence", status_path.display()));
    let _ = fs::remove_file(&evidence_path);
    let stdout = match File::create(&stdout_path) {
        Ok(file) => file,
        Err(err) => return error(format!("temp stdout failed: {err}")),
    };
    let stderr = match File::create(&stderr_path) {
        Ok(file) => file,
        Err(err) => return error(format!("temp stderr failed: {err}")),
    };
    let mut prepared_sandbox=tool_try!(ctx.policy.process_sandbox().map(|spec|
        spec.prepare_shell(&cwd,&command,verified_read_only.is_some())).transpose().map_err(error));
    let execution_command=tool_try!(prepared_sandbox.as_ref().map(|prepared|prepared.shell_invocation()).transpose().map_err(error))
        .unwrap_or_else(||command.clone());
    let child_command = &mut guarded_command;
    if prepared_sandbox.is_some() {
        crate::process_sandbox::isolate_shell_supervisor_environment(child_command);
        child_command.args(["--noprofile","--norc"]).env_remove("BASH_ENV").env_remove("ENV");
        for (name,_) in std::env::vars_os().filter(|(name,_)|name.to_string_lossy().starts_with("BASH_FUNC_")) {child_command.env_remove(name);}
    }
    if durable_task {
        child_command
            .arg("-c")
            .arg(
                if prepared_sandbox.is_some() { "command=$1\nstatus_path=$2\n/bin/bash --noprofile --norc -o pipefail -c \"$command\"\ncode=$?\ntmp_path=\"${status_path}.tmp.$$\"\nprintf '%s\\n' \"$code\" > \"$tmp_path\"\n/bin/mv -f \"$tmp_path\" \"$status_path\"\nexit \"$code\"" } else { "command=$1\nstatus_path=$2\nbash -o pipefail -lc \"$command\"\ncode=$?\ntmp_path=\"${status_path}.tmp.$$\"\nprintf '%s\\n' \"$code\" > \"$tmp_path\"\nmv -f \"$tmp_path\" \"$status_path\"\nexit \"$code\"" },
            )
            .arg(&task_id)
            .arg(shell_evidence_script(&execution_command))
            .arg(crate::platform_paths::shell_path(&status_path));
        child_command.stdin(Stdio::null());
    } else {
        child_command
            .arg("-o")
            .arg("pipefail")
            .arg(if verified_read_only.is_some() || prepared_sandbox.is_some() { "-c" } else { "-lc" })
            .arg(shell_evidence_script(&execution_command));
    }
    if verified_read_only.is_some() {
        child_command.env_remove("BASH_ENV").env_remove("ENV");
        for (name, _) in std::env::vars_os().filter(|(name,_)|name.to_string_lossy().starts_with("BASH_FUNC_")) {
            child_command.env_remove(name);
        }
    }
    child_command
        .env("NOLA_SHELL_EVIDENCE_PATH", crate::platform_paths::shell_path(&evidence_path))
        .current_dir(&cwd)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    child_command.process_group(0);
    if let Some(prepared)=prepared_sandbox.as_mut() {prepared.mark_dispatched();}
    let mut child = match crate::task_runtime::OwnedShellChild::spawn_owned(child_command) {
        Ok(child) => child,
        Err(err) => {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}return error(format!("Exec failed: {err}"));},
    };
    let started = Instant::now();
    let status = loop {
            if ctx.cancellation.is_requested() {
                let stop = child.stop_and_confirm(Duration::from_secs(2));
                if stop.is_ok() {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}}
                return interrupted_shell_output(&command, &cwd, &stdout_path, &stderr_path,
                    stop,
                    "cancel_requested",
                    "Command cancelled by user".to_string(),
                    "Command cancelled".to_string(),
                );
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() >= Duration::from_secs(timeout) {
                        let stop = child.stop_and_confirm(Duration::from_secs(2));
                        if stop.is_ok() {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}}
                return interrupted_shell_output(&command, &cwd, &stdout_path, &stderr_path,
                            stop,
                            "timeout", format!("Command timed out after {timeout}s"),
                            format!("$ {} timed out after {timeout}s", truncate(&command, 44)),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(err) => {
                    let stop = child.stop_and_confirm(Duration::from_secs(2));
                    if stop.is_ok() {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}}
                return interrupted_shell_output(&command, &cwd, &stdout_path, &stderr_path,
                        stop,
                        "wait_failed", format!("Wait failed: {err}"), "Command wait failed".into());
                }
            }
    };

    if child.residual_processes_stopped().is_some_and(|count| count > 0) {
        let stop = child.stop_and_confirm(Duration::from_secs(2));
        if stop.is_ok() {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}}
        return interrupted_shell_output(&command, &cwd, &stdout_path, &stderr_path, stop,
            "untransferred_processes",
            "The command root exited while untransferred processes were still running in its execution group".into(),
            "Command left untransferred processes".into());
    }
    if child.confirmed_local_stop_scope().is_some() {if let Some(prepared)=prepared_sandbox.as_mut() {prepared.settle();}}
    let confirmed_local_stop = child.confirmed_local_stop_scope().and_then(|scope|
        current_local_execution_identity().map(|identity|
            nola_domain::execution_evidence::LocalExecutionStop { identity, scope }));

    let stdout_full = tool_try!(read_shell_capture(&stdout_path));
    let stderr_full = tool_try!(read_shell_capture(&stderr_path));
    let _ = fs::remove_file(&stdout_path);
    let _ = fs::remove_file(&stderr_path);
    let _ = fs::remove_file(&status_path);
    let code = status.code().unwrap_or(-1);
    let exit_evidence = match prepared_sandbox.as_ref() {
        Some(prepared)=>prepared.exit_evidence(code),None=>read_shell_exit_evidence(&evidence_path,code),
    };
    let _ = fs::remove_file(&evidence_path);
    let (is_error, note) = command_semantic(&requested_command, code, exit_evidence.as_ref());
    #[cfg(unix)]
    let signal = status.signal();
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    let (stdout, stdout_compacted) = compact_head_tail_text(&stdout_full, 8_000);
    let (stderr, stderr_compacted) = compact_head_tail_text(&stderr_full, 8_000);
    let mut text = format!(
        "$ {command}\n(cwd={}, exit={code})\n\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        cwd.display()
    );
    if stdout_compacted || stderr_compacted {
        text.push_str(&format!(
            "\n--- output compaction ---\nstdout_chars_original: {}\nstderr_chars_original: {}\nhead_tail_preserved: true",
            stdout_full.chars().count(),
            stderr_full.chars().count(),
        ));
    }
    if !note.is_empty() {
        text.push_str("\n--- note ---\n");
        text.push_str(&note);
    }
    if is_error {
        let combined = format!("{stdout_full}\n{stderr_full}");
        if let Some(context) = automatic_failure_context(&cwd, &combined, ctx) {
            text.push_str("\n--- automatic failure context ---\n");
            text.push_str(&context);
            text.push_str(
                "\nfailure_context_auto_attached: true\nnext_action: use the attached source snippets and nearest symbols to repair the failure; do not call failure_context again for unchanged output.",
            );
        }
        if let Ok(locations) = failure_locations(&cwd, &combined, 8) {
            if !locations.is_empty() {
                text.push_str("\n--- parsed failure locations ---\n");
                for (path, line_no, message) in locations {
                    text.push_str(&format!(
                        "- {}:{} | {}\n",
                        relative_display(&path, &cwd),
                        line_no,
                        message
                    ));
                }
            }
        }
    }
    let output_summary = stderr_full
        .lines()
        .chain(stdout_full.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!(": {}", truncate(line, 90)))
        .unwrap_or_default();
    let display = if is_error {
        format!("$ {} (exit={code}){output_summary}", truncate(&requested_command, 44))
    } else {
        let explanation = if note.is_empty() { String::new() } else { format!(": {note}") };
        format!("$ {} (exit={code}){explanation}", truncate(&requested_command, 60))
    };
    let mut outcome = ToolOutcome {
        full_output_ref: None,
        data: Some(json!({
            "type": "shell_execution",
            "command": command,
            "cwd": cwd,
            "state": if is_error { "failed" } else { "completed" },
            "exit_code": status.code(),
            "signal": signal,
            "exit_evidence": exit_evidence,
            "semantic_note": note,
            // The text above is a presentation preview, not the saved payload.
            "stdout": stdout_full,
            "stderr": stderr_full,
            "output_compacted": false,
        })),
        execution: confirmed_local_stop.map(|stop| nola_host_protocol::tool_result::ExecutionReceipt {
            state: nola_host_protocol::tool_result::ExecutionState::Unknown,
            local_stop: Some(stop),
            ..Default::default()
        }),
        text,
        display,
        error: is_error,
        artifacts: Vec::new(),
        activation: None,
    };
    if !outcome.error && !declared_outputs.is_empty() {
        let output_states_after = match capture_file_artifact_states(&declared_output_paths) {
            Ok(states) => states,
            Err(observation_error) => {
                // The process already ran. Keep its original captured output
                // and independent local stop, not a fresh empty error result.
                outcome.error = true;
                outcome.display = "Bash returned but its output artifacts could not be observed".into();
                outcome.text = format!("{}\n{}", observation_error.text, outcome.text);
                if let Some(data) = outcome.data.as_mut().and_then(Value::as_object_mut) {
                    data.insert("output_contract_failure".into(), json!({
                        "code":"artifact_state_unavailable", "message":observation_error.text,
                    }));
                }
                preservation.annotate(&mut outcome);
                return outcome;
            }
        };
        for declared in &declared_outputs {
            let observed = observed_file_artifact_operation(
                &declared.path,
                &output_states_before,
                &output_states_after,
            );
            if observed != Some(declared.operation.as_str()) {
                let message = format!(
                        "Bash output artifact postcondition failed for {}: declared={}, observed={}",
                        declared.path.display(),
                        declared.operation,
                        observed.unwrap_or("unchanged")
                    );
                // Execution already happened: retain diagnostics and distinguish
                // the failed artifact contract from the process exit status.
                outcome.error = true;
                outcome.display = "Bash completed but its declared output contract was not satisfied".into();
                outcome.text = format!("{message}\n{}", outcome.text);
                if let Some(data) = outcome.data.as_mut().and_then(Value::as_object_mut) {
                    data.insert("output_contract_failure".into(), json!({
                        "path":declared.path,"declared":declared.operation,
                        "observed":observed.unwrap_or("unchanged"),"message":message
                    }));
                }
                preservation.annotate(&mut outcome);
                return outcome;
            }
        }
        outcome = outcome.with_file_artifact_changes(&output_states_before, &output_states_after);
    }
    preservation.annotate(&mut outcome);
    outcome
}

fn shell_terminal_status_fields(status: &str) -> (Option<i32>, Option<i32>) {
    let status = status.trim();
    if let Some(code) = status.strip_prefix("exited(").and_then(|value| value.strip_suffix(')'))
        .and_then(|value| value.parse::<i32>().ok()).filter(|code| *code >= 0) {
        return (Some(code), None);
    }
    if let Some(signal) = status.strip_prefix("signaled(").and_then(|value| value.strip_suffix(')'))
        .and_then(|value| value.parse::<i32>().ok()).filter(|signal| *signal > 0) {
        return (None, Some(signal));
    }
    (status.parse::<i32>().ok().filter(|code| *code >= 0), None)
}

#[cfg(test)]
#[test]
fn shell_terminal_receipt_preserves_exit_and_signal_without_inventing_success() {
    assert_eq!(shell_terminal_status_fields("exited(0)\n"), (Some(0), None));
    assert_eq!(shell_terminal_status_fields("exited(141)"), (Some(141), None));
    assert_eq!(shell_terminal_status_fields("signaled(13)"), (None, Some(13)));
    assert_eq!(shell_terminal_status_fields("23"), (Some(23), None));
    for status in ["completed", "cancelled", "failed(timeout)", "exited(unknown)", "exited(-1)"] {
        assert_eq!(shell_terminal_status_fields(status), (None, None));
    }
}

struct BackgroundTaskObservation {
    state: String,
    label: String,
    terminal: bool,
    cancel_requested: bool,
    exit_code: Option<i32>,
    ended_at_ms: Option<i64>,
}

fn attach_terminal_submission_guidance(data: &mut Value, task_id: &str) {
    // Submission precedes worker admission. Never manufacture an execution
    // identity from the public task ID or claim that queued means input-ready.
    data["terminal_session"] = json!({
        "state": "requires_observation",
        "next_call": {"tool": "bash_background_status", "arguments": {"task_id": task_id}},
        "instruction": "Before terminal_input, query this original task and copy terminal_session.attempt_id and worker_generation exactly. If absent, the worker is not ready; do not guess identities or restart."
    });
}

#[test]
fn queued_terminal_guidance_requires_observation_without_fabricated_identity() {
    for state in ["starting", "queued_for_slot"] {
        let mut data = json!({"task_id":"task-123", "state":state, "effects_completed":false});
        attach_terminal_submission_guidance(&mut data, "task-123");
        assert_eq!(data["state"], state);
        assert_eq!(data["effects_completed"], false);
        let terminal = &data["terminal_session"];
        assert_eq!(terminal["next_call"]["arguments"]["task_id"], "task-123");
        assert_eq!(terminal["next_call"]["tool"], "bash_background_status");
        assert!(terminal.get("attempt_id").is_none());
        assert!(terminal.get("worker_generation").is_none());
    }
}

fn canonical_background_lifecycle(status: &str) -> Result<(bool, String), String> {
    match status {
        "completed" => Ok((true, status.into())),
        "failed" | "cancelled" => Ok((true, status.into())),
        "queued" | "running" | "retry_waiting" | "suspended" | "waiting_permission" | "waiting_input" => Ok((false, status.into())),
        _ => Err(format!("Unknown canonical Shell task state: {status}")),
    }
}

fn observe_background_task(task: &BackgroundBashTask) -> Result<BackgroundTaskObservation, String> {
    classify_background_task_observation(task, crate::runtime_task_port::get(&task.id))
}

fn classify_background_task_observation(
    task: &BackgroundBashTask,
    observed: Result<Option<nola_application::RuntimeTask>, String>,
) -> Result<BackgroundTaskObservation, String> {
    let record = observed?
        .ok_or("Shell task observation has no record; execution outcome remains unknown")?;
    if record.id != task.id || record.kind != "shell_process" || record.owner_session_id != task.owner_session_id {
        return Err("Shell task observation owner or kind changed".into());
    }
    let (terminal, mut label) = canonical_background_lifecycle(&record.status)?;
    let exit_code = serde_json::from_str::<Value>(&record.metadata_json).ok()
        .and_then(|metadata| metadata.pointer("/shell_execution/exit_code").and_then(Value::as_i64))
        .and_then(|code| i32::try_from(code).ok());
    if terminal {
        if let Some(code) = exit_code { label = format!("{} (exit {code})", record.status); }
    }
    Ok(BackgroundTaskObservation { state: record.status, label, terminal, cancel_requested: record.cancel_requested, exit_code, ended_at_ms: record.ended_at_ms })
}

#[cfg(test)]
#[test]
fn only_canonical_terminal_states_finish_background_waits() {
    for status in ["queued", "running", "retry_waiting", "suspended", "waiting_permission", "waiting_input"] {
        assert!(!canonical_background_lifecycle(status).unwrap().0);
    }
    for status in ["completed", "failed", "cancelled"] {
        assert!(canonical_background_lifecycle(status).unwrap().0);
    }
    for status in ["exited(unknown)", "stopped", "running-ish", "unknown", ""] {
        assert!(canonical_background_lifecycle(status).is_err());
    }
}

#[cfg(test)]
#[test]
fn shell_observation_ignores_terminal_files_and_rejects_missing_or_foreign_records() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("nola.db")).unwrap();
    let record = DbBackgroundTask {
        process_sandbox: None,
        terminal: None,
        id: "queued-observation".into(), project_root: root.path().to_string_lossy().into_owned(),
        command: "not executed".into(), cwd: root.path().to_string_lossy().into_owned(), pid: 0,
        stdout_path: root.path().join("stdout").to_string_lossy().into_owned(),
        stderr_path: root.path().join("stderr").to_string_lossy().into_owned(),
        status_path: root.path().join("status").to_string_lossy().into_owned(),
        started_at_ms: unix_epoch_millis(), owner_session_id: None,
        output_contract_json: None, sop_invocation: None,
    };
    store.save_background_task(&record).unwrap();
    fs::write(&record.status_path, "0\n").unwrap();
    let task = background_task_from_record(record).unwrap();
    let pending = classify_background_task_observation(&task, store.runtime_task(&task.id)).unwrap();
    assert!(!pending.terminal, "status file is not a canonical terminal receipt");
    assert!(classify_background_task_observation(&task, Ok(None)).is_err());
    assert!(classify_background_task_observation(&task, Err("database unavailable".into())).is_err());
    let mut foreign = store.runtime_task(&task.id).unwrap().unwrap();
    foreign.owner_session_id = Some("other-owner".into());
    assert!(classify_background_task_observation(&task, Ok(Some(foreign))).is_err());
    let mut waiting = store.runtime_task(&task.id).unwrap().unwrap();
    waiting.status = "waiting_permission".into();
    waiting.cancel_requested = true;
    let waiting = classify_background_task_observation(&task, Ok(Some(waiting))).unwrap();
    assert!(waiting.cancel_requested);
    assert!(!waiting.terminal, "accepted cancellation is not completed cancellation");
    let mut completed = store.runtime_task(&task.id).unwrap().unwrap();
    completed.status = "completed".into();
    let observation = classify_background_task_observation(&task, Ok(Some(completed.clone()))).unwrap();
    assert!(observation.terminal);
    assert_eq!(observation.exit_code, None, "terminal label does not invent process exit evidence");
    completed.metadata_json = json!({"shell_execution":{"exit_code":23}}).to_string();
    let observation = classify_background_task_observation(&task, Ok(Some(completed))).unwrap();
    assert_eq!(observation.exit_code, Some(23));
}

fn unix_epoch_millis() -> i64 {
    nola_runtime_driver::monotonic_epoch_millis()
}

fn background_task_from_record(record: DbBackgroundTask) -> Result<BackgroundBashTask, String> {
    let output_contract = record
        .output_contract_json
        .as_deref()
        .map(serde_json::from_str::<BackgroundOutputContract>)
        .transpose()
        .map_err(|error| format!("decode background output contract failed: {error}"))?;
    Ok(BackgroundBashTask {
        id: record.id,
        project_root: PathBuf::from(record.project_root),
        command: record.command,
        cwd: PathBuf::from(record.cwd),
        child: None,
        pid: record.pid,
        stdout_path: PathBuf::from(record.stdout_path),
        stderr_path: PathBuf::from(record.stderr_path),
        status_path: PathBuf::from(record.status_path),
        started_at_ms: record.started_at_ms,
        owner_session_id: record.owner_session_id,
        notify_on_completion: true,
        output_contract,
    })
}

fn sync_background_tasks(tasks: &mut HashMap<String, BackgroundBashTask>) -> Result<(), String> {
    for record in crate::runtime_task_port::list_background()? {
        if let Some(task) = tasks.get_mut(&record.id) {
            task.pid = record.pid;
            task.started_at_ms = record.started_at_ms;
        } else {
            tasks.insert(record.id.clone(), background_task_from_record(record)?);
        }
    }
    Ok(())
}

fn hydrate_background_task(
    tasks: &mut HashMap<String, BackgroundBashTask>,
    task_id: &str,
) -> Result<(), String> {
    if tasks.contains_key(task_id) {
        return Ok(());
    }
    let Some(runtime_task) = crate::runtime_task_port::get(task_id)? else {
        return Ok(());
    };
    if runtime_task.kind != "shell_process" {
        return Ok(());
    }
    let record = crate::store::background_task_from_runtime(runtime_task)?;
    tasks.insert(task_id.to_string(), background_task_from_record(record)?);
    Ok(())
}

fn background_task_visible(task: &BackgroundBashTask, ctx: &ToolContext) -> bool {
    task.owner_session_id == ctx.permission_scope.owner_session_id()
        && background_task_within_roots(task, &ctx.allowed_dirs)
}
