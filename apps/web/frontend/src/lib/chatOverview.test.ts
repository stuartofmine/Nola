import test from 'node:test';
import assert from 'node:assert/strict';
import { committedContextOverview } from './chatOverview.ts';
import type { CommittedRunContextUsage } from './runContextUsage.ts';

test('overview displays only committed facts and preserves lossless counters', () => {
    const identity = { source_store_epoch: 'epoch', conversation_id: 'c', run_id: 'run', assistant_message_id: 'root' };
    const record: CommittedRunContextUsage = { protocol_version: 1, identity, attempt_sequence: '1', run_cache_usage: null,
        admission: { identity, sequence: '2', record_digest: 'a'.repeat(64) }, event_sequence: '5', record_digest: 'b'.repeat(64),
        usage: { measurement: null, prompt_cache_measurement: null, prompt_cache_diagnostic: null,
            used_tokens: '9007199254740993', context_window_tokens: '18014398509481986', prompt_token_limit: '9007199254740993', estimated: false } };
    assert.equal(committedContextOverview(null), null);
    assert.deepEqual(committedContextOverview(record), { usedTokens: '9007199254740993', contextWindowTokens: '18014398509481986', fraction: 0.5, estimated: false });
    assert.equal(committedContextOverview({ ...record, usage: { ...record.usage, estimated: true } })?.estimated, true);
    assert.equal(committedContextOverview({ ...record, usage: { ...record.usage, context_window_tokens: '0', prompt_token_limit: '0' } })?.fraction, null);
});
