use nola_core::agent::{AgentItem, AgentRole, CompactionPlan, ItemId};

fn plan() -> CompactionPlan {
    CompactionPlan {
        items_to_summarize: vec![AgentItem::Message {
            id: ItemId::from_string("archive".to_string()),
            role: AgentRole::Assistant,
            content: "x".repeat(800),
        }],
        kept_items: vec![AgentItem::Message {
            id: ItemId::from_string("current".to_string()),
            role: AgentRole::User,
            content: "x".repeat(400),
        }],
    }
}

#[test]
fn empty_archive_cannot_receive_a_summary_budget() {
    let mut plan = plan();
    plan.items_to_summarize.clear();
    assert!(plan.summary_token_budget(57_600, 128_000, false).is_err());
}

#[test]
fn summary_budget_subtracts_retained_context_and_framing() {
    let plan = plan();
    assert_eq!(plan.kept_items[0].estimated_tokens(), 104);
    assert_eq!(plan.summary_token_budget(200, 1000, false).unwrap(), 92);
    assert_eq!(plan.summary_token_budget(200, 150, false).unwrap(), 42);
}

#[test]
fn current_round_exceeding_soft_goal_is_preserved_but_hard_ceiling_is_enforced() {
    let plan = plan();
    assert_eq!(plan.summary_token_budget(80, 1000, false).unwrap(), 80);
    assert_eq!(plan.summary_token_budget(80, 150, false).unwrap(), 42);
    assert!(plan.summary_token_budget(200, 108, false).is_err());
    assert!(plan.summary_token_budget(200, 1, false).is_err());
    // Hard capacity is 892, but the archive savings contract further limits
    // this small source to 100. A zero soft goal disables neither constraint.
    assert_eq!(plan.summary_token_budget(0, 1000, false).unwrap(), 100);
    assert!(plan.summary_token_budget(0, 108, false).is_err());
    assert_eq!(plan.kept_items.len(), 1);
}
