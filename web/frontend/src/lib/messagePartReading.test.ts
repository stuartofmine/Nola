import test from 'node:test';
import assert from 'node:assert/strict';
import {MessageReadingScope,readingPartKey} from './messagePartReading.ts';
import type {TimelinePart} from './chatTimeline.ts';

test('regrouping retains explicit nested choices and an immutable public-field snapshot',()=>{
    const scope=new MessageReadingScope('host/conversation/message',['tool']);
    const original=scope.bind('tool');
    assert.equal(original.choose('technical',true),true);
    assert.equal(original.choose('input',false),true);
    assert.equal(original.choose('group',false),true);
    assert.equal(original.choose('tool',true),true);
    assert.equal(original.choose('process',false),true);
    const snapshot=original.open('result','Result','ORIGINAL');
    scope.update('host/conversation/message',['earlier','tool']);
    assert.strictEqual(scope.selected,snapshot);
    assert.equal(scope.selected?.text,'ORIGINAL');
    assert.equal(Object.isFrozen(snapshot),true);
    const regrouped=scope.bind('tool');
    assert.equal(regrouped.choice('technical'),true);
    assert.equal(regrouped.choice('input'),false);
    assert.equal(regrouped.choice('group'),false);
    assert.equal(regrouped.choice('tool'),true);
    assert.equal(regrouped.choice('process'),false);
    assert.equal(scope.close(snapshot!),true);
    const next=regrouped.open('result','Result','UPDATED');
    assert.equal(next?.text,'UPDATED');
    assert.equal(scope.close(snapshot!),false,'Late dismissal cannot clear a new snapshot');
    assert.strictEqual(scope.selected,next);
});

test('removed Part and source-owner ABA invalidate late open/choice callbacks',()=>{
    const scope=new MessageReadingScope('owner-A',['same-part']);
    const old=scope.bind('same-part');
    old.open('input','Input','PUBLIC-A');
    scope.update('owner-A',[]);
    assert.equal(scope.selected,null);
    scope.update('owner-A',['same-part']);
    assert.equal(old.open('input','Input','STALE'),null);
    assert.equal(old.choose('technical',true),false);
    const returnedPart=scope.bind('same-part');
    assert.equal(returnedPart.choice('tool'),null);
    assert.equal(returnedPart.choice('process'),null);
    const first=returnedPart.open('input','Input','PUBLIC-B');
    scope.update('owner-B',['same-part']);scope.update('owner-A',['same-part']);
    assert.equal(returnedPart.current(),false);
    assert.equal(returnedPart.open('input','Input','STALE-B'),null);
    const current=scope.bind('same-part');
    const next=current.open('result','Result','PUBLIC-C');
    assert.equal(scope.close(first!),false);
    assert.strictEqual(scope.selected,next);
    scope.dispose();
    assert.equal(current.open('result','Result','DETACHED'),null);
});

test('hiding a message closes its reader and fences activation while retaining explicit disclosure choices',()=>{
    const scope=new MessageReadingScope('owner',['tool']);
    const before=scope.bind('tool');
    before.choose('technical',true);before.choose('tool',true);before.choose('process',false);
    before.open('result','Result','PUBLIC');
    scope.update('owner',['tool'],false);
    assert.equal(scope.selected,null);
    assert.equal(before.open('result','Result','HIDDEN'),null);
    scope.update('owner',['tool'],true);
    assert.equal(before.current(),false);
    assert.equal(before.choose('process',true),false);
    const after=scope.bind('tool');
    assert.notEqual(after.identity,before.identity);
    assert.equal(after.choice('technical'),true);
    assert.equal(after.choice('tool'),true);
    assert.equal(after.choice('process'),false);
});

test('journal source and semantic kind are part of the reading identity, revision is not',()=>{
    const part:TimelinePart={source:'journal',id:'tool',journalOwner:'epoch-A/run/message',order:{section:'transcript',position:'1'},
        value:{kind:'tool',part_id:'tool',revision:'1'}};
    const key=readingPartKey(part);
    assert.equal(key,readingPartKey({...part,value:{...part.value,revision:'2'}}));
    assert.notEqual(key,readingPartKey({...part,journalOwner:'epoch-B/run/message'}));
    assert.notEqual(key,readingPartKey({...part,value:{...part.value,kind:'historical'}}));
});
