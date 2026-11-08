use nola_application::conversation::{create_conversation, rename_conversation, CreateConversation};
use nola_store_port::conversation_profile::*;
use nola_domain::conversation_configuration::ConfigurationCommitResult;

#[derive(Default)]
struct Repository { writes: Vec<String> }
impl ConversationProfileRepository for Repository {
    type Error = &'static str;
    fn patch_conversation_metadata(&mut self, _: &PatchConversationMetadataInput) -> Result<Option<i64>, Self::Error> { unreachable!() }
    fn patch_conversation_configuration(&mut self, _: &PatchConversationConfigurationInput) -> Result<ConfigurationCommitResult, Self::Error> { unreachable!() }
    fn create_conversation(&mut self, input: &CreateConversationInput) -> Result<CreateConversationResult, Self::Error> {
        assert_eq!(input.revision, 1);
        self.writes.push(format!("create:{}", input.id));
        Ok(CreateConversationResult { created: true, initial_configuration_revision: Some(1) })
    }
    fn rename_conversation(&mut self, id: &str, title: &str, _: Option<nola_domain::conversation_configuration::ConversationTitleSource>, _: i64) -> Result<bool, Self::Error> {
        self.writes.push(format!("rename:{id}:{title}")); Ok(true)
    }
}

#[test]
fn identity_use_cases_use_a_restricted_non_sqlite_repository() {
    let mut repository = Repository::default();
    assert!(create_conversation(&mut repository, CreateConversation {
        id: "c".into(), title: "Original".into(), settings_json: None,
        created_at_ms: 1, updated_at_ms: 1,
    }).unwrap().created);
    assert!(rename_conversation(&mut repository, "c", "  New title  ", None, 2).unwrap());
    assert!(rename_conversation(&mut repository, "c", " ", None, 3).is_err());
    assert_eq!(repository.writes, ["create:c", "rename:c:New title"]);
}
