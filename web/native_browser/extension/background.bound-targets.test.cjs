// SPDX-License-Identifier: Apache-2.0
'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { randomUUID } = require('node:crypto');

// Each frame runs Chrome's serialized injection in its own isolated world.
// Merely returning the first successful frame is deliberately insufficient.
class ElementFixture {
    constructor(tag = 'input', type = 'checkbox') {
        this.tagName = tag.toUpperCase(); this.type = type; this.id = 'target';
        this.isConnected = true; this.disabled = false; this.readOnly = false;
        this.checked = false; this.value = ''; this.isContentEditable = false;
        this.attributes = {}; this.options = []; this.multiple = false;
        this.focuses = 0; this.scrolls = 0; this.clicks = 0; this.events = [];
    }
    matches(selector) {
        return selector.split(',').some(raw => {
            const value = raw.trim();
            if (value.startsWith('#')) return value.slice(1) === this.id;
            if (value === "input[type='checkbox']") return this.tagName === 'INPUT' && this.type === 'checkbox';
            if (value === "input[type='radio']") return this.tagName === 'INPUT' && this.type === 'radio';
            if (value === "[contenteditable='true']") return this.isContentEditable;
            const role = value.match(/^\[role='([^']+)'\]$/);
            if (role) return this.getAttribute('role') === role[1];
            return value.toUpperCase() === this.tagName;
        });
    }
    querySelectorAll() { return []; }
    getAttribute(name) { return this.attributes[name] ?? null; }
    getBoundingClientRect() { return { left: 0, top: 0, width: 100, height: 30 }; }
    scrollIntoView() { this.scrolls += 1; }
    focus() { this.focuses += 1; this.ownerDocument.activeElement = this; this.onFocus?.(); }
    select() {}
    click() { this.clicks += 1; if (this.type === 'checkbox') this.checked = !this.checked; }
    dispatchEvent(event) { this.events.push(event.type); }
}

function frameFixture(frameId, nodes) {
    const document = {
        activeElement: null,
        querySelectorAll: selector => nodes.filter(node => node.matches(selector)),
        getElementById: id => nodes.find(node => node.id === id) || null,
        elementFromPoint: () => nodes[0] || null,
        execCommand: (_command, _showUI, text) => {
            document.insertions += 1; document.activeElement.value += text; return true;
        },
        insertions: 0,
    };
    nodes.forEach(node => { node.ownerDocument = document; });
    const window = { getComputedStyle: () => ({ display: 'block', visibility: 'visible' }) };
    window.top = frameId === 0 ? window : {};
    return { frameId, documentId: `document-${frameId}`, nodes, document, window };
}

function harness(frames, options = {}) {
    const listeners = [], calls = [];
    const backgroundChrome = {
        runtime: {
            onInstalled: { addListener() {} }, onStartup: { addListener() {} },
            onMessage: { addListener(listener) { listeners.push(listener); } },
        },
        action: { onClicked: { addListener() {} } },
        debugger: { async attach() { throw new Error('Bound DOM input must not attach a tab-wide debugger'); } },
        scripting: {
            async executeScript(request) {
                calls.push(request);
                const phase = request.args[0];
                if (phase === 'execute') options.beforeExecute?.(frames);
                const selected = request.target.allFrames ? frames : frames.filter(frame => request.target.documentIds?.includes(frame.documentId));
                if (!selected.length) throw new Error('The requested document no longer exists');
                const results = [];
                for (const frame of selected) {
                    frame.context.__injectionArguments = request.args;
                    const result = await vm.runInContext(`(${request.func.toString()})(...__injectionArguments)`, frame.context);
                    results.push({ frameId: frame.frameId, documentId: options.omitDocumentID ? undefined : frame.documentId, result });
                }
                if (phase === 'execute') options.afterExecute?.();
                return results;
            },
        },
    };
    const EventFixture = class { constructor(type) { this.type = type; } };
    for (const frame of frames) {
        frame.context = vm.createContext({ document: frame.document, window: frame.window,
            Event: EventFixture, KeyboardEvent: EventFixture,
            chrome: { runtime: { sendMessage: message => new Promise(resolve => {
                const sender = { tab: { id: 7 }, frameId: frame.frameId, documentId: frame.documentId };
                let responded = false;
                for (const listener of listeners) {
                    listener(message, sender, result => { responded = true; resolve(result); });
                    if (responded) return;
                }
                resolve(undefined);
            }) } },
        });
    }
    const context = vm.createContext({ chrome: backgroundChrome, crypto: { randomUUID },
        console: { log() {}, warn() {} }, setTimeout, clearTimeout });
    const source = fs.readFileSync(path.join(__dirname, 'background.js'), 'utf8');
    vm.runInContext(source, context);
    vm.runInContext(`
        requireOwnedTab = async () => ({ id: 7, windowId: 1, url: 'https://fixture.test' });
        installPageDiagnostics = async () => {};
        sleep = async () => {};
        let captures = 0;
        captureState = async () => {
            if (++captures > 1 && globalThis.failObservation) throw new Error('observation unavailable');
            return { ok: true, url: 'https://fixture.test', title: 'Fixture', visible_text: '', interactive_elements: [] };
        };
        globalThis.fixture = { activateFormControl, focusEditableTarget, executeBoundPageInteraction, interact };
    `, context);
    return { api: context.fixture, context, calls, chrome: backgroundChrome };
}

test('checkbox probe is read-only and rejects matching controls across frames', async () => {
    const one = new ElementFixture(), two = new ElementFixture();
    const h = harness([frameFixture(0, [one]), frameFixture(3, [two])]);
    await assert.rejects(h.api.activateFormControl(7, { selector: '#target' }), error => error.actionStatus === 'not_started');
    assert.equal(h.calls.length, 1);
    assert.equal(one.focuses + two.focuses + one.scrolls + two.scrolls + one.clicks + two.clicks, 0);
    assert.equal(one.checked || two.checked, false);
});

test('one iframe checkbox executes once in the probed document, not allFrames', async () => {
    const target = new ElementFixture();
    const h = harness([frameFixture(0, []), frameFixture(3, [target])]);
    const result = await h.api.activateFormControl(7, { selector: '#target' });
    assert.equal(result.documentId, 'document-3');
    assert.equal(result.frameId, 3);
    assert.equal(target.clicks, 1);
    assert.equal(target.checked, true);
    assert.deepEqual(Array.from(h.calls[1].target.documentIds), ['document-3']);
    assert.equal(h.calls[1].target.allFrames, undefined);
});

test('editable probe does not focus either of two matching frames', async () => {
    const one = new ElementFixture('input', 'text'), two = new ElementFixture('textarea', 'textarea');
    const h = harness([frameFixture(0, [one]), frameFixture(3, [two])]);
    await assert.rejects(h.api.focusEditableTarget(7, { selector: '#target', text: 'hello' }), error => error.actionStatus === 'not_started');
    assert.equal(one.focuses + two.focuses, 0);
    assert.equal(one.value + two.value, '');
});

test('iframe focus and text insertion stay in one exact document without CDP replay', async () => {
    const target = new ElementFixture('input', 'text');
    const frame = frameFixture(3, [target]);
    const h = harness([frameFixture(0, []), frame]);
    await h.api.focusEditableTarget(7, { selector: '#target', text: 'hello', clear_first: true });
    assert.equal(target.focuses, 1);
    assert.equal(frame.document.insertions, 0, 'No execCommand attempt followed by a fallback insertion');
    assert.deepEqual(target.events, ['input', 'change']);
    assert.equal(target.value, 'hello');
    assert.equal(h.calls.length, 2);
});

test('select_option chooses one document and preserves a single change dispatch', async () => {
    const select = new ElementFixture('select', 'select-one');
    select.options = [{ value: 'one', selected: false }, { value: 'two', selected: false }];
    const h = harness([frameFixture(0, []), frameFixture(3, [select])]);
    await h.api.executeBoundPageInteraction(7, 'select_option', { selector: '#target', value: 'two' });
    assert.equal(select.options[1].selected, true);
    assert.equal(select.value, 'two');
    assert.deepEqual(select.events, ['input', 'change']);
    assert.equal(select.focuses, 1);
});

test('set_checked rejects multiple visible matches inside one document before focus', async () => {
    const one = new ElementFixture(), two = new ElementFixture();
    const h = harness([frameFixture(0, [one, two])]);
    await assert.rejects(h.api.executeBoundPageInteraction(7, 'set_checked', { selector: '#target', checked: true }),
        error => error.actionStatus === 'not_started');
    assert.equal(one.focuses + two.focuses, 0);
    assert.equal(one.checked || two.checked, false);
});

test('navigation between probe and injection is not_started and never targets the replacement', async () => {
    const target = new ElementFixture();
    const h = harness([frameFixture(0, [target])], { beforeExecute(frames) { frames[0].documentId = 'replacement'; } });
    await assert.rejects(h.api.activateFormControl(7, { selector: '#target' }), error => error.actionStatus === 'not_started');
    assert.equal(target.clicks + target.focuses, 0);
    assert.equal(h.calls.length, 2);
});

test('same-document node replacement invalidates the exact node binding', async () => {
    const target = new ElementFixture(), replacement = new ElementFixture();
    const frame = frameFixture(0, [target]);
    const h = harness([frame], { beforeExecute() {
        target.isConnected = false; replacement.ownerDocument = frame.document; frame.nodes.splice(0, 1, replacement);
    } });
    await assert.rejects(h.api.activateFormControl(7, { selector: '#target' }), error => error.actionStatus === 'not_started');
    assert.equal(target.clicks + replacement.clicks + replacement.focuses, 0);
});

test('a missing execution receipt after input remains unknown without a fallback injection', async () => {
    const target = new ElementFixture();
    const h = harness([frameFixture(0, [target])], { afterExecute() { throw new Error('Execution channel disappeared'); } });
    await assert.rejects(h.api.activateFormControl(7, { selector: '#target' }), error => error.actionStatus === 'unknown');
    assert.equal(target.clicks, 1);
    assert.equal(h.calls.length, 2);
});

test('observation failure after set_checked preserves completed action and never repeats it', async () => {
    const target = new ElementFixture();
    const h = harness([frameFixture(0, [target])]);
    h.context.failObservation = true;
    const result = await h.api.interact('set_checked', { nola_owner_id: 'owner', selector: '#target', checked: true });
    assert.equal(result.action.status, 'completed');
    assert.equal(result.observation.status, 'failed');
    assert.equal(target.checked, true);
    assert.deepEqual(target.events, ['input', 'change']);
    assert.equal(h.calls.length, 2);
});

test('a consumed binding cannot be replayed into the same document', async () => {
    const target = new ElementFixture();
    const h = harness([frameFixture(0, [target])]);
    await h.api.activateFormControl(7, { selector: '#target' });
    const replay = await h.chrome.scripting.executeScript(h.calls[1]);
    assert.equal(replay[0].result.action_status, 'not_started');
    assert.equal(target.clicks, 1);
});

test('duplicate option values are ambiguous before focus or selection', async () => {
    const select = new ElementFixture('select', 'select-one');
    select.options = [{ value: 'same', selected: false }, { value: 'same', selected: false }];
    const h = harness([frameFixture(0, [select])]);
    await assert.rejects(h.api.executeBoundPageInteraction(7, 'select_option', { selector: '#target', value: 'same' }),
        error => error.actionStatus === 'not_started');
    assert.equal(select.focuses, 0);
    assert.equal(select.options.some(option => option.selected), false);
});

test('Chrome without exact document identity is rejected rather than using a frame-only fallback', async () => {
    const target = new ElementFixture();
    const h = harness([frameFixture(0, [target])], { omitDocumentID: true });
    await assert.rejects(h.api.activateFormControl(7, { selector: '#target' }), error => error.actionStatus === 'not_started');
    assert.equal(target.clicks + target.focuses, 0);
});
