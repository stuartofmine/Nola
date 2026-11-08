use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, NaiveDate, Utc};
use quick_xml::de::from_str as parse_xml;
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSearchOutcome {
    pub text: String,
    pub display: String,
    pub error: bool,
    pub evidence: Option<Value>,
}


type ToolOutcome = WebSearchOutcome;

fn ok(text: String, display: String) -> ToolOutcome {
    ToolOutcome {
        text,
        display,
        error: false,
        evidence: None,
    }
}

fn error(text: String) -> ToolOutcome {
    ToolOutcome {
        display: text.replace('\n', " ").chars().take(160).collect(),
        text,
        error: true,
        evidence: None,
    }
}

fn required_string(args: &Map<String, Value>, key: &str) -> Result<String, ToolOutcome> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| error(format!("{key} is required")))
}

fn validate_integer_arg(
    args: &Map<String, Value>,
    key: &str,
    default: i64,
    low: i64,
    high: i64,
) -> Result<i64, ToolOutcome> {
    let value = match args.get(key) {
        None => default,
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| error(format!("{key} must be an integer")))?,
        Some(_) => return Err(error(format!("{key} must be an integer"))),
    };
    if !(low..=high).contains(&value) {
        return Err(error(format!("{key} must be between {low} and {high}")));
    }
    Ok(value)
}

fn is_blocked_public_web_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
    {
        return true;
    }
    normalized
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| match address {
            std::net::IpAddr::V4(value) => {
                value.is_private()
                    || value.is_loopback()
                    || value.is_link_local()
                    || value.is_broadcast()
                    || value.is_documentation()
                    || value.is_unspecified()
                    || value.is_multicast()
            }
            std::net::IpAddr::V6(value) => {
                value.is_loopback()
                    || value.is_unspecified()
                    || value.is_multicast()
                    || value.is_unique_local()
                    || value.is_unicast_link_local()
            }
        })
}

fn validate_public_web_url(raw_url: &str, field_name: &str) -> Result<(), ToolOutcome> {
    let url = Url::parse(raw_url)
        .map_err(|_| error(format!("{field_name} must be an absolute http(s) URL")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(format!("{field_name} scheme must be http or https")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(error(format!("{field_name} must not contain credentials")));
    }
    let host = url
        .host_str()
        .ok_or_else(|| error(format!("{field_name} must include a host")))?;
    if is_blocked_public_web_host(host) {
        return Err(error(format!("{field_name} must target a public host")));
    }
    Ok(())
}

const SEARCH_RESULT_COUNT: usize = 20;
const SEARCH_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const PAGE_RESPONSE_BYTES: u64 = 3 * 1024 * 1024;
const PAGE_TEXT_CHARS: usize = 12_000;
const PAGE_CONCURRENCY: usize = 6;
const STANDARD_PAGE_TIMEOUT_SECONDS: u64 = 8;
const STANDARD_SERP_TIMEOUT_SECONDS: u64 = 10;
const ACADEMIC_PAGE_TIMEOUT_SECONDS: u64 = 25;
const ACADEMIC_SERP_TIMEOUT_SECONDS: u64 = 30;
const SEARCH_BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResearchMode {
    Standard,
    Academic,
}

impl ResearchMode {
    fn parse(value: Option<&str>) -> Result<Self, ToolOutcome> {
        match value.unwrap_or("standard") {
            "standard" => Ok(Self::Standard),
            "academic" => Ok(Self::Academic),
            _ => Err(error(
                "research_mode must be standard or academic".to_string(),
            )),
        }
    }

    fn page_timeout_seconds(self) -> u64 {
        match self {
            Self::Standard => STANDARD_PAGE_TIMEOUT_SECONDS,
            Self::Academic => ACADEMIC_PAGE_TIMEOUT_SECONDS,
        }
    }

    fn serp_timeout_seconds(self) -> u64 {
        match self {
            Self::Standard => STANDARD_SERP_TIMEOUT_SECONDS,
            Self::Academic => ACADEMIC_SERP_TIMEOUT_SECONDS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Freshness {
    None,
    Day,
    Week,
    Month,
    Year,
}

impl Freshness {
    fn parse(value: Option<&str>) -> Result<Self, ToolOutcome> {
        match value.unwrap_or("auto") {
            // The caller owns semantic query rewriting. Core never infers a
            // time filter from language-specific words in the raw query.
            "auto" => Ok(Self::None),
            "none" => Ok(Self::None),
            "day" => Ok(Self::Day),
            "week" => Ok(Self::Week),
            "month" => Ok(Self::Month),
            "year" => Ok(Self::Year),
            _ => Err(error(
                "freshness must be auto, none, day, week, month, or year".to_string(),
            )),
        }
    }

    fn brave(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Day => Some("pd"),
            Self::Week => Some("pw"),
            Self::Month => Some("pm"),
            Self::Year => Some("py"),
        }
    }

    fn searxng(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Day => Some("day"),
            Self::Week | Self::Month => Some("month"),
            Self::Year => Some("year"),
        }
    }

    fn serper(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Day => Some("qdr:d"),
            Self::Week => Some("qdr:w"),
            Self::Month => Some("qdr:m"),
            Self::Year => Some("qdr:y"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

#[derive(Clone, Debug)]
struct SearchRequest {
    query: String,
    result_count: usize,
    engine: String,
    freshness: Freshness,
    language: String,
    region: String,
    parse_pages: bool,
    research_mode: ResearchMode,
}

impl SearchRequest {
    fn parse(args: &Map<String, Value>) -> Result<Self, ToolOutcome> {
        let query = required_string(args, "query")?;
        if query.chars().count() > 500 || query.contains('\0') {
            return Err(error("query is invalid".to_string()));
        }
        let _requested_result_count = validate_integer_arg(
            args,
            "limit",
            SEARCH_RESULT_COUNT as i64,
            1,
            SEARCH_RESULT_COUNT as i64,
        )? as usize;
        // Web search is a discovery tool, not a single-result fetch. Always collect the
        // top twenty candidates so an underspecified model call cannot collapse source
        // diversity. The caller may still cite only the evidence it needs.
        let result_count = SEARCH_RESULT_COUNT;
        let engine = optional_string(args, "engine", "auto")?;
        if !matches!(
            engine.as_str(),
            "auto"
                | "dashscope"
                | "serper"
                | "brave"
                | "searxng"
                | "google_news"
                | "bing_news"
                | "duckduckgo"
                | "google"
                | "bing"
                | "baidu"
                | "sogou"
        ) {
            return Err(error(
                "engine must be auto, dashscope, serper, brave, searxng, google_news, bing_news, duckduckgo, google, bing, baidu, or sogou"
                    .to_string(),
            ));
        }
        let freshness_value = args.get("freshness").and_then(Value::as_str);
        let freshness = Freshness::parse(freshness_value)?;
        // Locale is only a presentation hint for providers. It must never be
        // inferred from query wording or used to choose a different provider
        // set; every query uses the same concurrent search pipeline.
        let default_language = default_search_language();
        let language = optional_string(args, "language", &default_language)?;
        let region = optional_string(args, "region", &default_search_region(&language))?;
        let parse_pages = match args.get("parse_pages") {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(error("parse_pages must be a boolean".to_string())),
        };
        let research_mode = ResearchMode::parse(args.get("research_mode").and_then(Value::as_str))?;
        Ok(Self {
            query,
            result_count,
            engine,
            freshness,
            language,
            region,
            parse_pages,
            research_mode,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct SearchHit {
    search_rank: usize,
    provider_rank: usize,
    provider: String,
    title: String,
    url: String,
    canonical_url: String,
    snippet: String,
    published_at: Option<String>,
    source_name: Option<String>,
    source_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ProviderAttempt {
    provider: String,
    status: String,
    http_status: Option<u16>,
    raw_results: usize,
    accepted_results: usize,
    latency_ms: u128,
    detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct PageAnalysis {
    search_rank: usize,
    provider: String,
    title: String,
    url: String,
    final_url: Option<String>,
    canonical_url: Option<String>,
    domain: String,
    status: String,
    http_status: Option<u16>,
    content_type: Option<String>,
    author: Option<String>,
    published_at: Option<String>,
    modified_at: Option<String>,
    content_chars: usize,
    freshness_score: i32,
    source_score: i32,
    evidence_score: i32,
    evidence: Vec<String>,
    content_excerpt: String,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchReport {
    status: String,
    query: String,
    searched_at: String,
    freshness: Freshness,
    research_mode: ResearchMode,
    requested_results: usize,
    retrieved_results: usize,
    parsed_pages: usize,
    useful_pages: usize,
    provider_attempts: Vec<ProviderAttempt>,
    warnings: Vec<String>,
    results: Vec<PageAnalysis>,
}

#[derive(Debug)]
struct ProviderOutput {
    hits: Vec<SearchHit>,
    attempt: ProviderAttempt,
}

#[derive(Clone, Debug)]
enum Provider {
    BaiduWeb,
    SogouWeb,
    GoogleWeb,
    BingWeb,
    BingNews,
    GoogleNews,
    DuckDuckGo,
    DashScopeMcp { api_key: String, endpoint: String },
    Serper(String),
    Brave(String),
    Searxng(String),
}

impl Provider {
    fn label(&self) -> &'static str {
        match self {
            Self::BaiduWeb => "baidu_json",
            Self::SogouWeb => "sogou_mobile_html",
            Self::GoogleWeb => "google_web_html",
            Self::BingWeb => "bing_web_html",
            Self::BingNews => "bing_news_rss",
            Self::GoogleNews => "google_news_rss",
            Self::DuckDuckGo => "duckduckgo_html",
            Self::DashScopeMcp { .. } => "dashscope_mcp",
            Self::Serper(_) => "serper",
            Self::Brave(_) => "brave",
            Self::Searxng(_) => "searxng",
        }
    }
}

pub fn execute(args: &Map<String, Value>) -> WebSearchOutcome {
    let request = match SearchRequest::parse(args) {
        Ok(request) => request,
        Err(outcome) => return outcome,
    };
    let client = match build_client(request.research_mode.serp_timeout_seconds()) {
        Ok(client) => client,
        Err(err) => return error(format!("Search client failed: {err}")),
    };
    let providers = match configured_providers(&request) {
        Ok(providers) => providers,
        Err(outcome) => return outcome,
    };
    let mut attempts = Vec::new();
    let warnings = Vec::new();
    let mut handles = Vec::new();
    for (index, provider) in providers.into_iter().enumerate() {
        let provider_label = provider.label().to_string();
        let provider_client = client.clone();
        let provider_request = request.clone();
        handles.push((
            index,
            provider_label,
            thread::spawn(move || search_provider(&provider_client, &provider, &provider_request)),
        ));
    }
    let mut completed = handles
        .into_iter()
        .map(|(index, provider, handle)| {
            let result = handle.join().unwrap_or_else(|_| {
                Err(ProviderAttempt {
                    provider,
                    status: "internal_error".to_string(),
                    http_status: None,
                    raw_results: 0,
                    accepted_results: 0,
                    latency_ms: 0,
                    detail: Some("search provider worker terminated unexpectedly".to_string()),
                })
            });
            (index, result)
        })
        .collect::<Vec<_>>();
    completed.sort_by_key(|(index, _)| *index);
    let mut provider_hits = Vec::new();
    for (_, result) in completed {
        match result {
            Ok(output) => {
                attempts.push(output.attempt);
                provider_hits.push(output.hits);
            }
            Err(attempt) => attempts.push(attempt),
        }
    }
    finish_search(&request, attempts, provider_hits, warnings)
}

// HTTP results, including empty/partial results, are terminal for this tool.
// This layer has no browser fallback or native browser authority.
fn finish_search(request: &SearchRequest, attempts: Vec<ProviderAttempt>,
    provider_hits: Vec<Vec<SearchHit>>, mut warnings: Vec<String>) -> WebSearchOutcome {
    let mut hits = fuse_provider_hits(provider_hits, request.result_count.saturating_mul(3))
        .into_iter()
        .filter(|hit| search_hit_matches_query(&request.query, hit))
        .take(request.result_count)
        .collect::<Vec<_>>();
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.search_rank = index + 1;
    }

    if hits.is_empty() {
        return error(format_provider_failure(&request.query, &attempts));
    }
    if hits.len() < request.result_count {
        warnings.push(format!(
            "Only {} unique ranked pages were retrieved out of the requested {}.",
            hits.len(),
            request.result_count
        ));
    }
    let pages = if request.parse_pages {
        parse_ranked_pages(&hits, request)
    } else {
        hits.iter().map(page_without_fetch).collect()
    };
    let parsed_pages = pages.iter().filter(|page| page.status == "parsed").count();
    let useful_pages = pages
        .iter()
        .filter(|page| !page.evidence.is_empty() && page.evidence_score > 0)
        .count();
    if request.parse_pages && parsed_pages == 0 {
        warnings.push("None of the ranked pages produced readable body text.".to_string());
    }
    if useful_pages == 0 {
        warnings.push(
            "No ranked page produced readable evidence; do not answer from model memory."
                .to_string(),
        );
    }
    let status = if hits.len() == request.result_count && useful_pages >= 2 {
        "ok"
    } else if useful_pages > 0 || !request.parse_pages {
        "partial"
    } else {
        "insufficient_evidence"
    };
    let report = SearchReport {
        status: status.to_string(),
        query: request.query.clone(),
        searched_at: utc_now_string(),
        freshness: request.freshness,
        research_mode: request.research_mode,
        requested_results: request.result_count,
        retrieved_results: hits.len(),
        parsed_pages,
        useful_pages,
        provider_attempts: attempts,
        warnings,
        results: pages,
    };
    let text = format_report(&report);
    let display = format!(
        "Web search '{}' retrieved {}/{} and parsed {} pages",
        request.query, report.retrieved_results, report.requested_results, report.parsed_pages
    );
    let evidence = model_source_evidence(&report);
    if report.status == "insufficient_evidence" {
        let mut outcome = error(text);
        outcome.display = display;
        outcome.evidence = Some(evidence);
        outcome
    } else {
        let mut outcome = ok(text, display);
        outcome.evidence = Some(evidence);
        outcome
    }
}

/// Source URLs remain complete, typed model evidence even when the human report
/// is cropped by a later Host/Core budget. A link is never cut into a fake URL.
fn model_source_evidence(report: &SearchReport) -> Value {
    let mut sources = Vec::new();
    let mut ranked = report.results.iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.evidence_score.cmp(&a.evidence_score).then_with(|| a.search_rank.cmp(&b.search_rank)));
    let mut projection = json!({"type":"web_search_evidence", "query":report.query,
        "status":report.status,"searched_at":report.searched_at,"sources":[],"sources_omitted":report.results.len(),
        "guidance":"Use only excerpts that support the claim; cite their exact source URLs in the answer. Missing support is unknown, not verification. Fetch an exact source URL when more original text is needed; omitted sources are not model-visible evidence."});
    for page in ranked {
        let source = json!({"source_id":format!("search-rank:{}",page.search_rank),
            "url":page.final_url.as_deref().unwrap_or(&page.url), "title":truncate(&page.title,96),
            "published_at":page.published_at,"status":page.status,
            "excerpts":page.evidence.iter().take(1).map(|text|truncate(text,160)).collect::<Vec<_>>(),
            "excerpts_truncated":page.evidence.len() > 1 || page.evidence.iter().any(|text|text.chars().count() > 160)});
        let cost = sources.iter().map(|value: &Value| value.to_string().chars().count() + 1).sum::<usize>();
        if projection.to_string().chars().count() + cost + source.to_string().chars().count() > 11_500 { continue; }
        sources.push(source);
    }
    let mut retained = Vec::new();
    for source in &sources {
        let short = json!({"url":source["url"],"title":source["title"]});
        let mut candidate = retained.clone(); candidate.push(short);
        let region = json!({"kind":"web_source_links","sources":candidate,
            "sources_omitted":report.results.len().saturating_sub(candidate.len())});
        if region.to_string().chars().count() <= 384 { retained = candidate; }
    }
    projection["structured_preview"] = json!({"kind":"web_source_links","sources":retained,
        "sources_omitted":report.results.len().saturating_sub(retained.len())});
    projection["sources_omitted"] = json!(report.results.len().saturating_sub(sources.len()));
    projection["sources"] = json!(sources);
    projection
}


fn fuse_provider_hits(provider_hits: Vec<Vec<SearchHit>>, limit: usize) -> Vec<SearchHit> {
    struct FusedHit {
        hit: SearchHit,
        score: u64,
        best_provider_order: usize,
        providers: Vec<String>,
    }

    let mut candidates = HashMap::<String, FusedHit>::new();
    for (provider_order, hits) in provider_hits.into_iter().enumerate() {
        for hit in hits {
            let canonical = canonicalize_url(&hit.url);
            if canonical.is_empty() {
                continue;
            }
            let contribution = 1_000_000_u64 / (60 + hit.provider_rank.max(1)) as u64;
            match candidates.get_mut(&canonical) {
                Some(candidate) => {
                    candidate.score = candidate.score.saturating_add(contribution);
                    if !candidate.providers.contains(&hit.provider) {
                        candidate.providers.push(hit.provider.clone());
                    }
                    if hit.provider_rank < candidate.hit.provider_rank {
                        candidate.hit = hit;
                        candidate.best_provider_order = provider_order;
                    }
                }
                None => {
                    let provider = hit.provider.clone();
                    candidates.insert(
                        canonical,
                        FusedHit {
                            hit,
                            score: contribution,
                            best_provider_order: provider_order,
                            providers: vec![provider],
                        },
                    );
                }
            }
        }
    }
    let mut ranked = candidates.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        is_search_wrapper_url(&left.hit.url)
            .cmp(&is_search_wrapper_url(&right.hit.url))
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| left.hit.provider_rank.cmp(&right.hit.provider_rank))
            .then_with(|| left.best_provider_order.cmp(&right.best_provider_order))
            .then_with(|| left.hit.canonical_url.cmp(&right.hit.canonical_url))
    });
    ranked
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(index, mut candidate)| {
            candidate.hit.search_rank = index + 1;
            candidate.hit.canonical_url = canonicalize_url(&candidate.hit.url);
            candidate.hit.provider = candidate.providers.join("+");
            candidate.hit
        })
        .collect()
}

fn search_hit_matches_query(query: &str, hit: &SearchHit) -> bool {
    let terms = relevance_terms(query);
    if terms.is_empty() {
        return true;
    }
    let metadata = normalized_search_text(&format!("{} {}", hit.title, hit.snippet), 6_000);
    let matched = terms
        .iter()
        .map(|term| normalized_search_text(term, 256))
        .filter(|term| !term.is_empty() && metadata.contains(term))
        .count();
    let minimum_matches = match terms.len() {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        count => (count + 2) / 3,
    };
    matched >= minimum_matches
}

fn configured_providers(request: &SearchRequest) -> Result<Vec<Provider>, ToolOutcome> {
    let dashscope = dashscope_mcp_config();
    let serper = env::var("NOLA_SERPER_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let brave = env::var("NOLA_BRAVE_SEARCH_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let searxng = env::var("NOLA_SEARXNG_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let explicit = match request.engine.as_str() {
        "dashscope" => Some(dashscope.clone().map(|(api_key, endpoint)| {
            Provider::DashScopeMcp { api_key, endpoint }
        }).ok_or_else(|| {
            error("DashScope search requires a DashScope endpoint/API key or NOLA_DASHSCOPE_WEB_SEARCH_KEY".to_string())
        })?),
        "serper" => Some(serper.clone().map(Provider::Serper).ok_or_else(|| {
            error("Serper search requires NOLA_SERPER_API_KEY".to_string())
        })?),
        "brave" => Some(brave.clone().map(Provider::Brave).ok_or_else(|| {
            error("Brave Search requires NOLA_BRAVE_SEARCH_API_KEY".to_string())
        })?),
        "searxng" => Some(searxng.clone().map(Provider::Searxng).ok_or_else(|| {
            error("SearXNG search requires NOLA_SEARXNG_URL".to_string())
        })?),
        "google_news" => Some(Provider::GoogleNews),
        "bing_news" => Some(Provider::BingNews),
        "duckduckgo" => Some(Provider::DuckDuckGo),
        "bing" => Some(Provider::BingWeb),
        "baidu" => Some(Provider::BaiduWeb),
        "sogou" => Some(Provider::SogouWeb),
        "google" => Some(Provider::GoogleWeb),
        _ => None,
    };
    if let Some(provider) = explicit {
        return Ok(vec![provider]);
    }
    let providers = if request.freshness == Freshness::None {
        vec![
            Provider::GoogleWeb,
            Provider::BingWeb,
            Provider::DuckDuckGo,
            Provider::BaiduWeb,
            Provider::SogouWeb,
            Provider::GoogleNews,
            Provider::BingNews,
        ]
    } else {
        vec![
            Provider::GoogleNews,
            Provider::BingNews,
            Provider::GoogleWeb,
            Provider::BingWeb,
            Provider::DuckDuckGo,
            Provider::BaiduWeb,
            Provider::SogouWeb,
        ]
    };
    Ok(providers)
}

fn search_provider(
    client: &Client,
    provider: &Provider,
    request: &SearchRequest,
) -> Result<ProviderOutput, ProviderAttempt> {
    let started = Instant::now();
    let result = match provider {
        Provider::BaiduWeb => search_baidu_json(client, request),
        Provider::SogouWeb => search_sogou_html(client, request),
        Provider::GoogleWeb => search_google_html(client, request),
        Provider::BingWeb => search_bing_html(client, request),
        Provider::BingNews => search_bing_rss(client, request, true),
        Provider::GoogleNews => search_google_news(client, request),
        Provider::DuckDuckGo => search_duckduckgo(client, request),
        Provider::DashScopeMcp { api_key, endpoint } => {
            search_dashscope_mcp(client, api_key, endpoint, request)
        }
        Provider::Serper(key) => search_serper(client, key, request),
        Provider::Brave(key) => search_brave(client, key, request),
        Provider::Searxng(url) => search_searxng(client, url, request),
    };
    match result {
        Ok((status, hits)) => Ok(ProviderOutput {
            attempt: ProviderAttempt {
                provider: provider.label().to_string(),
                status: "ok".to_string(),
                http_status: Some(status),
                raw_results: hits.len(),
                accepted_results: hits.len(),
                latency_ms: started.elapsed().as_millis(),
                detail: None,
            },
            hits,
        }),
        Err((status, detail)) => Err(ProviderAttempt {
            provider: provider.label().to_string(),
            status: classify_provider_error(&detail).to_string(),
            http_status: status,
            raw_results: 0,
            accepted_results: 0,
            latency_ms: started.elapsed().as_millis(),
            detail: Some(detail),
        }),
    }
}

fn search_google_html(
    client: &Client,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let mut url =
        Url::parse("https://www.google.com/search").map_err(|err| (None, err.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &request.query);
        query.append_pair("hl", &request.language);
        query.append_pair("filter", "0");
        query.append_pair("start", "0");
        query.append_pair("num", "20");
    }
    let response = client
        .get(url)
        .header("User-Agent", SEARCH_BROWSER_USER_AGENT)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", &request.language)
        .header("Cookie", "CONSENT=YES+")
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, _, final_url, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((Some(status), format!("HTTP {status}")));
    }
    if final_url.contains("sorry.google.com")
        || final_url.contains("/sorry/")
        || body.contains("/httpservice/retry/enablejs")
        || (body.len() < 2_000 && body.contains("/sorry/"))
    {
        return Err((
            Some(status),
            "Google bot or JavaScript challenge".to_string(),
        ));
    }
    let mut hits = parse_google_html_results(&body);
    hits.truncate(request.result_count.saturating_add(10).min(50));
    if hits.is_empty() {
        return Err((
            Some(status),
            "Google returned no parseable HTML results".to_string(),
        ));
    }
    Ok((status, hits))
}

fn parse_google_html_results(body: &str) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let Ok(anchor_selector) = Selector::parse("a") else {
        return Vec::new();
    };
    let Ok(title_selector) = Selector::parse("h3") else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for anchor in document.select(&anchor_selector) {
        let Some(title_element) = anchor.select(&title_selector).next() else {
            continue;
        };
        let Some(raw_href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(target) = normalize_google_result_url(raw_href) else {
            continue;
        };
        let title = normalize_text(&title_element.text().collect::<Vec<_>>().join(" "));
        if let Some(hit) = search_hit(
            "google_web_html",
            hits.len() + 1,
            title,
            target,
            String::new(),
            None,
            None,
            None,
        ) {
            hits.push(hit);
        }
    }
    hits
}

fn normalize_google_result_url(raw: &str) -> Option<String> {
    if raw.starts_with("https://") || raw.starts_with("http://") {
        let url = Url::parse(raw).ok()?;
        let host = url.host_str()?.to_ascii_lowercase();
        if host == "google.com" || host.ends_with(".google.com") {
            return None;
        }
        return Some(raw.to_string());
    }
    let wrapper = Url::parse("https://www.google.com").ok()?.join(raw).ok()?;
    if wrapper.path() != "/url" {
        return None;
    }
    wrapper
        .query_pairs()
        .find_map(|(key, value)| matches!(key.as_ref(), "q" | "url").then(|| value.into_owned()))
}

fn search_baidu_json(
    client: &Client,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let candidate_limit = request.result_count.saturating_add(10).min(50);
    let mut url = Url::parse("https://www.baidu.com/s").map_err(|err| (None, err.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("wd", &request.query);
        query.append_pair("rn", &candidate_limit.to_string());
        query.append_pair("pn", "0");
        query.append_pair("tn", "json");
    }
    let response = client
        .get(url)
        .header("User-Agent", SEARCH_BROWSER_USER_AGENT)
        .header("Accept", "application/json,text/plain,*/*")
        .header("Accept-Language", &request.language)
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, content_type, final_url, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((Some(status), format!("HTTP {status}")));
    }
    if final_url.contains("wappass.baidu.com/static/captcha") || body.contains("百度安全验证")
    {
        return Err((Some(status), "Baidu bot challenge".to_string()));
    }
    if !content_type.contains("json") && !body.trim_start().starts_with('{') {
        return Err((
            Some(status),
            "unexpected non-JSON response from Baidu".to_string(),
        ));
    }
    let mut hits = parse_baidu_json_results(&body, 1).map_err(|detail| (Some(status), detail))?;
    hits.truncate(candidate_limit);
    if hits.is_empty() {
        return Err((
            Some(status),
            "Baidu returned no parseable JSON results".to_string(),
        ));
    }
    Ok((status, hits))
}

fn parse_baidu_json_results(body: &str, first_rank: usize) -> Result<Vec<SearchHit>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|err| format!("invalid Baidu JSON: {err}"))?;
    if value.get("antiFlag").and_then(Value::as_i64) == Some(1) {
        return Err(value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Baidu denied automated search")
            .to_string());
    }
    let entries = value
        .get("feed")
        .and_then(|feed| feed.get("entry"))
        .and_then(Value::as_array)
        .ok_or_else(|| "Baidu response did not contain feed.entry".to_string())?;
    let mut hits = Vec::new();
    for entry in entries {
        let Some(title) = entry.get("title").and_then(Value::as_str) else {
            continue;
        };
        let Some(url) = entry.get("url").and_then(Value::as_str) else {
            continue;
        };
        let snippet = entry.get("abs").and_then(Value::as_str).unwrap_or_default();
        if let Some(hit) = search_hit(
            "baidu_json",
            first_rank + hits.len(),
            decode_html_text(title),
            url.to_string(),
            decode_html_text(snippet),
            None,
            None,
            None,
        ) {
            hits.push(hit);
        }
    }
    Ok(hits)
}

fn decode_html_text(value: &str) -> String {
    let fragment = Html::parse_fragment(value);
    normalize_text(&fragment.root_element().text().collect::<Vec<_>>().join(" "))
}

fn search_sogou_html(
    client: &Client,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let mut url = Url::parse("https://m.sogou.com/web/searchList.jsp")
        .map_err(|err| (None, err.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("keyword", &request.query);
        query.append_pair("p", "1");
    }
    let response = client
        .get(url)
        .header("User-Agent", SEARCH_BROWSER_USER_AGENT)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", &request.language)
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, _, final_url, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((Some(status), format!("HTTP {status}")));
    }
    if final_url.contains("captcha") || body.contains("请输入验证码") {
        return Err((Some(status), "Sogou bot challenge".to_string()));
    }
    let mut hits = parse_sogou_html_results(&body);
    hits.truncate(request.result_count.saturating_add(10).min(50));
    if hits.is_empty() {
        return Err((
            Some(status),
            "Sogou returned no parseable HTML results".to_string(),
        ));
    }
    Ok((status, hits))
}

fn parse_sogou_html_results(body: &str) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let Ok(anchor_selector) = Selector::parse("h3 a") else {
        return Vec::new();
    };
    let Ok(base) = Url::parse("https://m.sogou.com/web/searchList.jsp") else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for anchor in document.select(&anchor_selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Ok(wrapper) = base.join(href) else {
            continue;
        };
        let target = wrapper
            .query_pairs()
            .find_map(|(key, value)| {
                matches!(key.as_ref(), "pcurl" | "url").then(|| value.into_owned())
            })
            .unwrap_or_default();
        if target.is_empty() {
            continue;
        }
        let title = normalize_text(&anchor.text().collect::<Vec<_>>().join(" "));
        if let Some(hit) = search_hit(
            "sogou_mobile_html",
            hits.len() + 1,
            title,
            target,
            String::new(),
            None,
            None,
            None,
        ) {
            hits.push(hit);
        }
    }
    hits
}

#[cfg(test)]
mod chinese_provider_tests {
    use super::{
        configured_providers, parse_baidu_json_results, parse_google_html_results,
        parse_sogou_html_results, Freshness, ResearchMode, SearchRequest,
    };

    #[test]
    fn parses_baidu_json_search_results() {
        let body = r#"{
            "antiFlag": 0,
            "feed": {"entry": [
                {"title": "CFCA夺得2024 CCF国际AIOps挑战赛冠军!",
                 "url": "https://www.cfca.com.cn/20241115/200000658.html",
                 "abs": "综合得分位列决赛圈第一，夺得冠军。"}
            ]}
        }"#;
        let hits = parse_baidu_json_results(body, 1).expect("valid Baidu response");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].provider, "baidu_json");
        assert!(hits[0].title.contains("CFCA"));
        assert!(hits[0].snippet.contains("夺得冠军"));
    }

    #[test]
    fn unwraps_sogou_mobile_result_target() {
        let body = r#"<html><body><h3><a href="./tc?pcurl=https%3A%2F%2Fwww.cfca.com.cn%2F20241115%2F200000658.html&amp;url=https%3A%2F%2Fm.example.com">CFCA夺得2024 CCF国际AIOps挑战赛冠军!</a></h3></body></html>"#;
        let hits = parse_sogou_html_results(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].provider, "sogou_mobile_html");
        assert_eq!(
            hits[0].url,
            "https://www.cfca.com.cn/20241115/200000658.html"
        );
    }

    #[test]
    fn parses_google_web_result_and_unwraps_redirect() {
        let body = r#"<html><body><a href="/url?q=https%3A%2F%2Fexample.com%2Fanswer&amp;sa=U"><h3>Verified answer</h3></a></body></html>"#;
        let hits = parse_google_html_results(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].provider, "google_web_html");
        assert_eq!(hits[0].url, "https://example.com/answer");
    }

    #[test]
    fn auto_mode_enables_all_keyless_web_engines_for_english() {
        let request = SearchRequest {
            query: "open source agent runtimes".to_string(),
            result_count: 20,
            engine: "auto".to_string(),
            freshness: Freshness::None,
            language: "en-US".to_string(),
            region: "us".to_string(),
            parse_pages: false,
            research_mode: ResearchMode::Standard,
        };
        let labels = configured_providers(&request)
            .expect("auto providers")
            .iter()
            .map(|provider| provider.label())
            .collect::<Vec<_>>();
        for expected in [
            "google_web_html",
            "bing_web_html",
            "duckduckgo_html",
            "baidu_json",
            "sogou_mobile_html",
        ] {
            assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
        }
    }
}

fn search_bing_rss(
    client: &Client,
    request: &SearchRequest,
    news: bool,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let endpoint = if news {
        "https://www.bing.com/news/search"
    } else {
        "https://www.bing.com/search"
    };
    let mut url = Url::parse(endpoint).map_err(|err| (None, err.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &request.query);
        query.append_pair("count", &request.result_count.max(20).min(50).to_string());
        query.append_pair("format", "rss");
        query.append_pair("mkt", &request.language);
        query.append_pair("setlang", &request.language);
        query.append_pair("cc", &request.region.to_uppercase());
    }
    let response = client
        .get(url)
        .header("Accept", "application/rss+xml, application/xml, text/xml")
        .header("Accept-Language", &request.language)
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, content_type, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((Some(status), format!("HTTP {status}")));
    }
    let trimmed = body.trim_start();
    if !content_type.contains("xml")
        && !content_type.contains("rss")
        && !trimmed.starts_with("<?xml")
        && !trimmed.starts_with("<rss")
    {
        return Err((Some(status), "unexpected non-RSS response".to_string()));
    }
    let rss: GoogleNewsRss =
        parse_xml(&body).map_err(|err| (Some(status), format!("invalid Bing RSS: {err}")))?;
    let provider = if news {
        "bing_news_rss"
    } else {
        "bing_web_rss"
    };
    let hits = rss
        .channel
        .items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let source_name = item.source.as_ref().and_then(|source| source.name.clone());
            let source_url = item.source.as_ref().and_then(|source| source.url.clone());
            search_hit(
                provider,
                index + 1,
                item.title,
                item.link,
                item.description.unwrap_or_default(),
                item.pub_date,
                source_name,
                source_url,
            )
        })
        .take(request.result_count)
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Err((Some(status), "Bing returned no RSS items".to_string()));
    }
    Ok((status, hits))
}

fn search_bing_html(
    client: &Client,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let candidate_limit = request.result_count.saturating_add(10).min(50);
    let mut url =
        Url::parse("https://www.bing.com/search").map_err(|err| (None, err.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &request.query);
        query.append_pair("adlt", "off");
        query.append_pair("mkt", &request.language);
    }
    let response = client
        .get(url)
        .header("User-Agent", SEARCH_BROWSER_USER_AGENT)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(
            "Accept-Language",
            format!(
                "{},{};q=0.9",
                request.language,
                request.language.split('-').next().unwrap_or("en")
            ),
        )
        .header("DNT", "1")
        .header("Upgrade-Insecure-Requests", "1")
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, _, final_url, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((Some(status), format!("HTTP {status}")));
    }
    if final_url.contains("/challenge")
        || body.contains("Our systems have detected unusual traffic")
        || body.contains("b_captcha")
    {
        return Err((Some(status), "Bing bot challenge".to_string()));
    }
    let mut hits = parse_bing_html_results(&body, 1);
    hits.truncate(candidate_limit);
    if hits.is_empty() {
        return Err((
            Some(status),
            "Bing returned no parseable HTML results".to_string(),
        ));
    }
    Ok((status, hits))
}

fn dashscope_mcp_config() -> Option<(String, String)> {
    let llm_endpoint = env::var("NOLA_LLM_ENDPOINT").ok().unwrap_or_default();
    let dedicated_key = env::var("NOLA_DASHSCOPE_WEB_SEARCH_KEY")
        .ok()
        .or_else(|| env::var("DASHSCOPE_API_KEY").ok())
        .filter(|value| !value.trim().is_empty());
    let llm_key = if is_dashscope_endpoint(&llm_endpoint) {
        env::var("NOLA_LLM_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
    } else {
        None
    };
    let api_key = dedicated_key.or(llm_key)?;
    let endpoint = env::var("NOLA_DASHSCOPE_WEB_SEARCH_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| dashscope_mcp_endpoint(&llm_endpoint))
        .unwrap_or_else(|| "https://dashscope.aliyuncs.com/api/v1/mcps/WebSearch/mcp".to_string());
    Some((api_key, endpoint))
}

fn is_dashscope_endpoint(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            host == "dashscope.aliyuncs.com" || host.ends_with(".maas.aliyuncs.com")
        })
}

fn dashscope_mcp_endpoint(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if host != "dashscope.aliyuncs.com" && !host.ends_with(".maas.aliyuncs.com") {
        return None;
    }
    url.set_path("/api/v1/mcps/WebSearch/mcp");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn search_dashscope_mcp(
    client: &Client,
    api_key: &str,
    endpoint: &str,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "nola", "version": env!("CARGO_PKG_VERSION")}
        }
    });
    let (status, session_id, initialize_body) =
        dashscope_mcp_post(client, endpoint, api_key, None, &initialize, true)?;
    parse_mcp_result(&initialize_body)?;
    let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let _ = dashscope_mcp_post(
        client,
        endpoint,
        api_key,
        session_id.as_deref(),
        &initialized,
        false,
    )?;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "bailian_web_search",
            "arguments": {"query": request.query, "count": request.result_count}
        }
    });
    let (_, _, call_body) = dashscope_mcp_post(
        client,
        endpoint,
        api_key,
        session_id.as_deref(),
        &call,
        true,
    )?;
    let result = parse_mcp_result(&call_body)?;
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err((
            Some(status),
            "DashScope WebSearch MCP returned an error result".to_string(),
        ));
    }
    let hits = parse_dashscope_mcp_pages(&result, request.result_count);
    if hits.is_empty() {
        return Err((
            Some(status),
            "DashScope WebSearch MCP returned no pages".to_string(),
        ));
    }
    Ok((status, hits))
}

fn dashscope_mcp_post(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    session_id: Option<&str>,
    payload: &Value,
    expect_body: bool,
) -> Result<(u16, Option<String>, String), (Option<u16>, String)> {
    let mut request = client
        .post(endpoint)
        .bearer_auth(api_key)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("User-Agent", concat!("Nola/", env!("CARGO_PKG_VERSION")));
    if let Some(session_id) = session_id {
        request = request.header("Mcp-Session-Id", session_id);
    }
    let response = request
        .json(payload)
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let status = response.status().as_u16();
    let response_session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| session_id.map(str::to_string));
    let (_, _, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if !(200..300).contains(&status) {
        return Err((
            Some(status),
            format!("HTTP {status}: {}", truncate(&body, 500)),
        ));
    }
    if expect_body && body.trim().is_empty() {
        return Err((
            Some(status),
            "DashScope MCP returned an empty response".to_string(),
        ));
    }
    Ok((status, response_session_id, body))
}

fn parse_mcp_result(body: &str) -> Result<Value, (Option<u16>, String)> {
    let candidate = if body.trim_start().starts_with('{') {
        body.trim().to_string()
    } else {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| line.starts_with('{'))
            .last()
            .unwrap_or("")
            .to_string()
    };
    let decoded: Value = serde_json::from_str(&candidate)
        .map_err(|err| (None, format!("invalid DashScope MCP JSON: {err}")))?;
    if let Some(error_value) = decoded.get("error") {
        return Err((
            None,
            format!(
                "DashScope MCP error: {}",
                truncate(&error_value.to_string(), 500)
            ),
        ));
    }
    decoded
        .get("result")
        .cloned()
        .ok_or_else(|| (None, "DashScope MCP response has no result".to_string()))
}

fn parse_dashscope_mcp_pages(result: &Value, limit: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in content {
        let Some(text) = item.get("text").and_then(Value::as_str) else {
            continue;
        };
        let Ok(decoded) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let pages = decoded
            .get("pages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for page in pages {
            let title = page
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let url = page
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let canonical = canonicalize_url(&url);
            if canonical.is_empty() || !seen.insert(canonical) {
                continue;
            }
            let snippet = page
                .get("snippet")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let source = page
                .get("hostname")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(hit) = search_hit(
                "dashscope_mcp",
                hits.len() + 1,
                title,
                url,
                snippet,
                None,
                source,
                None,
            ) {
                hits.push(hit);
            }
            if hits.len() >= limit {
                return hits;
            }
        }
    }
    hits
}

fn search_serper(
    client: &Client,
    key: &str,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let endpoint = env::var("NOLA_SERPER_URL")
        .unwrap_or_else(|_| "https://google.serper.dev/search".to_string());
    let mut payload = json!({
        "q": request.query,
        "num": request.result_count,
        "hl": request.language,
        "gl": request.region.to_lowercase(),
    });
    if let Some(value) = request.freshness.serper() {
        payload["tbs"] = Value::String(value.to_string());
    }
    let response = client
        .post(endpoint)
        .header("X-API-KEY", key)
        .header("Accept", "application/json")
        .json(&payload)
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, content_type, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((
            Some(status),
            format!("HTTP {status}: {}", truncate(&body, 240)),
        ));
    }
    if !content_type.contains("json") {
        return Err((Some(status), "unexpected non-JSON response".to_string()));
    }
    let decoded: SerperResponse = serde_json::from_str(&body)
        .map_err(|err| (Some(status), format!("invalid Serper JSON: {err}")))?;
    let hits = decoded
        .organic
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            search_hit(
                "serper",
                item.position.unwrap_or(index + 1),
                item.title,
                item.link,
                item.snippet.unwrap_or_default(),
                item.date,
                None,
                None,
            )
        })
        .take(request.result_count.saturating_add(10).min(50))
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Err((
            Some(status),
            "Serper returned no organic results".to_string(),
        ));
    }
    Ok((status, hits))
}

fn search_brave(
    client: &Client,
    key: &str,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let mut url = Url::parse("https://api.search.brave.com/res/v1/web/search")
        .map_err(|err| (None, err.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &request.query);
        query.append_pair("count", &request.result_count.to_string());
        query.append_pair("search_lang", &request.language);
        query.append_pair("country", &request.region);
        query.append_pair("safesearch", "moderate");
        if let Some(value) = request.freshness.brave() {
            query.append_pair("freshness", value);
        }
    }
    let response = client
        .get(url)
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, content_type, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((
            Some(status),
            format!("HTTP {status}: {}", truncate(&body, 240)),
        ));
    }
    if !content_type.contains("json") {
        return Err((Some(status), "unexpected non-JSON response".to_string()));
    }
    let decoded: BraveResponse = serde_json::from_str(&body)
        .map_err(|err| (Some(status), format!("invalid Brave JSON: {err}")))?;
    let hits = decoded
        .web
        .map(|web| web.results)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            search_hit(
                "brave",
                index + 1,
                item.title,
                item.url,
                item.description.unwrap_or_default(),
                item.age.or(item.page_age),
                None,
                None,
            )
        })
        .take(request.result_count)
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Err((Some(status), "Brave returned no web results".to_string()));
    }
    Ok((status, hits))
}

fn search_searxng(
    client: &Client,
    endpoint: &str,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let mut url =
        Url::parse(endpoint).map_err(|err| (None, format!("invalid SearXNG URL: {err}")))?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/search");
    }
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &request.query);
        query.append_pair("format", "json");
        query.append_pair("categories", "general");
        query.append_pair("language", &request.language);
        if let Some(value) = request.freshness.searxng() {
            query.append_pair("time_range", value);
        }
    }
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, content_type, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((
            Some(status),
            format!("HTTP {status}: {}", truncate(&body, 240)),
        ));
    }
    if !content_type.contains("json") {
        return Err((Some(status), "unexpected non-JSON response".to_string()));
    }
    let decoded: SearxngResponse = serde_json::from_str(&body)
        .map_err(|err| (Some(status), format!("invalid SearXNG JSON: {err}")))?;
    let hits = decoded
        .results
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            search_hit(
                "searxng",
                index + 1,
                item.title,
                item.url,
                item.content.unwrap_or_default(),
                item.published_date,
                item.engine,
                None,
            )
        })
        .take(request.result_count)
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Err((Some(status), "SearXNG returned no results".to_string()));
    }
    Ok((status, hits))
}

fn search_google_news(
    client: &Client,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let mut url =
        Url::parse("https://news.google.com/rss/search").map_err(|err| (None, err.to_string()))?;
    let language = normalized_google_news_language(&request.language);
    let region = request.region.to_uppercase();
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("q", &request.query);
        query.append_pair("hl", &language);
        query.append_pair("gl", &region);
        query.append_pair("ceid", &format!("{region}:{language}"));
    }
    let response = client
        .get(url)
        .header("Accept", "application/rss+xml, application/xml")
        .send()
        .map_err(|err| (None, err.to_string()))?;
    let (status, content_type, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
    if status != 200 {
        return Err((Some(status), format!("HTTP {status}")));
    }
    if !content_type.contains("xml") && !body.trim_start().starts_with("<?xml") {
        return Err((Some(status), "unexpected non-RSS response".to_string()));
    }
    let rss: GoogleNewsRss = parse_xml(&body)
        .map_err(|err| (Some(status), format!("invalid Google News RSS: {err}")))?;
    let hits = rss
        .channel
        .items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let source_name = item.source.as_ref().and_then(|source| source.name.clone());
            let source_url = item.source.as_ref().and_then(|source| source.url.clone());
            search_hit(
                "google_news",
                index + 1,
                item.title,
                item.link,
                item.description.unwrap_or_default(),
                item.pub_date,
                source_name,
                source_url,
            )
        })
        .take(request.result_count)
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Err((
            Some(status),
            "Google News returned no RSS items".to_string(),
        ));
    }
    Ok((status, resolve_google_news_hits(client, hits, request)))
}

fn resolve_google_news_hits(
    client: &Client,
    hits: Vec<SearchHit>,
    request: &SearchRequest,
) -> Vec<SearchHit> {
    let jobs = Arc::new(Mutex::new(VecDeque::from(hits)));
    let results = Arc::new(Mutex::new(Vec::new()));
    let worker_count = PAGE_CONCURRENCY.min(
        jobs.lock()
            .map(|queue| queue.len())
            .unwrap_or_default()
            .max(1),
    );
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let client = client.clone();
            let jobs = Arc::clone(&jobs);
            let results = Arc::clone(&results);
            let request = request.clone();
            scope.spawn(move || loop {
                let hit = jobs.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some(hit) = hit else { break };
                let resolved = resolve_google_news_hit(&client, &hit, &request).unwrap_or(hit);
                if let Ok(mut output) = results.lock() {
                    output.push(resolved);
                }
            });
        }
    });
    let mut output = results
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    output.sort_by_key(|hit| hit.provider_rank);
    output
}

fn resolve_google_news_hit(
    client: &Client,
    hit: &SearchHit,
    _request: &SearchRequest,
) -> Option<SearchHit> {
    if !is_search_wrapper_url(&hit.url) {
        return Some(hit.clone());
    }
    let response = client
        .get(&hit.url)
        .header("Accept", "text/html")
        .send()
        .ok()?;
    let (status, _, _, article_body) = read_response(response, SEARCH_RESPONSE_BYTES).ok()?;
    if status != 200 {
        return None;
    }
    let document = Html::parse_document(&article_body);
    let metadata_selector = Selector::parse("[data-n-a-id][data-n-a-ts][data-n-a-sg]").ok()?;
    let metadata = document.select(&metadata_selector).next()?;
    let article_id = metadata.value().attr("data-n-a-id")?;
    let timestamp = metadata.value().attr("data-n-a-ts")?.parse::<i64>().ok()?;
    let signature = metadata.value().attr("data-n-a-sg")?;
    let inner = json!([
        "garturlreq",
        [
            [
                "en-US",
                "US",
                ["FINANCE_TOP_INDICES", "WEB_TEST_1_0_0"],
                null,
                null,
                1,
                1,
                "US:en",
                null,
                180,
                null,
                null,
                null,
                null,
                null,
                0
            ],
            "en-US",
            "US",
            1,
            [2, 3, 4, 8],
            1,
            0,
            "655000234",
            0,
            0,
            null,
            0
        ],
        article_id,
        timestamp,
        signature
    ])
    .to_string();
    let payload = json!([[["Fbv4je", inner, null, "generic"]]]).to_string();
    let response = client
        .post("https://news.google.com/_/DotsSplashUi/data/batchexecute")
        .header("Accept", "application/json")
        .form(&[("f.req", payload)])
        .send()
        .ok()?;
    let (status, _, _, rpc_body) = read_response(response, SEARCH_RESPONSE_BYTES).ok()?;
    if status != 200 {
        return None;
    }
    let response_start = rpc_body.find("[[")?;
    let decoded: Value = serde_json::from_str(&rpc_body[response_start..]).ok()?;
    let encoded_result = decoded.get(0)?.get(2)?.as_str()?;
    let decoded_result: Value = serde_json::from_str(encoded_result).ok()?;
    if decoded_result.get(0)?.as_str()? != "garturlres" {
        return None;
    }
    let target = decoded_result.get(1)?.as_str()?.to_string();
    if is_search_wrapper_url(&target) || validate_public_web_url(&target, "resolved_url").is_err() {
        return None;
    }
    let mut resolved = hit.clone();
    resolved.provider = "google_news_resolved".to_string();
    resolved.url = target;
    resolved.canonical_url = canonicalize_url(&resolved.url);
    Some(resolved)
}

fn search_duckduckgo(
    client: &Client,
    request: &SearchRequest,
) -> Result<(u16, Vec<SearchHit>), (Option<u16>, String)> {
    let mut hits = Vec::new();
    let mut last_status = 200_u16;
    for offset in [0_usize, 10_usize, 20_usize] {
        if hits.len() >= request.result_count {
            break;
        }
        let mut url = Url::parse("https://html.duckduckgo.com/html/")
            .map_err(|err| (None, err.to_string()))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("q", &request.query);
            query.append_pair("kl", &duckduckgo_region(&request.region, &request.language));
            if offset > 0 {
                query.append_pair("s", &offset.to_string());
                query.append_pair("dc", &(offset + 1).to_string());
            }
        }
        let response = client
            .get(url)
            .header("Accept", "text/html")
            .header("Accept-Language", &request.language)
            .send()
            .map_err(|err| (None, err.to_string()))?;
        let (status, _, _, body) = read_response(response, SEARCH_RESPONSE_BYTES)?;
        last_status = status;
        if status == 202 || looks_like_challenge(&body) {
            return Err((Some(status), "DuckDuckGo bot challenge".to_string()));
        }
        if status != 200 {
            return Err((Some(status), format!("HTTP {status}")));
        }
        let page = parse_duckduckgo_results(&body, hits.len() + 1);
        if page.is_empty() {
            break;
        }
        for hit in page {
            if !hits.iter().any(|existing: &SearchHit| {
                canonicalize_url(&existing.url) == canonicalize_url(&hit.url)
            }) {
                hits.push(hit);
            }
        }
    }
    hits.truncate(request.result_count);
    if hits.is_empty() {
        return Err((
            Some(last_status),
            "DuckDuckGo returned no parseable results".to_string(),
        ));
    }
    Ok((last_status, hits))
}

fn parse_ranked_pages(hits: &[SearchHit], request: &SearchRequest) -> Vec<PageAnalysis> {
    let jobs = Arc::new(Mutex::new(VecDeque::from(hits.to_vec())));
    let results = Arc::new(Mutex::new(Vec::with_capacity(hits.len())));
    let worker_count = PAGE_CONCURRENCY.min(hits.len().max(1));
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let jobs = Arc::clone(&jobs);
            let results = Arc::clone(&results);
            let request = request.clone();
            scope.spawn(move || {
                let client = build_client(request.research_mode.page_timeout_seconds());
                loop {
                    let hit = jobs.lock().ok().and_then(|mut queue| queue.pop_front());
                    let Some(hit) = hit else { break };
                    let page = match &client {
                        Ok(client) => fetch_and_parse_page(client, &hit, &request),
                        Err(err) => page_error(&hit, "client_error", None, err.clone()),
                    };
                    if let Ok(mut output) = results.lock() {
                        output.push(page);
                    }
                }
            });
        }
    });
    let mut pages = results
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    pages.sort_by_key(|page| page.search_rank);
    pages
}

fn fetch_and_parse_page(client: &Client, hit: &SearchHit, request: &SearchRequest) -> PageAnalysis {
    if let Err(outcome) = validate_public_web_url(&hit.url, "result_url") {
        return page_error(hit, "blocked", None, outcome.text);
    }
    let response = match client
        .get(&hit.url)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,text/plain,application/pdf;q=0.8,*/*;q=0.2",
        )
        .header("Accept-Language", &request.language)
        .send()
    {
        Ok(response) => response,
        Err(err) => return page_error(hit, "network_error", None, err.to_string()),
    };
    let final_url = response.url().as_str().to_string();
    if let Err(outcome) = validate_public_web_url(&final_url, "final_url") {
        return page_error(
            hit,
            "blocked_redirect",
            Some(response.status().as_u16()),
            outcome.text,
        );
    }
    let (status, content_type, _, bytes) = match read_response_bytes(response, PAGE_RESPONSE_BYTES)
    {
        Ok(value) => value,
        Err((status, detail)) => {
            return page_error(hit, "read_error", status, detail);
        }
    };
    if !(200..300).contains(&status) {
        return page_error(hit, "http_error", Some(status), format!("HTTP {status}"));
    }
    let lowered_type = content_type.to_lowercase();
    if lowered_type.contains("pdf") || bytes.starts_with(b"%PDF") {
        return page_error(
            hit,
            "unsupported_document",
            Some(status),
            "PDF result requires the document extraction pipeline".to_string(),
        );
    }
    let raw = String::from_utf8_lossy(&bytes).to_string();
    let parsed = if lowered_type.contains("html") || raw.trim_start().starts_with('<') {
        parse_html_document(&raw, hit)
    } else if lowered_type.contains("text") || lowered_type.is_empty() {
        ParsedDocument {
            title: hit.title.clone(),
            canonical_url: None,
            author: None,
            published_at: hit.published_at.clone(),
            modified_at: None,
            paragraphs: raw
                .lines()
                .map(normalize_text)
                .filter(|value| value.chars().count() >= 40)
                .collect(),
        }
    } else {
        return page_error(hit, "unsupported_content_type", Some(status), content_type);
    };
    let full_text = parsed.paragraphs.join("\n\n");
    if full_text.chars().count() < 220 {
        let status_name = if looks_like_challenge(&raw) {
            "blocked_or_challenge"
        } else {
            "requires_browser_rendering"
        };
        return page_error(
            hit,
            status_name,
            Some(status),
            "No readable server-rendered body was found".to_string(),
        );
    }
    let page_title = if parsed.title.is_empty() {
        hit.title.clone()
    } else {
        parsed.title.clone()
    };
    let relevance_score =
        document_relevance_score(&request.query, &page_title, &hit.snippet, &full_text);
    let evidence = if relevance_score > 0 {
        representative_evidence_for_query(&parsed.paragraphs, &request.query, 3)
    } else {
        Vec::new()
    };
    let published_at = parsed.published_at.or_else(|| hit.published_at.clone());
    let freshness_score = freshness_score(published_at.as_deref(), request.freshness);
    let domain = domain_of(&final_url);
    let source_score = source_score(&domain);
    let evidence_score = freshness_score + source_score + relevance_score;
    PageAnalysis {
        search_rank: hit.search_rank,
        provider: hit.provider.clone(),
        title: page_title,
        url: hit.url.clone(),
        final_url: Some(final_url),
        canonical_url: parsed.canonical_url,
        domain,
        status: "parsed".to_string(),
        http_status: Some(status),
        content_type: Some(content_type),
        author: parsed.author,
        published_at,
        modified_at: parsed.modified_at,
        content_chars: full_text.chars().count(),
        freshness_score,
        source_score,
        evidence_score,
        evidence,
        content_excerpt: full_text.chars().take(PAGE_TEXT_CHARS).collect(),
        error: None,
    }
}

#[derive(Debug)]
struct ParsedDocument {
    title: String,
    canonical_url: Option<String>,
    author: Option<String>,
    published_at: Option<String>,
    modified_at: Option<String>,
    paragraphs: Vec<String>,
}

fn parse_html_document(raw: &str, hit: &SearchHit) -> ParsedDocument {
    let document = Html::parse_document(raw);
    let title = metadata_value(&document, &["og:title", "twitter:title"])
        .or_else(|| selector_text(&document, "title"))
        .unwrap_or_else(|| hit.title.clone());
    let canonical_url = selector_attr(&document, "link[rel='canonical']", "href")
        .filter(|value| validate_public_web_url(value, "canonical_url").is_ok());
    let author = metadata_value(&document, &["author", "article:author", "byl"]);
    let published_at = metadata_value(
        &document,
        &[
            "article:published_time",
            "datepublished",
            "date",
            "pubdate",
            "publishdate",
        ],
    )
    .or_else(|| selector_attr(&document, "time[datetime]", "datetime"));
    let modified_at = metadata_value(
        &document,
        &["article:modified_time", "datemodified", "lastmod"],
    );
    let mut paragraphs = Vec::new();
    for css in [
        "article p, article h2, article h3, article blockquote, article li",
        "main p, main h2, main h3, main blockquote, main li",
        "[role='main'] p, [role='main'] h2, [role='main'] h3, [role='main'] blockquote",
        ".article p, .article h2, .article h3, .post p, .entry-content p",
        "p, h2, h3, blockquote",
    ] {
        let collected = collect_selector_text(&document, css);
        merge_unique_paragraphs(&mut paragraphs, collected);
        if paragraphs
            .iter()
            .map(|value| value.chars().count())
            .sum::<usize>()
            >= 1_200
        {
            break;
        }
    }
    ParsedDocument {
        title,
        canonical_url,
        author,
        published_at,
        modified_at,
        paragraphs,
    }
}

fn format_report(report: &SearchReport) -> String {
    const TARGET_REPORT_CHARS: usize = 14_000;
    let mut ranked_evidence = report
        .results
        .iter()
        .filter(|page| !page.evidence.is_empty() && page.evidence_score > 0)
        .collect::<Vec<_>>();
    ranked_evidence.sort_by(|left, right| {
        right
            .evidence_score
            .cmp(&left.evidence_score)
            .then_with(|| left.search_rank.cmp(&right.search_rank))
    });
    let mut lines = vec![
        "# Web search and parsed-page evidence".to_string(),
        format!("Status: {}", report.status),
        format!("Query: {}", report.query),
        format!("Searched at: {}", report.searched_at),
        format!(
            "Retrieved ranked pages: {}/{}; parsed: {}; useful evidence pages: {}",
            report.retrieved_results,
            report.requested_results,
            report.parsed_pages,
            report.useful_pages
        ),
        "Search rank is preserved separately from evidence quality rank.".to_string(),
        "Evidence contract: search rank, page relevance, and source quality identify candidate evidence; they do not validate a claim. Include a candidate in a filtered answer only when an extracted excerpt explicitly supports its identity and every required field at the searched timestamp. Missing or unknown required fields fail verification and the candidate must be omitted, not presented with an unknown placeholder.".to_string(),
    ];
    if !report.warnings.is_empty() {
        lines.push(format!("Warnings: {}", report.warnings.join(" | ")));
    }
    lines.push("\n## Original search ranking and source index".to_string());
    for page in &report.results {
        lines.push(format!(
            "{}. [{}] {} | Published: {} | URL: {}",
            page.search_rank,
            page.status,
            truncate(&page.title, 72),
            page.published_at.as_deref().unwrap_or("unknown"),
            page.final_url.as_deref().unwrap_or(&page.url),
        ));
    }
    lines.push("\n## Best available evidence".to_string());
    for page in ranked_evidence.iter().take(6) {
        let evidence = page
            .evidence
            .iter()
            .next()
            .map(|item| truncate(item, 320))
            .unwrap_or_default();
        let entry = format!(
            "\n[rank {}; score {}] {} | Published: {}\nURL: {}\nEvidence: {}",
            page.search_rank,
            page.evidence_score,
            truncate(&page.title, 96),
            page.published_at.as_deref().unwrap_or("unknown"),
            page.final_url.as_deref().unwrap_or(&page.url),
            evidence
        );
        let current_chars = lines
            .iter()
            .map(|line| line.chars().count() + 1)
            .sum::<usize>();
        if current_chars + entry.chars().count() + 1_000 > TARGET_REPORT_CHARS {
            break;
        }
        lines.push(entry);
    }
    lines.push("\n## Provider attempts".to_string());
    for attempt in &report.provider_attempts {
        lines.push(format!(
            "- {}: status={}, HTTP={}, results={}, latency={}ms{}",
            attempt.provider,
            attempt.status,
            attempt
                .http_status
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            attempt.accepted_results,
            attempt.latency_ms,
            attempt
                .detail
                .as_ref()
                .map(|value| format!(", detail={value}"))
                .unwrap_or_default()
        ));
    }
    lines.push(
        "\nThis report is self-contained. Cite the URLs above and answer from the extracted evidence. Use web_fetch on one specific cited URL only when a missing detail is essential; do not page through the tool-output cache."
            .to_string(),
    );
    lines.join("\n")
}

#[cfg(test)]
#[test]
fn formatted_search_report_is_self_contained_and_keeps_twenty_sources() {
    let results = (1..=20)
        .map(|rank| PageAnalysis {
            search_rank: rank,
            provider: "fixture".to_string(),
            title: format!("Source {rank} with a useful title"),
            url: format!("https://source{rank}.example/news/item-{rank}"),
            final_url: None,
            canonical_url: None,
            domain: format!("source{rank}.example"),
            status: "parsed".to_string(),
            http_status: Some(200),
            content_type: Some("text/html".to_string()),
            author: None,
            published_at: Some("2026-07-16T00:00:00Z".to_string()),
            modified_at: None,
            content_chars: 1_200,
            freshness_score: 10,
            source_score: 10,
            evidence_score: 20,
            evidence: vec![format!(
                "Evidence from source {rank}: {}",
                "relevant facts ".repeat(20)
            )],
            content_excerpt: "full body is intentionally not serialized".to_string(),
            error: None,
        })
        .collect::<Vec<_>>();
    let report = SearchReport {
        status: "ok".to_string(),
        query: "2026年7月伊朗局势".to_string(),
        searched_at: "2026-07-16T00:00:00Z".to_string(),
        freshness: Freshness::Month,
        research_mode: ResearchMode::Standard,
        requested_results: 20,
        retrieved_results: 20,
        parsed_pages: 20,
        useful_pages: 20,
        provider_attempts: vec![ProviderAttempt {
            provider: "fixture".to_string(),
            status: "ok".to_string(),
            http_status: Some(200),
            raw_results: 20,
            accepted_results: 20,
            latency_ms: 25,
            detail: None,
        }],
        warnings: Vec::new(),
        results,
    };
    let output = format_report(&report);
    assert!(output.contains("20. [parsed] Source 20"));
    assert!(output.contains("https://source20.example/news/item-20"));
    assert!(output.contains("This report is self-contained."));
    assert!(output.contains("Missing or unknown required fields fail verification"));
    assert!(!output.contains("Full structured parsed-page report"));
    assert!(!output.contains("content_excerpt"));
    assert!(output.chars().count() <= 14_000);
    assert!(
        output.find("Original search ranking").unwrap()
            < output.find("Best available evidence").unwrap()
    );
    let evidence = model_source_evidence(&report);
    assert_eq!(evidence["sources"].as_array().unwrap().len(), 20);
    assert_eq!(evidence["sources_omitted"], 0);
    let safe = nola_domain::tool_evidence::ModelToolEvidence::from_host_output(&evidence).into_value();
    assert_eq!(safe, evidence);
    assert!(safe["sources"].as_array().unwrap().iter().any(|source| source["url"] == "https://source20.example/news/item-20"));
    assert!(safe["structured_preview"].to_string().chars().count() <= 384);
    assert!(!safe["structured_preview"]["sources"].as_array().unwrap().is_empty());
    assert!(safe.to_string().chars().count() <= 12_000);
    let mut oversized_url = report;
    oversized_url.results[0].url = format!("https://source1.example/{}", "path".repeat(5_000));
    let bounded = model_source_evidence(&oversized_url);
    assert_eq!(bounded["sources_omitted"], 1, "An over-budget URL is omitted explicitly, never shortened into a different link");
    assert!(!bounded["sources"].as_array().unwrap().iter().any(|source| source["source_id"] == "search-rank:1"));
}

#[test]
fn relevance_scoring_rejects_same_name_brand_noise() {
    let query = "Boost.Geometry boolean operations union intersection difference logic";
    assert_eq!(
        document_relevance_score(
            query,
            "Phone Plans & Best New Phones | Boost Mobile",
            "Manage your monthly wireless bill",
            "Choose a mobile plan and pay your phone account online."
        ),
        0
    );
    assert!(document_relevance_score(
        query,
        "Boost.Geometry union_ implementation",
        "Boolean overlay algorithms for polygon union and intersection",
        "The geometry overlay implementation computes turns and traverses rings for difference."
    ) > 0);
    assert_eq!(
        document_relevance_score(
            query,
            "How do I perform an advanced search?",
            "Help center documentation",
            "The submitted query was Boost.Geometry boolean operations union intersection difference logic."
        ),
        0,
        "A page that only echoes the query in its body is not evidence"
    );
}

#[test]
fn ranked_search_candidates_reject_unrelated_navigation_results() {
    let relevant = SearchHit {
        search_rank: 1,
        provider_rank: 1,
        provider: "fixture".to_string(),
        title: "天猫超市组织架构调整".to_string(),
        url: "https://example.com/tmall".to_string(),
        canonical_url: "https://example.com/tmall".to_string(),
        snippet: "阿里巴巴旗下天猫超市业务团队信息".to_string(),
        published_at: None,
        source_name: None,
        source_url: None,
    };
    let unrelated = SearchHit {
        title: "Anmelden bei Hotmail | Microsoft Support".to_string(),
        snippet: "Manage your Microsoft account and Windows services".to_string(),
        url: "https://support.microsoft.com/hotmail".to_string(),
        canonical_url: "https://support.microsoft.com/hotmail".to_string(),
        ..relevant.clone()
    };

    assert!(search_hit_matches_query(
        "阿里巴巴天猫超市组织架构",
        &relevant
    ));
    assert!(!search_hit_matches_query(
        "阿里巴巴天猫超市组织架构",
        &unrelated
    ));
}

#[test]
fn relevance_scoring_supports_cjk_without_language_specific_rules() {
    let query = "2026年7月伊朗局势";
    assert_eq!(
        document_relevance_score(query, "手机促销", "七月优惠", "购买新手机可获折扣"),
        0
    );
    assert!(
        document_relevance_score(
            query,
            "2026年7月伊朗局势更新",
            "伊朗与地区各方的最新动态",
            "报道梳理了地区安全局势。"
        ) > 0
    );
}

#[test]
fn query_aware_evidence_prefers_relevant_paragraphs() {
    let paragraphs = vec![
        "This generic company introduction is deliberately long enough to be parsed but contains no matching technical material at all.".to_string(),
        "Boost.Geometry union and intersection share polygon overlay machinery that identifies turns and traverses output rings during boolean operations.".to_string(),
    ];
    let evidence = representative_evidence_for_query(
        &paragraphs,
        "Boost.Geometry boolean operations union intersection",
        2,
    );
    assert_eq!(evidence.len(), 1);
    assert!(evidence[0].contains("overlay machinery"));
}

fn page_without_fetch(hit: &SearchHit) -> PageAnalysis {
    PageAnalysis {
        search_rank: hit.search_rank,
        provider: hit.provider.clone(),
        title: hit.title.clone(),
        url: hit.url.clone(),
        final_url: None,
        canonical_url: Some(hit.canonical_url.clone()),
        domain: domain_of(&hit.url),
        status: "not_fetched".to_string(),
        http_status: None,
        content_type: None,
        author: None,
        published_at: hit.published_at.clone(),
        modified_at: None,
        content_chars: 0,
        freshness_score: 0,
        source_score: source_score(&domain_of(&hit.url)),
        evidence_score: 0,
        evidence: Vec::new(),
        content_excerpt: String::new(),
        error: None,
    }
}

fn page_error(
    hit: &SearchHit,
    status: &str,
    http_status: Option<u16>,
    detail: String,
) -> PageAnalysis {
    let snippet = normalize_text(&hit.snippet);
    let snippet_evidence = if snippet.chars().count() >= 20 {
        vec![snippet.clone()]
    } else {
        Vec::new()
    };
    let domain = hit
        .source_url
        .as_deref()
        .map(domain_of)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| domain_of(&hit.url));
    let source_score = source_score(&domain);
    PageAnalysis {
        search_rank: hit.search_rank,
        provider: hit.provider.clone(),
        title: hit.title.clone(),
        url: hit.url.clone(),
        final_url: None,
        canonical_url: Some(hit.canonical_url.clone()),
        domain,
        status: status.to_string(),
        http_status,
        content_type: None,
        author: None,
        published_at: hit.published_at.clone(),
        modified_at: None,
        content_chars: snippet.chars().count(),
        freshness_score: 0,
        source_score,
        evidence_score: source_score - 20,
        evidence: snippet_evidence,
        content_excerpt: snippet,
        error: Some(detail),
    }
}

fn representative_evidence_for_query(
    paragraphs: &[String],
    query: &str,
    limit: usize,
) -> Vec<String> {
    let mut ranked = paragraphs
        .iter()
        .filter(|paragraph| paragraph.chars().count() >= 60)
        .map(|paragraph| (text_relevance_score(query, paragraph), paragraph))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.cmp(&left.0));
    ranked
        .into_iter()
        .filter(|(score, _)| *score > 0)
        .take(limit)
        .map(|(_, paragraph)| paragraph.chars().take(700).collect())
        .collect()
}

fn document_relevance_score(query: &str, title: &str, snippet: &str, body: &str) -> i32 {
    let terms = relevance_terms(query);
    if terms.is_empty() {
        return 1;
    }
    let title = normalized_search_text(title, 2_000);
    let snippet = normalized_search_text(snippet, 4_000);
    let body = normalized_search_text(body, PAGE_TEXT_CHARS);
    let mut matched = 0usize;
    let mut matched_search_metadata_count = 0usize;
    let mut matched_title_count = 0usize;
    let mut score = 0i32;
    let mut matched_search_metadata = false;
    for term in &terms {
        let term = normalized_search_text(term, 256);
        if term.is_empty() {
            continue;
        }
        let weight = if title.contains(&term) {
            matched_search_metadata = true;
            matched_search_metadata_count += 1;
            matched_title_count += 1;
            8
        } else if snippet.contains(&term) {
            matched_search_metadata = true;
            matched_search_metadata_count += 1;
            4
        } else if body.contains(&term) {
            2
        } else {
            continue;
        };
        matched += 1;
        score += weight;
    }
    // A single shared brand word (for example, "Boost") must not make an
    // unrelated page useful. Scale the gate with the number of concepts in
    // the original query while keeping short queries usable.
    let minimum_matches = match terms.len() {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        count => (count + 2) / 3,
    };
    let minimum_metadata_matches = if terms.len() > 5 { 2 } else { 1 };
    if matched < minimum_matches
        || !matched_search_metadata
        || matched_search_metadata_count < minimum_metadata_matches
        || (terms.len() > 5 && matched_title_count == 0)
    {
        0
    } else {
        score
    }
}

fn text_relevance_score(query: &str, text: &str) -> i32 {
    let terms = relevance_terms(query);
    let text = normalized_search_text(text, 4_000);
    terms
        .iter()
        .map(|term| normalized_search_text(term, 256))
        .filter(|term| !term.is_empty() && text.contains(term))
        .count() as i32
}

fn relevance_terms(value: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, terms: &mut Vec<String>| {
        if current.chars().count() >= 2 {
            terms.push(current.to_lowercase());
            if current.chars().any(is_cjk) && current.chars().count() > 2 {
                let characters = current.chars().collect::<Vec<_>>();
                for pair in characters.windows(2) {
                    terms.push(pair.iter().collect());
                }
            }
        }
        current.clear();
    };
    for character in value.chars() {
        if character.is_alphanumeric() {
            current.push(character);
        } else {
            flush(&mut current, &mut terms);
        }
    }
    flush(&mut current, &mut terms);
    terms.sort();
    terms.dedup();
    terms
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

fn normalized_search_text(value: &str, limit: usize) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .take(limit)
        .collect()
}

fn metadata_value(document: &Html, keys: &[&str]) -> Option<String> {
    let selector = Selector::parse("meta").ok()?;
    for element in document.select(&selector) {
        let value = element.value();
        let key = value
            .attr("property")
            .or_else(|| value.attr("name"))
            .or_else(|| value.attr("itemprop"))
            .unwrap_or("")
            .to_lowercase();
        if keys.iter().any(|candidate| key == candidate.to_lowercase()) {
            if let Some(content) = value
                .attr("content")
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(content.to_string());
            }
        }
    }
    None
}

fn selector_text(document: &Html, css: &str) -> Option<String> {
    let selector = Selector::parse(css).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| normalize_text(&element.text().collect::<Vec<_>>().join(" ")))
        .filter(|value| !value.is_empty())
}

fn selector_attr(document: &Html, css: &str, attribute: &str) -> Option<String> {
    let selector = Selector::parse(css).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr(attribute))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn collect_selector_text(document: &Html, css: &str) -> Vec<String> {
    let Ok(selector) = Selector::parse(css) else {
        return Vec::new();
    };
    document
        .select(&selector)
        .map(|element| normalize_text(&element.text().collect::<Vec<_>>().join(" ")))
        .filter(|value| value.chars().count() >= 40)
        .collect()
}

fn merge_unique_paragraphs(target: &mut Vec<String>, values: Vec<String>) {
    let mut seen = target
        .iter()
        .map(|value| normalized_fingerprint(value))
        .collect::<HashSet<_>>();
    for value in values {
        let fingerprint = normalized_fingerprint(&value);
        if !fingerprint.is_empty() && seen.insert(fingerprint) {
            target.push(value);
        }
    }
}

fn search_hit(
    provider: &str,
    provider_rank: usize,
    title: String,
    url: String,
    snippet: String,
    published_at: Option<String>,
    source_name: Option<String>,
    source_url: Option<String>,
) -> Option<SearchHit> {
    let title = normalize_text(&title);
    let url = normalize_redirect_url(&url);
    if title.is_empty() || validate_public_web_url(&url, "result_url").is_err() {
        return None;
    }
    Some(SearchHit {
        search_rank: 0,
        provider_rank,
        provider: provider.to_string(),
        title,
        canonical_url: canonicalize_url(&url),
        url,
        snippet: normalize_text(&snippet),
        published_at,
        source_name,
        source_url,
    })
}

fn parse_duckduckgo_results(body: &str, first_rank: usize) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let Ok(result_selector) = Selector::parse(".result") else {
        return Vec::new();
    };
    let Ok(anchor_selector) = Selector::parse("a.result__a") else {
        return Vec::new();
    };
    let snippet_selector = Selector::parse(".result__snippet").ok();
    let mut hits = Vec::new();
    for result in document.select(&result_selector) {
        let Some(anchor) = result.select(&anchor_selector).next() else {
            continue;
        };
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let title = normalize_text(&anchor.text().collect::<Vec<_>>().join(" "));
        let snippet = snippet_selector
            .as_ref()
            .and_then(|selector| result.select(selector).next())
            .map(|element| normalize_text(&element.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        if let Some(hit) = search_hit(
            "duckduckgo_html",
            first_rank + hits.len(),
            title,
            href.to_string(),
            snippet,
            None,
            None,
            None,
        ) {
            hits.push(hit);
        }
    }
    hits
}

fn parse_bing_html_results(body: &str, first_rank: usize) -> Vec<SearchHit> {
    let document = Html::parse_document(body);
    let Ok(result_selector) = Selector::parse("li.b_algo") else {
        return Vec::new();
    };
    let Ok(anchor_selector) = Selector::parse("h2 a") else {
        return Vec::new();
    };
    let snippet_selector = Selector::parse(".b_caption p").ok();
    let mut hits = Vec::new();
    for result in document.select(&result_selector) {
        let Some(anchor) = result.select(&anchor_selector).next() else {
            continue;
        };
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let title = normalize_text(&anchor.text().collect::<Vec<_>>().join(" "));
        let snippet = snippet_selector
            .as_ref()
            .and_then(|selector| result.select(selector).next())
            .map(|element| normalize_text(&element.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        if let Some(hit) = search_hit(
            "bing_web_html",
            first_rank + hits.len(),
            title,
            href.to_string(),
            snippet,
            None,
            None,
            None,
        ) {
            hits.push(hit);
        }
    }
    hits
}

fn normalize_redirect_url(raw: &str) -> String {
    let value = raw.trim();
    let absolute = if value.starts_with("//") {
        format!("https:{value}")
    } else if value.starts_with('/') {
        format!("https://duckduckgo.com{value}")
    } else {
        value.to_string()
    };
    let Ok(url) = Url::parse(&absolute) else {
        return absolute;
    };
    if url
        .host_str()
        .is_some_and(|host| host.ends_with("duckduckgo.com"))
        && url.path().starts_with("/l/")
    {
        if let Some((_, target)) = url.query_pairs().find(|(key, _)| key == "uddg") {
            return target.into_owned();
        }
    }
    if let Some(target) = decode_bing_redirect(&url) {
        return target;
    }
    absolute
}

fn decode_bing_redirect(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    if !(host == "bing.com" || host.ends_with(".bing.com")) || !url.path().starts_with("/ck/a") {
        return None;
    }
    let encoded = url
        .query_pairs()
        .find_map(|(key, value)| (key == "u").then(|| value.into_owned()))?;
    let payload = encoded.strip_prefix("a1").unwrap_or(&encoded);
    for engine in [&URL_SAFE_NO_PAD, &URL_SAFE, &STANDARD_NO_PAD, &STANDARD] {
        let Ok(decoded) = engine.decode(payload) else {
            continue;
        };
        let Ok(target) = String::from_utf8(decoded) else {
            continue;
        };
        if target.starts_with("https://") || target.starts_with("http://") {
            return Some(target);
        }
    }
    None
}

fn is_search_wrapper_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return true;
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    (host == "news.google.com" && url.path().contains("/articles/"))
        || ((host == "bing.com" || host.ends_with(".bing.com")) && url.path().starts_with("/ck/a"))
        || (host.ends_with("duckduckgo.com") && url.path().starts_with("/l/"))
}

fn canonicalize_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return raw.trim_end_matches('/').to_string();
    };
    url.set_fragment(None);
    let retained = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_lowercase();
            !key.starts_with("utm_")
                && !matches!(
                    key.as_str(),
                    "gclid" | "fbclid" | "mc_cid" | "mc_eid" | "ref"
                )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !retained.is_empty() {
        url.query_pairs_mut().extend_pairs(retained);
    }
    let mut value = url.to_string();
    if value.ends_with('/') && url.path() == "/" && url.query().is_none() {
        value.pop();
    }
    value
}

fn build_client(timeout_seconds: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .connect_timeout(Duration::from_secs(8))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 6 {
                return attempt.stop();
            }
            let blocked = attempt
                .url()
                .host_str()
                .is_none_or(is_blocked_public_web_host);
            if blocked {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent("Nola/1.0 WebSearch (+https://nola.local)")
        .build()
        .map_err(|err| err.to_string())
}

fn read_response(
    response: Response,
    max_bytes: u64,
) -> Result<(u16, String, String, String), (Option<u16>, String)> {
    let (status, content_type, final_url, bytes) = read_response_bytes(response, max_bytes)?;
    Ok((
        status,
        content_type,
        final_url,
        String::from_utf8_lossy(&bytes).to_string(),
    ))
}

fn read_response_bytes(
    mut response: Response,
    max_bytes: u64,
) -> Result<(u16, String, String, Vec<u8>), (Option<u16>, String)> {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let final_url = response.url().as_str().to_string();
    let mut bytes = Vec::new();
    Read::by_ref(&mut response)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| (Some(status), err.to_string()))?;
    if bytes.len() as u64 > max_bytes {
        return Err((Some(status), format!("response exceeded {max_bytes} bytes")));
    }
    Ok((status, content_type, final_url, bytes))
}

fn source_score(domain: &str) -> i32 {
    if domain.ends_with(".gov")
        || domain.contains(".gov.")
        || domain.ends_with(".edu")
        || domain.ends_with(".int")
    {
        return 35;
    }
    let trusted = [
        "reuters.com",
        "apnews.com",
        "bbc.com",
        "bbc.co.uk",
        "bloomberg.com",
        "wsj.com",
        "nytimes.com",
        "washingtonpost.com",
        "npr.org",
        "theguardian.com",
        "politico.com",
        "axios.com",
        "xinhuanet.com",
        "people.com.cn",
        "cctv.com",
        "caixin.com",
    ];
    if trusted
        .iter()
        .any(|value| domain == *value || domain.ends_with(&format!(".{value}")))
    {
        return 25;
    }
    if domain.ends_with(".org") {
        return 10;
    }
    0
}

fn freshness_score(value: Option<&str>, requested: Freshness) -> i32 {
    if requested == Freshness::None {
        return 0;
    }
    let Some(value) = value else { return -12 };
    let Some(date) = parse_date(value) else {
        return -8;
    };
    let age = Utc::now()
        .date_naive()
        .signed_duration_since(date)
        .num_days();
    let window = match requested {
        Freshness::None => i64::MAX,
        Freshness::Day => 1,
        Freshness::Week => 7,
        Freshness::Month => 31,
        Freshness::Year => 366,
    };
    if age < 0 {
        -10
    } else if age <= window {
        35
    } else if age <= window.saturating_mul(2) {
        8
    } else {
        -25
    }
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Some(value.date_naive());
    }
    if let Ok(value) = DateTime::parse_from_rfc2822(value) {
        return Some(value.date_naive());
    }
    for format in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d", "%Y年%m月%d日"] {
        if let Ok(value) = NaiveDate::parse_from_str(value.trim(), format) {
            return Some(value);
        }
    }
    None
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn normalized_fingerprint(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .take(240)
        .collect()
}

fn domain_of(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string()
}

fn optional_string(
    args: &Map<String, Value>,
    key: &str,
    default: &str,
) -> Result<String, ToolOutcome> {
    match args.get(key) {
        None => Ok(default.to_string()),
        Some(Value::String(value)) if !value.trim().is_empty() && value.chars().count() <= 100 => {
            Ok(value.trim().to_string())
        }
        Some(Value::String(_)) => Err(error(format!("{key} is invalid"))),
        Some(_) => Err(error(format!("{key} must be a string"))),
    }
}

fn default_search_language() -> String {
    let locale = env::var("LC_ALL")
        .or_else(|_| env::var("LC_MESSAGES"))
        .or_else(|_| env::var("LANG"))
        .unwrap_or_else(|_| "en-US".to_string());
    let language = locale
        .split('.')
        .next()
        .unwrap_or("en-US")
        .replace('_', "-");
    if language.is_empty() || matches!(language.as_str(), "C" | "POSIX") {
        "en-US".to_string()
    } else {
        language
    }
}

fn default_search_region(language: &str) -> String {
    language
        .split(['-', '_'])
        .nth(1)
        .filter(|value| value.len() == 2)
        .unwrap_or("US")
        .to_uppercase()
}

fn normalized_google_news_language(language: &str) -> String {
    let lowered = language.to_lowercase();
    if lowered.starts_with("zh") {
        "zh-CN".to_string()
    } else {
        language.to_string()
    }
}

fn duckduckgo_region(region: &str, language: &str) -> String {
    let language = language
        .split(['-', '_'])
        .next()
        .unwrap_or("en")
        .to_lowercase();
    format!("{}-{}", region.to_lowercase(), language)
}

fn looks_like_challenge(body: &str) -> bool {
    let lowered = body.to_lowercase();
    [
        "captcha",
        "unusual traffic",
        "verify you are human",
        "bot challenge",
        "automated requests",
        "enable javascript and cookies",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn classify_provider_error(detail: &str) -> &'static str {
    let lowered = detail.to_lowercase();
    if lowered.contains("challenge") || lowered.contains("blocked") || lowered.contains("captcha") {
        "provider_blocked"
    } else if lowered.contains("timeout") {
        "timeout"
    } else if lowered.contains("no ") {
        "no_results"
    } else {
        "error"
    }
}

fn format_provider_failure(query: &str, attempts: &[ProviderAttempt]) -> String {
    let attempts = attempts
        .iter()
        .map(|attempt| {
            format!(
                "{}: {}{}",
                attempt.provider,
                attempt.status,
                attempt
                    .detail
                    .as_ref()
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Web search could not retrieve ranked pages for '{query}'. Provider attempts: {attempts}. Do not answer from model memory."
    )
}

fn utc_now_string() -> String {
    DateTime::<Utc>::from(SystemTime::now()).to_rfc3339()
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[derive(Debug, Deserialize)]
struct SerperResponse {
    #[serde(default)]
    organic: Vec<SerperItem>,
}

#[derive(Debug, Deserialize)]
struct SerperItem {
    position: Option<usize>,
    title: String,
    link: String,
    snippet: Option<String>,
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: Option<BraveWeb>,
}

#[derive(Debug, Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveItem>,
}

#[derive(Debug, Deserialize)]
struct BraveItem {
    title: String,
    url: String,
    description: Option<String>,
    age: Option<String>,
    page_age: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngItem>,
}

#[derive(Debug, Deserialize)]
struct SearxngItem {
    title: String,
    url: String,
    content: Option<String>,
    #[serde(rename = "publishedDate")]
    published_date: Option<String>,
    engine: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleNewsRss {
    channel: GoogleNewsChannel,
}

#[derive(Debug, Deserialize)]
struct GoogleNewsChannel {
    #[serde(rename = "item", default)]
    items: Vec<GoogleNewsItem>,
}

#[derive(Debug, Deserialize)]
struct GoogleNewsItem {
    title: String,
    link: String,
    description: Option<String>,
    #[serde(rename = "pubDate")]
    pub_date: Option<String>,
    source: Option<GoogleNewsSource>,
}

#[derive(Debug, Deserialize)]
struct GoogleNewsSource {
    #[serde(rename = "@url")]
    url: Option<String>,
    #[serde(rename = "$text")]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write as _};
    use std::net::TcpListener;

    #[test]
    fn empty_and_partial_http_results_finish_without_browser_escalation() {
        for query in ["current research", "检索近期论文", "search current directory"] {
            let args = json!({"query":query});
            let mut request = SearchRequest::parse(args.as_object().unwrap()).unwrap();
            request.parse_pages = false;
            let empty = finish_search(&request, vec![], vec![], vec![]);
            assert!(empty.error);
            let hit = SearchHit {
                search_rank: 1, provider_rank: 1, provider: "fixture_http".into(),
                title: query.into(), url: "https://example.com/evidence".into(),
                canonical_url: "https://example.com/evidence".into(), snippet: query.into(),
                published_at: None, source_name: None, source_url: None,
            };
            let partial = finish_search(&request, vec![], vec![vec![hit]], vec![]);
            assert!(!partial.error, "{}", partial.text);
            assert!(partial.text.contains("Status: partial"));
            assert!(partial.display.contains("1/20"));
        }
    }

    #[test]
    fn duckduckgo_protocol_relative_redirect_is_unwrapped() {
        let value = normalize_redirect_url(
            "//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2Fch04-00-understanding-ownership.html",
        );
        assert_eq!(
            value,
            "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html"
        );
    }

    #[test]
    fn dashscope_endpoint_reuses_configured_origin() {
        assert_eq!(
            dashscope_mcp_endpoint("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            Some("https://dashscope.aliyuncs.com/api/v1/mcps/WebSearch/mcp".to_string())
        );
        assert_eq!(
            dashscope_mcp_endpoint(
                "https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"
            ),
            Some(
                "https://workspace.cn-beijing.maas.aliyuncs.com/api/v1/mcps/WebSearch/mcp"
                    .to_string()
            )
        );
        assert_eq!(dashscope_mcp_endpoint("https://api.openai.com/v1"), None);
    }

    #[test]
    fn parses_twenty_dashscope_mcp_pages_without_reordering() {
        let pages = (1..=20)
            .map(|rank| {
                json!({
                    "title": format!("Result {rank}"),
                    "url": format!("https://example{rank}.com/article"),
                    "snippet": format!("Evidence {rank}"),
                    "hostname": format!("example{rank}.com")
                })
            })
            .collect::<Vec<_>>();
        let result = json!({
            "content": [{
                "type": "text",
                "text": json!({"pages": pages}).to_string()
            }]
        });
        let hits = parse_dashscope_mcp_pages(&result, 20);
        assert_eq!(hits.len(), 20);
        assert_eq!(hits.first().map(|hit| hit.title.as_str()), Some("Result 1"));
        assert_eq!(hits.last().map(|hit| hit.title.as_str()), Some("Result 20"));
        assert_eq!(
            hits.iter().map(|hit| hit.provider_rank).collect::<Vec<_>>(),
            (1..=20).collect::<Vec<_>>()
        );
    }

    #[test]
    fn dashscope_mcp_protocol_calls_search_with_twenty_results() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock MCP");
        let endpoint = format!(
            "http://{}/mcp",
            listener.local_addr().expect("mock address")
        );
        let server = thread::spawn(move || {
            let pages = (1..=20)
                .map(|rank| {
                    json!({
                        "title": format!("Page {rank}"),
                        "url": format!("https://source{rank}.example/story"),
                        "snippet": format!("Snippet {rank}"),
                        "hostname": format!("source{rank}.example")
                    })
                })
                .collect::<Vec<_>>();
            let tool_text = json!({"pages": pages}).to_string();
            let responses = vec![
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"mock","version":"1"}}}).to_string(),
                String::new(),
                json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":tool_text}],"isError":false}}).to_string(),
            ];
            let mut requests = Vec::new();
            for (index, response_body) in responses.into_iter().enumerate() {
                let (stream, _) = listener.accept().expect("accept mock request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                let mut reader = BufReader::new(stream);
                let mut headers = String::new();
                let mut content_length = 0_usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("read header");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = value.trim().parse().expect("content length");
                    }
                    headers.push_str(&line);
                }
                let mut body = vec![0_u8; content_length];
                reader.read_exact(&mut body).expect("read request body");
                requests.push(format!("{headers}\n{}", String::from_utf8_lossy(&body)));
                let status = if index == 1 { "202 Accepted" } else { "200 OK" };
                let session_header = if index == 0 {
                    "Mcp-Session-Id: session-1\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{session_header}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(), response_body
                );
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .expect("write mock response");
            }
            requests
        });
        let request = SearchRequest {
            query: "latest statement".to_string(),
            result_count: 20,
            engine: "dashscope".to_string(),
            freshness: Freshness::Week,
            language: "en-US".to_string(),
            region: "US".to_string(),
            parse_pages: false,
            research_mode: ResearchMode::Standard,
        };
        let client = build_client(5).expect("build MCP client");
        let (_, hits) = search_dashscope_mcp(&client, "test-key", &endpoint, &request)
            .expect("mock MCP search");
        let requests = server.join().expect("join mock MCP");
        assert_eq!(hits.len(), 20);
        assert!(requests[0].contains("\"method\":\"initialize\""));
        assert!(requests[1].contains("notifications/initialized"));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("mcp-session-id: session-1"));
        assert!(requests[2].contains("\"name\":\"bailian_web_search\""));
        assert!(requests[2].contains("\"count\":20"));
    }

    #[test]
    fn parses_serper_top_twenty_without_changing_rank() {
        let organic = (1..=20)
            .map(|rank| {
                json!({
                    "position": rank,
                    "title": format!("Result {rank}"),
                    "link": format!("https://example{rank}.com/page"),
                    "snippet": "Evidence"
                })
            })
            .collect::<Vec<_>>();
        let decoded: SerperResponse = serde_json::from_value(json!({"organic": organic})).unwrap();
        assert_eq!(decoded.organic.len(), 20);
        assert_eq!(decoded.organic[19].position, Some(20));
    }

    #[test]
    fn html_parser_extracts_article_and_metadata() {
        let hit = SearchHit {
            search_rank: 1,
            provider_rank: 1,
            provider: "fixture".to_string(),
            title: "Fallback".to_string(),
            url: "https://example.com/report".to_string(),
            canonical_url: "https://example.com/report".to_string(),
            snippet: String::new(),
            published_at: None,
            source_name: None,
            source_url: None,
        };
        let html = r#"<html><head><meta property="og:title" content="Report"><meta property="article:published_time" content="2026-07-15T01:00:00Z"><link rel="canonical" href="https://example.com/report"></head><body><nav>noise</nav><article><p>This is the first substantial paragraph containing the requested entity and enough body text for extraction.</p><p>This is the second substantial paragraph with supporting evidence and additional useful context for the reader.</p><p>This is the third substantial paragraph so the document is clearly server rendered and parseable.</p></article></body></html>"#;
        let parsed = parse_html_document(html, &hit);
        assert_eq!(parsed.title, "Report");
        assert_eq!(parsed.paragraphs.len(), 3);
        assert_eq!(parsed.published_at.as_deref(), Some("2026-07-15T01:00:00Z"));
    }

    #[test]
    fn automatic_freshness_never_rewrites_raw_query_semantics() {
        assert_eq!(Freshness::parse(Some("auto")).unwrap(), Freshness::None);
        assert_eq!(Freshness::parse(None).unwrap(), Freshness::None);
        assert_eq!(Freshness::parse(Some("day")).unwrap(), Freshness::Day);
    }

    #[test]
    fn canonical_url_removes_tracking_but_keeps_meaningful_query() {
        let value =
            canonicalize_url("https://example.com/page?id=42&utm_source=test&fbclid=abc#section");
        assert_eq!(value, "https://example.com/page?id=42");
    }

    #[test]
    fn search_request_preserves_site_qualified_query_verbatim() {
        let mut args = Map::new();
        let query = "人工智能论文 site:arxiv.org";
        args.insert("query".to_string(), Value::String(query.to_string()));
        let request = SearchRequest::parse(&args).expect("parse search request");
        assert_eq!(request.query, query);
    }

    #[test]
    fn search_request_always_collects_top_twenty_candidates() {
        let mut args = Map::new();
        args.insert(
            "query".to_string(),
            Value::String("latest news".to_string()),
        );
        args.insert("limit".to_string(), Value::Number(1.into()));
        let request = SearchRequest::parse(&args).expect("parse search request");
        assert_eq!(request.result_count, SEARCH_RESULT_COUNT);
    }

    #[test]
    fn search_timeout_profile_is_explicit_and_defaults_to_standard() {
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String("fixture".to_string()));
        let standard = SearchRequest::parse(&args).expect("parse standard request");
        assert_eq!(standard.research_mode, ResearchMode::Standard);
        assert_eq!(
            standard.research_mode.page_timeout_seconds(),
            STANDARD_PAGE_TIMEOUT_SECONDS
        );

        args.insert(
            "research_mode".to_string(),
            Value::String("academic".to_string()),
        );
        let academic = SearchRequest::parse(&args).expect("parse academic request");
        assert_eq!(academic.research_mode, ResearchMode::Academic);
        assert!(
            academic.research_mode.page_timeout_seconds()
                > standard.research_mode.page_timeout_seconds()
        );
    }

    #[test]
    fn page_failure_preserves_serp_snippet_as_degraded_evidence() {
        let hit = SearchHit {
            search_rank: 1,
            provider_rank: 1,
            provider: "fixture".to_string(),
            title: "Current report".to_string(),
            url: "https://example.com/report".to_string(),
            canonical_url: "https://example.com/report".to_string(),
            snippet: "A sufficiently detailed search result summary remains useful when the page blocks extraction.".to_string(),
            published_at: None,
            source_name: None,
            source_url: None,
        };
        let page = page_error(
            &hit,
            "requires_browser_rendering",
            Some(200),
            "blocked".to_string(),
        );
        assert_eq!(page.status, "requires_browser_rendering");
        assert_eq!(page.evidence, vec![hit.snippet]);
        assert!(page.content_chars > 20);
    }

    #[test]
    fn google_news_rss_fixture_preserves_source_and_date() {
        let xml = r#"<?xml version="1.0"?><rss><channel><item><title>特朗普发表声明 - Example</title><link>https://news.google.com/rss/articles/abc</link><description>summary</description><pubDate>Wed, 15 Jul 2026 01:00:00 GMT</pubDate><source url="https://example.com">Example</source></item></channel></rss>"#;
        let parsed: GoogleNewsRss = parse_xml(xml).unwrap();
        assert_eq!(parsed.channel.items.len(), 1);
        assert_eq!(
            parsed.channel.items[0]
                .source
                .as_ref()
                .and_then(|value| value.name.as_deref()),
            Some("Example")
        );
    }

    #[test]
    fn live_search_smoke_when_enabled() {
        if env::var("NOLA_LIVE_WEB_SEARCH_TEST").ok().as_deref() != Some("1") {
            return;
        }
        let query = env::var("NOLA_LIVE_WEB_SEARCH_QUERY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "特朗普最新表态".to_string());
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String(query));
        args.insert("limit".to_string(), Value::Number(20.into()));
        let outcome = execute(&args);
        println!("{}", outcome.text);
        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.text.contains("Retrieved ranked pages: 20/20"));
    }
}
