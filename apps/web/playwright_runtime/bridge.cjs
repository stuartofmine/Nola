// SPDX-License-Identifier: Apache-2.0
// Long-lived JSON-lines Playwright bridge owned by nola-agent-host.

'use strict';

const fs = require('node:fs');
const crypto = require('node:crypto');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const http = require('node:http');
const { chromium } = require('playwright-core');
const { OwnerOperationQueue } = require('./owner_queue.cjs');

const sessions = new Map();
const ownerOperations = new OwnerOperationQueue();
const startingSessions = new Set();
const pageIdentities = new WeakMap();
const pageDocumentGenerations = new WeakMap();

function pageIdentity(page) {
  if (!pageIdentities.has(page)) pageIdentities.set(page, crypto.randomUUID());
  return `${pageIdentities.get(page)}:${pageDocumentGenerations.get(page) || 0}`;
}

function pageOrigin(value) {
  try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) ? url.origin : null; }
  catch { return null; }
}

function permissionResource(args) {
  const id = cleanSessionID(args.session_id);
  const session = sessions.get(id);
  const pages = session && !session.closing ? session.context.pages() : [];
  if (args.index != null && (!Number.isInteger(args.index) || args.index < 0 || args.index >= pages.length)) {
    throw new Error('permission resource page index is out of range');
  }
  const candidate = args.index != null ? pages[args.index] : session?.activePage;
  const page = pages.includes(candidate) ? candidate : null;
  return { ok: true, target: 'managed', owner_id: id,
    connection_state: session && !session.closing ? 'existing' : 'absent',
    connection_generation: session && !session.closing ? session.generation : null,
    page_id: page ? pageIdentity(page) : null,
    page_origin: page ? pageOrigin(page.url()) : null, requested_origin: pageOrigin(args.url),
    authentication_state: 'unknown' };
}
const maximumLiveSessions = Math.max(1, Math.min(Number(process.env.NOLA_PLAYWRIGHT_MAX_LIVE_SESSIONS || 3), 8));
const idleSessionMilliseconds = Math.max(60_000, Number(process.env.NOLA_PLAYWRIGHT_IDLE_SESSION_MS || 15 * 60_000));
const diagnosticEntryLimit = Math.max(20, Math.min(Number(process.env.NOLA_PLAYWRIGHT_DIAGNOSTIC_LIMIT || 200), 1_000));
const previewCacheMilliseconds = Math.max(250, Number(process.env.NOLA_PLAYWRIGHT_PREVIEW_CACHE_MS || 1_200));
const previewToken = process.env.NOLA_PLAYWRIGHT_CONTROL_TOKEN || crypto.randomUUID();
let previewPort = null;
const stateRoot = process.env.NOLA_PLAYWRIGHT_STATE_ROOT
  || path.join(os.homedir(), 'Library', 'Application Support', 'Nola', 'Browser');
const controlFile = process.env.NOLA_PLAYWRIGHT_CONTROL_FILE || null;
const parentPID = Number(process.env.NOLA_PLAYWRIGHT_PARENT_PID || 0);

function parentIsAlive() {
  if (!Number.isInteger(parentPID) || parentPID <= 1) return true;
  try {
    process.kill(parentPID, 0);
    return true;
  } catch {
    return false;
  }
}

function cleanSessionID(value) {
  const id = typeof value === 'string' ? value.trim() : '';
  if (!/^[A-Za-z0-9._-]{1,128}$/.test(id)) throw new Error('invalid browser session_id');
  return id;
}

function sessionPreviewURL(session) {
  return previewPort == null
    ? null
    : `http://127.0.0.1:${previewPort}/view?session_id=${encodeURIComponent(session.id)}&generation=${encodeURIComponent(session.generation)}&token=${encodeURIComponent(previewToken)}`;
}

function sessionSummary(session) {
  return {
    session_id: session.id,
    generation: session.generation,
    target: 'managed',
    running: true,
    persistent: session.persistent,
    headless: session.headless,
    page_count: session.context.pages().length,
    selection_required: !session.context.pages().includes(session.activePage),
    revision: session.revision,
    sandboxed: true,
    preview_url: sessionPreviewURL(session),
  };
}

function touchSession(session) {
  session.lastUsedAt = Date.now();
}

function boundedPush(items, value) {
  items.push(value);
  if (items.length > diagnosticEntryLimit) items.splice(0, items.length - diagnosticEntryLimit);
}

function invalidatePreview(session) {
  session.previewFrame = null;
}

function clearReferences(session) {
  const references = [...session.refs.values()];
  session.refs.clear();
  for (const entry of references) entry.element.dispose().catch(() => {});
}

async function previewFrame(session) {
  touchSession(session);
  const now = Date.now();
  if (session.previewFrame && now - session.previewFrame.capturedAt < previewCacheMilliseconds) {
    return session.previewFrame;
  }
  if (!session.previewCapture) {
    const revision = session.revision;
    session.previewCapture = activePage(session).screenshot({type:'jpeg',quality:68})
      .then(bytes => ({ bytes, revision, capturedAt: Date.now() }))
      .then(frame => {
        if (session.revision === revision) session.previewFrame = frame;
        return frame;
      })
      .finally(() => { session.previewCapture = null; });
  }
  return session.previewCapture;
}

async function evictIdleSessions(exceptID = null, reservesSlot = false) {
  const now = Date.now();
  const ordered = [...sessions.values()].sort((left, right) => left.lastUsedAt - right.lastUsedAt);
  for (const session of ordered) {
    if (session.id === exceptID || session.closing || ownerOperations.isBusy(session.id)) continue;
    const overLimit = reservesSlot
      ? sessions.size + startingSessions.size > maximumLiveSessions
      : sessions.size > maximumLiveSessions;
    const idle = now - session.lastUsedAt >= idleSessionMilliseconds;
    if (!overLimit && !idle) continue;
    session.closing = true;
    try { await session.context.close(); }
    catch (error) { session.closing = false; throw error; }
    sessions.delete(session.id);
    if (!idle && sessions.size < maximumLiveSessions) break;
  }
}

function requestBody(request, maximumBytes = 65536) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let received = 0;
    request.on('data', chunk => {
      received += chunk.length;
      if (received > maximumBytes) {
        reject(new Error('request body is too large'));
        request.destroy();
        return;
      }
      chunks.push(chunk);
    });
    request.on('end', () => resolve(Buffer.concat(chunks)));
    request.on('error', reject);
  });
}

function previewDocument(sessionID, generation, token) {
  const query = `session_id=${encodeURIComponent(sessionID)}&generation=${encodeURIComponent(generation)}&token=${encodeURIComponent(token)}`;
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>
    html,body{width:100%;height:100%;margin:0;overflow:hidden;background:#111318}body{display:grid;place-items:center;outline:none}
    #frame{display:block;width:100%;height:100%;object-fit:contain;user-select:none;-webkit-user-drag:none}
    #empty{position:fixed;color:#8b909a;font:13px -apple-system,BlinkMacSystemFont,sans-serif}
  </style></head><body tabindex="0"><div id="empty">Loading page…</div><img id="frame" alt="Browser page" decoding="async"><textarea id="keyboard" aria-hidden="true" tabindex="-1" style="position:fixed;width:1px;height:1px;opacity:0;pointer-events:none"></textarea><script>
    const query=${JSON.stringify(query)}, frame=document.getElementById('frame'), empty=document.getElementById('empty'), keyboard=document.getElementById('keyboard');
    let timer=null,inputQueue=Promise.resolve(),inputDepth=0,wheelTimer=null,wheelX=0,wheelY=0;
    function scheduleRefresh(delay=650){clearTimeout(timer);if(inputDepth===0)timer=setTimeout(refresh,delay)}
    function refresh(){if(inputDepth===0)frame.src='/frame?'+query+'&t='+Date.now();else scheduleRefresh()}
    frame.onload=()=>{empty.hidden=true;scheduleRefresh()};
    frame.onerror=()=>{empty.hidden=false;scheduleRefresh(900)};
    function send(payload){inputDepth+=1;clearTimeout(timer);inputQueue=inputQueue.then(async()=>{try{await fetch('/input?'+query,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(payload)})}catch{}}).finally(()=>{inputDepth=Math.max(0,inputDepth-1);if(inputDepth===0)refresh()});return inputQueue}
    function point(event){const r=frame.getBoundingClientRect(),scale=Math.min(r.width/frame.naturalWidth,r.height/frame.naturalHeight),w=frame.naturalWidth*scale,h=frame.naturalHeight*scale,left=r.left+(r.width-w)/2,top=r.top+(r.height-h)/2;return{x:(event.clientX-left)/scale,y:(event.clientY-top)/scale}}
    frame.addEventListener('click',event=>{const p=point(event);if(p.x>=0&&p.y>=0&&p.x<=frame.naturalWidth&&p.y<=frame.naturalHeight)send({kind:'click',...p}).then(()=>keyboard.focus({preventScroll:true}))});
    frame.addEventListener('wheel',event=>{event.preventDefault();wheelX+=event.deltaX;wheelY+=event.deltaY;clearTimeout(wheelTimer);wheelTimer=setTimeout(()=>{const delta_x=wheelX,delta_y=wheelY;wheelX=0;wheelY=0;send({kind:'wheel',delta_x,delta_y})},45)},{passive:false});
    keyboard.addEventListener('input',()=>{const text=keyboard.value;keyboard.value='';if(text)send({kind:'text',text})});
    keyboard.addEventListener('keydown',event=>{if(event.isComposing)return;const controlKey=event.key.length>1||event.metaKey||event.ctrlKey||event.altKey;if(!controlKey)return;event.preventDefault();send({kind:'key',key:event.key,code:event.code,meta:event.metaKey,control:event.ctrlKey,alt:event.altKey,shift:event.shiftKey})});
    refresh();
  </script></body></html>`;
}

const previewReady = new Promise((resolve, reject) => {
  const server = http.createServer(async (request, response) => {
    try {
      const requestURL = new URL(request.url || '/', 'http://127.0.0.1');
      const suppliedToken = requestURL.searchParams.get('token')
        || request.headers['x-nola-playwright-token'];
      if (suppliedToken !== previewToken) {
        response.writeHead(403).end('Forbidden');
        return;
      }
      if (request.method === 'POST' && requestURL.pathname === '/rpc') {
        const payload = JSON.parse((await requestBody(request)).toString('utf8'));
        const result = await handle(payload);
        const bytes = Buffer.from(JSON.stringify({ result }), 'utf8');
        response.writeHead(200, {
          'content-type': 'application/json; charset=utf-8',
          'cache-control': 'no-store',
          'content-length': bytes.length,
        });
        response.end(bytes);
        return;
      }
      const sessionID = cleanSessionID(requestURL.searchParams.get('session_id'));
      const session = sessions.get(sessionID);
      if (!session) {
        response.writeHead(404).end('Browser session is not running');
        return;
      }
      if (requestURL.searchParams.get('generation') !== session.generation || session.closing) {
        response.writeHead(409).end('Browser generation changed; reopen the current preview');
        return;
      }
      if (request.method === 'GET' && requestURL.pathname === '/view') {
        response.writeHead(200, {'content-type':'text/html; charset=utf-8','cache-control':'no-store'});
        response.end(previewDocument(sessionID, session.generation, previewToken));
        return;
      }
      if (request.method === 'GET' && requestURL.pathname === '/frame') {
        const frame = await ownerOperations.run(sessionID, async () => {
          if (sessions.get(sessionID) !== session || session.closing) throw new Error('browser owner changed');
          return previewFrame(session);
        });
        const etag = `"${session.id}-${frame.revision}-${frame.capturedAt}"`;
        if (request.headers['if-none-match'] === etag) {
          response.writeHead(304, {'cache-control':'private, max-age=0, must-revalidate','etag':etag}).end();
          return;
        }
        response.writeHead(200, {'content-type':'image/jpeg','cache-control':'private, max-age=0, must-revalidate','etag':etag,'content-length':frame.bytes.length});
        response.end(frame.bytes);
        return;
      }
      if (request.method === 'POST' && requestURL.pathname === '/input') {
        const payload = JSON.parse((await requestBody(request)).toString('utf8'));
        await ownerOperations.run(sessionID, async () => {
        if (sessions.get(sessionID) !== session || session.closing) throw new Error('browser owner changed');
        const page = activePage(session);
        if (payload.kind === 'click') {
          await page.mouse.click(Number(payload.x), Number(payload.y));
        } else if (payload.kind === 'wheel') {
          await page.mouse.wheel(Number(payload.delta_x || 0), Number(payload.delta_y || 0));
        } else if (payload.kind === 'key') {
          const modifiers = [payload.meta&&'Meta',payload.control&&'Control',payload.alt&&'Alt',payload.shift&&'Shift'].filter(Boolean);
          const key = String(payload.key || '');
          if (key.length === 1 && modifiers.length === 0) await page.keyboard.insertText(key);
          else await page.keyboard.press([...modifiers, key].filter(Boolean).join('+'));
        } else if (payload.kind === 'text') {
          const text = String(payload.text || '');
          if (text) await page.keyboard.insertText(text);
        }
        clearReferences(session);
        session.revision += 1;
        invalidatePreview(session);
        touchSession(session);
        });
        response.writeHead(204).end();
        return;
      }
      response.writeHead(404).end('Not found');
    } catch (error) {
      response.writeHead(500).end(error?.message || String(error));
    }
  });
  server.once('error', reject);
  server.listen(0, '127.0.0.1', () => {
    previewPort = server.address().port;
    if (controlFile) {
      fs.mkdirSync(path.dirname(controlFile), { recursive: true });
      const temporary = `${controlFile}.${process.pid}.tmp`;
      fs.writeFileSync(temporary, JSON.stringify({
        pid: process.pid,
        rpc_url: `http://127.0.0.1:${previewPort}/rpc`,
        token: previewToken,
      }), { mode: 0o600 });
      fs.renameSync(temporary, controlFile);
    }
    resolve();
  });
});

function runtimeFile(profileRoot) {
  return path.join(profileRoot, '.nola-browser-runtime.json');
}

function activePageFile(profileRoot) {
  return path.join(profileRoot, '.nola-browser-active-page.json');
}

function pageURLDigest(page) {
  return crypto.createHash('sha256').update(page.url()).digest('hex');
}

function saveActivePage(session) {
  if (!session.persistent) return;
  const index = session.context.pages().indexOf(session.activePage);
  if (index < 0) return;
  fs.writeFileSync(activePageFile(session.profileRoot), JSON.stringify({ index, url_digest: pageURLDigest(session.activePage) }), { mode: 0o600 });
}

function restoredActivePage(context, profileRoot) {
  let saved;
  try {
    saved = JSON.parse(fs.readFileSync(activePageFile(profileRoot), 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw new Error('managed browser selection metadata is invalid; it was not replaced');
  }
  if (!saved || Object.keys(saved).some(key => !['index', 'url_digest'].includes(key))
      || !Number.isSafeInteger(saved.index) || saved.index < 0
      || typeof saved.url_digest !== 'string' || !/^[a-f0-9]{64}$/.test(saved.url_digest)) {
    throw new Error('managed browser selection metadata has an unsupported format; it was not replaced');
  }
  // Index alone is not page identity. Duplicate URLs are ambiguous too: leave
  // selection unset so an explicit list/switch operation can resolve it.
  const matches = context.pages().filter(page => pageURLDigest(page) === saved.url_digest);
  return matches.length === 1 ? matches[0] : null;
}

async function freeLoopbackPort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

async function createSession({ id, context, browser = null, persistent, headless, profileRoot }) {
  const pages = context.pages();
  const selected = restoredActivePage(context, profileRoot)
    || (pages.length === 1 && pages[0].url() === 'about:blank' ? pages[0] : null)
    || (pages.length === 0 ? await context.newPage() : null);
  const session = {
    id, context, browser, persistent, headless, profileRoot,
    generation: crypto.randomUUID(), closing: false,
    activePage: selected, refs: new Map(), revision: 0,
    console: [], network: [], lastUsedAt: Date.now(), previewFrame: null, previewCapture: null,
  };
  const observePage = page => {
    page.on('framenavigated', frame => {
      if (frame === page.mainFrame()) {
        pageDocumentGenerations.set(page, (pageDocumentGenerations.get(page) || 0) + 1);
        if (page === session.activePage) invalidatePreview(session);
      }
    });
    page.on('console', message => boundedPush(session.console, { type: message.type(), text: message.text().slice(0, 2_000) }));
    page.on('requestfailed', request => boundedPush(session.network, {
      method: request.method(), url: request.url(), error: request.failure()?.errorText || 'failed',
    }));
  };
  context.on('page', page => {
    touchSession(session);
    observePage(page);
    // Popups join this owner's page list, but cannot take over the selected
    // page while a command is awaiting its action/observation. Selection is an
    // explicit owner operation, never an asynchronous browser event.
  });
  for (const page of context.pages()) observePage(page);
  context.on('close', () => {
    if (sessions.get(id) === session) sessions.delete(id);
    try { fs.unlinkSync(runtimeFile(profileRoot)); } catch {}
    try { fs.unlinkSync(activePageFile(profileRoot)); } catch {}
  });
  sessions.set(id, session);
  return session;
}

async function reconnectSession(id, profileRoot, headless) {
  let runtime;
  try {
    runtime = JSON.parse(fs.readFileSync(runtimeFile(profileRoot), 'utf8'));
  } catch {
    return null;
  }
  if (!Number.isInteger(runtime.port)) return null;
  let browser;
  try {
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${runtime.port}`, {
      timeout: 2500,
    });
  } catch {
    try { fs.unlinkSync(runtimeFile(profileRoot)); } catch {}
    return null;
  }
  const context = browser.contexts()[0];
  if (!context) return null;
  // Invalid current-format selection is not a failed connection. Propagate it
  // rather than deleting runtime metadata and launching over the same profile.
  return createSession({ id, context, browser, persistent: true, headless, profileRoot });
}

async function launchSession(args) {
  await previewReady;
  const id = cleanSessionID(args.session_id);
  const existing = sessions.get(id);
  if (existing) {
    touchSession(existing);
    return sessionSummary(existing);
  }
  startingSessions.add(id);
  try {
  await evictIdleSessions(id, true);
  if (sessions.size + startingSessions.size > maximumLiveSessions) {
    throw new Error('browser capacity is busy; no in-flight owner was evicted');
  }

  const persistent = args.persistent !== false;
  // The managed browser is rendered inside Nola's inspector. It must never
  // create a second desktop window.
  const headless = true;
  const profileRoot = persistent
    ? path.join(stateRoot, 'Profiles', id)
    : fs.mkdtempSync(path.join(os.tmpdir(), `nola-browser-${id}-`));
  fs.mkdirSync(profileRoot, { recursive: true });
  if (persistent) {
    const reconnected = await reconnectSession(id, profileRoot, headless);
    if (reconnected) return sessionSummary(reconnected);
  }
  const debuggingPort = await freeLoopbackPort();
  const context = await chromium.launchPersistentContext(profileRoot, {
    channel: 'chrome',
    headless,
    chromiumSandbox: true,
    args: [`--remote-debugging-port=${debuggingPort}`],
    viewport: { width: 1440, height: 960 },
    acceptDownloads: true,
    locale: args.locale || undefined,
  });
  fs.writeFileSync(runtimeFile(profileRoot), JSON.stringify({
    port: debuggingPort,
    nonce: crypto.randomUUID(),
    pid: process.pid,
  }), { mode: 0o600 });
  const session = await createSession({ id, context, persistent, headless, profileRoot });
  return sessionSummary(session);
  } finally { startingSessions.delete(id); }
}

async function requireSession(args, allowLaunch = false) {
  const id = cleanSessionID(args.session_id);
  const session = sessions.get(id) || (allowLaunch ? (await launchSession(args), sessions.get(id)) : null);
  if (!session || session.closing) throw new Error('managed browser owner is unavailable');
  touchSession(session);
  return session;
}

function activePage(session) {
  if (session.closing || !session.context.pages().includes(session.activePage)) {
    throw new Error('the owned page was closed; list pages and explicitly select a current page');
  }
  return session.activePage;
}

async function snapshot(session, args) {
  const page = activePage(session);
  await page.waitForLoadState('domcontentloaded', { timeout: args.timeout_ms || 15000 }).catch(() => {});
  const documentIdentity = pageIdentity(page);
  session.revision += 1;
  clearReferences(session);
  invalidatePreview(session);
  touchSession(session);
  const maximumElements = Math.max(1, Math.min(args.max_elements || 160, 400));
  // Retain the exact sampled DOM nodes, not nth() locators that silently
  // resolve a different node after a re-render or same-URL navigation.
  const sampled = await page.evaluateHandle(limits => {
    const nodes = document.querySelectorAll('a,button,input,textarea,select,[role],[contenteditable="true"],[tabindex]');
    const selected = [];
    const scanCount = Math.min(nodes.length, limits.scan);
    for (let index = 0; index < scanCount && selected.length < limits.results; index += 1) {
      const element = nodes[index];
      const rect = element.getBoundingClientRect();
      const style = window.getComputedStyle(element);
      if (rect.width <= 0 || rect.height <= 0 || style.visibility === 'hidden' || style.display === 'none') continue;
      selected.push(element);
    }
    return selected;
  }, { scan: Math.min(maximumElements * 4, 1_600), results: maximumElements });
  const elements = [];
  try {
    const candidates = await sampled.evaluate(nodes => nodes.map(element => {
      const role = element.getAttribute('role') || ({
        A: 'link', BUTTON: 'button', INPUT: 'textbox', TEXTAREA: 'textbox', SELECT: 'combobox',
      }[element.tagName] || element.tagName.toLowerCase());
      const name = element.getAttribute('aria-label')
        || element.getAttribute('title')
        || element.getAttribute('placeholder')
        || element.innerText
        || element.value
        || '';
      return {
        role,
        name: String(name).replace(/\s+/g, ' ').trim().slice(0, 240),
        disabled: element.matches(':disabled,[aria-disabled="true"]'),
        checked: element.matches(':checked,[aria-checked="true"]'),
      };
    }));
    const handles = await sampled.getProperties();
    for (let index = 0; index < candidates.length; index += 1) {
      const element = handles.get(String(index))?.asElement();
      if (!element) throw new Error('snapshot element identity is unavailable');
      const ref = `e${session.generation.replaceAll('-', '')}_${session.revision}_${elements.length + 1}`;
      session.refs.set(ref, { element, revision: session.revision, page,
        generation: session.generation, documentIdentity });
      elements.push({ ref, ...candidates[index] });
    }
  } catch (error) {
    clearReferences(session);
    throw error;
  } finally {
    await sampled.dispose();
  }
  const body = page.locator('body');
  const maxAccessibility = Math.max(1_000, Math.min(args.max_accessibility_chars || 20_000, 80_000));
  const accessibility = (await body.ariaSnapshot({ timeout: 5000 }).catch(() => '')).slice(0, maxAccessibility);
  const maxText = Math.max(500, Math.min(args.max_text_chars || 12000, 80000));
  const text = (await body.innerText({ timeout: 5000 }).catch(() => '')).slice(0, maxText);
  const result = {
    ok: true, target: 'managed', session_id: session.id, generation: session.generation, revision: session.revision,
    page_id: documentIdentity, url: page.url(), title: await page.title(), text, accessibility, elements,
    viewport: page.viewportSize(), preview_url: sessionPreviewURL(session),
    pages: await Promise.all(session.context.pages().map(async (item, index) => ({
      index, url: item.url(), title: await item.title(), active: item === session.activePage,
    }))),
  };
  if (activePage(session) !== page || pageIdentity(page) !== documentIdentity) {
    clearReferences(session);
    throw new Error('owned page changed during observation; take a new snapshot before acting');
  }
  if (args.screenshot === true) Object.assign(result, await screenshot(session, args));
  return result;
}

/// A completed action is never reported as failed just because observation
/// failed. An uncertain action is not retried by this bridge.
async function actThenObserve(session, args, operation, action) {
  let receipt;
  try {
    await action();
    receipt = { operation, status: 'completed' };
  } catch (error) {
    clearReferences(session);
    invalidatePreview(session);
    return { ok: false, target: 'managed', session_id: session.id,
      generation: session.generation, action: { operation, status: 'unknown' },
      error: { code: 'action_outcome_unknown', message: error?.message || String(error),
        recovery: 'Inspect the owned page before deciding whether to repeat this action.' } };
  }
  clearReferences(session);
  invalidatePreview(session);
  try {
    return { ...(await snapshot(session, args)), action: receipt, observation: { status: 'completed' } };
  } catch (error) {
    return { ok: true, ...sessionSummary(session), action: receipt,
      observation: { status: 'failed', message: error?.message || String(error),
        recovery: 'The action completed. Observe again without repeating the action.' } };
  }
}

async function screenshot(session, args) {
  const page = activePage(session);
  const bytes = await page.screenshot({ type: 'png', fullPage: args.full_page === true });
  return {
    screenshot_data_url: `data:image/png;base64,${bytes.toString('base64')}`,
    screenshot_mime_type: 'image/png',
    viewport: page.viewportSize(),
  };
}

function referencedElement(session, args) {
  const ref = String(args.ref || '').trim();
  const entry = session.refs.get(ref);
  if (!entry) throw new Error(`unknown or stale element ref '${ref}'; call browser_snapshot again`);
  if (!Number.isInteger(args.revision) || args.revision !== entry.revision
      || entry.page !== activePage(session) || entry.generation !== session.generation
      || entry.documentIdentity !== pageIdentity(entry.page)) {
    throw new Error(`stale snapshot revision for '${ref}'; call browser_snapshot again`);
  }
  return entry.element;
}

async function handle(request) {
  const args = request.args || {};
  if (String(request.action || 'status') === 'status' && !args.session_id) return handleOwned(request);
  const owner = cleanSessionID(args.session_id);
  return ownerOperations.run(owner, () => handleOwned(request));
}

async function handleOwned(request) {
  await previewReady;
  const action = String(request.action || 'status');
  const args = request.args || {};
  if (action === 'permission_resource') return permissionResource(args);
  if (args._nola_permission_resource) {
    const actual = permissionResource(args), expected = args._nola_permission_resource;
    if (['target', 'owner_id', 'connection_state', 'connection_generation', 'page_id', 'page_origin', 'requested_origin']
        .some(key => actual[key] !== expected[key])) {
      return { ok: false, target: 'managed', action: { operation: action, status: 'not_started' },
        error: { code: 'permission_resource_changed', message: 'Owned browser resource changed; action was not started.' } };
    }
  }
  if (action === 'status') {
    if (args.session_id) {
      const id = cleanSessionID(args.session_id);
      const session = sessions.get(id);
      return {
        ok: true,
        target: 'managed',
        available: true,
        sessions: session ? [sessionSummary(session)] : [],
      };
    }
    return { ok: true, target: 'managed', available: true, sessions: [...sessions.values()].map(sessionSummary) };
  }
  if (action === 'start') return { ok: true, ...(await launchSession(args)) };
  if (action === 'stop') {
    const id = cleanSessionID(args.session_id);
    const session = sessions.get(id);
    if (session) await session.context.close();
    sessions.delete(id);
    return { ok: true, target: 'managed', session_id: id, running: false };
  }
  const session = await requireSession(args, action === 'navigate' || action === 'open_page');
  touchSession(session);
  if (action === 'snapshot' || action === 'inspect') return snapshot(session, args);
  if (action === 'screenshot') return { ok: true, target: 'managed', session_id: session.id, ...(await screenshot(session, args)) };
  if (action === 'navigate' || action === 'open_page') {
    const page = args.new_page === true ? null : activePage(session);
    return actThenObserve(session, args, 'navigate', async () => {
      const destination = args.new_page === true ? await session.context.newPage() : page;
      session.activePage = destination;
      await destination.goto(String(args.url), { waitUntil: 'commit', timeout: args.timeout_ms || 30000 });
      saveActivePage(session);
    });
  }
  if (action === 'click') {
    const element = referencedElement(session, args);
    return actThenObserve(session, { ...args, screenshot: args.screenshot !== false }, 'click',
      () => element.click({ timeout: args.timeout_ms || 15000 }));
  }
  if (action === 'type_text') {
    const element = referencedElement(session, args);
    return actThenObserve(session, { ...args, screenshot: args.screenshot === true }, 'type_text', async () => {
      if (args.clear !== false) await element.fill(String(args.text || ''), { timeout: args.timeout_ms || 15000 });
      else await element.type(String(args.text || ''), { delay: args.delay_ms || 20, timeout: args.timeout_ms || 15000 });
    });
  }
  if (action === 'press_key') {
    const locator = args.ref ? referencedElement(session, args) : activePage(session).locator('body');
    return actThenObserve(session, args, 'press_key',
      () => locator.press(String(args.key), { timeout: args.timeout_ms || 15000 }));
  }
  if (action === 'tabs' || action === 'list_pages') {
    return {
      ok: true, target: 'managed', session_id: session.id,
      preview_url: sessionPreviewURL(session),
      pages: await Promise.all(session.context.pages().map(async (item, index) => ({
        index, url: item.url(), title: await item.title(), active: item === session.activePage,
      }))),
    };
  }
  if (action === 'switch_page') {
    const pages = session.context.pages();
    const index = Number(args.index);
    if (!Number.isInteger(index) || index < 0 || index >= pages.length) throw new Error('page index is out of range');
    session.activePage = pages[index];
    saveActivePage(session);
    clearReferences(session);
    return snapshot(session, args);
  }
  throw new Error(`unsupported managed browser action '${action}'`);
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', async line => {
  let id = null;
  try {
    const request = JSON.parse(line);
    id = request.id ?? null;
    const result = await handle(request);
    process.stdout.write(`${JSON.stringify({ id, result })}\n`);
  } catch (error) {
    process.stdout.write(`${JSON.stringify({ id, error: error?.message || String(error) })}\n`);
  }
});

async function shutdown() {
  if (controlFile) {
    try { fs.unlinkSync(controlFile); } catch {}
  }
  for (const session of sessions.values()) {
    await session.context.close().catch(() => {});
  }
  process.exit(0);
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
const idleSessionMonitor = setInterval(() => {
  evictIdleSessions().catch(() => {});
}, 60_000);
idleSessionMonitor.unref();
if (parentPID > 1) {
  const parentMonitor = setInterval(() => {
    if (!parentIsAlive()) shutdown();
  }, 1_000);
  parentMonitor.unref();
}
