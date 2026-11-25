// SPDX-License-Identifier: Apache-2.0
//! Native `web_search` tool supporting DuckDuckGo HTML fallback + optional
//! Brave Search / Tavily / SearXNG backends via environment config.

use std::time::Duration;

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use scraper::{Html, Selector};
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

pub struct WebSearchTool;

impl ToolHandler for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "web_search".into(),
                description: "Search the public web and return ranked titles, URLs, and snippets.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "max_results": { "type": "integer", "description": "Default 8, max 20" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Network,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        const ALLOWED_KEYS: &[&str] = &["query", "max_results"];
        let Some(obj) = args.as_object() else {
            return ToolExecutionResult::err_structured(
                "invalid_params",
                "web_search arguments must be a JSON object",
                None,
            );
        };
        let extra: Vec<&str> = obj
            .keys()
            .map(String::as_str)
            .filter(|k| !ALLOWED_KEYS.contains(k))
            .collect();
        if !extra.is_empty() {
            return ToolExecutionResult::err_structured(
                "undeclared_parameter_rejected",
                format!(
                    "security error: undeclared parameter(s) are forbidden in web_search: {}",
                    extra.join(", ")
                ),
                Some(json!({"rejected_keys": extra})),
            );
        }
        if !ctx.network_allowed {
            return ToolExecutionResult::err_structured(
                "network_denied",
                "web_search blocked by network policy",
                None,
            );
        }
        let Some(query) = args.get("query").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing non-empty 'query'", None);
        };
        let limit = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(8)
            .clamp(1, 20) as usize;

        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) Nola/2.0")
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::err_structured("http_error", e.to_string(), None),
        };

        // 1. Optional SearXNG endpoint if configured
        if let Ok(searx) = std::env::var("NOLA_SEARXNG_URL") {
            if !searx.trim().is_empty() {
                if let Ok(results) = search_searxng(&client, searx.trim(), query, limit) {
                    let count = results.len();
                    return ToolExecutionResult::ok_structured(
                        json!({"query": query, "provider": "searxng", "count": count, "results": results}),
                        format!("Web search returned {count} result(s) via SearXNG"),
                    );
                }
            }
        }

        // 2. Optional Brave Search API if key is set
        if let Ok(brave_key) = std::env::var("BRAVE_API_KEY") {
            if !brave_key.trim().is_empty() {
                if let Ok(results) = search_brave(&client, brave_key.trim(), query, limit) {
                    let count = results.len();
                    return ToolExecutionResult::ok_structured(
                        json!({"query": query, "provider": "brave", "count": count, "results": results}),
                        format!("Web search returned {count} result(s) via Brave"),
                    );
                }
            }
        }

        // 3. Default keyless DuckDuckGo HTML search
        match search_duckduckgo_html(&client, query, limit) {
            Ok(results) => {
                let count = results.len();
                ToolExecutionResult::ok_structured(
                    json!({"query": query, "provider": "duckduckgo", "count": count, "results": results}),
                    format!("Web search returned {count} result(s) for '{query}'"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("web_search_error", e, None),
        }
    }
}

fn search_searxng(
    client: &reqwest::blocking::Client,
    base: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/search", base.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .query(&[("q", query), ("format", "json")])
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: Value = resp.json().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(arr) = body.get("results").and_then(|v| v.as_array()) {
        for item in arr.iter().take(limit) {
            out.push(json!({
                "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "url": item.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "snippet": item.get("content").and_then(|v| v.as_str()).unwrap_or("")
            }));
        }
    }
    Ok(out)
}

fn search_brave(
    client: &reqwest::blocking::Client,
    api_key: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .query(&[("q", query), ("count", &limit.to_string())])
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: Value = resp.json().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(arr) = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|v| v.as_array())
    {
        for item in arr.iter().take(limit) {
            out.push(json!({
                "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "url": item.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "snippet": item.get("description").and_then(|v| v.as_str()).unwrap_or("")
            }));
        }
    }
    Ok(out)
}

pub(crate) fn parse_duckduckgo_html(html: &str, limit: usize) -> Vec<Value> {
    let doc = Html::parse_document(html);
    let result_sel = Selector::parse(".result").unwrap();
    let link_sel = Selector::parse("a.result__a").unwrap();
    let snippet_sel = Selector::parse(".result__snippet").unwrap();
    let mut out = Vec::new();
    for el in doc.select(&result_sel).take(limit) {
        let Some(a) = el.select(&link_sel).next() else {
            continue;
        };
        let title = a.text().collect::<Vec<_>>().join(" ").trim().to_string();
        let raw_href = a.value().attr("href").unwrap_or("");
        let url = decode_ddg_href(raw_href);
        let snippet = el
            .select(&snippet_sel)
            .next()
            .map(|s| s.text().collect::<Vec<_>>().join(" ").trim().to_string())
            .unwrap_or_default();
        if !title.is_empty() && !url.is_empty() {
            out.push(json!({"title": title, "url": url, "snippet": snippet}));
        }
    }
    out
}

fn decode_ddg_href(href: &str) -> String {
    let normalized = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    if let Ok(parsed) = url::Url::parse(&normalized) {
        if parsed.host_str().is_some_and(|h| h.contains("duckduckgo.com")) {
            for (k, v) in parsed.query_pairs() {
                if k == "uddg" && !v.is_empty() {
                    return v.into_owned();
                }
            }
        }
    }
    normalized
}

fn search_duckduckgo_html(
    client: &reqwest::blocking::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let resp = client
        .post("https://html.duckduckgo.com/html/")
        .form(&[("q", query)])
        .send()
        .map_err(|e| format!("duckduckgo request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("duckduckgo returned HTTP {}", resp.status()));
    }
    let html = resp.text().map_err(|e| e.to_string())?;
    Ok(parse_duckduckgo_html(&html, limit))
}
