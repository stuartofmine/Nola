// SPDX-License-Identifier: Apache-2.0
// Actual returned coverage, content-version binding and Run isolation.

fn file_evidence_context(root: &Path) -> ToolContext {
    ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(), allowed_dirs: vec![root.to_path_buf()],
        policy: ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
        permission_scope: Default::default(), cancellation: Default::default(),
        plan_state: Default::default(), output_store: ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.join("browser")),
    }
}

#[test]
fn saved_output_read_rejection_exposes_host_preservation_recovery() {
    let fixture = tempfile::tempdir().unwrap();
    let ctx = file_evidence_context(fixture.path());
    let reference = ctx.output_store.persist("saved evidence").unwrap();
    let outcome = read_file(&object_args(json!({"path": reference})), &ctx);
    assert!(outcome.error);
    assert_eq!(outcome.data.as_ref().unwrap()["error"], "saved_result_requires_output_reader");
    let activation = outcome.activation.unwrap();
    assert_eq!(activation.source, "host_resource_preservation");
    assert_eq!(activation.capability_ids, vec!["tool_output_read", "tool_output_search"]);
    assert_eq!(fs::read_to_string(reference).unwrap(), "saved evidence");
}

fn record_text_read(path: &Path, args: Value) -> ObservedFileText {
    let observed = read_file_text(path, &object_args(args), true, 16_000, FileReadPresentationBudget::direct()).unwrap();
    assert!(!observed.presentation_exhausted);
    record_returned_file_observations(&observed.text, observed.observation.clone());
    observed
}

#[test]
fn file_read_machine_content_preserves_exact_source_and_selected_range() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("data.json");
    let raw = "{\"values\":[17,23]}\r\nsecond line\n";
    fs::write(&path, raw).unwrap();
    let ctx = file_evidence_context(fixture.path());
    let full = read_file(&object_args(json!({"path":path})), &ctx);
    assert!(!full.error);
    assert_eq!(full.data.as_ref().unwrap()["content"], raw);
    assert_eq!(full.data.as_ref().unwrap()["complete"], true);
    let range = read_file(&object_args(json!({"path":path,"start_line":1,"line_count":1})), &ctx);
    assert_eq!(range.data.as_ref().unwrap()["content"], "{\"values\":[17,23]}\r\n");
    assert_eq!(range.data.as_ref().unwrap()["next_start_line"], 2);
    assert_eq!(range.data.as_ref().unwrap()["complete"], false);
    assert!(range.text.contains("1 |"));
}

#[test]
fn file_evidence_short_overestimated_line_count_is_complete() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("short.txt");
    fs::write(&path, "one\ntwo\n").unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let observed = record_text_read(&path, json!({"start_line":1,"line_count":100}));
    assert_eq!(observed.observation.unwrap().coverage, FileReadCoverage::Full);
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_ok());
}

#[test]
fn file_evidence_nonfirst_and_byte_truncated_reads_cannot_authorize_full_or_unseen_lines() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("partial.txt");
    fs::write(&path, format!("one\n{}\nthree\n", "x".repeat(1500))).unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    record_text_read(&path, json!({"start_line":3,"line_count":100}));
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((3,3))).is_ok());
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((1,1))).is_err());
    forget_file_access_state(&path);
    let observed = record_text_read(&path, json!({"max_bytes":1024}));
    assert_eq!(observed.observation.unwrap().coverage, FileReadCoverage::Lines { first:1,last:1 });
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((1,1))).is_ok());
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((2,2))).is_err(),
        "a visible prefix of a line does not prove the complete edit range");
}

#[test]
fn file_evidence_batch_discard_and_late_error_leave_no_unreturned_receipts() {
    let fixture = tempfile::tempdir().unwrap();
    let ctx = file_evidence_context(fixture.path());
    let _scope = ctx.permission_scope.enter();
    let first = fixture.path().join("first.txt");
    let omitted = fixture.path().join("omitted.txt");
    fs::write(&first, (1..=190).map(|line| format!("first_{line:03}\n")).collect::<String>()).unwrap();
    fs::write(&omitted, (1..=80).map(|line| format!("later_{line:03}\n")).collect::<String>()).unwrap();
    let outcome = read_many_files(&object_args(json!({
        "files":[{"path":first},{"path":omitted},{"path":omitted}],"max_total_bytes":4096
    })), &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("total byte limit reached"));
    assert!(outcome.text.chars().count() <= MAX_TOOL_RESULT_CHARS);
    assert!(outcome.text.len() <= 4096, "separators and omission notice count against the byte budget");
    assert!(guard_existing_file_write(&first, &Map::new(), true, None).is_ok());
    assert!(existing_file_access_snapshot(&omitted, &Map::new()).is_err());
    forget_file_access_state(&first);
    let failed = read_many_files(&object_args(json!({
        "files":[{"path":first},{"path":fixture.path().join("missing.txt")}]
    })), &ctx);
    assert!(failed.error);
    assert!(existing_file_access_snapshot(&first, &Map::new()).is_err(),
        "an earlier internal read was not returned when a later batch item failed");
}

#[test]
fn file_evidence_utf8_cap_never_covers_a_partially_returned_line() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("unicode.txt");
    fs::write(&path, format!("first\n{}\nlast\n", "界".repeat(1000))).unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let observed = record_text_read(&path, json!({"max_bytes":1024}));
    assert!(!observed.text.contains('\u{fffd}'));
    assert!(observed.text.contains("byte truncated"));
    assert_eq!(observed.observation.unwrap().coverage, FileReadCoverage::Lines { first:1,last:1 });
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((2,2))).is_err());
}

#[test]
fn file_evidence_external_new_version_discards_old_full_and_ranges() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("external.txt");
    fs::write(&path, "old first\nold second\nold third\n").unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    record_text_read(&path, json!({}));
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_ok());
    fs::write(&path, "new first\nnew second\nnew third\n").unwrap();
    record_text_read(&path, json!({"start_line":2,"line_count":1}));
    let snapshot = existing_file_access_snapshot(&path, &Map::new()).unwrap().unwrap();
    assert!(!snapshot.full_read);
    assert_eq!(snapshot.ranges, vec![(2,2)]);
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((1,1))).is_err());
    fs::write(&path, "third first\nthird second\nthird third\n").unwrap();
    record_text_read(&path, json!({"start_line":3,"line_count":1}));
    let snapshot = existing_file_access_snapshot(&path, &Map::new()).unwrap().unwrap();
    assert_eq!(snapshot.ranges, vec![(3,3)], "new versions cannot inherit old ranges either");
}

#[test]
fn file_evidence_delayed_record_keeps_read_version_not_reopened_path_version() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("raced.txt");
    fs::write(&path, "observed version\n").unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let observed = read_file_text(&path, &Map::new(), true, 16_000, FileReadPresentationBudget::direct()).unwrap();
    assert_eq!(observed.observation.as_ref().unwrap().version.sha256,
        sha256_hex(b"observed version\n"));
    fs::write(&path, "unseen replacement\n").unwrap();
    record_returned_file_observations(&observed.text, observed.observation);
    let rejected = existing_file_access_snapshot(&path, &Map::new()).unwrap_err();
    assert!(rejected.text.contains("modified since it was read"));
}

#[test]
fn file_evidence_delayed_record_cannot_cross_run_owners() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("owners.txt");
    fs::write(&path, "owner scoped\n").unwrap();
    let mut reused = SessionPermissionScope::default();
    reused.bind_file_read_run("run-A");
    let first = reused.clone();
    let observed = {
        let _scope = first.enter();
        read_file_text(&path, &Map::new(), true, 16_000, FileReadPresentationBudget::direct()).unwrap()
    };
    reused.bind_file_read_run("run-B");
    {
        let _scope = reused.enter();
        record_returned_file_observations(&observed.text, observed.observation.clone());
        assert!(existing_file_access_snapshot(&path, &Map::new()).is_err());
    }
    let _scope = first.enter();
    record_returned_file_observations(&observed.text, observed.observation);
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_ok());
}

#[test]
fn file_evidence_generic_clamp_and_mutation_do_not_claim_unseen_content() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("clamped.txt");
    fs::write(&path, "x".repeat(5000)).unwrap();
    let ctx = file_evidence_context(fixture.path());
    let _scope = ctx.permission_scope.enter();
    let outcome = execute_authorized_tool(&ToolRequest {
        id:"read-large".into(),name:"read_file".into(),args:object_args(json!({"path":path}))
    }, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.full_output_ref.is_none(), "file projection must not recurse through a saved-output preview");
    assert!(outcome.text.contains("line_not_returned: 1"));
    assert!(outcome.text.contains("next_start_line: 1"));
    assert!(existing_file_access_snapshot(&path, &Map::new()).is_err());
    fs::write(&path, "visible\n").unwrap();
    let unseen = read_file_text(&path, &Map::new(), true, 16_000, FileReadPresentationBudget::direct()).unwrap();
    // The final boundary remains defensive if another caller wraps a returned
    // section into an oversized result after projection.
    let oversized = format!("{}{}", unseen.text, "x".repeat(MAX_TOOL_RESULT_CHARS));
    record_returned_file_observations(&oversized, unseen.observation);
    assert!(existing_file_access_snapshot(&path, &Map::new()).is_err());
    record_text_read(&path, json!({}));
    refresh_file_access_state(&path);
    assert!(existing_file_access_snapshot(&path, &Map::new()).is_err(),
        "successful writes/formatting cannot manufacture a fresh full read");
}

#[test]
fn file_evidence_same_version_ranges_merge_to_full_without_gaps() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("merge.txt");
    fs::write(&path, "one\ntwo\nthree").unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    record_text_read(&path, json!({"start_line":3,"line_count":1}));
    record_text_read(&path, json!({"start_line":1,"line_count":1}));
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
    record_text_read(&path, json!({"start_line":2,"line_count":1}));
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_ok());
}

#[test]
fn file_evidence_read_generations_leave_session_approval_cache_unchanged() {
    let mut scope = SessionPermissionScope::default();
    scope.bind_file_read_run("run-A");
    let permission_inner = Arc::clone(&scope.inner);
    let first_read_scope = Arc::clone(&scope.file_read_scope);
    {
        let _scope = scope.enter();
        add_session_bash_allow_prefix("fixture-command".into());
    }
    scope.bind_file_read_run("run-A");
    assert!(Arc::ptr_eq(&scope.file_read_scope, &first_read_scope));
    scope.bind_file_read_run("run-B");
    assert!(Arc::ptr_eq(&scope.inner, &permission_inner), "read owner rotation is not a permission reset");
    assert!(!Arc::ptr_eq(&scope.file_read_scope, &first_read_scope));
    let _scope = scope.enter();
    assert_eq!(session_bash_allow_prefix_for("fixture-command --check").as_deref(), Some("fixture-command"));
}

#[test]
fn file_evidence_direct_pages_keep_complete_lines_and_exact_next_line_until_full() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("five-kilobytes.txt");
    let source = (1..=160).map(|line| format!("source_{line:03} {}\n", "x".repeat(24))).collect::<String>();
    assert!(source.len() > 5000);
    fs::write(&path, &source).unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let mut start = 1;
    let mut pages = 0;
    loop {
        let observed = read_file_text(&path, &object_args(json!({
            "start_line":start,"line_count":1000,"max_bytes":200_000
        })), true, 200_000, FileReadPresentationBudget::direct()).unwrap();
        assert!(!observed.presentation_exhausted);
        assert!(observed.text.chars().count() <= MAX_TOOL_RESULT_CHARS);
        let observation = observed.observation.clone().unwrap();
        let last = match observation.coverage {
            FileReadCoverage::Full => 160,
            FileReadCoverage::Lines { first, last } => { assert_eq!(first, start); last }
            FileReadCoverage::None => panic!("short complete lines must make progress"),
        };
        assert!(observed.text.contains(&format!("{last:>4} | source_{last:03}")));
        let bounded = clamp_outcome_with_writer(ok(observed.text.clone(), "read".into()),
            MAX_TOOL_RESULT_CHARS, |text| ToolOutputStore::for_test().persist(text));
        assert!(bounded.full_output_ref.is_none(), "direct pages must bypass generic saved-output navigation");
        record_returned_file_observations(&observed.text, observed.observation);
        pages += 1;
        match observed.next_start_line {
            Some(next) => {
                assert_eq!(next, last + 1, "continuation follows actual coverage, not requested line_count");
                assert!(observed.text.contains(&format!("next_start_line: {next}")));
                assert!(guard_existing_file_write(&path, &Map::new(), false, Some((next,next))).is_err());
                assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
                start = next;
            }
            None => break,
        }
        assert!(pages < 10);
    }
    assert!(pages > 1);
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_ok());
}

#[test]
fn file_evidence_raw_byte_limit_remains_independent_of_display_budget() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("raw-budget.txt");
    fs::write(&path, (1..=200).map(|line| format!("raw_{line:03} {}\n", "x".repeat(32))).collect::<String>()).unwrap();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let small = read_file_text(&path, &object_args(json!({"max_bytes":1024})), true, 16_000,
        FileReadPresentationBudget::direct()).unwrap();
    let large = read_file_text(&path, &object_args(json!({"max_bytes":200_000})), true, 16_000,
        FileReadPresentationBudget::direct()).unwrap();
    assert!(small.text.contains("byte truncated"));
    assert!(small.next_start_line.unwrap() < large.next_start_line.unwrap());
    record_returned_file_observations(&small.text, small.observation);
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((small.next_start_line.unwrap(),small.next_start_line.unwrap()))).is_err());
}

#[test]
fn file_evidence_batch_unicode_counts_bytes_separators_and_final_omission_notice() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("unicode-page.txt");
    let omitted = fixture.path().join("later.txt");
    fs::write(&first, (1..=200).map(|line| format!("{line}: {}\n", "界".repeat(20))).collect::<String>()).unwrap();
    fs::write(&omitted, "not returned\n").unwrap();
    let ctx = file_evidence_context(fixture.path());
    let _scope = ctx.permission_scope.enter();
    let outcome = read_many_files(&object_args(json!({
        "files":[{"path":first},{"path":omitted}],"max_total_bytes":4096
    })), &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.len() <= 4096);
    assert!(outcome.text.chars().count() <= MAX_TOOL_RESULT_CHARS);
    assert!(outcome.text.contains("omitted_from_item: 2 of 2"));
    assert_eq!(outcome.display, "Read 1 file");
    let snapshot = existing_file_access_snapshot(&first, &Map::new()).unwrap().unwrap();
    assert!(!snapshot.full_read);
    assert!(!snapshot.ranges.is_empty());
    assert!(existing_file_access_snapshot(&omitted, &Map::new()).is_err());
}

#[test]
fn file_evidence_large_file_exact_returned_range_can_be_edited() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("large.txt");
    let source = (1..=10_000).map(|line| format!("line_{line}\n")).collect::<String>();
    fs::write(&path, &source).unwrap();
    let ctx = file_evidence_context(fixture.path());
    let _scope = ctx.permission_scope.enter();
    let read = read_file(&object_args(json!({"path":path,"start_line":7500,"line_count":2})), &ctx);
    assert!(!read.error, "{}", read.text);
    assert!(read.text.contains("7500 | line_7500"));
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((7499,7500))).is_err());
    let edit = edit_file_lines(&object_args(json!({
        "path":path,"start_line":7500,"line_count":2,"new_text":"changed_7500\nchanged_7501\n"
    })), &ctx);
    assert!(!edit.error, "{}", edit.text);
    assert_eq!(fs::read_to_string(&path).unwrap(),
        source.replace("line_7500\nline_7501\n", "changed_7500\nchanged_7501\n"));
    assert!(existing_file_access_snapshot(&path, &Map::new()).is_err(),
        "editing one observed range does not mark retained unseen lines as read");
}

#[test]
fn file_evidence_derived_views_and_automatic_diagnostics_do_not_grant_raw_full_read() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("source.rs");
    fs::write(&path, "fn example() {\n    missing();\n}\n").unwrap();
    let notebook = fixture.path().join("example.ipynb");
    fs::write(&notebook, json!({"cells":[{"cell_type":"markdown","source":["seen cell"]}],
        "metadata":{"unreturned":"retained"},"nbformat":4,"nbformat_minor":5}).to_string()).unwrap();
    let ctx = file_evidence_context(fixture.path());
    let _scope = ctx.permission_scope.enter();
    let outline = file_outline(&object_args(json!({"path":path})), &ctx);
    assert!(!outline.error, "{}", outline.text);
    assert!(existing_file_access_snapshot(&path, &Map::new()).is_err());
    let notebook_result = notebook_read(&object_args(json!({"path":notebook})), &ctx);
    assert!(!notebook_result.error, "{}", notebook_result.text);
    assert!(existing_file_access_snapshot(&notebook, &Map::new()).is_err());
    assert!(automatic_failure_context(fixture.path(), "source.rs:2: error: missing", &ctx).is_some());
    assert!(existing_file_access_snapshot(&path, &Map::new()).is_err());
    let direct = failure_context(&object_args(json!({
        "root":fixture.path(),"output":"source.rs:2: error: missing","context_lines":0
    })), &ctx);
    assert!(!direct.error, "{}", direct.text);
    assert!(guard_existing_file_write(&path, &Map::new(), false, Some((2,2))).is_ok());
    assert!(guard_existing_file_write(&path, &Map::new(), true, None).is_err());
}
