// SPDX-License-Identifier: Apache-2.0
//! Transaction-local bridge between visible planning and journal persistence.
//! Not a transport producer: callers must cover every visible family before
//! enabling journal-based consumers, and roll back their command on any error.
use nola_domain::assistant_journal::{AssistantJournalCursor, AssistantJournalIdentity};
use nola_domain::assistant_journal_operations::JournalContentRef;
use nola_store_port::assistant_journal::{
    AssistantJournalFrame, AssistantJournalPartState, AssistantJournalRepository,
    AssistantManifestPage, AssistantPublicationCommit, AssistantPublicationGroup,
    AssistantSourceCommitStamp, CommitAssistantPublicationSet,
};

use crate::assistant_journal_planner::{JournalPlanningReader, PlannerError, VisibleJournalPlan};

#[cfg(test)]
#[path = "assistant_journal_publication_tests.rs"]
mod tests;

pub(crate) struct StoreJournalPlanningReader<'a, S: ?Sized>(pub &'a S);

impl<S: AssistantJournalRepository + ?Sized> JournalPlanningReader
    for StoreJournalPlanningReader<'_, S>
{
    fn load_head(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
    ) -> Result<AssistantJournalPartState, PlannerError> {
        self.0
            .assistant_journal_part_state(identity, part_id)
            .map_err(|error| PlannerError::Invalid(error.to_string()))
    }
    fn load_manifest_page(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
        content: &JournalContentRef,
        after_chunk: u64,
        max_records: usize,
    ) -> Result<AssistantManifestPage, PlannerError> {
        self.0
            .assistant_journal_content_manifest(
                identity,
                part_id,
                content,
                after_chunk,
                max_records,
            )
            .map_err(|error| PlannerError::Invalid(error.to_string()))
    }
}

/// Publish the complete changed-owner set exactly once for this source commit.
/// Preparation errors and set failures propagate to the command's outer UoW;
/// neither a successful early frame nor a display notification is a commit.
pub(crate) fn publish_assistant_publication_set<S: AssistantJournalRepository + ?Sized>(
    store: &mut S,
    stamp: AssistantSourceCommitStamp,
    plans: Vec<(AssistantJournalCursor, VisibleJournalPlan)>,
) -> Result<Option<AssistantPublicationCommit>, String> {
    // Validate owner and canonical batch shape before the first preparation.
    // Store still verifies supplied bytes and preparation proofs; any failure
    // remains an error of the enclosing command, including prior preparations.
    let mut owners = std::collections::BTreeSet::new();
    for (expected, plan) in &plans {
        let VisibleJournalPlan::Changed(frames) = plan else {
            continue;
        };
        if frames.is_empty()
            || !owners.insert((
                expected.identity.source_store_epoch.as_str(),
                expected.identity.conversation_id.as_str(),
                expected.identity.run_id.as_str(),
                expected.identity.assistant_message_id.as_str(),
            ))
        {
            return Err("empty or duplicate assistant publication group".into());
        }
        for frame in frames {
            if frame.batch.identity != expected.identity {
                return Err("assistant journal plan/cursor owner mismatch".into());
            }
            frame
                .batch
                .canonical_bytes()
                .map_err(|error| error.to_string())?;
        }
    }
    let mut groups = Vec::new();
    for (expected, plan) in plans {
        let VisibleJournalPlan::Changed(prepared) = plan else {
            continue;
        };
        let mut frames = Vec::with_capacity(prepared.len());
        for frame in prepared {
            for preparation in frame.preparations {
                let actual = store
                    .prepare_assistant_content(
                        &expected.identity,
                        &preparation.part_id,
                        &preparation.previous,
                        &preparation.chunks,
                    )
                    .map_err(|error| format!("prepare committed assistant content: {error}"))?;
                if actual != preparation.resulting {
                    return Err("prepared assistant manifest differs from planned reference".into());
                }
            }
            frames.push(AssistantJournalFrame {
                batch: frame.batch,
                supplied_chunks: frame.supplied_chunks,
            });
        }
        groups.push(AssistantPublicationGroup { expected, frames });
    }
    if groups.is_empty() {
        // Unchanged state has no fabricated empty batch or journal clock advance.
        return Ok(None);
    }
    store
        .commit_assistant_publication_set(&CommitAssistantPublicationSet { stamp, groups })
        .map(Some)
        .map_err(|error| format!("publish committed assistant set: {error}"))
}
