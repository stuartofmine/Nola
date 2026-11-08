// SPDX-License-Identifier: Apache-2.0
'use strict';

global.window = global;
require('../app/static/js/agent-trace-runtime.js');

const store = global.NolaAgentTrace.createTraceStore();

function event(sequence, kind, status, callId) {
    return {
        type: 'agent_trace',
        trace: {
            schema_version: 1,
            trace_id: 'trace-current-tool',
            run_id: 'run-current-tool',
            conversation_id: 'conversation-current-tool',
            message_id: 'assistant-current-tool',
            turn_id: 'turn-current-tool',
            event_id: `event-${sequence}`,
            sequence,
            emitted_at: '2026-08-29T00:00:00.000Z',
            kind,
            status,
            span_id: callId,
            payload: {
                tool_name: callId === 'search' ? 'code_search' : 'read_file',
                call_id: callId,
                summary: `${kind}:${callId}`,
                arguments: { path: 'Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md' },
            },
        },
    };
}

let result = store.ingest(event(1, 'tool.started', 'running', 'read'));
result = store.ingest(event(2, 'tool.completed', 'succeeded', 'read'));
result = store.ingest(event(3, 'tool.started', 'running', 'search'));

if (result.state.tools.length !== 2) throw new Error('canonical tool history was lost');
if (result.state.currentTool?.callId !== 'search') throw new Error('latest active tool is not live focus');

result = store.ingest(event(4, 'tool.completed', 'succeeded', 'search'));
if (result.state.tools.length !== 2) throw new Error('terminal history was lost');
if (result.state.currentTool !== null) throw new Error('terminal tool remained in live focus');

process.stdout.write('agent trace current tool presentation: ok\n');
