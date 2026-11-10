import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ApiError } from './api.ts';
import { workerSnapshot, workerDraft, workerPayload, channelSnapshot, channelsSnapshot, channelDraft, channelPayload, channelReceipt, callbackURL, messagingErrorKey, channelKinds } from './messagingSettings.ts';

const settings = { enabled: false, backend_url: 'https://service.invalid', has_access_token: true, execution_backend: 'nola', turn_timeout_seconds: 900, poll_seconds: 5, project_root: null };
function connection(kind = 'telegram') { return { connection_id: 'fixture-channel', kind, display_name: 'Fixture', account_id: 'account', credential_ref: 'SEND_REF', webhook_secret_ref: 'WEBHOOK_REF', verify_token_ref: 'VERIFY_REF', enabled: 0, state: 'disabled', provider_config: { future: { value: 'preserved' }, app_id: 'app' } }; }

test('worker snapshots expose presence only, preserve supported fields and never return stored secrets', () => {
    const snapshot = workerSnapshot({ ...settings, access_token: 'must-not-reach-form', unknown_secret: 'hidden' });
    assert.deepEqual(snapshot, settings);
    const payload = workerPayload(workerDraft(snapshot), snapshot, '');
    assert.equal(Object.hasOwn(payload, 'access_token'), false);
    assert.equal(payload.enabled, false); assert.equal(payload.poll_seconds, 5);
    assert.throws(() => workerSnapshot({ ...settings, enabled: 'false' }));
    assert.throws(() => workerSnapshot({ ...settings, turn_timeout_seconds: -1 }));
});
test('changing worker service cannot forward saved credentials and numeric drafts remain exact', () => {
    const draft = workerDraft(settings);
    assert.throws(() => workerPayload({ ...draft, backend_url: 'https://other.invalid' }, settings, ''));
    assert.equal(workerPayload({ ...draft, backend_url: 'https://other.invalid' }, settings, 'explicit-new-token').access_token, 'explicit-new-token');
    for (const backend_url of ['https://user:secret@host.invalid', 'https://host.invalid?secret=x', 'file:///tmp/config']) assert.throws(() => workerPayload({ ...draft, backend_url }, settings, 'token'));
    for (const turn_timeout_seconds of ['30.5', '0', 'Infinity', '14401']) assert.throws(() => workerPayload({ ...draft, turn_timeout_seconds }, settings, ''));
    assert.throws(() => workerPayload({ ...draft, enabled: true }, { ...settings, has_access_token: false }, ''));
    assert.equal(workerPayload({ ...draft, backend_url: 'https://other.invalid' }, settings, '', true).clear_access_token, true);
    assert.throws(() => workerPayload(draft, settings, 'new-token', true));
    assert.throws(() => workerPayload({ ...draft, enabled: true }, settings, '', true));
});
test('all channel kinds preserve provider metadata and immutable identity without mixing drafts', () => {
    for (const kind of channelKinds) {
        const baseline = channelSnapshot(connection(kind)); const draft = channelDraft(baseline);
        draft.provider_config.app_id = 'edited';
        assert.equal(baseline.provider_config.app_id, 'app');
        assert.deepEqual(channelPayload(draft, baseline).provider_config.future, { value: 'preserved' });
        assert.throws(() => channelPayload({ ...draft, connection_id: 'different' }, baseline));
        assert.throws(() => channelPayload({ ...draft, kind: 'different' }, baseline));
        assert.deepEqual(channelDraft(undefined, kind).provider_config, {});
        assert.equal(channelDraft(undefined, kind).enabled, false);
    }
    assert.throws(() => channelsSnapshot({ channels: [connection(), connection()] }));
    assert.throws(() => channelsSnapshot({ channels: [{}] }));
    assert.equal(Object.hasOwn(channelSnapshot({ ...connection(), secret: 'not-for-browser' }), 'secret'), false);
});
test('credential fields accept references, cannot silently clear saved refs, and receipts must match identity', () => {
    const baseline = channelSnapshot(connection()); const draft = channelDraft(baseline);
    assert.throws(() => channelPayload({ ...draft, credential_ref: 'bot:secret/value' }, baseline));
    assert.throws(() => channelPayload({ ...draft, verify_token_ref: '' }, baseline));
    const response = { connection_id: draft.connection_id, kind: draft.kind, display_name: 'Fixture', account_id: 'account', enabled: false, state: 'disabled' };
    assert.equal(channelReceipt(response, draft).credential_ref, 'SEND_REF');
    assert.throws(() => channelReceipt({ ...response, connection_id: 'different' }, draft));
    assert.throws(() => channelReceipt({ ...response, kind: 'we_com' }, draft));
    assert.throws(() => channelReceipt({}, draft));
});
test('callback links use saved service identity and provider failures never echo secret bodies', () => {
    const draft = channelDraft(channelSnapshot(connection()));
    assert.equal(callbackURL(settings, draft), 'https://service.invalid/v1/channels/webhook/telegram/fixture-channel');
    assert.equal(callbackURL({ ...settings, backend_url: 'https://user:secret@host.invalid' }, draft), '');
    assert.equal(callbackURL(settings, { ...draft, kind: '../evil' }), '');
    assert.equal(messagingErrorKey(new ApiError(500, 'secret-token-in-provider-body')), 'messaging.requestError');
    assert.equal(messagingErrorKey(new ApiError(401, 'secret-token')), 'messaging.authError');
});
