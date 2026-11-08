// SPDX-License-Identifier: Apache-2.0
//! Bounded JSON Pointer updates. Mutation is prepared on a private candidate.
use super::DataMutation;
use nola_domain::generative_ui::{UiError, UI_MAX_DEPTH, UI_MAX_SURFACE_BYTES};
use serde_json::{Map, Value};

pub fn pointer_tokens(path: &str) -> Result<Vec<String>, UiError> {
    if path == "/" || path.is_empty() {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') || path.len() > 4096 {
        return Err(UiError::InvalidBinding);
    }
    let mut tokens = Vec::new();
    for token in path[1..].split('/') {
        let mut decoded = String::new();
        let mut chars = token.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                decoded.push(match chars.next() {
                    Some('0') => '~',
                    Some('1') => '/',
                    _ => return Err(UiError::InvalidBinding),
                });
            } else {
                decoded.push(ch);
            }
        }
        tokens.push(decoded);
    }
    if tokens.len() > UI_MAX_DEPTH {
        return Err(UiError::ResourceLimit);
    }
    Ok(tokens)
}

pub fn absolute_pointer(scope: &str, path: &str) -> Result<String, UiError> {
    let absolute = if path.starts_with('/') {
        path.to_owned()
    } else if path.is_empty() {
        scope.to_owned()
    } else {
        format!("{}/{}", if scope == "/" { "" } else { scope }, path)
    };
    pointer_tokens(&absolute)?;
    Ok(absolute)
}

pub fn read_pointer<'a>(data: &'a Value, path: &str) -> Result<Option<&'a Value>, UiError> {
    let tokens = pointer_tokens(path)?;
    let mut current = data;
    for token in tokens {
        current = match current {
            Value::Object(object) => match object.get(&token) {
                Some(value) => value,
                None => return Ok(None),
            },
            Value::Array(array) => match array.get(array_index(&token)?) {
                Some(value) => value,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
    }
    Ok(Some(current))
}

fn array_index(token: &str) -> Result<usize, UiError> {
    if token.is_empty() || (token.starts_with('0') && token.len() > 1)
        || !token.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(UiError::InvalidBinding);
    }
    token.parse().map_err(|_| UiError::InvalidBinding)
}

pub fn check_data_budget(data: &Value) -> Result<(), UiError> {
    let mut pending = vec![(data, 0usize)];
    let mut count = 0usize;
    while let Some((value, depth)) = pending.pop() {
        count += 1;
        if depth > UI_MAX_DEPTH || count > 65536 {
            return Err(UiError::ResourceLimit);
        }
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => pending.extend(values.values().map(|value| (value, depth + 1))),
            _ => {}
        }
    }
    if serde_json::to_vec(data).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_SURFACE_BYTES {
        return Err(UiError::ResourceLimit);
    }
    Ok(())
}

pub fn update_data(data: &Value, path: &str, mutation: &DataMutation) -> Result<Value, UiError> {
    update_data_batch(data, std::iter::once((path, mutation)))
}

/// Clone and validate the complete candidate once per atomic edit batch. The
/// caller orders non-overlapping mutations resolved from its original snapshot.
pub fn update_data_batch<'a>(
    data: &Value,
    mutations: impl IntoIterator<Item = (&'a str, &'a DataMutation)>,
) -> Result<Value, UiError> {
    let mut candidate = data.clone();
    for (path, mutation) in mutations {
        let tokens = pointer_tokens(path)?;
        if let DataMutation::Set(value) = mutation { check_data_budget(value)?; }
        update_at(&mut candidate, &tokens, mutation)?;
    }
    check_data_budget(&candidate)?;
    Ok(candidate)
}

fn update_at(current: &mut Value, tokens: &[String], mutation: &DataMutation) -> Result<(), UiError> {
    let Some((token, remaining)) = tokens.split_first() else {
        *current = match mutation {
            DataMutation::Set(value) => value.clone(),
            DataMutation::Delete => Value::Object(Map::new()),
        };
        return Ok(());
    };
    if current.is_null() && matches!(mutation, DataMutation::Set(_)) {
        *current = Value::Object(Map::new());
    }
    match current {
        Value::Object(object) => {
            if remaining.is_empty() {
                match mutation {
                    DataMutation::Set(value) => { object.insert(token.clone(), value.clone()); }
                    DataMutation::Delete => { object.remove(token); }
                }
            } else if let Some(child) = object.get_mut(token) {
                update_at(child, remaining, mutation)?;
            } else if matches!(mutation, DataMutation::Set(_)) {
                let mut child = Value::Object(Map::new());
                update_at(&mut child, remaining, mutation)?;
                object.insert(token.clone(), child);
            }
        }
        Value::Array(array) => {
            let index = array_index(token)?;
            if index >= array.len() {
                return Err(UiError::InvalidBinding);
            }
            if remaining.is_empty() {
                match mutation {
                    DataMutation::Set(value) => array[index] = value.clone(),
                    DataMutation::Delete => { array.remove(index); }
                }
            } else {
                update_at(&mut array[index], remaining, mutation)?;
            }
        }
        _ => return Err(UiError::InvalidBinding),
    }
    Ok(())
}

/// Both paths are admitted absolute pointers; '/' and '' denote the root.
pub fn overlapping_paths(left: &str, right: &str) -> bool {
    left == right || left.is_empty() || right.is_empty() || left == "/" || right == "/"
        || left.strip_prefix(right).is_some_and(|suffix| suffix.starts_with('/'))
        || right.strip_prefix(left).is_some_and(|suffix| suffix.starts_with('/'))
}
