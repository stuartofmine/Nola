import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadModelCatalog, replaceSavedProfile } from './modelCatalog.ts';
import type { RemoteProfile } from './modelProfiles.ts';

test('canonical profiles do not read unrelated singleton settings', async () => {
    const profiles = [{ id: 'a', revision: 2 }] as RemoteProfile[];
    const calls: string[] = [];
    const result = await loadModelCatalog(async <T,>(path: string) => {
        calls.push(path); return { profiles } as T;
    });
    assert.deepEqual(calls, ['/api/model-profiles']);
    assert.deepEqual(result, { profiles });
});

test('empty catalog uses the same canonical profile route as configured catalogs', async () => {
    const calls: string[] = [];
    assert.deepEqual(await loadModelCatalog(async <T,>(path: string) => {
        calls.push(path);
        return { profiles: [] } as T;
    }), { profiles: [] });
    assert.deepEqual(calls, ['/api/model-profiles']);
});

test('save receipt replaces only the matching model including its new revision', () => {
    const profiles = [{ id: 'a', revision: 2 }, { id: 'b', revision: 8 }] as RemoteProfile[];
    const receipt = { id: 'a', revision: 3, enabled: false } as RemoteProfile;
    const result = replaceSavedProfile(profiles, receipt);
    assert.equal(result[0], receipt);
    assert.equal(result[1], profiles[1]);
    assert.equal(profiles[0].revision, 2);
});
