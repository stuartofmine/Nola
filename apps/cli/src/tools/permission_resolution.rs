// SPDX-License-Identifier: Apache-2.0
//! Read-only permission facts from this Host's actual resources.
use super::*;
use nola_domain::permission_resource_plan::*;
use nola_domain::permission_scope::PermissionResource;
use sha2::{Digest, Sha256};

thread_local! {
    static BROWSER_PERMISSION_EXPECTATION: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    static DERIVED_PERMISSION_EXPECTATION: std::cell::RefCell<Option<(PermissionResolutionRequest, PermissionResolutionFacts)>> = const { std::cell::RefCell::new(None) };
    static MCP_PERMISSION_EXPECTATION: std::cell::RefCell<Option<crate::mcp_connection_authority::ConnectionResource>> = const { std::cell::RefCell::new(None) };
}

#[derive(Default)]
pub(crate) struct PermissionExecutionExpectations {
    pub(crate) browser: Option<Value>,
    derived: Option<(PermissionResolutionRequest, PermissionResolutionFacts)>,
    mcp: Option<crate::mcp_connection_authority::ConnectionResource>,
}

pub(crate) struct PermissionExecutionExpectationGuard {
    _browser: BrowserPermissionExpectationGuard,
    previous: Option<(PermissionResolutionRequest, PermissionResolutionFacts)>,
    previous_mcp: Option<crate::mcp_connection_authority::ConnectionResource>,
}

impl Drop for PermissionExecutionExpectationGuard {
    fn drop(&mut self) {
        DERIVED_PERMISSION_EXPECTATION.with(|slot| *slot.borrow_mut() = self.previous.take());
        MCP_PERMISSION_EXPECTATION.with(|slot| *slot.borrow_mut() = self.previous_mcp.take());
    }
}

pub(crate) fn enter_permission_resource_expectations(expected: PermissionExecutionExpectations) -> PermissionExecutionExpectationGuard {
    let previous = DERIVED_PERMISSION_EXPECTATION.with(|slot| slot.replace(expected.derived));
    let previous_mcp = MCP_PERMISSION_EXPECTATION.with(|slot| slot.replace(expected.mcp));
    PermissionExecutionExpectationGuard { _browser: enter_browser_permission_resource_expectation(expected.browser), previous,previous_mcp }
}

pub(crate) fn current_mcp_permission_resource_expectation() -> Option<crate::mcp_connection_authority::ConnectionResource> {
    MCP_PERMISSION_EXPECTATION.with(|slot|slot.borrow().clone())
}

pub(super) fn current_derived_permission_expectation() -> Option<(PermissionResolutionRequest, PermissionResolutionFacts)> {
    DERIVED_PERMISSION_EXPECTATION.with(|slot| slot.borrow().clone())
}

pub(crate) struct BrowserPermissionExpectationGuard {
    previous: Option<Value>,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for BrowserPermissionExpectationGuard {
    fn drop(&mut self) {
        BROWSER_PERMISSION_EXPECTATION.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

pub(crate) fn enter_browser_permission_resource_expectation(expected: Option<Value>) -> BrowserPermissionExpectationGuard {
    let previous = BROWSER_PERMISSION_EXPECTATION.with(|slot| slot.replace(expected));
    BrowserPermissionExpectationGuard {previous,_thread_bound:std::marker::PhantomData}
}

pub(crate) fn current_browser_permission_resource_expectation() -> Option<Value> {
    BROWSER_PERMISSION_EXPECTATION.with(|slot|slot.borrow().clone())
}

/// Used by every execution lane after validating the current Host schema.
/// Frozen facts are trusted only after exact declaration/input correlation and
/// fresh Host inspection; the browser then checks them again in its owner queue.
pub(crate) fn validate_permission_resource_execution(context: &ToolContext,
    contract: &PermissionOperationContract, host_id: &str, input: &Value, grant: Option<&Value>) -> Result<PermissionExecutionExpectations,String> {
    let Some(plan) = contract.resource_plan_for(input) else { return Ok(PermissionExecutionExpectations::default()); };
    let frozen = grant.and_then(|grant|grant.get("permission_resolution"))
        .ok_or("tool execution lacks its Host resource resolution")?;
    let resolution: PermissionResolutionRequest = serde_json::from_value(frozen.get("request").cloned()
        .ok_or("resource resolution request is missing")?).map_err(|error|error.to_string())?;
    let facts: PermissionResolutionFacts = serde_json::from_value(frozen.get("facts").cloned()
        .ok_or("resource resolution facts are missing")?).map_err(|error|error.to_string())?;
    if resolution.execution_target != host_id || resolution.capability != plan.capability || resolution.operation != plan.operation
        || resolution.demands != plan.extract(input).map_err(|error|error.to_string())?
        || resolution.invocation_digest != PermissionResolutionRequest::input_digest(input) {
        return Err("resource resolution differs from the current Host declaration or input".into());
    }
    facts.subject_for(&resolution).map_err(|error|error.to_string())?;
    let current = resolve_permission_resources(context,&resolution,input)?;
    if current != facts { return Err("permission resource changed after approval; resolve the resource again".into()); }
    let browser = facts.bindings.iter().filter_map(|binding|binding.browser.as_ref()).collect::<Vec<_>>();
    if browser.len() > 1 { return Err("browser execution has ambiguous owner resource facts".into()); }
    let browser = browser.first().map(|facts|serde_json::to_value(facts).map_err(|error|error.to_string())).transpose()?;
    let mcp = facts.bindings.iter().find_map(|binding| match &binding.resource {
        PermissionResource::Platform {namespace,resource_id,..} if namespace == "mcp.connection" => Some(resource_id), _ => None,
    }).map(|raw|serde_json::from_str(raw).map_err(|error|format!("invalid MCP resource identity: {error}"))).transpose()?;
    let derived = facts.bindings.iter().any(|binding| binding.derived.is_some()).then_some((resolution, facts));
    Ok(PermissionExecutionExpectations { browser, derived,mcp })
}

pub(crate) fn permission_resource_execution_failure(message: String) -> ToolOutcome {
    error_data(json!({"error":message,"code":"permission_resource_changed","tool_execution_skipped":true,
        "next_action":"Resolve the changed resource and obtain any required approval before a new attempt."}))
}

pub(super) struct LocalPermissionResources {
    contract: PermissionOperationContract,
    resolution: Option<(PermissionResolutionRequest,PermissionResolutionFacts)>,
    scope: String,
    derived_policy_resources: Vec<String>,
    pub(super) allow_conversation: bool,
}

impl LocalPermissionResources {
    pub(super) fn prepare(context: &ToolContext,spec: &ToolSpec,request: &ToolRequest) -> Result<Self,String> {
        let input = Value::Object(request.args.clone());
        nola_core::agent::host::validate_tool_input_schema(&spec.parameters,&input)?;
        let contract = builtin_permission_contract(spec.name,spec.read_only);
        let resolution = contract.resource_plan_for(&input).map(|plan| {
            let resolution = PermissionResolutionRequest {resolution_id:format!("local:{}",request.id),
                execution_target:format!("local:{}",context.permission_scope.inner.key),
                invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),
                operation:plan.operation.clone(),demands:plan.extract(&input).map_err(|error|error.to_string())?};
            let facts = resolve_permission_resources(context,&resolution,&input)?;
            Ok::<_,String>((resolution,facts))
        }).transpose()?;
        let scope = if let Some((resolution,facts)) = &resolution {
            let subject = facts.subject_for(resolution).map_err(|error|error.to_string())?;
            let identity = subject.approval_resource_identity().map_err(|error|error.to_string())?;
            let tree_evidence = facts.bindings.iter().filter(|binding|matches!(binding.demand.access.as_str(),"read_tree"|"read_instructions")
                && binding.observation.requires_confirmation()).map(|binding|binding.evidence_id.as_str()).collect::<Vec<_>>();
            json!({"resource":identity,"tree_evidence":tree_evidence,"process_risks":facts.process_approval_risks()}).to_string()
        } else {
            // Without a concrete resource projection a direct adapter cannot
            // turn one approval into authority over arbitrary later inputs.
            let declaration = nola_core::agent::permissions::permission_resource_declaration_for_builtin(&request.name);
            let resource = match declaration {
                nola_core::agent::host::HostPermissionResourceDeclaration::Constant {value} => Value::String(value),
                _ => json!(PermissionResolutionRequest::input_digest(&input)),
            };
            json!({"tool":request.name,"resource":resource}).to_string()
        };
        let derived_policy_resources = if resolution.as_ref().is_some_and(|(_,facts)| facts.bindings.iter().any(|binding| binding.derived.is_some())) {
            let plan = move_import_plan(context,&input)?;
            let mut paths = plan.updates.iter().map(|update| update.path.to_string_lossy().into_owned()).collect::<Vec<_>>();
            for key in ["source","destination"] {
                paths.push(resolved_path(input.get(key).and_then(Value::as_str).ok_or("Move resource is missing")?,context)?.to_string_lossy().into_owned());
            }
            paths
        } else { Vec::new() };
        let allow_conversation = resolution.as_ref().map(|(request,facts)|
            facts.subject_for(request).map(|subject| subject.supports_reusable_approval()))
            .transpose().map_err(|error|error.to_string())?.unwrap_or(true);
        Ok(Self {contract,resolution,scope,derived_policy_resources,allow_conversation})
    }

    pub(super) fn resource_policy(&self,context:&ToolContext,request:&ToolRequest,
        original:PermissionDecision) -> (PermissionDecision,bool) {
        let mut decision = original;
        let mut explicit_ask = false;
        for path in &self.derived_policy_resources {
            match crate::permissions::evaluate_tool_resource(context.policy.permission_rules(),&request.name,path) {
                Some(PermissionDecision::Deny) => return (PermissionDecision::Deny,false),
                Some(PermissionDecision::Allow) => {},
                value => { if decision != PermissionDecision::Deny { decision = PermissionDecision::Ask; }
                    explicit_ask |= value == Some(PermissionDecision::Ask); },
            }
        }
        (decision,explicit_ask)
    }

    pub(super) fn requires_confirmation(&self,input: &Value) -> bool {
        nola_core::agent::permissions::permission_contract_requires_confirmation(&self.contract.for_input(input),self.resolution.as_ref())
    }

    pub(super) fn approved_in_scope(&self,context: &ToolContext) -> bool {
        self.allow_conversation && context.permission_scope.inner.resolved_resource_grants.lock().ok().is_some_and(|grants|grants.contains(&self.scope))
    }

    pub(super) fn remember_approval(&self,context: &ToolContext) -> Result<(), String> {
        if !self.allow_conversation { return Err("this invocation has no valid reusable resource scope; choose allow once".into()); }
        if let Ok(mut grants) = context.permission_scope.inner.resolved_resource_grants.lock() { grants.insert(self.scope.clone()); }
        Ok(())
    }

    pub(super) fn revalidate(&self,context: &ToolContext,input: &Value) -> Result<PermissionExecutionExpectations,String> {
        let grant = self.resolution.as_ref().map(|(request,facts)|json!({"permission_resolution":{"request":request,"facts":facts}}));
        let host = self.resolution.as_ref().map(|(request,_)|request.execution_target.as_str()).unwrap_or("local:no-resource-plan");
        validate_permission_resource_execution(context,&self.contract,host,input,grant.as_ref())
    }
}

/// Preserve the user's once/conversation choice without writing a tool-wide
/// wildcard. Direct execution grants are tied to this exact resolved scope.
pub(super) fn confirm_local_permission(request: &ToolRequest, allow_conversation: bool) -> ToolPermissionDecision {
    if let Some(resolver) = current_host_permission_resolver() {
        return match resolver(request,&[],allow_conversation) {
            HostPermissionResolution::Allow => ToolPermissionDecision::Allow,
            HostPermissionResolution::AllowConversation => ToolPermissionDecision::AllowConversation,
            HostPermissionResolution::Custom(note) => ToolPermissionDecision::Custom(note),
            HostPermissionResolution::ReturnToComposer => ToolPermissionDecision::ReturnToComposer,
            HostPermissionResolution::StopRequested => ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected),
            HostPermissionResolution::TransportInterrupted(reason) => ToolPermissionDecision::TransportInterrupted(reason),
            HostPermissionResolution::Deny => ToolPermissionDecision::Deny,
        };
    }
    if native_permission_mode_enabled() { return confirm_native_tool_with_instructions(request,&[],allow_conversation); }
    if !permission_interaction_available() {
        return ToolPermissionDecision::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
    }
    let mut prompt = tool_permission_prompt(request);
    prompt.allow_conversation = allow_conversation;
    match prompt_permission(&prompt) {
        PermissionChoice::Once => ToolPermissionDecision::Allow,
        PermissionChoice::Session(_) => ToolPermissionDecision::AllowConversation,
        PermissionChoice::Custom(note) => ToolPermissionDecision::Custom(note),
        PermissionChoice::ReturnToComposer => ToolPermissionDecision::ReturnToComposer,
        PermissionChoice::TransportInterrupted(reason) => ToolPermissionDecision::TransportInterrupted(reason),
        PermissionChoice::Deny => ToolPermissionDecision::Deny,
    }
}

pub(crate) fn builtin_permission_contract(name: &str, read_only: bool) -> PermissionOperationContract {
    let requirement = |id: &str, pointer: &str, access: &str| ResourceRequirement {
        requirement_id: id.into(), locator_kind: ResourceLocatorKind::FilePath, access: access.into(),
        selector: ResourceSelector::One { pointer: pointer.into() },
    };
    let file_plan = |requirements| PermissionOperationContract { risk: match name {
        "delete_file" | "move_file" => PermissionOperationRisk::Delete,
        "edit_file" | "edit_file_lines" | "notebook_edit" => PermissionOperationRisk::Overwrite,
        _ => PermissionOperationRisk::Unknown,
    },
        preview_when_true: matches!(name,"write_file"|"move_file"|"delete_file"|"edit_file"|"edit_file_lines"|"notebook_edit")
            .then(||"dry_run".into()),
        risk_when_nonempty: Default::default(),
        resource_plan: Some(PermissionResourcePlan { capability: name.into(), operation: "filesystem".into(), requirements }) };
    if let Some((access, attached)) = browser_permission_access(name) {
        return PermissionOperationContract {resource_plan:Some(PermissionResourcePlan {
            capability:browser_permission_scope(access).into(),operation:browser_permission_scope(access).into(),
            requirements:vec![ResourceRequirement {requirement_id:access.into(),locator_kind:ResourceLocatorKind::PlatformResource,
                access:browser_permission_scope(access).into(),selector:ResourceSelector::FirstAvailable {
                    pointers:vec!["/target".into()],default_locator:Some(if attached {"attached"} else {"managed"}.into()),
                }}],
        }),..Default::default()};
    }
    match name {
        "mcp_connect" => PermissionOperationContract {resource_plan:Some(PermissionResourcePlan {
            capability:"mcp.connection".into(),operation:"connect".into(),requirements:vec![ResourceRequirement {
                requirement_id:"mcp-connection".into(),locator_kind:ResourceLocatorKind::PlatformResource,
                access:"connect".into(),selector:ResourceSelector::One {pointer:"/connection_ref".into()},
            }],
        }),..Default::default()},
        "conversation_recall" => PermissionOperationContract {resource_plan:Some(PermissionResourcePlan {
            capability:name.into(),operation:"canonical_history.read".into(),requirements:vec![ResourceRequirement {
                requirement_id:"canonical-history-scope".into(),locator_kind:ResourceLocatorKind::PlatformResource,
                access:"read".into(),selector:ResourceSelector::FirstAvailable {pointers:vec!["/scope".into()],
                    default_locator:Some("current_conversation".into())},
            }],
        }),..Default::default()},
        "read_file" | "notebook_read" | "file_outline" => file_plan(vec![requirement("file", "/path", "read")]),
        "codebase_index" => file_plan(vec![requirement("index-scope", "/repo_path", "read_tree")]),
        "read_many_files" => file_plan(vec![ResourceRequirement { requirement_id:"files".into(),
            locator_kind:ResourceLocatorKind::FilePath, access:"read".into(),
            selector:ResourceSelector::Each { array_pointer:"/files".into(), item_pointer:"/path".into() } }]),
        "grep" | "glob" | "read_symbol" | "code_search" | "repo_map" | "file_tree" | "list_dir" | "find_files"
        | "symbol_search" | "implementation_search" | "workspace_symbols" | "code_navigation" | "call_graph"
        | "file_dependencies" | "project_overview" | "context_pack" | "focused_context" =>
            file_plan(vec![ResourceRequirement {requirement_id:"search-scope".into(),locator_kind:ResourceLocatorKind::FilePath,
                access:"read_tree".into(),selector:ResourceSelector::FirstAvailable {
                    pointers:vec!["/path".into(),"/root".into()],default_locator:Some(".".into())}}]),
        "instruction_context" => file_plan(vec![requirement("instruction-scope","/root","read_instructions")]),
        "git_worktree_list" => file_plan(vec![requirement("repository", "/root", "read_tree")]),
        "git_worktree_create" => file_plan(vec![
            requirement("repository", "/root", "read_tree"),
            requirement("shared-git-metadata", "/expected_common_directory", "git_worktree_create"),
            requirement("worktree-destination", "/destination", "create_worktree"),
        ]),
        "git_worktree_integrate" => PermissionOperationContract {risk:PermissionOperationRisk::Overwrite, ..file_plan(vec![
            requirement("source-checkout", "/root", "overwrite"),
            requirement("reviewed-child", "/worktree/root", "read_tree"),
            requirement("shared-git-metadata", "/worktree/expected_common_directory", "read"),
        ])},
        "write_file" | "append_file" => file_plan(vec![requirement("file", "/path", "write")]),
        "edit_file" | "edit_file_lines" | "notebook_edit" => file_plan(vec![requirement("file", "/path", "overwrite")]),
        "delete_file" => file_plan(vec![requirement("file", "/path", "remove")]),
        "move_file" => file_plan(vec![requirement("source", "/source", "remove"), requirement("destination", "/destination", "write"),
            ResourceRequirement { requirement_id:"move-import-updates".into(), locator_kind:ResourceLocatorKind::DerivedFileCollection,
                access:"derive_move_imports".into(), selector:ResourceSelector::FirstAvailable {
                    pointers:vec!["/root".into()], default_locator:Some(".".into()) } }]),
        "bash" | "bash_background_start" | "bash_command_review" => PermissionOperationContract {
            risk: PermissionOperationRisk::Unknown,
            preview_when_true: None,
            risk_when_nonempty: Default::default(),
            resource_plan: Some(PermissionResourcePlan { capability:name.into(), operation:"process".into(),
                requirements:vec![ResourceRequirement { requirement_id:"command".into(), locator_kind:ResourceLocatorKind::ProcessCommand,
                    access:"execute".into(), selector:ResourceSelector::One { pointer:"/command".into() } }] }),
        },
        "language_service_query" => PermissionOperationContract {
            risk: PermissionOperationRisk::Unknown,
            resource_plan: Some(PermissionResourcePlan {
                capability: name.into(), operation: "language_service.query".into(),
                requirements: vec![
                    requirement("document", "/path", "read"),
                    ResourceRequirement { requirement_id:"supporting-documents".into(),
                        locator_kind:ResourceLocatorKind::FilePath, access:"read".into(),
                        selector:ResourceSelector::OptionalEach { array_pointer:"/supporting_documents".into(), item_pointer:"/path".into() } },
                    ResourceRequirement {
                        requirement_id: "language-service".into(),
                        locator_kind: ResourceLocatorKind::ProcessHandle,
                        access: "execute".into(),
                        selector: ResourceSelector::One { pointer: "/service_id".into() },
                    },
                ],
            }),
            ..Default::default()
        },
        "terminal_input" => PermissionOperationContract {
            risk: PermissionOperationRisk::Unknown,
            resource_plan: Some(PermissionResourcePlan {
                capability: name.into(), operation: "terminal.input".into(),
                requirements: vec![ResourceRequirement {
                    requirement_id: "terminal-task".into(),
                    locator_kind: ResourceLocatorKind::ProcessHandle,
                    access: "input".into(),
                    selector: ResourceSelector::One { pointer: "/task_id".into() },
                }],
            }),
            ..Default::default()
        },
        "memory_delete" => PermissionOperationContract {risk:PermissionOperationRisk::Delete, ..Default::default()},
        "memory_search" | "memory_update" => PermissionOperationContract {risk:PermissionOperationRisk::SensitiveRead, ..Default::default()},
        "memory_save" => PermissionOperationContract {risk:PermissionOperationRisk::NewFile,
            risk_when_nonempty: [("/supersedes".into(),PermissionOperationRisk::Overwrite)].into(), ..Default::default()},
        "create_reminder" | "create_calendar_event" => PermissionOperationContract {risk:PermissionOperationRisk::NewFile, ..Default::default()},
        // This only creates a child Task owned by the current conversation.
        // Its model profile, owner binding and every child tool are admitted
        // independently; this is not authority to execute the child's work.
        "delegate_expert" | "expert_exchange" => PermissionOperationContract {risk:PermissionOperationRisk::OrdinaryRead, ..Default::default()},
        "spawn_agent" => PermissionOperationContract {risk:PermissionOperationRisk::NewFile, ..Default::default()},
        // These fixed-schema retrieval adapters do not accept an arbitrary
        // HTTP method, request body, upload or external-message operation.
        // A provider's search POST is still retrieval, not a sending grant.
        "web_search" | "web_fetch" => PermissionOperationContract {risk:PermissionOperationRisk::OrdinaryRead, ..Default::default()},
        // Controls declare state or ask the owner; neither grants authority to
        // execute a subsequent external operation.
        "request_user_input" | "goal_complete" | "skill_update" | "skill_skip" | "sop_run_update" => PermissionOperationContract {risk:PermissionOperationRisk::OrdinaryRead, ..Default::default()},
        _ => {
            let domain = nola_core::agent::tool_permission_domain(name);
            let sensitive = domain.as_deref().is_some_and(|domain| matches!(domain,
                "camera"|"microphone"|"photos"|"location"|"calendar"|"reminders"|"contacts"|"health"|"clipboard"|"notifications"|"computer"|"browser"));
            PermissionOperationContract { risk: if sensitive && read_only { PermissionOperationRisk::SensitiveRead }
                else if sensitive { PermissionOperationRisk::Unknown }
                else if read_only { PermissionOperationRisk::OrdinaryRead } else { PermissionOperationRisk::Unknown }, ..Default::default() }
        }
    }
}

fn browser_permission_access(name: &str) -> Option<(&'static str,bool)> {
    let access = match name {
        "browser_start" => ("browser_connect",false),
        "browser_status" => ("browser_status",false),
        "browser_snapshot" | "browser_screenshot" => ("browser_observe",false),
        "browser_tabs" => ("browser_list",false),
        "browser_switch_page" => ("browser_select_zero",false),
        "browser_navigate" => ("browser_navigate",false),
        "browser_click" | "browser_type_text" => ("browser_interact",false),
        "browser_stop" => ("browser_close",false),
        "chrome_native_status" => ("browser_status",true),
        "chrome_native_inspect" | "chrome_native_screenshot" | "chrome_native_dom_snapshot"
        | "chrome_native_accessibility_snapshot" | "chrome_native_assert" | "chrome_native_wait_for"
        | "chrome_native_console_logs" | "chrome_native_network_events" => ("browser_observe",true),
        "chrome_native_storage_state" => ("browser_storage",true),
        "chrome_native_list_pages" => ("browser_list",true),
        "chrome_native_switch_page" => ("browser_select",true),
        "chrome_native_open_page" => ("browser_navigate",true),
        "chrome_native_click" | "chrome_native_type_text" | "chrome_native_hover" | "chrome_native_drag"
        | "chrome_native_press_key" | "chrome_native_scroll" | "chrome_native_select_option"
        | "chrome_native_set_checked" => ("browser_interact",true),
        _ => return None,
    };
    Some(access)
}

fn browser_permission_scope(access: &str) -> &str {
    match access {
        "browser_observe" | "browser_select" | "browser_select_zero" => "browser.observe",
        "browser_status" => "browser.status",
        "browser_list" => "browser.list_pages",
        "browser_connect" => "browser.connection",
        "browser_navigate" => "browser.navigation",
        "browser_interact" => "browser.interaction",
        "browser_storage" => "browser.storage",
        "browser_close" => "browser.close",
        _ => "browser.unknown",
    }
}

fn resolve_browser_resource(context: &ToolContext, access: &str, locator: &str,
    input: &Value) -> Result<(PermissionResource,PermissionOperationRisk,Value),String> {
    let owner = context.permission_scope.owner_session_id().filter(|owner|!owner.trim().is_empty())
        .ok_or("browser permission resolution requires its bound Host owner")?;
    let owner_id = canonical_managed_browser_session_id(&owner);
    if !matches!(locator,"managed"|"attached") { return Err("unsupported browser permission target".into()); }
    let mut args = Map::new();
    args.insert(if locator == "managed" {"session_id"} else {"nola_owner_id"}.into(),Value::String(owner_id.clone()));
    for key in ["url","index"] {
        if let Some(value) = input.get(key) { args.insert(key.into(),value.clone()); }
    }
    // The unified Host contract uses zero-based indices; the native bridge is
    // one-based. Raw native tools already use the bridge's native index.
    if locator == "attached" && access == "browser_select_zero" {
        if let Some(index) = input.get("index").and_then(Value::as_u64) {
            args.insert("index".into(),json!(index.checked_add(1).ok_or("browser page index overflow")?));
        }
    }
    let result = if locator == "managed" {
        run_managed_browser_action("permission_resource",&args,&context.browser.state_dir)
    } else {run_chrome_native_action("permission_resource",&args)}?;
    browser_resource_from_facts(locator,&owner_id,access,input,
        serde_json::from_value(Value::Object(result)).map_err(|error|format!("invalid browser permission facts: {error}"))?)
}

fn browser_resource_from_facts(target: &str, owner_id: &str, access: &str, input: &Value,
    facts: BrowserPermissionResourceFacts) -> Result<(PermissionResource,PermissionOperationRisk,Value),String> {
    if !facts.ok || facts.target != target || facts.owner_id != owner_id || facts.authentication_state != "unknown"
        || !matches!(facts.connection_state.as_str(),"existing"|"absent") {
        return Err("browser permission facts do not match the Host-owned target".into());
    }
    let requested_origin = input.get("url").and_then(Value::as_str).map(|value| {
        let url = url::Url::parse(value).map_err(|_|"invalid browser destination URL")?;
        if !matches!(url.scheme(),"http"|"https") || !url.username().is_empty() || url.password().is_some() {
            return Err("browser destination requires an HTTP(S) URL without credentials");
        }
        Ok(url.origin().ascii_serialization())
    }).transpose().map_err(str::to_owned)?;
    if facts.requested_origin != requested_origin { return Err("browser permission destination differs from the requested origin".into()); }
    let existing = facts.connection_state == "existing";
    let metadata_only = access == "browser_status" || (target == "attached" && access == "browser_connect");
    if existing && !metadata_only && facts.connection_generation.as_ref().is_none_or(|value|value.trim().is_empty()) {
        return Err("existing browser connection lacks its generation".into());
    }
    if !existing && (facts.connection_generation.is_some() || facts.page_id.is_some() || facts.page_origin.is_some()) {
        return Err("absent browser connection returned a live resource identity".into());
    }
    let session_only = matches!(access,"browser_connect"|"browser_list"|"browser_status"|"browser_close");
    if existing && !session_only && facts.page_id.as_ref().is_none_or(|value|value.trim().is_empty()) {
        return Err("owned browser page is stale; select an explicit current page".into());
    }
    if target == "attached" && access == "browser_close" { return Err("attached browser cannot be closed by Nola".into()); }
    if !existing && matches!(access,"browser_observe"|"browser_interact"|"browser_storage") {
        return Err("browser observation requires an existing owned page; explicitly connect or select a page first".into());
    }
    let risk = if metadata_only {PermissionOperationRisk::OrdinaryRead}
        else if !existing {PermissionOperationRisk::NewConnection}
        else {match access {
            "browser_connect" => PermissionOperationRisk::OrdinaryRead,
            "browser_observe" | "browser_select" | "browser_select_zero" | "browser_list" | "browser_status" | "browser_storage" => PermissionOperationRisk::SensitiveRead,
            "browser_navigate" if facts.page_origin != facts.requested_origin => PermissionOperationRisk::NewConnection,
            "browser_navigate" | "browser_interact" | "browser_close" => PermissionOperationRisk::Unknown,
            _ => return Err("unknown browser permission operation".into()),
        }};
    let scope = browser_permission_scope(access);
    let resource_id = json!({"owner_id":owner_id,"connection_generation":facts.connection_generation,
        "page_id":if session_only {None} else {facts.page_id.as_ref()},
        "origin":if access == "browser_navigate" {facts.requested_origin.as_ref()} else if session_only {None} else {facts.page_origin.as_ref()}}).to_string();
    let resource = PermissionResource::Platform {namespace:format!("browser.{target}"),resource_id,access:scope.into()};
    Ok((resource,risk,serde_json::to_value(facts).expect("browser permission facts are serializable")))
}

fn resolved_path(raw: &str, context: &ToolContext) -> Result<PathBuf, String> {
    let input = expand_home(Path::new(raw));
    let absolute = if input.is_absolute() { input } else {
        context.allowed_dirs.first().ok_or("no resource resolution base directory")?.join(input)
    };
    resolve_missing_path(&absolute).ok_or_else(|| format!("cannot resolve resource: {raw}"))
}

fn resolve_canonical_history_resource(context: &ToolContext,input: &Value,scope: &str)
    -> Result<(PermissionResource,PermissionOperationRisk,Value),String> {
    let owner = context.permission_scope.owner_session_id().filter(|owner|!owner.trim().is_empty())
        .ok_or("canonical history requires its bound Host owner")?;
    let nonempty = |key|input.get(key).and_then(Value::as_str).filter(|value|!value.trim().is_empty());
    let direct_part = nonempty("message_id").is_some() || nonempty("part_id").is_some();
    if direct_part && (nonempty("message_id").is_none() || nonempty("part_id").is_none()) {
        return Err("canonical original part requires both message_id and part_id".into());
    }
    let (resolved_scope,conversations) = if direct_part {
        let target = nonempty("conversation_id").unwrap_or(&owner).to_string();
        ("conversations",vec![target])
    } else {match scope {
        "current_conversation" => ("conversations",vec![owner.clone()]),
        "local_user_history" => ("local_user_history",Vec::new()),
        "conversations" => {
            let ids:Vec<String> = serde_json::from_value(input.get("conversation_ids").cloned()
                .ok_or("conversation_ids is required for selected history scope")?).map_err(|error|error.to_string())?;
            if ids.is_empty() || ids.iter().any(|id|id.trim().is_empty()) { return Err("history scope requires nonempty conversation identities".into()); }
            ("conversations",ids)
        },
        _ => return Err("unsupported canonical history scope".into()),
    }};
    let conversations = conversations.into_iter().collect::<std::collections::BTreeSet<_>>();
    let ordinary = resolved_scope == "conversations" && conversations.len() == 1 && conversations.contains(&owner);
    let identity = json!({"requester_conversation_id":owner,"scope":resolved_scope,"conversation_ids":conversations});
    Ok((PermissionResource::Platform {namespace:"nola.canonical_history".into(),resource_id:identity.to_string(),access:"read".into()},
        if ordinary {PermissionOperationRisk::OrdinaryRead} else {PermissionOperationRisk::SensitiveRead},identity))
}

fn sensitive_path(path: &Path, _context: &ToolContext) -> bool {
    super::bash_token_is_sensitive_path(&path.to_string_lossy())
}

fn path_identity(path: &Path) -> Result<Value, String> {
    match fs::metadata(path) {
        Ok(metadata) => {
            #[cfg(unix)]
            let platform = { use std::os::unix::fs::MetadataExt;
                json!({"device":metadata.dev(),"inode":metadata.ino(),"ctime":metadata.ctime(),"ctime_nsec":metadata.ctime_nsec()}) };
            #[cfg(not(unix))]
            let platform = Value::Null;
            Ok(json!({"path":path,"exists":true,"size":metadata.len(),"modified":metadata.modified().ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|value|value.as_nanos().to_string()),"platform":platform}))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(json!({"path":path,"exists":false})),
        Err(error) => Err(format!("cannot inspect permission resource: {error}")),
    }
}

fn evidence_id(value: &Value) -> String { format!("permission-resource:{:x}", Sha256::digest(value.to_string().as_bytes())) }

// A read scope is the same directory after ordinary entries or Host journals
// change. Bind its filesystem identity, then separately rescan the sensitive
// resources below it; a replaced directory still cannot reuse this evidence.
fn read_scope_identity(path: &Path) -> Result<Value, String> {
    let mut identity = path_identity(path)?;
    if path.is_dir() && identity.pointer("/platform/device").is_some()
        && identity.pointer("/platform/inode").is_some() {
        let object = identity.as_object_mut().expect("path identity is an object");
        object.remove("size");
        object.remove("modified");
        object.insert("directory".into(), Value::Bool(true));
        if let Some(platform) = object.get_mut("platform").and_then(Value::as_object_mut) {
            platform.remove("ctime");
            platform.remove("ctime_nsec");
        }
    }
    Ok(identity)
}

fn inspect_read_tree(path: &Path, context: &ToolContext, capability: &str) -> Result<(PermissionOperationRisk,Value),String> {
    if sensitive_path(path,context) { return Ok((PermissionOperationRisk::SensitiveRead,path_identity(path)?)); }
    if !path.is_dir() { return Ok((PermissionOperationRisk::OrdinaryRead,path_identity(path)?)); }
    if capability == "codebase_index" {
        let paths = nola_code_memory::NativeCodeMemory::index_file_paths(
            path, &|| context.cancellation.is_requested(),
        )?;
        let mut sensitive = Vec::new();
        for candidate in paths {
            let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
            if !canonical.starts_with(path) {
                return Err("code index candidate escaped its approved root".into());
            }
            if sensitive_path(&canonical, context) {
                sensitive.push(path_identity(&canonical)?);
            }
        }
        sensitive.sort_by_key(Value::to_string);
        return Ok((
            if sensitive.is_empty() { PermissionOperationRisk::OrdinaryRead }
            else { PermissionOperationRisk::SensitiveRead },
            json!({"scope": read_scope_identity(path)?, "sensitive_resources": sensitive}),
        ));
    }
    let mut stack = vec![path.to_path_buf()];
    let mut visited = std::collections::BTreeSet::new();
    let mut sensitive = Vec::new();
    let source_only = matches!(capability,"code_search"|"repo_map"|"symbol_search"|"implementation_search"|"workspace_symbols"
        |"code_navigation"|"call_graph"|"file_dependencies"|"read_symbol");
    let mut count = 0usize;
    while let Some(directory) = stack.pop() {
        let directory = directory.canonicalize().map_err(|error|error.to_string())?;
        if !visited.insert(directory.clone()) { continue; }
        for entry in fs::read_dir(directory).map_err(|error|error.to_string())? {
            count += 1;
            if count > 10_000 { return Ok((PermissionOperationRisk::Unknown,json!({"scope":path,"inspection":"bounded"}))); }
            let entry = entry.map_err(|error|error.to_string())?;
            let entry_path = entry.path();
            let canonical = entry_path.canonicalize().map_err(|error|error.to_string())?;
            if sensitive_path(&canonical,context) {
                let selected_file = !source_only || is_text_source(&entry_path);
                if canonical.is_dir() || selected_file { sensitive.push(path_identity(&canonical)?); }
                continue;
            }
            if canonical.is_dir() && !should_skip_dir(&entry.file_name().to_string_lossy()) { stack.push(canonical); }
        }
    }
    sensitive.sort_by_key(Value::to_string);
    Ok((if sensitive.is_empty() {PermissionOperationRisk::OrdinaryRead} else {PermissionOperationRisk::SensitiveRead},
        json!({"scope":read_scope_identity(path)?,"sensitive_resources":sensitive})))
}

fn inspect_instruction_resources(root: &Path,context: &ToolContext,input: &Value) -> Result<(PermissionOperationRisk,Value),String> {
    let names = input.get("names").and_then(Value::as_array).filter(|values|!values.is_empty())
        .map(|values|values.iter().map(|value|value.as_str().map(str::to_owned).ok_or("instruction name must be a string"))
            .collect::<Result<Vec<_>,_>>()).transpose()?.unwrap_or_else(||INSTRUCTION_FILE_NAMES.iter().map(|name|(*name).into()).collect());
    if names.iter().any(|name|name.trim().is_empty()||name.contains(['/', '\\'])) { return Err("instruction names must be plain filenames".into()); }
    let targets = input.get("paths").and_then(Value::as_array).filter(|values|!values.is_empty())
        .map(|values|values.iter().map(|value| {
            let raw = value.as_str().ok_or("instruction path must be a string")?;
            let candidate = if Path::new(raw).is_absolute() {PathBuf::from(raw)} else {root.join(raw)};
            let target = resolved_path(&candidate.to_string_lossy(),context)?;
            if !target.starts_with(root) { return Err("instruction target is outside its root".to_string()); }
            Ok(target)
        }).collect::<Result<Vec<_>,String>>()).transpose()?.unwrap_or_else(||vec![root.to_path_buf()]);
    let mut candidates = std::collections::BTreeSet::new();
    for target in targets {
        for directory in instruction_search_dirs(root,&target) {
            for name in &names { candidates.insert(directory.join(name.trim())); }
            for relative in INSTRUCTION_REL_PATHS { candidates.insert(directory.join(relative)); }
            let mut stack = INSTRUCTION_RULE_DIRS.iter().map(|relative|directory.join(relative)).collect::<Vec<_>>();
            let mut visited = std::collections::BTreeSet::new();
            while let Some(path) = stack.pop() {
                if !path.exists() { continue; }
                let canonical = path.canonicalize().map_err(|error|error.to_string())?;
                if !visited.insert(canonical.clone()) { continue; }
                if visited.len() + candidates.len() > 10_000 {
                    return Ok((PermissionOperationRisk::Unknown,json!({"root":root,"inspection":"bounded"})));
                }
                if canonical.is_dir() {
                    // Do not traverse a sensitive symlink subtree before approval.
                    if sensitive_path(&canonical,context) { candidates.insert(canonical); continue; }
                    for entry in fs::read_dir(&canonical).map_err(|error|error.to_string())? { stack.push(entry.map_err(|error|error.to_string())?.path()); }
                } else if canonical.extension().and_then(|value|value.to_str()).is_some_and(|extension|
                    extension.eq_ignore_ascii_case("md")||extension.eq_ignore_ascii_case("mdc")) { candidates.insert(canonical); }
            }
        }
    }
    let mut sensitive = sensitive_path(root,context);
    let mut identities = Vec::new();
    for candidate in candidates {
        let path = resolved_path(&candidate.to_string_lossy(),context)?;
        sensitive |= sensitive_path(&path,context);
        identities.push(path_identity(&path)?);
    }
    Ok((if sensitive {PermissionOperationRisk::SensitiveRead} else {PermissionOperationRisk::OrdinaryRead},
        json!({"root":read_scope_identity(root)?,"instruction_candidates":identities})))
}

pub(crate) fn resolve_permission_resources(context: &ToolContext, request: &PermissionResolutionRequest,
    input: &Value) -> Result<PermissionResolutionFacts, String> {
    if context.policy.bound_file_workspace().is_some() {
        let args=input.as_object().ok_or("File tool input must be an object")?.clone();
        validate_bound_file_workspace(&ToolRequest {id:request.resolution_id.clone(),name:request.capability.clone(),args},context)?;
    }
    if request.invocation_digest != PermissionResolutionRequest::input_digest(input) {
        return Err("permission resolution input identity mismatch".into());
    }
    let mut bindings = Vec::new();
    for demand in &request.demands {
        let mut browser = None;
        let mut derived = None;
        let (resource, observation, evidence) = match demand.locator_kind {
            ResourceLocatorKind::FilePath => {
                let path = resolved_path(&demand.locator, context)?;
                let identity = path_identity(&path)?;
                let exists = identity["exists"] == true;
                let tree = match demand.access.as_str() {
                    "read_tree" => Some(inspect_read_tree(&path,context,&request.capability)?),
                    "read_instructions" => Some(inspect_instruction_resources(&path,context,input)?),
                    _ => None,
                };
                let observation = match demand.access.as_str() {
                    "read" if sensitive_path(&path, context) => PermissionOperationRisk::SensitiveRead,
                    "read" => PermissionOperationRisk::OrdinaryRead,
                    "read_tree" | "read_instructions" => tree.as_ref().map(|(risk,_)|*risk).unwrap_or(PermissionOperationRisk::Unknown),
                    "write" if !exists && !sensitive_path(&path, context) => PermissionOperationRisk::NewFile,
                    "write" | "overwrite" => PermissionOperationRisk::Overwrite,
                    "remove" => PermissionOperationRisk::Delete,
                    _ => PermissionOperationRisk::Unknown,
                };
                let components = path.components().filter_map(|part| match part {
                    std::path::Component::Normal(part) => part.to_str().map(str::to_owned), _ => None,
                }).collect::<Vec<_>>();
                let filesystem_id = identity.pointer("/platform/device").map(Value::to_string).unwrap_or_else(|| "host-filesystem".into());
                let evidence = tree.map(|(_,evidence)|evidence).unwrap_or(identity);
                (PermissionResource::File {filesystem_id, components, access:demand.access.clone()}, observation, evidence)
            }
            ResourceLocatorKind::DerivedFileCollection => {
                if request.capability != "move_file" || demand.requirement_id != "move-import-updates"
                    || !matches!(demand.access.as_str(), "derive_move_imports" | "read") {
                    return Err("Host does not implement this declared resource collection".into());
                }
                let plan = move_import_plan(context, input)?;
                let root_identity = path_identity(&plan.root)?;
                let filesystem_id = root_identity.pointer("/platform/device").map(Value::to_string).unwrap_or_else(|| "host-filesystem".into());
                let components = file_resource_components(&plan.root)?;
                let preview = demand.access == "read";
                let members = move_import_resource_members(context, &plan, preview)?;
                let collection = HostDerivedResourceCollection { collection_id:demand.requirement_id.clone(), discovery_complete:true,
                    plan_digest:HostDerivedResourceCollection::digest(request,demand,&members), members };
                let evidence = json!({"root":root_identity,"collection":collection});
                derived = Some(collection);
                (PermissionResource::File { filesystem_id,components,access:demand.access.clone() },
                    PermissionOperationRisk::OrdinaryRead,evidence)
            }
            ResourceLocatorKind::ProcessCommand => {
                let (observation, evidence) = inspect_read_only_shell(context, input)?;
                let cwd = process_working_directory(context,input)?;
                (PermissionResource::ProcessInvocation {invocation_digest:request.invocation_digest,
                    command:demand.locator.clone(),cwd:cwd.to_string_lossy().into_owned()}, observation, evidence)
            }
            ResourceLocatorKind::ProcessHandle if request.capability == "language_service_query"
                && request.operation == "language_service.query" && demand.requirement_id == "language-service"
                && demand.access == "execute" => {
                let plan = super::language_service_tools::query_plan(context, input)?;
                if plan.service.id != demand.locator { return Err("language service resource identity mismatch".into()); }
                let evidence = json!({"service_id":plan.service.id,"configuration_fingerprint":plan.service.fingerprint()?,
                    "executable":path_identity(&plan.executable)?,"project_root":path_identity(&plan.root)?,
                    "document":path_identity(&plan.file)?,"language_id":plan.language,
                    "supporting_documents":plan.supporting_documents.iter().map(|(path, language)| {
                        Ok(json!({"document":path_identity(path)?,"language_id":language}))
                    }).collect::<Result<Vec<Value>,String>>()?});
                (PermissionResource::ProcessInvocation { invocation_digest:request.invocation_digest,
                    command:demand.locator.clone(),cwd:plan.root.to_string_lossy().into_owned() },
                    PermissionOperationRisk::Unknown,evidence)
            }
            ResourceLocatorKind::ProcessHandle if request.capability == "terminal_input"
                && request.operation == "terminal.input" && demand.requirement_id == "terminal-task"
                && demand.access == "input" => {
                let task = crate::runtime_task_port::get(&demand.locator)?
                    .ok_or("Unknown terminal task")?;
                if task.kind != "shell_process" || task.owner_session_id.is_none()
                    || task.owner_session_id != context.permission_scope.owner_session_id() {
                    return Err("Terminal task is not owned by this conversation".into());
                }
                let metadata: Value = serde_json::from_str(&task.metadata_json).map_err(|err|err.to_string())?;
                let terminal: nola_domain::terminal_input::TerminalSessionConfiguration =
                    serde_json::from_value(metadata.get("terminal").cloned().unwrap_or(Value::Null))
                        .map_err(|err|format!("Task has no valid terminal: {err}"))?;
                terminal.validate()?;
                if terminal.host_id != request.execution_target {
                    return Err("Terminal belongs to another execution Host".into());
                }
                // Terminal text can execute arbitrary side effects. Bind the exact
                // invocation, never create a reusable task-wide input grant.
                (PermissionResource::ProcessInvocation {
                    invocation_digest: request.invocation_digest,
                    command: demand.locator.clone(),
                    cwd: metadata.get("cwd").and_then(Value::as_str).unwrap_or("").into(),
                }, PermissionOperationRisk::Unknown,
                    json!({"task_id":task.id,"host_id":terminal.host_id,"operation":"terminal.input"}))
            }
            ResourceLocatorKind::PlatformResource if request.capability == "mcp.connection"
                && request.operation == "connect" && demand.requirement_id == "mcp-connection" && demand.access == "connect" => {
                let facts = crate::mcp_connection_authority::permission_resource(&demand.locator)?;
                if facts.host_id != request.execution_target || context.permission_scope.owner_session_id().as_deref() != Some(&facts.session_id) {
                    return Err("MCP permission target is not owned by the current Host/session".into());
                }
                let observation = if facts.generation.is_some() {PermissionOperationRisk::OrdinaryRead} else {PermissionOperationRisk::NewConnection};
                let evidence = serde_json::to_value(facts).map_err(|error|error.to_string())?;
                (PermissionResource::Platform {namespace:"mcp.connection".into(),resource_id:evidence.to_string(),access:"connect".into()},observation,evidence)
            }
            ResourceLocatorKind::PlatformResource if demand.requirement_id == "canonical-history-scope" && demand.access == "read" => {
                resolve_canonical_history_resource(context,input,&demand.locator)?
            }
            ResourceLocatorKind::PlatformResource if demand.access == browser_permission_scope(&demand.requirement_id)
                && demand.access != "browser.unknown" => {
                let resolved = resolve_browser_resource(context,&demand.requirement_id,&demand.locator,input)?;
                browser = Some(serde_json::from_value(resolved.2.clone()).map_err(|error|error.to_string())?);
                resolved
            }
            _ => return Err("resource resolver does not support this locator contract".into()),
        };
        // Model judgment can raise the risk of an ordinary read, never lower
        // Host-observed sensitivity or authorize an operation by itself.
        let observation = if observation == PermissionOperationRisk::OrdinaryRead
            && input.get("sensitive_read_reason").and_then(Value::as_str).is_some_and(|reason| !reason.trim().is_empty()) {
            PermissionOperationRisk::SensitiveRead
        } else { observation };
        bindings.push(ResolvedDemandBinding {demand:demand.clone(),resource,observation,evidence_id:evidence_id(&evidence),browser,derived});
    }
    let facts = PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
        invocation_digest:request.invocation_digest,bindings};
    facts.subject_for(request).map_err(|error|error.to_string())?;
    Ok(facts)
}

fn file_resource_components(path: &Path) -> Result<Vec<String>, String> {
    path.components().filter_map(|component| match component {
        std::path::Component::Normal(value) => Some(value.to_str().map(str::to_owned)
            .ok_or_else(|| "Permission file path is not UTF-8".to_string())),
        _ => None,
    }).collect()
}

fn move_import_resource_members(context: &ToolContext, plan: &MoveImportPlan, preview: bool)
    -> Result<Vec<HostDerivedResourceMember>, String> {
    let mut members = Vec::new();
    for update in &plan.updates {
        let identity = path_identity(&update.path)?;
        let filesystem_id = identity.pointer("/platform/device").map(Value::to_string).unwrap_or_else(|| "host-filesystem".into());
        let before_version = format!("sha256:{}", update.version.sha256);
        let planned_after_version = format!("sha256:{}", sha256_hex(update.updated.as_bytes()));
        let evidence_id = evidence_id(&json!({"identity":identity,"before_version":before_version,
            "planned_after_version":planned_after_version}));
        members.push(HostDerivedResourceMember { resource:PermissionResource::File {
            filesystem_id, components:file_resource_components(&update.path)?,
            access:if preview {"read"} else {"overwrite"}.into(),
        }, before_version,planned_after_version,evidence_id,
            observation:if !preview {PermissionOperationRisk::Overwrite} else if sensitive_path(&update.path,context) {
                PermissionOperationRisk::SensitiveRead
            } else {PermissionOperationRisk::OrdinaryRead},
        });
    }
    // Match the Domain's canonical member ordering, independently of host path
    // ordering (including platform path separators).
    members.sort_by(|left,right| {
        let components = |member:&HostDerivedResourceMember| match &member.resource {
            PermissionResource::File {components,..} => components.clone(), _ => unreachable!(),
        };
        components(left).cmp(&components(right))
    });
    Ok(members)
}

pub(super) fn validate_move_import_plan(context: &ToolContext, input: &Value, plan: &MoveImportPlan) -> Result<(), String> {
    if input.get("update_imports").and_then(Value::as_bool) != Some(true) { return Ok(()); }
    let (request,facts) = current_derived_permission_expectation()
        .ok_or("Import updates lack an independently admitted Host resource collection")?;
    if request.invocation_digest != PermissionResolutionRequest::input_digest(input) {
        return Err("Import plan belongs to another normalized invocation".into());
    }
    let bindings = facts.bindings.iter().filter(|binding|
        binding.demand.locator_kind == ResourceLocatorKind::DerivedFileCollection).collect::<Vec<_>>();
    if bindings.len() != 1 { return Err("Import plan has no unique declared collection".into()); }
    let binding = bindings[0];
    let members = move_import_resource_members(context,plan,false)?;
    let expected = HostDerivedResourceCollection { collection_id:binding.demand.requirement_id.clone(), discovery_complete:true,
        plan_digest:HostDerivedResourceCollection::digest(&request,&binding.demand,&members), members };
    if binding.derived.as_ref() != Some(&expected) {
        return Err("Import members or versions changed after approval; resolve and approve the new plan".into());
    }
    let PermissionResource::File {components,..} = &binding.resource else { return Err("Import collection has no file boundary".into()); };
    if *components != file_resource_components(&plan.root)? { return Err("Import collection boundary changed".into()); }
    Ok(())
}

fn process_working_directory(context: &ToolContext,input: &Value) -> Result<PathBuf,String> {
    let raw = input.get("cwd").and_then(Value::as_str).filter(|cwd|!cwd.trim().is_empty())
        .map(str::to_owned).or_else(||context.allowed_dirs.first().map(|path|path.to_string_lossy().into_owned()))
        .ok_or("no shell resource base directory")?;
    let cwd = resolved_path(&raw,context)?;
    if !cwd.is_dir() { return Err("shell resource cwd is not a directory".into()); }
    Ok(cwd)
}

fn inspect_read_only_shell(context: &ToolContext, input: &Value) -> Result<(PermissionOperationRisk, Value), String> {
    let command = input.get("command").and_then(Value::as_str).unwrap_or_default();
    let unknown = || (PermissionOperationRisk::Unknown, input.clone());
    if input.get("execution_mode").and_then(Value::as_str) == Some("background") { return Ok(unknown()); }
    let cwd = process_working_directory(context,input)?;
    if let Some(plan) = super::git_read_plan::GitReadPlan::from_command(command, &cwd) {
        if sensitive_path(&plan.root, context) { return Ok(unknown()); }
        if !plan.has_passive_configuration() { return Ok(unknown()); }
        return Ok((PermissionOperationRisk::OrdinaryRead,json!({
            "program":crate::platform_paths::git_program(), "cwd":cwd, "resources":[path_identity(&plan.root)?],
            "verified_command":plan.command, "input":input
        })));
    }
    if let Some(plan) = super::shell_read_plan::ShellReadPlan::from_command(command, &cwd) {
        let sensitive = plan.paths.iter().any(|path| sensitive_path(path, context));
        let resources = plan.paths.iter().map(|path| path_identity(path)).collect::<Result<Vec<_>, _>>()?;
        return Ok((if sensitive { PermissionOperationRisk::SensitiveRead } else { PermissionOperationRisk::OrdinaryRead },
            json!({"cwd":cwd,"resources":resources,"verified_command":plan.command,"input":input})));
    }
    let Some(tokens) = nola_core::agent::permissions::supported_shell_tokens(command) else { return Ok(unknown()); };
    // Require literal argv. Expansion, programs, options with side effects and
    // compound shell syntax never acquire read authority from operation_kind.
    if tokens.iter().any(|token| token.contains(['*','?','[',']','{','}','~','\0'])) { return Ok(unknown()); }
    let program = tokens[0].as_str();
    let basename = Path::new(program).file_name().and_then(|name|name.to_str()).unwrap_or_default();
    let flags: &[&str] = match basename {
        "pwd" => &["-L","-P"], "cat" => &["-n","-b","-s","-v","-e","-t"],
        "ls" => &["-a","-l","-la","-al","-h","-lah","-1","-d","-R"],
        "wc" => &["-l","-w","-c","-m"], "head" | "tail" => &["-n","-c"],
        "stat" => &[], _ => return Ok(unknown()),
    };
    let system_program = [format!("/bin/{basename}"),format!("/usr/bin/{basename}")].into_iter()
        .find(|candidate|Path::new(candidate).is_file());
    if system_program.is_none() || (program.contains('/') && system_program.as_deref() != Some(program)) { return Ok(unknown()); }
    let mut resources = Vec::new();
    let cwd = process_working_directory(context,input)?;
    let mut numeric_argument = false;
    for token in tokens.iter().skip(1) {
        if numeric_argument {
            if token.parse::<u64>().is_err() { return Ok(unknown()); }
            numeric_argument = false; continue;
        }
        if token.starts_with('-') {
            if !flags.contains(&token.as_str()) { return Ok(unknown()); }
            numeric_argument = matches!(token.as_str(),"-n"|"-c") && matches!(basename,"head"|"tail");
            continue;
        }
        let path = cwd.join(token);
        let path = path.canonicalize().map_err(|error|format!("cannot resolve shell read resource: {error}"))?;
        if sensitive_path(&path,context) { return Ok((PermissionOperationRisk::SensitiveRead,path_identity(&path)?)); }
        resources.push(path_identity(&path)?);
    }
    if numeric_argument { return Ok(unknown()); }
    if resources.is_empty() && matches!(basename,"cat"|"head"|"tail"|"wc") { return Ok(unknown()); }
    if resources.is_empty() { resources.push(path_identity(&cwd)?); }
    Ok((PermissionOperationRisk::OrdinaryRead,json!({"program":system_program,"cwd":cwd,"resources":resources,"input":input})))
}

pub(super) fn verified_read_only_shell_command(context: &ToolContext, args: &Map<String,Value>) -> Option<String> {
    let input = Value::Object(args.clone());
    let (risk, evidence) = inspect_read_only_shell(context,&input).ok()?;
    if risk != PermissionOperationRisk::OrdinaryRead { return None; }
    if let Some(command) = evidence.get("verified_command").and_then(Value::as_str) {
        return Some(command.to_owned());
    }
    let tokens = nola_core::agent::permissions::supported_shell_tokens(input["command"].as_str()?)?;
    let quote = |value: &str| format!("'{}'",value.replace('\'',"'\\''"));
    Some(std::iter::once(quote(evidence["program"].as_str()?)).chain(tokens.iter().skip(1).map(|value|quote(value)))
        .collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(root: &Path) -> ToolContext {
        ToolContext {memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),language_services: Default::default(), skills:crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs:vec![root.to_path_buf()],policy:ToolExecutionPolicy::new(Vec::new(),false,"ask".into(),Vec::new()),
            permission_scope:Default::default(),cancellation:Default::default(),plan_state:Default::default(),
            output_store:ToolOutputStore::for_test(),browser:BrowserRuntime::new(root.join("browser"))}
    }
    fn facts(context: &ToolContext, name: &str, input: Value) -> PermissionResolutionFacts {
        let plan = builtin_permission_contract(name,false).resource_plan_for(&input).unwrap();
        resolve_permission_resources(context,&PermissionResolutionRequest {resolution_id:"resolution".into(),
            execution_target:"isolated-host".into(),invocation_digest:PermissionResolutionRequest::input_digest(&input),
            capability:plan.capability.clone(),operation:plan.operation.clone(),demands:plan.extract(&input).unwrap()},&input).unwrap()
    }
    #[test]
    fn code_memory_index_resolves_the_actual_index_inventory_before_authorization() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        fs::write(directory.path().join("main.rs"), "fn main() {}").unwrap();
        let input = json!({"repo_path":directory.path()});
        let ordinary = facts(&context, "codebase_index", input.clone());
        assert!(ordinary.bindings.iter().all(|binding|
            binding.observation == PermissionOperationRisk::OrdinaryRead));
        fs::create_dir(directory.path().join(".ssh")).unwrap();
        fs::write(directory.path().join(".ssh/generated.rs"), "fn sensitive_fixture() {}").unwrap();
        let sensitive = facts(&context, "codebase_index", input.clone());
        assert!(sensitive.bindings.iter().any(|binding|
            binding.observation == PermissionOperationRisk::SensitiveRead));
        fs::write(directory.path().join(".gitignore"), ".ssh/\n").unwrap();
        let ignored = facts(&context, "codebase_index", input);
        assert!(ignored.bindings.iter().all(|binding|
            binding.observation == PermissionOperationRisk::OrdinaryRead),
            "ignored sources must not be read or cause an unrelated approval");
    }
    #[test]
    fn outside_workspace_is_ordinary_but_sensitive_reads_require_approval() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let context = context(workspace.path());
        let ordinary = outside.path().join("notes.txt");
        fs::write(&ordinary, "fixture").unwrap();
        assert_eq!(facts(&context,"read_file",json!({"path":ordinary})).bindings[0].observation,
            PermissionOperationRisk::OrdinaryRead);
        assert_eq!(facts(&context,"read_file",json!({"path":ordinary,"sensitive_read_reason":"Contains personal medical notes"})).bindings[0].observation,
            PermissionOperationRisk::SensitiveRead);
        let secret = outside.path().join(".env");
        fs::write(&secret, "synthetic fixture").unwrap();
        assert_eq!(facts(&context,"read_file",json!({"path":secret})).bindings[0].observation,
            PermissionOperationRisk::SensitiveRead);
        assert_eq!(facts(&context,"write_file",json!({"path":ordinary,"content":"replacement"})).bindings[0].observation,
            PermissionOperationRisk::Overwrite);
        assert_eq!(facts(&context,"write_file",json!({"path":outside.path().join("new.txt"),"content":"new"})).bindings[0].observation,
            PermissionOperationRisk::NewFile);
        #[cfg(unix)] {
            let alias = workspace.path().join("ordinary-name.txt");
            std::os::unix::fs::symlink(&secret,&alias).unwrap();
            assert_eq!(facts(&context,"read_file",json!({"path":alias})).bindings[0].observation,
                PermissionOperationRisk::SensitiveRead);
        }
    }

    fn browser_facts() -> BrowserPermissionResourceFacts {
        BrowserPermissionResourceFacts {ok:true,target:"managed".into(),owner_id:"owner".into(),
            connection_state:"existing".into(),connection_generation:Some("generation".into()),page_id:Some("page".into()),
            page_origin:Some("https://example.test".into()),requested_origin:None,authentication_state:"unknown".into()}
    }
    #[test]
    fn mcp_new_connection_uses_concrete_owner_facts_and_both_profiles() {
        use nola_core::agent::permissions::{PermissionRuleSet,PermissionSubject};
        use nola_core::agent::protocol::AgentInputOrigin;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let owner = uuid::Uuid::new_v4().to_string();
        context.permission_scope.bind_owner_session(&owner);
        let configured = crate::mcp_connection_authority::bind_configurations(&[
            json!({"server_id":"permission-fixture","endpoint":format!("http://{}/mcp",listener.local_addr().unwrap())})],
            "isolated-host",&owner).unwrap();
        crate::mcp_host::register_configured_servers(&configured).unwrap();
        let input = json!({"connection_ref":configured[0]["connection_ref"]});
        let contract = builtin_permission_contract("mcp_connect",false);
        let plan = contract.resource_plan_for(&input).unwrap();
        let request = PermissionResolutionRequest {resolution_id:"mcp-resolution".into(),execution_target:"isolated-host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),
            operation:plan.operation.clone(),demands:plan.extract(&input).unwrap()};
        let facts = resolve_permission_resources(&context,&request,&input).unwrap();
        assert_eq!(facts.bindings[0].observation,PermissionOperationRisk::NewConnection);
        assert!(facts.subject_for(&request).is_ok());
        let subject = PermissionSubject {capability:"mcp_connection".into(),resource:input.to_string()};
        for (profile,decision) in [(nola_domain::PermissionProfile::Cautious,nola_domain::PermissionPolicyDecision::Ask),
            (nola_domain::PermissionProfile::AllowAll,nola_domain::PermissionPolicyDecision::Allow)] {
            assert_eq!(PermissionRuleSet::default().with_profile(profile).evaluate_resolved_for_origin_with_explanation(
                &subject,AgentInputOrigin::InteractiveUser,true).decision,decision);
        }
        let mut foreign = request.clone(); foreign.execution_target = "another-host".into();
        assert!(resolve_permission_resources(&context,&foreign,&input).is_err());
        assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
    }
    #[test]
    fn current_owner_goal_declaration_is_not_an_external_action_grant() {
        let contract = builtin_permission_contract("goal_complete", false);
        assert_eq!(contract.risk, PermissionOperationRisk::OrdinaryRead);
        assert!(contract.resource_plan.is_none());
        assert_eq!(builtin_permission_contract("delete_file", false).risk, PermissionOperationRisk::Delete);
        assert_eq!(builtin_permission_contract("browser_start", false).risk, PermissionOperationRisk::Unknown);
    }
    #[test]
    fn internal_skill_progress_keeps_its_transaction_without_prompting_for_external_access() {
        use nola_core::agent::permissions::{PermissionRuleSet,PermissionSubject};
        use nola_core::agent::protocol::AgentInputOrigin;
        for name in ["skill_update", "skill_skip", "sop_run_update"] {
            let contract = builtin_permission_contract(name, false);
            assert_eq!(contract.risk, PermissionOperationRisk::OrdinaryRead);
            assert!(!contract.risk.requires_confirmation());
            assert!(contract.resource_plan.is_none());
            let subject = PermissionSubject { capability: name.into(), resource: "current-skill".into() };
            let rules = PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::Cautious);
            assert_eq!(rules.evaluate_resolved_for_origin_with_explanation(
                &subject, AgentInputOrigin::InteractiveUser, false).decision, nola_domain::PermissionPolicyDecision::Allow);
        }
        for decision in [nola_domain::PermissionPolicyDecision::Ask, nola_domain::PermissionPolicyDecision::Deny] {
            let mut rules = PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::Cautious);
            rules.rules.push(nola_core::agent::permissions::PermissionRule {
                capability_pattern: "skill_update".into(), resource_pattern: "*".into(),
                decision, source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
            });
            let subject = PermissionSubject { capability: "skill_update".into(), resource: "current-skill".into() };
            assert_eq!(rules.evaluate_resolved_for_origin_with_explanation(&subject, AgentInputOrigin::InteractiveUser, false).decision, decision);
        }
        assert!(builtin_permission_contract("unknown_mutation", false).risk.requires_confirmation());
        assert!(builtin_permission_contract("delete_file", false).risk.requires_confirmation());
    }

    #[test]
    fn fixed_http_retrieval_contracts_are_not_upload_or_message_tools() {
        for capability in ["web_fetch", "web_search"] {
            let contract = builtin_permission_contract(capability, true);
            assert_eq!(contract.risk, PermissionOperationRisk::OrdinaryRead);
            assert!(!contract.risk.requires_confirmation());
            assert!(contract.resource_plan.is_none());
        }
        assert!(PermissionOperationRisk::ExternalSend.requires_confirmation(),
            "a real trusted sending operation must not inherit retrieval risk");
    }

    #[test]
    fn http_retrieval_rejects_outbound_extensions_before_dispatch_in_both_profiles() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        for (capability, baseline) in [
            ("web_fetch", json!({"url":"https://example.com/reference"})),
            ("web_search", json!({"query":"current public reference"})),
        ] {
            let valid = ToolRequest {id:"retrieval-schema".into(),name:capability.into(),
                args:baseline.as_object().unwrap().clone()};
            assert!(validate_tool_request(&valid).is_none(), "current valid read arguments remain accepted");
            for (field, value) in [
                ("method", json!("POST")),
                ("body", json!({"message":"synthetic outgoing message"})),
                ("files", json!(["synthetic-private-file.txt"])),
                ("headers", json!({"authorization":"synthetic-test-only"})),
                ("recipient", json!("synthetic-recipient")),
            ] {
                let mut request = valid.clone();
                request.args.insert(field.into(), value);
                let preflight = validate_tool_request(&request).expect("undeclared argument must fail");
                assert!(preflight.error);
                assert!(preflight.text.contains("invalid_tool_arguments"));
                for profile in [nola_domain::PermissionProfile::Cautious, nola_domain::PermissionProfile::AllowAll] {
                    let outcome = execute_tool_inner(&request,&context,false,
                        ExecutionAuthority::CoreApproved(profile),None);
                    assert!(outcome.error, "{capability}.{field} must not reach a network handler");
                    assert!(outcome.text.contains("invalid_tool_arguments"));
                    assert!(outcome.text.contains("tool_execution_skipped: true"));
                    assert!(outcome.execution.is_none());
                    assert!(outcome.artifacts.is_empty());
                }
            }
        }
    }

    #[test]
    fn unregistered_http_upload_and_external_message_names_have_no_dispatch_authority() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let catalog = specs();
        for capability in ["http_request","http_post","upload_file","browser_upload_file",
            "chrome_native_upload_file","send_message","send_email"] {
            assert!(!catalog.iter().any(|spec|spec.name == capability),
                "this regression describes the current tool surface, not a keyword denylist");
            for profile in [nola_domain::PermissionProfile::Cautious, nola_domain::PermissionProfile::AllowAll] {
                let request = ToolRequest {id:"unregistered-send".into(),name:capability.into(),
                    args:json!({"url":"https://example.com/messages","method":"POST",
                        "body":"synthetic message"}).as_object().unwrap().clone()};
                let outcome = execute_tool_inner(&request,&context,false,
                    ExecutionAuthority::CoreApproved(profile),None);
                assert!(outcome.error);
                assert!(outcome.text.contains("Unknown tool"), "{capability}: {}",outcome.text);
                assert!(outcome.execution.is_none());
                assert!(outcome.artifacts.is_empty());
            }
        }
        assert_eq!(chrome_native_action("chrome_native_upload_file"),None);
    }
    #[test]
    fn task_creation_does_not_grant_child_overwrite_or_process_authority() {
        use nola_core::agent::permissions::{PermissionRuleSet, PermissionSubject};
        use nola_core::agent::protocol::AgentInputOrigin;
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        context.permission_scope.bind_owner_session("parent-owner");
        fs::write(directory.path().join("owned.txt"), "existing").unwrap();
        let catalog = crate::agent_host_driver::build_cli_host_catalog("task-creation-contract");
        let creation = &catalog.capabilities["spawn_agent"];
        let input = json!({"agent_type":"general","description":"Child","prompt":"Perform work"});
        let contract = &creation.input_normalization.permission_contract;
        assert_eq!(contract.risk, PermissionOperationRisk::NewFile);
        let confirmation = nola_core::agent::permissions::permission_contract_requires_confirmation(contract, None);
        assert!(!confirmation);
        let subject = creation.permission_subject(&input).unwrap();
        assert_eq!(PermissionRuleSet::default().evaluate_resolved_for_origin_with_explanation(
            &subject, AgentInputOrigin::InteractiveUser, confirmation).decision,
            nola_domain::PermissionPolicyDecision::Allow);

        let overwrite_input = json!({"path":"owned.txt","content":"replacement"});
        let overwrite = facts(&context,"write_file",overwrite_input);
        assert_eq!(overwrite.bindings[0].observation, PermissionOperationRisk::Overwrite);
        let child = PermissionSubject {capability:"edit".into(),resource:"owned.txt".into()};
        for (profile, expected) in [(nola_domain::PermissionProfile::Cautious,nola_domain::PermissionPolicyDecision::Ask),
            (nola_domain::PermissionProfile::AllowAll,nola_domain::PermissionPolicyDecision::Allow)] {
            assert_eq!(PermissionRuleSet::default().with_profile(profile)
                .evaluate_resolved_for_origin_with_explanation(&child, AgentInputOrigin::DelegatedAgent, true).decision, expected);
        }
        let process = builtin_permission_contract("bash_background_start", false);
        assert_eq!(process.risk, PermissionOperationRisk::Unknown);
        assert!(process.resource_plan.is_some());
        assert!(nola_core::agent::permissions::permission_contract_requires_confirmation(&process, None));
    }
    #[test]
    fn browser_unknown_authentication_never_becomes_public_read_authority() {
        let facts = browser_facts();
        let (resource,risk,_) = browser_resource_from_facts("managed","owner","browser_observe",&json!({}),facts.clone()).unwrap();
        assert_eq!(risk,PermissionOperationRisk::SensitiveRead);
        let mut public = facts.clone(); public.authentication_state = "public".into();
        assert!(browser_resource_from_facts("managed","owner","browser_observe",&json!({}),public).is_err());
        assert!(browser_resource_from_facts("managed","another-owner","browser_observe",&json!({}),facts.clone()).is_err());
        let mut changed = facts; changed.connection_generation = Some("another-generation".into());
        assert_ne!(resource,browser_resource_from_facts("managed","owner","browser_observe",&json!({}),changed).unwrap().0);
    }
    #[test]
    fn browser_observation_scope_ignores_query_options_but_not_page_identity() {
        let snapshot = builtin_permission_contract("browser_snapshot",true).resource_plan.unwrap();
        let screenshot = builtin_permission_contract("browser_screenshot",true).resource_plan.unwrap();
        assert_eq!(snapshot,screenshot);
        let resolve = |input:&Value,facts:BrowserPermissionResourceFacts| {
            let (resource,observation,evidence) = browser_resource_from_facts("managed","owner","browser_observe",input,facts.clone()).unwrap();
            let request = PermissionResolutionRequest {resolution_id:"browser-facts".into(),execution_target:"host".into(),
                invocation_digest:PermissionResolutionRequest::input_digest(input),capability:snapshot.capability.clone(),
                operation:snapshot.operation.clone(),demands:snapshot.extract(input).unwrap()};
            PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
                invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {demand:request.demands[0].clone(),
                    resource,observation,evidence_id:evidence_id(&evidence),browser:Some(facts),derived:None}]}
                .subject_for(&request).unwrap().approval_resource_identity().unwrap()
        };
        let scope = resolve(&json!({"max_text_chars":1000}),browser_facts());
        assert_eq!(scope,resolve(&json!({"full_page":true}),browser_facts()));
        let mut changed = browser_facts(); changed.page_id = Some("different-page".into());
        assert_ne!(scope,resolve(&json!({}),changed));
    }
    #[test]
    fn browser_new_connection_and_cross_origin_navigation_require_confirmation() {
        let mut absent = browser_facts(); absent.connection_state = "absent".into();
        absent.connection_generation = None; absent.page_id = None; absent.page_origin = None;
        assert_eq!(browser_resource_from_facts("managed","owner","browser_connect",&json!({}),absent.clone()).unwrap().1,
            PermissionOperationRisk::NewConnection);
        assert_eq!(browser_resource_from_facts("managed","owner","browser_status",&json!({}),absent.clone()).unwrap().1,
            PermissionOperationRisk::OrdinaryRead);
        assert!(browser_resource_from_facts("managed","owner","browser_observe",&json!({}),absent).is_err());
        let mut cross = browser_facts(); cross.requested_origin = Some("https://destination.test".into());
        assert_eq!(browser_resource_from_facts("managed","owner","browser_navigate",&json!({"url":"https://destination.test/path"}),cross).unwrap().1,
            PermissionOperationRisk::NewConnection);
    }
    #[test]
    fn browser_expectation_is_effect_local_nested_and_not_inherited_by_workers() {
        assert!(current_browser_permission_resource_expectation().is_none());
        let _guard = enter_browser_permission_resource_expectation(Some(json!({"owner_id":"first"})));
        assert_eq!(current_browser_permission_resource_expectation().unwrap()["owner_id"],"first");
        {
            let _clear = enter_browser_permission_resource_expectation(None);
            assert!(current_browser_permission_resource_expectation().is_none());
        }
        assert!(std::thread::spawn(current_browser_permission_resource_expectation).join().unwrap().is_none());
        assert_eq!(current_browser_permission_resource_expectation().unwrap()["owner_id"],"first");
    }
    #[test]
    fn execution_revalidation_rejects_changed_facts_and_no_plan_needs_no_resolver() {
        let directory = tempfile::tempdir().unwrap(); let context = context(directory.path());
        let input = json!({"path":"notes.txt","content":"new"});
        let contract = builtin_permission_contract("write_file",false);
        let plan = contract.resource_plan_for(&input).unwrap();
        let resolution = PermissionResolutionRequest {resolution_id:"execution".into(),execution_target:"host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),operation:plan.operation.clone(),
            demands:plan.extract(&input).unwrap()};
        let facts = resolve_permission_resources(&context,&resolution,&input).unwrap();
        let grant = json!({"permission_resolution":{"request":resolution,"facts":facts}});
        assert!(validate_permission_resource_execution(&context,&contract,"host",&input,Some(&grant)).unwrap().browser.is_none());
        fs::write(directory.path().join("notes.txt"),"now exists").unwrap();
        assert!(validate_permission_resource_execution(&context,&contract,"host",&input,Some(&grant)).is_err());
        assert!(validate_permission_resource_execution(&context,&contract,"other-host",&input,Some(&grant)).is_err());
        assert!(validate_permission_resource_execution(&context,&builtin_permission_contract("web_search",true),"host",&json!({}),None).unwrap().browser.is_none());
    }
    #[cfg(unix)]
    #[test]
    fn directory_read_revalidation_ignores_ordinary_journals_but_rejects_sensitive_or_replaced_scope() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let root = directory.path().join("scope");
        fs::create_dir(&root).unwrap();
        let input = json!({"root":root});
        let contract = builtin_permission_contract("project_overview", true);
        let plan = contract.resource_plan_for(&input).unwrap();
        let resolution = PermissionResolutionRequest {resolution_id:"directory-observation".into(),execution_target:"host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),operation:plan.operation.clone(),
            demands:plan.extract(&input).unwrap()};
        let facts = resolve_permission_resources(&context,&resolution,&input).unwrap();
        assert_eq!(facts.bindings[0].observation, PermissionOperationRisk::OrdinaryRead);
        let grant = json!({"permission_resolution":{"request":resolution,"facts":facts}});

        fs::write(root.join("host-journal.log"), "ordinary local metadata").unwrap();
        assert!(validate_permission_resource_execution(&context,&contract,"host",&input,Some(&grant)).unwrap().browser.is_none());
        fs::write(root.join(".env"), "synthetic sensitive fixture").unwrap();
        assert!(validate_permission_resource_execution(&context,&contract,"host",&input,Some(&grant)).is_err(),
            "the complete sensitive resource set must still be inspected before execution");

        fs::rename(&root, directory.path().join("previous-scope")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(validate_permission_resource_execution(&context,&contract,"host",&input,Some(&grant)).is_err(),
            "a different directory inode is not the approved scope even at the same path");
    }
    #[test]
    fn explicit_process_scope_authorizes_execution_not_immutable_payload_bytes() {
        let _environment = crate::store::test_env_lock();
        let previous = std::env::var_os("NOLA_OPAQUE_APPROVAL_INPUT");
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore { fn drop(&mut self) {
            if let Some(value) = &self.0 { std::env::set_var("NOLA_OPAQUE_APPROVAL_INPUT",value); }
            else { std::env::remove_var("NOLA_OPAQUE_APPROVAL_INPUT"); }
        } }
        let _restore = Restore(previous);
        std::env::set_var("NOLA_OPAQUE_APPROVAL_INPUT","first-fixture-target");
        let directory = tempfile::tempdir().unwrap(); let context = context(directory.path());
        let spec = specs_with_skills(&context.skills).into_iter().find(|spec|spec.name == "bash").unwrap();
        let payload = directory.path().join("payload.txt");
        fs::write(&payload,"first synthetic body").unwrap();
        // Neither a literal command nor its digest freezes @file or inherited
        // environment. No network/process is executed by this regression.
        let request = ToolRequest {id:"opaque-first".into(),name:"bash".into(),args:json!({
            "command":"curl --data-binary @payload.txt https://example.test/$NOLA_OPAQUE_APPROVAL_INPUT",
            "cwd":directory.path().to_string_lossy(),
            "operation_kind":"execute",
        }).as_object().unwrap().clone()};
        let first = LocalPermissionResources::prepare(&context,&spec,&request).unwrap();
        assert!(first.requires_confirmation(&Value::Object(request.args.clone())));
        assert!(first.allow_conversation);
        assert!(!first.approved_in_scope(&context));
        first.remember_approval(&context).unwrap();
        assert!(first.approved_in_scope(&context));
        fs::write(&payload,"different synthetic body").unwrap();
        std::env::set_var("NOLA_OPAQUE_APPROVAL_INPUT","different-fixture-target");
        let mut repeated = request.clone(); repeated.id = "opaque-second".into();
        let second = LocalPermissionResources::prepare(&context,&spec,&repeated).unwrap();
        assert_eq!(first.scope,second.scope,"a selected process grant is not a payload snapshot");
        assert!(second.approved_in_scope(&context));
        let mut different = repeated.clone();
        different.args.insert("command".into(),json!("another-command"));
        let changed = LocalPermissionResources::prepare(&context,&spec,&different).unwrap();
        assert!(!changed.approved_in_scope(&context));
        different = repeated.clone();
        let other = tempfile::tempdir().unwrap();
        different.args.insert("cwd".into(),json!(other.path()));
        assert!(!LocalPermissionResources::prepare(&context,&spec,&different).unwrap().approved_in_scope(&context));
        assert!(second.requires_confirmation(&Value::Object(repeated.args.clone())));
        let _session = enter_host_permission_session("opaque-once");
        let _resolver = enter_host_permission_resolver(std::sync::Arc::new(|_,_,allow_conversation| {
            assert!(allow_conversation); HostPermissionResolution::Allow
        }));
        assert_eq!(confirm_local_permission(&repeated,second.allow_conversation),ToolPermissionDecision::Allow);
    }
    #[cfg(unix)]
    #[test]
    fn process_grant_rechecks_new_sensitive_targets_before_reuse() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let spec = specs_with_skills(&context.skills).into_iter().find(|spec|spec.name == "bash").unwrap();
        fs::write(directory.path().join("ordinary.txt"),"fixture").unwrap();
        fs::write(directory.path().join(".env"),"synthetic-secret").unwrap();
        let alias = directory.path().join("alias.txt");
        std::os::unix::fs::symlink(directory.path().join("ordinary.txt"), &alias).unwrap();
        let request = ToolRequest {id:"first".into(), name:"bash".into(),
            args:json!({"command":"cat alias.txt","cwd":directory.path(),"operation_kind":"inspect"}).as_object().unwrap().clone()};
        let first = LocalPermissionResources::prepare(&context,&spec,&request).unwrap();
        first.remember_approval(&context).unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(directory.path().join(".env"), &alias).unwrap();
        let second = LocalPermissionResources::prepare(&context,&spec,&request).unwrap();
        assert!(second.requires_confirmation(&Value::Object(request.args.clone())));
        assert!(!second.approved_in_scope(&context));
        assert_ne!(first.scope, second.scope);
        second.remember_approval(&context).unwrap();
        let other = directory.path().join(".ssh");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("key"),"second synthetic secret").unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(other.join("key"), &alias).unwrap();
        let third = LocalPermissionResources::prepare(&context,&spec,&request).unwrap();
        assert!(!third.approved_in_scope(&context), "same risk does not grant another sensitive target");
    }

    #[test]
    fn direct_policy_uses_resource_facts_and_conversation_grants_never_expand_to_peers() {
        let directory = tempfile::tempdir().unwrap(); let context = context(directory.path());
        context.permission_scope.bind_owner_session("first-owner");
        let spec = specs_with_skills(&context.skills).into_iter().find(|spec|spec.name == "write_file").unwrap();
        let request = ToolRequest {id:"direct".into(),name:"write_file".into(),
            args:json!({"path":"notes.txt","content":"ordinary"}).as_object().unwrap().clone()};
        let new = LocalPermissionResources::prepare(&context,&spec,&request).unwrap();
        assert!(!new.requires_confirmation(&Value::Object(request.args.clone())));
        fs::write(directory.path().join("notes.txt"),"existing").unwrap();
        let overwrite = LocalPermissionResources::prepare(&context,&spec,&request).unwrap();
        assert!(overwrite.requires_confirmation(&Value::Object(request.args.clone())));
        overwrite.remember_approval(&context).unwrap();
        assert!(overwrite.approved_in_scope(&context));
        let peer = ToolContext {memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),permission_scope:Default::default(),..context.clone()};
        assert!(!overwrite.approved_in_scope(&peer));
        context.permission_scope.bind_owner_session("another-owner");
        assert!(!overwrite.approved_in_scope(&context));
        let mut forged = request; forged.args.insert("_nola_approved_once".into(),json!(true));
        assert!(LocalPermissionResources::prepare(&context,&spec,&forged).is_err());
    }
    #[test]
    fn instruction_preflight_inspects_only_declared_candidates_not_unrelated_secrets() {
        let directory = tempfile::tempdir().unwrap(); let context = context(directory.path());
        fs::write(directory.path().join(".env"),"secret fixture").unwrap();
        fs::write(directory.path().join("AGENTS.md"),"ordinary instructions").unwrap();
        let input = json!({"root":directory.path()});
        assert_eq!(facts(&context,"instruction_context",input).bindings[0].observation,PermissionOperationRisk::OrdinaryRead);
        let explicit = json!({"root":directory.path(),"names":[".env"]});
        assert_eq!(facts(&context,"instruction_context",explicit).bindings[0].observation,PermissionOperationRisk::SensitiveRead);
    }
    #[test]
    fn canonical_history_scope_is_host_owned_and_private_history_requires_cautious_consent() {
        let directory = tempfile::tempdir().unwrap(); let context = context(directory.path());
        context.permission_scope.bind_owner_session("current");
        let identity = |input:Value| {
            let contract = builtin_permission_contract("conversation_recall",true);
            let plan = contract.resource_plan_for(&input).unwrap();
            let request = PermissionResolutionRequest {resolution_id:"history".into(),execution_target:"host".into(),
                invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),operation:plan.operation.clone(),
                demands:plan.extract(&input).unwrap()};
            let facts = resolve_permission_resources(&context,&request,&input).unwrap();
            let resource = facts.subject_for(&request).unwrap().approval_resource_identity().unwrap();
            let confirmation = nola_core::agent::permissions::permission_contract_requires_confirmation(&contract,Some(&(request,facts)));
            (resource,confirmation)
        };
        let (current,confirmation) = identity(json!({"query":"ordinary current","scope":"current_conversation"}));
        assert!(!confirmation);
        assert_eq!(current,identity(json!({"query":"后续検索","scope":"conversations","conversation_ids":["current"]})).0);
        let (private,confirmation) = identity(json!({"query":"first","scope":"local_user_history"}));
        assert!(confirmation);
        assert_eq!(private,identity(json!({"query":"different query","scope":"local_user_history"})).0);
        let subject = nola_core::agent::permissions::PermissionSubject {capability:"conversation_recall".into(),resource:private};
        for (profile,expected) in [(nola_domain::PermissionProfile::Cautious,nola_domain::PermissionPolicyDecision::Ask),
            (nola_domain::PermissionProfile::AllowAll,nola_domain::PermissionPolicyDecision::Allow)] {
            let evaluation = nola_core::agent::permissions::PermissionRuleSet::default().with_profile(profile)
                .evaluate_resolved_for_origin_with_explanation(&subject,nola_core::agent::protocol::AgentInputOrigin::InteractiveUser,confirmation);
            assert_eq!(evaluation.decision,expected);
        }
        assert!(identity(json!({"scope":"current_conversation","conversation_id":"other","message_id":"m","part_id":"p"})).1,
            "a direct part lookup cannot claim current scope while targeting a different conversation");
        assert!(!identity(json!({"message_id":"m","part_id":"p"})).1);
    }
    #[test]
    fn filesystem_observations_distinguish_creation_existing_content_and_secret_reads() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let input = json!({"path":"notes.txt","content":"hello","operation_kind":"inspect"});
        let new = facts(&context,"write_file",input.clone());
        assert_eq!(new.bindings[0].observation,PermissionOperationRisk::NewFile);
        fs::write(directory.path().join("notes.txt"),"existing").unwrap();
        let existing = facts(&context,"write_file",input);
        assert_eq!(existing.bindings[0].observation,PermissionOperationRisk::Overwrite);
        assert_ne!(existing.bindings[0].evidence_id,new.bindings[0].evidence_id);
        fs::write(directory.path().join(".env"),"fixture-only").unwrap();
        assert_eq!(facts(&context,"read_file",json!({"path":".env"})).bindings[0].observation,PermissionOperationRisk::SensitiveRead);
        assert_eq!(facts(&context,"write_file",json!({"path":"notes.txt","dry_run":true})).bindings[0].observation,PermissionOperationRisk::OrdinaryRead);
    }
    #[test]
    fn git_read_plan_drives_both_permission_and_execution() {
        let directory = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new(crate::platform_paths::git_program()).args(["init", "--quiet"]).arg(directory.path()).status().unwrap().success());
        let context = context(directory.path());
        let input = json!({"command":"git status --porcelain=v1 | cut -c1-2 | sort | uniq -c", "operation_kind":"execute"});
        assert_eq!(facts(&context,"bash",input.clone()).bindings[0].observation, PermissionOperationRisk::OrdinaryRead);
        let contract = builtin_permission_contract("bash",false);
        let plan = contract.resource_plan_for(&input).unwrap();
        let request = PermissionResolutionRequest {resolution_id:"git-inspection".into(),execution_target:"isolated-host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),
            operation:plan.operation.clone(),demands:plan.extract(&input).unwrap()};
        let observed = resolve_permission_resources(&context,&request,&input).unwrap();
        assert!(!nola_core::agent::permissions::permission_contract_requires_confirmation(&contract,Some(&(request,observed))));
        let command = verified_read_only_shell_command(&context,input.as_object().unwrap()).unwrap();
        assert!(command.contains(crate::platform_paths::git_program()));
        assert!(command.contains("core.fsmonitor=false"));
        for command in ["git commit -m x", "git push", "git status && git commit -m x"] {
            assert_eq!(facts(&context,"bash",json!({"command":command,"operation_kind":"inspect"})).bindings[0].observation, PermissionOperationRisk::Unknown);
        }
        let other = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new(crate::platform_paths::git_program()).args(["init", "--quiet"]).arg(other.path()).status().unwrap().success());
        let command = format!("git -C '{}' status",other.path().display());
        assert_eq!(facts(&context,"bash",json!({"command":command})).bindings[0].observation,PermissionOperationRisk::OrdinaryRead);
    }

    #[test]
    fn latest_cli_source_inspection_commands_keep_bounded_read_authority() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        for folder in ["apps/cli/src/agent_parts", "apps/cli/src/main_parts"] {
            fs::create_dir_all(directory.path().join(folder)).unwrap();
            fs::write(directory.path().join(folder).join("fixture.rs"), "fn fixture() {}\n").unwrap();
        }
        let cases: Vec<Value> = serde_json::from_str(include_str!("../../tests/fixtures/permissions/source_search_commands.json")).unwrap();
        assert_eq!(cases.len(), 26);
        for case in cases {
            let command = case["command"].as_str().unwrap().replace("{workspace}", &format!("'{}'", crate::platform_paths::shell_path(directory.path()).replace('\'', "'\\''")));
            let expected = if case["ordinary_read"] == true { PermissionOperationRisk::OrdinaryRead }
                else { PermissionOperationRisk::Unknown };
            assert_eq!(facts(&context, "bash", json!({"command":command,"operation_kind":"inspect"})).bindings[0].observation,
                expected, "{command}");
        }
    }

    #[test]
    fn source_search_pipelines_are_read_only_with_exact_resource_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        fs::write(directory.path().join("source.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        fs::write(directory.path().join(".env"), "synthetic-secret\n").unwrap();
        for command in [
            "'/usr/bin/grep' -n 'fn alpha\\|fn beta' source.rs | '/usr/bin/head' -5",
            "grep -n 'fn alpha' -A 8 source.rs; echo '==='; grep -n 'fn beta' source.rs | head -10",
            "grep -n 'fn [a-z]*' source.rs && grep -n beta source.rs || echo 'no match'",
            "ls . && echo '---' && ls -l source.rs",
            "sed -n '1,2p' source.rs | head -5",
            "grep -n alpha missing.rs source.rs 2>/dev/null | head -10",
            "grep -n 'fn alpha' source.rs | grep -E 'alpha|beta' | wc -l",
        ] {
            let input = json!({"command":command, "operation_kind":"inspect"});
            let observed = facts(&context, "bash", input.clone());
            assert_eq!(observed.bindings[0].observation, PermissionOperationRisk::OrdinaryRead, "{command}");
            let contract = builtin_permission_contract("bash", false);
            let resource_plan = contract.resource_plan_for(&input).unwrap();
            let request = PermissionResolutionRequest { resolution_id:"resolution".into(),
                execution_target:"isolated-host".into(), invocation_digest:PermissionResolutionRequest::input_digest(&input),
                capability:resource_plan.capability.clone(), operation:resource_plan.operation.clone(),
                demands:resource_plan.extract(&input).unwrap() };
            assert!(!nola_core::agent::permissions::permission_contract_requires_confirmation(
                &contract, Some(&(request, observed))), "{command}");
            let pinned = verified_read_only_shell_command(&context, input.as_object().unwrap()).unwrap();
            let original = std::process::Command::new(crate::platform_paths::bash_program()).args(["-o", "pipefail", "-c", command])
                .current_dir(directory.path()).env_clear().env("PATH", "/usr/bin:/bin").output().unwrap();
            let actual = std::process::Command::new(crate::platform_paths::bash_program()).args(["-o", "pipefail", "-c", &pinned])
                .current_dir(directory.path()).env_clear().env("PATH", "/usr/bin:/bin").output().unwrap();
            assert_eq!(actual.status.code(), original.status.code(), "{command}");
            assert_eq!(actual.stdout, original.stdout, "{command}");
            assert_eq!(actual.stderr, original.stderr, "{command}");
        }
        for command in ["grep secret .env | head -5", "echo ordinary; grep -n secret .env"] {
            assert_eq!(facts(&context,"bash",json!({"command":command})).bindings[0].observation,
                PermissionOperationRisk::SensitiveRead, "{command}");
            assert!(verified_read_only_shell_command(&context,json!({"command":command}).as_object().unwrap()).is_none());
        }
        let source = directory.path().join("src");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("main.rs"), "fn alpha() {}\n").unwrap();
        assert_eq!(facts(&context,"bash",json!({"command":"grep -rn alpha src --include='*.rs' | head -5"})).bindings[0].observation,
            PermissionOperationRisk::OrdinaryRead);
        fs::write(source.join(".env"), "synthetic-secret").unwrap();
        assert_eq!(facts(&context,"bash",json!({"command":"grep -rn alpha src | head -5"})).bindings[0].observation,
            PermissionOperationRisk::SensitiveRead);
        // Inspect labels cannot authorize a write, expansion, recursive read,
        // user executable, hidden pattern file or stream-to-file operation.
        for command in [
            "grep alpha source.rs; touch created", "grep alpha source.rs > created",
            "grep alpha source.rs 2>/dev/null.created", "grep alpha source.rs &",
            "grep alpha $(touch created)", "grep alpha \"$HOME/.env\"",
            "grep alpha source.rs | tee created", "grep -R alpha .",
            "grep -f .env source.rs", "./grep alpha source.rs",
            "grep alpha *.rs", "echo $HOME", "echo -e 'x'", "grep '' source.rs",
        ] {
            assert_eq!(facts(&context,"bash",json!({"command":command,"operation_kind":"inspect"})).bindings[0].observation,
                PermissionOperationRisk::Unknown, "{command}");
        }
        assert!(!directory.path().join("created").exists());
    }

    #[cfg(unix)]
    #[test]
    fn source_search_plan_observes_external_and_symlinked_sensitive_operands() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let ordinary = outside.path().join("code.rs");
        let secret = outside.path().join(".env");
        fs::write(&ordinary,"fixture").unwrap();
        fs::write(&secret,"synthetic-secret").unwrap();
        let command = format!("grep fixture '{}' | head -5",ordinary.display());
        assert_eq!(facts(&context,"bash",json!({"command":command})).bindings[0].observation,
            PermissionOperationRisk::OrdinaryRead);
        let alias = directory.path().join("alias.rs");
        std::os::unix::fs::symlink(&secret,&alias).unwrap();
        assert_eq!(facts(&context,"bash",json!({"command":"grep secret alias.rs | head -5"})).bindings[0].observation,
            PermissionOperationRisk::SensitiveRead);
    }

    #[test]
    fn shell_labels_never_replace_supported_argv_and_resource_inspection() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        fs::write(directory.path().join("notes.txt"),"ordinary").unwrap();
        fs::write(directory.path().join(".env"),"fixture-only").unwrap();
        assert_eq!(facts(&context,"bash",json!({"command":"cat notes.txt","operation_kind":"execute"})).bindings[0].observation,
            PermissionOperationRisk::OrdinaryRead);
        for command in ["rm notes.txt","cat notes.txt; rm notes.txt","cat $(whoami)","python -c 'print(1)'","cat"] {
            assert_eq!(facts(&context,"bash",json!({"command":command,"operation_kind":"inspect"})).bindings[0].observation,
                PermissionOperationRisk::Unknown,"{command}");
        }
        assert_eq!(facts(&context,"bash",json!({"command":"cat .env","operation_kind":"inspect"})).bindings[0].observation,
            PermissionOperationRisk::SensitiveRead);
        let pinned = verified_read_only_shell_command(&context,json!({"command":"cat notes.txt"}).as_object().unwrap()).unwrap();
        assert!(pinned.contains(&crate::platform_paths::system_utility("cat").unwrap()));
        assert!(pinned.starts_with(&format!("'{}' '-i'", crate::platform_paths::system_utility("env").unwrap())));
    }
    #[cfg(unix)]
    #[test]
    fn symlink_target_changes_the_approval_resource_and_resolution_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        fs::write(directory.path().join("first"),"one").unwrap();
        fs::write(directory.path().join("second"),"two").unwrap();
        let link = directory.path().join("alias");
        std::os::unix::fs::symlink(directory.path().join("first"),&link).unwrap();
        let first = facts(&context,"read_file",json!({"path":"alias"}));
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(directory.path().join("second"),&link).unwrap();
        let second = facts(&context,"read_file",json!({"path":"alias"}));
        assert_ne!(first.bindings[0].resource,second.bindings[0].resource);
        assert_ne!(first.bindings[0].evidence_id,second.bindings[0].evidence_id);
    }
}

pub(super) fn validate_bound_file_workspace(request:&ToolRequest,ctx:&ToolContext)->Result<(),String> {
    let Some(root)=ctx.policy.bound_file_workspace() else {return Ok(());};
    if tool_names_are_read_only(&[request.name.clone()]) {return Ok(());}
    if !tool_has_bounded_file_writes(&request.name) {
        return Err("This task permits only its declared bounded file edits; commands, plugins and other mutations are unavailable".into());
    }
    let input=Value::Object(request.args.clone());
    let plan=builtin_permission_contract(&request.name,false).resource_plan_for(&input)
        .ok_or("Bounded file tool has no resource plan")?;
    for demand in plan.extract(&input).map_err(|e|format!("Invalid bounded file request: {e:?}"))? {
        if demand.locator_kind!=ResourceLocatorKind::FilePath {return Err("Unbounded resource in file-only task".into());}
        if matches!(demand.access.as_str(),"write"|"overwrite"|"remove") {
            let path = resolved_path(&demand.locator,ctx)?;
            if !path.starts_with(root) {
                return Err("File-only worktree task cannot write outside its frozen execution root".into());
            }
            if path.starts_with(root.join(".git")) {
                return Err("File-only worktree task cannot change its Git binding or metadata".into());
            }
        }
    }
    Ok(())
}
