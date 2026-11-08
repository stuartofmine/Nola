// SPDX-License-Identifier: Apache-2.0
//! Pure occurrence calculation. Hosts register returned instants; they do not
//! reinterpret recurrence, DST, or the identity of a missed occurrence.
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceQuery {
    pub automation_id: String,
    pub timezone: String,
    pub frequency: String,
    pub time: Option<String>,
    /// ISO Monday=1, Sunday=7. Transport adapters convert their existing format.
    pub weekday: Option<u32>,
    pub once_at_ms: Option<i64>,
    /// Exclusive/inclusive interval supports both strict-future and catch-up.
    pub after_ms: i64,
    pub through_ms: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Occurrence {
    pub scheduled_for_ms: i64,
    pub occurrence_key: String,
    pub local_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OccurrencePage {
    pub occurrences: Vec<Occurrence>,
    pub next_after_ms: Option<i64>,
}

fn instant(zone: Tz, local: NaiveDateTime) -> Result<DateTime<Tz>, String> {
    match zone.from_local_datetime(&local) {
        LocalResult::Single(value) => Ok(value),
        LocalResult::Ambiguous(first, second) => Ok(first.min(second)),
        LocalResult::None => {
            // Resolve a missing wall clock by its actual offset gap, retaining
            // minutes (02:30 -> 03:30). This also handles a skipped local day.
            let before = zone.from_local_datetime(&(local - Duration::days(2))).earliest()
                .ok_or("Cannot resolve timezone before transition")?;
            let after = zone.from_local_datetime(&(local + Duration::days(2))).earliest()
                .ok_or("Cannot resolve timezone after transition")?;
            let gap = after.offset().fix().local_minus_utc() - before.offset().fix().local_minus_utc();
            if gap <= 0 { return Err("Cannot resolve timezone gap".into()); }
            zone.from_local_datetime(&(local + Duration::seconds(i64::from(gap)))).earliest()
                .ok_or_else(|| "Cannot resolve shifted occurrence".into())
        }
    }
}

pub fn query(input: &OccurrenceQuery) -> Result<OccurrencePage, String> {
    if input.automation_id.is_empty() || input.automation_id.len() > 512
        || input.automation_id.chars().any(char::is_control)
        || !(1..=64).contains(&input.limit) || input.through_ms < input.after_ms
        || input.through_ms.saturating_sub(input.after_ms) > 366 * 86_400_000 {
        return Err("Invalid bounded automation occurrence query".into());
    }
    let zone_name=if input.timezone=="system" {std::env::var("TZ").ok().filter(|value|value.parse::<Tz>().is_ok())
        .map(Ok).unwrap_or_else(iana_time_zone::get_timezone).map_err(|_|"System IANA timezone unavailable")?}else{input.timezone.clone()};
    let zone: Tz = zone_name.parse().map_err(|_| "Unknown IANA timezone")?;
    let after = DateTime::<Utc>::from_timestamp_millis(input.after_ms).ok_or("Invalid occurrence start")?;
    let through = DateTime::<Utc>::from_timestamp_millis(input.through_ms).ok_or("Invalid occurrence end")?;
    let mut candidates = Vec::new();
    if input.frequency == "once" {
        candidates.push(DateTime::<Utc>::from_timestamp_millis(input.once_at_ms.ok_or("Missing one-time date")?)
            .ok_or("Invalid one-time date")?.with_timezone(&zone));
    } else {
        if !matches!(input.frequency.as_str(), "hourly" | "daily" | "weekly") {
            return Err("Unsupported automation recurrence".into());
        }
        let clock=input.time.as_deref().ok_or("Missing recurrence time")?;
        let time=clock.parse::<NaiveTime>().or_else(|_|NaiveTime::parse_from_str(clock,"%H:%M"))
            .map_err(|_|"Invalid recurrence time")?;
        if input.frequency == "weekly" && !input.weekday.is_some_and(|day| (1..=7).contains(&day)) {
            return Err("Invalid ISO recurrence weekday".into());
        }
        let mut date = after.with_timezone(&zone).date_naive() - Duration::days(1);
        let end = through.with_timezone(&zone).date_naive();
        while date <= end {
            if input.frequency != "weekly" || Some(date.weekday().number_from_monday()) == input.weekday {
                if input.frequency == "hourly" {
                    use chrono::Timelike;
                    for hour in 0..24 {
                        candidates.push(instant(zone, date.and_time(time.with_hour(hour).unwrap()))?);
                    }
                } else { candidates.push(instant(zone, date.and_time(time))?); }
            }
            date = date.succ_opt().ok_or("Occurrence date overflow")?;
        }
    }
    candidates.retain(|value| value.timestamp_millis() > input.after_ms && value.timestamp_millis() <= input.through_ms);
    candidates.sort();
    candidates.dedup_by_key(|value| value.timestamp_millis());
    let next_after_ms = (candidates.len() > input.limit).then(|| candidates[input.limit - 1].timestamp_millis());
    Ok(OccurrencePage { occurrences: candidates.into_iter().take(input.limit).map(|value| {
        let scheduled_for_ms = value.timestamp_millis();
        Occurrence { scheduled_for_ms, occurrence_key: format!("{}:{scheduled_for_ms}", input.automation_id),
            local_date: value.format("%Y-%m-%d").to_string() }
    }).collect(), next_after_ms })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(value: &str) -> i64 { DateTime::parse_from_rfc3339(value).unwrap().timestamp_millis() }
    fn input(from: &str, through: &str) -> OccurrenceQuery {
        OccurrenceQuery { automation_id:"fixture".into(),timezone:"America/New_York".into(),frequency:"daily".into(),
            time:Some("02:30".into()),weekday:None,once_at_ms:None,after_ms:ms(from),through_ms:ms(through),limit:1 }
    }
    #[test]
    fn gap_shifts_by_offset_and_fold_selects_first_instant() {
        let gap=query(&input("2026-03-08T00:00:00Z","2026-03-09T00:00:00Z")).unwrap();
        assert_eq!(gap.occurrences[0].scheduled_for_ms,ms("2026-03-08T07:30:00Z"));
        let mut fold=input("2026-11-01T00:00:00Z","2026-11-02T00:00:00Z");fold.time=Some("01:30".into());
        assert_eq!(query(&fold).unwrap().occurrences[0].scheduled_for_ms,ms("2026-11-01T05:30:00Z"));
    }
    #[test]
    fn catch_up_pages_and_strict_future_preserve_occurrence_identity() {
        let mut query_input=input("2026-05-01T00:00:00Z","2026-05-04T00:00:00Z");
        let first=query(&query_input).unwrap();query_input.after_ms=first.next_after_ms.unwrap();
        let second=query(&query_input).unwrap();
        assert!(second.occurrences[0].scheduled_for_ms>first.occurrences[0].scheduled_for_ms);
        assert_eq!(first.occurrences[0].occurrence_key,format!("fixture:{}",first.occurrences[0].scheduled_for_ms));
    }
}
