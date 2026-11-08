// SPDX-License-Identifier: Apache-2.0
//! Reproducible CPU-only baseline; excludes persistence, FFI and renderer frames.
//! Run: cargo run -p nola_kernel --release --example a2ui_profile
use nola_domain::{generative_ui::*, generative_ui_tool::UiToolInput};
use nola_kernel::generative_ui::{projection::project_surface, schema::NOLA_CATALOG_ID, state::transition_messages};
use serde_json::json;
use std::{hint::black_box, time::Instant};

fn main() {
    println!("{}", json!({"benchmark":"a2ui_kernel_cpu", "wire_version":A2UI_WIRE_VERSION,
        "optimized":!cfg!(debug_assertions), "arch":std::env::consts::ARCH, "os":std::env::consts::OS,
        "catalog_digest":nola_kernel::generative_ui::schema::catalog_digest(),
        "excludes":["database_transaction","ffi","transport","renderer","frame_time","input_latency","peak_memory"],
        "admission_includes":"input serialization, strict JSON parsing, pinned schema validation, graph/state admission"}));
    let identity = UiSurfaceIdentity {
        owner: UiSurfaceOwner {
            authority_id: "benchmark-host".into(), conversation_id: "benchmark-chat".into(),
            participant_id: "agent".into(), context_id: "context".into(),
            originating_run_id: "run".into(), message_id: "message".into(), part_id: "part".into(),
        }, surface_id: "profile".into(), generation: 1,
    };
    for nodes in [10, 100, UI_MAX_NODES] {
        let children: Vec<_> = (1..nodes).map(|i| format!("text-{i}")).collect();
        let mut components = vec![json!({"id":"root", "component":"Column", "children":children})];
        components.extend((1..nodes).map(|i| json!({"id":format!("text-{i}"), "component":"Text", "text":{"path":"/label"}})));
        let input = UiToolInput { expected_revision: 0, messages: vec![
            json!({"version":A2UI_WIRE_VERSION, "createSurface":{"surfaceId":"profile", "catalogId":NOLA_CATALOG_ID}}),
            json!({"version":A2UI_WIRE_VERSION, "updateDataModel":{"surfaceId":"profile", "path":"/", "value":{"label":"A2UI 基准"}}}),
            json!({"version":A2UI_WIRE_VERSION, "updateComponents":{"surfaceId":"profile", "components":components}}),
        ] };
        let mut samples = Vec::new();
        let mut bytes = 0;
        // Only the first size starts with a cold process-wide schema cache.
        // Exclude one warmup per fixture size from the measured samples.
        for iteration in 0..21 {
            let start = Instant::now();
            let state = transition_messages(None, &identity, black_box(&input)).expect("valid bounded fixture");
            let admitted = start.elapsed().as_micros();
            let projected_at = Instant::now();
            let projection = project_surface(&state).expect("bounded projection");
            let projected = projected_at.elapsed().as_micros();
            assert_eq!(projection.snapshot.nodes.len(), nodes);
            let encoded_at = Instant::now();
            bytes = black_box(serde_json::to_vec(&projection.snapshot).unwrap()).len();
            let encoded = encoded_at.elapsed().as_micros();
            if iteration == 0 {
                println!("{}", json!({"nodes":nodes,"schema_cache_cold":nodes == 10,
                    "warmup_admission_us":admitted,"warmup_projection_us":projected,"warmup_json_encode_us":encoded}));
            } else {
                samples.push([admitted, projected, encoded]);
            }
        }
        let percentiles: Vec<_> = (0..3).map(|phase| {
            let mut values: Vec<_> = samples.iter().map(|v| v[phase]).collect();
            values.sort_unstable();
            json!({"median_us":(values[9] as f64 + values[10] as f64) / 2.0,
                "p95_nearest_rank_us":values[18],"max_us":values[19]})
        }).collect();
        println!("{}", json!({"nodes":nodes,"samples":20,"snapshot_bytes":bytes,
            "admission":percentiles[0],"projection":percentiles[1],"json_encode":percentiles[2]}));
    }
}
