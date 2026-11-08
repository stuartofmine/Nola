// SPDX-License-Identifier: Apache-2.0
//! Persistent personal intelligence state shared by every Nola client.
//!
//! The model may propose events, memories, and outcomes. This module owns the
//! deterministic lifecycle: dedupe, reinforcement, conflict handling, recall,
//! bounded retention, and strategy scoring.

use crate::persistence::{
    config_file_exists, read_config_file, remove_config_file, write_config_file,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
use std::sync::{LazyLock, Mutex};
use uuid::Uuid;

const STATE_FILE: &str = "personal_intelligence.json";
const SCHEMA_VERSION: u32 = 1;
/// Version of the native intelligence request/response contract, not the Agent
/// reducer or Application command protocol. Persistence schema is independent.
const INTELLIGENCE_ABI_VERSION: u32 = 1;

#[no_mangle]
pub extern "C" fn nola_core_intelligence_abi_version() -> u32 {
    INTELLIGENCE_ABI_VERSION
}

const DEFAULT_MAX_EVENTS: usize = 5_000;
const DEFAULT_MAX_MEMORIES: usize = 1_200;
const DEFAULT_MAX_OUTCOMES: usize = 1_000;
const DEFAULT_RECALL_CONTEXT_CHARS: usize = 8_000;
const MAX_RECALLED_STATEMENT_CHARS: usize = 1_200;

static INTELLIGENCE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
}

impl Default for MemoryKind {
    fn default() -> Self {
        Self::Semantic
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    Disputed,
}

impl Default for MemoryStatus {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub occurred_at: String,
    #[serde(default)]
    pub observed_at: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default = "default_half")]
    pub importance: f64,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dedupe_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryCandidate {
    #[serde(default)]
    pub semantic_key: Option<String>,
    #[serde(default)]
    pub kind: MemoryKind,
    #[serde(default)]
    pub statement: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub source_event_ids: Vec<String>,
    #[serde(default)]
    pub observed_at: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub semantic_key: String,
    pub kind: MemoryKind,
    pub statement: String,
    pub confidence: f64,
    pub first_seen_at: String,
    pub last_verified_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub use_count: u64,
    #[serde(default)]
    pub source_event_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub status: MemoryStatus,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub conflicting_statements: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskOutcome {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub task_type: String,
    #[serde(default)]
    pub route: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub user_corrected: bool,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub occurred_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrategyStat {
    pub signature: String,
    pub task_type: String,
    pub route: String,
    pub model: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub attempts: u64,
    #[serde(default)]
    pub successes: u64,
    #[serde(default)]
    pub corrections: u64,
    #[serde(default)]
    pub retries: u64,
    #[serde(default)]
    pub average_latency_ms: f64,
    #[serde(default)]
    pub score: f64,
    pub last_updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntelligenceState {
    pub schema_version: u32,
    #[serde(default)]
    pub events: Vec<PersonalEvent>,
    #[serde(default)]
    pub memories: Vec<MemoryRecord>,
    #[serde(default)]
    pub outcomes: Vec<TaskOutcome>,
    #[serde(default)]
    pub strategies: Vec<StrategyStat>,
    #[serde(default)]
    pub updated_at: String,
}

impl Default for IntelligenceState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            events: Vec::new(),
            memories: Vec::new(),
            outcomes: Vec::new(),
            strategies: Vec::new(),
            updated_at: now_iso(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IntelligenceIngestInput {
    #[serde(default)]
    pub events: Vec<PersonalEvent>,
    #[serde(default)]
    pub memory_candidates: Vec<MemoryCandidate>,
    #[serde(default)]
    pub outcomes: Vec<TaskOutcome>,
    #[serde(default)]
    pub max_events: Option<usize>,
    #[serde(default)]
    pub max_memories: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryChange {
    pub semantic_key: String,
    pub action: String,
    pub memory_id: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateSummary {
    pub event_count: usize,
    pub active_memory_count: usize,
    pub superseded_memory_count: usize,
    pub outcome_count: usize,
    pub strategy_count: usize,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntelligenceIngestResponse {
    pub ok: bool,
    pub inserted_events: usize,
    pub duplicate_events: usize,
    pub memory_changes: Vec<MemoryChange>,
    pub updated_strategies: Vec<String>,
    pub insights: Vec<String>,
    pub summary: StateSummary,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecallInput {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub kinds: Vec<MemoryKind>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub minimum_score: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecalledMemory {
    pub memory: MemoryRecord,
    pub score: f64,
    pub relevance: f64,
    pub reason: String,
    pub evidence: Vec<PersonalEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecallResponse {
    pub ok: bool,
    pub query: String,
    pub memories: Vec<RecalledMemory>,
    pub context: String,
}

fn default_half() -> f64 {
    0.5
}

fn default_confidence() -> f64 {
    0.7
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn clamp_unit(value: f64) -> f64 {
    value.max(0.0).min(1.0)
}

fn normalized_text(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn semantic_key(candidate: &MemoryCandidate) -> String {
    candidate
        .semantic_key
        .as_deref()
        .map(normalized_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let digest = md5::compute(normalized_text(&candidate.statement).as_bytes());
            format!("memory:{digest:x}")
        })
}

fn event_dedupe_key(event: &PersonalEvent) -> String {
    if let Some(key) = event
        .dedupe_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        return key.to_string();
    }
    let raw = format!(
        "{}|{}|{}|{}|{}",
        event.source,
        event.kind,
        event.subject.as_deref().unwrap_or_default(),
        event.occurred_at,
        normalized_text(&event.summary)
    );
    format!("{:x}", md5::compute(raw.as_bytes()))
}

fn merge_unique(target: &mut Vec<String>, additions: impl IntoIterator<Item = String>) {
    let mut seen: HashSet<String> = target.iter().cloned().collect();
    for value in additions {
        if !value.is_empty() && seen.insert(value.clone()) {
            target.push(value);
        }
    }
}

fn load_state() -> Result<IntelligenceState, String> {
    if !config_file_exists(STATE_FILE)? {
        return Ok(IntelligenceState::default());
    }
    let raw = read_config_file(STATE_FILE)?;
    let state: IntelligenceState = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid personal intelligence state: {error}"))?;
    if state.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported personal intelligence schema {}",
            state.schema_version
        ));
    }
    Ok(state)
}

fn save_state(state: &IntelligenceState) -> Result<(), String> {
    let raw = serde_json::to_string(state).map_err(|error| error.to_string())?;
    write_config_file(STATE_FILE, &raw)
}

fn summarize(state: &IntelligenceState) -> StateSummary {
    StateSummary {
        event_count: state.events.len(),
        active_memory_count: state
            .memories
            .iter()
            .filter(|memory| memory.status == MemoryStatus::Active)
            .count(),
        superseded_memory_count: state
            .memories
            .iter()
            .filter(|memory| memory.status == MemoryStatus::Superseded)
            .count(),
        outcome_count: state.outcomes.len(),
        strategy_count: state.strategies.len(),
        updated_at: state.updated_at.clone(),
    }
}

fn ingest_into_state(
    state: &mut IntelligenceState,
    input: IntelligenceIngestInput,
) -> IntelligenceIngestResponse {
    let now = now_iso();
    let mut inserted_events = 0;
    let mut duplicate_events = 0;
    let mut memory_changes = Vec::new();
    let mut updated_strategies = Vec::new();
    let mut insights = Vec::new();
    let mut known_event_keys: HashSet<String> = state.events.iter().map(event_dedupe_key).collect();
    let mut inserted_by_source: HashMap<String, usize> = HashMap::new();

    for mut event in input.events {
        if event.summary.trim().is_empty() {
            continue;
        }
        if event.id.trim().is_empty() {
            event.id = Uuid::new_v4().to_string();
        }
        if event.occurred_at.trim().is_empty() {
            event.occurred_at = now.clone();
        }
        if event.observed_at.trim().is_empty() {
            event.observed_at = now.clone();
        }
        event.importance = clamp_unit(event.importance);
        event.confidence = clamp_unit(event.confidence);
        let key = event_dedupe_key(&event);
        event.dedupe_key = Some(key.clone());
        if !known_event_keys.insert(key) {
            duplicate_events += 1;
            continue;
        }
        *inserted_by_source.entry(event.source.clone()).or_insert(0) += 1;
        state.events.push(event);
        inserted_events += 1;
    }

    for candidate in input.memory_candidates {
        let statement = candidate.statement.trim().to_string();
        if statement.is_empty() {
            continue;
        }
        let key = semantic_key(&candidate);
        let observed_at = if candidate.observed_at.trim().is_empty() {
            now.clone()
        } else {
            candidate.observed_at.clone()
        };
        let normalized_statement = normalized_text(&statement);
        let active_index = state
            .memories
            .iter()
            .position(|memory| memory.semantic_key == key && memory.status == MemoryStatus::Active);

        if let Some(index) = active_index {
            let same_statement =
                normalized_text(&state.memories[index].statement) == normalized_statement;
            if same_statement {
                let memory = &mut state.memories[index];
                let evidence = clamp_unit(candidate.confidence) * 0.45;
                memory.confidence = clamp_unit(1.0 - (1.0 - memory.confidence) * (1.0 - evidence));
                memory.last_verified_at = observed_at;
                merge_unique(&mut memory.source_event_ids, candidate.source_event_ids);
                merge_unique(&mut memory.tags, candidate.tags);
                memory_changes.push(MemoryChange {
                    semantic_key: key,
                    action: "reinforced".to_string(),
                    memory_id: Some(memory.id.clone()),
                    detail: "Existing memory was independently observed again.".to_string(),
                });
                continue;
            }

            let candidate_confidence = clamp_unit(candidate.confidence);
            let existing_confidence = state.memories[index].confidence;
            state.memories[index]
                .conflicting_statements
                .push(statement.clone());
            if candidate_confidence + 0.08 >= existing_confidence {
                let superseded_id = state.memories[index].id.clone();
                state.memories[index].status = MemoryStatus::Superseded;
                let new_id = Uuid::new_v4().to_string();
                state.memories.push(MemoryRecord {
                    id: new_id.clone(),
                    semantic_key: key.clone(),
                    kind: candidate.kind,
                    statement,
                    confidence: candidate_confidence,
                    first_seen_at: observed_at.clone(),
                    last_verified_at: observed_at,
                    last_used_at: None,
                    use_count: 0,
                    source_event_ids: candidate.source_event_ids,
                    tags: candidate.tags,
                    status: MemoryStatus::Active,
                    supersedes: Some(superseded_id),
                    conflicting_statements: Vec::new(),
                });
                memory_changes.push(MemoryChange {
                    semantic_key: key.clone(),
                    action: "superseded".to_string(),
                    memory_id: Some(new_id),
                    detail: "Newer evidence replaced a conflicting active memory.".to_string(),
                });
                insights.push(format!("Memory changed: {key}"));
            } else {
                state.memories[index].status = MemoryStatus::Disputed;
                memory_changes.push(MemoryChange {
                    semantic_key: key.clone(),
                    action: "disputed".to_string(),
                    memory_id: Some(state.memories[index].id.clone()),
                    detail: "Conflicting evidence was retained for later verification.".to_string(),
                });
                insights.push(format!("Memory needs verification: {key}"));
            }
            continue;
        }

        let id = Uuid::new_v4().to_string();
        state.memories.push(MemoryRecord {
            id: id.clone(),
            semantic_key: key.clone(),
            kind: candidate.kind,
            statement,
            confidence: clamp_unit(candidate.confidence),
            first_seen_at: observed_at.clone(),
            last_verified_at: observed_at,
            last_used_at: None,
            use_count: 0,
            source_event_ids: candidate.source_event_ids,
            tags: candidate.tags,
            status: MemoryStatus::Active,
            supersedes: None,
            conflicting_statements: Vec::new(),
        });
        memory_changes.push(MemoryChange {
            semantic_key: key,
            action: "created".to_string(),
            memory_id: Some(id),
            detail: "A new structured memory was created.".to_string(),
        });
    }

    for mut outcome in input.outcomes {
        if outcome.id.trim().is_empty() {
            outcome.id = Uuid::new_v4().to_string();
        }
        if outcome.occurred_at.trim().is_empty() {
            outcome.occurred_at = now.clone();
        }
        outcome.tools.sort();
        outcome.tools.dedup();
        let signature = strategy_signature(&outcome);
        update_strategy(state, &outcome, &signature, &now);
        updated_strategies.push(signature);
        state.outcomes.push(outcome);
    }

    for (source, count) in inserted_by_source {
        if count >= 3 {
            insights.push(format!("{source} contributed {count} new observations."));
        }
    }

    cap_state(
        state,
        input.max_events.unwrap_or(DEFAULT_MAX_EVENTS),
        input.max_memories.unwrap_or(DEFAULT_MAX_MEMORIES),
    );
    state.schema_version = SCHEMA_VERSION;
    state.updated_at = now;

    IntelligenceIngestResponse {
        ok: true,
        inserted_events,
        duplicate_events,
        memory_changes,
        updated_strategies,
        insights,
        summary: summarize(state),
    }
}

fn strategy_signature(outcome: &TaskOutcome) -> String {
    format!(
        "{}|{}|{}|{}",
        normalized_text(&outcome.task_type),
        normalized_text(&outcome.route),
        normalized_text(&outcome.model),
        outcome.tools.join(",")
    )
}

fn update_strategy(
    state: &mut IntelligenceState,
    outcome: &TaskOutcome,
    signature: &str,
    now: &str,
) {
    let index = state
        .strategies
        .iter()
        .position(|strategy| strategy.signature == signature);
    let strategy = if let Some(index) = index {
        &mut state.strategies[index]
    } else {
        state.strategies.push(StrategyStat {
            signature: signature.to_string(),
            task_type: outcome.task_type.clone(),
            route: outcome.route.clone(),
            model: outcome.model.clone(),
            tools: outcome.tools.clone(),
            attempts: 0,
            successes: 0,
            corrections: 0,
            retries: 0,
            average_latency_ms: 0.0,
            score: 0.0,
            last_updated_at: now.to_string(),
        });
        state.strategies.last_mut().expect("strategy was inserted")
    };

    let previous_attempts = strategy.attempts;
    strategy.attempts += 1;
    if outcome.success {
        strategy.successes += 1;
    }
    if outcome.user_corrected {
        strategy.corrections += 1;
    }
    strategy.retries += outcome.retry_count as u64;
    strategy.average_latency_ms = if previous_attempts == 0 {
        outcome.latency_ms as f64
    } else {
        ((strategy.average_latency_ms * previous_attempts as f64) + outcome.latency_ms as f64)
            / strategy.attempts as f64
    };
    let posterior_success = (strategy.successes as f64 + 1.0) / (strategy.attempts as f64 + 2.0);
    let correction_penalty = strategy.corrections as f64 / strategy.attempts as f64 * 0.22;
    let retry_penalty = strategy.retries as f64 / strategy.attempts as f64 * 0.035;
    strategy.score = clamp_unit(posterior_success - correction_penalty - retry_penalty);
    strategy.last_updated_at = now.to_string();
}

fn cap_state(state: &mut IntelligenceState, max_events: usize, max_memories: usize) {
    state
        .events
        .sort_by(|left, right| left.observed_at.cmp(&right.observed_at));
    if state.events.len() > max_events.max(100) {
        let remove_count = state.events.len() - max_events.max(100);
        state.events.drain(0..remove_count);
    }

    if state.memories.len() > max_memories.max(100) {
        state.memories.sort_by(|left, right| {
            memory_retention_score(right)
                .partial_cmp(&memory_retention_score(left))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        state.memories.truncate(max_memories.max(100));
    }

    state
        .outcomes
        .sort_by(|left, right| left.occurred_at.cmp(&right.occurred_at));
    if state.outcomes.len() > DEFAULT_MAX_OUTCOMES {
        let remove_count = state.outcomes.len() - DEFAULT_MAX_OUTCOMES;
        state.outcomes.drain(0..remove_count);
    }
}

fn memory_retention_score(memory: &MemoryRecord) -> f64 {
    let status = match memory.status {
        MemoryStatus::Active => 1.0,
        MemoryStatus::Disputed => 0.55,
        MemoryStatus::Superseded => 0.15,
    };
    status * 0.5
        + memory.confidence * 0.35
        + (memory.use_count as f64 + 1.0).ln().min(5.0) / 5.0 * 0.15
}

fn terms(value: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    for word in value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        result.insert(word.to_string());
    }
    let non_ascii: Vec<char> = value
        .chars()
        .filter(|character| !character.is_ascii() && character.is_alphanumeric())
        .collect();
    for character in &non_ascii {
        result.insert(character.to_string());
    }
    for window in non_ascii.windows(2) {
        result.insert(window.iter().collect());
    }
    for window in non_ascii.windows(3) {
        result.insert(window.iter().collect());
    }
    result
}

fn memory_terms(memory: &MemoryRecord) -> HashSet<String> {
    terms(&format!(
        "{} {} {}",
        memory.semantic_key,
        memory.statement,
        memory.tags.join(" ")
    ))
}

fn term_weight(
    term: &str,
    document_frequency: &HashMap<String, usize>,
    document_count: usize,
) -> f64 {
    let length = term.chars().count();
    let specificity = if term.is_ascii() {
        if length <= 2 {
            0.35
        } else {
            1.0
        }
    } else {
        match length {
            0 => 0.0,
            1 => 0.28,
            2 => 1.0,
            _ => 1.35,
        }
    };
    let frequency = document_frequency.get(term).copied().unwrap_or(0) as f64;
    let idf = (((document_count as f64 + 1.0) / (frequency + 1.0)).ln() + 1.0).clamp(1.0, 3.0);
    specificity * idf
}

fn relevance(
    query: &str,
    query_terms: &HashSet<String>,
    haystack: &HashSet<String>,
    document_frequency: &HashMap<String, usize>,
    document_count: usize,
    memory: &MemoryRecord,
) -> f64 {
    if query_terms.is_empty() {
        return 0.5;
    }
    let total_weight = query_terms
        .iter()
        .map(|term| term_weight(term, document_frequency, document_count))
        .sum::<f64>();
    let matched_weight = query_terms
        .intersection(haystack)
        .map(|term| term_weight(term, document_frequency, document_count))
        .sum::<f64>();
    if matched_weight == 0.0 || total_weight == 0.0 {
        return 0.0;
    }
    let coverage = matched_weight / total_weight;
    let normalized_query = normalized_text(query);
    let normalized_haystack = normalized_text(&format!(
        "{}{}{}",
        memory.semantic_key,
        memory.statement,
        memory.tags.join("")
    ));
    let phrase_match =
        normalized_query.chars().count() >= 4 && normalized_haystack.contains(&normalized_query);
    clamp_unit(coverage * 0.88 + if phrase_match { 0.12 } else { 0.0 })
}

fn term_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f64;
    let union = left.union(right).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn diversify_scored(
    mut candidates: Vec<(usize, f64, f64)>,
    memory_term_sets: &[HashSet<String>],
    limit: usize,
) -> Vec<(usize, f64, f64)> {
    let mut selected = Vec::with_capacity(limit.min(candidates.len()));
    while selected.len() < limit && !candidates.is_empty() {
        let mut best_position = 0;
        let mut best_adjusted = f64::NEG_INFINITY;
        for (position, (index, score, _)) in candidates.iter().enumerate() {
            let redundancy = selected
                .iter()
                .map(|(selected_index, _, _)| {
                    term_similarity(
                        &memory_term_sets[*index],
                        &memory_term_sets[*selected_index],
                    )
                })
                .fold(0.0, f64::max);
            let adjusted = score - redundancy * 0.18;
            if adjusted > best_adjusted {
                best_adjusted = adjusted;
                best_position = position;
            }
        }
        let (index, _, relevance) = candidates.remove(best_position);
        selected.push((index, clamp_unit(best_adjusted), relevance));
    }
    selected
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn recency_score(timestamp: &str) -> f64 {
    let parsed = DateTime::parse_from_rfc3339(timestamp).ok();
    let Some(parsed) = parsed else { return 0.4 };
    let age_days = (Utc::now() - parsed.with_timezone(&Utc)).num_days().max(0) as f64;
    (-age_days / 180.0).exp()
}

fn recall_from_state(state: &mut IntelligenceState, input: RecallInput) -> RecallResponse {
    let query_terms = terms(&input.query);
    let limit = input.limit.unwrap_or(8).max(1).min(32);
    let minimum_score = input.minimum_score.unwrap_or(0.18).max(0.0).min(1.0);
    let event_by_id: HashMap<String, PersonalEvent> = state
        .events
        .iter()
        .map(|event| (event.id.clone(), event.clone()))
        .collect();
    let memory_term_sets = state.memories.iter().map(memory_terms).collect::<Vec<_>>();
    let mut document_frequency = HashMap::new();
    for term_set in &memory_term_sets {
        for term in term_set {
            *document_frequency.entry(term.clone()).or_insert(0usize) += 1;
        }
    }
    let document_count = memory_term_sets.len().max(1);
    let scored: Vec<(usize, f64, f64)> = state
        .memories
        .iter()
        .enumerate()
        .filter(|(_, memory)| memory.status == MemoryStatus::Active)
        .filter(|(_, memory)| input.kinds.is_empty() || input.kinds.contains(&memory.kind))
        .filter(|(_, memory)| {
            input.source.as_ref().map_or(true, |source| {
                memory.source_event_ids.iter().any(|id| {
                    event_by_id
                        .get(id)
                        .map_or(false, |event| event.source == *source)
                })
            })
        })
        .map(|(index, memory)| {
            let relevance = relevance(
                &input.query,
                &query_terms,
                &memory_term_sets[index],
                &document_frequency,
                document_count,
                memory,
            );
            let use_score = ((memory.use_count as f64 + 1.0).ln() / 5.0).min(1.0);
            let score = if query_terms.is_empty() {
                memory.confidence * 0.58
                    + recency_score(&memory.last_verified_at) * 0.27
                    + use_score * 0.15
            } else {
                relevance * 0.72
                    + memory.confidence * 0.16
                    + recency_score(&memory.last_verified_at) * 0.07
                    + use_score * 0.05
            };
            (index, clamp_unit(score), relevance)
        })
        .filter(|(_, _, relevance)| query_terms.is_empty() || *relevance > 0.0)
        .filter(|(_, score, _)| *score >= minimum_score)
        .collect();
    let scored = diversify_scored(scored, &memory_term_sets, limit);

    let now = now_iso();
    let mut recalled = Vec::new();
    for (index, score, relevance) in scored {
        state.memories[index].use_count += 1;
        state.memories[index].last_used_at = Some(now.clone());
        let memory = state.memories[index].clone();
        let evidence = memory
            .source_event_ids
            .iter()
            .filter_map(|id| event_by_id.get(id).cloned())
            .take(4)
            .collect::<Vec<_>>();
        let reason = if relevance >= 0.66 {
            "Strong query match"
        } else if relevance > 0.0 {
            "Related terms plus trusted memory"
        } else {
            "High-confidence recent memory"
        };
        recalled.push(RecalledMemory {
            memory,
            score,
            relevance,
            reason: reason.to_string(),
            evidence,
        });
    }
    state.updated_at = now;
    let mut context_lines = Vec::new();
    let mut context_chars = 0usize;
    for item in &recalled {
        let line = format!(
            "- [{}; confidence {:.0}%; score {:.2}] {}",
            match item.memory.kind {
                MemoryKind::Episodic => "episode",
                MemoryKind::Semantic => "fact",
                MemoryKind::Procedural => "procedure",
            },
            item.memory.confidence * 100.0,
            item.score,
            truncate_chars(&item.memory.statement, MAX_RECALLED_STATEMENT_CHARS)
        );
        let line_chars = line.chars().count() + usize::from(!context_lines.is_empty());
        if context_chars + line_chars > DEFAULT_RECALL_CONTEXT_CHARS {
            break;
        }
        context_chars += line_chars;
        context_lines.push(line);
    }
    let context = context_lines.join("\n");
    RecallResponse {
        ok: true,
        query: input.query,
        memories: recalled,
        context,
    }
}

fn json_pointer<T: Serialize>(value: &T) -> *mut c_char {
    serde_json::to_string(value)
        .ok()
        .and_then(|raw| CString::new(raw).ok())
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

fn error_pointer(message: impl ToString) -> *mut c_char {
    json_pointer(&json!({ "ok": false, "error": message.to_string() }))
}

fn parse_input<T: for<'de> Deserialize<'de>>(
    data_ptr: *const u8,
    data_len: usize,
) -> Result<T, String> {
    if data_ptr.is_null() || data_len == 0 {
        return Err("input JSON is empty".to_string());
    }
    let bytes = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

#[no_mangle]
pub extern "C" fn nola_core_intelligence_ingest(
    data_ptr: *const u8,
    data_len: usize,
) -> *mut c_char {
    let input: IntelligenceIngestInput = match parse_input(data_ptr, data_len) {
        Ok(input) => input,
        Err(error) => return error_pointer(error),
    };
    let _guard = match INTELLIGENCE_LOCK.lock() {
        Ok(guard) => guard,
        Err(_) => return error_pointer("intelligence state lock is poisoned"),
    };
    let mut state = match load_state() {
        Ok(state) => state,
        Err(error) => return error_pointer(error),
    };
    let response = ingest_into_state(&mut state, input);
    if let Err(error) = save_state(&state) {
        return error_pointer(error);
    }
    json_pointer(&response)
}

#[no_mangle]
pub extern "C" fn nola_core_intelligence_recall(
    data_ptr: *const u8,
    data_len: usize,
) -> *mut c_char {
    let input: RecallInput = match parse_input(data_ptr, data_len) {
        Ok(input) => input,
        Err(error) => return error_pointer(error),
    };
    let _guard = match INTELLIGENCE_LOCK.lock() {
        Ok(guard) => guard,
        Err(_) => return error_pointer("intelligence state lock is poisoned"),
    };
    let mut state = match load_state() {
        Ok(state) => state,
        Err(error) => return error_pointer(error),
    };
    let response = recall_from_state(&mut state, input);
    if let Err(error) = save_state(&state) {
        return error_pointer(error);
    }
    json_pointer(&response)
}

#[no_mangle]
pub extern "C" fn nola_core_intelligence_snapshot() -> *mut c_char {
    let _guard = match INTELLIGENCE_LOCK.lock() {
        Ok(guard) => guard,
        Err(_) => return error_pointer("intelligence state lock is poisoned"),
    };
    match load_state() {
        Ok(state) => json_pointer(&state),
        Err(error) => error_pointer(error),
    }
}

#[no_mangle]
pub extern "C" fn nola_core_intelligence_clear() -> bool {
    let _guard = match INTELLIGENCE_LOCK.lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };
    remove_config_file(STATE_FILE).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intelligence_state_requires_current_schema() {
        let missing = serde_json::from_str::<IntelligenceState>(
            r#"{"events":[],"memories":[],"outcomes":[],"strategies":[],"updated_at":"now"}"#,
        );
        assert!(missing.is_err());

        let unsupported = serde_json::from_str::<IntelligenceState>(
            r#"{"schema_version":0,"events":[],"memories":[],"outcomes":[],"strategies":[],"updated_at":"now"}"#,
        )
        .expect("typed state");
        assert_ne!(unsupported.schema_version, SCHEMA_VERSION);
    }

    fn candidate(key: &str, statement: &str, confidence: f64) -> MemoryCandidate {
        MemoryCandidate {
            semantic_key: Some(key.to_string()),
            kind: MemoryKind::Semantic,
            statement: statement.to_string(),
            confidence,
            source_event_ids: Vec::new(),
            observed_at: "2026-07-13T00:00:00Z".to_string(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn repeated_memory_is_reinforced_without_duplication() {
        let mut state = IntelligenceState::default();
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![candidate(
                    "preference.response",
                    "Prefers concise answers",
                    0.72,
                )],
                ..Default::default()
            },
        );
        let original_confidence = state.memories[0].confidence;
        let response = ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![candidate(
                    "preference.response",
                    "Prefers concise answers",
                    0.80,
                )],
                ..Default::default()
            },
        );
        assert_eq!(state.memories.len(), 1);
        assert!(state.memories[0].confidence > original_confidence);
        assert_eq!(response.memory_changes[0].action, "reinforced");
    }

    #[test]
    fn stronger_conflict_supersedes_old_memory() {
        let mut state = IntelligenceState::default();
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![candidate(
                    "work.location",
                    "Usually works in Shanghai",
                    0.62,
                )],
                ..Default::default()
            },
        );
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![candidate(
                    "work.location",
                    "Usually works in Hangzhou",
                    0.88,
                )],
                ..Default::default()
            },
        );
        assert_eq!(state.memories.len(), 2);
        assert_eq!(state.memories[0].status, MemoryStatus::Superseded);
        assert_eq!(state.memories[1].status, MemoryStatus::Active);
        assert_eq!(
            state.memories[1].supersedes.as_deref(),
            Some(state.memories[0].id.as_str())
        );
    }

    #[test]
    fn recall_prefers_relevant_memory_and_returns_evidence() {
        let mut state = IntelligenceState::default();
        state.events.push(PersonalEvent {
            id: "health-1".to_string(),
            source: "health".to_string(),
            kind: "sleep_summary".to_string(),
            occurred_at: "2026-07-12T00:00:00Z".to_string(),
            observed_at: "2026-07-13T00:00:00Z".to_string(),
            subject: Some("sleep".to_string()),
            summary: "Average sleep dropped below six hours".to_string(),
            payload: Value::Null,
            importance: 0.8,
            confidence: 0.95,
            tags: vec!["sleep".to_string()],
            dedupe_key: Some("health-1".to_string()),
        });
        let mut health = candidate(
            "health.sleep.baseline",
            "Recent sleep is below the usual baseline",
            0.9,
        );
        health.source_event_ids.push("health-1".to_string());
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![
                    health,
                    candidate("preference.ui", "Prefers compact sidebars", 0.9),
                ],
                ..Default::default()
            },
        );
        let response = recall_from_state(
            &mut state,
            RecallInput {
                query: "sleep health trend".to_string(),
                limit: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(response.memories.len(), 1);
        assert_eq!(
            response.memories[0].memory.semantic_key,
            "healthsleepbaseline"
        );
        assert_eq!(response.memories[0].evidence[0].source, "health");
    }

    #[test]
    fn recall_excludes_high_confidence_but_unrelated_memory() {
        let mut state = IntelligenceState::default();
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![
                    candidate("preference.drink", "用户喝咖啡时不加糖", 0.72),
                    candidate("preference.interface", "用户喜欢紧凑的侧边栏", 0.99),
                ],
                ..Default::default()
            },
        );
        let response = recall_from_state(
            &mut state,
            RecallInput {
                query: "咖啡不加糖".to_string(),
                limit: Some(8),
                ..Default::default()
            },
        );
        assert_eq!(response.memories.len(), 1);
        assert_eq!(response.memories[0].memory.semantic_key, "preferencedrink");
    }

    #[test]
    fn exact_long_term_memory_beats_recent_partial_match() {
        let mut state = IntelligenceState::default();
        let mut exact = candidate("preference.coffee", "用户喝咖啡时不加糖", 0.74);
        exact.observed_at = "2023-01-01T00:00:00Z".to_string();
        let mut partial = candidate("work.cafe", "用户偶尔在咖啡店工作", 0.98);
        partial.observed_at = now_iso();
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![partial, exact],
                ..Default::default()
            },
        );
        let response = recall_from_state(
            &mut state,
            RecallInput {
                query: "咖啡不加糖".to_string(),
                limit: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(response.memories[0].memory.semantic_key, "preferencecoffee");
    }

    #[test]
    fn recall_context_is_bounded_even_for_oversized_memories() {
        let mut state = IntelligenceState::default();
        let statement = format!("project reference {}", "detail ".repeat(3_000));
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                memory_candidates: vec![candidate("project.reference", &statement, 0.9)],
                ..Default::default()
            },
        );
        let response = recall_from_state(
            &mut state,
            RecallInput {
                query: "project reference".to_string(),
                limit: Some(8),
                ..Default::default()
            },
        );
        assert!(!response.context.is_empty());
        assert!(response.context.chars().count() <= DEFAULT_RECALL_CONTEXT_CHARS);
        assert!(response.context.contains('…'));
    }

    #[test]
    fn outcomes_update_route_score() {
        let mut state = IntelligenceState::default();
        let outcome = TaskOutcome {
            id: String::new(),
            task_type: "code_change".to_string(),
            route: "host".to_string(),
            model: "qwen".to_string(),
            tools: vec!["shell".to_string()],
            success: true,
            user_corrected: false,
            retry_count: 0,
            latency_ms: 1200,
            occurred_at: String::new(),
        };
        ingest_into_state(
            &mut state,
            IntelligenceIngestInput {
                outcomes: vec![outcome.clone(), outcome],
                ..Default::default()
            },
        );
        assert_eq!(state.strategies.len(), 1);
        assert_eq!(state.strategies[0].attempts, 2);
        assert_eq!(state.strategies[0].successes, 2);
        assert!(state.strategies[0].score > 0.7);
    }
}
