// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements background logic in the web client.
// - Interfaces:
//   - HOST_NAME
//   - CAPABILITIES
//   - nativePort
//   - reconnectTimer
//   - reconnectDelayMs
//   - nativeStatus
//   - COMMAND_TIMEOUT_MS
//   - COMMAND_RESULT_TTL_MS
// - APIs:
//   - none
//
const HOST_NAME = "com.nola.native_browser";
const ownedTabIds = new Map();
const ownerCommandTails = new Map();
// Ephemeral admission receipts for one exact document injection. The page cannot
// turn an absent execution receipt into permission to repeat an input.
const boundInteractionExecutions = new Map();
const CAPABILITIES = [
  "status",
  "permission_resource",
  "inspect",
  "dom_snapshot",
  "accessibility_snapshot",
  "storage_state",
  "console_logs",
  "network_events",
  "list_pages",
  "switch_page",
  "open_page",
  "hover",
  "click",
  "drag",
  "type_text",
  "select_option",
  "set_checked",
  "press_key",
  "scroll",
  "assert",
  "wait_for",
  "screenshot",
  "search_results",
];

let nativePort = null;
let reconnectTimer = null;
let reconnectDelayMs = 1500;
let nativeConnectedAt = 0;
const STABLE_CONNECTION_MS = 60_000;
let nativeStatus = {
  state: "idle",
  detail: "Native host has not been contacted yet.",
  updatedAt: Date.now(),
};
const COMMAND_TIMEOUT_MS = 45_000;
const COMMAND_RESULT_TTL_MS = 10 * 60 * 1000;
const COMMAND_RESULT_CACHE_MAX = 200;
const inFlightCommandIds = new Set();
const recentCommandResults = new Map();

function setNativeStatus(state, detail) {
  nativeStatus = { state, detail: String(detail || ""), updatedAt: Date.now() };
  console.log("Nola native status", nativeStatus);
}

function scheduleReconnect() {
  if (reconnectTimer) return;
  const delay = Math.min(reconnectDelayMs, 30_000) + Math.floor(Math.random() * 250);
  reconnectDelayMs = Math.min(Math.round(reconnectDelayMs * 1.7), 30_000);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectNativeHost();
  }, delay);
}

function postNative(message) {
  if (!nativePort) {
    setNativeStatus("disconnected", "Native port is not connected; scheduling reconnect.");
    scheduleReconnect();
    return;
  }
  try {
    nativePort.postMessage(message);
  } catch (error) {
    console.warn("Nola native post failed", error);
    nativePort = null;
    scheduleReconnect();
  }
}

function connectNativeHost() {
  if (nativePort) {
    setNativeStatus("connected", "Native port already connected.");
    return;
  }
  if (!chrome.runtime || typeof chrome.runtime.connectNative !== "function") {
    setNativeStatus("unavailable", "chrome.runtime.connectNative is unavailable. Check the nativeMessaging permission.");
    return;
  }
  try {
    setNativeStatus("connecting", `Connecting to ${HOST_NAME}...`);
    nativePort = chrome.runtime.connectNative(HOST_NAME);
  } catch (error) {
    const detail = String(error && error.message ? error.message : error);
    setNativeStatus("error", detail);
    console.warn("Nola native host connect failed", error);
    scheduleReconnect();
    return;
  }

  nativePort.onMessage.addListener((message) => {
    if (!message) return;
    if (message.type === "host_ready") {
      nativeConnectedAt = Date.now();
      setNativeStatus("connected", `Connected to ${HOST_NAME}.`);
      return;
    }
    if (message.type === "command") handleCommandMessage(message.command);
  });

  nativePort.onDisconnect.addListener(() => {
    const error = chrome.runtime.lastError;
    setNativeStatus("disconnected", error ? error.message : "Native host disconnected.");
    if (error) console.warn("Nola native host disconnected", error.message);
    if (nativeConnectedAt > 0 && Date.now() - nativeConnectedAt >= STABLE_CONNECTION_MS) {
      reconnectDelayMs = 1500;
    }
    nativeConnectedAt = 0;
    nativePort = null;
    scheduleReconnect();
  });

  setNativeStatus("connecting", `Waiting for ${HOST_NAME} handshake...`);
  postNative({ type: "hello", capabilities: CAPABILITIES, extension_version: chrome.runtime.getManifest().version });
}

chrome.runtime.onInstalled.addListener(connectNativeHost);
chrome.runtime.onStartup.addListener(connectNativeHost);
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message && message.type === "bound_interaction_started") {
    const execution = boundInteractionExecutions.get(message.binding_id);
    const accepted = Boolean(execution && !execution.started &&
      _sender.tab?.id === execution.tabId && _sender.frameId === execution.frameId &&
      _sender.documentId === execution.documentId);
    if (accepted) execution.started = true;
    sendResponse({ accepted });
    return false;
  }
  if (message && message.type === "popup_ping") {
    connectNativeHost();
    sendResponse({ ok: true, nativeStatus });
    return true;
  }
  if (message && message.type === "native_status") {
    sendResponse({ ok: true, nativeStatus });
    return true;
  }
  return false;
});
chrome.action.onClicked.addListener(connectNativeHost);
connectNativeHost();

function nowMs() {
  return Date.now();
}

function cleanupCommandResultCache() {
  const cutoff = nowMs() - COMMAND_RESULT_TTL_MS;
  for (const [commandId, entry] of recentCommandResults.entries()) {
    if (!entry || Number(entry.at || 0) < cutoff) {
      recentCommandResults.delete(commandId);
    }
  }
  while (recentCommandResults.size > COMMAND_RESULT_CACHE_MAX) {
    const firstKey = recentCommandResults.keys().next().value;
    if (!firstKey) break;
    recentCommandResults.delete(firstKey);
  }
}

function cacheCommandResult(commandId, result) {
  if (!commandId) return;
  recentCommandResults.set(commandId, { at: nowMs(), result });
  cleanupCommandResultCache();
}

function withTimeout(promise, timeoutMs, message) {
  const timeout = Math.max(1000, Number(timeoutMs || COMMAND_TIMEOUT_MS));
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      setTimeout(() => reject(new Error(message || `Operation timed out after ${timeout}ms`)), timeout);
    }),
  ]);
}

async function handleCommandMessage(command) {
  const commandId = command && command.id ? String(command.id) : "";
  if (!commandId) {
    postNative({
      type: "result",
      command_id: "",
      result: { ok: false, error: "Received command without command_id." },
    });
    return;
  }

  cleanupCommandResultCache();
  const cached = recentCommandResults.get(commandId);
  if (cached && cached.result) {
    postNative({ type: "result", command_id: commandId, result: cached.result });
    return;
  }
  if (inFlightCommandIds.has(commandId)) {
    return;
  }

  inFlightCommandIds.add(commandId);
  let result;
  const timeoutMs = commandTimeoutMs(command || {});
  try {
    result = await withTimeout(
      executeOrderedCommand(command || {}),
      timeoutMs,
      `Command timed out after ${timeoutMs}ms`,
    );
  } catch (error) {
    result = { ok: false, error: String(error && error.message ? error.message : error) };
  } finally {
    inFlightCommandIds.delete(commandId);
  }
  cacheCommandResult(commandId, result);
  postNative({ type: "result", command_id: commandId, result });
}

function commandTimeoutMs(command) {
  if (command && command.action === "wait_for") {
    const args = command.args || {};
    const timeoutMs = Math.max(0, Math.min(Number(args.timeout_ms || 5000), 60_000));
    return Math.max(COMMAND_TIMEOUT_MS, timeoutMs + 5_000);
  }
  return COMMAND_TIMEOUT_MS;
}

async function executeCommand(command) {
  const action = command.action;
  const args = command.args || {};
  if (action === "permission_resource") return permissionResource(args);
  if (args._nola_permission_resource) {
    const actual = await permissionResource(args), expected = args._nola_permission_resource;
    if (["target", "owner_id", "connection_state", "connection_generation", "page_id", "page_origin", "requested_origin"]
      .some(key => actual[key] !== expected[key])) {
      return { ok: false, target: "attached", action: { operation: action, status: "not_started" },
        error: { code: "permission_resource_changed", message: "Owned browser resource changed; action was not started." } };
    }
  }
  if (action === "status") {
    const resource = await permissionResource(args);
    return { ok: true, target: "attached", running: resource.connection_state === "existing", resource };
  }
  const observesOwnedPage = !["list_pages", "switch_page", "open_page", "search_results"].includes(action);
  const ownedPage = observesOwnedPage ? await requireOwnedTab(args.nola_owner_id) : null;
  if (action === "open_page") return openPage(args);
  if (action === "search_results") return searchResults(args);
  if (action === "hover") return interact("hover", args);
  if (action === "click") return interact("click", args);
  if (action === "drag") return interact("drag", args);
  if (action === "type_text") return interact("type_text", args);
  if (action === "select_option") return interact("select_option", args);
  if (action === "set_checked") return interact("set_checked", args);
  if (action === "press_key") return interact("press_key", args);
  if (action === "scroll") return interact("scroll", args);
  if (action === "assert") return assertCondition(args);
  if (action === "wait_for") return waitForCondition(args);
  if (action === "inspect") return captureState({ tabId: ownedPage.id, screenshot: args.screenshot !== false });
  if (action === "screenshot") return captureState({ tabId: ownedPage.id, screenshot: true });
  if (action === "dom_snapshot") return domSnapshot(args);
  if (action === "accessibility_snapshot") return accessibilitySnapshot(args);
  if (action === "storage_state") return storageState(args);
  if (action === "console_logs") return diagnosticsEvents("console_logs", args);
  if (action === "network_events") return diagnosticsEvents("network_events", args);
  if (action === "list_pages") return listPages(args);
  if (action === "switch_page") return switchPage(args);
  throw new Error(`Unsupported native browser action: ${action}`);
}

function executeOrderedCommand(command) {
  const owner = String(command.args?.nola_owner_id || "").trim();
  if (!owner) return Promise.reject(new Error("Browser command has no owner"));
  const previous = ownerCommandTails.get(owner) || Promise.resolve();
  // Chain the actual operation, not the response timeout: a timed-out input
  // can still be running in Chrome and must not overlap the next owner action.
  const result = previous.then(() => executeCommand(command));
  const tail = result.then(() => undefined, () => undefined);
  ownerCommandTails.set(owner, tail);
  tail.then(() => { if (ownerCommandTails.get(owner) === tail) ownerCommandTails.delete(owner); });
  return result;
}

async function bindOwnedTab(ownerId, tabId) {
  const owner = String(ownerId || "").trim();
  if (!owner || !Number.isInteger(tabId)) throw new Error("Browser owner and page identity are required");
  await chrome.storage.session.set({ [`nola.browser.owner:${owner}`]: tabId });
  ownedTabIds.set(owner, tabId);
}

async function permissionResource(args) {
  const owner = String(args.nola_owner_id || "").trim();
  if (!owner) throw new Error("Browser resource requires an owner");
  const origin = value => { try { const url = new URL(value); return ["http:", "https:"].includes(url.protocol) ? url.origin : null; } catch { return null; } };
  const stored = await chrome.storage.session.get(`nola.browser.owner:${owner}`);
  let tabId = ownedTabIds.get(owner) ?? stored[`nola.browser.owner:${owner}`];
  if (args.index != null) {
    if (!Number.isInteger(args.index) || args.index < 1) throw new Error("Permission resource page index is invalid");
    const selected = (await collectPages()).find(page => page.index === args.index);
    if (!selected) throw new Error("Permission resource page no longer exists");
    tabId = selected.tab_id;
  }
  const tab = Number.isInteger(tabId) ? await chrome.tabs.get(tabId).catch(() => null) : null;
  // Chrome's document identity detects same-URL reloads without touching DOM,
  // changing the foreground tab, or inventing a generation during preflight.
  const frame = tab ? await chrome.webNavigation.getFrame({ tabId: tab.id, frameId: 0 }).catch(() => null) : null;
  return { ok: true, target: "attached", owner_id: owner,
    connection_state: tab ? "existing" : "absent",
    connection_generation: frame?.documentId ? `tab:${tab.id}:document:${frame.documentId}` : null,
    page_id: tab ? String(tab.id) : null, page_origin: tab ? origin(tab.url) : null,
    requested_origin: origin(args.url), authentication_state: "unknown" };
}

async function requireOwnedTab(ownerId) {
  const owner = String(ownerId || "").trim();
  if (!owner) throw new Error("Browser command has no owner; select an explicit page");
  const key = `nola.browser.owner:${owner}`;
  const tabId = ownedTabIds.get(owner) || (await chrome.storage.session.get(key))[key];
  const tab = tabId ? await chrome.tabs.get(tabId).catch(() => null) : null;
  if (!tab) {
    ownedTabIds.delete(owner);
    await chrome.storage.session.remove(key);
    throw new Error("Owned browser page is unavailable; use open_page or switch_page to select a page");
  }
  return tab;
}

async function openPage(args) {
  const url = String(args.url || "").trim();
  if (!/^https?:\/\//i.test(url)) throw new Error("Only http(s) URLs can be opened");
  const owner = String(args.nola_owner_id || "").trim();
  if (!owner) throw new Error("Browser navigation requires an owner");
  let tab;
  if (args.new_tab) {
    tab = await chrome.tabs.create({ url, active: true });
  } else {
    const active = await requireOwnedTab(owner).catch(() => null);
    tab = active ? await chrome.tabs.update(active.id, { url, active: true }) : await chrome.tabs.create({ url, active: true });
  }
  await bindOwnedTab(owner, tab.id);
  try {
    await waitForTabComplete(tab.id, 30000);
    await installPageDiagnostics(tab.id);
    return { ...(await captureState({ tabId: tab.id, screenshot: args.screenshot !== false })),
      action_receipt: { action: "open_page", state: "executed", tab_id: tab.id, url } };
  } catch (error) {
    // Navigation has happened. Observation failure must not invite replaying it.
    return { ok: true, url, final_url: tab.url || url, title: tab.title || "",
      action_receipt: { action: "open_page", state: "executed", tab_id: tab.id, url },
      observation: { state: "failed", error: String(error?.message || error), retry_action: "inspect" } };
  }
}

async function searchResults(args) {
  const engine = String(args.engine || "google").trim().toLowerCase();
  if (!["google", "bing", "sogou"].includes(engine)) throw new Error("Unsupported search engine");
  const query = String(args.query || "").trim();
  if (!query || query.length > 500 || query.includes("\0")) throw new Error("Invalid search query");
  const limit = intArg(args, "limit", 20, 1, 20);
  const language = String(args.language || "").trim().slice(0, 32);
  const region = String(args.region || "").trim().slice(0, 16);
  const freshness = String(args.freshness || "none").trim().toLowerCase();
  const searchUrl = buildSearchUrl(engine, query, language, region, freshness, limit);
  let tab = null;
  try {
    tab = await chrome.tabs.create({ url: searchUrl, active: false });
    await waitForTabComplete(tab.id, 30_000);
    const injected = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: collectSearchResultsInPage,
      args: [engine, limit],
    });
    const payload = Array.isArray(injected) && injected[0] ? injected[0].result : null;
    if (!payload || payload.ok !== true) {
      throw new Error((payload && payload.error) || "Chrome search returned no structured result");
    }
    return {
      ok: true,
      engine,
      query,
      final_url: payload.url || searchUrl,
      title: payload.title || "",
      results: Array.isArray(payload.results) ? payload.results.slice(0, limit) : [],
    };
  } finally {
    if (tab && tab.id) await chrome.tabs.remove(tab.id).catch(() => null);
  }
}

function buildSearchUrl(engine, query, language, region, freshness, limit) {
  let url;
  if (engine === "google") {
    url = new URL("https://www.google.com/search");
    url.searchParams.set("q", query);
    url.searchParams.set("num", String(limit));
    url.searchParams.set("filter", "0");
    if (language) url.searchParams.set("hl", language);
    if (region) url.searchParams.set("gl", region);
    const tbs = { day: "qdr:d", week: "qdr:w", month: "qdr:m", year: "qdr:y" }[freshness];
    if (tbs) url.searchParams.set("tbs", tbs);
  } else if (engine === "bing") {
    url = new URL("https://www.bing.com/search");
    url.searchParams.set("q", query);
    url.searchParams.set("count", String(limit));
    if (language) url.searchParams.set("setlang", language);
    if (region) url.searchParams.set("cc", region);
  } else {
    url = new URL("https://www.sogou.com/web");
    url.searchParams.set("query", query);
    url.searchParams.set("num", String(limit));
  }
  return url.toString();
}

function collectSearchResultsInPage(engine, limit) {
  const clean = (value) => String(value || "").replace(/\s+/g, " ").trim();
  const unwrap = (raw) => {
    try {
      const url = new URL(raw, location.href);
      if (engine === "google" && url.hostname.endsWith("google.com") && url.pathname === "/url") {
        return url.searchParams.get("q") || url.searchParams.get("url") || raw;
      }
      return url.href;
    } catch (_error) {
      return "";
    }
  };
  const isUsable = (url) => {
    try {
      const parsed = new URL(url);
      if (!["http:", "https:"].includes(parsed.protocol)) return false;
      const host = parsed.hostname.toLowerCase();
      if (engine === "google" && (host.endsWith("google.com") || host.endsWith("googleusercontent.com"))) return false;
      if (engine === "bing" && host.endsWith("bing.com") && !parsed.pathname.startsWith("/ck/a")) return false;
      if (engine === "sogou" && host.endsWith("sogou.com") && !parsed.pathname.startsWith("/link")) return false;
      return true;
    } catch (_error) {
      return false;
    }
  };
  const selectors = {
    google: ["#search a:has(h3)", "a:has(h3)"],
    bing: ["#b_results li.b_algo h2 a", "#b_results h2 a"],
    sogou: [".vrwrap h3 a", ".results h3 a", "h3 a"],
  }[engine] || ["h2 a", "h3 a"];
  const anchors = [];
  const seenNodes = new Set();
  for (const selector of selectors) {
    for (const anchor of document.querySelectorAll(selector)) {
      if (seenNodes.has(anchor)) continue;
      seenNodes.add(anchor);
      anchors.push(anchor);
    }
  }
  const results = [];
  const seenUrls = new Set();
  for (const anchor of anchors) {
    const title = clean(anchor.querySelector("h2, h3")?.textContent || anchor.textContent);
    const url = unwrap(anchor.href || anchor.getAttribute("href"));
    if (!title || !isUsable(url) || seenUrls.has(url)) continue;
    const container = anchor.closest("li.b_algo, .MjjYud, .g, .vrwrap, .rb, article") || anchor.parentElement?.parentElement;
    let snippet = clean(container?.querySelector("p, .b_caption, .VwiC3b, .text-layout, .str_info")?.textContent || "");
    if (!snippet && container) snippet = clean(container.innerText).replace(title, "").trim();
    seenUrls.add(url);
    results.push({ title: title.slice(0, 500), url, snippet: snippet.slice(0, 2000) });
    if (results.length >= limit) break;
  }
  if (!results.length) {
    return {
      ok: false,
      error: `No structured ${engine} results were found; the page may be a challenge or consent page`,
      url: location.href,
      title: document.title || "",
    };
  }
  return { ok: true, url: location.href, title: document.title || "", results };
}

async function installPageDiagnostics(tabId) {
  try {
    await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      world: "MAIN",
      func: installDiagnosticsRecorder,
    });
  } catch (error) {
    console.warn("Nola page diagnostics install failed", error);
  }
}

async function diagnosticsEvents(kind, args = {}) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  await installPageDiagnostics(tab.id);
  const injected = await chrome.scripting.executeScript({
    target: { tabId: tab.id, allFrames: true },
    world: "MAIN",
    func: collectDiagnosticsEvents,
    args: [kind, args || {}],
  });
  const entries = Array.isArray(injected) ? injected.filter((entry) => entry && entry.result) : [];
  const events = [];
  const warnings = [];
  for (const entry of entries) {
    const result = entry.result || {};
    if (!result.ok) {
      if (result.error) warnings.push(String(result.error));
      continue;
    }
    const frameEvents = Array.isArray(result.events) ? result.events : [];
    for (const event of frameEvents) {
      if (!event || typeof event !== "object") continue;
      events.push({
        ...event,
        frame_id: entry.frameId || 0,
        page_url: event.page_url || result.url || "",
        title: event.title || result.title || "",
      });
    }
  }
  const level = String(args.level || (kind === "network_events" ? "warnings" : "all"));
  const maxEntries = intArg(args, "max_entries", 50, 1, 300);
  const matching = events.filter((event) => diagnosticEventMatches(kind, event, level));
  return {
    ok: true,
    kind,
    level,
    total_count: events.length,
    matching_count: matching.length,
    events: matching.slice(-maxEntries),
    warnings,
  };
}

async function collectPages() {
  const tabs = await chrome.tabs.query({});
  return (tabs || [])
    .slice()
    .sort((a, b) => {
      if ((a.windowId || 0) !== (b.windowId || 0)) return (a.windowId || 0) - (b.windowId || 0);
      return (a.index || 0) - (b.index || 0);
    })
    .map((tab, index) => ({
      index: index + 1,
      tab_id: tab.id || 0,
      window_id: tab.windowId || 0,
      window_index: tab.index || 0,
      active: Boolean(tab.active),
      highlighted: Boolean(tab.highlighted),
      pinned: Boolean(tab.pinned),
      title: tab.title || "",
      url: tab.url || "",
    }));
}

async function listPages(args = {}) {
  const owned = await requireOwnedTab(args.nola_owner_id).catch(() => null);
  const pages = (await collectPages()).map(page => ({ ...page, foreground: page.active, active: page.tab_id === owned?.id }));
  const active = pages.find((page) => page.active);
  return {
    ok: true,
    page_count: pages.length,
    active_index: active ? active.index : 0,
    pages,
  };
}

async function switchPage(args = {}) {
  const pages = await collectPages();
  const indexValue = args.index !== undefined && args.index !== null && args.index !== "" ? Number(args.index) : 0;
  const title = String(args.title || "").trim().toLowerCase();
  const url = String(args.url || "").trim().toLowerCase();
  const targetCount = [indexValue > 0, Boolean(title), Boolean(url)].filter(Boolean).length;
  if (targetCount !== 1) {
    throw new Error("provide exactly one switch target: index, title, or url");
  }

  let matches = [];
  if (indexValue > 0) {
    matches = pages.filter((page) => page.index === indexValue);
  } else if (title) {
    matches = pages.filter((page) => String(page.title || "").toLowerCase().includes(title));
  } else if (url) {
    matches = pages.filter((page) => String(page.url || "").toLowerCase().includes(url));
  }
  if (!matches.length) throw new Error("No matching Chrome page found");
  if (matches.length > 1) {
    throw new Error(`Multiple Chrome pages matched (${matches.map((page) => page.index).join(", ")}). Use list_pages and switch by index.`);
  }

  const page = matches[0];
  await bindOwnedTab(args.nola_owner_id, page.tab_id);
  if (page.window_id) await chrome.windows.update(page.window_id, { focused: true }).catch(() => null);
  const tab = await chrome.tabs.update(page.tab_id, { active: true });
  await sleep(200);
  try {
    return await captureState({ tabId: page.tab_id, screenshot: args.screenshot !== false });
  } catch (error) {
    return {
      ok: true,
      url: tab.url || page.url || "",
      final_url: tab.url || page.url || "",
      title: tab.title || page.title || "",
      visible_text: "",
      interactive_elements: [],
      message: `Switched Chrome page, but page state is unavailable: ${String(error && error.message ? error.message : error)}`,
    };
  }
}

function waitForTabComplete(tabId, timeoutMs) {
  return new Promise((resolve) => {
    let done = false;
    const timer = setTimeout(finish, timeoutMs);
    function finish() {
      if (done) return;
      done = true;
      clearTimeout(timer);
      chrome.tabs.onUpdated.removeListener(listener);
      resolve();
    }
    function listener(updatedTabId, info) {
      if (updatedTabId === tabId && info.status === "complete") finish();
    }
    chrome.tabs.onUpdated.addListener(listener);
    chrome.tabs.get(tabId, (tab) => {
      if (chrome.runtime.lastError || !tab || tab.status === "complete") finish();
    });
  });
}

async function interact(kind, args) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  await installPageDiagnostics(tab.id);
  const before = await captureState({ tabId: tab.id, screenshot: false }).catch(() => null);
  if (kind === "hover" || kind === "click" || kind === "drag" || kind === "type_text" || kind === "press_key") {
    try {
      const result = await trustedInteract(tab, kind, args || {});
      return annotateActionResult(kind, before, result);
    } catch (error) {
      const message = String(error && error.message ? error.message : error);
      // Input may already have reached Chrome. Do not replay it as DOM events
      // based on an error string or a missing observation receipt.
      return { ok: false, target: "attached", page: { tab_id: tab.id, window_id: tab.windowId, url: tab.url || "", title: tab.title || "" },
        action: { operation: kind, status: error?.actionStatus === "not_started" ? "not_started" : "unknown" },
        error: { code: error?.actionStatus === "not_started" ? "action_not_started" : "action_outcome_unknown", message,
          recovery: error?.actionStatus === "not_started" ? "Inspect the page and provide one exact target." :
            "Inspect the owned page before deciding whether to repeat the action." } };
    }
  }
  if (kind === "select_option" || kind === "set_checked") {
    try {
      await executeBoundPageInteraction(tab.id, kind, args || {});
      await sleep(intArg(args || {}, "wait_after_ms", 350, 0, 5000));
      return annotateActionResult(kind, before,
        await captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false }));
    } catch (error) {
      return { ok: false, target: "attached", page: { tab_id: tab.id },
        action: { operation: kind, status: error?.actionStatus === "not_started" ? "not_started" : "unknown" },
        error: { code: error?.actionStatus === "not_started" ? "action_not_started" : "action_outcome_unknown",
          message: error?.message || String(error), recovery: "Inspect the page before deciding whether to repeat the action." } };
    }
  }
  const injected = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: safePageInteraction,
    args: [kind, args || {}],
  });
  const results = Array.isArray(injected) ? injected.map((entry) => entry && entry.result).filter(Boolean) : [];
  const handled = results.find((result) => result && result.ok && result.handled);
  if (!handled) {
    const failed = results.find((result) => result && result.error);
    throw new Error((failed && failed.error) || `No ${kind} target found`);
  }
  await sleep(intArg(args || {}, "wait_after_ms", kind === "click" ? 500 : 350, 0, 5000));
  const result = await captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
  return annotateActionResult(kind, before, result);
}

async function captureAfterAction(options) {
  try {
    return { ...(await captureState(options)), action: { status: "completed" }, observation: { status: "completed" } };
  } catch (error) {
    return { ok: true, target: "attached", page: { tab_id: options.tabId },
      action: { status: "completed" }, observation: { status: "failed",
        message: error?.message || String(error), recovery: "Observe again; do not repeat the completed action." } };
  }
}

function actionStateFingerprint(state) {
  if (!state) return "";
  const controls = (Array.isArray(state.interactive_elements) ? state.interactive_elements : [])
    .slice(0, 100)
    .map((item) => `${item.selector || ""}|${item.text || ""}|${item.checked}|${item.disabled}`)
    .join("\n");
  return [
    state.final_url || state.url || "",
    state.title || "",
    String(state.visible_text || "").slice(0, 20_000),
    controls,
  ].join("\n---\n");
}

function annotateActionResult(kind, before, after) {
  const result = after || { ok: false, error: `No ${kind} result was returned` };
  const observed = result.observation?.status !== "failed";
  const changed = observed && Boolean(before) && actionStateFingerprint(before) !== actionStateFingerprint(result);
  const expectsChange = ["click", "drag", "type_text", "select_option", "set_checked", "press_key", "scroll"].includes(kind);
  return {
    ...result,
    action: { operation: kind, status: result.action?.status || "completed" },
    action_changed_page: changed,
    action_verified: !expectsChange || changed,
    action_warning: expectsChange && !changed
      ? "The action was dispatched, but URL, title, visible text, and interactive state did not change. Inspect the page before retrying; do not repeat the same guessed target."
      : "",
    before_url: (before && (before.final_url || before.url)) || "",
    before_title: (before && before.title) || "",
  };
}

async function assertCondition(args) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  const state = await evaluateCondition(tab, args || {});
  const capture = await captureState({ tabId: tab.id, screenshot: args.screenshot !== false });
  const expectedExists = boolArg(args, "exists", true);
  const passed = Boolean(state.found) === expectedExists;
  const result = {
    ...capture,
    condition: state,
    message: `condition ${passed ? "matched" : "failed"}: ${conditionDescription(state, expectedExists)}`,
  };
  if (passed) return result;
  return {
    ...result,
    ok: false,
    error: `Browser assertion failed: expected ${state.kind} \`${state.expected}\` ${expectedExists ? "to exist" : "to be absent"}`,
  };
}

async function waitForCondition(args) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  const timeoutMs = intArg(args, "timeout_ms", 5000, 0, 60_000);
  const intervalMs = intArg(args, "interval_ms", 250, 100, 5000);
  const expectedExists = boolArg(args, "exists", true);
  const started = Date.now();
  const deadline = started + timeoutMs;
  let attempts = 0;
  let state = null;
  while (true) {
    attempts += 1;
    state = await evaluateCondition(tab, args || {});
    state.attempts = attempts;
    state.elapsed_ms = Date.now() - started;
    if (Boolean(state.found) === expectedExists) {
      const capture = await captureState({ tabId: tab.id, screenshot: args.screenshot !== false });
      return {
        ...capture,
        condition: state,
        message: `waited for ${conditionDescription(state, expectedExists)}`,
      };
    }
    const now = Date.now();
    if (now >= deadline) break;
    await sleep(Math.min(intervalMs, Math.max(0, deadline - now)));
  }
  const capture = await captureState({ tabId: tab.id, screenshot: args.screenshot !== false });
  return {
    ...capture,
    ok: false,
    condition: state,
    error: `Browser wait timed out after ${timeoutMs}ms: expected ${state.kind} \`${state.expected}\` ${expectedExists ? "to exist" : "to be absent"}`,
  };
}

function intArg(args, key, defaultValue, low, high) {
  const raw = args && args[key] !== undefined && args[key] !== null && args[key] !== "" ? args[key] : defaultValue;
  const value = Number.parseInt(String(raw), 10);
  if (!Number.isFinite(value)) throw new Error(`${key} must be an integer`);
  if (value < low || value > high) throw new Error(`${key} must be between ${low} and ${high}`);
  return value;
}

function boolArg(args, key, defaultValue) {
  if (!args || args[key] === undefined || args[key] === null) return defaultValue;
  if (typeof args[key] === "boolean") return args[key];
  const value = String(args[key]).trim().toLowerCase();
  if (["true", "1", "yes", "on"].includes(value)) return true;
  if (["false", "0", "no", "off"].includes(value)) return false;
  return Boolean(args[key]);
}

function diagnosticEventMatches(kind, event, level) {
  if (level === "all") return true;
  if (kind === "console_logs") {
    const eventLevel = String((event && event.level) || "").toLowerCase();
    const eventKind = String((event && event.kind) || "").toLowerCase();
    if (level === "errors") return eventKind === "pageerror" || eventLevel === "error" || eventLevel === "assert";
    if (level === "warnings") {
      return eventKind === "pageerror" || ["error", "assert", "warning", "warn"].includes(eventLevel);
    }
    return true;
  }
  const status = Number((event && event.status) || 0);
  const eventKind = String((event && event.kind) || "");
  if (level === "errors") return eventKind === "requestfailed" || status >= 500;
  if (level === "warnings") return eventKind === "requestfailed" || status >= 400;
  return true;
}

function installDiagnosticsRecorder() {
  const MAX_EVENTS = 300;
  const truncate = (value, limit = 2000) => {
    const text = String(value === undefined || value === null ? "" : value).replace(/\0/g, "");
    return text.length > limit ? `${text.slice(0, limit)}... [truncated, ${text.length} chars]` : text;
  };
  const stringify = (value) => {
    if (value instanceof Error) return value.stack || value.message || String(value);
    if (typeof value === "string") return value;
    try {
      return JSON.stringify(value);
    } catch (_error) {
      return String(value);
    }
  };
  const store =
    window.__nolaDiagnostics ||
    {
      installed: false,
      consoleEvents: [],
      networkEvents: [],
      originalConsole: {},
      originalFetch: null,
      originalXhrOpen: null,
      originalXhrSend: null,
    };
  window.__nolaDiagnostics = store;
  const push = (key, event) => {
    const list = Array.isArray(store[key]) ? store[key] : [];
    list.push({
      timestamp: Date.now() / 1000,
      page_url: location.href,
      title: document.title || "",
      ...event,
    });
    while (list.length > MAX_EVENTS) list.shift();
    store[key] = list;
  };
  if (store.installed) return { ok: true, already_installed: true };
  store.installed = true;

  for (const level of ["debug", "log", "info", "warn", "error", "assert"]) {
    const original = console[level];
    if (typeof original !== "function") continue;
    store.originalConsole[level] = original;
    console[level] = function (...args) {
      try {
        if (level === "assert" && args.length > 0 && Boolean(args[0])) {
          return original.apply(this, args);
        }
        push("consoleEvents", {
          kind: "console",
          level,
          text: truncate((level === "assert" ? args.slice(1) : args).map(stringify).join(" "), 4000),
          url: location.href,
        });
      } catch (_error) {}
      return original.apply(this, args);
    };
  }

  window.addEventListener(
    "error",
    (event) => {
      push("consoleEvents", {
        kind: "pageerror",
        level: "error",
        text: truncate((event.error && (event.error.stack || event.error.message)) || event.message || "Error", 4000),
        source_url: event.filename || location.href,
        line: event.lineno || null,
        column: event.colno || null,
      });
    },
    true,
  );
  window.addEventListener(
    "unhandledrejection",
    (event) => {
      push("consoleEvents", {
        kind: "pageerror",
        level: "error",
        text: truncate(stringify(event.reason || "Unhandled promise rejection"), 4000),
        source_url: location.href,
      });
    },
    true,
  );

  const requestUrl = (input) => {
    try {
      if (typeof input === "string") return input;
      if (input && typeof input.url === "string") return input.url;
      if (input !== undefined && input !== null) return String(input);
    } catch (_error) {}
    return "";
  };
  const requestMethod = (input, init) => {
    const raw = (init && init.method) || (input && input.method) || "GET";
    return String(raw || "GET").toUpperCase();
  };

  if (typeof window.fetch === "function" && !store.originalFetch) {
    store.originalFetch = window.fetch;
    window.fetch = async function (input, init) {
      const started = performance.now();
      const method = requestMethod(input, init);
      const url = requestUrl(input);
      try {
        const response = await store.originalFetch.apply(this, arguments);
        push("networkEvents", {
          kind: "response",
          method,
          resource_type: "fetch",
          url: response.url || url,
          status: response.status,
          status_text: response.statusText || "",
          duration_ms: Math.round(performance.now() - started),
        });
        return response;
      } catch (error) {
        push("networkEvents", {
          kind: "requestfailed",
          method,
          resource_type: "fetch",
          url,
          failure: truncate(stringify(error), 2000),
          duration_ms: Math.round(performance.now() - started),
        });
        throw error;
      }
    };
  }

  if (window.XMLHttpRequest && !store.originalXhrOpen && !store.originalXhrSend) {
    store.originalXhrOpen = XMLHttpRequest.prototype.open;
    store.originalXhrSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url) {
      this.__nolaRequest = { method: String(method || "GET").toUpperCase(), url: String(url || ""), started: 0 };
      return store.originalXhrOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function () {
      const request = this.__nolaRequest || { method: "GET", url: "", started: 0 };
      request.started = performance.now();
      const record = (kind, failure = "") => {
        if (request.recorded) return;
        request.recorded = true;
        push("networkEvents", {
          kind,
          method: request.method,
          resource_type: "xhr",
          url: this.responseURL || request.url,
          status: this.status || null,
          status_text: this.statusText || "",
          failure: truncate(failure, 2000),
          duration_ms: Math.round(performance.now() - request.started),
        });
      };
      this.addEventListener("loadend", () => record(this.status ? "response" : "requestfailed"));
      this.addEventListener("error", () => record("requestfailed", "xhr error"));
      this.addEventListener("timeout", () => record("requestfailed", "xhr timeout"));
      this.addEventListener("abort", () => record("requestfailed", "xhr aborted"));
      return store.originalXhrSend.apply(this, arguments);
    };
  }
  return { ok: true, installed: true };
}

function collectDiagnosticsEvents(kind, args) {
  const store = window.__nolaDiagnostics;
  if (!store) {
    return { ok: true, url: location.href, title: document.title || "", events: [] };
  }
  const key = kind === "network_events" ? "networkEvents" : "consoleEvents";
  const events = Array.isArray(store[key]) ? store[key].map((event) => ({ ...event })) : [];
  if (args && args.clear === true) store[key] = [];
  return {
    ok: true,
    url: location.href,
    title: document.title || "",
    events,
  };
}

function normalizeMatchText(value) {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function matchesText(value, expected, mode) {
  const haystack = normalizeMatchText(value).toLocaleLowerCase();
  const needle = normalizeMatchText(expected).toLocaleLowerCase();
  if (mode === "exact") return haystack === needle;
  return haystack.includes(needle);
}

function conditionTarget(args) {
  const fields = ["text", "selector", "url_contains", "title_contains"];
  const values = Object.fromEntries(fields.map((field) => [field, String((args && args[field]) || "").trim()]));
  const present = fields.filter((field) => values[field]);
  if (present.length !== 1) {
    throw new Error("provide exactly one condition: text, selector, url_contains, or title_contains");
  }
  const kind = present[0];
  const expected = values[kind];
  if (expected.length > 1000 || expected.includes("\0")) throw new Error(`${kind} is invalid`);
  if (kind !== "text" && /[\r\n]/.test(expected)) throw new Error(`${kind} cannot contain newlines`);
  const matchMode = String((args && args.match_mode) || "contains").trim();
  if (!["contains", "exact"].includes(matchMode)) throw new Error("match_mode must be contains/exact");
  return { kind, expected, matchMode };
}

async function evaluateCondition(tab, args) {
  const target = conditionTarget(args);
  const latestTab = await chrome.tabs.get(tab.id).catch(() => tab);
  if (target.kind === "url_contains") {
    const value = latestTab.url || "";
    const found = matchesText(value, target.expected, target.matchMode);
    return { kind: target.kind, expected: target.expected, found, match_count: found ? 1 : 0, detail: value };
  }
  if (target.kind === "title_contains") {
    const value = latestTab.title || "";
    const found = matchesText(value, target.expected, target.matchMode);
    return { kind: target.kind, expected: target.expected, found, match_count: found ? 1 : 0, detail: value };
  }
  if (target.kind === "selector") {
    const injected = await chrome.scripting.executeScript({
      target: { tabId: tab.id, allFrames: true },
      func: selectorConditionInPage,
      args: [target.expected],
    });
    const results = Array.isArray(injected) ? injected.map((entry) => entry && entry.result).filter(Boolean) : [];
    const selectorError = results.find((item) => item && item.error);
    if (selectorError) throw new Error(selectorError.error);
    const visible = results.reduce((sum, item) => sum + Number(item.visible || 0), 0);
    const total = results.reduce((sum, item) => sum + Number(item.total || 0), 0);
    return {
      kind: target.kind,
      expected: target.expected,
      found: visible > 0,
      match_count: visible,
      total_count: total,
      detail: `${visible} visible match(es), ${total} total`,
    };
  }
  const capture = await captureState({ tabId: tab.id, screenshot: false });
  const visibleText = capture.visible_text || "";
  const found = matchesText(visibleText, target.expected, target.matchMode);
  return {
    kind: target.kind,
    expected: target.expected,
    found,
    match_count: found ? 1 : 0,
    detail: target.expected,
  };
}

function conditionDescription(state, expectedExists) {
  return `${state.kind}: ${state.expected} ${expectedExists ? "exists" : "is absent"}`;
}

function selectorConditionInPage(selector) {
  const isVisible = (el) => {
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  let nodes = [];
  try {
    nodes = Array.from(document.querySelectorAll(selector));
  } catch (error) {
    return { visible: 0, total: 0, error: `invalid selector: ${error.message}` };
  }
  return { visible: nodes.filter(isVisible).length, total: nodes.length };
}

async function trustedInteract(tab, kind, args) {
  if (kind !== "type_text" && (!chrome.debugger || typeof chrome.debugger.attach !== "function")) {
    throw new Error("chrome.debugger is unavailable");
  }
  if (kind === "hover") {
    const point = await resolveTargetPoint(tab.id, args);
    if (point.frameId && point.frameId !== 0) {
      throw new Error("hover target is inside an iframe; using frame-local DOM fallback");
    }
    await withDebugger(tab.id, async (debuggee) => {
      await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
        type: "mouseMoved",
        x: point.x,
        y: point.y,
        button: "none",
        buttons: 0,
        modifiers: debuggerModifierMask(args),
      });
    });
    await sleep(intArg(args || {}, "wait_after_ms", 350, 0, 5000));
    return captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
  }

  if (kind === "click") {
    const button = mouseButtonName(args.button);
    const buttons = mouseButtonMask(button);
    const modifiers = debuggerModifierMask(args);
    const clicks = intArg(args || {}, "clicks", 1, 1, 3);
    const waitAfterMs = intArg(args || {}, "wait_after_ms", 500, 0, 5000);
    const canUseFormControlFastPath = button === "left" && clicks === 1 && modifiers === 0;
    const formControl = canUseFormControlFastPath ? await activateFormControl(tab.id, args) : null;
    if (formControl && formControl.ok && formControl.handled) {
      await sleep(waitAfterMs);
      return captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
    }
    const point = await resolveTargetPoint(tab.id, args);
    if (point.frameId && point.frameId !== 0) {
      throw new Error("target is inside an iframe; using frame-local DOM fallback");
    }
    await withDebugger(tab.id, async (debuggee) => {
      await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
        type: "mouseMoved",
        x: point.x,
        y: point.y,
        button: "none",
        buttons: 0,
        modifiers,
      });
      for (let clickIndex = 1; clickIndex <= clicks; clickIndex += 1) {
        await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
          type: "mousePressed",
          x: point.x,
          y: point.y,
          button,
          buttons,
          clickCount: clickIndex,
          modifiers,
        });
        await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
          type: "mouseReleased",
          x: point.x,
          y: point.y,
          button,
          buttons: 0,
          clickCount: clickIndex,
          modifiers,
        });
        if (clickIndex < clicks) await sleep(80);
      }
    });
    await sleep(waitAfterMs);
    return captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
  }

  if (kind === "drag") {
    const points = await resolveDragPoints(tab.id, args);
    if (points.sourceFrameId !== 0 || points.targetFrameId !== 0 || points.sourceFrameId !== points.targetFrameId) {
      throw new Error("drag target is inside an iframe; using frame-local DOM fallback");
    }
    const button = mouseButtonName(args.button);
    const buttons = mouseButtonMask(button);
    const modifiers = debuggerModifierMask(args);
    const steps = intArg(args, "steps", 12, 1, 100);
    await withDebugger(tab.id, async (debuggee) => {
      await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
        type: "mouseMoved",
        x: points.startX,
        y: points.startY,
        button: "none",
        modifiers,
      });
      await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
        type: "mousePressed",
        x: points.startX,
        y: points.startY,
        button,
        buttons,
        clickCount: 1,
        modifiers,
      });
      for (let step = 1; step <= steps; step += 1) {
        const ratio = step / steps;
        await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
          type: "mouseMoved",
          x: points.startX + (points.endX - points.startX) * ratio,
          y: points.startY + (points.endY - points.startY) * ratio,
          button,
          buttons,
          modifiers,
        });
      }
      await sendDebuggerCommand(debuggee, "Input.dispatchMouseEvent", {
        type: "mouseReleased",
        x: points.endX,
        y: points.endY,
        button,
        buttons: 0,
        clickCount: 1,
        modifiers,
      });
    });
    await sleep(350);
    return captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
  }

  if (kind === "type_text") {
    // Focusing one document and then sending tab-wide CDP text can type into a
    // replacement document. Keep focus and text mutation in the same binding.
    await focusEditableTarget(tab.id, args);
    await sleep(350);
    return captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
  }

  if (kind === "press_key") {
    await withDebugger(tab.id, async (debuggee) => {
      await dispatchDebuggerKey(debuggee, String(args.key || ""));
    });
    await sleep(350);
    return captureAfterAction({ tabId: tab.id, screenshot: args.screenshot !== false });
  }

  throw new Error(`Unsupported trusted interaction: ${kind}`);
}

async function activateFormControl(tabId, args) {
  return executeBoundPageInteraction(tabId, "form_control", args || {});
}

async function resolveTargetPoint(tabId, args) {
  const injected = await chrome.scripting.executeScript({
    target: { tabId, allFrames: true },
    func: findTargetPoint,
    args: [args || {}],
  });
  const resultEntry = (injected || []).find((entry) => entry && entry.result && entry.result.ok) || (injected && injected[0]);
  const result = resultEntry && resultEntry.result;
  if (!result || !result.ok) {
    const candidates = result && Array.isArray(result.candidates) && result.candidates.length
      ? ` Candidates: ${JSON.stringify(result.candidates)}`
      : "";
    throw new Error(`${result && result.error ? result.error : "No clickable target found"}${candidates}`);
  }
  return {
    x: result.x,
    y: result.y,
    frameId: resultEntry.frameId || 0,
    matchedTarget: result.matchedTarget || null,
  };
}

async function resolveDragPoints(tabId, args) {
  const injected = await chrome.scripting.executeScript({
    target: { tabId, allFrames: true },
    func: findDragPoints,
    args: [args || {}],
  });
  const resultEntry = (injected || []).find((entry) => entry && entry.result && entry.result.ok) || (injected && injected[0]);
  const result = resultEntry && resultEntry.result;
  if (!result || !result.ok) throw new Error(result && result.error ? result.error : "No drag target found");
  return {
    startX: result.startX,
    startY: result.startY,
    endX: result.endX,
    endY: result.endY,
    sourceFrameId: resultEntry.frameId || 0,
    targetFrameId: resultEntry.frameId || 0,
  };
}

async function focusEditableTarget(tabId, args) {
  return executeBoundPageInteraction(tabId, "type_text", args || {});
}

async function executeBoundPageInteraction(tabId, kind, args) {
  const failure = (message, actionStatus = "not_started") => Object.assign(new Error(message), { actionStatus });
  const bindingID = crypto.randomUUID();
  let probes;
  try {
    probes = await chrome.scripting.executeScript({ target: { tabId, allFrames: true },
      func: boundPageInteraction, args: ["probe", kind, args, bindingID] });
  } catch (error) { throw failure(error?.message || String(error)); }
  if (!Array.isArray(probes) || !probes.length || probes.some(entry => !entry?.result?.ok)) {
    throw failure((Array.isArray(probes) && probes.find(entry => entry?.result?.error)?.result.error) || "Target probe could not be completed");
  }
  const matches = probes.filter(entry => entry.result.matched_count > 0);
  if (matches.reduce((count, entry) => count + entry.result.matched_count, 0) !== 1) {
    throw failure(matches.length ? "Ambiguous target across documents; provide one exact target" : "No matching target found");
  }
  const selected = matches[0];
  if (typeof selected.documentId !== "string" || !selected.documentId || !Number.isInteger(selected.frameId) || selected.frameId < 0) {
    throw failure("Chrome did not provide an exact target document identity");
  }
  if (kind === "form_control" && !selected.result.handled) {
    return { ok: true, handled: false, action_status: "not_started", frameId: selected.frameId, documentId: selected.documentId };
  }
  const execution = { tabId, frameId: selected.frameId, documentId: selected.documentId, started: false };
  boundInteractionExecutions.set(bindingID, execution);
  try {
    const injected = await chrome.scripting.executeScript({
      target: { tabId, documentIds: [selected.documentId] },
      func: boundPageInteraction, args: ["execute", kind, args, bindingID],
    });
    const entry = Array.isArray(injected) && injected.length === 1 ? injected[0] : null;
    if (!entry || entry.documentId !== selected.documentId || entry.frameId !== selected.frameId) {
      throw failure("The bound document did not return an execution receipt", execution.started ? "unknown" : "not_started");
    }
    if (!entry.result?.ok || !entry.result.handled || !execution.started) {
      throw failure(entry.result?.error || "The bound target was not executed",
        entry.result?.action_status === "not_started" ? "not_started" : execution.started ? "unknown" : "not_started");
    }
    return { ...entry.result, frameId: selected.frameId, documentId: selected.documentId };
  } catch (error) {
    if (error?.actionStatus) throw error;
    throw failure(error?.message || String(error), execution.started ? "unknown" : "not_started");
  } finally { boundInteractionExecutions.delete(bindingID); }
}

async function withDebugger(tabId, callback) {
  const debuggee = { tabId };
  let attached = false;
  try {
    await chrome.debugger.attach(debuggee, "1.3");
    attached = true;
  } catch (error) {
    const message = String(error && error.message ? error.message : error);
    if (!message.includes("Another debugger is already attached")) throw error;
  }
  try {
    return await callback(debuggee);
  } finally {
    if (attached) {
      try {
        await chrome.debugger.detach(debuggee);
      } catch (error) {
        console.warn("Nola debugger detach failed", error);
      }
    }
  }
}

function sendDebuggerCommand(debuggee, method, params) {
  return chrome.debugger.sendCommand(debuggee, method, params || {});
}

async function dispatchDebuggerKey(debuggee, key, extra = {}) {
  const normalized = key.split("+").pop();
  const definition = keyDefinition(normalized);
  const modifiers = Number(extra.modifiers || modifierMask(key));
  await sendDebuggerCommand(debuggee, "Input.dispatchKeyEvent", {
    type: definition.text ? "keyDown" : "rawKeyDown",
    key: definition.key,
    code: definition.code,
    text: definition.text,
    unmodifiedText: definition.text,
    windowsVirtualKeyCode: definition.keyCode,
    nativeVirtualKeyCode: definition.keyCode,
    modifiers,
  });
  await sendDebuggerCommand(debuggee, "Input.dispatchKeyEvent", {
    type: "keyUp",
    key: definition.key,
    code: definition.code,
    windowsVirtualKeyCode: definition.keyCode,
    nativeVirtualKeyCode: definition.keyCode,
    modifiers,
  });
}

async function dispatchDebuggerText(debuggee, text) {
  for (const char of String(text || "")) {
    const definition = keyDefinition(char);
    await sendDebuggerCommand(debuggee, "Input.dispatchKeyEvent", {
      type: "rawKeyDown",
      key: definition.key,
      code: definition.code,
      windowsVirtualKeyCode: definition.keyCode,
      nativeVirtualKeyCode: definition.keyCode,
    });
    await sendDebuggerCommand(debuggee, "Input.dispatchKeyEvent", {
      type: "char",
      key: definition.key,
      code: definition.code,
      text: char,
      unmodifiedText: char,
      windowsVirtualKeyCode: definition.keyCode,
      nativeVirtualKeyCode: definition.keyCode,
    });
    await sendDebuggerCommand(debuggee, "Input.dispatchKeyEvent", {
      type: "keyUp",
      key: definition.key,
      code: definition.code,
      windowsVirtualKeyCode: definition.keyCode,
      nativeVirtualKeyCode: definition.keyCode,
    });
  }
}

function modifierMask(key) {
  let mask = 0;
  if (key.includes("Alt+")) mask |= 1;
  if (key.includes("Control+")) mask |= 2;
  if (key.includes("Meta+")) mask |= 4;
  if (key.includes("Shift+")) mask |= 8;
  return mask;
}

function debuggerModifierMask(args) {
  const aliases = { Command: "Meta", Cmd: "Meta", Option: "Alt" };
  const raw = Array.isArray(args && args.modifiers) ? args.modifiers : [];
  let mask = 0;
  for (const item of raw) {
    const modifier = aliases[String(item || "").trim()] || String(item || "").trim();
    if (modifier === "Alt") mask |= 1;
    else if (modifier === "Control") mask |= 2;
    else if (modifier === "Meta") mask |= 4;
    else if (modifier === "Shift") mask |= 8;
  }
  return mask;
}

function mouseButtonName(button) {
  const value = String(button || "left");
  if (value === "right" || value === "middle") return value;
  return "left";
}

function mouseButtonMask(button) {
  if (button === "right") return 2;
  if (button === "middle") return 4;
  return 1;
}

function keyDefinition(key) {
  const definitions = {
    Enter: { key: "Enter", code: "Enter", keyCode: 13, text: "" },
    Tab: { key: "Tab", code: "Tab", keyCode: 9, text: "\t" },
    Escape: { key: "Escape", code: "Escape", keyCode: 27, text: "" },
    Backspace: { key: "Backspace", code: "Backspace", keyCode: 8, text: "" },
    Delete: { key: "Delete", code: "Delete", keyCode: 46, text: "" },
    Space: { key: " ", code: "Space", keyCode: 32, text: " " },
    ArrowUp: { key: "ArrowUp", code: "ArrowUp", keyCode: 38, text: "" },
    ArrowDown: { key: "ArrowDown", code: "ArrowDown", keyCode: 40, text: "" },
    ArrowLeft: { key: "ArrowLeft", code: "ArrowLeft", keyCode: 37, text: "" },
    ArrowRight: { key: "ArrowRight", code: "ArrowRight", keyCode: 39, text: "" },
    Home: { key: "Home", code: "Home", keyCode: 36, text: "" },
    End: { key: "End", code: "End", keyCode: 35, text: "" },
    PageUp: { key: "PageUp", code: "PageUp", keyCode: 33, text: "" },
    PageDown: { key: "PageDown", code: "PageDown", keyCode: 34, text: "" },
    ",": { key: ",", code: "Comma", keyCode: 188, text: "," },
    ".": { key: ".", code: "Period", keyCode: 190, text: "." },
    "@": { key: "@", code: "Digit2", keyCode: 50, text: "@" },
    "-": { key: "-", code: "Minus", keyCode: 189, text: "-" },
    "_": { key: "_", code: "Minus", keyCode: 189, text: "_" },
  };
  if (definitions[key]) return definitions[key];
  if (key.length === 1) {
    const upper = key.toUpperCase();
    return { key, code: `Key${upper}`, keyCode: upper.charCodeAt(0), text: key };
  }
  return { key, code: key, keyCode: 0, text: "" };
}

async function captureState(options = {}) {
  const tab = options.tabId ? await chrome.tabs.get(options.tabId) : await requireOwnedTab(options.ownerId);
  await installPageDiagnostics(tab.id);
  const injected = await chrome.scripting.executeScript({
    target: { tabId: tab.id, allFrames: true },
    func: collectPageState,
    args: [100],
  });
  // Cloudflare and other dashboards may render content in nested frames.
  // Pick the richest frame state, but merge text and controls across frames
  // so assertions can see nested console content.
  const results = Array.isArray(injected) ? injected.map((item) => item && item.result).filter(Boolean) : [];
  const isLikelyCssNoise = (text) => {
    const value = String(text || "").trim();
    if (!value) return false;
    // Injected stylesheets (e.g. Dark Reader) can dominate innerText.
    if (value.startsWith("@media screen")) return true;
    const cssHits = ["-webkit-filter", "hue-rotate", "filter: invert", "background-image:url", ":fullscreen"];
    let hitCount = 0;
    for (const marker of cssHits) {
      if (value.includes(marker)) hitCount += 1;
    }
    return hitCount >= 2;
  };
  const scoreState = (state) => {
    const text = String(state.visibleText || "");
    const interactive = Array.isArray(state.interactiveElements) ? state.interactiveElements.length : 0;
    const textScore = Math.min(text.length, 10_000);
    const interactiveScore = interactive * 100;
    const cssPenalty = isLikelyCssNoise(text) ? 50_000 : 0;
    return interactiveScore + textScore - cssPenalty;
  };
  const bestState = results
    .slice()
    .sort((a, b) => scoreState(b) - scoreState(a))[0] || {};
  const mergedVisibleTexts = [];
  const seenText = new Set();
  for (const state of [bestState, ...results]) {
    const text = String((state && state.visibleText) || "").replace(/\s+/g, " ").trim();
    if (!text || isLikelyCssNoise(text)) continue;
    const key = text.slice(0, 500);
    if (seenText.has(key)) continue;
    seenText.add(key);
    mergedVisibleTexts.push(text);
    if (mergedVisibleTexts.join("\n\n").length >= 200_000) break;
  }
  const mergedVisibleText = (mergedVisibleTexts.join("\n\n") || String(bestState.visibleText || "")).slice(0, 200_000);
  const mergedInteractive = [];
  const seen = new Set();
  for (const state of results) {
    const items = Array.isArray(state.interactiveElements) ? state.interactiveElements : [];
    for (const item of items) {
      const key = `${item.selector || ""}|${item.text || ""}|${item.label || ""}|${item.tag || ""}|${item.role || ""}`;
      if (seen.has(key)) continue;
      seen.add(key);
      mergedInteractive.push(item);
      if (mergedInteractive.length >= 200) break;
    }
    if (mergedInteractive.length >= 200) break;
  }
  const state = {
    ...bestState,
    visibleText: mergedVisibleText,
    interactiveElements: mergedInteractive.slice(0, 100),
  };
  let screenshotDataUrl = "";
  let screenshotError = null;
  if (options.screenshot) {
    try {
      // captureVisibleTab would photograph whichever tab is now foreground,
      // not the owner resolved above. CDP addresses the exact tab without focus.
      const image = await withDebugger(tab.id, debuggee =>
        sendDebuggerCommand(debuggee, "Page.captureScreenshot", { format: "png", fromSurface: true }));
      screenshotDataUrl = `data:image/png;base64,${image.data}`;
    } catch (error) {
      screenshotError = { code: "owned_page_screenshot_failed", message: error?.message || String(error) };
    }
  }
  return {
    ok: true,
    url: tab.url || "",
    final_url: tab.url || "",
    title: tab.title || state.title || "",
    visible_text: state.visibleText || "",
    page_html: state.pageHtml || "",
    interactive_elements: state.interactiveElements || [],
    screenshot_data_url: screenshotDataUrl,
    screenshot_error: screenshotError,
  };
}

async function domSnapshot(args = {}) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  const injected = await chrome.scripting.executeScript({
    target: { tabId: tab.id, allFrames: true },
    func: collectDomSnapshot,
    args: [args || {}],
  });
  const entries = Array.isArray(injected) ? injected.filter((entry) => entry && entry.result) : [];
  const snapshots = entries
    .filter((entry) => entry.result && entry.result.ok)
    .map((entry) => ({ ...entry.result, frame_id: entry.frameId || 0, frame_url: entry.result.url || "" }));
  if (!snapshots.length) {
    const failed = entries.find((entry) => entry.result && entry.result.error);
    throw new Error((failed && failed.result && failed.result.error) || "No DOM snapshot target found");
  }
  const latestTab = await chrome.tabs.get(tab.id).catch(() => tab);
  const snapshot = snapshots[0];
  return {
    ...snapshot,
    ok: true,
    url: latestTab.url || snapshot.url || "",
    final_url: latestTab.url || snapshot.url || "",
    title: latestTab.title || snapshot.title || "",
  };
}

async function accessibilitySnapshot(args = {}) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  const maxNodes = intArg(args, "max_nodes", 120, 1, 1000);
  const injected = await chrome.scripting.executeScript({
    target: { tabId: tab.id, allFrames: true },
    func: collectAccessibilitySnapshot,
    args: [{ ...(args || {}), max_nodes: maxNodes }],
  });
  const entries = Array.isArray(injected) ? injected.filter((entry) => entry && entry.result) : [];
  const snapshots = entries
    .filter((entry) => entry.result && entry.result.ok)
    .map((entry) => ({ ...entry.result, frame_id: entry.frameId || 0 }));
  if (!snapshots.length) {
    const failed = entries.find((entry) => entry.result && entry.result.error);
    throw new Error((failed && failed.result && failed.result.error) || "No accessibility snapshot is available");
  }

  const latestTab = await chrome.tabs.get(tab.id).catch(() => tab);
  const nodes = [];
  let totalNodes = 0;
  let truncated = false;
  for (const snapshot of snapshots) {
    totalNodes += Number(snapshot.total_nodes || 0);
    const items = Array.isArray(snapshot.nodes) ? snapshot.nodes : [];
    for (const item of items) {
      if (nodes.length >= maxNodes) {
        truncated = true;
        break;
      }
      nodes.push({
        ...item,
        index: nodes.length + 1,
        frame_id: snapshot.frame_id,
        frame_url: snapshot.url || "",
      });
    }
    if (nodes.length >= maxNodes) break;
    if (snapshot.truncated) truncated = true;
  }
  return {
    ok: true,
    url: latestTab.url || snapshots[0].url || "",
    final_url: latestTab.url || snapshots[0].url || "",
    title: latestTab.title || snapshots[0].title || "",
    root_selector: snapshots[0].root_selector || "",
    total_nodes: totalNodes,
    returned_nodes: nodes.length,
    truncated: truncated || totalNodes > nodes.length,
    nodes,
  };
}

async function storageState(args = {}) {
  const tab = await requireOwnedTab(args.nola_owner_id);
  const maxItems = intArg(args, "max_items", 100, 1, 500);
  const includeValues = boolArg(args, "include_values", false);
  const injected = await chrome.scripting.executeScript({
    target: { tabId: tab.id, allFrames: true },
    func: collectStorageState,
    args: [{
      include_local_storage: args.include_local_storage !== false,
      include_session_storage: args.include_session_storage !== false,
      include_cookies: args.include_cookies !== false,
      include_values: includeValues,
      key_filter: String(args.key_filter || ""),
      max_items: maxItems,
    }],
  });
  const entries = Array.isArray(injected) ? injected.filter((entry) => entry && entry.result) : [];
  const frames = entries
    .filter((entry) => entry.result && entry.result.ok)
    .map((entry) => ({ ...entry.result, frame_id: entry.frameId || 0 }));
  if (!frames.length) {
    const failed = entries.find((entry) => entry.result && entry.result.error);
    throw new Error((failed && failed.result && failed.result.error) || "No storage state is available");
  }

  const latestTab = await chrome.tabs.get(tab.id).catch(() => tab);
  let localStorageCount = 0;
  let sessionStorageCount = 0;
  let cookieCount = 0;
  for (const frame of frames) {
    localStorageCount += Number((frame.local_storage && frame.local_storage.total_count) || 0);
    sessionStorageCount += Number((frame.session_storage && frame.session_storage.total_count) || 0);
    cookieCount += Number((frame.cookies && frame.cookies.total_count) || 0);
  }
  return {
    ok: true,
    url: latestTab.url || frames[0].url || "",
    final_url: latestTab.url || frames[0].url || "",
    title: latestTab.title || frames[0].title || "",
    frame_count: frames.length,
    local_storage_count: localStorageCount,
    session_storage_count: sessionStorageCount,
    cookie_count: cookieCount,
    values_included: includeValues,
    frames,
  };
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function findTargetPoint(args) {
  const resolution = resolveInteractionTarget(args, false);
  const target = resolution.target;
  if (!target) return { ok: false, error: resolution.error || "No clickable target found", candidates: resolution.candidates || [] };
  target.scrollIntoView({ block: "center", inline: "center" });
  const firstRect = target.getBoundingClientRect();
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const rect = target.getBoundingClientRect();
  if (!rect.width || !rect.height) return { ok: false, error: "Clickable target has no visible bounds" };
  if (target.disabled || target.getAttribute("aria-disabled") === "true") {
    return { ok: false, error: "Target is disabled", candidates: [describeInteractionTarget(target)] };
  }
  const style = window.getComputedStyle(target);
  if (style.pointerEvents === "none") return { ok: false, error: "Target does not receive pointer events" };
  if (Math.abs(firstRect.left - rect.left) > 2 || Math.abs(firstRect.top - rect.top) > 2 ||
      Math.abs(firstRect.width - rect.width) > 2 || Math.abs(firstRect.height - rect.height) > 2) {
    return { ok: false, error: "Target is still moving; inspect or wait before retrying" };
  }
  const centerX = Math.max(0, Math.min(window.innerWidth - 1, rect.left + rect.width / 2));
  const centerY = Math.max(0, Math.min(window.innerHeight - 1, rect.top + rect.height / 2));
  const hit = document.elementFromPoint(centerX, centerY);
  if (hit && hit !== target && !target.contains(hit) && !hit.contains(target)) {
    return {
      ok: false,
      error: "Target is covered by another element",
      candidates: [describeInteractionTarget(target), describeInteractionTarget(hit)],
    };
  }
  return {
    ok: true,
    x: args.x !== undefined ? Number(args.x) : centerX,
    y: args.y !== undefined ? Number(args.y) : centerY,
    matchedTarget: describeInteractionTarget(target),
  };
}

function findDragPoints(args) {
  const isVisible = (el) => {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const firstVisible = (nodes) => Array.from(nodes || []).find(isVisible);
  const centerOf = (el) => {
    el.scrollIntoView({ block: "center", inline: "center" });
    const rect = el.getBoundingClientRect();
    if (!rect.width || !rect.height) return null;
    return {
      x: Math.max(0, Math.min(window.innerWidth - 1, rect.left + rect.width / 2)),
      y: Math.max(0, Math.min(window.innerHeight - 1, rect.top + rect.height / 2)),
    };
  };
  const byText = (text) => {
    const needle = String(text || "").replace(/\s+/g, " ").trim().toLowerCase();
    if (!needle) return null;
    const selectors = [
      "button",
      "a",
      "summary",
      "label",
      "[draggable='true']",
      "[role]",
      "li",
      "tr",
      "[class*='card' i]",
      "[class*='item' i]",
      "[class*='row' i]",
      "div",
      "span",
    ];
    const nodes = Array.from(document.querySelectorAll(selectors.join(","))).filter((el) => {
      const value = String(el.innerText || el.value || el.getAttribute("aria-label") || "").replace(/\s+/g, " ").trim().toLowerCase();
      return value.includes(needle);
    });
    nodes.sort((a, b) => {
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return ar.width * ar.height - br.width * br.height;
    });
    return firstVisible(nodes);
  };
  const pointFrom = (kind) => {
    const selector = String(args[`${kind}_selector`] || "").trim();
    const text = String(args[`${kind}_text`] || "").trim();
    const xKey = kind === "source" ? "start_x" : "end_x";
    const yKey = kind === "source" ? "start_y" : "end_y";
    if (selector) {
      const el = firstVisible(document.querySelectorAll(selector));
      const point = el ? centerOf(el) : null;
      if (point) return point;
      throw new Error(`No visible ${kind} selector target found`);
    }
    if (text) {
      const el = byText(text);
      const point = el ? centerOf(el) : null;
      if (point) return point;
      throw new Error(`No visible ${kind} text target found`);
    }
    if (args[xKey] !== undefined && args[yKey] !== undefined) {
      return { x: Number(args[xKey]), y: Number(args[yKey]) };
    }
    throw new Error(`Missing drag ${kind}`);
  };
  const start = pointFrom("source");
  const end = pointFrom("target");
  return { ok: true, startX: start.x, startY: start.y, endX: end.x, endY: end.y };
}

// This closure is self-contained: Chrome serializes only the injected function.
// Probe reads the DOM; only its isolated-world bookkeeping retains node refs.
async function boundPageInteraction(phase, kind, args, bindingID) {
  let actionStarted = false;
  try {
    const key = Symbol.for("nola.native.bound-interaction");
    if (!globalThis[key]) globalThis[key] = new Map();
    const bindings = globalThis[key];
    for (const [id, value] of bindings) if (value.expiresAt <= Date.now()) bindings.delete(id);
    while (bindings.size > 128) bindings.delete(bindings.keys().next().value);
    const normalize = value => String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
    const visible = node => {
      if (!node?.isConnected) return false;
      const style = window.getComputedStyle(node), rect = node.getBoundingClientRect();
      return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
    };
    const unique = nodes => [...new Set(nodes)].filter(visible);
    const editable = node => Boolean(node?.isContentEditable || node?.tagName?.toLowerCase() === "textarea" ||
      node?.tagName?.toLowerCase() === "input" && !["button", "submit", "reset", "checkbox", "radio", "file", "hidden", "image"].includes(node.type));
    const controlSelector = "input[type='checkbox'], input[type='radio'], [role='checkbox'], [role='radio'], [role='switch']";
    const matches = () => {
      if (args.selector) return unique([...document.querySelectorAll(args.selector)]);
      // Coordinates belong to the top viewport, not every frame's local origin.
      if (args.x !== undefined && args.y !== undefined) return window.top === window ?
        unique([document.elementFromPoint(Number(args.x), Number(args.y))]) : [];
      if (args.label) {
        const selector = kind === "select_option" ? "select" : kind === "type_text" ?
          "input, textarea, [contenteditable='true']" : controlSelector;
        const needle = normalize(args.label), found = [];
        for (const label of document.querySelectorAll("label")) {
          if (!normalize(label.innerText || label.textContent).includes(needle)) continue;
          const control = label.control || (label.getAttribute("for") ? document.getElementById(label.getAttribute("for")) : null);
          if (control) found.push(control);
          else found.push(...label.querySelectorAll(selector));
        }
        found.push(...[...document.querySelectorAll(selector)].filter(node =>
          normalize([node.getAttribute("aria-label"), node.getAttribute("placeholder"), node.getAttribute("name"),
            node.getAttribute("title")].filter(Boolean).join(" ")).includes(needle)));
        return unique(found);
      }
      if (kind === "form_control" && args.text) {
        const needle = normalize(args.text);
        return unique([...document.querySelectorAll("button, a, summary, label, mat-checkbox, mat-radio-button, [role='button'], [role='link'], " +
          "[role='checkbox'], [role='radio'], input[type='button'], input[type='submit']")].filter(node =>
          normalize(node.innerText || node.value || node.getAttribute("aria-label")).includes(needle)));
      }
      return kind === "type_text" && editable(document.activeElement) ? unique([document.activeElement]) : [];
    };
    const prepare = target => {
      if (target.disabled || target.getAttribute("aria-disabled") === "true") throw new Error("Target is disabled");
      if (kind === "type_text") {
        if (!editable(target) || target.readOnly) throw new Error("Target is not editable");
        return { target, control: target };
      }
      const selector = kind === "select_option" ? "select" : kind === "form_control" ?
        "input[type='checkbox'], input[type='radio']" : controlSelector;
      const controls = [...new Set(target.matches(selector) ? [target] : [...target.querySelectorAll(selector)])];
      if (kind === "form_control" && !controls.length) return { target, control: null };
      if (controls.length !== 1) throw new Error("Target must identify exactly one form control");
      const control = controls[0];
      if (control.disabled || control.getAttribute("aria-disabled") === "true") throw new Error("Form control is disabled");
      if (kind !== "select_option") return { target, control };
      const options = [...(control.options || [])];
      let selected;
      if (args.value !== undefined && args.value !== null && String(args.value).trim()) {
        selected = options.filter(option => option.value === String(args.value));
      } else if (args.option_label !== undefined && args.option_label !== null && String(args.option_label).trim()) {
        const needle = normalize(args.option_label);
        selected = options.filter(option => normalize(option.label || option.textContent) === needle);
        if (!selected.length) selected = options.filter(option => normalize(option.label || option.textContent).includes(needle));
      } else if (args.index !== undefined && args.index !== null && args.index !== "") {
        const index = Number(args.index);
        selected = Number.isInteger(index) && index >= 0 ? options.slice(index, index + 1) : [];
      } else throw new Error("Missing option target");
      if (selected.length !== 1 || selected[0].disabled || selected[0].parentElement?.disabled) {
        throw new Error("Option target is missing, ambiguous, or disabled");
      }
      return { target, control, option: selected[0] };
    };
    const nodes = matches();
    if (phase === "probe") {
      if (nodes.length !== 1) return { ok: true, matched_count: nodes.length, handled: false };
      const prepared = prepare(nodes[0]);
      if (prepared.control) bindings.set(bindingID, { ...prepared, kind, arguments: JSON.stringify(args),
        document, expiresAt: Date.now() + 30_000 });
      return { ok: true, matched_count: 1, handled: Boolean(prepared.control) };
    }
    const cached = bindings.get(bindingID);
    bindings.delete(bindingID); // A binding admits at most one execution attempt.
    const stillBound = () => {
      const current = matches();
      if (!cached || cached.kind !== kind || cached.arguments !== JSON.stringify(args) || cached.document !== document ||
          cached.expiresAt <= Date.now() || current.length !== 1 || current[0] !== cached.target ||
          !cached.target.isConnected || !cached.control.isConnected) return false;
      const prepared = prepare(current[0]);
      return prepared.control === cached.control && prepared.option === cached.option;
    };
    if (phase !== "execute" || !stillBound()) throw new Error("The probed document or target changed before execution");
    const receipt = await chrome.runtime.sendMessage({ type: "bound_interaction_started", binding_id: bindingID });
    if (!receipt?.accepted || !stillBound()) throw new Error("The exact document execution admission is no longer valid");
    const control = cached.control;
    const dispatchChange = () => {
      control.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
      control.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    };
    const nativeSet = (name, value) => {
      let prototype = Object.getPrototypeOf(control), descriptor;
      while (prototype && !(descriptor = Object.getOwnPropertyDescriptor(prototype, name))) prototype = Object.getPrototypeOf(prototype);
      if (descriptor?.set) descriptor.set.call(control, value);
      else control[name] = value;
    };
    actionStarted = true;
    control.scrollIntoView({ block: "center", inline: "center" });
    control.focus?.({ preventScroll: true });
    if (!control.isConnected) throw new Error("The bound control detached after focus");
    if (kind === "form_control") control.click();
    else if (kind === "set_checked") {
      const desired = args.checked !== false;
      if (typeof control.checked === "boolean") {
        if (control.checked !== desired) { nativeSet("checked", desired); dispatchChange(); }
      } else {
        const state = control.getAttribute("aria-checked");
        if (!["true", "false"].includes(state)) throw new Error("The bound control has no boolean checked state");
        if ((state === "true") !== desired) control.click();
      }
    } else if (kind === "select_option") {
      if (!control.multiple) for (const option of control.options) option.selected = false;
      cached.option.selected = true;
      if (!control.multiple) nativeSet("value", cached.option.value);
      dispatchChange();
    } else if (kind === "type_text") {
      const text = String(args.text || "");
      if (control.isContentEditable) {
        control.innerText = args.clear_first ? text : `${control.innerText || ""}${text}`;
      } else {
        const previous = String(control.value || "");
        const start = args.clear_first ? 0 : typeof control.selectionStart === "number" ? control.selectionStart : previous.length;
        const end = args.clear_first ? previous.length : typeof control.selectionEnd === "number" ? control.selectionEnd : start;
        // One native value mutation, never an execCommand-plus-fallback retry.
        nativeSet("value", previous.slice(0, start) + text + previous.slice(end));
        if (typeof control.selectionStart === "number") control.setSelectionRange?.(start + text.length, start + text.length);
      }
      dispatchChange();
      if (args.press_enter) {
        control.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true }));
        control.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", code: "Enter", bubbles: true }));
        control.form?.requestSubmit?.();
      }
    } else throw new Error("Unsupported bound document interaction");
    return { ok: true, handled: true, method: "dom", action_status: "completed" };
  } catch (error) {
    return { ok: false, handled: false, error: error?.message || String(error),
      action_status: actionStarted ? "unknown" : "not_started" };
  }
}

function normalizeInteractionText(value) {
  return String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
}

function describeInteractionTarget(el) {
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  return {
    tag: String(el.tagName || "").toLowerCase(),
    role: el.getAttribute && (el.getAttribute("role") || ""),
    text: normalizeInteractionText(el.innerText || el.value || el.getAttribute("aria-label") || "").slice(0, 160),
    label: el.getAttribute && (el.getAttribute("aria-label") || el.getAttribute("placeholder") || ""),
    disabled: Boolean(el.disabled) || (el.getAttribute && el.getAttribute("aria-disabled") === "true"),
    rect: { x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.width), height: Math.round(rect.height) },
  };
}

function resolveInteractionTarget(args, editableOnly) {
  const isVisible = (el) => {
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const visible = (nodes) => Array.from(nodes || []).filter(isVisible);
  const strict = (nodes, description) => {
    const matches = visible(nodes);
    if (matches.length === 1) return { target: matches[0], matchedBy: description, candidates: [] };
    if (matches.length > 1) {
      return {
        target: null,
        error: `Ambiguous ${description}: ${matches.length} visible targets matched. Inspect the page and use an exact selector.`,
        candidates: matches.slice(0, 8).map(describeInteractionTarget),
      };
    }
    return null;
  };
  if (args.selector) {
    try {
      const selected = strict(document.querySelectorAll(args.selector), `selector \`${args.selector}\``);
      if (selected) return selected;
    } catch (error) {
      return { target: null, error: `Invalid selector: ${String(error && error.message ? error.message : error)}`, candidates: [] };
    }
  }
  if (args.x !== undefined && args.y !== undefined) {
    const atPoint = document.elementFromPoint(Number(args.x), Number(args.y));
    if (atPoint) return { target: atPoint, matchedBy: "coordinates", candidates: [] };
  }
  if (editableOnly && args.label) {
    const needle = String(args.label || "").trim().toLowerCase();
    const labels = Array.from(document.querySelectorAll("label"));
    for (const labelEl of labels) {
      if (!String(labelEl.innerText || "").toLowerCase().includes(needle)) continue;
      const forId = labelEl.getAttribute("for");
      if (forId) {
        const target = document.getElementById(forId);
        if (target && isVisible(target)) return { target, matchedBy: "label", candidates: [] };
      }
      const nested = labelEl.querySelector("input, textarea, [contenteditable='true']");
      if (nested && isVisible(nested)) return { target: nested, matchedBy: "label", candidates: [] };
    }
    const fields = Array.from(document.querySelectorAll("input, textarea, [contenteditable='true']"));
    const matched = fields.find((el) => {
      const value = `${el.getAttribute("aria-label") || ""} ${el.getAttribute("placeholder") || ""}`.toLowerCase();
      return value.includes(needle) && isVisible(el);
    });
    if (matched) return { target: matched, matchedBy: "label", candidates: [] };
  }
  if (!editableOnly && args.text) {
    const needle = normalizeInteractionText(args.text);
    const nodes = document.querySelectorAll(
      "button, a, summary, label, mat-checkbox, mat-radio-button, [role='button'], [role='link'], " +
        "[role='checkbox'], [role='radio'], input[type='button'], input[type='submit']"
    );
    const values = Array.from(nodes).map((el) => ({
      el,
      value: normalizeInteractionText(el.innerText || el.value || el.getAttribute("aria-label") || ""),
    }));
    const exact = strict(values.filter((item) => item.value === needle).map((item) => item.el), `text \`${args.text}\``);
    if (exact) return exact;
    const contains = strict(values.filter((item) => item.value.includes(needle)).map((item) => item.el), `partial text \`${args.text}\``);
    if (contains) return contains;
  }
  return { target: null, error: editableOnly ? "No editable target found" : "No clickable target found", candidates: [] };
}

function findInteractionTarget(args, editableOnly) {
  return resolveInteractionTarget(args, editableOnly).target;
}

function collectPageState(limit) {
  const cssEscape = (value) => {
    if (window.CSS && CSS.escape) return CSS.escape(String(value));
    return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  };
  const attrEscape = (value) => String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  const isVisible = (el) => {
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const textOf = (el) => {
    const value = el.value || el.innerText || el.getAttribute("aria-label") ||
      el.getAttribute("placeholder") || el.getAttribute("title") || "";
    return String(value).replace(/\s+/g, " ").trim().slice(0, 120);
  };
  const selectorOf = (el) => {
    if (el.id) return `#${cssEscape(el.id)}`;
    const tag = el.tagName.toLowerCase();
    const name = el.getAttribute("name");
    if (name) return `${tag}[name="${attrEscape(name)}"]`;
    const aria = el.getAttribute("aria-label");
    if (aria) return `${tag}[aria-label="${attrEscape(aria)}"]`;
    const parts = [];
    let node = el;
    for (let depth = 0; node && node.nodeType === 1 && node !== document.body && depth < 5; depth += 1) {
      const nodeTag = node.tagName.toLowerCase();
      let index = 1;
      let sibling = node.previousElementSibling;
      while (sibling) {
        if (sibling.tagName.toLowerCase() === nodeTag) index += 1;
        sibling = sibling.previousElementSibling;
      }
      parts.unshift(`${nodeTag}:nth-of-type(${index})`);
      node = node.parentElement;
    }
    return parts.length ? `body > ${parts.join(" > ")}` : tag;
  };
  const interactiveElements = Array.from(document.querySelectorAll(
    "a, button, input, textarea, select, summary, [role='button'], [role='link'], [contenteditable='true'], [tabindex]:not([tabindex='-1'])"
  ))
    .filter(isVisible)
    .slice(0, limit)
    .map((el, index) => ({
      element_index: index + 1,
      tag: el.tagName.toLowerCase(),
      role: el.getAttribute("role") || "",
      type: el.getAttribute("type") || "",
      name: el.getAttribute("name") || "",
      label: el.getAttribute("aria-label") || el.getAttribute("placeholder") || "",
      text: textOf(el),
      selector: selectorOf(el),
      checked: typeof el.checked === "boolean" ? el.checked : undefined,
      disabled: typeof el.disabled === "boolean" ? el.disabled : undefined,
      ariaChecked: el.getAttribute("aria-checked") || "",
      ariaDisabled: el.getAttribute("aria-disabled") || "",
      rect: (() => {
        const rect = el.getBoundingClientRect();
        return {
          x: Math.round(rect.left + rect.width / 2),
          y: Math.round(rect.top + rect.height / 2),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
        };
      })(),
    }));
  const textCandidates = [
    document.body && document.body.innerText ? document.body.innerText : "",
    document.documentElement && document.documentElement.innerText ? document.documentElement.innerText : "",
    document.querySelector("[role='main']") ? document.querySelector("[role='main']").innerText || "" : "",
    document.querySelector("main") ? document.querySelector("main").innerText || "" : "",
  ];
  const visibleText = textCandidates
    .map((text) => String(text || "").replace(/\s+/g, " ").trim())
    .sort((a, b) => b.length - a.length)[0] || "";
  return {
    title: document.title || "",
    visibleText,
    pageHtml: String(
      (document.documentElement && document.documentElement.outerHTML) ||
      (document.body && document.body.outerHTML) ||
      ""
    ).slice(0, 2_000_000),
    interactiveElements,
  };
}

function collectDomSnapshot(args) {
  args = args || {};
  const cssEscape = (value) => {
    if (window.CSS && CSS.escape) return CSS.escape(String(value));
    return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  };
  const attrEscape = (value) => String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  const isVisible = (el) => {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const selectorOf = (el) => {
    if (!el || !el.tagName) return "";
    if (el.id) return `#${cssEscape(el.id)}`;
    const tag = el.tagName.toLowerCase();
    const name = el.getAttribute("name");
    if (name) return `${tag}[name="${attrEscape(name)}"]`;
    const aria = el.getAttribute("aria-label");
    if (aria) return `${tag}[aria-label="${attrEscape(aria)}"]`;
    const parts = [];
    let node = el;
    for (let depth = 0; node && node.nodeType === 1 && node !== document.body && depth < 6; depth += 1) {
      const nodeTag = node.tagName.toLowerCase();
      let index = 1;
      let sibling = node.previousElementSibling;
      while (sibling) {
        if (sibling.tagName.toLowerCase() === nodeTag) index += 1;
        sibling = sibling.previousElementSibling;
      }
      parts.unshift(`${nodeTag}:nth-of-type(${index})`);
      node = node.parentElement;
    }
    return parts.length ? `body > ${parts.join(" > ")}` : tag;
  };
  const textOf = (el) =>
    String(
      (el && "value" in el && el.type !== "password" ? el.value : "") ||
        (el && el.innerText) ||
        (el &&
          el.getAttribute &&
          (el.getAttribute("aria-label") || el.getAttribute("placeholder") || el.getAttribute("title"))) ||
        ""
    )
      .replace(/\s+/g, " ")
      .trim();
  const pickByText = (text) => {
    const needle = String(text || "").replace(/\s+/g, " ").trim().toLowerCase();
    if (!needle) return null;
    const nodes = Array.from(document.querySelectorAll("body *")).filter((el) => {
      if (!isVisible(el)) return false;
      return textOf(el).toLowerCase().includes(needle);
    });
    nodes.sort((a, b) => {
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return ar.width * ar.height - br.width * br.height;
    });
    return nodes[0] || null;
  };
  let target = null;
  if (args.selector) {
    try {
      const nodes = Array.from(document.querySelectorAll(String(args.selector)));
      target = nodes.find(isVisible) || nodes[0] || null;
    } catch (error) {
      return { ok: false, error: `invalid selector: ${error.message}` };
    }
  } else if (args.text) {
    target = pickByText(args.text);
  } else if (args.x !== undefined && args.x !== null && args.y !== undefined && args.y !== null) {
    target = document.elementFromPoint(Number(args.x), Number(args.y));
  }
  if (!target) return { ok: false, error: "No DOM target found" };
  if (target.scrollIntoView) target.scrollIntoView({ block: "center", inline: "center" });
  const rect = target.getBoundingClientRect();
  const computed = window.getComputedStyle(target);
  const styleKeys = [
    "display",
    "visibility",
    "opacity",
    "position",
    "zIndex",
    "boxSizing",
    "width",
    "height",
    "margin",
    "padding",
    "border",
    "borderRadius",
    "color",
    "backgroundColor",
    "fontFamily",
    "fontSize",
    "fontWeight",
    "lineHeight",
    "letterSpacing",
    "textAlign",
    "overflow",
    "overflowX",
    "overflowY",
    "pointerEvents",
    "cursor",
    "transform",
  ];
  const styles = {};
  if (args.include_styles !== false) {
    for (const key of styleKeys) styles[key] = computed[key] || "";
  }
  const sensitiveAttr = /token|secret|password|passwd|credential|authorization|auth|session|cookie|jwt|api[_-]?key/i;
  const attrs = {};
  for (const attr of Array.from(target.attributes || [])) {
    attrs[attr.name] = sensitiveAttr.test(attr.name) ? "[redacted]" : String(attr.value || "").slice(0, 500);
  }
  const sanitizeClone = (node) => {
    const clone = node.cloneNode(true);
    const nodes = [clone, ...Array.from(clone.querySelectorAll ? clone.querySelectorAll("*") : [])];
    for (const item of nodes) {
      for (const attr of Array.from(item.attributes || [])) {
        if (sensitiveAttr.test(attr.name)) item.setAttribute(attr.name, "[redacted]");
        if (attr.name === "value" && /input|textarea/i.test(item.tagName || "")) item.setAttribute(attr.name, "[redacted]");
      }
      if ("value" in item && /input|textarea/i.test(item.tagName || "")) {
        try {
          item.value = "";
        } catch (_error) {}
      }
    }
    return clone;
  };
  const maxHtml = Math.max(0, Math.min(Number(args.max_html_chars || 4000), 20000));
  let outerHTML = "";
  if (args.include_html !== false && maxHtml > 0) {
    outerHTML = sanitizeClone(target).outerHTML || "";
    if (outerHTML.length > maxHtml) {
      outerHTML = outerHTML.slice(0, maxHtml) + `\n...[truncated, outerHTML ${outerHTML.length} chars]`;
    }
  }
  const childSummary = args.include_children
    ? Array.from(target.children || [])
        .slice(0, 20)
        .map((child) => ({
          tag: child.tagName.toLowerCase(),
          selector: selectorOf(child),
          text: textOf(child).slice(0, 160),
          visible: isVisible(child),
        }))
    : [];
  const parent = target.parentElement
    ? {
        tag: target.parentElement.tagName.toLowerCase(),
        selector: selectorOf(target.parentElement),
        text: textOf(target.parentElement).slice(0, 160),
      }
    : null;
  return {
    ok: true,
    url: location.href,
    title: document.title || "",
    tag: target.tagName.toLowerCase(),
    selector: selectorOf(target),
    text: textOf(target).slice(0, 2000),
    attributes: attrs,
    rect: {
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
      center_x: rect.left + rect.width / 2,
      center_y: rect.top + rect.height / 2,
    },
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      scroll_x: window.scrollX,
      scroll_y: window.scrollY,
    },
    styles,
    parent,
    children: childSummary,
    outerHTML,
  };
}

function collectStorageState(args) {
  args = args || {};
  const keyFilter = String(args.key_filter || "").toLowerCase();
  const maxItems = Math.max(1, Math.min(Number(args.max_items || 100), 500));
  const includeValues = args.include_values === true;
  const sensitiveKey = (key) =>
    /token|secret|password|passwd|credential|authorization|auth|session|cookie|jwt|api[_-]?key/i.test(String(key || ""));
  const valueItem = (key, value, extra) => {
    const text = value === null || value === undefined ? "" : String(value);
    const item = { ...(extra || {}), value_length: text.length };
    if (includeValues && !sensitiveKey(key)) {
      item.value = text;
    } else {
      item.redacted = true;
    }
    return item;
  };
  const collectStore = (store) => {
    const keys = [];
    try {
      for (let index = 0; index < store.length; index += 1) {
        const key = String(store.key(index) || "");
        if (keyFilter && !key.toLowerCase().includes(keyFilter)) continue;
        keys.push(key);
      }
      keys.sort();
      const items = keys.slice(0, maxItems).map((key) => {
        let value = "";
        try {
          const raw = store.getItem(key);
          value = raw === null || raw === undefined ? "" : String(raw);
        } catch (error) {
          value = `[unreadable: ${error && error.message ? error.message : error}]`;
        }
        return valueItem(key, value, { key });
      });
      return { ok: true, total_count: keys.length, truncated: keys.length > items.length, items };
    } catch (error) {
      return {
        ok: false,
        error: String(error && error.message ? error.message : error),
        total_count: 0,
        truncated: false,
        items: [],
      };
    }
  };
  const collectCookies = () => {
    try {
      const raw = String(document.cookie || "");
      const pairs = raw
        ? raw
            .split(";")
            .map((part) => part.trim())
            .filter(Boolean)
            .map((part) => {
              const equalsIndex = part.indexOf("=");
              if (equalsIndex < 0) return [part, ""];
              return [part.slice(0, equalsIndex), part.slice(equalsIndex + 1)];
            })
        : [];
      const items = pairs
        .filter(([name]) => !keyFilter || String(name || "").toLowerCase().includes(keyFilter))
        .sort((a, b) => String(a[0]).localeCompare(String(b[0])))
        .slice(0, maxItems)
        .map(([name, value]) =>
          valueItem(name, value, {
            name,
            domain: location.hostname || "",
            path: "",
            httpOnly: false,
            secure: location.protocol === "https:",
          })
        );
      const totalCount = pairs.filter(
        ([name]) => !keyFilter || String(name || "").toLowerCase().includes(keyFilter)
      ).length;
      return { ok: true, total_count: totalCount, truncated: totalCount > items.length, items };
    } catch (error) {
      return {
        ok: false,
        error: String(error && error.message ? error.message : error),
        total_count: 0,
        truncated: false,
        items: [],
      };
    }
  };
  const empty = { ok: true, total_count: 0, truncated: false, items: [] };
  return {
    ok: true,
    url: location.href,
    origin: location.origin || "",
    title: document.title || "",
    local_storage: args.include_local_storage === false ? empty : collectStore(window.localStorage),
    session_storage: args.include_session_storage === false ? empty : collectStore(window.sessionStorage),
    cookies: args.include_cookies === false ? empty : collectCookies(),
  };
}

function collectAccessibilitySnapshot(args) {
  args = args || {};
  const cssEscape = (value) => {
    if (window.CSS && CSS.escape) return CSS.escape(String(value));
    return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  };
  const attrEscape = (value) => String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  const normalize = (value, maxChars) => {
    const text = String(value || "").replace(/\s+/g, " ").trim();
    return text.length > maxChars ? `${text.slice(0, maxChars)}... [truncated, ${text.length} chars]` : text;
  };
  const isVisible = (el) => {
    if (!el) return false;
    if (args.include_hidden === true) return true;
    if (el === document.body || el === document.documentElement) return true;
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  };
  const selectorOf = (el) => {
    if (!el || !el.tagName) return "";
    if (el.id) return `#${cssEscape(el.id)}`;
    const tag = el.tagName.toLowerCase();
    const testAttr = (name) => {
      const value = el.getAttribute(name);
      return value ? `${tag}[${name}="${attrEscape(value)}"]` : "";
    };
    const stable =
      testAttr("data-testid") ||
      testAttr("data-test") ||
      testAttr("name") ||
      testAttr("aria-label") ||
      testAttr("placeholder");
    if (stable) return stable;
    const parts = [];
    let node = el;
    for (let depth = 0; node && node.nodeType === 1 && node !== document.body && depth < 6; depth += 1) {
      const nodeTag = node.tagName.toLowerCase();
      let index = 1;
      let sibling = node.previousElementSibling;
      while (sibling) {
        if (sibling.tagName.toLowerCase() === nodeTag) index += 1;
        sibling = sibling.previousElementSibling;
      }
      parts.unshift(`${nodeTag}:nth-of-type(${index})`);
      node = node.parentElement;
    }
    return parts.length ? `body > ${parts.join(" > ")}` : tag;
  };
  const textOf = (el) =>
    String(
      (el && "value" in el && el.type !== "password" ? el.value : "") ||
        (el && el.innerText) ||
        (el &&
          el.getAttribute &&
          (el.getAttribute("aria-label") || el.getAttribute("placeholder") || el.getAttribute("title"))) ||
        ""
    )
      .replace(/\s+/g, " ")
      .trim();
  const pickByText = (text) => {
    const needle = normalize(text, 1000).toLowerCase();
    if (!needle) return null;
    const nodes = Array.from(document.querySelectorAll("body *")).filter((el) => {
      if (!isVisible(el)) return false;
      return textOf(el).toLowerCase().includes(needle);
    });
    nodes.sort((a, b) => {
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return ar.width * ar.height - br.width * br.height;
    });
    return nodes[0] || null;
  };
  const targetRoot = () => {
    if (args.selector) {
      try {
        const nodes = Array.from(document.querySelectorAll(String(args.selector)));
        return nodes.find(isVisible) || nodes[0] || null;
      } catch (error) {
        return { error: `invalid selector: ${error.message}` };
      }
    }
    if (args.text) return pickByText(args.text);
    if (args.x !== undefined && args.x !== null && args.y !== undefined && args.y !== null) {
      return document.elementFromPoint(Number(args.x), Number(args.y));
    }
    return document.body;
  };
  const labelledByText = (el) =>
    String(el.getAttribute("aria-labelledby") || "")
      .split(/\s+/)
      .filter(Boolean)
      .map((id) => {
        const label = document.getElementById(id);
        return label ? label.innerText || label.textContent || "" : "";
      })
      .join(" ");
  const associatedLabelText = (el) => {
    if (!el) return "";
    const id = el.getAttribute("id");
    const fromFor = id
      ? Array.from(document.querySelectorAll(`label[for="${attrEscape(id)}"]`))
          .map((label) => label.innerText || label.textContent || "")
          .join(" ")
      : "";
    const parentLabel = el.closest ? el.closest("label") : null;
    const fromParent = parentLabel ? parentLabel.innerText || parentLabel.textContent || "" : "";
    return fromFor || fromParent || "";
  };
  const roleOf = (el) => {
    const explicit = el.getAttribute("role");
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    const type = String(el.getAttribute("type") || "").toLowerCase();
    if (tag === "a" && el.getAttribute("href")) return "link";
    if (tag === "button" || ["button", "submit", "reset"].includes(type)) return "button";
    if (tag === "textarea") return "textbox";
    if (tag === "select") return el.multiple ? "listbox" : "combobox";
    if (tag === "option") return "option";
    if (tag === "input") {
      if (type === "checkbox") return "checkbox";
      if (type === "radio") return "radio";
      if (type === "range") return "slider";
      if (type === "number") return "spinbutton";
      if (type === "search") return "searchbox";
      if (type === "hidden") return "hidden";
      return "textbox";
    }
    if (/^h[1-6]$/.test(tag)) return "heading";
    if (tag === "img") return "img";
    if (tag === "ul" || tag === "ol") return "list";
    if (tag === "li") return "listitem";
    if (tag === "table") return "table";
    if (tag === "tr") return "row";
    if (tag === "th") return "columnheader";
    if (tag === "td") return "cell";
    if (tag === "nav") return "navigation";
    if (tag === "main") return "main";
    if (tag === "form") return "form";
    if (tag === "dialog") return "dialog";
    if (tag === "summary") return "button";
    if (tag === "details") return "group";
    if (tag === "p") return "paragraph";
    if (tag === "body") return "document";
    return "generic";
  };
  const sensitiveNode = (el) => {
    const tag = el.tagName.toLowerCase();
    const type = String(el.getAttribute("type") || "").toLowerCase();
    if (tag === "input" && type === "password") return true;
    const combined = [
      el.getAttribute("name"),
      el.getAttribute("id"),
      el.getAttribute("aria-label"),
      el.getAttribute("placeholder"),
      el.getAttribute("autocomplete"),
    ]
      .filter(Boolean)
      .join(" ");
    return /token|secret|password|passwd|credential|authorization|auth|session|cookie|jwt|api[_-]?key/i.test(combined);
  };
  const nameOf = (el, role) => {
    const maxChars = Math.max(20, Math.min(Number(args.max_name_chars || 160), 1000));
    const aria = el.getAttribute("aria-label") || labelledByText(el);
    if (aria) return normalize(aria, maxChars);
    const tag = el.tagName.toLowerCase();
    if (tag === "img") return normalize(el.getAttribute("alt") || el.getAttribute("title") || "", maxChars);
    const label = associatedLabelText(el);
    if (label) return normalize(label, maxChars);
    if (tag === "input" || tag === "textarea") {
      return normalize(el.getAttribute("placeholder") || el.getAttribute("title") || "", maxChars);
    }
    if (tag === "option") return normalize(el.textContent || "", maxChars);
    if (
      /^h[1-6]$/.test(tag) ||
      ["button", "link", "checkbox", "radio", "switch", "tab", "menuitem", "option", "listitem", "paragraph"].includes(role)
    ) {
      return normalize(el.innerText || el.textContent || el.getAttribute("title") || "", maxChars);
    }
    return normalize(el.getAttribute("title") || "", maxChars);
  };
  const stateOf = (el) => {
    const state = {};
    const tag = el.tagName.toLowerCase();
    const type = String(el.getAttribute("type") || "").toLowerCase();
    if (el.disabled || el.getAttribute("aria-disabled") === "true") state.disabled = true;
    if (el.getAttribute("aria-expanded") !== null) state.expanded = el.getAttribute("aria-expanded") === "true";
    if (el.getAttribute("aria-selected") !== null) state.selected = el.getAttribute("aria-selected") === "true";
    if (el.getAttribute("aria-pressed") !== null) state.pressed = el.getAttribute("aria-pressed") === "true";
    if (el.getAttribute("aria-checked") !== null) state.checked = el.getAttribute("aria-checked") === "true";
    if (tag === "input" && (type === "checkbox" || type === "radio")) state.checked = Boolean(el.checked);
    if (tag === "option") state.selected = Boolean(el.selected);
    if (tag === "details") state.expanded = Boolean(el.open);
    if (document.activeElement === el) state.focused = true;
    if (el.required || el.getAttribute("aria-required") === "true") state.required = true;
    if (el.getAttribute("aria-invalid") && el.getAttribute("aria-invalid") !== "false") state.invalid = true;
    if (/^h[1-6]$/.test(tag)) state.level = Number(tag.slice(1));
    if (args.include_values === true && !sensitiveNode(el) && (tag === "input" || tag === "textarea" || tag === "select")) {
      state.value = normalize(el.value || "", Math.max(20, Math.min(Number(args.max_name_chars || 160), 1000)));
    } else if ((tag === "input" || tag === "textarea") && el.value) {
      state.value_length = String(el.value || "").length;
    }
    return state;
  };
  const isMeaningful = (el, role, name, root) => {
    if (el === root) return true;
    if (role !== "generic" && role !== "hidden") return true;
    if (name) return true;
    return el.hasAttribute("aria-label") || el.hasAttribute("aria-labelledby");
  };

  const root = targetRoot();
  if (!root) return { ok: false, error: "No accessibility snapshot target found" };
  if (root.error) return { ok: false, error: root.error };
  if (root.scrollIntoView && root !== document.body) root.scrollIntoView({ block: "center", inline: "center" });
  const maxNodes = Math.max(1, Math.min(Number(args.max_nodes || 120), 1000));
  const includeBounds = args.include_bounds !== false;
  const includeSelectors = args.include_selectors !== false;
  const nodes = [];
  let seen = 0;
  let truncated = false;
  const walk = (el, depth) => {
    if (!el || el.nodeType !== 1 || truncated) return;
    if (!isVisible(el)) return;
    const role = roleOf(el);
    const name = nameOf(el, role);
    const record = isMeaningful(el, role, name, root);
    let nextDepth = depth;
    if (record) {
      seen += 1;
      if (nodes.length >= maxNodes) {
        truncated = true;
        return;
      }
      const rect = el.getBoundingClientRect();
      const item = {
        index: seen,
        depth,
        tag: el.tagName.toLowerCase(),
        role,
        name,
        state: stateOf(el),
      };
      if (includeSelectors) item.selector = selectorOf(el);
      if (includeBounds) {
        item.rect = {
          x: Math.round(rect.left),
          y: Math.round(rect.top),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
          center_x: Math.round(rect.left + rect.width / 2),
          center_y: Math.round(rect.top + rect.height / 2),
        };
      }
      nodes.push(item);
      nextDepth = depth + 1;
    }
    for (const child of Array.from(el.children || [])) walk(child, nextDepth);
  };
  walk(root, 0);
  return {
    ok: true,
    url: location.href,
    title: document.title || "",
    root_selector: selectorOf(root),
    total_nodes: seen,
    returned_nodes: nodes.length,
    truncated,
    nodes,
  };
}

function safePageInteraction(kind, args) {
  try {
    pageInteraction(kind, args || {});
    return { ok: true, handled: true };
  } catch (error) {
    return {
      ok: false,
      handled: false,
      error: String(error && error.message ? error.message : error),
    };
  }
  // chrome.scripting serializes this function alone, not its outer module.
  // Keep the implementation inside the injected closure.
  function pageInteraction(kind, args) {
  const findVisible = (nodes) => Array.from(nodes || []).find((el) => {
    const style = window.getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && rect.width > 0 && rect.height > 0;
  });
  const intArg = (source, key, defaultValue, low, high) => {
    const raw = source && source[key] !== undefined && source[key] !== null && source[key] !== "" ? source[key] : defaultValue;
    const value = Number.parseInt(String(raw), 10);
    if (!Number.isFinite(value)) throw new Error(`${key} must be an integer`);
    if (value < low || value > high) throw new Error(`${key} must be between ${low} and ${high}`);
    return value;
  };
  const mouseButtonName = (button) => {
    const value = String(button || "left");
    if (value === "right" || value === "middle") return value;
    return "left";
  };
  const mouseButtonNumber = (button) => {
    if (button === "right") return 2;
    if (button === "middle") return 1;
    return 0;
  };
  const mouseButtonMask = (button) => {
    if (button === "right") return 2;
    if (button === "middle") return 4;
    return 1;
  };
  const modifierState = () => {
    const aliases = { Command: "Meta", Cmd: "Meta", Option: "Alt" };
    const raw = Array.isArray(args.modifiers) ? args.modifiers : [];
    const values = new Set(raw.map((item) => aliases[String(item || "").trim()] || String(item || "").trim()));
    return {
      altKey: values.has("Alt"),
      ctrlKey: values.has("Control"),
      metaKey: values.has("Meta"),
      shiftKey: values.has("Shift"),
    };
  };
  const bySelector = (selector) => {
    if (!selector) return null;
    const nodes = document.querySelectorAll(selector);
    return findVisible(nodes);
  };
  const byText = (text) => {
    const needle = String(text || "").trim().toLowerCase();
    if (!needle) return null;
    const nodes = document.querySelectorAll(
      "button, a, summary, label, mat-checkbox, mat-radio-button, [role='button'], [role='link'], " +
        "[role='checkbox'], [role='radio'], input[type='button'], input[type='submit']"
    );
    return findVisible(Array.from(nodes).filter((el) => {
      const value = (el.innerText || el.value || el.getAttribute("aria-label") || "").toLowerCase();
      return value.includes(needle);
    }));
  };
  const byDragText = (text) => {
    const needle = String(text || "").replace(/\s+/g, " ").trim().toLowerCase();
    if (!needle) return null;
    const selectors = [
      "button",
      "a",
      "summary",
      "label",
      "[draggable='true']",
      "[role]",
      "li",
      "tr",
      "[class*='card' i]",
      "[class*='item' i]",
      "[class*='row' i]",
      "div",
      "span",
    ];
    const nodes = Array.from(document.querySelectorAll(selectors.join(","))).filter((el) => {
      const value = String(el.innerText || el.value || el.getAttribute("aria-label") || "")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      return value.includes(needle);
    });
    nodes.sort((a, b) => {
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return ar.width * ar.height - br.width * br.height;
    });
    return findVisible(nodes);
  };
  const byLabel = (label) => {
    const needle = String(label || "").trim().toLowerCase();
    if (!needle) return null;
    const labels = Array.from(document.querySelectorAll("label"));
    for (const labelEl of labels) {
      if (!String(labelEl.innerText || "").toLowerCase().includes(needle)) continue;
      const forId = labelEl.getAttribute("for");
      if (forId) {
        const target = document.getElementById(forId);
        if (target) return target;
      }
      const nested = labelEl.querySelector("input, textarea, [contenteditable='true']");
      if (nested) return nested;
    }
    const fields = document.querySelectorAll("input, textarea, [contenteditable='true']");
    return findVisible(Array.from(fields).filter((el) => {
      const value = `${el.getAttribute("aria-label") || ""} ${el.getAttribute("placeholder") || ""}`.toLowerCase();
      return value.includes(needle);
    }));
  };
  const bySelectLabel = (label) => {
    const needle = String(label || "").replace(/\s+/g, " ").trim().toLowerCase();
    if (!needle) return null;
    const labels = Array.from(document.querySelectorAll("label"));
    for (const labelEl of labels) {
      const labelText = String(labelEl.innerText || labelEl.textContent || "").replace(/\s+/g, " ").trim().toLowerCase();
      if (!labelText.includes(needle)) continue;
      const forId = labelEl.getAttribute("for");
      if (forId) {
        const target = document.getElementById(forId);
        if (target && target.tagName && target.tagName.toLowerCase() === "select") return target;
      }
      const nested = labelEl.querySelector("select");
      if (nested) return nested;
    }
    const selects = Array.from(document.querySelectorAll("select"));
    return findVisible(
      selects.filter((el) => {
        const value = String(
          el.getAttribute("aria-label") ||
            el.getAttribute("name") ||
            el.getAttribute("title") ||
            ""
        )
          .replace(/\s+/g, " ")
          .trim()
          .toLowerCase();
        return value.includes(needle);
      })
    );
  };
  const byCheckLabel = (label) => {
    const needle = String(label || "").replace(/\s+/g, " ").trim().toLowerCase();
    if (!needle) return null;
    const labels = Array.from(document.querySelectorAll("label"));
    for (const labelEl of labels) {
      const labelText = String(labelEl.innerText || labelEl.textContent || "").replace(/\s+/g, " ").trim().toLowerCase();
      if (!labelText.includes(needle)) continue;
      const forId = labelEl.getAttribute("for");
      if (forId) {
        const target = document.getElementById(forId);
        if (target) return target;
      }
      const nested = labelEl.querySelector(
        "input[type='checkbox'], input[type='radio'], [role='checkbox'], [role='radio'], [role='switch']"
      );
      if (nested) return nested;
      return labelEl;
    }
    const controls = Array.from(
      document.querySelectorAll(
        "input[type='checkbox'], input[type='radio'], [role='checkbox'], [role='radio'], [role='switch'], mat-checkbox, mat-radio-button"
      )
    );
    return findVisible(
      controls.filter((el) => {
        const value = String(
          el.getAttribute("aria-label") ||
            el.getAttribute("name") ||
            el.getAttribute("title") ||
            el.innerText ||
            el.textContent ||
            ""
        )
          .replace(/\s+/g, " ")
          .trim()
          .toLowerCase();
        return value.includes(needle);
      })
    );
  };
  const byCoords = () => {
    if (args.x === undefined || args.y === undefined) return null;
    return document.elementFromPoint(Number(args.x), Number(args.y));
  };
  const keyboardEvent = (type, key) => {
    const normalized = key.split("+").pop();
    const keyCodes = {
      Enter: 13,
      Tab: 9,
      Escape: 27,
      Backspace: 8,
      Delete: 46,
      Space: 32,
      ArrowUp: 38,
      ArrowDown: 40,
      ArrowLeft: 37,
      ArrowRight: 39,
      Home: 36,
      End: 35,
      PageUp: 33,
      PageDown: 34,
    };
    const event = new KeyboardEvent(type, {
      key: normalized,
      code: normalized === " " || normalized === "Space" ? "Space" : normalized,
      bubbles: true,
      cancelable: true,
      composed: true,
      ctrlKey: key.includes("Control+"),
      metaKey: key.includes("Meta+"),
      shiftKey: key.includes("Shift+"),
      altKey: key.includes("Alt+"),
    });
    const keyCode = keyCodes[normalized] || (normalized.length === 1 ? normalized.toUpperCase().charCodeAt(0) : 0);
    Object.defineProperty(event, "keyCode", { get: () => keyCode });
    Object.defineProperty(event, "which", { get: () => keyCode });
    return event;
  };
  const setNativeValue = (el, value) => {
    const prototype = Object.getPrototypeOf(el);
    const descriptor = prototype ? Object.getOwnPropertyDescriptor(prototype, "value") : null;
    if (descriptor && typeof descriptor.set === "function") descriptor.set.call(el, value);
    else el.value = value;
  };
  const dispatchInput = (el) => {
    el.dispatchEvent(new InputEvent("input", { bubbles: true, composed: true, inputType: "insertText" }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const dispatchSelectChange = (el) => {
    el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    el.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  };
  const centerOf = (el) => {
    const rect = el.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(window.innerWidth - 1, rect.left + rect.width / 2)),
      y: Math.max(0, Math.min(window.innerHeight - 1, rect.top + rect.height / 2)),
    };
  };
  const dispatchMouse = (el, type, point, optionsOverride = {}) => {
    const button = mouseButtonName(optionsOverride.button || args.button);
    const activeButton = optionsOverride.active === true || ["pointerdown", "mousedown"].includes(type);
    const options = {
      bubbles: true,
      cancelable: true,
      composed: true,
      view: window,
      clientX: point.x,
      clientY: point.y,
      screenX: window.screenX + point.x,
      screenY: window.screenY + point.y,
      button: mouseButtonNumber(button),
      buttons: activeButton ? mouseButtonMask(button) : 0,
      detail: Number(optionsOverride.detail || 0),
      ...modifierState(),
    };
    if (type.startsWith("pointer")) {
      options.pointerType = "mouse";
      options.isPrimary = true;
    }
    const EventClass = window.PointerEvent && type.startsWith("pointer") ? PointerEvent : MouseEvent;
    el.dispatchEvent(new EventClass(type, options));
  };
  const dispatchDragEvent = (el, type, point, dataTransfer) => {
    const options = {
      bubbles: true,
      cancelable: true,
      composed: true,
      view: window,
      clientX: point.x,
      clientY: point.y,
      screenX: window.screenX + point.x,
      screenY: window.screenY + point.y,
      dataTransfer,
    };
    try {
      el.dispatchEvent(new DragEvent(type, options));
    } catch (_error) {
      const event = new Event(type, { bubbles: true, cancelable: true, composed: true });
      Object.assign(event, { clientX: point.x, clientY: point.y, dataTransfer });
      el.dispatchEvent(event);
    }
  };
  const clickLikeUser = (target) => {
    target.scrollIntoView({ block: "center", inline: "center" });
    const point = args.x !== undefined && args.y !== undefined ? { x: Number(args.x), y: Number(args.y) } : centerOf(target);
    const hit = document.elementFromPoint(point.x, point.y) || target;
    const clickTarget = hit.closest("label, button, a, [role='button'], [role='checkbox'], [role='radio'], mat-checkbox, mat-radio-button") || hit;
    const button = mouseButtonName(args.button);
    const clicks = intArg(args, "clicks", 1, 1, 3);
    const modifiers = modifierState();
    const hasModifiers = modifiers.altKey || modifiers.ctrlKey || modifiers.metaKey || modifiers.shiftKey;
    for (const type of ["pointerover", "mouseover", "mousemove"]) {
      dispatchMouse(clickTarget, type, point, { button: "left" });
    }
    for (let clickIndex = 1; clickIndex <= clicks; clickIndex += 1) {
      for (const type of ["pointerdown", "mousedown"]) {
        dispatchMouse(clickTarget, type, point, { button, active: true, detail: clickIndex });
      }
      if (typeof clickTarget.focus === "function") clickTarget.focus({ preventScroll: true });
      for (const type of ["pointerup", "mouseup"]) {
        dispatchMouse(clickTarget, type, point, { button, detail: clickIndex });
      }
      if (button === "right") dispatchMouse(clickTarget, "contextmenu", point, { button, detail: clickIndex });
      else if (button === "middle") dispatchMouse(clickTarget, "auxclick", point, { button, detail: clickIndex });
      else dispatchMouse(clickTarget, "click", point, { button, detail: clickIndex });
      if (button === "left" && clickIndex === 2) {
        dispatchMouse(clickTarget, "dblclick", point, { button, detail: clickIndex });
      }
    }
    if (button === "left" && clicks === 1 && !hasModifiers && typeof clickTarget.click === "function") clickTarget.click();
  };
  const hoverLikeUser = (target) => {
    target.scrollIntoView({ block: "center", inline: "center" });
    const point = args.x !== undefined && args.y !== undefined ? { x: Number(args.x), y: Number(args.y) } : centerOf(target);
    const hit = document.elementFromPoint(point.x, point.y) || target;
    const hoverTarget = hit.closest("button, a, summary, label, [role], mat-checkbox, mat-radio-button") || hit;
    for (const type of ["pointerover", "mouseover", "mouseenter", "pointermove", "mousemove"]) {
      dispatchMouse(hoverTarget, type, point);
    }
  };
  const setNativeChecked = (el, checked) => {
    const prototype = Object.getPrototypeOf(el);
    const descriptor = prototype ? Object.getOwnPropertyDescriptor(prototype, "checked") : null;
    if (descriptor && typeof descriptor.set === "function") descriptor.set.call(el, checked);
    else el.checked = checked;
  };
  const dispatchChecked = (el) => {
    el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    el.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  };
  const checkedState = (target) => {
    const control =
      target.matches && target.matches("input[type='checkbox'], input[type='radio']")
        ? target
        : target.querySelector && target.querySelector("input[type='checkbox'], input[type='radio']");
    if (control && typeof control.checked === "boolean") return { kind: "input", checked: Boolean(control.checked), control };
    const ariaTarget =
      target.matches && target.matches("[role='checkbox'], [role='radio'], [role='switch']")
        ? target
        : target.closest && target.closest("[role='checkbox'], [role='radio'], [role='switch']");
    if (ariaTarget && ariaTarget.getAttribute("aria-checked") !== null) {
      return { kind: "aria", checked: ariaTarget.getAttribute("aria-checked") === "true", control: ariaTarget };
    }
    return { kind: "unknown", checked: false, control: target };
  };
  const setCheckedLikeUser = (target) => {
    target.scrollIntoView({ block: "center", inline: "center" });
    const desired = args.checked !== false;
    let state = checkedState(target);
    if (state.kind === "input") {
      if (state.checked !== desired) {
        setNativeChecked(state.control, desired);
        dispatchChecked(state.control);
      }
      return;
    }
    if (state.checked !== desired) {
      clickLikeUser(state.control || target);
      state = checkedState(target);
    }
    if (state.kind === "aria" && state.checked !== desired) {
      state.control.setAttribute("aria-checked", desired ? "true" : "false");
      dispatchChecked(state.control);
    }
  };
  const selectOptionLikeUser = (target) => {
    const select =
      target && target.tagName && target.tagName.toLowerCase() === "select"
        ? target
        : target && target.querySelector
          ? target.querySelector("select")
          : null;
    if (!select) throw new Error("Target is not a native select element");
    if (select.disabled || select.getAttribute("aria-disabled") === "true") throw new Error("Select element is disabled");
    select.scrollIntoView({ block: "center", inline: "center" });
    if (typeof select.focus === "function") select.focus({ preventScroll: true });

    const options = Array.from(select.options || []);
    if (!options.length) throw new Error("Select element has no options");
    let option = null;
    if (args.value !== undefined && args.value !== null && String(args.value).trim() !== "") {
      const value = String(args.value);
      option = options.find((item) => item.value === value);
      if (!option) throw new Error(`No option value matched: ${value}`);
    } else if (args.option_label !== undefined && args.option_label !== null && String(args.option_label).trim() !== "") {
      const label = String(args.option_label).replace(/\s+/g, " ").trim().toLowerCase();
      option =
        options.find((item) => String(item.label || item.textContent || "").replace(/\s+/g, " ").trim().toLowerCase() === label) ||
        options.find((item) => String(item.label || item.textContent || "").replace(/\s+/g, " ").trim().toLowerCase().includes(label));
      if (!option) throw new Error(`No option label matched: ${args.option_label}`);
    } else if (args.index !== undefined && args.index !== null && args.index !== "") {
      const index = Number(args.index);
      if (!Number.isInteger(index) || index < 0 || index >= options.length) throw new Error("option index is out of range");
      option = options[index];
    } else {
      throw new Error("Missing option target");
    }

    if (select.multiple) {
      option.selected = true;
    } else {
      for (const item of options) item.selected = false;
      option.selected = true;
      setNativeValue(select, option.value);
    }
    dispatchSelectChange(select);
  };
  const dragPoint = (kind) => {
    const selector = String(args[`${kind}_selector`] || "").trim();
    const text = String(args[`${kind}_text`] || "").trim();
    const xKey = kind === "source" ? "start_x" : "end_x";
    const yKey = kind === "source" ? "start_y" : "end_y";
    let target = null;
    if (selector) target = bySelector(selector);
    else if (text) target = byDragText(text);
    if (target) {
      target.scrollIntoView({ block: "center", inline: "center" });
      return { ...centerOf(target), target };
    }
    if (args[xKey] !== undefined && args[yKey] !== undefined) {
      const point = { x: Number(args[xKey]), y: Number(args[yKey]) };
      return { ...point, target: document.elementFromPoint(point.x, point.y) || document.body };
    }
    throw new Error(`No drag ${kind} target found`);
  };
  const dragLikeUser = () => {
    const start = dragPoint("source");
    const end = dragPoint("target");
    const steps = intArg(args, "steps", 12, 1, 100);
    const source = document.elementFromPoint(start.x, start.y) || start.target || document.body;
    const target = document.elementFromPoint(end.x, end.y) || end.target || document.body;
    const dataTransfer = typeof DataTransfer === "function" ? new DataTransfer() : null;
    for (const type of ["pointerover", "mouseover", "mousemove", "pointerdown", "mousedown"]) {
      dispatchMouse(source, type, start);
    }
    dispatchDragEvent(source, "dragstart", start, dataTransfer);
    for (let step = 1; step <= steps; step += 1) {
      const ratio = step / steps;
      const point = {
        x: start.x + (end.x - start.x) * ratio,
        y: start.y + (end.y - start.y) * ratio,
      };
      const hit = document.elementFromPoint(point.x, point.y) || target;
      dispatchMouse(hit, "pointermove", point);
      dispatchMouse(hit, "mousemove", point);
      dispatchDragEvent(hit, "dragover", point, dataTransfer);
    }
    dispatchDragEvent(target, "drop", end, dataTransfer);
    for (const type of ["pointerup", "mouseup"]) {
      dispatchMouse(target, type, end);
    }
    dispatchDragEvent(source, "dragend", end, dataTransfer);
  };

  if (kind === "hover") {
    const target = bySelector(args.selector) || byText(args.text) || byCoords();
    if (!target) throw new Error("No hover target found");
    hoverLikeUser(target);
    return;
  }

  if (kind === "click") {
    const target = bySelector(args.selector) || byText(args.text) || byCoords();
    if (!target) throw new Error("No clickable target found");
    clickLikeUser(target);
    return;
  }

  if (kind === "drag") {
    dragLikeUser();
    return;
  }

  if (kind === "type_text") {
    const target = bySelector(args.selector) || byLabel(args.label) || byCoords() || document.activeElement;
    if (!target) throw new Error("No editable target found");
    target.scrollIntoView({ block: "center", inline: "center" });
    target.focus();
    const text = String(args.text || "");
    if (target.isContentEditable) {
      if (args.clear_first) target.innerText = "";
      target.innerText += text;
      dispatchInput(target);
    } else if ("value" in target) {
      setNativeValue(target, args.clear_first ? "" : target.value || "");
      target.focus();
      if (args.clear_first && typeof target.select === "function") target.select();
      let inserted = false;
      if (typeof document.execCommand === "function") {
        inserted = document.execCommand("insertText", false, text);
      }
      if (!inserted) setNativeValue(target, `${target.value || ""}${text}`);
      dispatchInput(target);
    } else {
      throw new Error("Target is not editable");
    }
    if (args.press_enter) {
      target.dispatchEvent(keyboardEvent("keydown", "Enter"));
      target.dispatchEvent(keyboardEvent("keyup", "Enter"));
      if (target.form && typeof target.form.requestSubmit === "function") target.form.requestSubmit();
    }
    return;
  }

  if (kind === "select_option") {
    const target = bySelector(args.selector) || bySelectLabel(args.label);
    if (!target) throw new Error("No native select target found");
    selectOptionLikeUser(target);
    return;
  }

  if (kind === "set_checked") {
    const target = bySelector(args.selector) || byCheckLabel(args.label) || byCoords();
    if (!target) throw new Error("No checkbox, radio, or switch target found");
    setCheckedLikeUser(target);
    return;
  }

  if (kind === "press_key") {
    const key = String(args.key || "");
    const target = document.activeElement || document.body;
    target.dispatchEvent(keyboardEvent("keydown", key.split("+").pop()));
    target.dispatchEvent(keyboardEvent("keyup", key.split("+").pop()));
    return;
  }

  if (kind === "scroll") {
    const direction = String(args.direction || "down");
    const amount = Number(args.amount || 800);
    if (direction === "top") window.scrollTo({ top: 0, behavior: "instant" });
    else if (direction === "bottom") window.scrollTo({ top: document.documentElement.scrollHeight, behavior: "instant" });
    else if (direction === "left" || direction === "right") {
      window.scrollBy({ top: 0, left: direction === "left" ? -amount : amount, behavior: "instant" });
    } else {
      window.scrollBy({ top: direction === "up" ? -amount : amount, left: 0, behavior: "instant" });
    }
  }
  }
}
