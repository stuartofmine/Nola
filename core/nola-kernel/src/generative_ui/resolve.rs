// SPDX-License-Identifier: Apache-2.0
//! Pure bound-value evaluation, with a shared operation budget per projection.
use super::data::{absolute_pointer, read_pointer};
use nola_domain::generative_ui::{UiError, UI_MAX_DEPTH};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::{cell::Cell, rc::Rc};

pub use super::schema::SUPPORTED_FUNCTIONS as PURE_FUNCTIONS;

pub struct Evaluator<'a> {
    pub data: &'a Value,
    remaining: Rc<Cell<usize>>,
    pub dependencies: BTreeSet<String>,
}

impl<'a> Evaluator<'a> {
    pub fn new(data: &'a Value) -> Self {
        Self::with_budget(data, Rc::new(Cell::new(10000)))
    }

    pub(crate) fn with_budget(data: &'a Value, remaining: Rc<Cell<usize>>) -> Self {
        Self { data, remaining, dependencies: BTreeSet::new() }
    }

    pub fn resolve(&mut self, value: &Value, scope: &str) -> Result<Value, UiError> {
        self.resolve_at(value, scope, 0)
    }

    fn resolve_at(&mut self, value: &Value, scope: &str, depth: usize) -> Result<Value, UiError> {
        if self.remaining.get() == 0 || depth > UI_MAX_DEPTH { return Err(UiError::ResourceLimit); }
        self.remaining.set(self.remaining.get() - 1);
        match value {
            Value::Object(object) if object.len() == 1 && object.contains_key("path") => {
                let path = object["path"].as_str().ok_or(UiError::InvalidBinding)?;
                let path = absolute_pointer(scope, path)?;
                self.dependencies.insert(path.clone());
                Ok(read_pointer(self.data, &path)?.cloned().unwrap_or(Value::Null))
            }
            Value::Object(object) if object.contains_key("call") => {
                let name = object["call"].as_str().ok_or(UiError::InvalidMessage)?;
                if !PURE_FUNCTIONS.contains(&name) { return Err(UiError::UnsupportedFunction); }
                let args = object.get("args").and_then(Value::as_object).ok_or(UiError::InvalidMessage)?;
                let mut resolved = Map::new();
                for (key, value) in args {
                    resolved.insert(key.clone(), self.resolve_at(value, scope, depth + 1)?);
                }
                let value = evaluate(name, &resolved)?;
                if let Some(return_type) = object.get("returnType").and_then(Value::as_str) {
                    if return_type != "boolean" && return_type != "any" {
                        return Err(UiError::InvalidBinding);
                    }
                }
                Ok(value)
            }
            Value::Object(object) => {
                let mut result = Map::new();
                for (key, value) in object {
                    result.insert(key.clone(), self.resolve_at(value, scope, depth + 1)?);
                }
                Ok(Value::Object(result))
            }
            Value::Array(values) => values.iter().map(|value| self.resolve_at(value, scope, depth + 1))
                .collect::<Result<Vec<_>, _>>().map(Value::Array),
            _ => Ok(value.clone()),
        }
    }
}

fn evaluate(name: &str, args: &Map<String, Value>) -> Result<Value, UiError> {
    let value = args.get("value").unwrap_or(&Value::Null);
    let result = match name {
        "required" => match value {
            Value::Null => false,
            Value::String(value) => !value.is_empty(),
            Value::Array(value) => !value.is_empty(),
            _ => true,
        },
        "regex" => {
            let pattern = args.get("pattern").and_then(Value::as_str).ok_or(UiError::InvalidBinding)?;
            let regex = bounded_regex(pattern)?;
            value.as_str().is_some_and(|value| regex.is_match(value))
        }
        "length" => {
            let length = match value.as_str() { Some(value) => value.chars().count() as u64, None => return Ok(Value::Bool(false)) };
            args.get("min").and_then(Value::as_u64).is_none_or(|min| length >= min)
                && args.get("max").and_then(Value::as_u64).is_none_or(|max| length <= max)
        }
        "numeric" => {
            let number = match value.as_f64() { Some(number) => number, None => return Ok(Value::Bool(false)) };
            args.get("min").and_then(Value::as_f64).is_none_or(|min| number >= min)
                && args.get("max").and_then(Value::as_f64).is_none_or(|max| number <= max)
        }
        "and" | "or" => {
            let values = args.get("values").and_then(Value::as_array).ok_or(UiError::InvalidBinding)?;
            let values = values.iter().map(|value| value.as_bool().ok_or(UiError::InvalidBinding))
                .collect::<Result<Vec<_>, _>>()?;
            if name == "and" { values.into_iter().all(|value| value) } else { values.into_iter().any(|value| value) }
        }
        "not" => !value.as_bool().ok_or(UiError::InvalidBinding)?,
        _ => return Err(UiError::UnsupportedFunction),
    };
    Ok(Value::Bool(result))
}


pub(crate) fn bounded_regex(pattern: &str) -> Result<regex::Regex, UiError> {
    if pattern.len() > 4096 { return Err(UiError::ResourceLimit); }
    regex::RegexBuilder::new(pattern).size_limit(256 * 1024)
        .build().map_err(|_| UiError::InvalidBinding)
}
