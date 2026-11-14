// SPDX-License-Identifier: Apache-2.0
'use strict';

const assert = require('node:assert/strict');
const childProcess = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');

async function main() {
  const stateRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-playwright-e2e-'));
  const bridgeEnvironment = { ...process.env, NOLA_PLAYWRIGHT_STATE_ROOT: stateRoot };
  const server = http.createServer((_request, response) => {
    response.setHeader('content-type', 'text/html; charset=utf-8');
    const controls = Array.from({length: 300}, (_, index) => `<button aria-label="Control ${index}">${index}</button>`).join('');
    response.end(`<!doctype html><html><body>
      <label>Name <input aria-label="Name"></label>
      <button onclick="document.querySelector('#result').textContent='Saved '+document.querySelector('input').value">Save</button>
      <button onclick="window.open('/popup', '_blank')">Open popup</button>
      <p id="result">Idle</p>
      ${controls}
    </body></html>`);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  const bridge = childProcess.spawn(process.execPath, [path.join(__dirname, 'bridge.cjs')], {
    stdio: ['pipe', 'pipe', 'inherit'],
    env: bridgeEnvironment,
  });
  const output = readline.createInterface({ input: bridge.stdout, crlfDelay: Infinity });
  const pending = new Map();
  output.on('line', line => {
    const response = JSON.parse(line);
    const waiter = pending.get(response.id);
    pending.delete(response.id);
    if (response.error) waiter.reject(new Error(response.error));
    else waiter.resolve(response.result);
  });
  let nextID = 1;
  const call = (action, args) => new Promise((resolve, reject) => {
    const id = nextID++;
    pending.set(id, { resolve, reject });
    bridge.stdin.write(`${JSON.stringify({ id, action, args })}\n`);
  });
  try {
    const sessionID = 'managed-browser-e2e';
    const preflightAbsent = await call('permission_resource', { session_id: sessionID });
    assert.equal(preflightAbsent.connection_state, 'absent');
    assert.equal(preflightAbsent.authentication_state, 'unknown');
    const absent = await call('status', { session_id: sessionID });
    assert.deepEqual(absent.sessions, [], 'reading a browser session must not start it');
    const started = await call('start', { session_id: sessionID, headless: true, persistent: true });
    assert.equal(started.running, true);
    assert.equal(started.engine, 'browser-use');
    assert.equal(started.engine_version, '1.0.2');
    assert.equal(started.sandboxed, true);
    assert.equal(started.headless, true);
    assert.ok(started.preview_url?.startsWith('http://127.0.0.1:'));
    const previewResponse = await fetch(started.preview_url);
    assert.equal(previewResponse.status, 200);
    assert.match(await previewResponse.text(), /Browser page/);
    const frameURL = new URL(started.preview_url);
    frameURL.pathname = '/frame';
    const firstFrame = await fetch(frameURL);
    assert.equal(firstFrame.status, 200);
    const frameETag = firstFrame.headers.get('etag');
    assert.ok(frameETag);
    const cachedFrame = await fetch(frameURL, { headers: { 'if-none-match': frameETag } });
    assert.equal(cachedFrame.status, 304);
    const destination = `http://127.0.0.1:${port}`;
    const resource = await call('permission_resource', { session_id: sessionID, url: destination });
    const rejected = await call('navigate', { session_id: sessionID, url: destination,
      _nola_permission_resource: { ...resource, page_id: 'another-page' } });
    assert.equal(rejected.action.status, 'not_started');
    assert.equal(rejected.error.code, 'permission_resource_changed');
    const page = await call('navigate', { session_id: sessionID, url: destination, _nola_permission_resource: resource });
    assert.equal(page.action.status, 'completed');
    assert.equal(page.observation.status, 'completed');
    assert.equal(page.generation, started.generation);
    const wrongGeneration = new URL(started.preview_url);
    wrongGeneration.searchParams.set('generation', 'obsolete-fixture-generation');
    assert.equal((await fetch(wrongGeneration)).status, 409);
    let input = page.elements.find(element => element.name === 'Name');
    let save = page.elements.find(element => element.name === 'Save');
    const staleSave = save;
    const staleRevision = page.revision;
    assert.ok(input?.ref);
    assert.ok(save?.ref);
    assert.equal(page.elements.length, 160);
    assert.ok(page.accessibility.length <= 20000);
    const snapshotStarted = performance.now();
    let currentPage = page;
    for (let index = 0; index < 5; index += 1) {
      currentPage = await call('snapshot', { session_id: sessionID, max_elements: 160 });
    }
    assert.ok(performance.now() - snapshotStarted < 5000, 'five dense DOM snapshots should stay below five seconds');
    input = currentPage.elements.find(element => element.name === 'Name');
    save = currentPage.elements.find(element => element.name === 'Save');
    assert.ok(input?.ref);
    assert.ok(save?.ref);
    let typed = await call('type_text', { session_id: sessionID, ref: input.ref, revision: currentPage.revision, text: 'Nola' });
    input = typed.elements.find(element => element.name === 'Name');
    typed = await call('type_text', { session_id: sessionID, ref: input.ref, revision: typed.revision, text: ' 你好', clear: false });
    const currentSave = typed.elements.find(element => element.name === 'Save');
    assert.ok(currentSave?.ref);
    const clicked = await call('click', { session_id: sessionID, ref: currentSave.ref, revision: typed.revision, screenshot: true });
    assert.match(clicked.text, /Saved Nola 你好/);
    assert.match(clicked.screenshot_data_url, /^data:image\/png;base64,/);
    await assert.rejects(
      call('click', { session_id: sessionID, ref: staleSave.ref, revision: staleRevision }),
      /stale|unknown/
    );
    const popup = clicked.elements.find(element => element.name === 'Open popup');
    assert.ok(popup?.ref);
    const afterPopup = await call('click', { session_id: sessionID, ref: popup.ref, revision: clicked.revision });
    assert.equal(afterPopup.url, clicked.url, 'a popup must not silently select a different page');
    const popupTabs = await call('tabs', { session_id: sessionID });
    assert.equal(popupTabs.pages.filter(item => item.active).length, 1);
    assert.equal(popupTabs.pages.find(item => item.active).url, clicked.url);
    await call('navigate', { session_id: sessionID, url: `http://127.0.0.1:${port}/second`, new_page: true });
    const tabs = await call('tabs', { session_id: sessionID });
    assert.equal(tabs.pages.length, 3);
    const originalTab = tabs.pages.find(page => page.url === clicked.url);
    assert.ok(originalTab);
    assert.equal(tabs.pages.filter(page => page.active).length, 1);
    const switched = await call('switch_page', { session_id: sessionID, index: originalTab.index });
    assert.equal(switched.pages.find(page => page.url === clicked.url)?.active, true);
    assert.equal(switched.url, clicked.url);
    const secondBridge = childProcess.spawn(process.execPath, [path.join(__dirname, 'bridge.cjs')], {
      stdio: ['pipe', 'pipe', 'inherit'],
      env: bridgeEnvironment,
    });
    const secondOutput = readline.createInterface({ input: secondBridge.stdout, crlfDelay: Infinity });
    const secondResponses = new Map();
    secondOutput.on('line', line => {
      const response = JSON.parse(line);
      const waiter = secondResponses.get(response.id);
      secondResponses.delete(response.id);
      if (response.error) waiter.reject(new Error(response.error));
      else waiter.resolve(response.result);
    });
    const secondCall = (id, action, args) => new Promise((resolve, reject) => {
      secondResponses.set(id, { resolve, reject });
      secondBridge.stdin.write(`${JSON.stringify({ id, action, args })}\n`);
    });
    const status = await secondCall(1, 'status', {
      session_id: sessionID,
      persistent: true,
    });
    assert.deepEqual(status.sessions, [], 'status must not attach a browser as a side effect');
    const reconnected = await secondCall(2, 'start', { session_id: sessionID, persistent: true });
    assert.equal(reconnected.session_id, sessionID);
    assert.notEqual(reconnected.generation, started.generation);
    const resumed = await secondCall(3, 'snapshot', {
      session_id: sessionID,
      persistent: true,
    });
    assert.equal(resumed.url, clicked.url);
    assert.match(resumed.text, /Saved/);
    secondBridge.kill('SIGTERM');
    await call('stop', { session_id: sessionID });
  } finally {
    bridge.kill('SIGTERM');
    server.close();
    if (bridge.exitCode === null && bridge.signalCode === null) await new Promise(resolve => bridge.once('exit', resolve));
    fs.rmSync(stateRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
