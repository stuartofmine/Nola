import test from 'node:test';
import assert from 'node:assert/strict';
import { hostDiagnostics, hostDiagnosticReport } from './hostDiagnostics.ts';
import { supportedLocales, t } from './i18n.ts';
const payload = { ok: true, service: 'nola-cli-web', mode: 'rust-cli', host: { id: 'fixture', permissionProtocolVersion: 1, token: 'private' }, logs: 'private', apiKey: 'private' };
test('diagnostics validate success and current protocol and retain only report allowlist', () => {
    const snapshot = hostDiagnostics(payload);
    assert.deepEqual(snapshot, { service: 'nola-cli-web', mode: 'rust-cli', hostId: 'fixture', permissionProtocolVersion: 1 });
    assert.ok(!hostDiagnosticReport(snapshot).includes('private'));
    for (const value of [null, {}, { ...payload, ok: false }, { ...payload, service: '' }, { ...payload, host: { id: 'fixture', permissionProtocolVersion: '1' } }, { ...payload, host: { id: '', permissionProtocolVersion: 1 } }]) assert.throws(() => hostDiagnostics(value));
});
test('diagnostic states and privacy scope exist in all supported languages', () => {
    for (const language of supportedLocales) for (const key of ['scope', 'checking', 'failed', 'stale', 'reachable', 'service', 'mode', 'protocol', 'copy', 'copyFailed']) assert.notEqual(t(language, `diagnostics.${key}`), `diagnostics.${key}`);
});
