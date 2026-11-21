// SPDX-License-Identifier: Apache-2.0
//! ModelFit publishes its compatibility estimates under CC BY 4.0 at
//! https://modelfit.io/data/. Only the shared public dataset is requested.
//! Model names are discovery hints. The selected Hub supplies the actual files and revision.
use super::{Engine, Source};
use reqwest::{blocking::Client, redirect::Policy};
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use std::{io::Read, time::Duration};

const DATASET: &str = "https://modelfit.io/api/dataset/";
const PICKS: &str = "https://modelfit.io/stats/";
const MAX_DATASET_BYTES: u64 = 4 * 1024 * 1024;
const GIB: f64 = (1u64 << 30) as f64;

#[derive(Deserialize)]
struct Catalog {
    models: Vec<Model>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Model {
    model: String,
    params: Option<f64>,
    #[serde(default)]
    best_for: String,
    min_ram_gb: Option<f64>,
    estimated_load_gb: Option<f64>,
    kv_kb_per_token: Option<f64>,
    #[serde(default)]
    runs_locally: bool,
    #[serde(default)]
    open_weights: bool,
}

pub(super) fn queries(
    engine: Engine,
    _source: Source,
    hardware: &Value,
) -> Result<Vec<String>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Nola/1.0")
        .redirect(Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let (data, picks) = std::thread::scope(|scope| {
        let picks = scope.spawn(|| fetch(&client, PICKS));
        (
            fetch(&client, DATASET),
            picks.join().ok().and_then(Result::ok),
        )
    });
    let catalog =
        serde_json::from_slice(&data?).map_err(|e| format!("Invalid ModelFit catalog: {e}"))?;
    let picks = picks
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(published_picks)
        .unwrap_or_default();
    Ok(rank(catalog, engine, hardware, &picks))
}

fn fetch(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("ModelFit catalog unavailable: {e}"))?;
    let mut data = Vec::new();
    response
        .take(MAX_DATASET_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_DATASET_BYTES {
        return Err("ModelFit catalog exceeds size limit".into());
    }
    Ok(data)
}

// ModelFit's public RAM/VRAM table includes its actual editorial pick per tier.
// Resolve column headings explicitly so a site redesign cannot turn another cell into a model.
fn published_picks(html: &str) -> Vec<(u64, String)> {
    let document = Html::parse_document(html);
    let tables = Selector::parse("table").unwrap();
    let rows = Selector::parse("tr").unwrap();
    let headers = Selector::parse("th").unwrap();
    let cells = Selector::parse("td").unwrap();
    for table in document.select(&tables) {
        let mut rows = table.select(&rows);
        let Some(header) = rows.next() else { continue };
        let labels: Vec<_> = header
            .select(&headers)
            .map(|h| h.text().collect::<String>())
            .collect();
        let Some(tier) = labels.iter().position(|h| h.contains("RAM / VRAM")) else {
            continue;
        };
        let Some(pick) = labels
            .iter()
            .position(|h| h.eq_ignore_ascii_case("Strong pick"))
        else {
            continue;
        };
        let mut picks: Vec<_> = rows
            .filter_map(|row| {
                let values: Vec<_> = row
                    .select(&cells)
                    .map(|c| c.text().collect::<String>())
                    .collect();
                let size = values
                    .get(tier)?
                    .trim()
                    .strip_suffix("GB")?
                    .trim()
                    .parse()
                    .ok()?;
                Some((size, hub_name(values.get(pick)?)?))
            })
            .collect();
        picks.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
        return picks;
    }
    Vec::new()
}

fn rank(
    catalog: Catalog,
    engine: Engine,
    hardware: &Value,
    picks: &[(u64, String)],
) -> Vec<String> {
    let ram = hardware["memoryBytes"].as_u64().unwrap_or(0) as f64 / GIB;
    let vram = hardware["gpuMemoryBytes"].as_u64().unwrap_or(0) as f64 / GIB;
    // Use a single memory pool. Unified RAM must never be counted twice.
    let pool = if hardware["unifiedMemory"] == true {
        ram
    } else if engine == Engine::Vllm {
        vram
    } else if vram > 0.0 {
        vram.min(ram)
    } else {
        ram
    };
    if pool <= 0.0 || ram <= 0.0 {
        return Vec::new();
    }
    let budget = pool * (0.7 + 0.15 * ((pool - 32.0) / 96.0).clamp(0.0, 1.0));
    let mut models: Vec<_> = catalog
        .models
        .into_iter()
        .filter_map(|model| {
            let usage = model.best_for.to_ascii_lowercase();
            let conversational = model.model.to_ascii_lowercase().contains("instruct")
                || [
                    "chat",
                    "assistant",
                    "agent",
                    "tool calling",
                    "reasoning",
                    "coding",
                ]
                .iter()
                .any(|tag| usage.contains(tag));
            if !conversational
                || !model.open_weights
                || !model.runs_locally
                || model.min_ram_gb? > ram
            {
                return None;
            }
            let load = model.estimated_load_gb?;
            let params = model.params?;
            if load <= 0.0 || params <= 0.0 {
                return None;
            }
            // The dataset mostly rates quantized GGUF files. Reserve space for BF16 weights
            // when seeding a vLLM search; the Hub planner will check the actual artifacts.
            let load = if engine == Engine::Vllm
                && !(hardware["unifiedMemory"] == true
                    && super::VllmBackend::current().ok() == Some(super::VllmBackend::MetalMlx))
            {
                load.max(params * 2.0e9 / GIB)
            } else {
                load
            };
            if load > budget {
                return None;
            }
            let query = hub_name(&model.model)?;
            // This is only a discovery hint. The Hub planner later verifies the architecture,
            // native context and actual weight bytes of each specific build.
            let projected = model
                .kv_kb_per_token
                .filter(|n| *n > 0.0)
                .map(|kb| load + kb * 1024.0 * super::planning::TARGET_CONTEXT as f64 / GIB);
            Some((query, load, projected))
        })
        .collect();
    let mut names = Vec::new();
    for (tier, name) in picks {
        if *tier as f64 <= pool
            && models
                .iter()
                .any(|(candidate, _, est)| candidate == name && est.is_none_or(|n| n <= budget))
            && !names.contains(name)
        {
            names.push(name.clone());
            if names.len() == 2 {
                break;
            }
        }
    }
    // Reserve two search slots for models whose published KV cost fits long conversations.
    let mut long: Vec<_> = models
        .iter()
        .filter(|(_, _, est)| est.is_some_and(|n| n <= budget))
        .collect();
    long.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut added = 0;
    for (name, _, _) in long {
        if !names.contains(name) {
            names.push(name.clone());
            added += 1;
            if added == 2 {
                break;
            }
        }
    }
    // Include smaller alternatives when the best-fitting conversion is missing from
    // the selected Hub or disk space is tighter than memory.
    for fraction in [0.80, 0.50, 0.25] {
        let target = budget * fraction;
        models.sort_by(|a, b| {
            (a.1 - target)
                .abs()
                .total_cmp(&(b.1 - target).abs())
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut added = 0;
        for (name, _, _) in &models {
            if !names.contains(name) {
                names.push(name.clone());
                added += 1;
                if added == 2 {
                    break;
                }
            }
        }
    }
    names
}

fn hub_name(display: &str) -> Option<String> {
    // Quantization in parentheses describes a file within the same model family.
    let name = display.split('(').next()?.trim();
    let query = name.split_whitespace().collect::<Vec<_>>().join("-");
    (!query.is_empty()
        && query.len() <= 96
        && query
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)))
    .then_some(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn site_picks_are_ranked_by_published_memory_tier() {
        let html = "<table><tr><th>Other</th></tr></table><table><tr><th>Strong pick</th><th>Memory (RAM / VRAM)</th></tr><tr><td>Small 3B Instruct</td><td>8 GB</td></tr><tr><td>Medium 7B</td><td>16 GB</td></tr></table>";
        assert_eq!(
            published_picks(html),
            [(16, "Medium-7B".into()), (8, "Small-3B-Instruct".into())]
        );
    }

    #[test]
    fn model_ideas_respect_memory_pools_and_remain_source_independent() {
        let data = json!({"models":[
            {"model":"Big 70B","params":70,"estimatedLoadGb":45,"minRamGb":64,"openWeights":true,"runsLocally":true},
            {"model":"Small 3B Instruct (Q8)","params":3,"estimatedLoadGb":3.3,"minRamGb":8,"openWeights":true,"runsLocally":true},
            {"model":"Small 3B Instruct","params":3,"estimatedLoadGb":2.3,"minRamGb":8,"openWeights":true,"runsLocally":true},
            {"model":"Cloud model","params":1,"estimatedLoadGb":1,"minRamGb":1,"openWeights":false,"runsLocally":false}
        ]});
        let hardware =
            json!({"memoryBytes":16u64<<30,"gpuMemoryBytes":2u64<<30,"unifiedMemory":false});
        let pick = |engine, hw: &Value| {
            rank(
                serde_json::from_value(data.clone()).unwrap(),
                engine,
                hw,
                &[],
            )
        };
        assert!(pick(Engine::Vllm, &hardware).is_empty());
        let mut unified = hardware;
        unified["unifiedMemory"] = json!(true);
        assert_eq!(pick(Engine::Mlx, &unified), ["Small-3B-Instruct"]);
        assert_eq!(pick(Engine::Vllm, &unified), ["Small-3B-Instruct"]);
        assert!(hub_name("../bad/name").is_none());
    }
}
