// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Real-file Skill isolation across runtime and worker boundaries.

mod skill_runtime_isolation_tests {
    use super::*;
    use crate::skill_runtime::SkillRuntimeContext;
    use std::sync::{Arc, Barrier};

    const SHARED_SKILL_ID: &str = "nola-test-runtime-owned-shared";

    struct RuntimeFixture {
        _directory: tempfile::TempDir,
        context: ToolContext,
        skill_directory: PathBuf,
        marker: String,
        instructions: String,
    }

    fn fixture(label: &str) -> RuntimeFixture {
        let directory = tempfile::tempdir().expect("isolated Skill runtime");
        let trusted = directory.path().join("trusted");
        let workspace = directory.path().join("workspace");
        let skill_directory = trusted.join("shared");
        fs::create_dir_all(&skill_directory).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        let trusted = trusted.canonicalize().unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let skill_directory = skill_directory.canonicalize().unwrap();
        let marker = format!("runtime-{label}-{}", uuid::Uuid::new_v4());
        let instructions = format!(
            "---\nname: {SHARED_SKILL_ID}\ndescription: {marker}\n---\n\n# Runtime-owned instructions\n\n{marker}\n"
        );
        fs::write(skill_directory.join("SKILL.md"), &instructions).unwrap();
        fs::write(
            skill_directory.join("NOLA_MANIFEST.json"),
            json!({"version": marker, "required_tools": []}).to_string(),
        )
        .unwrap();
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: SkillRuntimeContext::from_configured_dirs(Some(vec![trusted])),
            allowed_dirs: vec![workspace],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "deny".to_string(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(directory.path().join("browser")),
        };
        RuntimeFixture {
            _directory: directory,
            context,
            skill_directory,
            marker,
            instructions,
        }
    }

    fn assert_owned_source(
        context: &ToolContext,
        expected: &RuntimeFixture,
        other: &RuntimeFixture,
    ) {
        let index = context.skills.index().expect("runtime-owned index");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0]["id"], SHARED_SKILL_ID);
        assert_eq!(index[0]["description"], expected.marker);
        assert_eq!(
            index[0]["base_directory"].as_str(),
            Some(expected.skill_directory.to_string_lossy().as_ref())
        );
        let prompt = context
            .skills
            .catalog_prompt()
            .expect("runtime-owned prompt");
        assert!(prompt.contains(&expected.marker));
        assert!(!prompt.contains(&other.marker));
        assert_eq!(
            context.skills.invocation_schema()["enum"],
            json!([SHARED_SKILL_ID])
        );
        let schemas = tool_schemas_for_host_with_skills(&context.skills);
        let invocation = schemas
            .iter()
            .find(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some("Skill")
            })
            .expect("this runtime exposes its configured Skill");
        assert_eq!(
            invocation.pointer("/function/parameters/properties/skill/enum"),
            Some(&json!([SHARED_SKILL_ID]))
        );
        let arguments = Map::from_iter([(
            "skill".to_string(),
            Value::String(SHARED_SKILL_ID.to_string()),
        )]);
        let outcome = skill_invoke(&arguments, context);
        assert!(!outcome.error, "{}", outcome.text);
        let loaded: Value = serde_json::from_str(&outcome.text).unwrap();
        assert_eq!(loaded["instructions"], expected.instructions);
        assert_eq!(loaded["version"], expected.marker);
        assert_eq!(
            loaded["base_directory"].as_str(),
            Some(expected.skill_directory.to_string_lossy().as_ref())
        );
        assert_eq!(
            loaded["workspace_root"].as_str(),
            Some(context.allowed_dirs[0].to_string_lossy().as_ref())
        );
        assert!(!outcome.text.contains(&other.marker));
    }

    #[test]
    fn equal_skill_ids_keep_distinct_catalog_prompt_and_loaded_files() {
        let runtime_a = fixture("a");
        let runtime_b = fixture("b");
        assert_owned_source(&runtime_a.context, &runtime_a, &runtime_b);
        assert_owned_source(&runtime_b.context, &runtime_b, &runtime_a);
        assert_owned_source(&runtime_a.context, &runtime_a, &runtime_b);
    }

    #[test]
    fn equal_skill_ids_remain_isolated_during_parallel_interleaving() {
        let runtime_a = fixture("parallel-a");
        let runtime_b = fixture("parallel-b");
        let barrier = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            let barrier_a = Arc::clone(&barrier);
            let runtime_a_ref = &runtime_a;
            let runtime_b_ref = &runtime_b;
            scope.spawn(move || {
                barrier_a.wait();
                for _ in 0..8 {
                    assert_owned_source(&runtime_a_ref.context, runtime_a_ref, runtime_b_ref);
                    std::thread::yield_now();
                }
            });
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..8 {
                    assert_owned_source(&runtime_b_ref.context, runtime_b_ref, runtime_a_ref);
                    std::thread::yield_now();
                }
            });
        });
    }

    #[test]
    fn worker_and_permission_boundary_clones_retain_the_original_skill_source() {
        let runtime_a = fixture("clone-a");
        let worker = runtime_a.context.clone();
        let permission_resumed = worker.clone();
        let runtime_b = fixture("clone-b");
        assert_ne!(permission_resumed.skills, runtime_b.context.skills);
        assert_eq!(permission_resumed.skills, runtime_a.context.skills);
        assert_owned_source(&runtime_b.context, &runtime_b, &runtime_a);
        std::thread::scope(|scope| {
            let runtime_a_ref = &runtime_a;
            let runtime_b_ref = &runtime_b;
            scope.spawn(move || assert_owned_source(&worker, runtime_a_ref, runtime_b_ref));
            scope.spawn(move || {
                assert_owned_source(&permission_resumed, runtime_a_ref, runtime_b_ref)
            });
        });
        assert_owned_source(&runtime_a.context, &runtime_a, &runtime_b);
    }

    #[test]
    fn configured_catalogs_do_not_enable_disabled_or_rebind_managed_selection() {
        let runtime_a = fixture("selection-a");
        let runtime_b = fixture("selection-b");
        let managed = SkillRuntimeContext::default();
        let mut disabled = runtime_a.context.clone();
        disabled.skills = SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        // A filesystem grant cannot resurrect an explicitly disabled Skill.
        disabled
            .allowed_dirs
            .push(runtime_a.skill_directory.clone());
        assert_eq!(managed.configured_dirs(), None);
        assert!(managed.enabled());
        assert_eq!(disabled.skills.configured_dirs(), Some([].as_slice()));
        assert!(disabled.skills.index().unwrap().is_empty());
        assert!(!disabled
            .skills
            .catalog_prompt()
            .unwrap()
            .contains(SHARED_SKILL_ID));
        for schemas in [
            tool_schemas_with_skills(&disabled.skills),
            tool_schemas_for_host_with_skills(&disabled.skills),
        ] {
            for name in ["Skill", "skill_skip", "skill_update"] {
                assert!(!schemas.iter().any(|schema| {
                    schema.pointer("/function/name").and_then(Value::as_str) == Some(name)
                }));
            }
        }
        let arguments = Map::from_iter([(
            "skill".to_string(),
            Value::String(SHARED_SKILL_ID.to_string()),
        )]);
        assert!(skill_invoke(&arguments, &disabled).error);
        assert_eq!(managed, SkillRuntimeContext::from_configured_dirs(None));
        assert_owned_source(&runtime_a.context, &runtime_a, &runtime_b);
    }

    #[test]
    fn unknown_exact_id_is_rejected_without_loading_another_runtime_file() {
        let runtime_a = fixture("unknown-a");
        let runtime_b = fixture("unknown-b");
        let arguments = Map::from_iter([(
            "skill".to_string(),
            Value::String(format!("missing-runtime-skill-{}", uuid::Uuid::new_v4())),
        )]);
        for context in [&runtime_a.context, &runtime_b.context] {
            let outcome = skill_invoke(&arguments, context);
            assert!(outcome.error);
            assert!(outcome.text.contains("Unknown or disabled Skill id"));
            assert!(!outcome.text.contains(&runtime_a.marker));
            assert!(!outcome.text.contains(&runtime_b.marker));
        }
    }
}
