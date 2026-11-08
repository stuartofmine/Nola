// SPDX-License-Identifier: Apache-2.0
//! Date-only/time-only values are wall-clock values; date-time is an instant.
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use nola_domain::generative_ui::{UiError, UiNodeSnapshot};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(PartialEq, PartialOrd)]
enum DateValue {
    Date(NaiveDate),
    Time(NaiveTime),
    Instant(DateTime<FixedOffset>),
}

fn parse(text: &str, date: bool, time: bool) -> Result<DateValue, UiError> {
    let invalid = || UiError::InvalidBinding;
    match (date, time) {
        (true, true) => DateTime::parse_from_rfc3339(text).map(DateValue::Instant).map_err(|_| invalid()),
        (true, false) if text.len() == 10 => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(DateValue::Date).map_err(|_| invalid()),
        (false, true) => NaiveTime::parse_from_str(text, "%H:%M:%S%.f")
            .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M"))
            .map(DateValue::Time).map_err(|_| invalid()),
        _ => Err(invalid()),
    }
}

pub fn validate(node: &UiNodeSnapshot, value: &str) -> Result<(), UiError> {
    let date = node.props.get("enableDate").and_then(Value::as_bool).unwrap_or(false);
    let time = node.props.get("enableTime").and_then(Value::as_bool).unwrap_or(false);
    let bound = |name: &str| -> Result<Option<DateValue>, UiError> {
        match node.props.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => parse(text, date, time).map(Some),
            _ => Err(UiError::InvalidBinding),
        }
    };
    let minimum = bound("min")?;
    let maximum = bound("max")?;
    if minimum.as_ref().zip(maximum.as_ref()).is_some_and(|(min, max)| min > max) {
        return Err(UiError::InvalidBinding);
    }
    // Optional empty value is part of v0.9.1; required is an explicit check.
    if value.is_empty() { return Ok(()); }
    let parsed = parse(value, date, time)?;
    if minimum.as_ref().is_some_and(|min| parsed.partial_cmp(min) == Some(Ordering::Less))
        || maximum.as_ref().is_some_and(|max| parsed.partial_cmp(max) == Some(Ordering::Greater)) {
        return Err(UiError::InvalidBinding);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(date: bool, time: bool, min: Value, max: Value) -> UiNodeSnapshot {
        UiNodeSnapshot {
            instance_id: "date".into(), component_id: "date".into(),
            component: "DateTimeInput".into(),
            props: [("enableDate".into(), json!(date)), ("enableTime".into(), json!(time)),
                ("min".into(), min), ("max".into(), max)].into(),
            slots: Default::default(), bindings: Default::default(),
            actions: Default::default(), validation_errors: vec![],
        }
    }

    #[test]
    fn instant_bounds_compare_instants_across_offsets() {
        let field = node(true, true, json!("2026-10-21T10:00:00+08:00"), json!("2026-10-21T03:00:00Z"));
        assert!(validate(&field, "2026-10-20T19:00:00-07:00").is_ok());
        assert!(validate(&field, "2026-10-21T11:00:00+08:00").is_ok());
        assert!(validate(&field, "2026-10-21T11:00:00.001+08:00").is_err());
        assert!(validate(&field, "2026-10-21T10:30:00").is_err());
    }

    #[test]
    fn wall_clock_values_keep_precision_and_calendar_validation() {
        let time = node(false, true, json!("09:00:00.125"), json!("09:00:00.250"));
        assert!(validate(&time, "09:00:00.200").is_ok());
        assert!(validate(&time, "09:00:00.124").is_err());
        let date = node(true, false, Value::Null, Value::Null);
        assert!(validate(&date, "2028-02-29").is_ok());
        assert!(validate(&date, "2026-02-29").is_err());
        assert!(validate(&date, "2026-10-21T00:00:00Z").is_err());
        assert!(validate(&date, "").is_ok());
    }

    #[test]
    fn malformed_or_reversed_bounds_are_rejected_even_when_empty() {
        let reversed = node(true, false, json!("2026-10-22"), json!("2026-10-21"));
        assert!(validate(&reversed, "").is_err());
        let malformed = node(true, true, json!("tomorrow"), Value::Null);
        assert!(validate(&malformed, "").is_err());
    }
}
