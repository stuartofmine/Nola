const assert = require('assert');
const fs = require('fs');
const vm = require('vm');

const html = fs.readFileSync('apps/web/app/templates/chat.html', 'utf8');
const start = html.indexOf('const toolActivityRowIndexes = new WeakMap();');
const end = html.indexOf('\nfunction refreshToolActivitySummary', start);
assert(start >= 0 && end > start, 'tool activity row index source is available');

const context = vm.createContext({ WeakMap, Map, Array, String });
vm.runInContext(html.slice(start, end), context);

const first = { dataset: { traceCallId: 'call-1' }, isConnected: true };
const second = { dataset: { traceCallId: 'call-2' }, isConnected: true };
let rows = [first, second];
let scans = 0;
const tools = {
    querySelectorAll() {
        scans += 1;
        return rows;
    },
    contains(row) {
        return rows.includes(row);
    },
};

assert.strictEqual(context.findToolActivityRow(tools, 'call-1'), first);
assert.strictEqual(context.findToolActivityRow(tools, 'call-2'), second);
assert.strictEqual(context.findToolActivityRow(tools, 'call-1'), first);
assert.strictEqual(scans, 1, 'known call identities do not rescan a long tool list');

first.isConnected = false;
rows = [second];
assert.strictEqual(context.findToolActivityRow(tools, 'call-1'), null);
assert.strictEqual(scans, 2, 'a detached cached row is invalidated and checked once');

const permission = { dataset: { traceCallId: 'call-3' }, isConnected: true };
rows.push(permission);
context.indexToolActivityRow(tools, permission);
assert.strictEqual(context.findToolActivityRow(tools, 'call-3'), permission);
assert.strictEqual(context.findToolActivityRow(tools, 'call-3'), permission);
assert.strictEqual(scans, 2, 'a newly inserted row is indexed without rescanning the tool list');

console.log('tool activity row index tests passed');
