// SPDX-License-Identifier: Apache-2.0
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-browser-owner-unit-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const localRequire = name => {
    if (name === 'playwright-core') return { chromium: {} };
    if (name === 'node:http') return { createServer: () => ({ once() {}, listen() {} }) };
    if (name === 'node:readline') return { createInterface: () => ({ on() {} }) };
    if (name === './owner_queue.cjs') return require('./owner_queue.cjs');
    return require(name);
  };
  const context = vm.createContext({ require: localRequire, Buffer, URL, console,
    process: { env: { NOLA_PLAYWRIGHT_STATE_ROOT: root }, stdin: {}, on() {} },
    setInterval: () => ({ unref() {} }),
  });
  vm.runInContext(fs.readFileSync(path.join(__dirname, 'bridge.cjs'), 'utf8'), context);
  const api = vm.runInContext('({createSession, activePage, pageIdentity, permissionResource, referencedElement, restoredActivePage, cleanSessionID})', context);
  return { root, api };
}

function page(url = 'about:blank') {
  const handlers = new Map(), main = {};
  return { url: () => url, on: (name, handler) => handlers.set(name, handler),
    handlers, mainFrame: () => main };
}

function browserContext(pages) {
  const handlers = new Map();
  return { pages: () => pages, on: (name, handler) => handlers.set(name, handler), handlers };
}

test('a popup joins the owner without changing selected page or its references', async t => {
  const { root, api } = fixture(t);
  const original = page(), pages = [original], context = browserContext(pages);
  const session = await api.createSession({ id: 'owner', context, persistent: false, headless: true, profileRoot: root });
  const element = { dispose() { throw new Error('Popup must not invalidate unrelated references'); } };
  session.refs.set('ref', { element });
  const popup = page('https://popup.test'); pages.push(popup);
  context.handlers.get('page')(popup);
  assert.equal(api.activePage(session), original);
  assert.equal(session.refs.get('ref').element, element);
});

test('same URL main-document navigation invalidates an old reference, but child navigation does not', async t => {
  const { root, api } = fixture(t);
  const original = page(), context = browserContext([original]);
  const session = await api.createSession({ id: 'owner', context, persistent: false, headless: true, profileRoot: root });
  const element = {};
  session.refs.set('ref', { element, revision: 1, page: original,
    generation: session.generation, documentIdentity: api.pageIdentity(original) });
  const args = { ref: 'ref', revision: 1 };
  assert.equal(api.referencedElement(session, args), element);
  original.handlers.get('framenavigated')({});
  assert.equal(api.referencedElement(session, args), element);
  original.handlers.get('framenavigated')(original.mainFrame());
  assert.throws(() => api.referencedElement(session, args), /stale/);
});

test('ambiguous persisted page URLs require an explicit selection instead of index fallback', async t => {
  const { root, api } = fixture(t);
  const url = 'https://same.test', crypto = require('node:crypto');
  const metadata = { index: 0, url_digest: crypto.createHash('sha256').update(url).digest('hex') };
  fs.writeFileSync(path.join(root, '.nola-browser-active-page.json'), JSON.stringify(metadata));
  const context = browserContext([page(url), page(url)]);
  assert.equal(api.restoredActivePage(context, root), null);
  const session = await api.createSession({ id: 'owner', context, persistent: true, headless: true, profileRoot: root });
  assert.throws(() => api.activePage(session), /explicitly select/);
  assert.equal(session.activePage, null);
});

test('invalid selection metadata is rejected unchanged, not repaired or removed', t => {
  const { root, api } = fixture(t), file = path.join(root, '.nola-browser-active-page.json');
  for (const content of ['{"index":0}', 'invalid JSON']) {
    fs.writeFileSync(file, content);
    assert.throws(() => api.restoredActivePage(browserContext([page()]), root), /metadata/);
    assert.equal(fs.readFileSync(file, 'utf8'), content);
  }
});

test('closing owner returns absent facts without a live page or connection generation', async t => {
  const { root, api } = fixture(t), context = browserContext([page()]);
  const session = await api.createSession({ id: 'owner', context, persistent: false, headless: true, profileRoot: root });
  session.closing = true;
  const facts = api.permissionResource({ session_id: 'owner' });
  assert.equal(facts.connection_state, 'absent');
  assert.equal(facts.connection_generation, null);
  assert.equal(facts.page_id, null);
  for (const value of [undefined, null, '', ' ', '../other']) {
    assert.throws(() => api.cleanSessionID(value), /invalid/);
  }
});
