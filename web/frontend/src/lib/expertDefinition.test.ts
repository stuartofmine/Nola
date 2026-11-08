import test from 'node:test';
import assert from 'node:assert/strict';
import {confirmsExpertWrite, editedExpertDefaults} from './expertDefinition.ts';
import type {PersistentAgent} from './agentRooms.ts';

const original: PersistentAgent = {id:'expert',revision:4,name:'Original',description:'Responsibilities',availability:'archived',
    defaults:{modelSelectionID:'configured:old', selectedPromptTemplateID:'template',language:'zh',modelOverrides:{a:{temperature:0.2}}}};

test('explicit inheritance clears only model/template selections and preserves unrelated defaults', () => {
    assert.deepEqual(editedExpertDefaults(original,'',''),{language:'zh',modelOverrides:{a:{temperature:0.2}}});
    assert.equal(original.defaults.modelSelectionID,'configured:old');
    assert.equal(editedExpertDefaults(original,'configured:new','next').selectedPromptTemplateID,'next');
    const inherited = {...original,defaults:{language:'zh',selectedPromptTemplateID:null}};
    assert.deepEqual(editedExpertDefaults(inherited,'',''),inherited.defaults);
});

test('uncertain edit needs the newer exact definition, not merely an existing directory ID', () => {
    const attempted = {...original,name:'Changed'};
    assert.equal(confirmsExpertWrite(original,attempted),false);
    assert.equal(confirmsExpertWrite({...original,revision:5},attempted),false);
    assert.equal(confirmsExpertWrite({...attempted,revision:5},attempted),true);
    assert.equal(confirmsExpertWrite({...attempted,revision:5,availability:'active'},attempted),false);
    assert.equal(confirmsExpertWrite({...attempted,revision:5,defaults:{...attempted.defaults,language:'en'}},attempted),false);
    assert.equal(confirmsExpertWrite({...attempted,revision:5,defaults:Object.fromEntries(Object.entries(attempted.defaults).reverse())},attempted),true);
});
