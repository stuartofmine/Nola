// SPDX-License-Identifier: Apache-2.0
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { OwnerOperationQueue } = require('./owner_queue.cjs');

test('same owner is FIFO while another owner can complete', async () => {
  const queue = new OwnerOperationQueue();
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const events = [];
  const first = queue.run('one', async () => { events.push('first'); await gate; });
  const second = queue.run('one', async () => { events.push('second'); });
  assert.equal(queue.isBusy('one'), true, 'queued work protects its owner');
  await queue.run('two', async () => { events.push('other'); });
  assert.deepEqual(events, ['first', 'other']);
  release();
  await Promise.all([first, second]);
  assert.deepEqual(events, ['first', 'other', 'second']);
  assert.equal(queue.isBusy('one'), false);
});

test('a failed operation releases the queue without dropping its successor', async () => {
  const queue = new OwnerOperationQueue();
  const failed = queue.run('one', async () => { throw new Error('observation failed'); });
  const next = queue.run('one', async () => 'still runs');
  await assert.rejects(failed, /observation failed/);
  assert.equal(await next, 'still runs');
  assert.equal(queue.isBusy('one'), false);
});
