// SPDX-License-Identifier: Apache-2.0
//! Pure Architecture V2 state transitions.
//!
//! This crate deliberately has no persistence, transport, process, network or
//! platform dependencies. Application adapters validate wire DTOs before
//! submitting a domain event here and persist the accepted result afterwards.

mod turn_transition;
pub mod generative_ui;
pub mod strict_json;
pub use turn_transition::*;

mod context_accounting;
pub use context_accounting::*;

mod compaction_request;
pub use compaction_request::*;

mod conversation_metadata;
mod projection_diff;
mod projection_reducer;
pub use conversation_metadata::*;
mod conversation_transition;
pub use conversation_transition::*;
mod message_admission;
pub use message_admission::*;
mod part_admission;
pub use part_admission::*;
mod body_binding;
pub use body_binding::{
    validate_body_binding_declarations, validate_body_bindings, BodyBindingDeclaration,
    BodyBindingError,
};
mod semantic_identity;
pub use semantic_identity::*;

pub use projection_diff::{conversation_projection_diff, text_part_mutations};
pub use projection_reducer::*;

mod tool_batch;
pub use tool_batch::*;

mod permission_policy;
pub use permission_policy::*;

pub mod compaction_plan;

pub mod compaction_lifecycle;
pub use compaction_lifecycle::*;

pub mod context_budget;
pub use context_budget::*;

pub mod model_response_policy;

pub mod item_token_estimate;

pub mod model_completion;
pub use model_completion::*;

pub mod run_recovery;
pub use run_recovery::*;

pub mod model_attempt;
pub use model_attempt::*;

pub mod delegated_invocation;
pub use delegated_invocation::*;
pub mod delegated_permission;
pub use delegated_permission::*;
pub mod effect_delivery_attempt;
pub use effect_delivery_attempt::*;

pub mod compaction_maintenance;

pub mod compaction_summary;
