const assert = require('node:assert/strict');
function explicitProfile(profiles, id) {
    const profile = profiles.find(value => value.id === id);
    assert.ok(profile?.enabled && profile.endpoint?.enabled, 'explicit model and endpoint must be enabled');
    assert.ok(profile.capabilities?.includes('chat') && profile.executionCapabilityStatus === 'tools_enabled', 'explicit model must support chat and tools');
    return { selectedModelSource: 'cloud', selectedModelId: profile.id };
}
function successfulCalls(projection, runId) {
    return projection.messages.filter(message => message.message_id === `${runId}:assistant`)
        .flatMap(message => message.parts || [])
        .filter(part => part.kind === 'tool' && part.capability_id === 'render_ui' && part.state === 'completed')
        .map(part => part.part_id);
}
function sealedOutcome(observation, cid, runId) {
    if (!observation?.terminal_seal) return null;
    const seal = observation.terminal_seal;
    assert.equal(observation.status, 'sealed');
    assert.equal(seal.identity.conversation_id, cid);
    assert.equal(seal.identity.run_id, runId);
    assert.equal(seal.identity.assistant_message_id, `${runId}:assistant`);
    assert.equal(seal.result.conversation_id, cid);
    assert.equal(seal.result.run_id, runId);
    return seal.result.value.kind;
}
function retryableObservationRace(path, method, status, body) {
    // A read-only snapshot race permits another exact read, never admission,
    // permission, recovery, cancellation or a different execution identity.
    return path === '/api/chat/agent/v2/observe' && method === 'POST' && status === 409
        && body?.code === 'observation_unavailable'
        && body?.error === 'execution changed during observation';
}
module.exports = { explicitProfile, successfulCalls, sealedOutcome, retryableObservationRace };
