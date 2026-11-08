import test from 'node:test';
import assert from 'node:assert/strict';
import { matchingChatMessageIds } from './chatSearch.ts';
import { journalPart } from './journalTimeline.fixture.ts';
import type { TimelineMessage } from './chatTimeline.ts';

const message = (id: string, kind: string, value: string): TimelineMessage => ({
    id, role: 'assistant', state: 'completed', canonical: null,
    parts: [journalPart({source_store_epoch:'epoch',conversation_id:'conv',run_id:'turn',assistant_message_id:id}, {part_id:`${id}:part`,kind,
        ...(kind === 'text' ? {markdown:value} : {summary:value})})],
});

test('chat search matches only loaded message text, not reasoning or tool details', () => {
    const messages = [message('answer', 'text', 'Hello 世界'), message('thought', 'reasoning', 'Hello'), message('tool', 'tool', 'Hello')];
    assert.deepEqual([...matchingChatMessageIds(messages, '  HELLO ', 'en')], ['answer']);
    assert.deepEqual([...matchingChatMessageIds(messages, '世界', 'zh-Hans')], ['answer']);
    assert.equal(matchingChatMessageIds(messages, ' ', 'en').size, 0);
});
