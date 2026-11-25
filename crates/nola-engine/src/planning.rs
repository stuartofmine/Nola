// SPDX-License-Identifier: Apache-2.0
//! Session planning, structured todos, and interactive user input (`ToolTier::InteractiveOnly`).

use std::sync::{Arc, Mutex};

use nola_store::Store;
use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    now_ms, CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Callback invoked when `request_user_input` runs in an interactive Host (TUI/Web).
pub type UserInputCallback = Arc<dyn Fn(&str, &[String]) -> Option<String> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Default)]
pub struct PlanningState {
    pub todos: Mutex<Vec<TodoItem>>,
    pub latest_plan: Mutex<Option<String>>,
    pub user_input: Mutex<Option<UserInputCallback>>,
}

impl PlanningState {
    pub fn set_user_input_callback(&self, cb: Option<UserInputCallback>) {
        *self.user_input.lock().unwrap() = cb;
    }
}

pub fn register_planning_tools(
    store: Store,
    state: &Arc<PlanningState>,
    registry: &mut ToolRegistry,
) {
    registry.register(Arc::new(UpdateTodosTool {
        store: store.clone(),
        state: Arc::clone(state),
    }));
    registry.register(Arc::new(ProposePlanTool {
        store,
        state: Arc::clone(state),
    }));
    registry.register(Arc::new(RequestUserInputTool {
        state: Arc::clone(state),
    }));
}

/// Load the persisted structured todos for a specific session.
pub fn load_session_todos(store: &Store, session_id: &str) -> Value {
    let key = format!("todos:{session_id}");
    store
        .get_kv(&key)
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({"session_id": session_id, "todos": []}))
}

/// Load the persisted proposed plan for a specific session.
pub fn load_session_plan(store: &Store, session_id: &str) -> Value {
    let key = format!("plan:{session_id}");
    store
        .get_kv(&key)
        .ok()
        .flatten()
        .unwrap_or(Value::Null)
}

struct UpdateTodosTool {
    store: Store,
    state: Arc<PlanningState>,
}

impl ToolHandler for UpdateTodosTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "update_agent_todos".into(),
                description: "Maintain a structured live todo checklist for complex multi-step work.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "todos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "title": { "type": "string" },
                                    "status": { "type": "string", "enum": ["pending", "in_progress", "done", "blocked"] }
                                },
                                "required": ["title", "status"]
                            }
                        }
                    },
                    "required": ["todos"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(arr) = args.get("todos").and_then(|v| v.as_array()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'todos' array", None);
        };
        let items: Vec<TodoItem> = arr
            .iter()
            .enumerate()
            .map(|(i, v)| TodoItem {
                id: v
                    .get("id")
                    .and_then(|s| s.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("t{}", i + 1)),
                title: v
                    .get("title")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                status: v
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("pending")
                    .to_string(),
            })
            .collect();
        *self.state.todos.lock().unwrap() = items.clone();
        let val = json!({
            "session_id": ctx.session_id,
            "updated_at_ms": now_ms(),
            "todos": items,
        });
        let key = format!("todos:{}", ctx.session_id);
        let _ = self.store.set_kv(&key, &val);
        ToolExecutionResult::ok_structured(val, format!("Updated {} todo item(s)", items.len()))
    }
}

struct ProposePlanTool {
    store: Store,
    state: Arc<PlanningState>,
}

impl ToolHandler for ProposePlanTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "propose_agent_plan".into(),
                description: "Record and present a structured implementation plan before executing risky or broad changes.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "markdown": { "type": "string" }
                    },
                    "required": ["title", "markdown"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("Plan");
        let md = args.get("markdown").and_then(|v| v.as_str()).unwrap_or("");
        *self.state.latest_plan.lock().unwrap() = Some(md.to_string());
        let val = json!({
            "session_id": ctx.session_id,
            "title": title,
            "markdown": md,
            "updated_at_ms": now_ms(),
        });
        let key = format!("plan:{}", ctx.session_id);
        let _ = self.store.set_kv(&key, &val);
        ToolExecutionResult::ok_structured(val, format!("Proposed plan: {title}"))
    }
}

struct RequestUserInputTool {
    state: Arc<PlanningState>,
}

impl ToolHandler for RequestUserInputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "request_user_input".into(),
                description: "Ask the user a clarifying question in an interactive session (hidden automatically in headless/batch runs).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "question": { "type": "string" },
                        "options": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["question"]
                }),
            },
            tier: ToolTier::InteractiveOnly,
            domain: CapabilityDomain::InteractiveUi,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(question) = args.get("question").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'question'", None);
        };
        let options: Vec<String> = args
            .get("options")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let cb = self.state.user_input.lock().unwrap().clone();
        match cb.and_then(|f| f(question, &options)) {
            Some(answer) => ToolExecutionResult::ok_structured(
                json!({"question": question, "answer": answer}),
                "Received user input",
            ),
            None => ToolExecutionResult::err_structured(
                "interactive_unavailable",
                "no interactive user input channel is attached; proceed autonomously with best judgment",
                None,
            ),
        }
    }
}
