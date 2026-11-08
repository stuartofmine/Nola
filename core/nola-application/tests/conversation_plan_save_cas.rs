use nola_application::{execute_application_command, ApplicationCommand, ConversationPlanCommand, ConversationPlanSopIdentity};
use nola_store::NolaStore;
use serde_json::json;

#[test]
fn plan_save_uses_the_callers_version_and_absence_is_create_only() {
    let directory=tempfile::tempdir().unwrap();
    let database=directory.path().join("plan-cas.db");
    let mut first=NolaStore::open(&database).unwrap();
    let mut second=NolaStore::open(&database).unwrap();
    let save=|expected_revision,content:&str| ApplicationCommand::ConversationPlan {
        command:ConversationPlanCommand::Save {
            conversation_id:"c".into(),expected_revision,
            sop:ConversationPlanSopIdentity {run_id:"r".into(),sop_id:"s".into(),sop_version:1},
            todos:json!([{ "id":"step", "content":content, "status":"pending", "priority":"medium", "dependencyIds":[], "approvalStatus":"not_required", "capabilityRefs":[], "expectedArtifactRefs":[], "successCriteria":["done"], "verificationStatus":"pending" }]),updated_at_ms:1,
        }
    };
    let created=execute_application_command(&mut first,save(None,"original")).unwrap();
    assert_eq!(created["revision"],1);
    assert!(execute_application_command(&mut second,save(None,"duplicate create")).is_err());
    let winner=execute_application_command(&mut first,save(Some(1),"winner")).unwrap();
    assert_eq!(winner["revision"],2);
    assert!(execute_application_command(&mut second,save(Some(1),"stale edit")).unwrap_err().contains("revision conflict"));
    let current=execute_application_command(&mut second,ApplicationCommand::ConversationPlan { command:ConversationPlanCommand::Get {conversation_id:"c".into()} }).unwrap();
    assert_eq!(current["todos"][0]["content"],"winner");
}
