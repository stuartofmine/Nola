// SPDX-License-Identifier: Apache-2.0
//! Repository adapter on the owning Store connection.
use crate::{NolaStore, StoreError};
use nola_store_port::trace_journal::*;
impl TraceJournalRepository for NolaStore {
    type Error = StoreError;
    fn append_trace_journal_event(
        &mut self,
        input: &AppendTraceJournalEventInput,
    ) -> Result<bool, Self::Error> {
        NolaStore::append_trace_journal_event(self, input)
    }
    fn trace_journal_events_after(
        &self,
        trace_id: &str,
        sequence: i64,
    ) -> Result<Vec<StoredTraceJournalEvent>, Self::Error> {
        NolaStore::trace_journal_events_after(self, trace_id, sequence)
    }
    fn trace_journal_runs_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<StoredTraceJournalRun>, Self::Error> {
        NolaStore::trace_journal_runs_for_conversation(self, conversation_id)
    }
    fn exact_run_trace_replay(&self, conversation_id: &str, run_id: &str, assistant_message_id: &str,
        after_sequence: i64, through_sequence: Option<i64>, limit: usize,
    ) -> Result<StoredExactTraceReplay, Self::Error> {
        NolaStore::exact_run_trace_replay(self,conversation_id,run_id,assistant_message_id,after_sequence,through_sequence,limit)
    }
    fn latest_trace_journal_id(
        &self,
        conversation_id: &str,
    ) -> Result<Option<String>, Self::Error> {
        NolaStore::latest_trace_journal_id(self, conversation_id)
    }
}
