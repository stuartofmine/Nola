// SPDX-License-Identifier: Apache-2.0
//! Pure-v2 Authoritative A2UI (Generative UI) Projection Engine & Tools.
//!
//! Produces exact `UiSnapshot` / `UiNodeSnapshot` / `UiDraftPreview` / `UiChangeSet`
//! schemas consumed by Web (`vendor/a2ui-svelte`), Android (`GenerativeUiSurface.kt`),
//! and Apple (`GenerativeUIProjection.swift`), with:
//! - Pure-function expression & `CheckRule` evaluator (shared 10,000-op budget)
//! - Dynamic `List` template expansion (`InvalidListIdentity` / `ImmutableListIdentity`)
//! - Component graph cycle detection & max depth (32)
//! - `sendDataModel` authoritative `a2uiClientDataModel` payload support
//! - Incremental `UiChangeSet` diff vs full snapshot size selection
//! - A2UI v0.9.1 message envelope ingestion (`createSurface`, `updateComponents`,
//!   `updateDataModel`, `deleteSurface`)

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use nola_store::Store;
use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::ModulesSuite;

pub const KIND_UI_SURFACE: &str = "ui_surface";
pub const DEFAULT_CATALOG_ID: &str = "nola.a2ui.basic.v1";

pub const MAX_ACTIVE_SURFACES_PER_CONVERSATION: usize = 8;
pub const MAX_NODES_PER_SURFACE: usize = 1_000;
pub const MAX_SURFACE_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_SUMMARY_BYTES: usize = 4_096;
pub const MAX_INTERACTION_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_COMPONENT_DEPTH: usize = 32;
pub const MAX_EVAL_OPS: usize = 10_000;
const MAX_STORED_REVISIONS: usize = 20;

const INTERNAL_TEMPLATE_PROP: &str = "__a2ui_template";
const INTERNAL_CHECKS_PROP: &str = "__a2ui_checks";
const INTERNAL_ACTION_EVENT_PROP: &str = "__a2ui_action_event";

const SUPPORTED_COMPONENTS: &[&str] = &[
    "Text",
    "Row",
    "Column",
    "List",
    "Card",
    "Tabs",
    "Modal",
    "Divider",
    "Button",
    "TextField",
    "CheckBox",
    "ChoicePicker",
    "Slider",
    "DateTimeInput",
    "Icon",
];

pub const SUPPORTED_PURE_FUNCTIONS: &[&str] = &[
    "required",
    "regex",
    "length",
    "numeric",
    "email",
    "and",
    "or",
    "not",
    "formatString",
    "formatNumber",
    "formatCurrency",
    "formatDate",
    "pluralize",
];

pub fn catalog_digest() -> String {
    sha256_hex(format!("{DEFAULT_CATALOG_ID}:{}", SUPPORTED_COMPONENTS.join(",")).as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceOwner {
    pub authority_id: String,
    pub conversation_id: String,
    pub participant_id: String,
    pub context_id: String,
    pub originating_run_id: String,
    pub message_id: String,
    pub part_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaceIdentity {
    pub owner: UiSurfaceOwner,
    pub surface_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiActionRef {
    pub instance_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckRule {
    pub condition: Value,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNodeSnapshot {
    pub instance_id: String,
    pub component_id: String,
    pub component: String,
    pub props: Map<String, Value>,
    pub slots: BTreeMap<String, Vec<String>>,
    pub bindings: BTreeMap<String, String>,
    pub actions: BTreeMap<String, UiActionRef>,
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
    pub catalog_id: String,
    pub catalog_digest: String,
    pub phase: String,
    pub root_instance_id: Option<String>,
    pub nodes: BTreeMap<String, UiNodeSnapshot>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiChangeSet {
    pub identity: UiSurfaceIdentity,
    pub base_revision: u64,
    pub revision: u64,
    pub catalog_digest: String,
    pub phase: String,
    pub root_instance_id: Option<String>,
    pub upserted_nodes: Vec<UiNodeSnapshot>,
    pub removed_instance_ids: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEdit {
    Set {
        instance_id: String,
        binding: String,
        value: Value,
    },
    Delete {
        instance_id: String,
        binding: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiInteraction {
    pub identity: UiSurfaceIdentity,
    pub expected_surface_revision: u64,
    #[serde(default)]
    pub edits: Vec<UiEdit>,
    pub action: Option<UiActionRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDraftPreviewInput {
    pub identity: UiSurfaceIdentity,
    pub expected_surface_revision: u64,
    #[serde(default)]
    pub edits: Vec<UiEdit>,
    pub draft_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDraftPreview {
    pub identity: UiSurfaceIdentity,
    pub base_revision: u64,
    pub draft_generation: u64,
    pub snapshot: UiSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSurfaceRevisionRef {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredUiSurface {
    pub storage_key: String,
    pub conversation_id: String,
    pub surface_id: String,
    pub current: UiSnapshot,
    pub data_model: Value,
    pub revisions: BTreeMap<u64, UiSnapshot>,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub base_nodes: BTreeMap<String, UiNodeSnapshot>,
    #[serde(default)]
    pub base_root_id: Option<String>,
    #[serde(default)]
    pub send_data_model: bool,
    #[serde(default)]
    pub live_revisions: BTreeMap<u64, UiSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommittedInteraction {
    pub snapshot: UiSnapshot,
    pub data_model: Value,
    pub action_id: Option<String>,
    pub summary: String,
    #[serde(
        rename = "a2uiClientDataModel",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub a2ui_client_data_model: Option<Value>,
}

pub fn surface_storage_key(conversation_id: &str, surface_id: &str) -> String {
    format!("{conversation_id}::{surface_id}")
}

fn absolute_pointer(scope: &str, path: &str) -> String {
    if path.is_empty() || path == "/" {
        return if scope.is_empty() {
            "/".to_string()
        } else {
            scope.to_string()
        };
    }
    if path.starts_with('/') {
        path.to_string()
    } else if scope.is_empty() || scope == "/" {
        format!("/{path}")
    } else {
        format!("{}/{path}", scope.trim_end_matches('/'))
    }
}

fn pointer_segments(pointer: &str) -> Vec<String> {
    let trimmed = pointer.trim_start_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split('/')
        .map(|seg| seg.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn overlapping_paths(a: &str, b: &str) -> bool {
    let sa = pointer_segments(a);
    let sb = pointer_segments(b);
    let min_len = sa.len().min(sb.len());
    sa[..min_len] == sb[..min_len]
}

fn json_pointer_get<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() || pointer == "/" {
        return Some(root);
    }
    let normalized = if pointer.starts_with('/') {
        pointer.to_string()
    } else {
        format!("/{pointer}")
    };
    root.pointer(&normalized)
}

fn json_pointer_set(root: &mut Value, pointer: &str, val: Value) {
    let segments = pointer_segments(pointer);
    if segments.is_empty() {
        *root = val;
        return;
    }
    let mut cursor = root;
    for seg in &segments[..segments.len() - 1] {
        let arr_idx = seg
            .parse::<usize>()
            .ok()
            .filter(|&idx| cursor.as_array().is_some_and(|arr| idx < arr.len()));
        if let Some(idx) = arr_idx {
            cursor = &mut cursor.as_array_mut().unwrap()[idx];
            continue;
        }
        if !cursor.is_object() {
            *cursor = json!({});
        }
        let obj = cursor.as_object_mut().unwrap();
        cursor = obj.entry(seg.clone()).or_insert_with(|| json!({}));
    }
    let last_key = segments.last().unwrap();
    if let Ok(idx) = last_key.parse::<usize>() {
        if let Some(arr) = cursor.as_array_mut() {
            if idx < arr.len() {
                arr[idx] = val;
                return;
            }
        }
    }
    if !cursor.is_object() {
        *cursor = json!({});
    }
    cursor.as_object_mut().unwrap().insert(last_key.clone(), val);
}

fn json_pointer_delete(root: &mut Value, pointer: &str) {
    let segments = pointer_segments(pointer);
    if segments.is_empty() {
        return;
    }
    let mut cursor = root;
    for seg in &segments[..segments.len() - 1] {
        if let Ok(idx) = seg.parse::<usize>() {
            if let Some(arr_len) = cursor.as_array().map(|a| a.len()) {
                if idx < arr_len {
                    cursor = &mut cursor.as_array_mut().unwrap()[idx];
                    continue;
                }
                return;
            }
        }
        let Some(next) = cursor.as_object_mut().and_then(|o| o.get_mut(seg)) else {
            return;
        };
        cursor = next;
    }
    let last_key = segments.last().unwrap();
    if let Ok(idx) = last_key.parse::<usize>() {
        if let Some(arr) = cursor.as_array_mut() {
            if idx < arr.len() {
                arr.remove(idx);
                return;
            }
        }
    }
    if let Some(obj) = cursor.as_object_mut() {
        obj.remove(last_key);
    }
}

/// Pure-function expression & check evaluator with a shared 10,000-operation budget.
pub struct PureEvaluator<'a> {
    data: &'a Value,
    remaining: Rc<Cell<usize>>,
}

impl<'a> PureEvaluator<'a> {
    pub fn new(data: &'a Value) -> Self {
        Self::with_budget(data, Rc::new(Cell::new(MAX_EVAL_OPS)))
    }

    pub fn with_budget(data: &'a Value, remaining: Rc<Cell<usize>>) -> Self {
        Self { data, remaining }
    }

    pub fn resolve(&mut self, value: &Value, scope: &str) -> Result<Value, String> {
        self.resolve_at(value, scope, 0)
    }

    fn consume_op(&self) -> Result<(), String> {
        let left = self.remaining.get();
        if left == 0 {
            return Err(format!(
                "expression evaluation exceeded {MAX_EVAL_OPS}-operation budget"
            ));
        }
        self.remaining.set(left - 1);
        Ok(())
    }

    fn resolve_at(&mut self, value: &Value, scope: &str, depth: usize) -> Result<Value, String> {
        self.consume_op()?;
        if depth > MAX_COMPONENT_DEPTH {
            return Err(format!(
                "expression evaluation exceeded max depth of {MAX_COMPONENT_DEPTH}"
            ));
        }
        match value {
            Value::Object(obj) if obj.len() == 1 && obj.contains_key("path") => {
                let raw_path = obj["path"]
                    .as_str()
                    .ok_or_else(|| "binding path must be a string".to_string())?;
                let abs = absolute_pointer(scope, raw_path);
                Ok(json_pointer_get(self.data, &abs)
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            Value::Object(obj) if obj.contains_key("call") => {
                let name = obj["call"]
                    .as_str()
                    .ok_or_else(|| "'call' function name must be a string".to_string())?;
                if !SUPPORTED_PURE_FUNCTIONS.contains(&name) {
                    return Err(format!("unsupported pure function '{name}'"));
                }
                let raw_args = obj
                    .get("args")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let mut resolved_args = Map::new();
                for (k, v) in &raw_args {
                    resolved_args.insert(k.clone(), self.resolve_at(v, scope, depth + 1)?);
                }
                evaluate_pure_function(name, &resolved_args, self.data, scope)
            }
            Value::Object(obj) => {
                let mut out = Map::new();
                for (k, v) in obj {
                    out.insert(k.clone(), self.resolve_at(v, scope, depth + 1)?);
                }
                Ok(Value::Object(out))
            }
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_at(item, scope, depth + 1)?);
                }
                Ok(Value::Array(out))
            }
            _ => Ok(value.clone()),
        }
    }
}

fn bounded_regex(pattern: &str) -> Result<regex::Regex, String> {
    if pattern.len() > 4096 {
        return Err("regex pattern exceeds 4096-byte limit".into());
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(256 * 1024)
        .build()
        .map_err(|e| format!("invalid regex pattern: {e}"))
}

fn is_valid_email(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.len() > 254 || trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = trimmed.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
    {
        return false;
    }
    if domain.len() < 3
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return false;
    }
    domain.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn value_to_display_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn format_number_with_grouping(num: f64, decimals: Option<usize>, grouping: bool) -> String {
    let raw = match decimals {
        Some(d) => format!("{num:.*}", d.min(12)),
        None => {
            if num.fract() == 0.0 && num.abs() < 1e15 {
                format!("{:.0}", num)
            } else {
                format!("{num}")
            }
        }
    };
    if !grouping {
        return raw;
    }
    let (neg, rest) = if let Some(stripped) = raw.strip_prefix('-') {
        ("-", stripped)
    } else {
        ("", raw.as_str())
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3);
    for (idx, ch) in int_part.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let grouped_int: String = grouped.chars().rev().collect();
    match frac_part {
        Some(f) => format!("{neg}{grouped_int}.{f}"),
        None => format!("{neg}{grouped_int}"),
    }
}

fn evaluate_pure_function(
    name: &str,
    args: &Map<String, Value>,
    data: &Value,
    scope: &str,
) -> Result<Value, String> {
    let value = args.get("value").unwrap_or(&Value::Null);
    match name {
        "required" => {
            let ok = match value {
                Value::Null => false,
                Value::String(s) => !s.trim().is_empty(),
                Value::Array(a) => !a.is_empty(),
                Value::Bool(b) => *b,
                _ => true,
            };
            Ok(Value::Bool(ok))
        }
        "regex" => {
            let pattern = args
                .get("pattern")
                .and_then(Value::as_str)
                .ok_or_else(|| "regex() requires string 'pattern'".to_string())?;
            let re = bounded_regex(pattern)?;
            Ok(Value::Bool(value.as_str().is_some_and(|s| re.is_match(s))))
        }
        "length" => {
            let len = match value {
                Value::String(s) => s.chars().count() as u64,
                Value::Array(a) => a.len() as u64,
                _ => return Ok(Value::Bool(false)),
            };
            let min_ok = args
                .get("min")
                .and_then(Value::as_u64)
                .is_none_or(|min| len >= min);
            let max_ok = args
                .get("max")
                .and_then(Value::as_u64)
                .is_none_or(|max| len <= max);
            Ok(Value::Bool(min_ok && max_ok))
        }
        "numeric" => {
            let num = match value {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.trim().parse::<f64>().ok(),
                _ => None,
            };
            let Some(n) = num.filter(|v| v.is_finite()) else {
                return Ok(Value::Bool(false));
            };
            let min_ok = args
                .get("min")
                .and_then(Value::as_f64)
                .is_none_or(|min| n >= min);
            let max_ok = args
                .get("max")
                .and_then(Value::as_f64)
                .is_none_or(|max| n <= max);
            Ok(Value::Bool(min_ok && max_ok))
        }
        "email" => Ok(Value::Bool(
            value.as_str().is_some_and(is_valid_email),
        )),
        "and" | "or" => {
            let vals = args
                .get("values")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{name}() requires boolean array 'values'"))?;
            let mut bools = Vec::with_capacity(vals.len());
            for item in vals {
                bools.push(
                    item.as_bool()
                        .ok_or_else(|| format!("{name}() requires boolean elements in 'values'"))?,
                );
            }
            let res = if name == "and" {
                bools.into_iter().all(|b| b)
            } else {
                bools.into_iter().any(|b| b)
            };
            Ok(Value::Bool(res))
        }
        "not" => {
            let b = value
                .as_bool()
                .ok_or_else(|| "not() requires boolean 'value'".to_string())?;
            Ok(Value::Bool(!b))
        }
        "formatString" => {
            let template = args
                .get("template")
                .or_else(|| args.get("value"))
                .or_else(|| args.get("format"))
                .and_then(Value::as_str)
                .ok_or_else(|| "formatString() requires string 'template' or 'value'".to_string())?;
            let mut out = template.to_string();

            // Replace `${path}` expressions from data_model (A2UI v0.9.1 string interpolation)
            let interp_re = bounded_regex(r"\$\{([^}]+)\}")?;
            let mut replaced = String::with_capacity(out.len());
            let mut last_end = 0;
            for caps in interp_re.captures_iter(&out) {
                let m = caps.get(0).unwrap();
                replaced.push_str(&out[last_end..m.start()]);
                let expr = caps[1].trim();
                let replacement = if let Some(arg_val) = args.get(expr) {
                    value_to_display_string(arg_val)
                } else {
                    let abs = absolute_pointer(scope, expr);
                    json_pointer_get(data, &abs)
                        .map(value_to_display_string)
                        .unwrap_or_default()
                };
                replaced.push_str(&replacement);
                last_end = m.end();
            }
            replaced.push_str(&out[last_end..]);
            out = replaced;

            // Replace `{key}` from args.params / args.values / top-level args
            if let Some(params) = args.get("params").and_then(Value::as_object) {
                for (k, v) in params {
                    out = out.replace(&format!("{{{k}}}"), &value_to_display_string(v));
                }
            }
            if let Some(vals) = args.get("values").and_then(Value::as_array) {
                for (idx, v) in vals.iter().enumerate() {
                    out = out.replace(&format!("{{{idx}}}"), &value_to_display_string(v));
                }
            }
            for (k, v) in args {
                if !matches!(k.as_str(), "template" | "value" | "format" | "params" | "values") {
                    out = out.replace(&format!("{{{k}}}"), &value_to_display_string(v));
                }
            }
            Ok(Value::String(out))
        }
        "formatNumber" => {
            let num = value
                .as_f64()
                .ok_or_else(|| "formatNumber() requires numeric 'value'".to_string())?;
            if !num.is_finite() {
                return Err("formatNumber() requires finite 'value'".into());
            }
            let decimals = args
                .get("decimals")
                .or_else(|| args.get("maximumFractionDigits"))
                .and_then(Value::as_u64)
                .map(|d| d as usize);
            let grouping = args
                .get("grouping")
                .or_else(|| args.get("useGrouping"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            Ok(Value::String(format_number_with_grouping(
                num, decimals, grouping,
            )))
        }
        "formatCurrency" => {
            let num = value
                .as_f64()
                .ok_or_else(|| "formatCurrency() requires numeric 'value'".to_string())?;
            if !num.is_finite() {
                return Err("formatCurrency() requires finite 'value'".into());
            }
            let code = args
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or("USD")
                .trim()
                .to_ascii_uppercase();
            let default_decimals = if code == "JPY" || code == "KRW" { 0 } else { 2 };
            let decimals = args
                .get("decimals")
                .and_then(Value::as_u64)
                .map(|d| d as usize)
                .unwrap_or(default_decimals);
            let symbol = match code.as_str() {
                "USD" => "$",
                "EUR" => "€",
                "GBP" => "£",
                "CNY" | "JPY" => "¥",
                other => other,
            };
            let abs_formatted = format_number_with_grouping(num.abs(), Some(decimals), true);
            let sign = if num < 0.0 { "-" } else { "" };
            let sep = if symbol.len() > 1 && symbol.is_ascii() {
                " "
            } else {
                ""
            };
            Ok(Value::String(format!("{sign}{symbol}{sep}{abs_formatted}")))
        }
        "formatDate" => {
            let dt: DateTime<Utc> = match value {
                Value::String(s) => {
                    let trimmed = s.trim();
                    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
                        parsed.with_timezone(&Utc)
                    } else if let Ok(ndt) =
                        NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S")
                    {
                        ndt.and_utc()
                    } else if let Ok(nd) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
                        nd.and_hms_opt(0, 0, 0)
                            .ok_or_else(|| "invalid date".to_string())?
                            .and_utc()
                    } else {
                        return Err(format!("formatDate() cannot parse '{trimmed}'"));
                    }
                }
                Value::Number(n) => {
                    let raw = n
                        .as_i64()
                        .ok_or_else(|| "formatDate() requires integer timestamp".to_string())?;
                    if raw.abs() > 10_000_000_000 {
                        DateTime::<Utc>::from_timestamp_millis(raw)
                            .ok_or_else(|| "formatDate() timestamp out of range".to_string())?
                    } else {
                        DateTime::<Utc>::from_timestamp(raw, 0)
                            .ok_or_else(|| "formatDate() timestamp out of range".to_string())?
                    }
                }
                _ => return Err("formatDate() requires ISO date string or unix timestamp".into()),
            };
            let fmt = args
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("medium");
            let formatted = match fmt {
                "iso" => dt.to_rfc3339(),
                "short" => dt.format("%Y-%m-%d").to_string(),
                "medium" => dt.format("%b %d, %Y").to_string(),
                "long" => dt.format("%B %d, %Y %H:%M UTC").to_string(),
                custom => dt.format(custom).to_string(),
            };
            Ok(Value::String(formatted))
        }
        "pluralize" => {
            let count = args
                .get("count")
                .or_else(|| args.get("value"))
                .and_then(Value::as_f64)
                .ok_or_else(|| "pluralize() requires numeric 'count' or 'value'".to_string())?;
            let one = args
                .get("one")
                .and_then(Value::as_str)
                .ok_or_else(|| "pluralize() requires string 'one'".to_string())?;
            let other = args
                .get("other")
                .and_then(Value::as_str)
                .ok_or_else(|| "pluralize() requires string 'other'".to_string())?;
            let chosen = if count == 0.0 {
                args.get("zero").and_then(Value::as_str).unwrap_or(other)
            } else if (count - 1.0).abs() < f64::EPSILON {
                one
            } else {
                other
            };
            let count_str = if count.fract() == 0.0 {
                format!("{}", count as i64)
            } else {
                count.to_string()
            };
            Ok(Value::String(
                chosen
                    .replace("{count}", &count_str)
                    .replace('#', &count_str),
            ))
        }
        _ => Err(format!("unsupported pure function '{name}'")),
    }
}

/// Evaluate bindings from `data_model` onto every node and compute authoritative validation errors.
/// Preserves infallible v2 signature; for full dynamic List template expansion and error propagation,
/// use `project_surface_nodes`.
pub fn project_nodes(
    base_nodes: &BTreeMap<String, UiNodeSnapshot>,
    data_model: &Value,
) -> BTreeMap<String, UiNodeSnapshot> {
    let budget = Rc::new(Cell::new(MAX_EVAL_OPS));
    let mut out = BTreeMap::new();
    for (id, node) in base_nodes {
        if let Ok(projected) = project_single_node(node, data_model, "", &budget, false) {
            out.insert(id.clone(), projected);
        }
    }
    out
}

fn project_single_node(
    node: &UiNodeSnapshot,
    data_model: &Value,
    scope: &str,
    budget: &Rc<Cell<usize>>,
    read_only: bool,
) -> Result<UiNodeSnapshot, String> {
    let mut projected = node.clone();
    let mut evaluator = PureEvaluator::with_budget(data_model, Rc::clone(budget));

    // Extract internal metadata before resolving props
    let checks_val = projected.props.remove(INTERNAL_CHECKS_PROP);
    projected.props.remove(INTERNAL_TEMPLATE_PROP);
    projected.props.remove(INTERNAL_ACTION_EVENT_PROP);

    // Resolve dynamic expressions / path bindings in props
    let prop_keys: Vec<String> = projected.props.keys().cloned().collect();
    for key in prop_keys {
        if let Some(raw_val) = projected.props.get(&key).cloned() {
            if is_dynamic_expr(&raw_val) {
                let resolved = evaluator.resolve(&raw_val, scope)?;
                projected.props.insert(key, resolved);
            }
        }
    }

    // Rewrite relative bindings to absolute paths and project bound values
    let mut abs_bindings = BTreeMap::new();
    for (binding_name, pointer) in &projected.bindings {
        let abs = absolute_pointer(scope, pointer);
        if let Some(bound_val) = json_pointer_get(data_model, &abs) {
            projected
                .props
                .insert(binding_name.clone(), bound_val.clone());
        }
        abs_bindings.insert(binding_name.clone(), abs);
    }
    projected.bindings = abs_bindings;

    // Built-in prop-based validation
    let mut errors = validate_node(&projected)?;

    // CheckRule validation arrays
    if let Some(raw_checks) = checks_val {
        let rules: Vec<CheckRule> = serde_json::from_value(raw_checks)
            .map_err(|e| format!("invalid CheckRule array on '{}': {e}", node.component_id))?;
        for rule in rules {
            let cond_val = evaluator.resolve(&rule.condition, scope)?;
            if cond_val.as_bool() != Some(true) {
                errors.push(rule.message);
            }
        }
    }
    projected.validation_errors = errors;

    if read_only {
        projected.props.insert("disabled".into(), Value::Bool(true));
        projected.actions.clear();
        projected.bindings.clear();
    }

    Ok(projected)
}

fn is_dynamic_expr(val: &Value) -> bool {
    match val {
        Value::Object(obj) => {
            (obj.len() == 1 && obj.contains_key("path"))
                || obj.contains_key("call")
                || obj.values().any(is_dynamic_expr)
        }
        Value::Array(arr) => arr.iter().any(is_dynamic_expr),
        _ => false,
    }
}

fn validate_node(node: &UiNodeSnapshot) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    let val = node.props.get("value");
    let required = node
        .props
        .get("required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let label = node
        .props
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or(&node.component_id);

    if required {
        let empty = match val {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            Some(Value::Array(a)) => a.is_empty(),
            Some(Value::Bool(b)) => !*b,
            _ => false,
        };
        if empty {
            errors.push(format!("{label} is required"));
        }
    }

    if let Some(Value::String(s)) = val {
        if let Some(min_len) = node.props.get("minLength").and_then(|v| v.as_u64()) {
            if !s.is_empty() && (s.chars().count() as u64) < min_len {
                errors.push(format!("{label} must be at least {min_len} characters"));
            }
        }
        if let Some(max_len) = node.props.get("maxLength").and_then(|v| v.as_u64()) {
            if (s.chars().count() as u64) > max_len {
                errors.push(format!("{label} must be at most {max_len} characters"));
            }
        }
        if node.component == "TextField" {
            if let Some(pattern) = node.props.get("validationRegexp").and_then(Value::as_str) {
                let re = bounded_regex(pattern)?;
                if !re.is_match(s) {
                    errors.push(format!("{label} does not match required format"));
                }
            }
        }
    }

    if let Some(num) = val.and_then(|v| v.as_f64()) {
        if let Some(min_v) = node.props.get("min").and_then(|v| v.as_f64()) {
            if num < min_v {
                errors.push(format!("{label} must be at least {min_v}"));
            }
        }
        if let Some(max_v) = node.props.get("max").and_then(|v| v.as_f64()) {
            if num > max_v {
                errors.push(format!("{label} must be at most {max_v}"));
            }
        }
    }

    Ok(errors)
}

fn node_child_ids(node: &UiNodeSnapshot) -> Vec<String> {
    let mut ids = Vec::new();
    for slot_ids in node.slots.values() {
        ids.extend(slot_ids.iter().cloned());
    }
    if let Some(tpl) = node
        .props
        .get(INTERNAL_TEMPLATE_PROP)
        .and_then(|v| v.get("componentId"))
        .and_then(Value::as_str)
    {
        ids.push(tpl.to_string());
    }
    ids
}

fn validate_component_graph(nodes: &BTreeMap<String, UiNodeSnapshot>) -> Result<(), String> {
    let mut completed: BTreeMap<String, usize> = BTreeMap::new();
    let mut visiting: BTreeSet<String> = BTreeSet::new();

    fn dfs(
        id: &str,
        nodes: &BTreeMap<String, UiNodeSnapshot>,
        visiting: &mut BTreeSet<String>,
        completed: &mut BTreeMap<String, usize>,
        depth: usize,
    ) -> Result<usize, String> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(format!(
                "component graph exceeds maximum depth of {MAX_COMPONENT_DEPTH} at '{id}'"
            ));
        }
        if let Some(&height) = completed.get(id) {
            if depth + height > MAX_COMPONENT_DEPTH {
                return Err(format!(
                    "component graph exceeds maximum depth of {MAX_COMPONENT_DEPTH} at '{id}'"
                ));
            }
            return Ok(height);
        }
        let Some(node) = nodes.get(id) else {
            return Ok(0);
        };
        if !visiting.insert(id.to_string()) {
            return Err(format!("component graph contains a cycle involving '{id}'"));
        }
        let mut max_child_height = 0usize;
        for child_id in node_child_ids(node) {
            let child_h = dfs(&child_id, nodes, visiting, completed, depth + 1)?;
            max_child_height = max_child_height.max(child_h + 1);
        }
        visiting.remove(id);
        completed.insert(id.to_string(), max_child_height);
        Ok(max_child_height)
    }

    for id in nodes.keys() {
        dfs(id, nodes, &mut visiting, &mut completed, 0)?;
    }
    Ok(())
}

fn is_editable_component_node(node: &UiNodeSnapshot) -> bool {
    matches!(
        node.component.as_str(),
        "TextField" | "DateTimeInput" | "CheckBox" | "Slider" | "ChoicePicker"
    ) && node.bindings.contains_key("value")
}

fn has_editable_descendant(
    id: &str,
    base_nodes: &BTreeMap<String, UiNodeSnapshot>,
    visiting: &mut BTreeSet<String>,
) -> Result<bool, String> {
    if !visiting.insert(id.to_string()) {
        return Err(format!("component graph contains a cycle involving '{id}'"));
    }
    let Some(node) = base_nodes.get(id) else {
        visiting.remove(id);
        return Ok(false);
    };
    let mut editable = is_editable_component_node(node);
    for child in node_child_ids(node) {
        if has_editable_descendant(&child, base_nodes, visiting)? {
            editable = true;
        }
    }
    visiting.remove(id);
    Ok(editable)
}

fn template_instance_id(component_id: &str, item_scope_key: &str) -> String {
    if item_scope_key.is_empty() {
        component_id.to_string()
    } else {
        format!("{component_id}__{item_scope_key}")
    }
}

fn stable_item_key(item: &Value, index: usize) -> String {
    if let Some(id_val) = item.get("id") {
        if let Some(s) = id_val.as_str().filter(|s| !s.trim().is_empty()) {
            let clean: String = s
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                .collect();
            if !clean.is_empty() && clean == s {
                return clean;
            }
            let h = hex::encode(Sha256::digest(s.as_bytes()));
            return format!("k_{}", &h[..12]);
        }
        if let Some(n) = id_val.as_i64() {
            return n.to_string();
        }
        if let Some(n) = id_val.as_u64() {
            return n.to_string();
        }
    }
    format!("idx_{index}")
}

/// Full authoritative projection: expands dynamic `List` templates, enforces `InvalidListIdentity`
/// & `ImmutableListIdentity`, evaluates pure-function expressions and `CheckRule`s under the shared
/// 10,000-op budget, and checks node count limits.
pub fn project_surface_nodes(
    root_id: Option<&str>,
    base_nodes: &BTreeMap<String, UiNodeSnapshot>,
    data_model: &Value,
    read_only: bool,
) -> Result<(Option<String>, BTreeMap<String, UiNodeSnapshot>), String> {
    validate_component_graph(base_nodes)?;
    let has_templates = base_nodes
        .values()
        .any(|n| n.props.contains_key(INTERNAL_TEMPLATE_PROP));
    let budget = Rc::new(Cell::new(MAX_EVAL_OPS));

    if !has_templates {
        let mut out = BTreeMap::new();
        for (id, node) in base_nodes {
            let projected = project_single_node(node, data_model, "", &budget, read_only)?;
            out.insert(id.clone(), projected);
        }
        return Ok((root_id.map(ToString::to_string), out));
    }

    let mut out_nodes: BTreeMap<String, UiNodeSnapshot> = BTreeMap::new();
    let mut protected_identity_paths: BTreeSet<String> = BTreeSet::new();
    let mut template_only_ids: BTreeSet<String> = BTreeSet::new();
    for node in base_nodes.values() {
        if let Some(tpl_id) = node
            .props
            .get(INTERNAL_TEMPLATE_PROP)
            .and_then(|v| v.get("componentId"))
            .and_then(Value::as_str)
        {
            template_only_ids.insert(tpl_id.to_string());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_node(
        comp_id: &str,
        scope: &str,
        instance_key: &str,
        depth: usize,
        base_nodes: &BTreeMap<String, UiNodeSnapshot>,
        data_model: &Value,
        budget: &Rc<Cell<usize>>,
        read_only: bool,
        out_nodes: &mut BTreeMap<String, UiNodeSnapshot>,
        protected_identity_paths: &mut BTreeSet<String>,
    ) -> Result<String, String> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(format!(
                "component graph exceeds maximum depth of {MAX_COMPONENT_DEPTH}"
            ));
        }
        let inst_id = template_instance_id(comp_id, instance_key);
        if out_nodes.contains_key(&inst_id) {
            return Ok(inst_id);
        }
        if out_nodes.len() >= MAX_NODES_PER_SURFACE {
            return Err(format!(
                "UI surface exceeds max {MAX_NODES_PER_SURFACE} expanded nodes"
            ));
        }
        let base = base_nodes
            .get(comp_id)
            .ok_or_else(|| format!("referenced component '{comp_id}' not found"))?;
        let mut projected = project_single_node(base, data_model, scope, budget, read_only)?;
        projected.instance_id = inst_id.clone();
        // Rewrite action instance_ids to match expanded instance_id
        for action_ref in projected.actions.values_mut() {
            action_ref.instance_id = inst_id.clone();
        }

        // Expand static slots
        let raw_slots = base.slots.clone();
        projected.slots.clear();
        for (slot_name, child_ids) in raw_slots {
            let mut expanded_children = Vec::with_capacity(child_ids.len());
            for child_id in child_ids {
                if base_nodes.contains_key(&child_id) {
                    let expanded_id = expand_node(
                        &child_id,
                        scope,
                        instance_key,
                        depth + 1,
                        base_nodes,
                        data_model,
                        budget,
                        read_only,
                        out_nodes,
                        protected_identity_paths,
                    )?;
                    expanded_children.push(expanded_id);
                } else {
                    expanded_children.push(child_id);
                }
            }
            projected.slots.insert(slot_name, expanded_children);
        }

        // Expand dynamic List template if present
        if let Some(tpl) = base.props.get(INTERNAL_TEMPLATE_PROP) {
            let tpl_comp_id = tpl
                .get("componentId")
                .and_then(Value::as_str)
                .ok_or_else(|| "List template missing 'componentId'".to_string())?;
            let raw_list_path = tpl
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "List template missing 'path'".to_string())?;
            let abs_list_path = absolute_pointer(scope, raw_list_path);
            let collection = json_pointer_get(data_model, &abs_list_path)
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let items = collection.as_array().ok_or_else(|| {
                format!("List template path '{abs_list_path}' does not resolve to an array")
            })?;
            if items.len() > MAX_NODES_PER_SURFACE {
                return Err(format!(
                    "List at '{abs_list_path}' exceeds max {MAX_NODES_PER_SURFACE} items"
                ));
            }
            let editable = !read_only
                && has_editable_descendant(tpl_comp_id, base_nodes, &mut BTreeSet::new())?;
            let mut seen_ids = BTreeSet::new();
            let mut expanded_children = Vec::with_capacity(items.len());

            for (idx, item) in items.iter().enumerate() {
                let valid_id = item.get("id").filter(|id| {
                    id.as_str().is_some_and(|s| !s.trim().is_empty()) || id.is_number()
                });
                if editable {
                    let Some(id_val) = valid_id else {
                        return Err(format!(
                            "InvalidListIdentity: editable list '{abs_list_path}' item[{idx}] must have a non-empty string or numeric 'id'"
                        ));
                    };
                    let key_encoded = serde_json::to_string(id_val).unwrap_or_default();
                    if !seen_ids.insert(key_encoded) {
                        return Err(format!(
                            "InvalidListIdentity: editable list '{abs_list_path}' has duplicate item id {id_val}"
                        ));
                    }
                    protected_identity_paths
                        .insert(absolute_pointer(&abs_list_path, &format!("{idx}/id")));
                }
                let item_key = stable_item_key(item, idx);
                let nested_key = if instance_key.is_empty() {
                    item_key
                } else {
                    format!("{instance_key}_{item_key}")
                };
                let item_scope = absolute_pointer(&abs_list_path, &idx.to_string());
                let child_inst = expand_node(
                    tpl_comp_id,
                    &item_scope,
                    &nested_key,
                    depth + 1,
                    base_nodes,
                    data_model,
                    budget,
                    read_only,
                    out_nodes,
                    protected_identity_paths,
                )?;
                expanded_children.push(child_inst);
            }
            projected.slots.insert("children".into(), expanded_children);
        }

        out_nodes.insert(inst_id.clone(), projected);
        Ok(inst_id)
    }

    let expanded_root = if let Some(root) = root_id {
        Some(expand_node(
            root,
            "",
            "",
            0,
            base_nodes,
            data_model,
            &budget,
            read_only,
            &mut out_nodes,
            &mut protected_identity_paths,
        )?)
    } else {
        None
    };

    // Also include any non-template top-level nodes not already expanded (preserves disconnected static nodes)
    for id in base_nodes.keys() {
        if !out_nodes.contains_key(id) && !template_only_ids.contains(id) {
            let is_descendant_of_template = template_only_ids.iter().any(|tpl_id| {
                base_nodes
                    .get(tpl_id)
                    .is_some_and(|tpl| node_child_ids(tpl).contains(id))
            });
            if !is_descendant_of_template {
                expand_node(
                    id,
                    "",
                    "",
                    0,
                    base_nodes,
                    data_model,
                    &budget,
                    read_only,
                    &mut out_nodes,
                    &mut protected_identity_paths,
                )?;
            }
        }
    }

    // Enforce ImmutableListIdentity across all projected bindings
    for node in out_nodes.values() {
        for binding_path in node.bindings.values() {
            if protected_identity_paths
                .iter()
                .any(|id_path| overlapping_paths(binding_path, id_path))
            {
                return Err(format!(
                    "ImmutableListIdentity: binding '{}' on '{}' cannot mutate protected list item identity",
                    binding_path, node.instance_id
                ));
            }
        }
    }

    Ok((expanded_root, out_nodes))
}

fn apply_edits_to_model(
    nodes: &BTreeMap<String, UiNodeSnapshot>,
    data_model: &mut Value,
    edits: &[UiEdit],
) -> Result<(), String> {
    for edit in edits {
        match edit {
            UiEdit::Set {
                instance_id,
                binding,
                value,
            } => {
                let node = nodes
                    .get(instance_id)
                    .ok_or_else(|| format!("unknown UI node '{instance_id}'"))?;
                if node.props.get("disabled") == Some(&Value::Bool(true)) {
                    return Err(format!("UI node '{instance_id}' is disabled"));
                }
                let pointer = node
                    .bindings
                    .get(binding)
                    .ok_or_else(|| format!("node '{instance_id}' has no binding '{binding}'"))?;
                json_pointer_set(data_model, pointer, value.clone());
            }
            UiEdit::Delete {
                instance_id,
                binding,
            } => {
                let node = nodes
                    .get(instance_id)
                    .ok_or_else(|| format!("unknown UI node '{instance_id}'"))?;
                if node.props.get("disabled") == Some(&Value::Bool(true)) {
                    return Err(format!("UI node '{instance_id}' is disabled"));
                }
                let pointer = node
                    .bindings
                    .get(binding)
                    .ok_or_else(|| format!("node '{instance_id}' has no binding '{binding}'"))?;
                json_pointer_delete(data_model, pointer);
            }
        }
    }
    Ok(())
}

/// Convert a snapshot into a strictly immutable historical read-only snapshot (no actions/bindings).
pub fn to_read_only_snapshot(snapshot: &UiSnapshot) -> UiSnapshot {
    let mut ro = snapshot.clone();
    if ro.phase != "deleted" {
        ro.phase = "read_only".into();
    }
    for node in ro.nodes.values_mut() {
        node.actions.clear();
        node.bindings.clear();
    }
    ro
}

fn trim_revisions(revisions: &mut BTreeMap<u64, UiSnapshot>) {
    while revisions.len() > MAX_STORED_REVISIONS {
        if let Some(oldest) = revisions.keys().next().copied() {
            revisions.remove(&oldest);
        } else {
            break;
        }
    }
}

fn extract_slot_ids(val: &Value) -> Vec<String> {
    match val {
        Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| {
                v.as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_component_object(idx: usize, obj: &Map<String, Value>) -> Result<UiNodeSnapshot, String> {
    let id = obj
        .get("instance_id")
        .or_else(|| obj.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("node_{idx}"));
    let component = obj
        .get("component")
        .or_else(|| obj.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("Text")
        .to_string();
    if !SUPPORTED_COMPONENTS.contains(&component.as_str()) {
        return Err(format!(
            "unsupported A2UI component '{component}' (supported: {})",
            SUPPORTED_COMPONENTS.join(", ")
        ));
    }
    let mut props = obj
        .get("props")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for key in [
        "text",
        "label",
        "placeholder",
        "variant",
        "value",
        "options",
        "required",
        "minLength",
        "maxLength",
        "min",
        "max",
        "step",
        "steps",
        "axis",
        "displayStyle",
        "filterable",
        "name",
        "ariaLabel",
        "weight",
        "disabled",
        "tab_titles",
        "enableDate",
        "enableTime",
        "direction",
        "justify",
        "align",
        "validationRegexp",
    ] {
        if !props.contains_key(key) {
            if let Some(val) = obj.get(key) {
                props.insert(key.into(), val.clone());
            }
        }
    }
    if let Some(checks) = obj.get("checks").or_else(|| props.get("checks")).cloned() {
        if checks.is_array() {
            props.remove("checks");
            props.insert(INTERNAL_CHECKS_PROP.into(), checks);
        }
    }
    if component == "Slider" && !props.contains_key("steps") {
        if let Some(step_val) = props.get("step").or_else(|| obj.get("step")).cloned() {
            props.insert("steps".into(), step_val);
        }
    }
    if component == "DateTimeInput"
        && !props.contains_key("enableDate")
        && !props.contains_key("enableTime")
    {
        props.insert("enableDate".into(), Value::Bool(true));
    }

    let mut slots = BTreeMap::new();
    if let Some(raw_slots) = obj.get("slots").and_then(|v| v.as_object()) {
        for (slot_name, ids_val) in raw_slots {
            let ids = extract_slot_ids(ids_val);
            if !ids.is_empty() || ids_val.is_array() {
                slots.insert(slot_name.clone(), ids);
            }
        }
    }
    if let Some(child_id) = obj.get("child").and_then(|v| v.as_str()) {
        slots
            .entry("child".into())
            .or_insert_with(|| vec![child_id.to_string()]);
    }
    if component == "Modal" {
        if let Some(trig) = obj.get("trigger") {
            let ids = extract_slot_ids(trig);
            if !ids.is_empty() {
                slots.entry("trigger".into()).or_insert(ids);
            }
        }
        if let Some(cont) = obj.get("content") {
            let ids = extract_slot_ids(cont);
            if !ids.is_empty() {
                slots.entry("content".into()).or_insert(ids);
            }
        }
    }
    if component == "Tabs" {
        if let Some(tabs) = obj.get("tabs").and_then(Value::as_array) {
            let mut titles = Vec::with_capacity(tabs.len());
            for (tab_idx, tab) in tabs.iter().enumerate() {
                if let Some(title) = tab.get("title") {
                    titles.push(title.clone());
                }
                if let Some(child_id) = tab.get("child").and_then(Value::as_str) {
                    slots
                        .entry(format!("tab:{tab_idx}"))
                        .or_insert_with(|| vec![child_id.to_string()]);
                }
            }
            if !titles.is_empty() && !props.contains_key("tab_titles") {
                props.insert("tab_titles".into(), Value::Array(titles));
            }
        }
    }
    if let Some(children_val) = obj.get("children") {
        if let Some(children) = children_val.as_array() {
            if component == "Tabs" {
                for (tab_idx, child_val) in children.iter().enumerate() {
                    let ids = extract_slot_ids(child_val);
                    if !ids.is_empty() {
                        slots.entry(format!("tab:{tab_idx}")).or_insert(ids);
                    }
                }
            }
            slots.entry("children".into()).or_insert_with(|| {
                children
                    .iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            });
        } else if let Some(tpl_obj) = children_val.as_object() {
            if tpl_obj.get("componentId").and_then(Value::as_str).is_some()
                && tpl_obj.get("path").and_then(Value::as_str).is_some()
            {
                props.insert(INTERNAL_TEMPLATE_PROP.into(), children_val.clone());
            } else {
                return Err(format!(
                    "component '{id}' template children requires string 'componentId' and 'path'"
                ));
            }
        }
    }

    let mut bindings = BTreeMap::new();
    if let Some(raw_bindings) = obj.get("bindings").and_then(|v| v.as_object()) {
        for (b_name, ptr) in raw_bindings {
            if let Some(s) = ptr.as_str() {
                bindings.insert(b_name.clone(), s.to_string());
            }
        }
    }
    if let Some(bind_path) = obj.get("bind").and_then(|v| v.as_str()) {
        bindings
            .entry("value".into())
            .or_insert_with(|| bind_path.to_string());
    }
    // Support v0.9.1 `value: {"path": "/..."}` two-way binding declaration
    if let Some(val_obj) = props.get("value").and_then(Value::as_object) {
        if val_obj.len() == 1 {
            if let Some(path) = val_obj.get("path").and_then(Value::as_str) {
                bindings
                    .entry("value".into())
                    .or_insert_with(|| path.to_string());
            }
        }
    }

    let mut actions = BTreeMap::new();
    if let Some(raw_actions) = obj.get("actions").and_then(|v| v.as_object()) {
        for (a_name, a_val) in raw_actions {
            if let Ok(a_ref) = serde_json::from_value::<UiActionRef>(a_val.clone()) {
                actions.insert(a_name.clone(), a_ref);
            } else if let Some(action_id) = a_val.as_str() {
                actions.insert(
                    a_name.clone(),
                    UiActionRef {
                        instance_id: id.clone(),
                        action_id: action_id.to_string(),
                    },
                );
            }
        }
    }
    if let Some(action_id) = obj
        .get("action_id")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
    {
        actions.entry("action".into()).or_insert(UiActionRef {
            instance_id: id.clone(),
            action_id: action_id.to_string(),
        });
    } else if let Some(action_obj) = obj.get("action").and_then(Value::as_object) {
        // v0.9.1 `action: {"event": {"name": "...", "context": {...}}}`
        let action_name = action_obj
            .get("event")
            .and_then(|e| e.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("action");
        actions.entry("action".into()).or_insert(UiActionRef {
            instance_id: id.clone(),
            action_id: action_name.to_string(),
        });
        props.insert(INTERNAL_ACTION_EVENT_PROP.into(), Value::Object(action_obj.clone()));
    }

    Ok(UiNodeSnapshot {
        instance_id: id.clone(),
        component_id: obj
            .get("component_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string(),
        component,
        props,
        slots,
        bindings,
        actions,
        validation_errors: Vec::new(),
    })
}

/// Parse LLM tool arguments (`components` array, `nodes` map, or v0.9.1 `updateComponents` envelope)
/// into normalized `UiNodeSnapshot`s and validate the component graph for cycles and max depth (32).
pub fn build_nodes_from_spec(
    raw: &Value,
) -> Result<(Option<String>, BTreeMap<String, UiNodeSnapshot>), String> {
    let effective = raw.get("updateComponents").unwrap_or(raw);
    let mut nodes = BTreeMap::new();
    let explicit_root = effective
        .get("root_instance_id")
        .or_else(|| effective.get("root_id"))
        .or_else(|| effective.get("root"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let mut first_id: Option<String> = None;

    if let Some(arr) = effective.get("components").and_then(|v| v.as_array()) {
        for (idx, item) in arr.iter().enumerate() {
            let Some(obj) = item.as_object() else {
                return Err(format!("component[{idx}] must be an object"));
            };
            let node = parse_component_object(idx, obj)?;
            if first_id.is_none() {
                first_id = Some(node.instance_id.clone());
            }
            nodes.insert(node.instance_id.clone(), node);
        }
    } else if let Some(map) = effective.get("nodes").and_then(|v| v.as_object()) {
        for (id, node_val) in map {
            let mut node: UiNodeSnapshot = serde_json::from_value(node_val.clone())
                .map_err(|e| format!("invalid node '{id}': {e}"))?;
            if !SUPPORTED_COMPONENTS.contains(&node.component.as_str()) {
                return Err(format!(
                    "unsupported A2UI component '{}' (supported: {})",
                    node.component,
                    SUPPORTED_COMPONENTS.join(", ")
                ));
            }
            if node.instance_id.is_empty() {
                node.instance_id = id.clone();
            }
            if first_id.is_none() {
                first_id = Some(node.instance_id.clone());
            }
            nodes.insert(node.instance_id.clone(), node);
        }
    } else {
        return Err("missing 'components' array or 'nodes' map".into());
    }

    let root = explicit_root
        .or_else(|| {
            if nodes.contains_key("root") {
                Some("root".to_string())
            } else {
                None
            }
        })
        .or(first_id);
    if let Some(ref r) = root {
        if !nodes.contains_key(r) {
            return Err(format!("root component '{r}' not found in components"));
        }
    }

    validate_component_graph(&nodes)?;
    Ok((root, nodes))
}

/// Upsert a surface in SQLite and return the projected `UiSnapshot`.
pub fn upsert_ui_surface(
    store: &Store,
    owner: UiSurfaceOwner,
    surface_id: &str,
    summary: &str,
    root_instance_id: Option<String>,
    raw_nodes: BTreeMap<String, UiNodeSnapshot>,
    data_model: Value,
) -> Result<UiSnapshot, String> {
    upsert_ui_surface_with_options(
        store,
        owner,
        surface_id,
        summary,
        root_instance_id,
        raw_nodes,
        data_model,
        false,
    )
}

/// Upsert a surface with explicit `send_data_model` setting.
#[allow(clippy::too_many_arguments)]
pub fn upsert_ui_surface_with_options(
    store: &Store,
    owner: UiSurfaceOwner,
    surface_id: &str,
    summary: &str,
    root_instance_id: Option<String>,
    raw_nodes: BTreeMap<String, UiNodeSnapshot>,
    data_model: Value,
    send_data_model: bool,
) -> Result<UiSnapshot, String> {
    if summary.len() > MAX_SUMMARY_BYTES {
        return Err(format!(
            "UI surface summary exceeds {} byte budget ({} bytes)",
            MAX_SUMMARY_BYTES,
            summary.len()
        ));
    }
    if raw_nodes.len() > MAX_NODES_PER_SURFACE {
        return Err(format!(
            "UI surface exceeds max {} nodes ({} provided)",
            MAX_NODES_PER_SURFACE,
            raw_nodes.len()
        ));
    }
    if send_data_model && !data_model.is_object() {
        return Err("sendDataModel requires data_model to be a JSON object".into());
    }

    let key = surface_storage_key(&owner.conversation_id, surface_id);
    let existing: Option<StoredUiSurface> = store
        .get_module_record(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?;

    let needs_new_active_slot = match &existing {
        None => true,
        Some(prev) => prev.current.phase == "deleted",
    };
    if needs_new_active_slot {
        let active_count = list_conversation_surfaces(store, &owner.conversation_id)
            .into_iter()
            .filter(|s| s.phase != "deleted")
            .count();
        if active_count >= MAX_ACTIVE_SURFACES_PER_CONVERSATION {
            return Err(format!(
                "conversation '{}' already has the maximum {} active UI surfaces",
                owner.conversation_id, MAX_ACTIVE_SURFACES_PER_CONVERSATION
            ));
        }
    }

    let (generation, revision, mut revisions, mut live_revisions) = match existing {
        Some(prev) if prev.current.phase == "deleted" => (
            prev.current.identity.generation.saturating_add(1),
            1,
            BTreeMap::new(),
            BTreeMap::new(),
        ),
        Some(prev) => (
            prev.current.identity.generation,
            prev.current.revision + 1,
            prev.revisions,
            prev.live_revisions,
        ),
        None => (1, 1, BTreeMap::new(), BTreeMap::new()),
    };

    let (projected_root, projected) =
        project_surface_nodes(root_instance_id.as_deref(), &raw_nodes, &data_model, false)?;
    let identity = UiSurfaceIdentity {
        owner: owner.clone(),
        surface_id: surface_id.to_string(),
        generation,
    };
    let snapshot = UiSnapshot {
        identity,
        revision,
        catalog_id: DEFAULT_CATALOG_ID.into(),
        catalog_digest: catalog_digest(),
        phase: "ready".into(),
        root_instance_id: projected_root,
        nodes: projected,
        summary: summary.to_string(),
    };

    let payload_size = serde_json::to_vec(&json!({
        "snapshot": &snapshot,
        "data_model": &data_model
    }))
    .map(|v| v.len())
    .unwrap_or(0);
    if payload_size > MAX_SURFACE_PAYLOAD_BYTES {
        return Err(format!(
            "UI surface serialized payload exceeds {} byte budget ({} bytes)",
            MAX_SURFACE_PAYLOAD_BYTES, payload_size
        ));
    }

    revisions.insert(revision, to_read_only_snapshot(&snapshot));
    trim_revisions(&mut revisions);
    live_revisions.insert(revision, snapshot.clone());
    trim_revisions(&mut live_revisions);

    let stored = StoredUiSurface {
        storage_key: key.clone(),
        conversation_id: owner.conversation_id,
        surface_id: surface_id.to_string(),
        current: snapshot.clone(),
        data_model,
        revisions,
        updated_at_ms: now_ms(),
        base_nodes: raw_nodes,
        base_root_id: root_instance_id,
        send_data_model,
        live_revisions,
    };
    store
        .put_module_record(KIND_UI_SURFACE, &key, &stored)
        .map_err(|e| e.to_string())?;
    Ok(snapshot)
}

/// Apply one or more A2UI v0.9.1 protocol messages (`createSurface`, `updateComponents`,
/// `updateDataModel`, `deleteSurface`) atomically to SQLite storage.
pub fn apply_a2ui_messages(
    store: &Store,
    owner: UiSurfaceOwner,
    messages: &[Value],
) -> Result<Option<UiSnapshot>, String> {
    if messages.is_empty() {
        return Err("A2UI messages array must not be empty".into());
    }
    let mut active_surface_id: Option<String> = None;
    let mut base_nodes: BTreeMap<String, UiNodeSnapshot> = BTreeMap::new();
    let mut root_id: Option<String> = None;
    let mut data_model = json!({});
    let mut send_data_model = false;
    let mut summary = "Interactive UI Surface".to_string();
    let mut deleted = false;
    let mut loaded_existing = false;

    for msg in messages {
        let Some(obj) = msg.as_object() else {
            return Err("each A2UI message must be a JSON object".into());
        };
        if let Some(create) = obj.get("createSurface").and_then(Value::as_object) {
            let sid = create
                .get("surfaceId")
                .or_else(|| create.get("surface_id"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| "createSurface missing non-empty 'surfaceId'".to_string())?;
            active_surface_id = Some(sid.to_string());
            send_data_model = create
                .get("sendDataModel")
                .or_else(|| create.get("send_data_model"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(s) = create.get("summary").and_then(Value::as_str) {
                summary = s.to_string();
            }
            base_nodes.clear();
            root_id = None;
            data_model = json!({});
            deleted = false;
            loaded_existing = true;
        } else if let Some(update) = obj.get("updateComponents").and_then(Value::as_object) {
            let sid = update
                .get("surfaceId")
                .or_else(|| update.get("surface_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| active_surface_id.clone())
                .ok_or_else(|| "updateComponents missing 'surfaceId'".to_string())?;
            if !loaded_existing {
                let key = surface_storage_key(&owner.conversation_id, &sid);
                if let Some(prev) = store
                    .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
                    .map_err(|e| e.to_string())?
                {
                    base_nodes = if prev.base_nodes.is_empty() {
                        prev.current.nodes
                    } else {
                        prev.base_nodes
                    };
                    root_id = prev.base_root_id.or(prev.current.root_instance_id);
                    data_model = prev.data_model;
                    send_data_model = prev.send_data_model;
                    summary = prev.current.summary;
                }
                loaded_existing = true;
            }
            active_surface_id = Some(sid);
            let (parsed_root, parsed_nodes) = build_nodes_from_spec(&Value::Object(update.clone()))?;
            for (k, v) in parsed_nodes {
                base_nodes.insert(k, v);
            }
            if root_id.is_none() || base_nodes.contains_key("root") {
                root_id = parsed_root.or(root_id);
            }
            validate_component_graph(&base_nodes)?;
        } else if let Some(dm) = obj.get("updateDataModel").and_then(Value::as_object) {
            let sid = dm
                .get("surfaceId")
                .or_else(|| dm.get("surface_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| active_surface_id.clone())
                .ok_or_else(|| "updateDataModel missing 'surfaceId'".to_string())?;
            if !loaded_existing {
                let key = surface_storage_key(&owner.conversation_id, &sid);
                if let Some(prev) = store
                    .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
                    .map_err(|e| e.to_string())?
                {
                    base_nodes = if prev.base_nodes.is_empty() {
                        prev.current.nodes
                    } else {
                        prev.base_nodes
                    };
                    root_id = prev.base_root_id.or(prev.current.root_instance_id);
                    data_model = prev.data_model;
                    send_data_model = prev.send_data_model;
                    summary = prev.current.summary;
                }
                loaded_existing = true;
            }
            active_surface_id = Some(sid);
            let path = dm.get("path").and_then(Value::as_str).unwrap_or("/");
            if let Some(val) = dm.get("value") {
                json_pointer_set(&mut data_model, path, val.clone());
            } else {
                json_pointer_delete(&mut data_model, path);
            }
        } else if let Some(del) = obj.get("deleteSurface").and_then(Value::as_object) {
            let sid = del
                .get("surfaceId")
                .or_else(|| del.get("surface_id"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| active_surface_id.clone())
                .ok_or_else(|| "deleteSurface missing 'surfaceId'".to_string())?;
            active_surface_id = Some(sid);
            deleted = true;
        } else {
            return Err(
                "unsupported A2UI message envelope (expected createSurface, updateComponents, updateDataModel, or deleteSurface)"
                    .into(),
            );
        }
    }

    let sid = active_surface_id
        .ok_or_else(|| "A2UI messages did not identify a surfaceId".to_string())?;
    if deleted {
        return delete_ui_surface(store, &owner.conversation_id, &sid);
    }
    let snap = upsert_ui_surface_with_options(
        store,
        owner,
        &sid,
        &summary,
        root_id,
        base_nodes,
        data_model,
        send_data_model,
    )?;
    Ok(Some(snap))
}

pub fn list_conversation_surfaces(store: &Store, conversation_id: &str) -> Vec<UiSnapshot> {
    store
        .list_module_records::<StoredUiSurface>(KIND_UI_SURFACE)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.conversation_id == conversation_id)
        .map(|s| s.current)
        .collect()
}

pub fn read_ui_surface_revision(
    store: &Store,
    reference: &UiSurfaceRevisionRef,
) -> Result<Option<UiSnapshot>, String> {
    let key = surface_storage_key(
        &reference.identity.owner.conversation_id,
        &reference.identity.surface_id,
    );
    let Some(stored) = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    if let Some(rev) = stored.revisions.get(&reference.revision) {
        return Ok(Some(rev.clone()));
    }
    if stored.current.revision == reference.revision {
        return Ok(Some(to_read_only_snapshot(&stored.current)));
    }
    Ok(None)
}

/// Compare `base_revision` snapshot against current snapshot; emit an incremental `UiChangeSet`
/// (`kind: "changes"`, `upserted_nodes`, `removed_instance_ids`) when the serialized diff is
/// smaller than the full snapshot, otherwise emit `"snapshot"`.
pub fn read_ui_surface_changes(
    store: &Store,
    identity: &UiSurfaceIdentity,
    base_revision: Option<u64>,
) -> Result<Value, String> {
    let key = surface_storage_key(&identity.owner.conversation_id, &identity.surface_id);
    let Some(stored) = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
    else {
        return Ok(json!({
            "type": "missing",
            "kind": "missing",
            "identity": identity
        }));
    };
    if stored.current.identity.generation != identity.generation {
        return Ok(json!({
            "type": "snapshot",
            "kind": "snapshot",
            "snapshot": stored.current
        }));
    }
    if base_revision == Some(stored.current.revision) {
        return Ok(json!({
            "type": "unchanged",
            "kind": "unchanged",
            "identity": stored.current.identity,
            "revision": stored.current.revision
        }));
    }

    if let Some(base_rev) = base_revision {
        if base_rev < stored.current.revision {
            if let Some(prev) = stored.live_revisions.get(&base_rev) {
                let upserted_nodes: Vec<UiNodeSnapshot> = stored
                    .current
                    .nodes
                    .iter()
                    .filter(|(id, node)| prev.nodes.get(*id) != Some(*node))
                    .map(|(_, node)| node.clone())
                    .collect();
                let removed_instance_ids: Vec<String> = prev
                    .nodes
                    .keys()
                    .filter(|id| !stored.current.nodes.contains_key(*id))
                    .cloned()
                    .collect();

                let changes = UiChangeSet {
                    identity: stored.current.identity.clone(),
                    base_revision: prev.revision,
                    revision: stored.current.revision,
                    catalog_digest: stored.current.catalog_digest.clone(),
                    phase: stored.current.phase.clone(),
                    root_instance_id: stored.current.root_instance_id.clone(),
                    upserted_nodes,
                    removed_instance_ids,
                    summary: stored.current.summary.clone(),
                };
                let diff_size = serde_json::to_vec(&changes)
                    .map(|v| v.len())
                    .unwrap_or(usize::MAX);
                let snap_size = serde_json::to_vec(&stored.current)
                    .map(|v| v.len())
                    .unwrap_or(0);
                if diff_size < snap_size {
                    return Ok(json!({
                        "type": "changes",
                        "kind": "changes",
                        "identity": changes.identity,
                        "base_revision": changes.base_revision,
                        "revision": changes.revision,
                        "catalog_digest": changes.catalog_digest,
                        "phase": changes.phase,
                        "root_instance_id": changes.root_instance_id,
                        "upserted_nodes": changes.upserted_nodes,
                        "removed_instance_ids": changes.removed_instance_ids,
                        "summary": changes.summary,
                        "changes": changes
                    }));
                }
            }
        }
    }

    Ok(json!({
        "type": "snapshot",
        "kind": "snapshot",
        "snapshot": stored.current
    }))
}

pub fn preview_ui_surface(
    store: &Store,
    input: &UiDraftPreviewInput,
) -> Result<UiDraftPreview, String> {
    let key = surface_storage_key(
        &input.identity.owner.conversation_id,
        &input.identity.surface_id,
    );
    let stored = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "UI surface not found".to_string())?;
    if stored.current.revision != input.expected_surface_revision {
        return Err(format!(
            "revision conflict: expected {}, actual {}",
            input.expected_surface_revision, stored.current.revision
        ));
    }
    let mut draft_model = stored.data_model.clone();
    apply_edits_to_model(&stored.current.nodes, &mut draft_model, &input.edits)?;
    let source_nodes = if stored.base_nodes.is_empty() {
        &stored.current.nodes
    } else {
        &stored.base_nodes
    };
    let source_root = stored
        .base_root_id
        .as_deref()
        .or(stored.current.root_instance_id.as_deref());
    let (projected_root, projected_nodes) =
        project_surface_nodes(source_root, source_nodes, &draft_model, false)?;
    let mut snapshot = stored.current.clone();
    snapshot.root_instance_id = projected_root;
    snapshot.nodes = projected_nodes;
    Ok(UiDraftPreview {
        identity: stored.current.identity,
        base_revision: stored.current.revision,
        draft_generation: input.draft_generation,
        snapshot,
    })
}

pub fn commit_ui_interaction(
    store: &Store,
    interaction: &UiInteraction,
) -> Result<CommittedInteraction, String> {
    let interaction_bytes = serde_json::to_vec(interaction)
        .map(|v| v.len())
        .unwrap_or(0);
    if interaction_bytes > MAX_INTERACTION_PAYLOAD_BYTES {
        return Err(format!(
            "UI interaction payload exceeds {} byte budget ({} bytes)",
            MAX_INTERACTION_PAYLOAD_BYTES, interaction_bytes
        ));
    }

    let key = surface_storage_key(
        &interaction.identity.owner.conversation_id,
        &interaction.identity.surface_id,
    );
    let mut stored = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "UI surface not found".to_string())?;
    if stored.current.phase == "deleted" {
        return Err("cannot interact with a deleted UI surface".into());
    }
    if stored.current.identity.generation != interaction.identity.generation {
        return Err(format!(
            "generation conflict: expected {}, actual {}",
            interaction.identity.generation, stored.current.identity.generation
        ));
    }
    if stored.current.revision != interaction.expected_surface_revision {
        return Err(format!(
            "revision conflict: expected {}, actual {}",
            interaction.expected_surface_revision, stored.current.revision
        ));
    }
    let mut next_model = stored.data_model.clone();
    apply_edits_to_model(&stored.current.nodes, &mut next_model, &interaction.edits)?;
    if stored.send_data_model && !next_model.is_object() {
        return Err("sendDataModel requires data_model to remain a JSON object".into());
    }

    let source_nodes = if stored.base_nodes.is_empty() {
        stored.current.nodes.clone()
    } else {
        stored.base_nodes.clone()
    };
    let source_root = stored
        .base_root_id
        .as_deref()
        .or(stored.current.root_instance_id.as_deref());
    let (projected_root, projected_nodes) =
        project_surface_nodes(source_root, &source_nodes, &next_model, false)?;

    let all_errors: Vec<String> = projected_nodes
        .values()
        .flat_map(|n| n.validation_errors.iter().cloned())
        .collect();
    if !all_errors.is_empty() && interaction.action.is_some() {
        return Err(format!("validation failed: {}", all_errors.join("; ")));
    }

    let action_id = interaction.action.as_ref().map(|a| a.action_id.clone());
    if let Some(ref act) = interaction.action {
        let valid_action = projected_nodes
            .get(&act.instance_id)
            .is_some_and(|n| n.actions.values().any(|a| a.action_id == act.action_id));
        if !valid_action {
            return Err(format!(
                "action '{}' not found on node '{}'",
                act.action_id, act.instance_id
            ));
        }
    }

    let next_rev = stored.current.revision + 1;
    stored.current.revision = next_rev;
    stored.current.root_instance_id = projected_root;
    stored.current.nodes = projected_nodes;
    stored.data_model = next_model.clone();
    stored
        .revisions
        .insert(next_rev, to_read_only_snapshot(&stored.current));
    trim_revisions(&mut stored.revisions);
    stored
        .live_revisions
        .insert(next_rev, stored.current.clone());
    trim_revisions(&mut stored.live_revisions);
    stored.updated_at_ms = now_ms();
    store
        .put_module_record(KIND_UI_SURFACE, &key, &stored)
        .map_err(|e| e.to_string())?;

    let a2ui_client_data_model = if stored.send_data_model && action_id.is_some() {
        Some(json!({
            "version": "v0.9",
            "surfaces": {
                stored.surface_id.clone(): next_model.clone()
            }
        }))
    } else {
        None
    };

    let summary = match &action_id {
        Some(aid) => format!(
            "Submitted UI action '{}' on surface '{}' with data: {}",
            aid, stored.surface_id, next_model
        ),
        None => format!(
            "Updated UI surface '{}' with data: {}",
            stored.surface_id, next_model
        ),
    };
    Ok(CommittedInteraction {
        snapshot: stored.current,
        data_model: next_model,
        action_id,
        summary,
        a2ui_client_data_model,
    })
}

pub fn delete_ui_surface(
    store: &Store,
    conversation_id: &str,
    surface_id: &str,
) -> Result<Option<UiSnapshot>, String> {
    let key = surface_storage_key(conversation_id, surface_id);
    let Some(mut stored) = store
        .get_module_record::<StoredUiSurface>(KIND_UI_SURFACE, &key)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    stored.current.revision += 1;
    stored.current.phase = "deleted".into();
    stored.current.root_instance_id = None;
    stored.current.nodes.clear();
    stored.base_nodes.clear();
    stored.base_root_id = None;
    stored
        .revisions
        .insert(stored.current.revision, stored.current.clone());
    trim_revisions(&mut stored.revisions);
    stored
        .live_revisions
        .insert(stored.current.revision, stored.current.clone());
    trim_revisions(&mut stored.live_revisions);
    stored.updated_at_ms = now_ms();
    store
        .put_module_record(KIND_UI_SURFACE, &key, &stored)
        .map_err(|e| e.to_string())?;
    Ok(Some(stored.current))
}

/// Register A2UI tools on a `ToolRegistry`.
pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    register_scoped_tools(suite, registry, "local-host", "default");
}

/// Register A2UI tools bound to a specific conversation + host identity (used by WebHost / Desktop).
pub fn register_scoped_tools(
    suite: &Arc<ModulesSuite>,
    registry: &mut ToolRegistry,
    authority_id: &str,
    conversation_id: &str,
) {
    registry.register(Arc::new(RenderUiSurfaceTool {
        suite: Arc::clone(suite),
        authority_id: authority_id.to_string(),
        conversation_id: conversation_id.to_string(),
    }));
    registry.register(Arc::new(DeleteUiSurfaceTool {
        suite: Arc::clone(suite),
        conversation_id: conversation_id.to_string(),
    }));
}

pub struct RenderUiSurfaceTool {
    suite: Arc<ModulesSuite>,
    authority_id: String,
    conversation_id: String,
}

impl ToolHandler for RenderUiSurfaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "render_ui_surface".into(),
                description: "Render or update an interactive A2UI Generative UI surface (forms, cards, choices, sliders, dynamic lists, buttons) inside the conversation. Accepts either flat v2 component specs or v0.9.1 message envelopes.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "surface_id": { "type": "string", "description": "Stable identifier for this UI surface (e.g. 'deploy_form')" },
                        "summary": { "type": "string", "description": "Short human-readable summary of the surface" },
                        "root_id": { "type": "string", "description": "ID of the root component" },
                        "sendDataModel": { "type": "boolean", "description": "Include authoritative a2uiClientDataModel in committed action payloads" },
                        "components": {
                            "type": "array",
                            "description": "Flat list of A2UI Basic Catalog components (Card, Column, Row, Text, Button, TextField, CheckBox, ChoicePicker, Slider, DateTimeInput, Tabs, Modal, List, Divider, Icon)",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "component": { "type": "string" },
                                    "props": { "type": "object" },
                                    "children": {},
                                    "child": { "type": "string" },
                                    "bind": { "type": "string" },
                                    "checks": { "type": "array" },
                                    "action_id": { "type": "string" }
                                },
                                "required": ["id", "component"]
                            }
                        },
                        "data_model": { "type": "object", "description": "Initial JSON data model for two-way component bindings" },
                        "messages": { "type": "array", "description": "Optional A2UI v0.9.1 message envelopes (createSurface, updateComponents, updateDataModel, deleteSurface)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let owner = UiSurfaceOwner {
            authority_id: self.authority_id.clone(),
            conversation_id: self.conversation_id.clone(),
            participant_id: "assistant".into(),
            context_id: "conversation".into(),
            originating_run_id: new_id(),
            message_id: format!("ui-{}", now_ms()),
            part_id: format!(
                "part-{}",
                args.get("surface_id")
                    .and_then(Value::as_str)
                    .unwrap_or("surface")
            ),
        };

        // Support v0.9.1 message envelope list or single envelope directly
        if let Some(msgs) = args.get("messages").and_then(Value::as_array) {
            return match apply_a2ui_messages(&self.suite.store, owner, msgs) {
                Ok(Some(snapshot)) => ToolExecutionResult::ok_structured(
                    json!({
                        "ui_surface": snapshot,
                        "surface_id": snapshot.identity.surface_id,
                        "revision": snapshot.revision
                    }),
                    format!(
                        "Applied A2UI messages to '{}' (rev {}, {} nodes)",
                        snapshot.identity.surface_id,
                        snapshot.revision,
                        snapshot.nodes.len()
                    ),
                ),
                Ok(None) => ToolExecutionResult::ok_structured(
                    json!({"deleted": true}),
                    "Deleted A2UI surface via message envelope",
                ),
                Err(e) => ToolExecutionResult::err_structured("ui_surface_error", e, None),
            };
        }
        if args.get("createSurface").is_some()
            || args.get("updateComponents").is_some()
            || args.get("updateDataModel").is_some()
            || args.get("deleteSurface").is_some()
        {
            return match apply_a2ui_messages(&self.suite.store, owner, &[args]) {
                Ok(Some(snapshot)) => ToolExecutionResult::ok_structured(
                    json!({
                        "ui_surface": snapshot,
                        "surface_id": snapshot.identity.surface_id,
                        "revision": snapshot.revision
                    }),
                    format!(
                        "Applied A2UI envelope to '{}' (rev {}, {} nodes)",
                        snapshot.identity.surface_id,
                        snapshot.revision,
                        snapshot.nodes.len()
                    ),
                ),
                Ok(None) => ToolExecutionResult::ok_structured(
                    json!({"deleted": true}),
                    "Deleted A2UI surface via envelope",
                ),
                Err(e) => ToolExecutionResult::err_structured("ui_surface_error", e, None),
            };
        }

        let Some(surface_id) = args
            .get("surface_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        else {
            return ToolExecutionResult::err_structured(
                "missing_param",
                "missing non-empty 'surface_id'",
                None,
            );
        };
        let summary = args
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("Interactive UI Surface");
        let data_model = args.get("data_model").cloned().unwrap_or_else(|| json!({}));
        let send_data_model = args
            .get("sendDataModel")
            .or_else(|| args.get("send_data_model"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (root_id, nodes) = match build_nodes_from_spec(&args) {
            Ok(pair) => pair,
            Err(e) => return ToolExecutionResult::err_structured("invalid_ui_spec", e, None),
        };
        match upsert_ui_surface_with_options(
            &self.suite.store,
            owner,
            surface_id,
            summary,
            root_id,
            nodes,
            data_model,
            send_data_model,
        ) {
            Ok(snapshot) => ToolExecutionResult::ok_structured(
                json!({
                    "ui_surface": snapshot,
                    "surface_id": surface_id,
                    "revision": snapshot.revision
                }),
                format!(
                    "Rendered A2UI surface '{}' (rev {}, {} nodes)",
                    surface_id,
                    snapshot.revision,
                    snapshot.nodes.len()
                ),
            ),
            Err(e) => ToolExecutionResult::err_structured("ui_surface_error", e, None),
        }
    }
}

pub struct DeleteUiSurfaceTool {
    suite: Arc<ModulesSuite>,
    conversation_id: String,
}

impl ToolHandler for DeleteUiSurfaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "delete_ui_surface".into(),
                description: "Remove an existing interactive A2UI surface from the conversation."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "surface_id": { "type": "string" }
                    },
                    "required": ["surface_id"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(surface_id) = args.get("surface_id").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured(
                "missing_param",
                "missing 'surface_id'",
                None,
            );
        };
        match delete_ui_surface(&self.suite.store, &self.conversation_id, surface_id) {
            Ok(Some(snapshot)) => ToolExecutionResult::ok_structured(
                json!({"ui_surface": snapshot, "deleted": true}),
                format!("Deleted A2UI surface '{surface_id}'"),
            ),
            Ok(None) => ToolExecutionResult::ok_structured(
                json!({"deleted": false, "surface_id": surface_id}),
                format!("UI surface '{surface_id}' did not exist"),
            ),
            Err(e) => ToolExecutionResult::err_structured("ui_surface_error", e, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_owner(conv_id: &str) -> UiSurfaceOwner {
        UiSurfaceOwner {
            authority_id: "host-1".into(),
            conversation_id: conv_id.into(),
            participant_id: "assistant".into(),
            context_id: "conversation".into(),
            originating_run_id: "run-1".into(),
            message_id: "msg-1".into(),
            part_id: "part-1".into(),
        }
    }

    #[test]
    fn render_ui_surface_tool_is_deferred_tier() {
        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store, std::path::PathBuf::from("."), None));
        let tool = RenderUiSurfaceTool {
            suite,
            authority_id: "host-1".into(),
            conversation_id: "conv-1".into(),
        };
        assert_eq!(tool.spec().tier, ToolTier::Deferred);
    }

    #[test]
    fn a2ui_surface_lifecycle_preview_validation_interaction_and_history() {
        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(
            store.clone(),
            std::path::PathBuf::from("."),
            None,
        ));
        let mut reg = ToolRegistry::with_builtin_tools();
        register_scoped_tools(&suite, &mut reg, "host-1", "conv-1");
        let ctx = ToolContext::new(".");

        let res = reg.execute(
            "render_ui_surface",
            &ctx,
            json!({
                "surface_id": "deploy_form",
                "summary": "Deploy Configuration",
                "root_id": "card",
                "components": [
                    { "id": "card", "component": "Card", "child": "col" },
                    { "id": "col", "component": "Column", "children": ["title", "env_input", "submit_btn"] },
                    { "id": "title", "component": "Text", "text": "Deploy Service", "variant": "h2" },
                    { "id": "env_input", "component": "TextField", "label": "Environment", "bind": "/env", "required": true, "minLength": 3 },
                    { "id": "submit_btn", "component": "Button", "variant": "primary", "child": "btn_label", "action_id": "confirm_deploy" },
                    { "id": "btn_label", "component": "Text", "text": "Deploy Now" }
                ],
                "data_model": { "env": "" }
            }),
        );
        assert!(
            !res.is_error,
            "render_ui_surface failed: {}",
            res.payload.to_model_value()
        );
        let snap: UiSnapshot =
            serde_json::from_value(res.payload.to_model_value()["ui_surface"].clone()).unwrap();
        assert_eq!(snap.revision, 1);
        assert_eq!(snap.phase, "ready");
        assert!(!snap.nodes["env_input"].validation_errors.is_empty());

        let bad_preview = preview_ui_surface(
            &store,
            &UiDraftPreviewInput {
                identity: snap.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "env_input".into(),
                    binding: "value".into(),
                    value: json!("ab"),
                }],
                draft_generation: 2,
            },
        )
        .unwrap();
        assert!(!bad_preview.snapshot.nodes["env_input"]
            .validation_errors
            .is_empty());

        let ok_preview = preview_ui_surface(
            &store,
            &UiDraftPreviewInput {
                identity: snap.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "env_input".into(),
                    binding: "value".into(),
                    value: json!("production"),
                }],
                draft_generation: 3,
            },
        )
        .unwrap();
        assert!(ok_preview.snapshot.nodes["env_input"]
            .validation_errors
            .is_empty());
        assert_eq!(
            ok_preview.snapshot.nodes["env_input"].props["value"],
            json!("production")
        );

        let committed = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: snap.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "env_input".into(),
                    binding: "value".into(),
                    value: json!("production"),
                }],
                action: Some(UiActionRef {
                    instance_id: "submit_btn".into(),
                    action_id: "confirm_deploy".into(),
                }),
            },
        )
        .unwrap();
        assert_eq!(committed.snapshot.revision, 2);
        assert_eq!(committed.action_id.as_deref(), Some("confirm_deploy"));
        assert_eq!(committed.data_model["env"], json!("production"));

        let hist = read_ui_surface_revision(
            &store,
            &UiSurfaceRevisionRef {
                identity: snap.identity.clone(),
                revision: 1,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(hist.phase, "read_only");
        assert!(hist
            .nodes
            .values()
            .all(|n| n.actions.is_empty() && n.bindings.is_empty()));

        let deleted = delete_ui_surface(&store, "conv-1", "deploy_form")
            .unwrap()
            .unwrap();
        assert_eq!(deleted.phase, "deleted");
    }

    #[test]
    fn pure_function_evaluator_and_check_rules() {
        let data = json!({
            "email": "alice@example.com",
            "bad_email": "not-an-email",
            "price": 1234.5,
            "count": 3,
            "date": "2026-11-25T15:30:00Z",
            "name": "Nola"
        });
        let mut eval = PureEvaluator::new(&data);

        assert_eq!(
            eval.resolve(
                &json!({"call": "email", "args": {"value": {"path": "/email"}}}),
                ""
            )
            .unwrap(),
            json!(true)
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "email", "args": {"value": {"path": "/bad_email"}}}),
                ""
            )
            .unwrap(),
            json!(false)
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "and", "args": {"values": [
                    {"call": "required", "args": {"value": {"path": "/name"}}},
                    {"call": "length", "args": {"value": {"path": "/name"}, "min": 2, "max": 10}},
                    {"call": "regex", "args": {"value": {"path": "/name"}, "pattern": "^N[a-z]+$"}},
                    {"call": "numeric", "args": {"value": {"path": "/price"}, "min": 100, "max": 2000}},
                    {"call": "not", "args": {"value": false}}
                ]}}),
                ""
            )
            .unwrap(),
            json!(true)
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "or", "args": {"values": [false, true]}}),
                ""
            )
            .unwrap(),
            json!(true)
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "formatString", "args": {"template": "Hello ${/name}, total {count}", "count": {"path": "/count"}}}),
                ""
            )
            .unwrap(),
            json!("Hello Nola, total 3")
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "formatNumber", "args": {"value": {"path": "/price"}, "decimals": 2}}),
                ""
            )
            .unwrap(),
            json!("1,234.50")
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "formatCurrency", "args": {"value": {"path": "/price"}, "currency": "USD"}}),
                ""
            )
            .unwrap(),
            json!("$1,234.50")
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "formatDate", "args": {"value": {"path": "/date"}, "format": "short"}}),
                ""
            )
            .unwrap(),
            json!("2026-11-25")
        );
        assert_eq!(
            eval.resolve(
                &json!({"call": "pluralize", "args": {"count": {"path": "/count"}, "one": "{count} item", "other": "{count} items"}}),
                ""
            )
            .unwrap(),
            json!("3 items")
        );

        // Shared op budget exhaustion check
        let tiny_budget = Rc::new(Cell::new(2));
        let mut bounded_eval = PureEvaluator::with_budget(&data, tiny_budget);
        let budget_err = bounded_eval
            .resolve(
                &json!({"call": "and", "args": {"values": [true, true, true]}}),
                "",
            )
            .unwrap_err();
        assert!(budget_err.contains("budget"), "got: {budget_err}");

        // CheckRule validation on component
        let store = Store::open_in_memory().unwrap();
        let (root, nodes) = build_nodes_from_spec(&json!({
            "root_id": "email_field",
            "components": [{
                "id": "email_field",
                "component": "TextField",
                "label": "Email",
                "bind": "/email",
                "checks": [
                    {
                        "condition": { "call": "email", "args": { "value": { "path": "/email" } } },
                        "message": "Must be a valid email address"
                    }
                ]
            }]
        }))
        .unwrap();
        let bad_snap = upsert_ui_surface(
            &store,
            test_owner("conv-chk"),
            "chk",
            "Check test",
            root,
            nodes,
            json!({"email": "bad"}),
        )
        .unwrap();
        assert_eq!(
            bad_snap.nodes["email_field"].validation_errors,
            vec!["Must be a valid email address"]
        );
    }

    #[test]
    fn dynamic_list_expansion_enforces_identity_and_immutability() {
        let store = Store::open_in_memory().unwrap();
        let (root, nodes) = build_nodes_from_spec(&json!({
            "root_id": "list",
            "components": [
                { "id": "list", "component": "List", "children": { "componentId": "row_field", "path": "/items" } },
                { "id": "row_field", "component": "TextField", "label": "Item Name", "value": { "path": "name" } }
            ]
        }))
        .unwrap();

        // Valid editable list with unique stable ids
        let snap = upsert_ui_surface(
            &store,
            test_owner("conv-list"),
            "items_surf",
            "Items",
            root.clone(),
            nodes.clone(),
            json!({
                "items": [
                    { "id": "a", "name": "Alpha" },
                    { "id": "b", "name": "Beta" }
                ]
            }),
        )
        .unwrap();
        assert_eq!(snap.nodes["list"].slots["children"].len(), 2);
        let first_inst = &snap.nodes["list"].slots["children"][0];
        assert_eq!(snap.nodes[first_inst].props["value"], json!("Alpha"));
        assert_eq!(snap.nodes[first_inst].bindings["value"], "/items/0/name");

        // Missing or duplicate id on editable list -> InvalidListIdentity
        let missing_id_err = upsert_ui_surface(
            &store,
            test_owner("conv-list"),
            "items_bad1",
            "Items",
            root.clone(),
            nodes.clone(),
            json!({ "items": [{ "name": "NoId" }] }),
        )
        .unwrap_err();
        assert!(
            missing_id_err.contains("InvalidListIdentity"),
            "got: {missing_id_err}"
        );

        let dup_id_err = upsert_ui_surface(
            &store,
            test_owner("conv-list"),
            "items_bad2",
            "Items",
            root,
            nodes,
            json!({ "items": [{ "id": "dup", "name": "A" }, { "id": "dup", "name": "B" }] }),
        )
        .unwrap_err();
        assert!(
            dup_id_err.contains("InvalidListIdentity"),
            "got: {dup_id_err}"
        );

        // Attempting to bind editable input to item `id` -> ImmutableListIdentity
        let (bad_root, bad_nodes) = build_nodes_from_spec(&json!({
            "root_id": "list",
            "components": [
                { "id": "list", "component": "List", "children": { "componentId": "id_field", "path": "/items" } },
                { "id": "id_field", "component": "TextField", "label": "ID", "value": { "path": "id" } }
            ]
        }))
        .unwrap();
        let immut_err = upsert_ui_surface(
            &store,
            test_owner("conv-list"),
            "items_immut",
            "Items",
            bad_root,
            bad_nodes,
            json!({ "items": [{ "id": "a", "name": "Alpha" }] }),
        )
        .unwrap_err();
        assert!(
            immut_err.contains("ImmutableListIdentity"),
            "got: {immut_err}"
        );
    }

    #[test]
    fn component_graph_cycle_detection_and_max_depth_32() {
        // Direct cycle A -> B -> A
        let cycle_err = build_nodes_from_spec(&json!({
            "root_id": "a",
            "components": [
                { "id": "a", "component": "Card", "child": "b" },
                { "id": "b", "component": "Card", "child": "a" }
            ]
        }))
        .unwrap_err();
        assert!(cycle_err.contains("cycle"), "got: {cycle_err}");

        // Chain deeper than 32 rejected
        let mut chain = Vec::new();
        for i in 0..=34 {
            let mut obj = json!({ "id": format!("n{i}"), "component": "Card" });
            if i < 34 {
                obj["child"] = json!(format!("n{}", i + 1));
            }
            chain.push(obj);
        }
        let depth_err = build_nodes_from_spec(&json!({
            "root_id": "n0",
            "components": chain
        }))
        .unwrap_err();
        assert!(depth_err.contains("maximum depth"), "got: {depth_err}");
    }

    #[test]
    fn send_data_model_v091_envelopes_and_incremental_changeset_diff() {
        let store = Store::open_in_memory().unwrap();
        let owner = test_owner("conv-v091");

        // 1. Ingest v0.9.1 message envelopes with sendDataModel: true and many sibling nodes
        let mut comps = vec![json!({
            "id": "root",
            "component": "Column",
            "children": ["input_0", "btn_submit", "t1", "t2", "t3", "t4", "t5", "t6"]
        })];
        comps.push(json!({
            "id": "input_0",
            "component": "TextField",
            "label": "Title",
            "value": { "path": "/title" }
        }));
        comps.push(json!({
            "id": "btn_submit",
            "component": "Button",
            "child": "t1",
            "action": { "event": { "name": "save_form" } }
        }));
        for i in 1..=6 {
            comps.push(json!({
                "id": format!("t{i}"),
                "component": "Text",
                "text": format!("Static description paragraph #{i} with enough content to make full snapshots noticeably larger than single-node diffs.")
            }));
        }

        let snap1 = apply_a2ui_messages(
            &store,
            owner,
            &[
                json!({ "createSurface": { "surfaceId": "v091_form", "sendDataModel": true, "summary": "v0.9.1 Form" } }),
                json!({ "updateComponents": { "surfaceId": "v091_form", "components": comps } }),
                json!({ "updateDataModel": { "surfaceId": "v091_form", "path": "/title", "value": "Initial" } }),
            ],
        )
        .unwrap()
        .unwrap();
        assert_eq!(snap1.revision, 1);
        assert_eq!(snap1.nodes["input_0"].props["value"], json!("Initial"));

        // 2. Commit edit + action -> includes authoritative a2uiClientDataModel
        let committed = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: snap1.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "input_0".into(),
                    binding: "value".into(),
                    value: json!("Updated Title"),
                }],
                action: Some(UiActionRef {
                    instance_id: "btn_submit".into(),
                    action_id: "save_form".into(),
                }),
            },
        )
        .unwrap();
        assert_eq!(committed.snapshot.revision, 2);
        let client_dm = committed
            .a2ui_client_data_model
            .expect("sendDataModel must include a2uiClientDataModel");
        assert_eq!(
            client_dm["surfaces"]["v091_form"]["title"],
            json!("Updated Title")
        );

        // 3. Incremental UiChangeSet diff between rev 1 and rev 2 emits "changes" with only input_0
        let diff = read_ui_surface_changes(&store, &snap1.identity, Some(1)).unwrap();
        assert_eq!(diff["kind"], json!("changes"));
        assert_eq!(diff["type"], json!("changes"));
        let upserted = diff["upserted_nodes"].as_array().unwrap();
        assert_eq!(upserted.len(), 1);
        assert_eq!(upserted[0]["instance_id"], json!("input_0"));
        assert_eq!(diff["removed_instance_ids"], json!([]));

        // Same revision emits "unchanged"
        let unchanged = read_ui_surface_changes(&store, &snap1.identity, Some(2)).unwrap();
        assert_eq!(unchanged["kind"], json!("unchanged"));
    }

    #[test]
    fn a2ui_enforces_generation_bump_disabled_edits_and_budgets() {
        let store = Store::open_in_memory().unwrap();
        let (_, nodes) = build_nodes_from_spec(&json!({
            "root_id": "locked",
            "components": [
                { "id": "locked", "component": "TextField", "label": "Locked", "bind": "/val", "disabled": true }
            ]
        }))
        .unwrap();

        let s1 = upsert_ui_surface(
            &store,
            test_owner("conv-g"),
            "panel",
            "First generation",
            Some("locked".into()),
            nodes.clone(),
            json!({"val": "initial"}),
        )
        .unwrap();
        assert_eq!(s1.identity.generation, 1);
        assert_eq!(s1.revision, 1);

        let disabled_err = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: s1.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "locked".into(),
                    binding: "value".into(),
                    value: json!("hacked"),
                }],
                action: None,
            },
        )
        .unwrap_err();
        assert!(disabled_err.contains("disabled"));

        let del = delete_ui_surface(&store, "conv-g", "panel")
            .unwrap()
            .unwrap();
        assert_eq!(del.phase, "deleted");

        let s2 = upsert_ui_surface(
            &store,
            test_owner("conv-g"),
            "panel",
            "Second generation",
            Some("locked".into()),
            nodes.clone(),
            json!({"val": "fresh"}),
        )
        .unwrap();
        assert_eq!(s2.identity.generation, 2);
        assert_eq!(s2.revision, 1);
        assert_eq!(s2.phase, "ready");

        let huge_summary = "x".repeat(MAX_SUMMARY_BYTES + 1);
        let summary_err = upsert_ui_surface(
            &store,
            test_owner("conv-g"),
            "panel",
            &huge_summary,
            Some("locked".into()),
            nodes.clone(),
            json!({}),
        )
        .unwrap_err();
        assert!(summary_err.contains("summary"));

        for i in 0..MAX_ACTIVE_SURFACES_PER_CONVERSATION {
            upsert_ui_surface(
                &store,
                test_owner("conv-limit"),
                &format!("surf-{i}"),
                "ok",
                Some("locked".into()),
                nodes.clone(),
                json!({}),
            )
            .unwrap();
        }
        let limit_err = upsert_ui_surface(
            &store,
            test_owner("conv-limit"),
            "surf-overflow",
            "ok",
            Some("locked".into()),
            nodes.clone(),
            json!({}),
        )
        .unwrap_err();
        assert!(limit_err.contains("maximum 8"));

        let huge_interaction_err = commit_ui_interaction(
            &store,
            &UiInteraction {
                identity: s2.identity.clone(),
                expected_surface_revision: 1,
                edits: vec![UiEdit::Set {
                    instance_id: "locked".into(),
                    binding: "value".into(),
                    value: json!("a".repeat(MAX_INTERACTION_PAYLOAD_BYTES + 100)),
                }],
                action: None,
            },
        )
        .unwrap_err();
        assert!(huge_interaction_err.contains("budget"));
    }

    #[test]
    fn a2ui_basic_catalog_parity_and_revision_cap() {
        let store = Store::open_in_memory().unwrap();
        let (_, nodes) = build_nodes_from_spec(&json!({
            "root_id": "tabs",
            "components": [
                { "id": "tabs", "component": "Tabs", "tab_titles": ["A", "B"], "children": ["modal", "dt"] },
                { "id": "modal", "component": "Modal", "trigger": "open_btn", "content": "slider" },
                { "id": "open_btn", "component": "Button", "text": "Open" },
                { "id": "slider", "component": "Slider", "min": 0, "max": 100, "step": 5 },
                { "id": "dt", "component": "DateTimeInput", "label": "When" },
                { "id": "div", "component": "Divider", "axis": "vertical" },
                { "id": "cp", "component": "ChoicePicker", "displayStyle": "chips", "filterable": true }
            ]
        }))
        .unwrap();
        assert_eq!(nodes["tabs"].slots["tab:0"], vec!["modal"]);
        assert_eq!(nodes["tabs"].slots["tab:1"], vec!["dt"]);
        assert_eq!(nodes["modal"].slots["trigger"], vec!["open_btn"]);
        assert_eq!(nodes["modal"].slots["content"], vec!["slider"]);
        assert_eq!(nodes["slider"].props["steps"], json!(5));
        assert_eq!(nodes["dt"].props["enableDate"], json!(true));
        assert_eq!(nodes["div"].props["axis"], json!("vertical"));
        assert_eq!(nodes["cp"].props["displayStyle"], json!("chips"));
        assert_eq!(nodes["cp"].props["filterable"], json!(true));

        let bad_nodes = build_nodes_from_spec(&json!({
            "nodes": {
                "x": {
                    "instance_id": "x",
                    "component_id": "x",
                    "component": "EvilScriptTag",
                    "props": {},
                    "slots": {},
                    "bindings": {},
                    "actions": {},
                    "validation_errors": []
                }
            }
        }));
        assert!(bad_nodes.is_err());

        for _ in 0..25 {
            upsert_ui_surface(
                &store,
                test_owner("conv-rev"),
                "surf",
                "summary",
                Some("tabs".into()),
                nodes.clone(),
                json!({}),
            )
            .unwrap();
        }
        let key = surface_storage_key("conv-rev", "surf");
        let stored: StoredUiSurface = store
            .get_module_record(KIND_UI_SURFACE, &key)
            .unwrap()
            .unwrap();
        assert_eq!(stored.revisions.len(), 20);
        assert_eq!(*stored.revisions.keys().next().unwrap(), 6);
        assert_eq!(*stored.revisions.keys().last().unwrap(), 25);
    }
}
