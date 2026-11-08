// SPDX-License-Identifier: Apache-2.0
//! Accepted Core Turn identity supplied to the canonical writer, never inferred
//! from Part JSON, a Run identifier, or an assistant Message suffix.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantTurnBinding {
    SegmentStarted { turn_id: String, assistant_message_id: String },
    Activity { turn_id: String, assistant_message_id: String, activity_id: String, activity_kind: String },
    Effect { turn_id: String, assistant_message_id: String, effect_id: String },
    TurnStarted {
        turn_id: String,
        assistant_message_id: String,
    },
    Admission {
        turn_id: String,
        assistant_message_id: String,
    },
    Part {
        turn_id: String,
        assistant_message_id: String,
        part_id: String,
    },
}
