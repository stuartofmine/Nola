// SPDX-License-Identifier: Apache-2.0
//! Long-term FTS5 Memory v2 (typed owner/scope/kind/confidence/importance/sensitivity/provenance,
//! LLM-backed atomic extraction & consolidation with offline fallback), Codebase Memory snapshot,
//! and post-compaction `conversation_recall` tool backed by `events_fts` & `read_event_range`.

use std::collections::HashSet;
use std::process::Command;
use std::sync::Arc;

use nola_llm::LlmClient;
use nola_store::Store;
use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, FrozenSystemBlock, HistoryMessage, MemoryItem,
    ModelRouteConfig, StructuredModelRequest, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_MEMORY_META: &str = "memory_meta";

/// Typed owner for a Memory v2 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOwner {
    #[default]
    User,
    Assistant,
    Workspace,
    System,
}

impl MemoryOwner {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "workspace" => Some(Self::Workspace),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Workspace => "workspace",
            Self::System => "system",
        }
    }
}

/// Typed scope for a Memory v2 record (`global` | `workspace:<id>` | `conversation:<id>` | `turn:<id>` | `custom:<name>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MemoryScope {
    #[default]
    Global,
    Workspace(String),
    Conversation(String),
    Turn(String),
    Custom(String),
}

impl MemoryScope {
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("global") {
            return Self::Global;
        }
        if let Some(rest) = trimmed.strip_prefix("workspace:") {
            return Self::Workspace(rest.trim().to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("conversation:") {
            return Self::Conversation(rest.trim().to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("turn:") {
            return Self::Turn(rest.trim().to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("custom:") {
            return Self::Custom(rest.trim().to_string());
        }
        Self::Custom(trimmed.to_string())
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Workspace(id) => format!("workspace:{id}"),
            Self::Conversation(id) => format!("conversation:{id}"),
            Self::Turn(id) => format!("turn:{id}"),
            Self::Custom(name) => format!("custom:{name}"),
        }
    }
}

impl Serialize for MemoryScope {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_string())
    }
}

impl<'de> Deserialize<'de> for MemoryScope {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::parse(&s))
    }
}

/// Semantic kind of memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[default]
    Fact,
    Preference,
    Procedure,
    Episode,
    Decision,
}

impl MemoryKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fact" => Some(Self::Fact),
            "preference" => Some(Self::Preference),
            "procedure" => Some(Self::Procedure),
            "episode" => Some(Self::Episode),
            "decision" => Some(Self::Decision),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Procedure => "procedure",
            Self::Episode => "episode",
            Self::Decision => "decision",
        }
    }
}

/// Privacy sensitivity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySensitivity {
    #[default]
    Normal,
    Sensitive,
}

impl MemorySensitivity {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(Self::Normal),
            "sensitive" => Some(Self::Sensitive),
            _ => None,
        }
    }
}

/// Provenance tracking where a memory originated and what prior entries it superseded.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemoryProvenance {
    #[serde(default = "default_provenance_source")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
}

fn default_provenance_source() -> String {
    "explicit".to_string()
}

fn default_unit_score() -> f32 {
    0.8
}

/// Sidecar Memory v2 metadata persisted alongside `MemoryItem` in `module_records("memory_meta")`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryMetadata {
    #[serde(default)]
    pub owner: MemoryOwner,
    #[serde(default)]
    pub scope: MemoryScope,
    #[serde(default)]
    pub kind: MemoryKind,
    #[serde(default = "default_unit_score")]
    pub confidence: f32,
    #[serde(default = "default_unit_score")]
    pub importance: f32,
    #[serde(default)]
    pub sensitivity: MemorySensitivity,
    #[serde(default)]
    pub provenance: MemoryProvenance,
}

impl Default for MemoryMetadata {
    fn default() -> Self {
        Self {
            owner: MemoryOwner::User,
            scope: MemoryScope::Global,
            kind: MemoryKind::Fact,
            confidence: 0.8,
            importance: 0.5,
            sensitivity: MemorySensitivity::Normal,
            provenance: MemoryProvenance::default(),
        }
    }
}

/// Enriched Memory v2 view flattening the canonical `MemoryItem` with typed v2 metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecordV2 {
    #[serde(flatten)]
    pub item: MemoryItem,
    pub owner: MemoryOwner,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub confidence: f32,
    pub importance: f32,
    pub sensitivity: MemorySensitivity,
    pub provenance: MemoryProvenance,
}

impl MemoryRecordV2 {
    pub fn from_parts(item: MemoryItem, meta: MemoryMetadata) -> Self {
        Self {
            item,
            owner: meta.owner,
            scope: meta.scope,
            kind: meta.kind,
            confidence: meta.confidence.clamp(0.0, 1.0),
            importance: meta.importance.clamp(0.0, 1.0),
            sensitivity: meta.sensitivity,
            provenance: meta.provenance,
        }
    }

    pub fn metadata(&self) -> MemoryMetadata {
        MemoryMetadata {
            owner: self.owner,
            scope: self.scope.clone(),
            kind: self.kind,
            confidence: self.confidence,
            importance: self.importance,
            sensitivity: self.sensitivity,
            provenance: self.provenance.clone(),
        }
    }
}

/// Optional filter criteria for Memory v2 searches.
#[derive(Debug, Clone, Default)]
pub struct MemorySearchFilter {
    pub namespace: Option<String>,
    pub owner: Option<MemoryOwner>,
    pub scope: Option<MemoryScope>,
    pub kind: Option<MemoryKind>,
    pub sensitivity: Option<MemorySensitivity>,
    pub min_confidence: Option<f32>,
    pub min_importance: Option<f32>,
}

/// Persist a Memory v2 record (`MemoryItem` in FTS5 + `MemoryMetadata` sidecar).
pub fn save_memory_v2(
    store: &Store,
    item: MemoryItem,
    mut meta: MemoryMetadata,
) -> Result<MemoryRecordV2, nola_store::StoreError> {
    meta.confidence = meta.confidence.clamp(0.0, 1.0);
    meta.importance = meta.importance.clamp(0.0, 1.0);
    store.upsert_memory(&item)?;
    store.put_module_record(KIND_MEMORY_META, &item.id, &meta)?;
    Ok(MemoryRecordV2::from_parts(item, meta))
}

/// Load typed `MemoryMetadata` for a stored memory ID (defaulting cleanly for legacy rows).
pub fn load_memory_metadata(store: &Store, id: &str) -> MemoryMetadata {
    store
        .get_module_record::<MemoryMetadata>(KIND_MEMORY_META, id)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Search Memory v2 entries using SQLite FTS5 BM25 plus optional typed metadata filters.
pub fn search_memory_v2(
    store: &Store,
    query: &str,
    filter: &MemorySearchFilter,
    limit: usize,
) -> Result<Vec<MemoryRecordV2>, nola_store::StoreError> {
    let has_meta_filter = filter.owner.is_some()
        || filter.scope.is_some()
        || filter.kind.is_some()
        || filter.sensitivity.is_some()
        || filter.min_confidence.is_some()
        || filter.min_importance.is_some();
    let fetch_limit = if has_meta_filter {
        limit.saturating_mul(5).clamp(25, 250)
    } else {
        limit.max(1)
    };
    let raw = store.search_memory(query, filter.namespace.as_deref(), fetch_limit)?;
    let mut results = Vec::new();
    for item in raw {
        let meta = load_memory_metadata(store, &item.id);
        if let Some(owner) = filter.owner {
            if meta.owner != owner {
                continue;
            }
        }
        if let Some(ref scope) = filter.scope {
            if &meta.scope != scope {
                continue;
            }
        }
        if let Some(kind) = filter.kind {
            if meta.kind != kind {
                continue;
            }
        }
        if let Some(sens) = filter.sensitivity {
            if meta.sensitivity != sens {
                continue;
            }
        }
        if let Some(min_conf) = filter.min_confidence {
            if meta.confidence < min_conf {
                continue;
            }
        }
        if let Some(min_imp) = filter.min_importance {
            if meta.importance < min_imp {
                continue;
            }
        }
        results.push(MemoryRecordV2::from_parts(item, meta));
        if results.len() >= limit.max(1) {
            break;
        }
    }
    Ok(results)
}

#[derive(Debug, Deserialize)]
struct ExtractedFactDraft {
    title: String,
    content: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    importance: Option<f32>,
    #[serde(default)]
    sensitivity: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn token_jaccard(a: &str, b: &str) -> f32 {
    let tok = |s: &str| -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .map(|w| w.trim().to_ascii_lowercase())
            .filter(|w| w.len() >= 2)
            .collect()
    };
    let sa = tok(a);
    let sb = tok(b);
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn parse_extracted_facts_json(raw: &str) -> Option<Vec<ExtractedFactDraft>> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim().trim_end_matches("```").trim())
        .unwrap_or(trimmed);

    if let Ok(arr) = serde_json::from_str::<Vec<ExtractedFactDraft>>(stripped) {
        return Some(arr);
    }
    if let Ok(val) = serde_json::from_str::<Value>(stripped) {
        if let Some(items) = val.get("facts").or_else(|| val.get("memories")) {
            if let Ok(arr) = serde_json::from_value::<Vec<ExtractedFactDraft>>(items.clone()) {
                return Some(arr);
            }
        }
    }
    if let (Some(start), Some(end)) = (stripped.find('['), stripped.rfind(']')) {
        if end > start {
            if let Ok(arr) = serde_json::from_str::<Vec<ExtractedFactDraft>>(&stripped[start..=end])
            {
                return Some(arr);
            }
        }
    }
    None
}

fn consolidate_fact(
    store: &Store,
    namespace: &str,
    session_id: &str,
    source: &str,
    draft: ExtractedFactDraft,
) -> Result<Option<MemoryRecordV2>, String> {
    let title = nola_llm::PrivacyRedactor::redact_text(draft.title.trim());
    let content = nola_llm::PrivacyRedactor::redact_text(draft.content.trim());
    if title.is_empty() || content.is_empty() {
        return Ok(None);
    }
    let ts = now_ms();
    let existing = store
        .search_memory(&title, Some(namespace), 5)
        .unwrap_or_default();
    let mut matched_id: Option<String> = None;
    let mut created_at_ms = ts;
    let mut supersedes = Vec::new();
    for cand in existing {
        let title_match = cand.title.trim().eq_ignore_ascii_case(title.trim());
        let sim = token_jaccard(
            &format!("{} {}", cand.title, cand.content),
            &format!("{title} {content}"),
        );
        if title_match || sim >= 0.65 {
            matched_id = Some(cand.id.clone());
            created_at_ms = cand.created_at_ms;
            supersedes.push(cand.id);
            break;
        }
    }

    let mut tags = draft.tags;
    if !tags.iter().any(|t| t == "consolidated") {
        tags.push("consolidated".into());
    }
    let item = MemoryItem {
        id: matched_id.unwrap_or_else(new_id),
        namespace: namespace.to_string(),
        title: title.chars().take(120).collect(),
        content: content.chars().take(1_200).collect(),
        tags,
        created_at_ms,
        updated_at_ms: ts,
    };
    let meta = MemoryMetadata {
        owner: draft
            .owner
            .as_deref()
            .and_then(MemoryOwner::parse)
            .unwrap_or(MemoryOwner::User),
        scope: if session_id.trim().is_empty() {
            MemoryScope::Global
        } else {
            MemoryScope::Conversation(session_id.to_string())
        },
        kind: draft
            .kind
            .as_deref()
            .and_then(MemoryKind::parse)
            .unwrap_or(MemoryKind::Fact),
        confidence: draft.confidence.unwrap_or(0.8).clamp(0.0, 1.0),
        importance: draft.importance.unwrap_or(0.6).clamp(0.0, 1.0),
        sensitivity: draft
            .sensitivity
            .as_deref()
            .and_then(MemorySensitivity::parse)
            .unwrap_or(MemorySensitivity::Normal),
        provenance: MemoryProvenance {
            source: source.to_string(),
            session_id: (!session_id.trim().is_empty()).then(|| session_id.to_string()),
            turn_id: None,
            supersedes,
        },
    };
    save_memory_v2(store, item, meta)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn deterministic_fallback_extract(
    user_prompt: &str,
    assistant_answer: &str,
) -> Vec<ExtractedFactDraft> {
    let clean_prompt = nola_llm::PrivacyRedactor::redact_text(user_prompt);
    let clean_answer = nola_llm::PrivacyRedactor::redact_text(assistant_answer.trim());
    if clean_answer.chars().count() < 40 {
        return Vec::new();
    }
    let title: String = clean_prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("Session insight")
        .trim()
        .chars()
        .take(80)
        .collect();
    let lower_prompt = clean_prompt.to_ascii_lowercase();
    let kind = if lower_prompt.contains("prefer") || lower_prompt.contains("always use") {
        "preference"
    } else if lower_prompt.contains("decid") || lower_prompt.contains("choose") {
        "decision"
    } else if lower_prompt.contains("how to") || lower_prompt.contains("steps") {
        "procedure"
    } else {
        "fact"
    };
    vec![ExtractedFactDraft {
        title,
        content: clean_answer.chars().take(600).collect(),
        kind: Some(kind.into()),
        owner: Some("user".into()),
        confidence: Some(0.7),
        importance: Some(0.6),
        sensitivity: Some("normal".into()),
        tags: vec!["auto_distilled".into()],
    }]
}

/// Background memory distillation & consolidation worker.
pub struct MemoryWorker;

impl MemoryWorker {
    pub fn distill_turn(
        suite: &ModulesSuite,
        namespace: &str,
        user_prompt: &str,
        assistant_answer: &str,
    ) -> Result<Option<MemoryItem>, nola_store::StoreError> {
        let answer = assistant_answer.trim();
        if answer.chars().count() < 80 {
            return Ok(None);
        }
        let clean_prompt = nola_llm::PrivacyRedactor::redact_text(user_prompt);
        let clean_answer = nola_llm::PrivacyRedactor::redact_text(answer);
        let title: String = clean_prompt
            .lines()
            .next()
            .unwrap_or("Session insight")
            .chars()
            .take(80)
            .collect();
        let content: String = clean_answer.chars().take(600).collect();
        let ts = now_ms();
        let item = MemoryItem {
            id: new_id(),
            namespace: namespace.to_string(),
            title,
            content,
            tags: vec!["auto_distilled".into()],
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        suite.store.upsert_memory(&item)?;
        Ok(Some(item))
    }

    /// Extract atomic Memory v2 facts using an LLM route when available, deduplicating and
    /// superseding overlapping stored memories; falls back cleanly to deterministic redacted
    /// extraction when offline or if the LLM route is unavailable.
    pub fn extract_and_consolidate_with_llm(
        store: &Store,
        llm_route: Option<&ModelRouteConfig>,
        session_id: &str,
        user_prompt: &str,
        assistant_answer: &str,
    ) -> Result<Vec<MemoryRecordV2>, String> {
        let clean_prompt = nola_llm::PrivacyRedactor::redact_text(user_prompt);
        let clean_answer = nola_llm::PrivacyRedactor::redact_text(assistant_answer);
        let namespace = "default";

        let llm_drafts: Option<Vec<ExtractedFactDraft>> = llm_route.and_then(|route| {
            let client = LlmClient::new(30).ok()?;
            let req = StructuredModelRequest {
                frozen_system_prefix: vec![FrozenSystemBlock {
                    id: "memory_extractor".into(),
                    text: "Extract durable atomic facts, user preferences, decisions, or procedures from the exchange. Respond ONLY with a JSON array of objects containing keys: title, content, kind (fact|preference|procedure|episode|decision), owner (user|assistant|workspace|system), confidence (0..1), importance (0..1), sensitivity (normal|sensitive), tags (string array). Omit secrets or transient chatter."
                        .into(),
                }],
                prefix_stable_tools: Vec::new(),
                history: vec![HistoryMessage::User {
                    content: format!("USER:\n{clean_prompt}\n\nASSISTANT:\n{clean_answer}"),
                    media: Vec::new(),
                }],
                ephemeral_tail_reminders: Vec::new(),
                max_output_tokens: 1024,
                json_schema: None,
            };
            let resp = client.complete(route, &req, false, &mut |_| {}).ok()?;
            parse_extracted_facts_json(&resp.content)
        });

        let (source, drafts) = match llm_drafts {
            Some(list) if !list.is_empty() => ("llm_consolidated", list),
            _ => (
                "deterministic_fallback",
                deterministic_fallback_extract(&clean_prompt, &clean_answer),
            ),
        };

        let mut saved = Vec::new();
        for draft in drafts.into_iter().take(8) {
            if let Some(rec) = consolidate_fact(store, namespace, session_id, source, draft)? {
                saved.push(rec);
            }
        }
        Ok(saved)
    }
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(MemorySaveTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(MemorySearchTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(ConversationRecallTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(CodeMemorySummaryTool {
        suite: Arc::clone(suite),
    }));
}

struct MemorySaveTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for MemorySaveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "memory_save".into(),
                description: "Persist a durable fact, preference, procedure, episode, or decision into FTS5-indexed Memory v2.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "namespace": { "type": "string", "description": "Default 'default'" },
                        "title": { "type": "string" },
                        "content": { "type": "string" },
                        "tags": { "type": "array", "items": { "type": "string" } },
                        "owner": { "type": "string", "enum": ["user", "assistant", "workspace", "system"] },
                        "scope": { "type": "string", "description": "global | workspace:<id> | conversation:<id> | turn:<id> | custom:<name>" },
                        "kind": { "type": "string", "enum": ["fact", "preference", "procedure", "episode", "decision"] },
                        "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "importance": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "sensitivity": { "type": "string", "enum": ["normal", "sensitive"] },
                        "provenance": { "description": "Source string or structured provenance object" },
                        "supersedes": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["title", "content"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(title) = args.get("title").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'title'", None);
        };
        let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'content'", None);
        };
        let ts = now_ms();
        let item = MemoryItem {
            id: args
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id),
            namespace: args
                .get("namespace")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string(),
            title: title.to_string(),
            content: content.to_string(),
            tags: args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        let mut provenance = match args.get("provenance") {
            Some(Value::String(s)) => MemoryProvenance {
                source: s.clone(),
                session_id: (!ctx.session_id.is_empty()).then(|| ctx.session_id.clone()),
                turn_id: None,
                supersedes: Vec::new(),
            },
            Some(v @ Value::Object(_)) => {
                serde_json::from_value::<MemoryProvenance>(v.clone()).unwrap_or_default()
            }
            _ => MemoryProvenance {
                source: "tool:memory_save".into(),
                session_id: (!ctx.session_id.is_empty()).then(|| ctx.session_id.clone()),
                turn_id: None,
                supersedes: Vec::new(),
            },
        };
        if let Some(arr) = args.get("supersedes").and_then(|v| v.as_array()) {
            for s in arr.iter().filter_map(|v| v.as_str()) {
                if !provenance.supersedes.iter().any(|x| x == s) {
                    provenance.supersedes.push(s.to_string());
                }
            }
        }
        let meta = MemoryMetadata {
            owner: args
                .get("owner")
                .and_then(|v| v.as_str())
                .and_then(MemoryOwner::parse)
                .unwrap_or(MemoryOwner::User),
            scope: args
                .get("scope")
                .and_then(|v| v.as_str())
                .map(MemoryScope::parse)
                .unwrap_or(MemoryScope::Global),
            kind: args
                .get("kind")
                .and_then(|v| v.as_str())
                .and_then(MemoryKind::parse)
                .unwrap_or(MemoryKind::Fact),
            confidence: args
                .get("confidence")
                .and_then(|v| v.as_f64())
                .map(|v| (v as f32).clamp(0.0, 1.0))
                .unwrap_or(0.9),
            importance: args
                .get("importance")
                .and_then(|v| v.as_f64())
                .map(|v| (v as f32).clamp(0.0, 1.0))
                .unwrap_or(0.6),
            sensitivity: args
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .and_then(MemorySensitivity::parse)
                .unwrap_or(MemorySensitivity::Normal),
            provenance,
        };
        match save_memory_v2(&self.suite.store, item, meta) {
            Ok(rec) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&rec).unwrap_or(json!({})),
                format!("Saved memory '{}' ({})", rec.item.title, rec.item.id),
            ),
            Err(e) => ToolExecutionResult::err_structured("memory_error", e.to_string(), None),
        }
    }
}

struct MemorySearchTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for MemorySearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "memory_search".into(),
                description: "Search persisted long-term Memory v2 entries using SQLite FTS5 BM25 ranking with optional owner/scope/kind/sensitivity filters.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "namespace": { "type": "string" },
                        "owner": { "type": "string", "enum": ["user", "assistant", "workspace", "system"] },
                        "scope": { "type": "string" },
                        "kind": { "type": "string", "enum": ["fact", "preference", "procedure", "episode", "decision"] },
                        "sensitivity": { "type": "string", "enum": ["normal", "sensitive"] },
                        "min_confidence": { "type": "number" },
                        "min_importance": { "type": "number" },
                        "limit": { "type": "integer", "description": "Default 10" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 100) as usize;
        let filter = MemorySearchFilter {
            namespace: args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            owner: args
                .get("owner")
                .and_then(|v| v.as_str())
                .and_then(MemoryOwner::parse),
            scope: args
                .get("scope")
                .and_then(|v| v.as_str())
                .map(MemoryScope::parse),
            kind: args
                .get("kind")
                .and_then(|v| v.as_str())
                .and_then(MemoryKind::parse),
            sensitivity: args
                .get("sensitivity")
                .and_then(|v| v.as_str())
                .and_then(MemorySensitivity::parse),
            min_confidence: args
                .get("min_confidence")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            min_importance: args
                .get("min_importance")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
        };
        match search_memory_v2(&self.suite.store, query, &filter, limit) {
            Ok(items) => {
                let count = items.len();
                ToolExecutionResult::ok_structured(
                    json!({"query": query, "count": count, "items": items}),
                    format!("Found {count} memory item(s)"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("memory_error", e.to_string(), None),
        }
    }
}

/// Post-compaction conversation transcript recall backed by SQLite `events_fts` and `read_event_range`.
pub struct ConversationRecallTool {
    suite: Arc<ModulesSuite>,
}

impl ConversationRecallTool {
    pub fn new(suite: Arc<ModulesSuite>) -> Self {
        Self { suite }
    }
}

impl ToolHandler for ConversationRecallTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "conversation_recall".into(),
                description: "Search historical conversation transcripts using FTS5 ('search') or read an ordered event sequence window ('read_range'), even after context compaction.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["search", "read_range"] },
                        "query": { "type": "string", "description": "Search query (for action=search)" },
                        "scope_id": { "type": "string", "description": "Session/conversation ID (optional for search; defaults to current session for read_range)" },
                        "session_id": { "type": "string", "description": "Alias for scope_id" },
                        "from_seq": { "type": "integer", "description": "Starting event sequence number inclusive (for action=read_range, default 1)" },
                        "limit": { "type": "integer", "description": "Maximum events to return (default 20, max 100)" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let scope_arg = args
            .get("scope_id")
            .or_else(|| args.get("session_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 100) as usize;

        match action {
            "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if query.is_empty() {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing non-empty 'query' for conversation_recall search",
                        None,
                    );
                }
                match self.suite.store.search_events_fts(scope_arg, query, limit) {
                    Ok(events) => {
                        let count = events.len();
                        ToolExecutionResult::ok_structured(
                            json!({
                                "action": "search",
                                "query": query,
                                "scope_id": scope_arg,
                                "count": count,
                                "events": events
                            }),
                            format!("Recalled {count} conversation event(s) matching '{query}'"),
                        )
                    }
                    Err(e) => {
                        ToolExecutionResult::err_structured("recall_error", e.to_string(), None)
                    }
                }
            }
            "read_range" => {
                let scope_id = scope_arg.unwrap_or(ctx.session_id.as_str());
                if scope_id.trim().is_empty() {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing 'scope_id' for conversation_recall read_range",
                        None,
                    );
                }
                let from_seq = args
                    .get("from_seq")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(1);
                match self.suite.store.read_event_range(scope_id, from_seq, limit) {
                    Ok(events) => {
                        let count = events.len();
                        ToolExecutionResult::ok_structured(
                            json!({
                                "action": "read_range",
                                "scope_id": scope_id,
                                "from_seq": from_seq,
                                "count": count,
                                "events": events
                            }),
                            format!("Read {count} event(s) from '{scope_id}' starting at seq {from_seq}"),
                        )
                    }
                    Err(e) => {
                        ToolExecutionResult::err_structured("recall_error", e.to_string(), None)
                    }
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported conversation_recall action '{other}'"),
                None,
            ),
        }
    }
}

struct CodeMemorySummaryTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for CodeMemorySummaryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "code_memory_snapshot".into(),
                description: "Capture and index current repository HEAD, changed files, and recent commits into Code-Memory.".into(),
                parameters: json!({ "type": "object", "properties": {} }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, _args: Value) -> ToolExecutionResult {
        let root = if ctx.workspace_root.exists() {
            &ctx.workspace_root
        } else {
            &self.suite.workspace_root
        };
        let index_stats = crate::code_memory::index_workspace(&self.suite.store, root, false).ok();
        let head = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "none".into());
        let log = Command::new("git")
            .args(["log", "--oneline", "-n", "10"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let (total_files, total_symbols, duration_ms) = index_stats
            .as_ref()
            .map(|s| (s.total_files, s.total_symbols, s.duration_ms))
            .unwrap_or((0, 0, 0));
        let summary_content = if log.is_empty() {
            format!("Indexed {total_files} source file(s) with {total_symbols} symbol(s) in {duration_ms}ms.")
        } else {
            format!(
                "Indexed {total_files} source file(s) with {total_symbols} symbol(s) in {duration_ms}ms.\nRecent commits:\n{log}"
            )
        };
        let ts = now_ms();
        let item = MemoryItem {
            id: format!("code_memory:{head}"),
            namespace: "code_memory".into(),
            title: format!("Repository snapshot @ {head}"),
            content: summary_content,
            tags: vec!["git".into(), "code_memory".into(), "symbols".into()],
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        if let Err(e) = self.suite.store.upsert_memory(&item) {
            return ToolExecutionResult::err_structured("memory_error", e.to_string(), None);
        }
        ToolExecutionResult::ok_structured(
            json!({
                "head": head,
                "recent_commits": log,
                "total_files": total_files,
                "total_symbols": total_symbols,
                "duration_ms": duration_ms
            }),
            format!(
                "Indexed Code-Memory snapshot @ {head} ({total_files} files, {total_symbols} symbols)"
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store::DEFAULT_MAX_EVENTS_PER_SCOPE;

    #[test]
    fn memory_v2_typed_metadata_filtering_consolidation_and_conversation_recall_work() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));
        let mut reg = ToolRegistry::default();
        register_tools(&suite, &mut reg);
        let ctx = ToolContext::new(dir.path()).with_session_id("sess-recall-1");

        // Save Memory v2 with typed metadata
        let saved = reg.execute(
            "memory_save",
            &ctx,
            json!({
                "title": "Editor preference",
                "content": "User prefers Neovim with rust-analyzer",
                "owner": "user",
                "scope": "workspace:nola",
                "kind": "preference",
                "confidence": 0.95,
                "importance": 0.9,
                "sensitivity": "normal",
                "provenance": "explicit_chat"
            }),
        );
        assert!(!saved.is_error);
        let sv = saved.payload.to_model_value();
        assert_eq!(sv["owner"], "user");
        assert_eq!(sv["scope"], "workspace:nola");
        assert_eq!(sv["kind"], "preference");

        // Filtered search matches owner/scope/kind
        let hit = reg.execute(
            "memory_search",
            &ctx,
            json!({
                "query": "Neovim",
                "owner": "user",
                "scope": "workspace:nola",
                "kind": "preference",
                "min_confidence": 0.9
            }),
        );
        assert!(!hit.is_error);
        assert_eq!(hit.payload.to_model_value()["count"], 1);

        // Wrong owner filters it out
        let miss = reg.execute(
            "memory_search",
            &ctx,
            json!({ "query": "Neovim", "owner": "system" }),
        );
        assert_eq!(miss.payload.to_model_value()["count"], 0);

        // Offline consolidation + superseding deduplication
        let first = MemoryWorker::extract_and_consolidate_with_llm(
            &store,
            None,
            "sess-recall-1",
            "How should we release?",
            "Always run cargo test --workspace before tagging any release and verify zero warnings.",
        )
        .unwrap();
        assert_eq!(first.len(), 1);
        let first_id = first[0].item.id.clone();

        let second = MemoryWorker::extract_and_consolidate_with_llm(
            &store,
            None,
            "sess-recall-1",
            "How should we release?",
            "Always run cargo test --workspace and cargo clippy before tagging any release and verify zero warnings.",
        )
        .unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].item.id, first_id);
        assert!(second[0].provenance.supersedes.contains(&first_id));

        // ConversationRecallTool search & read_range
        store
            .append_event(
                "sess-recall-1",
                "user_message",
                json!({"text": "We decided to use SQLite WAL mode for Nola v2"}),
                DEFAULT_MAX_EVENTS_PER_SCOPE,
            )
            .unwrap();
        store
            .append_event(
                "sess-recall-1",
                "assistant_message",
                json!({"text": "Confirmed: WAL mode with FTS5 indexing enabled."}),
                DEFAULT_MAX_EVENTS_PER_SCOPE,
            )
            .unwrap();

        let recalled = reg.execute(
            "conversation_recall",
            &ctx,
            json!({"action": "search", "query": "SQLite", "scope_id": "sess-recall-1"}),
        );
        assert!(!recalled.is_error);
        assert_eq!(recalled.payload.to_model_value()["count"], 1);
        assert!(recalled.payload.to_model_value()["events"][0]["payload"]["text"]
            .as_str()
            .unwrap()
            .contains("SQLite WAL"));

        let range = reg.execute(
            "conversation_recall",
            &ctx,
            json!({"action": "read_range", "from_seq": 1, "limit": 10}),
        );
        assert!(!range.is_error);
        assert_eq!(range.payload.to_model_value()["count"], 2);
    }
}
