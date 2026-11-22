// SPDX-License-Identifier: Apache-2.0
// A download belongs to the Host, so reloading or closing its model page leaves it running.
use super::{
    json_response, read_json, source_management_request_allowed, web_csrf_is_valid,
    web_session_is_authenticated,
};
use crate::{local_models, model_acquisition};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::Read, sync::Mutex};
use tiny_http::{Request, Response, StatusCode};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    engine: String,
    source: String,
    repository: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Downloading,
    Cancelling,
    Installed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Download {
    id: String,
    #[serde(flatten)]
    selection: Selection,
    status: Phase,
    downloaded_bytes: u64,
    total_bytes: u64,
    error: Option<String>,
}

impl Download {
    fn active(&self) -> bool {
        matches!(self.status, Phase::Downloading | Phase::Cancelling)
    }
}

static DOWNLOAD: Mutex<Option<Download>> = Mutex::new(None);

fn claim(
    slot: &mut Option<Download>,
    selection: Selection,
) -> Result<(Download, bool), &'static str> {
    if let Some(current) = slot.as_ref().filter(|job| job.active()) {
        if current.selection == selection {
            return Ok((current.clone(), false));
        }
        return Err("Another model is downloading.");
    }
    let job = Download {
        id: uuid::Uuid::new_v4().to_string(),
        selection,
        status: Phase::Downloading,
        downloaded_bytes: 0,
        total_bytes: 0,
        error: None,
    };
    *slot = Some(job.clone());
    Ok((job, true))
}

pub(super) fn catalog() -> Value {
    let job = DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut value = local_models::catalog_json();
    value["acquisition"] = json!(job);
    value["defaultSource"] = json!(model_acquisition::default_source_name());
    value
}

fn response(code: u16, data: Value) -> Response<Box<dyn Read + Send>> {
    json_response(StatusCode(code), data)
}

fn finish(id: &str, result: Result<Value, String>) {
    let mut slot = DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(job) = slot.as_mut().filter(|current| current.id == id) {
        match result {
            Ok(_) => job.status = Phase::Installed,
            Err(_) if job.status == Phase::Cancelling => job.status = Phase::Cancelled,
            Err(error) => {
                job.status = Phase::Failed;
                job.error = Some(error);
            }
        }
    }
}

fn start(selection: Selection) -> Response<Box<dyn Read + Send>> {
    let (job, new) = match claim(
        &mut DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner()),
        selection.clone(),
    ) {
        Ok(start) => start,
        Err(error) => return response(409, json!({"error":error})),
    };
    if new {
        let id = job.id.clone();
        let worker = std::thread::Builder::new()
            .name("local-model-install".into())
            .spawn(move || {
                let progress = |downloaded, total| {
                    let mut slot = DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(job) = slot
                        .as_mut()
                        .filter(|current| current.id == id && current.status == Phase::Downloading)
                    else {
                        return false;
                    };
                    job.downloaded_bytes = downloaded;
                    job.total_bytes = total;
                    true
                };
                let result = match &selection.plan {
                    Some(plan) => model_acquisition::install_plan(
                        &selection.engine,
                        &selection.source,
                        plan,
                        progress,
                    ),
                    None => model_acquisition::install(
                        &selection.engine,
                        &selection.source,
                        &selection.repository,
                        progress,
                    ),
                };
                finish(&id, result);
            });
        if let Err(error) = worker {
            let message = format!("Could not start the model download: {error}");
            finish(&job.id, Err(message.clone()));
            return response(503, json!({"error":message}));
        }
    }
    response(202, json!({"job":job}))
}

pub(super) fn handle(
    request: &mut Request,
    action: &str,
    tailcat: bool,
) -> Response<Box<dyn Read + Send>> {
    if !(source_management_request_allowed(request, tailcat)
        || (web_session_is_authenticated(request) && web_csrf_is_valid(request)))
    {
        return response(
            403,
            json!({"error":"Open model management on the Host or sign in to it."}),
        );
    }
    if action == "cancel" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Cancel {
            id: String,
        }
        let input = match read_json::<Cancel>(request) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let mut slot = DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner());
        let Some(job) = slot.as_mut().filter(|job| job.id == input.id) else {
            return response(409, json!({"error":"This download is no longer current."}));
        };
        if job.status == Phase::Downloading {
            job.status = Phase::Cancelling;
        }
        return response(200, json!({"job":job}));
    }
    if matches!(action, "search" | "recommend") {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Search {
            engine: String,
            #[serde(default)]
            source: String,
            #[serde(default)]
            query: String,
        }
        let input = match read_json::<Search>(request) {
            Ok(input) => input,
            Err(error) => return error,
        };
        let resolved_source = if input.source.trim().is_empty()
            || input.source.trim().eq_ignore_ascii_case("auto")
            || input.source.trim().eq_ignore_ascii_case("default")
        {
            model_acquisition::default_source_name()
        } else {
            input.source.as_str()
        };
        if !valid_engine_and_source(&input.engine, resolved_source)
            || (action == "search" && input.query.trim().is_empty())
            || input.query.len() > 512
        {
            return response(400, json!({"error":"Invalid model search."}));
        }
        let lookup = if action == "recommend" {
            model_acquisition::recommend
        } else {
            model_acquisition::search
        };
        return match lookup(&input.engine, resolved_source, &input.query) {
            Ok(result) => response(200, result),
            Err(error) => response(502, json!({"error":error})),
        };
    }
    let selection = match read_json::<Selection>(request) {
        Ok(input) => input,
        Err(error) => return error,
    };
    if !valid_engine_and_source(&selection.engine, &selection.source)
        || selection.repository.is_empty()
        || selection.repository.len() > 512
    {
        return response(400, json!({"error":"Invalid model selection."}));
    }
    if action == "install-plan"
        && !matches!(selection.plan.as_ref(), Some(plan) if plan["repository"].as_str() == Some(selection.repository.as_str()))
    {
        return response(
            400,
            json!({"error":"A confirmed plan for this repository is required."}),
        );
    }
    start(selection)
}

fn valid_engine_and_source(engine: &str, source: &str) -> bool {
    matches!(engine, "llama.cpp" | "vllm" | "mlx") && matches!(source, "huggingface" | "modelscope")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_install_reattaches_and_retry_gets_a_new_id() {
        let first = Selection {
            engine: "llama.cpp".into(),
            source: "huggingface".into(),
            repository: "test/model".into(),
            plan: Some(json!({"repository":"test/model","revision":"abc"})),
        };
        let mut slot = None;
        let (original, started) = claim(&mut slot, first.clone()).unwrap();
        assert!(started);
        let (duplicate, started) = claim(&mut slot, first.clone()).unwrap();
        assert!(!started);
        assert_eq!(original.id, duplicate.id);
        let mut revised = first.clone();
        revised.plan.as_mut().unwrap()["revision"] = json!("def");
        assert!(claim(&mut slot, revised).is_err());
        let mut other = first.clone();
        other.source = "modelscope".into();
        assert!(claim(&mut slot, other).is_err());
        slot.as_mut().unwrap().status = Phase::Failed;
        let (retried, started) = claim(&mut slot, first).unwrap();
        assert!(started);
        assert_ne!(original.id, retried.id);
    }
}
