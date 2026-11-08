/// Stateful facade for executing tools within one immutable Host context.
pub struct ToolHost<'a> {
    context: &'a ToolContext,
}

/// Internal dispatch authority, never serializable into model tool arguments.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionAuthority {
    LocalPolicy,
    CoreApproved(nola_domain::PermissionProfile),
}

impl ExecutionAuthority {
    fn is_core_approved(self) -> bool { matches!(self, Self::CoreApproved(_)) }
    fn admitted_profile(self) -> Option<nola_domain::PermissionProfile> {
        match self { Self::CoreApproved(profile) => Some(profile), Self::LocalPolicy => None }
    }
}

impl<'a> ToolHost<'a> {
    pub fn new(context: &'a ToolContext) -> Self {
        Self { context }
    }

    pub fn execute(&self, request: &ToolRequest) -> ToolOutcome {
        execute_tool_inner(request, self.context, true, ExecutionAuthority::LocalPolicy, None)
    }
}

pub fn execute_tool(request: &ToolRequest, ctx: &ToolContext) -> ToolOutcome {
    ToolHost::new(ctx).execute(request)
}

pub(crate) fn execute_authorized_tool(request: &ToolRequest, ctx: &ToolContext, authority: ExecutionAuthority) -> ToolOutcome {
    execute_tool_inner(request, ctx, true, authority, None)
}

pub(crate) fn execute_authorized_tool_with_catalog(request: &ToolRequest, ctx: &ToolContext,
    authority: ExecutionAuthority, catalog: Option<&ToolDiscoveryCatalog>) -> ToolOutcome {
    execute_tool_inner(request, ctx, true, authority, catalog)
}

pub fn execute_internal_read_only_tool(request: &ToolRequest, ctx: &ToolContext) -> ToolOutcome {
    if !tool_names_are_read_only(std::slice::from_ref(&request.name)) {
        return error(format!(
            "Internal recovery cannot execute non-read-only tool '{}'",
            request.name
        ));
    }
    execute_tool_inner(request, ctx, false, ExecutionAuthority::LocalPolicy, None)
}

fn conversation_recall(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    use nola_domain::canonical_history::*;
    let Some(session_id) = ctx.permission_scope.owner_session_id() else {
        return error_with_display(
            "conversation_recall requires an active canonical conversation".to_string(),
            "Conversation history is unavailable".to_string(),
        );
    };
    let target_conversation = arg_string(args, "conversation_id").unwrap_or_else(|| session_id.clone());
    let message_id = arg_string(args, "message_id").unwrap_or_default();
    let part_id = arg_string(args, "part_id").unwrap_or_default();
    if !message_id.trim().is_empty() || !part_id.trim().is_empty() {
        if message_id.trim().is_empty() || part_id.trim().is_empty() {
            return error("message_id and part_id must be supplied together".to_string());
        }
        let offset_bytes = args
            .get("offset_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let limit_bytes = args
            .get("limit_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(8_192)
            .clamp(4, 32_768) as usize;
        return match crate::session::query_canonical_history(CanonicalHistoryQuery::Read {
            requester_conversation_id: session_id,
            conversation_id: target_conversation,
            message_id,
            part_id,
            expected_conversation_revision: args.get("expected_conversation_revision").and_then(Value::as_i64),
            offset_bytes,
            limit_bytes,
        }) {
            Ok(response) => ok_data(serde_json::to_value(response).expect("canonical history response"), "Read canonical conversation original text".into()),
            Err(message) => error_with_display(
                format!("Read canonical conversation text failed: {message}"),
                "Conversation history could not be loaded".to_string(),
            ),
        };
    }
    let query = arg_string(args, "query").unwrap_or_default();
    if args.contains_key("conversation_id") {
        return error("Search uses scope and conversation_ids; conversation_id selects the source of an original-text read".into());
    }
    let query = query.trim();
    if query.is_empty() {
        return error("query must contain 1 to 500 characters".to_string());
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let scope = match args.get("scope").and_then(Value::as_str).unwrap_or("current_conversation") {
        "current_conversation" => CanonicalHistoryScope::CurrentConversation,
        "local_user_history" => CanonicalHistoryScope::LocalUserHistory,
        "conversations" => {
            let ids = match args.get("conversation_ids").cloned().map(serde_json::from_value::<Vec<String>>) {
                Some(Ok(ids)) => ids,
                _ => return error("conversation_ids is required for the selected history scope".into()),
            };
            CanonicalHistoryScope::Conversations { conversation_ids: ids }
        }
        _ => return error("unsupported canonical history scope".into()),
    };
    let response = match crate::session::query_canonical_history(CanonicalHistoryQuery::Search {
        requester_conversation_id: session_id, scope, query: query.into(), limit,
        after_conversation_id: arg_string(args, "after_conversation_id"),
    }) {
        Ok(response) => response,
        Err(message) => {
            return error_with_display(
                format!("Load canonical conversation failed: {message}"),
                "Conversation history could not be loaded".to_string(),
            )
        }
    };
    ok_data(serde_json::to_value(response).expect("canonical history response"), "Searched canonical conversation originals".into())
}

fn execute_tool_inner(request: &ToolRequest, ctx: &ToolContext, enforce_availability: bool, authority: ExecutionAuthority, catalog: Option<&ToolDiscoveryCatalog>) -> ToolOutcome {
    if let Err(message)=permission_resolution::validate_bound_file_workspace(request,ctx) {
        return error_data(json!({"error":"bound_file_workspace_required","message":message,"tool_execution_skipped":true}));
    }
    let _admitted_policy = enter_execution_permission_profile(authority.admitted_profile());
    let _execution_scope_guard = ctx.cancellation.enter();
    let _permission_scope_guard = ctx.permission_scope.enter();
    // Core executions carry an effect-local profile; direct Host operations
    // use the session profile. Neither is inferred from auto_approve.
    let allow_all = unrestricted_resource_policy();
    if current_host_permission_session_is_blocked() {
        return error_with_display(
            "Host tool execution is paused pending a mobile permission decision".to_string(),
            "Waiting for permission on the connected device".to_string(),
        );
    }
    let availability = HostCapabilityAvailability::capture(&ctx.skills);
    if enforce_availability && !availability.manifest_valid {
        return error_data(json!({"error":"invalid_host_capability_manifest",
            "tool_execution_skipped":true,
            "message":"Host capability manifest is malformed or unsupported; correct the Host configuration before execution."}));
    }
    if enforce_availability && !availability.enabled(&request.name) {
        return error_with_display(
            format!(
                "Host capability is disabled for this session: {}",
                request.name
            ),
            "Capability unavailable in this session".to_string(),
        );
    }
    if let Some(runtime_tool) = current_device_bridge_runtime_tool(&request.name) {
        if !authority.is_core_approved() {
            return error_data(json!({"error":"Device bridge execution requires a formally admitted Core effect",
                "code":"core_admission_required","tool_execution_skipped":true}));
        }
        if let Some(resolver) =
            current_device_bridge_resolver()
        {
            return resolver(request, &runtime_tool);
        }
        return error_with_display(
            "Device bridge execution required".to_string(),
            "Waiting for this device to run the requested tool".to_string(),
        );
    }
    let Some(spec) = specs_with_skills(&ctx.skills)
        .into_iter()
        .find(|spec| spec.name == request.name)
    else {
        return error(format!("Unknown tool '{}'", request.name));
    };
    if let Some(outcome) = validate_tool_request_with_skills(request, &ctx.skills) {
        return outcome;
    }
    if !authority.is_core_approved() && is_network_sensitive_tool(request) && ctx.policy.network_policy() == "deny" {
        return error_data(json!({"error":"Network access is disabled by configured policy",
            "code":"permission_denied","tool_execution_skipped":true}));
    }
    // Core admits concrete effects once. Direct/internal callers use the same
    // trusted contract and resource observations, not a read_only bypass.
    let local_resources = if authority.is_core_approved() { None } else {
        match permission_resolution::LocalPermissionResources::prepare(ctx,&spec,request) {
            Ok(resources) => Some(resources),
            Err(error) => return permission_resource_execution_failure(error),
        }
    };
    let mut permission_decision = if authority.is_core_approved() {PermissionDecision::Allow}
        else {evaluate_tool_request(ctx.policy.permission_rules(),request)};
    let mut derived_explicit_ask = false;
    if let Some(resources) = &local_resources {
        (permission_decision, derived_explicit_ask) = resources.resource_policy(ctx,request,permission_decision);
    }
    if permission_decision == PermissionDecision::Deny {
        return error_data(json!({"code":"permission_denied", "source":"configured_rule",
            "error":format!("Permission denied by configured rule for tool '{}'",request.name),
            "tool_execution_skipped":true, "next_action":tr(Message::PermissionDeniedGuidance)}));
    }
    let input = Value::Object(request.args.clone());
    if let Some(resources) = &local_resources {
        let explicit_ask = derived_explicit_ask || has_explicit_ask_rule(ctx.policy.permission_rules(),request);
        let ask = !allow_all && permission_decision != PermissionDecision::Allow
            && (explicit_ask || resources.requires_confirmation(&input))
            && (explicit_ask || !resources.approved_in_scope(ctx));
        if ask {
            match permission_resolution::confirm_local_permission(request, resources.allow_conversation) {
                ToolPermissionDecision::Allow => {},
                ToolPermissionDecision::AllowConversation => {
                    if let Err(error) = resources.remember_approval(ctx) { return error_data(json!({"error":error,"tool_execution_skipped":true})); }
                },
                ToolPermissionDecision::Custom(note) => return suspend_for_custom_permission(note,ctx),
                ToolPermissionDecision::ReturnToComposer => return suspend_for_return_to_composer(ctx),
                ToolPermissionDecision::TransportInterrupted(reason) => return permission_transport_interrupted_outcome(reason),
                ToolPermissionDecision::Deny => return error_data(json!({"code":"permission_denied", "source":"user_decision",
                    "error":"Permission denied by user", "tool_execution_skipped":true,
                    "next_action":tr(Message::PermissionDeniedGuidance)})),
            }
        }
    }
    let _local_browser_permission_guard = if let Some(resources) = &local_resources {
        match resources.revalidate(ctx,&input) {
            Ok(expected) => Some(enter_permission_resource_expectations(expected)),
            Err(error) => return permission_resource_execution_failure(error),
        }
    } else {None};
    // Carries only this completed admission to lower-level resource guards.
    let approved_once = true;

    let work_step_transaction = match prepare_tool_work_step_transaction(request, ctx, &spec) {
        Ok(transaction) => transaction,
        Err(outcome) => return outcome,
    };

    let outcome = match request.name.as_str() {
        "read_file" => read_file(&request.args, ctx),
        "environment_probe" => environment_probe(&request.args, ctx),
        "read_many_files" => read_many_files(&request.args, ctx),
        "notebook_read" => notebook_read(&request.args, ctx),
        "notebook_edit" => notebook_edit(&request.args, ctx),
        "file_outline" => file_outline(&request.args, ctx),
        "read_symbol" => read_symbol(&request.args, ctx),
        "repo_map" => repo_map(&request.args, ctx),
        "code_search" => code_search(&request.args, ctx),
        "workspace_symbols" => workspace_symbols(&request.args, ctx),
        "symbol_search" => symbol_search(&request.args, ctx),
        "implementation_search" => implementation_search(&request.args, ctx),
        "code_navigation" => code_navigation(&request.args, ctx),
        "call_graph" => call_graph(&request.args, ctx),
        "code_diagnostics" => code_diagnostics(&request.args, ctx),
        "failure_context" => failure_context(&request.args, ctx),
        "failure_triage" => failure_triage(&request.args, ctx),
        "file_dependencies" => file_dependencies(&request.args, ctx),
        "coding_conventions" => coding_conventions(&request.args, ctx),
        "project_overview" => project_overview(&request.args, ctx),
        "instruction_context" => instruction_context(&request.args, ctx),
        "Skill" | "skill_invoke" => skill_invoke(&request.args, ctx),
        "skill_update" => skill_update(&request.args, ctx),
        "SOP" | "sop_invoke" => sop_invoke(&request.args, ctx),
        "sop_run_update" => sop_run_update(&request.args, ctx),
        "skill_skip" => skill_skip(&request.args),
        "nola_orchestrate" => nola_orchestrate(&request.args, &ctx.skills),
        "agent_context" => agent_context(&request.args, ctx),
        "command_context" => command_context(&request.args, ctx),
        "agent_config_context" => agent_config_context(&request.args, ctx),
        "ci_context" => ci_context(&request.args, ctx),
        "context_pack" => context_pack(&request.args, ctx),
        "focused_context" => focused_context(&request.args, ctx),
        "impact_analysis" => impact_analysis(&request.args, ctx),
        "validation_plan" => validation_plan(&request.args, ctx),
        "test_context" => test_context(&request.args, ctx),
        "format_plan" => format_plan(&request.args, ctx),
        "format_files" => format_files(&request.args, ctx),
        "related_files" => related_files(&request.args, ctx),
        "write_file" => write_file(&request.args, ctx),
        "append_file" => append_file(&request.args, ctx),
        "move_file" => move_file(&request.args, ctx),
        "delete_file" => delete_file(&request.args, ctx),
        "replace_symbol_body" => replace_symbol_body(&request.args, ctx),
        "edit_file" => edit_file(&request.args, ctx),
        "edit_file_lines" => edit_file_lines(&request.args, ctx),
        "multi_edit_file" => multi_edit_file(&request.args, ctx),
        "replace_in_files" => replace_in_files(&request.args, ctx),
        "rename_symbol" => rename_symbol(&request.args, ctx),
        "apply_patch" => apply_patch_file(&request.args, ctx),
        "git_history" => git_history(&request.args, ctx),
        "git_show" => git_show(&request.args, ctx),
        "git_blame" => git_blame(&request.args, ctx),
        "git_diff" => git_diff(&request.args, ctx),
        "workspace_checkpoint" => workspace_checkpoint(&request.args, ctx),
        "workspace_status" => workspace_status(&request.args, ctx),
        "git_commit" => git_commit(&request.args, ctx),
        "git_worktree_list" => git_worktree_list(&request.args, ctx),
        "git_worktree_create" => git_worktree_create(&request.args, ctx),
        "git_worktree_integrate" => git_worktree_integrate(&request.args, ctx),
        "workspace_access" => workspace_access(&request.args, approved_once),
        "diff_review" => diff_review(&request.args, ctx),
        "change_review" => change_review(&request.args, ctx),
        "verification_review" => verification_review(&request.args, ctx),
        "handoff_context" => handoff_context(&request.args, ctx),
        "changed_symbols" => changed_symbols(&request.args, ctx),
        "file_tree" => file_tree(&request.args, ctx),
        "list_dir" => list_dir(&request.args, ctx),
        "find_files" => find_files(&request.args, ctx),
        "find_up" => find_up(&request.args, ctx),
        "glob" => glob_files(&request.args, ctx),
        "grep" => grep_files(&request.args, ctx),
        "tool_search" => tool_search_with_catalog(&request.args, &ctx.skills, catalog),
        "tool_output_list" => tool_output_list_in_store(&request.args, &ctx.output_store),
        "tool_output_read" => tool_output_read_in_store(&request.args, &ctx.output_store),
        "tool_output_search" => tool_output_search_in_store(&request.args, &ctx.output_store),
        "tool_output_find" => tool_output_find_in_store(&request.args, &ctx.output_store),
        "set_thinking_mode" => set_thinking_mode(&request.args),
        "memory_save" => memory_save(&request.args, &ctx.memory_binding),
        "memory_search" => memory_search(&request.args, &ctx.memory_binding),
        "document_search" => document_search(&request.args),
        "request_user_input" => request_user_input(&request.args),
        "agent_task_status" => agent_task_status(&request.args, &ctx.plan_state),
        "goal_complete" => goal_complete(&request.args),
        "propose_agent_plan" => propose_agent_plan_with_state(&request.args, &ctx.plan_state),
        "agent_plan_file" => agent_plan_file(&request.args, ctx),
        "update_agent_todos" => update_agent_todos(&request.args, &ctx.plan_state),
        "bash_command_review" => bash_command_review(&request.args, ctx),
        "bash" => run_bash_with_permission(
            &request.args,
            ctx,
            approved_once,
        ),
        "bash_background_status" => bash_background_status(&request.args, ctx),
        "bash_background_wait" => bash_background_wait(&request.args, ctx),
        "bash_background_stop" => bash_background_stop(&request.args, ctx),
        "terminal_input" => terminal_input(&request.args, ctx),
        "language_service_list" => language_service_tools::list(ctx),
        "language_service_query" => language_service_tools::query(&request.args, ctx),
        "web_fetch" => web_fetch(&request.args),
        "system_time" => system_time(&request.args),
        "web_search" => web_search_engine::execute(&request.args),
        "conversation_recall" => conversation_recall(&request.args, ctx),
        "search_calendar_events" => run_computer_tool("search_calendar_events", &request.args, ctx),
        "create_calendar_event" => run_computer_tool("create_calendar_event", &request.args, ctx),
        "search_reminders" => run_computer_tool("search_reminders", &request.args, ctx),
        "create_reminder" => run_computer_tool("create_reminder", &request.args, ctx),
        "update_reminder" => run_computer_tool("update_reminder", &request.args, ctx),
        name if name.starts_with("chrome_native_") => {
            run_chrome_native_tool(name, &request.args, ctx)
        }
        name if name.starts_with("browser_") => run_browser_tool(name, &request.args, ctx),
        "computer_status" => run_computer_tool("computer_status", &request.args, ctx),
        "computer_observe" => run_computer_tool("computer_observe", &request.args, ctx),
        "computer_list_displays" => run_computer_tool("computer_list_displays", &request.args, ctx),
        "computer_screenshot" => run_computer_tool("computer_screenshot", &request.args, ctx),
        "computer_screenshot_element" => {
            run_computer_tool("computer_screenshot_element", &request.args, ctx)
        }
        "computer_screenshot_window" => {
            run_computer_tool("computer_screenshot_window", &request.args, ctx)
        }
        "computer_read_text" => run_computer_tool("computer_read_text", &request.args, ctx),
        "computer_find_text" => run_computer_tool("computer_find_text", &request.args, ctx),
        "computer_move_to_text" => run_computer_tool("computer_move_to_text", &request.args, ctx),
        "computer_click_text" => run_computer_tool("computer_click_text", &request.args, ctx),
        "computer_select_text" => run_computer_tool("computer_select_text", &request.args, ctx),
        "computer_assert_text" => run_computer_tool("computer_assert_text", &request.args, ctx),
        "computer_wait_for_text" => run_computer_tool("computer_wait_for_text", &request.args, ctx),
        "computer_wait" => run_computer_tool("computer_wait", &request.args, ctx),
        "computer_accessibility_snapshot" => {
            run_computer_tool("computer_accessibility_snapshot", &request.args, ctx)
        }
        "computer_element_at" => run_computer_tool("computer_element_at", &request.args, ctx),
        "computer_press_at" => run_computer_tool("computer_press_at", &request.args, ctx),
        "computer_type_at" => run_computer_tool("computer_type_at", &request.args, ctx),
        "computer_scroll_at" => run_computer_tool("computer_scroll_at", &request.args, ctx),
        "computer_move_to_element" => {
            run_computer_tool("computer_move_to_element", &request.args, ctx)
        }
        "computer_click_element" => run_computer_tool("computer_click_element", &request.args, ctx),
        "computer_press_element" => run_computer_tool("computer_press_element", &request.args, ctx),
        "computer_focus_element" => run_computer_tool("computer_focus_element", &request.args, ctx),
        "computer_perform_element_action" => {
            run_computer_tool("computer_perform_element_action", &request.args, ctx)
        }
        "computer_type_element" => run_computer_tool("computer_type_element", &request.args, ctx),
        "computer_set_element_value" => {
            run_computer_tool("computer_set_element_value", &request.args, ctx)
        }
        "computer_assert_element" => {
            run_computer_tool("computer_assert_element", &request.args, ctx)
        }
        "computer_wait_for_element" => {
            run_computer_tool("computer_wait_for_element", &request.args, ctx)
        }
        "computer_get_position" => run_computer_tool("computer_get_position", &request.args, ctx),
        "computer_move_mouse" => run_computer_tool("computer_move_mouse", &request.args, ctx),
        "computer_mouse_down" => run_computer_tool("computer_mouse_down", &request.args, ctx),
        "computer_mouse_up" => run_computer_tool("computer_mouse_up", &request.args, ctx),
        "computer_click" => run_computer_tool("computer_click", &request.args, ctx),
        "computer_drag" => run_computer_tool("computer_drag", &request.args, ctx),
        "computer_drag_element" => run_computer_tool("computer_drag_element", &request.args, ctx),
        "computer_type_text" => run_computer_tool("computer_type_text", &request.args, ctx),
        "computer_get_clipboard" => run_computer_tool("computer_get_clipboard", &request.args, ctx),
        "computer_set_clipboard" => run_computer_tool("computer_set_clipboard", &request.args, ctx),
        "computer_copy_selection" => {
            run_computer_tool("computer_copy_selection", &request.args, ctx)
        }
        "computer_assert_clipboard" => {
            run_computer_tool("computer_assert_clipboard", &request.args, ctx)
        }
        "computer_wait_for_clipboard" => {
            run_computer_tool("computer_wait_for_clipboard", &request.args, ctx)
        }
        "computer_press_key" => run_computer_tool("computer_press_key", &request.args, ctx),
        "computer_hotkey" => run_computer_tool("computer_hotkey", &request.args, ctx),
        "computer_key_down" => run_computer_tool("computer_key_down", &request.args, ctx),
        "computer_key_up" => run_computer_tool("computer_key_up", &request.args, ctx),
        "computer_scroll" => run_computer_tool("computer_scroll", &request.args, ctx),
        "computer_scroll_element" => {
            run_computer_tool("computer_scroll_element", &request.args, ctx)
        }
        "computer_list_apps" => run_computer_tool("computer_list_apps", &request.args, ctx),
        "computer_assert_app" => run_computer_tool("computer_assert_app", &request.args, ctx),
        "computer_wait_for_app" => run_computer_tool("computer_wait_for_app", &request.args, ctx),
        "computer_list_windows" => run_computer_tool("computer_list_windows", &request.args, ctx),
        "computer_focus_window" => run_computer_tool("computer_focus_window", &request.args, ctx),
        "computer_set_window_bounds" => {
            run_computer_tool("computer_set_window_bounds", &request.args, ctx)
        }
        "computer_perform_window_action" => {
            run_computer_tool("computer_perform_window_action", &request.args, ctx)
        }
        "computer_close_window" => run_computer_tool("computer_close_window", &request.args, ctx),
        "computer_assert_window" => run_computer_tool("computer_assert_window", &request.args, ctx),
        "computer_wait_for_window" => {
            run_computer_tool("computer_wait_for_window", &request.args, ctx)
        }
        "computer_list_menus" => run_computer_tool("computer_list_menus", &request.args, ctx),
        "computer_select_menu" => run_computer_tool("computer_select_menu", &request.args, ctx),
        "computer_activate_app" => run_computer_tool("computer_activate_app", &request.args, ctx),
        "computer_quit_app" => run_computer_tool("computer_quit_app", &request.args, ctx),
        "computer_open_item" => run_computer_tool("computer_open_item", &request.args, ctx),
        _ => error(format!("Unknown tool '{}'", request.name)),
    };
    let outcome = code_evidence::annotate(&request.name, outcome);
    // Persist the same result the caller observes. Provenance added after
    // committing would disappear when the completed call is replayed.
    let outcome = finish_tool_work_step_transaction(work_step_transaction, outcome);
    // If a custom permission response was queued during the tool dispatch (e.g. from
    // request_allowed_dir_access), feed it back through the cancellation steering so
    // the next main-loop iteration presents it as user input.
    PENDING_CUSTOM_PERMISSION.with(|pending| {
        let note = pending.borrow_mut().take();
        if let Some(note) = note {
            if let Err(error) = ctx.cancellation.handoff_user_instruction(note.clone()) {
                *pending.borrow_mut() = Some(note);
                eprintln!("permission instruction handoff remains pending: {error}");
            }
        }
    });
    if !outcome.error {
        if matches!(
            request.name.as_str(),
            "notebook_edit"
                | "format_files"
                | "write_file"
                | "append_file"
                | "move_file"
                | "delete_file"
                | "replace_symbol_body"
                | "edit_file"
                | "edit_file_lines"
                | "multi_edit_file"
                | "replace_in_files"
                | "rename_symbol"
                | "apply_patch"
                | "workspace_checkpoint"
        ) {
            ctx.plan_state.record_mutation();
        } else if request.name == "bash" {
            if let Some(command) = request.args.get("command").and_then(Value::as_str) {
                if is_validation_command_like(command) {
                    ctx.plan_state.record_validation_evidence(format!(
                        "successful validation command after the latest edit: {command} (exit=0)"
                    ));
                }
            }
        }
    }
    if matches!(
        request.name.as_str(),
        "Skill" | "skill_invoke" | "SOP" | "sop_invoke"
    ) {
        // Skill activation is an atomic protocol payload. Truncating it turns
        // the JSON into prose, prevents Core from recording the active Skill,
        // and sends the model into recursive tool_output_read calls. Keep the
        // bounded payload intact; reject an invalid oversized Skill instead.
        let payload_limit = (MAX_SKILL_BYTES as usize).saturating_add(20_000);
        if outcome.text.chars().count() > payload_limit {
            return error(format!(
                "Skill activation payload exceeds the {payload_limit}-character limit"
            ));
        }
        return outcome;
    }
    clamp_outcome_with_writer(
        outcome,
        if matches!(request.name.as_str(), "web_search" | "web_fetch") {
            MAX_WEB_SEARCH_RESULT_CHARS
        } else {
            MAX_TOOL_RESULT_CHARS
        },
        |text| ctx.output_store.persist(text),
    )
}

pub(crate) fn preflight_tool_request(request: &ToolRequest) -> Option<ToolOutcome> {
    validate_tool_request(request)
}

pub(crate) fn preflight_tool_request_with_skills(
    request: &ToolRequest,
    skills: &SkillRuntimeContext,
) -> Option<ToolOutcome> {
    validate_tool_request_with_skills(request, skills)
}

#[cfg(test)]
mod work_step_transaction_execution_tests {
    use super::*;
    use nola_application::{
        ApplicationCommand, ConversationPlan, ConversationPlanCommand, WorkStepTransaction,
        WorkStepTransactionCommand, WorkStepTransactionPhase,
    };
    use std::process::Command;
    fn environment_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::store::test_env_lock()
    }

    fn run_git_test(root: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", arguments);
    }

    #[test]
    fn shell_missing_purpose_is_recoverable_without_expanding_workspace() {
        let _guard = environment_lock();
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let allowed = root.join("allowed");
        let outside = root.join("outside");
        fs::create_dir_all(&allowed).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let previous_db = std::env::var_os("NOLA_DB_PATH");
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![allowed.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".to_string(), Vec::new()),
            permission_scope: SessionPermissionScope::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        // Permission approval does not waive the tool contract. A missing
        // declaration is recoverable without executing the command.
        let mut request = ToolRequest {
            id: "command-only-approved".into(),
            name: "bash".into(),
            args: json!({"command":"printf contract-ok"}).as_object().unwrap().clone(),
        };
        assert!(validate_tool_request(&request).is_some());
        let rejected = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
        assert!(rejected.error);
        assert!(rejected.text.contains("tool_execution_skipped: true"));
        assert!(rejected.text.contains("operation_kind"));
        request.args.insert("operation_kind".into(), json!("execute"));
        assert!(validate_tool_request(&request).is_none());
        let outcome = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.text.contains("contract-ok"));
        assert!(outcome.artifacts.is_empty());
        let data = outcome.data.as_ref().expect("structured shell result");
        assert_eq!(data["type"], "shell_execution");
        assert_eq!(data["state"], "completed");
        assert_eq!(data["exit_code"], 0);
        assert_eq!(data["stdout"], "contract-ok");

        let failed_request = ToolRequest {
            id: "command-only-failed".into(),
            name: "bash".into(),
            args: json!({"command":"/usr/bin/false", "operation_kind":"execute"}).as_object().unwrap().clone(),
        };
        let failed = execute_authorized_tool(&failed_request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
        assert!(failed.error, "{}", failed.text);
        let data = failed.data.as_ref().expect("structured failed shell result");
        assert_eq!(data["state"], "failed");
        assert_eq!(data["exit_code"], 1);

        let outside_request = ToolRequest {
            id: "command-only-outside".into(),
            name: "bash".into(),
            args: json!({"command":"printf must-not-run", "cwd":outside, "operation_kind":"execute"})
                .as_object().unwrap().clone(),
        };
        let refused = execute_authorized_tool(
            &outside_request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious),
        );
        assert!(!refused.error, "{}", refused.text);
        assert!(refused.text.contains("must-not-run"));
        assert_eq!(ctx.allowed_dirs, vec![allowed]);
        match previous_db {
            Some(value) => std::env::set_var("NOLA_DB_PATH", value),
            None => std::env::remove_var("NOLA_DB_PATH"),
        }
    }

    #[test]
    fn mutating_tool_is_checkpointed_committed_and_not_replayed() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-work-step-tool-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
        run_git_test(&root, &["init", "--quiet"]);
        run_git_test(
            &root,
            &["config", "user.email", "nola-test@example.invalid"],
        );
        run_git_test(&root, &["config", "user.name", "Nola Test"]);
        fs::write(root.join("tracked.txt"), "baseline\n").unwrap();
        run_git_test(&root, &["add", "tracked.txt"]);
        run_git_test(&root, &["commit", "--quiet", "-m", "baseline"]);

        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-test");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let target = root.join("events.txt");
        let request = ToolRequest {
            id: "call-append-1".to_string(),
            name: "append_file".to_string(),
            args: Map::from_iter([
                (
                    "path".to_string(),
                    Value::String(target.to_string_lossy().into_owned()),
                ),
                ("content".to_string(), Value::String("once\n".to_string())),
                ("create".to_string(), Value::Bool(true)),
            ]),
        };

        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "once\n");

        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "once\n");

        let mut application = NolaStore::open_default().unwrap().application().unwrap();
        let transaction: Option<WorkStepTransaction> = application
            .execute(ApplicationCommand::WorkStepTransaction {
                command: WorkStepTransactionCommand::Get {
                    id: "work-step:session-test:call-append-1".to_string(),
                },
            })
            .unwrap();
        let transaction = transaction.unwrap();
        assert_eq!(transaction.phase, WorkStepTransactionPhase::Committed);
        assert!(transaction.workspace_checkpoint_id.is_some());

        let read = execute_tool(
            &ToolRequest {
                id: "call-read-1".to_string(),
                name: "read_file".to_string(),
                args: Map::from_iter([(
                    "path".to_string(),
                    Value::String(target.to_string_lossy().into_owned()),
                )]),
            },
            &ctx,
        );
        assert!(!read.error, "{}", read.text);
        let read_transaction: Option<WorkStepTransaction> = application
            .execute(ApplicationCommand::WorkStepTransaction {
                command: WorkStepTransactionCommand::Get {
                    id: "work-step:session-test:call-read-1".to_string(),
                },
            })
            .unwrap();
        assert!(read_transaction.is_none());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_write_file_artifact_is_atomically_linked_replayed_and_restored_for_the_model() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-plan-artifact-tool-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-plan-artifact").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "write-result",
                "content": "Write the requested result file",
                "activeForm": "Writing the requested result file",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["write_file"],
                "successCriteria": ["The file exists with the requested contents"],
                "expectedArtifactRefs": ["result.txt"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();

        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-plan-artifact");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state: plan_state.clone(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let target = root.join("result.txt");
        let request = ToolRequest {
            id: "call-write-artifact".to_string(),
            name: "write_file".to_string(),
            args: Map::from_iter([
                (
                    "path".to_string(),
                    Value::String(target.to_string_lossy().into_owned()),
                ),
                (
                    "content".to_string(),
                    Value::String("canonical artifact\n".to_string()),
                ),
            ]),
        };

        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "canonical artifact\n");
        assert_eq!(first.artifacts.len(), 1);
        first.artifacts[0].validate().unwrap();
        let artifact_uri = first.artifacts[0].uri.clone();
        assert_eq!(first.artifacts[0].operation.as_deref(), Some("write"));

        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(replay.artifacts, first.artifacts);
        assert_eq!(fs::read_to_string(&target).unwrap(), "canonical artifact\n");

        {
            let mut application = NolaStore::open_default().unwrap().application().unwrap();
            let transaction: Option<WorkStepTransaction> = application
                .execute(ApplicationCommand::WorkStepTransaction {
                    command: WorkStepTransactionCommand::Get {
                        id: "work-step:session-plan-artifact:call-write-artifact".to_string(),
                    },
                })
                .unwrap();
            let transaction = transaction.unwrap();
            assert_eq!(transaction.phase, WorkStepTransactionPhase::Committed);
            assert_eq!(transaction.result.as_ref().unwrap()["artifacts"][0]["uri"], artifact_uri);

            let plan: Option<ConversationPlan> = application
                .execute(ApplicationCommand::ConversationPlan {
                    command: ConversationPlanCommand::Get {
                        conversation_id: "session-plan-artifact".to_string(),
                    },
                })
                .unwrap();
            let plan = plan.unwrap();
            assert_eq!(plan.revision, 2);
            assert_eq!(
                plan.todos[0].work_step_transaction_refs,
                vec!["work-step:session-plan-artifact:call-write-artifact"]
            );
            assert_eq!(plan.todos[0].artifact_refs, vec![artifact_uri.clone()]);
        }

        let restored_state = AgentPlanState::default();
        assert!(restored_state.bind_session("session-plan-artifact").unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains(&artifact_uri));
        assert!(restored_context.contains("call-write-artifact"));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_notebook_move_and_delete_artifacts_follow_real_effects_and_survive_reopen() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-file-artifact-suite-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-file-artifact-suite").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "mutate-files",
                "content": "Apply the requested file mutations",
                "activeForm": "Applying the requested file mutations",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["append_file", "notebook_edit", "move_file", "delete_file"],
                "successCriteria": ["Each file mutation is committed and auditable"],
                "expectedArtifactRefs": ["mutated files"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-file-artifact-suite");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };

        let source = root.join("source.txt");
        let append = execute_tool(
            &ToolRequest {
                id: "call-append-artifact".to_string(),
                name: "append_file".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(source.to_string_lossy().into_owned())),
                    ("content".to_string(), Value::String("first line\n".to_string())),
                    ("create".to_string(), Value::Bool(true)),
                ]),
            },
            &ctx,
        );
        assert!(!append.error, "{}", append.text);
        assert_eq!(append.artifacts.len(), 1);
        assert_eq!(append.artifacts[0].operation.as_deref(), Some("append"));
        append.artifacts[0].validate().unwrap();
        let source_uri = append.artifacts[0].uri.clone();

        let notebook_path = root.join("notes.ipynb");
        fs::write(
            &notebook_path,
            "{\"cells\":[],\"metadata\":{},\"nbformat\":4,\"nbformat_minor\":5}\n",
        )
        .unwrap();
        let notebook_read = execute_tool(
            &ToolRequest {
                id: "call-read-notebook-before-edit".to_string(),
                name: "read_file".to_string(),
                args: Map::from_iter([(
                    "path".to_string(),
                    Value::String(notebook_path.to_string_lossy().into_owned()),
                )]),
            },
            &ctx,
        );
        assert!(!notebook_read.error, "{}", notebook_read.text);
        let notebook_digest = file_sha256_hex(&notebook_path).unwrap();
        let notebook = execute_tool(
            &ToolRequest {
                id: "call-notebook-artifact".to_string(),
                name: "notebook_edit".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(notebook_path.to_string_lossy().into_owned())),
                    ("edit_mode".to_string(), Value::String("insert".to_string())),
                    ("cell_index".to_string(), Value::Number(0.into())),
                    ("cell_type".to_string(), Value::String("markdown".to_string())),
                    ("source".to_string(), Value::String("# Notes".to_string())),
                    ("dry_run".to_string(), Value::Bool(false)),
                    ("expected_sha256".to_string(), Value::String(notebook_digest)),
                ]),
            },
            &ctx,
        );
        assert!(!notebook.error, "{}", notebook.text);
        assert_eq!(notebook.artifacts.len(), 1);
        assert_eq!(notebook.artifacts[0].operation.as_deref(), Some("edit"));
        let notebook_uri = notebook.artifacts[0].uri.clone();
        assert!(fs::read_to_string(&notebook_path).unwrap().contains("# Notes"));

        let destination = root.join("destination.txt");
        let source_digest = file_sha256_hex(&source).unwrap();
        let moved = execute_tool(
            &ToolRequest {
                id: "call-move-artifact".to_string(),
                name: "move_file".to_string(),
                args: Map::from_iter([
                    ("source".to_string(), Value::String(source.to_string_lossy().into_owned())),
                    ("destination".to_string(), Value::String(destination.to_string_lossy().into_owned())),
                    ("expected_sha256".to_string(), Value::String(source_digest)),
                ]),
            },
            &ctx,
        );
        assert!(!moved.error, "{}", moved.text);
        assert_eq!(moved.artifacts.len(), 2);
        assert_eq!(moved.artifacts[0].uri, source_uri);
        assert_eq!(moved.artifacts[0].operation.as_deref(), Some("delete"));
        assert_eq!(moved.artifacts[1].operation.as_deref(), Some("move"));
        let destination_uri = moved.artifacts[1].uri.clone();
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "first line\n");

        let destination_digest = file_sha256_hex(&destination).unwrap();
        let deleted = execute_tool(
            &ToolRequest {
                id: "call-delete-artifact".to_string(),
                name: "delete_file".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(destination.to_string_lossy().into_owned())),
                    ("expected_sha256".to_string(), Value::String(destination_digest)),
                ]),
            },
            &ctx,
        );
        assert!(!deleted.error, "{}", deleted.text);
        assert_eq!(deleted.artifacts.len(), 1);
        assert_eq!(deleted.artifacts[0].uri, destination_uri);
        assert_eq!(deleted.artifacts[0].operation.as_deref(), Some("delete"));
        assert!(!destination.exists());

        let mut application = NolaStore::open_default().unwrap().application().unwrap();
        let plan: Option<ConversationPlan> = application
            .execute(ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "session-file-artifact-suite".to_string(),
                },
            })
            .unwrap();
        let plan = plan.unwrap();
        assert_eq!(plan.revision, 5);
        assert_eq!(plan.todos[0].work_step_transaction_refs.len(), 4);
        assert_eq!(
            plan.todos[0].artifact_refs,
            vec![source_uri, notebook_uri.clone(), destination_uri]
        );
        drop(application);

        let restored_state = AgentPlanState::default();
        assert!(restored_state.bind_session("session-file-artifact-suite").unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains(&notebook_uri));
        assert!(restored_context.contains("call-delete-artifact"));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn structured_single_and_multi_file_edits_link_only_the_real_touched_resources() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-structured-edit-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        let first_path = root.join("first.rs");
        let second_path = root.join("second.rs");
        fs::write(&first_path, "fn alpha() {\n    let old = 1;\n}\n").unwrap();
        fs::write(
            &second_path,
            "fn caller() {\n    alpha();\n    let old = 1;\n}\n",
        )
        .unwrap();

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-structured-edits").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "structured-edits",
                "content": "Apply structured edits to the source files",
                "activeForm": "Applying structured edits to the source files",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["replace_symbol_body", "edit_file", "edit_file_lines", "multi_edit_file", "replace_in_files", "rename_symbol"],
                "successCriteria": ["All concrete changed files are linked as artifacts"],
                "expectedArtifactRefs": ["first.rs", "second.rs"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-structured-edits");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        for (index, path) in [&first_path, &second_path].into_iter().enumerate() {
            let read = execute_tool(
                &ToolRequest {
                    id: format!("call-read-structured-{index}"),
                    name: "read_file".to_string(),
                    args: Map::from_iter([(
                        "path".to_string(),
                        Value::String(path.to_string_lossy().into_owned()),
                    )]),
                },
                &ctx,
            );
            assert!(!read.error, "{}", read.text);
        }

        let replace_body = execute_tool(
            &ToolRequest {
                id: "call-replace-body-artifact".to_string(),
                name: "replace_symbol_body".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(first_path.to_string_lossy().into_owned())),
                    ("symbol".to_string(), Value::String("alpha".to_string())),
                    ("new_text".to_string(), Value::String("fn alpha() {\n    let old = 2;\n}\n".to_string())),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!replace_body.error, "{}", replace_body.text);
        assert_eq!(replace_body.artifacts.len(), 1);

        let edit = execute_tool(
            &ToolRequest {
                id: "call-edit-file-artifact".to_string(),
                name: "edit_file".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(first_path.to_string_lossy().into_owned())),
                    ("old_text".to_string(), Value::String("let old = 2;".to_string())),
                    ("new_text".to_string(), Value::String("let old = 3;".to_string())),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!edit.error, "{}", edit.text);
        assert_eq!(edit.artifacts.len(), 1);

        // Position-based editing needs an actual observation of the new
        // version; the preceding exact preimage replacement is not a read.
        let line_read = execute_tool(&ToolRequest {
            id: "call-read-new-line-before-line-edit".into(), name: "read_file".into(),
            args: json!({"path": first_path, "start_line": 2, "line_count": 1}).as_object().unwrap().clone(),
        }, &ctx);
        assert!(!line_read.error, "{}", line_read.text);
        assert!(line_read.text.contains("let old = 3;"));

        let line_edit = execute_tool(
            &ToolRequest {
                id: "call-line-edit-artifact".to_string(),
                name: "edit_file_lines".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(first_path.to_string_lossy().into_owned())),
                    ("start_line".to_string(), Value::Number(2.into())),
                    ("line_count".to_string(), Value::Number(1.into())),
                    ("new_text".to_string(), Value::String("    let old = 4;".to_string())),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!line_edit.error, "{}", line_edit.text);
        assert_eq!(line_edit.artifacts.len(), 1);

        let multi_edit = execute_tool(
            &ToolRequest {
                id: "call-multi-edit-artifact".to_string(),
                name: "multi_edit_file".to_string(),
                args: Map::from_iter([
                    ("path".to_string(), Value::String(second_path.to_string_lossy().into_owned())),
                    ("edits".to_string(), json!([
                        {"old_text":"alpha();","new_text":"alpha(); // call"},
                        {"old_text":"let old = 1;","new_text":"let old = 2;"}
                    ])),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!multi_edit.error, "{}", multi_edit.text);
        assert_eq!(multi_edit.artifacts.len(), 1);

        let replace_many = execute_tool(
            &ToolRequest {
                id: "call-replace-many-artifact".to_string(),
                name: "replace_in_files".to_string(),
                args: Map::from_iter([
                    ("root".to_string(), Value::String(root.to_string_lossy().into_owned())),
                    ("old_text".to_string(), Value::String("old".to_string())),
                    ("new_text".to_string(), Value::String("value".to_string())),
                    ("glob".to_string(), Value::String("**/*.rs".to_string())),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!replace_many.error, "{}", replace_many.text);
        assert_eq!(replace_many.artifacts.len(), 2);

        let rename = execute_tool(
            &ToolRequest {
                id: "call-rename-artifact".to_string(),
                name: "rename_symbol".to_string(),
                args: Map::from_iter([
                    ("root".to_string(), Value::String(root.to_string_lossy().into_owned())),
                    ("old_symbol".to_string(), Value::String("alpha".to_string())),
                    ("new_symbol".to_string(), Value::String("beta".to_string())),
                    ("glob".to_string(), Value::String("**/*.rs".to_string())),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!rename.error, "{}", rename.text);
        assert_eq!(rename.artifacts.len(), 2);
        assert!(rename
            .artifacts
            .iter()
            .all(|artifact| artifact.operation.as_deref() == Some("edit")));

        let first_uri = url::Url::from_file_path(first_path.canonicalize().unwrap())
            .unwrap()
            .to_string();
        let second_uri = url::Url::from_file_path(second_path.canonicalize().unwrap())
            .unwrap()
            .to_string();
        let first = fs::read_to_string(&first_path).unwrap();
        let second = fs::read_to_string(&second_path).unwrap();
        assert!(first.contains("fn beta()"));
        assert!(first.contains("let value = 4;"));
        assert!(second.contains("beta(); // call"));
        assert!(second.contains("let value = 2;"));

        let mut application = NolaStore::open_default().unwrap().application().unwrap();
        let plan: Option<ConversationPlan> = application
            .execute(ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "session-structured-edits".to_string(),
                },
            })
            .unwrap();
        let plan = plan.unwrap();
        assert_eq!(plan.revision, 7);
        assert_eq!(plan.todos[0].work_step_transaction_refs.len(), 6);
        assert_eq!(plan.todos[0].artifact_refs, vec![first_uri, second_uri.clone()]);
        drop(application);

        let restored_state = AgentPlanState::default();
        assert!(restored_state.bind_session("session-structured-edits").unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains(&second_uri));
        assert!(restored_context.contains("call-rename-artifact"));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_patch_declares_created_edited_and_deleted_resources_and_replays_once() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-patch-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        let edited_path = root.join("edited.txt");
        let deleted_path = root.join("deleted.txt");
        let created_path = root.join("created.txt");
        fs::write(&edited_path, "old\n").unwrap();
        fs::write(&deleted_path, "remove\n").unwrap();

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-patch-artifacts").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "apply-patch",
                "content": "Apply the requested multi-file patch",
                "activeForm": "Applying the requested multi-file patch",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["apply_patch"],
                "successCriteria": ["Created, edited and deleted resources match the patch"],
                "expectedArtifactRefs": ["created.txt", "edited.txt", "deleted.txt"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-patch-artifacts");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        for (index, path) in [&edited_path, &deleted_path].into_iter().enumerate() {
            let read = execute_tool(
                &ToolRequest {
                    id: format!("call-read-patch-{index}"),
                    name: "read_file".to_string(),
                    args: Map::from_iter([(
                        "path".to_string(),
                        Value::String(path.to_string_lossy().into_owned()),
                    )]),
                },
                &ctx,
            );
            assert!(!read.error, "{}", read.text);
        }
        let patch = concat!(
            "--- /dev/null\n",
            "+++ b/created.txt\n",
            "@@ -0,0 +1 @@\n",
            "+created\n",
            "--- a/deleted.txt\n",
            "+++ /dev/null\n",
            "@@ -1 +0,0 @@\n",
            "-remove\n",
            "--- a/edited.txt\n",
            "+++ b/edited.txt\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "+new\n",
        );
        let request = ToolRequest {
            id: "call-apply-patch-artifacts".to_string(),
            name: "apply_patch".to_string(),
            args: Map::from_iter([
                ("root".to_string(), Value::String(root.to_string_lossy().into_owned())),
                ("patch".to_string(), Value::String(patch.to_string())),
                ("dry_run".to_string(), Value::Bool(false)),
            ]),
        };
        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&created_path).unwrap(), "created\n");
        assert_eq!(fs::read_to_string(&edited_path).unwrap(), "new\n");
        assert!(!deleted_path.exists());
        assert_eq!(first.artifacts.len(), 3);
        assert_eq!(
            first
                .artifacts
                .iter()
                .map(|artifact| artifact.operation.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["create", "delete", "edit"]
        );
        for artifact in &first.artifacts {
            artifact.validate().unwrap();
        }

        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(replay.artifacts, first.artifacts);
        assert_eq!(fs::read_to_string(&created_path).unwrap(), "created\n");
        assert_eq!(fs::read_to_string(&edited_path).unwrap(), "new\n");
        assert!(!deleted_path.exists());

        let artifact_uris = first
            .artifacts
            .iter()
            .map(|artifact| artifact.uri.clone())
            .collect::<Vec<_>>();
        let mut application = NolaStore::open_default().unwrap().application().unwrap();
        let plan: Option<ConversationPlan> = application
            .execute(ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "session-patch-artifacts".to_string(),
                },
            })
            .unwrap();
        let plan = plan.unwrap();
        assert_eq!(plan.revision, 2);
        assert_eq!(plan.todos[0].work_step_transaction_refs.len(), 1);
        assert_eq!(plan.todos[0].artifact_refs, artifact_uris);
        drop(application);

        let restored_state = AgentPlanState::default();
        assert!(restored_state.bind_session("session-patch-artifacts").unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains("created.txt"));
        assert!(restored_context.contains("deleted.txt"));
        assert!(restored_context.contains("edited.txt"));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_git_file_mutation_persists_a_bounded_preimage_and_replays_no_write() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-non-git-preimage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        let target = root.join("notes.txt");
        fs::write(&target, "baseline\n").unwrap();

        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-non-git-preimage");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let read = execute_tool(
            &ToolRequest {
                id: "read-before-preimage".to_string(),
                name: "read_file".to_string(),
                args: Map::from_iter([(
                    "path".to_string(),
                    Value::String(target.to_string_lossy().into_owned()),
                )]),
            },
            &ctx,
        );
        assert!(!read.error, "{}", read.text);
        let request = ToolRequest {
            id: "append-with-preimage".to_string(),
            name: "append_file".to_string(),
            args: Map::from_iter([
                (
                    "path".to_string(),
                    Value::String(target.to_string_lossy().into_owned()),
                ),
                ("content".to_string(), Value::String("once\n".to_string())),
            ]),
        };
        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "baseline\nonce\n");

        let store = NolaStore::open_default().unwrap();
        let transaction = crate::work_step_port::work_step_transaction(
            "work-step:session-non-git-preimage:append-with-preimage",
        )
            .unwrap()
            .unwrap();
        assert_eq!(transaction.phase, WorkStepTransactionPhase::Committed);
        let checkpoint_id = transaction
            .workspace_checkpoint_id
            .as_deref()
            .expect("non-Git transaction preimage id");
        let root_canonical = root.canonicalize().unwrap();
        let checkpoint = store
            .list_workspace_checkpoints(&root_canonical.to_string_lossy())
            .unwrap()
            .into_iter()
            .find(|checkpoint| checkpoint.id == checkpoint_id)
            .expect("persisted file preimage checkpoint");
        assert_eq!(checkpoint.source, "file_preimage");
        let manifest: Value = serde_json::from_str(&checkpoint.status).unwrap();
        assert_eq!(manifest["kind"], "file_preimage");
        assert_eq!(manifest["entries"][0]["existed"], true);
        assert_eq!(
            fs::read_to_string(Path::new(&checkpoint.revision).join("0.bin")).unwrap(),
            "baseline\n"
        );
        workspace_checkpoint_store().lock().unwrap().clear();

        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "baseline\nonce\n");

        let preview = execute_tool(
            &ToolRequest {
                id: "preview-file-preimage".to_string(),
                name: "workspace_checkpoint".to_string(),
                args: Map::from_iter([
                    (
                        "root".to_string(),
                        Value::String(root_canonical.to_string_lossy().into_owned()),
                    ),
                    ("action".to_string(), Value::String("restore".to_string())),
                    (
                        "checkpoint_id".to_string(),
                        Value::String(checkpoint_id.to_string()),
                    ),
                    ("dry_run".to_string(), Value::Bool(true)),
                ]),
            },
            &ctx,
        );
        assert!(!preview.error, "{}", preview.text);
        assert!(preview.text.contains("No files changed"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "baseline\nonce\n");

        let restore = execute_tool(
            &ToolRequest {
                id: "restore-file-preimage".to_string(),
                name: "workspace_checkpoint".to_string(),
                args: Map::from_iter([
                    (
                        "root".to_string(),
                        Value::String(root_canonical.to_string_lossy().into_owned()),
                    ),
                    ("action".to_string(), Value::String("restore".to_string())),
                    (
                        "checkpoint_id".to_string(),
                        Value::String(checkpoint_id.to_string()),
                    ),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!restore.error, "{}", restore.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "baseline\n");
        assert_eq!(restore.artifacts.len(), 1);
        assert_eq!(restore.artifacts[0].operation.as_deref(), Some("edit"));
        restore.artifacts[0].validate().unwrap();

        let repeated_restore = execute_tool(
            &ToolRequest {
                id: "repeat-restore-file-preimage".to_string(),
                name: "workspace_checkpoint".to_string(),
                args: Map::from_iter([
                    (
                        "root".to_string(),
                        Value::String(root_canonical.to_string_lossy().into_owned()),
                    ),
                    ("action".to_string(), Value::String("restore".to_string())),
                    (
                        "checkpoint_id".to_string(),
                        Value::String(checkpoint_id.to_string()),
                    ),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!repeated_restore.error, "{}", repeated_restore.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "baseline\n");
        assert!(repeated_restore.artifacts.is_empty());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_git_preimage_restore_removes_a_file_created_by_the_tool_call() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-non-git-created-preimage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        let target = root.join("created.txt");
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-non-git-created-preimage");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let write = execute_tool(
            &ToolRequest {
                id: "create-with-preimage".to_string(),
                name: "write_file".to_string(),
                args: Map::from_iter([
                    (
                        "path".to_string(),
                        Value::String(target.to_string_lossy().into_owned()),
                    ),
                    (
                        "content".to_string(),
                        Value::String("created\n".to_string()),
                    ),
                ]),
            },
            &ctx,
        );
        assert!(!write.error, "{}", write.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "created\n");

        let transaction = crate::work_step_port::work_step_transaction(
            "work-step:session-non-git-created-preimage:create-with-preimage",
        )
            .unwrap()
            .unwrap();
        let checkpoint_id = transaction.workspace_checkpoint_id.unwrap();
        let restore = execute_tool(
            &ToolRequest {
                id: "restore-created-file-preimage".to_string(),
                name: "workspace_checkpoint".to_string(),
                args: Map::from_iter([
                    (
                        "root".to_string(),
                        Value::String(root.canonicalize().unwrap().to_string_lossy().into_owned()),
                    ),
                    ("action".to_string(), Value::String("restore".to_string())),
                    ("checkpoint_id".to_string(), Value::String(checkpoint_id)),
                    ("dry_run".to_string(), Value::Bool(false)),
                ]),
            },
            &ctx,
        );
        assert!(!restore.error, "{}", restore.text);
        assert!(!target.exists());
        assert_eq!(restore.artifacts.len(), 1);
        assert_eq!(restore.artifacts[0].operation.as_deref(), Some("delete"));
        restore.artifacts[0].validate().unwrap();

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_git_preimage_budget_failure_prevents_the_file_side_effect() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-non-git-large-preimage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        let target = root.join("large.bin");
        let file = fs::File::create(&target).unwrap();
        file.set_len(MAX_PREIMAGE_FILE_BYTES + 1).unwrap();
        drop(file);
        let original_len = fs::metadata(&target).unwrap().len();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-non-git-large-preimage");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let read = execute_tool(
            &ToolRequest {
                id: "read-large-before-preimage".to_string(),
                name: "read_file".to_string(),
                args: Map::from_iter([
                    (
                        "path".to_string(),
                        Value::String(target.to_string_lossy().into_owned()),
                    ),
                    ("max_bytes".to_string(), Value::Number(1024.into())),
                ]),
            },
            &ctx,
        );
        assert!(!read.error, "{}", read.text);
        let append = execute_tool(
            &ToolRequest {
                id: "append-over-preimage-budget".to_string(),
                name: "append_file".to_string(),
                args: Map::from_iter([
                    (
                        "path".to_string(),
                        Value::String(target.to_string_lossy().into_owned()),
                    ),
                    ("content".to_string(), Value::String("x".to_string())),
                ]),
            },
            &ctx,
        );
        assert!(append.error);
        assert!(append.text.contains("bounded recovery budget"));
        assert_eq!(fs::metadata(&target).unwrap().len(), original_len);

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_format_files_links_only_changed_artifacts_and_replays_without_formatting_again() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-format-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"format_artifact_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let target = root.join("src/lib.rs");
        fs::write(&target, "pub fn answer()->i32{42}\n").unwrap();

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-format-artifacts").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "format-source",
                "content": "Format the source file",
                "activeForm": "Formatting the source file",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["format_files"],
                "successCriteria": ["The source is formatted and linked"],
                "expectedArtifactRefs": ["src/lib.rs"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-format-artifacts");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let request = ToolRequest {
            id: "call-format-artifacts".to_string(),
            name: "format_files".to_string(),
            args: Map::from_iter([
                (
                    "root".to_string(),
                    Value::String(root.to_string_lossy().into_owned()),
                ),
                ("paths".to_string(), json!([target.to_string_lossy()])),
                ("dry_run".to_string(), Value::Bool(false)),
            ]),
        };

        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "pub fn answer() -> i32 {\n    42\n}\n");
        assert_eq!(first.artifacts.len(), 1);
        assert_eq!(first.artifacts[0].operation.as_deref(), Some("edit"));
        first.artifacts[0].validate().unwrap();
        let artifact_uri = first.artifacts[0].uri.clone();

        fs::write(&target, "pub fn answer()->i32{7}\n").unwrap();
        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(replay.artifacts, first.artifacts);
        assert_eq!(fs::read_to_string(&target).unwrap(), "pub fn answer()->i32{7}\n");

        let restored_state = AgentPlanState::default();
        assert!(restored_state.bind_session("session-format-artifacts").unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains(&artifact_uri));
        assert!(restored_context.contains("call-format-artifacts"));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_checkpoint_restore_links_exact_create_edit_delete_artifacts_and_replays_once() {
        let _guard = environment_lock();
        let base = std::env::temp_dir().join(format!(
            "nola-git-restore-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("workspace");
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", base.join("state").join("nola.db"));
        run_git_test(&root, &["init", "--quiet"]);
        run_git_test(
            &root,
            &["config", "user.email", "nola-test@example.invalid"],
        );
        run_git_test(&root, &["config", "user.name", "Nola Test"]);
        let edited = root.join("edited.txt");
        let restored = root.join("restored.txt");
        let removed = root.join("new.txt");
        fs::write(&edited, "baseline edit\n").unwrap();
        fs::write(&restored, "baseline restored\n").unwrap();
        run_git_test(&root, &["add", "."]);
        run_git_test(&root, &["commit", "--quiet", "-m", "baseline"]);

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-git-restore-artifacts").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "restore-workspace",
                "content": "Restore the workspace checkpoint",
                "activeForm": "Restoring the workspace checkpoint",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["workspace_checkpoint"],
                "successCriteria": ["The checkpoint resources are restored and linked"],
                "expectedArtifactRefs": ["edited.txt", "restored.txt", "new.txt"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-git-restore-artifacts");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(base.join("browser-runtime")),
        };
        let create = workspace_checkpoint(
            &Map::from_iter([
                (
                    "root".to_string(),
                    Value::String(root.to_string_lossy().into_owned()),
                ),
                ("action".to_string(), Value::String("create".to_string())),
            ]),
            &ctx,
        );
        assert!(!create.error, "{}", create.text);
        let checkpoint_id = create
            .text
            .lines()
            .find_map(|line| line.strip_prefix("checkpoint_id:"))
            .map(str::trim)
            .unwrap()
            .to_string();

        fs::write(&edited, "changed\n").unwrap();
        fs::remove_file(&restored).unwrap();
        fs::write(&removed, "remove me\n").unwrap();
        let request = ToolRequest {
            id: "call-git-restore-artifacts".to_string(),
            name: "workspace_checkpoint".to_string(),
            args: Map::from_iter([
                (
                    "root".to_string(),
                    Value::String(root.to_string_lossy().into_owned()),
                ),
                ("action".to_string(), Value::String("restore".to_string())),
                ("checkpoint_id".to_string(), Value::String(checkpoint_id)),
                ("dry_run".to_string(), Value::Bool(false)),
                ("remove_new_untracked".to_string(), Value::Bool(true)),
            ]),
        };
        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&edited).unwrap(), "baseline edit\n");
        assert_eq!(
            fs::read_to_string(&restored).unwrap(),
            "baseline restored\n"
        );
        assert!(!removed.exists());
        assert_eq!(first.artifacts.len(), 3);
        let operations = first
            .artifacts
            .iter()
            .map(|artifact| artifact.operation.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert!(operations.contains(&"edit"));
        assert!(operations.contains(&"create"));
        assert!(operations.contains(&"delete"));
        for artifact in &first.artifacts {
            artifact.validate().unwrap();
        }

        fs::write(&edited, "changed after commit\n").unwrap();
        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(replay.artifacts, first.artifacts);
        assert_eq!(fs::read_to_string(&edited).unwrap(), "changed after commit\n");

        let restored_state = AgentPlanState::default();
        assert!(restored_state
            .bind_session("session-git-restore-artifacts")
            .unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains("edited.txt"));
        assert!(restored_context.contains("restored.txt"));
        assert!(restored_context.contains("new.txt"));
        assert!(restored_context.contains("call-git-restore-artifacts"));

        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn bash_uses_only_explicit_observed_output_contracts_and_replays_them() {
        let _guard = environment_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-bash-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )).join("tests");
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", root.join("state").join("nola.db"));
        std::env::set_var("NOLA_TEST_FOLDER_PERMISSIVE", "1");

        let plan_state = AgentPlanState::default();
        assert!(!plan_state.bind_session("session-bash-artifacts").unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "run-generator",
                "content": "Run the requested generator",
                "activeForm": "Running the requested generator",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["bash"],
                "successCriteria": ["The declared output is observed and linked"],
                "expectedArtifactRefs": ["result.txt"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-bash-artifacts");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-runtime")),
        };
        let target = root.join("result.txt");
        let request = ToolRequest {
            id: "call-bash-artifact".to_string(),
            name: "bash".to_string(),
            args: Map::from_iter([
                (
                    "command".to_string(),
                    Value::String("printf 'generated\\n' > result.txt".to_string()),
                ),
                (
                    "cwd".to_string(),
                    Value::String(root.to_string_lossy().into_owned()),
                ),
                (
                    "operation_kind".to_string(),
                    Value::String("mutate".to_string()),
                ),
                (
                    "execution_mode".to_string(),
                    Value::String("foreground".to_string()),
                ),
                (
                    "output_artifacts".to_string(),
                    json!([{"path":"result.txt","operation":"create"}]),
                ),
            ]),
        };

        let first = execute_tool(&request, &ctx);
        assert!(!first.error, "{}", first.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "generated\n");
        assert_eq!(first.artifacts.len(), 1);
        assert_eq!(first.artifacts[0].operation.as_deref(), Some("create"));
        first.artifacts[0].validate().unwrap();
        let artifact_uri = first.artifacts[0].uri.clone();

        fs::write(&target, "changed after commit\n").unwrap();
        let replay = execute_tool(&request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(replay.artifacts, first.artifacts);
        assert_eq!(fs::read_to_string(&target).unwrap(), "changed after commit\n");

        let undeclared = execute_tool(
            &ToolRequest {
                id: "call-bash-undeclared-output".to_string(),
                name: "bash".to_string(),
                args: Map::from_iter([
                    (
                        "command".to_string(),
                        Value::String("printf 'hidden\\n' > hidden.txt".to_string()),
                    ),
                    (
                        "cwd".to_string(),
                        Value::String(root.to_string_lossy().into_owned()),
                    ),
                    (
                        "operation_kind".to_string(),
                        Value::String("mutate".to_string()),
                    ),
                ]),
            },
            &ctx,
        );
        assert!(!undeclared.error, "{}", undeclared.text);
        assert!(root.join("hidden.txt").is_file());
        assert!(undeclared.artifacts.is_empty());

        let restored_state = AgentPlanState::default();
        assert!(restored_state.bind_session("session-bash-artifacts").unwrap());
        let restored_context = restored_state.restored_context().unwrap().unwrap();
        assert!(restored_context.contains(&artifact_uri));
        assert!(restored_context.contains("call-bash-artifact"));

        std::env::remove_var("NOLA_TEST_FOLDER_PERMISSIVE");
        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn background_bash_persists_and_settles_declared_artifacts_once_after_reconstruction() {
        let _guard = environment_lock();
        let base = std::env::temp_dir().join(format!(
            "nola-background-bash-artifacts-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("tests");
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("NOLA_DB_PATH", base.join("state").join("nola.db"));
        std::env::set_var("NOLA_TEST_FOLDER_PERMISSIVE", "1");

        let plan_state = AgentPlanState::default();
        assert!(!plan_state
            .bind_session("session-background-bash-artifacts")
            .unwrap());
        let mut plan_args = Map::new();
        plan_args.insert(
            "todos".to_string(),
            json!([{
                "id": "run-background-generator",
                "content": "Run the background generator",
                "activeForm": "Running the background generator",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "capabilityRefs": ["bash", "bash_background_wait"],
                "successCriteria": ["The background output is settled once"],
                "expectedArtifactRefs": ["background.txt"],
                "approvalStatus": "approved",
                "evidenceRefs": [],
                "verificationStatus": "pending"
            }]),
        );
        plan_state.restore_from_args(&plan_args).unwrap();
        let permission_scope = SessionPermissionScope::default();
        permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
        permission_scope.bind_owner_session("session-background-bash-artifacts");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope,
            cancellation: Default::default(),
            plan_state,
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(base.join("browser-runtime")),
        };
        let target = root.join("background.txt");
        let started = execute_tool(
            &ToolRequest {
                id: "call-background-bash-artifact".to_string(),
                name: "bash".to_string(),
                args: Map::from_iter([
                    (
                        "command".to_string(),
                        Value::String(
                            "sleep 0.2; printf 'background\\n' > background.txt".to_string(),
                        ),
                    ),
                    (
                        "cwd".to_string(),
                        Value::String(root.to_string_lossy().into_owned()),
                    ),
                    (
                        "operation_kind".to_string(),
                        Value::String("mutate".to_string()),
                    ),
                    (
                        "execution_mode".to_string(),
                        Value::String("background".to_string()),
                    ),
                    (
                        "output_artifacts".to_string(),
                        json!([{"path":"background.txt","operation":"create"}]),
                    ),
                ]),
            },
            &ctx,
        );
        assert!(!started.error, "{}", started.text);
        assert!(started.artifacts.is_empty());
        let task_id = serde_json::from_str::<Value>(&started.text).unwrap()["task_id"]
            .as_str()
            .unwrap()
            .to_string();

        BACKGROUND_BASH_TASKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .clear();
        let wait_request = ToolRequest {
            id: "wait-background-bash-artifact".to_string(),
            name: "bash_background_wait".to_string(),
            args: Map::from_iter([
                ("task_id".to_string(), Value::String(task_id.clone())),
                ("timeout_sec".to_string(), Value::Number(5.into())),
                ("poll_interval_ms".to_string(), Value::Number(100.into())),
            ]),
        };
        let settled = execute_tool(&wait_request, &ctx);
        assert!(!settled.error, "{}", settled.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "background\n");
        assert_eq!(settled.artifacts.len(), 1);
        assert_eq!(settled.artifacts[0].operation.as_deref(), Some("create"));
        settled.artifacts[0].validate().unwrap();
        let artifact_uri = settled.artifacts[0].uri.clone();

        let mut application = NolaStore::open_default().unwrap().application().unwrap();
        let plan_before: Option<ConversationPlan> = application
            .execute(ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "session-background-bash-artifacts".to_string(),
                },
            })
            .unwrap();
        let plan_before = plan_before.unwrap();
        assert!(plan_before.todos[0]
            .work_step_transaction_refs
            .iter()
            .any(|reference| reference.ends_with(":output-settlement")));
        assert!(plan_before.todos[0].artifact_refs.contains(&artifact_uri));
        let revision_before = plan_before.revision;
        drop(application);

        BACKGROUND_BASH_TASKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .clear();
        let replay = execute_tool(&wait_request, &ctx);
        assert!(!replay.error, "{}", replay.text);
        assert_eq!(replay.artifacts, settled.artifacts);
        let mut application = NolaStore::open_default().unwrap().application().unwrap();
        let plan_after: Option<ConversationPlan> = application
            .execute(ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "session-background-bash-artifacts".to_string(),
                },
            })
            .unwrap();
        let plan_after = plan_after.unwrap();
        assert_eq!(plan_after.revision, revision_before);

        std::env::remove_var("NOLA_TEST_FOLDER_PERMISSIVE");
        std::env::remove_var("NOLA_DB_PATH");
        let _ = fs::remove_dir_all(base);
    }
}

fn is_network_sensitive_tool(request: &ToolRequest) -> bool {
    matches!(
        request.name.as_str(),
        "chrome_native_open_page" | "web_fetch" | "web_search"
    )
}

fn skill_invocation_schema_from_ids(ids: impl IntoIterator<Item = String>) -> Value {
    let mut ids = ids
        .into_iter()
        .filter(|id| !id.trim().is_empty())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();

    let mut schema = crate::builtin_skills::invocation_id_schema();
    if !ids.is_empty() {
        schema
            .as_object_mut()
            .expect("built-in Skill invocation schema must be an object")
            .insert(
                "enum".to_string(),
                Value::Array(ids.into_iter().map(Value::String).collect()),
            );
    }
    schema
}
