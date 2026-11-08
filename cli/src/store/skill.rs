use super::NolaStore;
use crate::application_port::CliApplicationPort;
#[cfg(test)]
use crate::skill_catalog_port::validate_skills;
use serde_json::Value;

impl NolaStore {
    pub fn load_web_uploaded_skills(&self) -> Result<Vec<Value>, String> {
        crate::skill_catalog_port::load_web_uploaded_skills_with(
            &mut CliApplicationPort::persistent(&self.path)?,
        )
    }
    pub fn save_web_uploaded_skills(
        &mut self,
        expected: &[Value],
        skills: &[Value],
    ) -> Result<(), String> {
        crate::skill_catalog_port::save_web_uploaded_skills_with(
            &mut CliApplicationPort::persistent(&self.path)?,
            expected,
            skills,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::validate_skills;
    use crate::store::NolaStore;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn uploaded_skill_payload_rejects_resolved_id_collisions() {
        let skill = json!({
            "id": "same",
            "name": "Same",
            "description": "Same description",
            "category_id": "custom",
            "content": "Instructions"
        });
        let skills = vec![skill.clone(), skill];
        assert!(validate_skills(&skills).is_err());
    }

    #[test]
    fn uploaded_skill_payload_rejects_removed_field_aliases() {
        let skills = vec![json!({
            "id": "old-shape",
            "title": "Old shape",
            "summary": "Old description",
            "category": "custom",
            "instructions": "Old instructions"
        })];
        assert!(validate_skills(&skills).is_err());
    }

    #[test]
    fn web_skill_adapter_uses_only_application_port_after_cold_reopen() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nola-skill-canonical-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("nola.db");

        {
            let mut store = NolaStore::open(&database).unwrap();
            store
                .save_web_uploaded_skills(
                    &[],
                    &[json!({
                        "id": "report-author",
                        "name": "Report author",
                        "description": "Writes reports",
                        "category_id": "documents",
                        "content": "# First version\nCreate a report.",
                        "enabled": true
                    })],
                )
                .unwrap();
            let loaded = store.load_web_uploaded_skills().unwrap();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0]["content"], "# First version\nCreate a report.");
        }

        {
            let mut reopened = NolaStore::open(&database).unwrap();
            let loaded = reopened.load_web_uploaded_skills().unwrap();
            assert_eq!(loaded[0]["id"], "report-author");
            reopened
                .save_web_uploaded_skills(
                    &loaded,
                    &[json!({
                        "id": "report-author",
                        "name": "Report author",
                        "description": "Writes better reports",
                        "category_id": "documents",
                        "content": "# Second version\nCreate and verify a report.",
                        "enabled": false
                    })],
                )
                .unwrap();
        }

        let mut writer = NolaStore::open(&database).unwrap();
        let baseline = writer.load_web_uploaded_skills().unwrap();
        let mut with_new = baseline.clone();
        with_new.push(json!({"id":"other-editor","name":"Other","description":"Independent addition",
            "category_id":"custom","content":"Keep this independently created workflow.","enabled":true}));
        writer
            .save_web_uploaded_skills(&baseline, &with_new)
            .unwrap();
        // A stale browser can only remove IDs in its original baseline.
        writer
            .save_web_uploaded_skills(&baseline, &baseline)
            .unwrap();
        assert_eq!(writer.load_web_uploaded_skills().unwrap().len(), 2);
        let mut changed = baseline.clone();
        changed[0]["content"] = json!("Newer instructions.");
        writer
            .save_web_uploaded_skills(&baseline, &changed)
            .unwrap();
        assert!(writer
            .save_web_uploaded_skills(&baseline, &baseline)
            .is_err());
        assert!(writer
            .load_web_uploaded_skills()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "other-editor"));
        // Restore through the checked API for the original assertions below.
        let current = writer.load_web_uploaded_skills().unwrap();
        let original = baseline.clone();
        writer
            .save_web_uploaded_skills(&current, &original)
            .unwrap();

        let canonical = nola_store::NolaStore::open(&database).unwrap();
        let skill = canonical.skill("report-author").unwrap().unwrap();
        assert_eq!(
            skill.instructions_text,
            "# Second version\nCreate and verify a report."
        );
        assert!(!skill.enabled);
        drop(canonical);

        let mut application = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let builtins = crate::skill_catalog_port::load_web_builtin_skills_with(&mut application).unwrap();
        let mut selected = builtins[0].clone();
        selected["content"] = json!("User-owned built-in workflow.");
        selected["enabled"] = json!(false);
        crate::skill_catalog_port::save_web_uploaded_skills_with(&mut application, &builtins, &[selected.clone()]).unwrap();
        drop(application);
        let mut reopened = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let refreshed = crate::skill_catalog_port::load_web_builtin_skills_with(&mut reopened).unwrap();
        let saved = refreshed.iter().find(|item| item["id"] == selected["id"]).unwrap();
        assert_eq!(saved["content"], selected["content"]);
        assert_eq!(saved["enabled"], false);
        assert!(saved["revision"].as_i64().unwrap() > selected["revision"].as_i64().unwrap());
        assert!(crate::skill_catalog_port::save_web_uploaded_skills_with(&mut reopened, &builtins, &[selected]).is_err());
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
