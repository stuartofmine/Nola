import {test} from 'node:test';
import assert from 'node:assert/strict';
import {activityQueryKey, sameActivityOwner} from './activityRequest.ts';
import {activityPath, activitySourcePath, type ActivityDetail} from './activity.ts';

test('pagination keys normalize whitespace but separate every effective filter', () => {
    const all = activityQueryKey('', false, ' current ');
    assert.equal(all, activityQueryKey('', false, 'current'));
    assert.notEqual(all, activityQueryKey('agent', false, 'current'));
    assert.notEqual(all, activityQueryKey('', true, 'current'));
    assert.notEqual(all, activityQueryKey('', false, 'different'));
    assert.notEqual(activityQueryKey('agent', false, 'a,b'), activityQueryKey('agent,a', false, 'b'));
});

test('activity detail and source navigation require all three owner fields to agree', () => {
    const requested = {conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'assistant'};
    assert.equal(sameActivityOwner(requested, {...requested}), true);
    for (const field of Object.keys(requested) as (keyof typeof requested)[]) {
        const returned = {...requested, [field]: 'other'};
        assert.equal(sameActivityOwner(requested, returned), false, field);
    }
    const detail = {entry: {...requested, needs_response: true}, source_conversation_id: 'source', review_in_source: false} as ActivityDetail;
    const source = new URL(activitySourcePath(detail)!, 'http://nola.invalid');
    assert.equal(source.searchParams.get('request_conversation'), requested.conversation_id);
    assert.equal(source.searchParams.get('run'), requested.run_id);
    assert.equal(source.searchParams.get('from_activity'), activityPath(requested));
});
