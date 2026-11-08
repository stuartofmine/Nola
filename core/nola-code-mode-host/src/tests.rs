// SPDX-License-Identifier: Apache-2.0
//! Isolated engine contracts; these are not end-to-end permission evidence.

use super::*;
use std::sync::{Mutex, atomic::AtomicUsize};

#[derive(Default)]
struct Bridge {
    calls: Mutex<Vec<CodeNestedCall>>,
    outputs: Mutex<Vec<String>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    finished: Arc<AtomicUsize>,
}

impl CodeModeBridge for Bridge {
    fn invoke(&self, call: CodeNestedCall) -> ToolFuture {
        self.calls.lock().unwrap().push(call.clone());
        let active = self.active.clone();
        let peak = self.peak.clone();
        let finished = self.finished.clone();
        Box::pin(async move {
            let count = active.fetch_add(1, Ordering::AcqRel) + 1;
            peak.fetch_max(count, Ordering::AcqRel);
            tokio::time::sleep(Duration::from_millis(2)).await;
            active.fetch_sub(1, Ordering::AcqRel);
            finished.fetch_add(1, Ordering::AcqRel);
            if call.tool_name == "denied" { return Err("permission_denied".into()); }
            Ok(call.arguments)
        })
    }

    fn output(&self, _: &CodeCellIdentity, text: String) -> Result<(), String> {
        self.outputs.lock().unwrap().push(text);
        Ok(())
    }
}

fn owner() -> CodeCellIdentity {
    CodeCellIdentity { host_id: "host".into(), conversation_id: "session".into(),
        turn_id: "turn".into(), parent_call_id: "parent".into(), cell_id: "cell".into(),
        generation: "generation".into() }
}

async fn run(source: &str, limits: CodeCellLimits, bridge: Arc<Bridge>) -> Result<(), String> {
    evaluate(owner(), limits, source.into(), Vec::new(), bridge, Arc::new(AtomicBool::new(false))).await
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_local_arguments_do_not_create_sequence_holes() {
    let bridge = Arc::new(Bridge::default());
    run(r#"
        for (const name of ['read_file', 'code_mode_exec']) {
            try { await tools[name](name === 'read_file' ? [] : {}); } catch (_) {}
        }
        text(await tools.read_file({path: 'one'}));
        try { await tools.denied({}); } catch (_) {}
        text(await tools.read_file({path: 'two'}));
    "#, CodeCellLimits::default(), bridge.clone()).await.unwrap();
    let calls = bridge.calls.lock().unwrap();
    assert_eq!(calls.iter().map(|call| call.sequence).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(calls[1].tool_name, "denied");
    assert_eq!(bridge.outputs.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn parallel_calls_are_bounded_and_unawaited_calls_are_drained() {
    let bridge = Arc::new(Bridge::default());
    let mut limits = CodeCellLimits::default();
    limits.max_in_flight = 2;
    run("for (let i = 0; i < 6; i++) tools.read_file({i});", limits, bridge.clone())
        .await.unwrap();
    assert_eq!(bridge.finished.load(Ordering::Acquire), 6);
    assert_eq!(bridge.peak.load(Ordering::Acquire), 2);
    assert_eq!(bridge.calls.lock().unwrap().iter().map(|call| call.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]);
}

#[tokio::test(flavor = "current_thread")]
async fn throwing_program_still_settles_started_children() {
    let bridge = Arc::new(Bridge::default());
    assert!(run("tools.read_file({}); throw new Error('stop');",
        CodeCellLimits::default(), bridge.clone()).await.is_err());
    assert_eq!(bridge.finished.load(Ordering::Acquire), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn native_io_is_not_exposed_and_output_is_bounded() {
    let bridge = Arc::new(Bridge::default());
    run("text([typeof require, typeof process, typeof fetch, typeof __nola_invoke]);",
        CodeCellLimits::default(), bridge.clone()).await.unwrap();
    assert_eq!(bridge.outputs.lock().unwrap()[0], r#"["undefined","undefined","undefined","undefined"]"#);
    let mut limits = CodeCellLimits::default();
    limits.output_bytes = 4;
    assert!(run("text('12345');", limits, bridge).await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn tool_catalog_is_read_only_and_preserves_exact_names() {
    let bridge = Arc::new(Bridge::default());
    let catalog = vec![nola_domain::code_mode::CodeToolDescription {
        name: "plugin:example/read".into(), description: "Read example".into(),
        input_schema: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
    }];
    evaluate(owner(), CodeCellLimits::default(), r#"
        const tool = ALL_TOOLS[0];
        if (!Object.isFrozen(tool) || !Object.isFrozen(tool.input_schema.properties.path)) {
            throw new Error('catalog must be deeply frozen');
        }
        try { tool.input_schema.properties.path.type = 'number'; } catch (error) {
            if (!(error instanceof TypeError)) throw error;
        }
        try { tool.name = 'changed'; } catch (error) {
            if (!(error instanceof TypeError)) throw error;
        }
        text([tool.name, tool.input_schema.properties.path.type, typeof __nola_catalog]);
    "#.into(), catalog, bridge.clone(), Arc::new(AtomicBool::new(false))).await.unwrap();
    assert_eq!(bridge.outputs.lock().unwrap()[0], r#"["plugin:example/read","string","undefined"]"#);
}

#[tokio::test(flavor = "current_thread")]
async fn script_failures_retain_bounded_actionable_diagnostics() {
    let error = run("throw new Error('result_shape_mismatch');", CodeCellLimits::default(),
        Arc::new(Bridge::default())).await.unwrap_err();
    assert!(error.contains("result_shape_mismatch"), "{error}");
    let error = run("throw new Error('中'.repeat(10000));", CodeCellLimits::default(),
        Arc::new(Bridge::default())).await.unwrap_err();
    assert!(error.chars().count() <= 4096);
}

#[tokio::test(flavor = "current_thread")]
async fn heap_exhaustion_is_contained_and_next_cell_still_runs() {
    let bridge = Arc::new(Bridge::default());
    let mut limits = CodeCellLimits::default();
    limits.heap_bytes = 2 * 1024 * 1024;
    let result = run(
        "const retained = []; for (let i = 0; i < 100000; i++) retained.push(new Array(1024).fill(i)); text('unexpected');",
        limits, bridge.clone(),
    ).await;
    assert!(result.is_err(), "allocation must not escape the cell heap budget");
    assert!(bridge.outputs.lock().unwrap().is_empty());
    run("text('healthy');", CodeCellLimits::default(), bridge.clone()).await.unwrap();
    assert_eq!(bridge.outputs.lock().unwrap().as_slice(), ["healthy"]);
}

#[tokio::test(flavor = "current_thread")]
async fn cpu_loop_is_interrupted_and_sets_supervisor_cancel_flag() {
    let mut limits = CodeCellLimits::default();
    limits.wall_time_ms = 20;
    let cancelled = Arc::new(AtomicBool::new(false));
    assert!(evaluate(owner(), limits, "while (true) {}".into(), Vec::new(),
        Arc::new(Bridge::default()), cancelled.clone()).await.is_err());
    assert!(cancelled.load(Ordering::Acquire));
}
