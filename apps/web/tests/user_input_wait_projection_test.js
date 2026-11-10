// SPDX-License-Identifier: Apache-2.0
'use strict';
const fs = require('fs');
const path = require('path');
const assert = require('assert');
global.window = global;
require('../app/static/js/agent-trace-runtime.js');
const source = JSON.parse(fs.readFileSync(path.join(__dirname, '../../android/app/src/test/resources/conversation_projection_v1.json')));
const tool = source.messages.flatMap(message => message.parts).find(part => part.kind === 'tool');
assert(tool);
tool.state = 'waiting_for_input';
let rendered;
const store = NolaAgentTrace.createTraceStore({onProjection(value) { if (value.tools.length) rendered = value; }});
store.ingestProjection(source);
assert(rendered);
assert.equal(rendered.tools[0].status, 'waiting_for_input');
assert.equal(rendered.currentTool.status, 'waiting_for_input');
console.log('User input wait Web projection: passed');
