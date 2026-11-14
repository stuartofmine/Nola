import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {parseExpertRoles, expertLocalized, expertEfforts, expertDefaultProfileId} from './expertRoles.ts';
import type {RemoteProfile} from './modelProfiles.ts';
const catalog = JSON.parse(readFileSync(new URL('../../../../../core/nola-application/src/expert_roles/catalog.json',import.meta.url),'utf8'));

test('the real Core catalog exposes all seven stable IDs with prompt, declared tools and readable Skills', () => {
    const roles = parseExpertRoles(catalog);
    assert.equal(roles.length,7);
    assert.equal(new Set(roles.map(role=>role.role_id)).size,7);
    for (const role of roles) {
        assert.match(role.id,/^9f7d0000-4e6f-4c61-8000-00000000000[1-7]$/);
        assert.ok(role.prompt.trim()); assert.ok(role.tool_pack.tools.length); assert.ok(role.skills.length);
        assert.ok(expertLocalized(role.name,'zh-Hans')); assert.ok(expertLocalized(role.description,'ja'));
        for (const skill of role.skills) assert.ok(expertLocalized(skill.name,'de'));
    }
    assert.throws(()=>parseExpertRoles({...catalog,version:2}));
    assert.throws(()=>parseExpertRoles({...catalog,templates:[catalog.templates[0],catalog.templates[0]]}));
    assert.throws(()=>parseExpertRoles({...catalog,templates:[{...catalog.templates[0],prompt:''}]}));
});

test('expert effort options come only from the readable selected model declaration, including five or seven actual tiers', () => {
    const profile={id:'model',selected:true,enabled:true,endpoint:{enabled:true},executionCapabilities:{reasoning:true},
        reasoning:{control:'effort',efforts:['minimal','low','medium','high','xhigh']}} as RemoteProfile;
    assert.deepEqual(expertEfforts([profile],''),[]);
    assert.deepEqual(expertEfforts([profile],'','model'),['minimal','low','medium','high','xhigh']);
    assert.equal(expertDefaultProfileId({kind:'resolved',profile_id:'model'}),'model');
    assert.equal(expertDefaultProfileId({kind:'unavailable',profile_id:'model'}),null);
    assert.deepEqual(expertEfforts([profile],'configured:model'),['minimal','low','medium','high','xhigh']);
    assert.deepEqual(expertEfforts([profile],'configured:missing'),[]);
    assert.deepEqual(expertEfforts([{...profile,executionCapabilities:undefined}],''),[]);
    assert.deepEqual(expertEfforts([{...profile,reasoning:{control:'budget',budgets:[1024,2048,4096]}}],''),[]);
    assert.deepEqual(expertEfforts([{...profile,reasoning:{control:'effort',efforts:['invented','none','max']}}],'configured:model'),['none','max']);
});
