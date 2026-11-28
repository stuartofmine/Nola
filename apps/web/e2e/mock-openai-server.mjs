import http from 'node:http';

function readJsonBody(req) {
    return new Promise((resolve, reject) => {
        const chunks = [];
        req.on('data', chunk => chunks.push(chunk));
        req.on('end', () => {
            const raw = Buffer.concat(chunks).toString('utf8').trim();
            if (!raw) {
                resolve({});
                return;
            }
            try {
                resolve(JSON.parse(raw));
            } catch (err) {
                reject(err);
            }
        });
        req.on('error', reject);
    });
}

function extractMessageText(content) {
    if (typeof content === 'string') return content;
    if (Array.isArray(content)) {
        return content
            .map(part => {
                if (typeof part === 'string') return part;
                if (part && typeof part.text === 'string') return part.text;
                return '';
            })
            .join('\n');
    }
    return '';
}

function writeSseChunk(res, payload) {
    if (res.writableEnded || res.destroyed) return false;
    res.write(`data: ${JSON.stringify(payload)}\n\n`);
    return true;
}

function makeDeltaChunk(id, model, delta, finishReason = null, usage = undefined) {
    const chunk = {
        id,
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [
            {
                index: 0,
                delta,
                finish_reason: finishReason,
            },
        ],
    };
    if (usage) chunk.usage = usage;
    return chunk;
}

function makeNonStreamResponse(id, model, content, reasoning = '') {
    const message = {
        role: 'assistant',
        content,
    };
    if (reasoning) message.reasoning_content = reasoning;
    return {
        id,
        object: 'chat.completion',
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [
            {
                index: 0,
                message,
                finish_reason: 'stop',
            },
        ],
        usage: {
            prompt_tokens: 42,
            completion_tokens: 24,
            total_tokens: 66,
        },
    };
}

function splitIntoChunks(text, chunkSize = 28) {
    const parts = [];
    for (let i = 0; i < text.length; i += chunkSize) {
        parts.push(text.slice(i, i + chunkSize));
    }
    return parts.length ? parts : [''];
}

export async function startMockOpenAIServer({
    models = ['mock-nola-alpha', 'mock-nola-beta', 'mock-nola-gamma'],
} = {}) {
    const requests = [];
    const activeTimers = new Set();

    const server = http.createServer(async (req, res) => {
        const url = new URL(req.url || '/', 'http://127.0.0.1');
        const pathname = url.pathname;

        // CORS / preflight support if ever hit directly
        res.setHeader('Access-Control-Allow-Origin', '*');
        res.setHeader('Access-Control-Allow-Headers', '*');
        if (req.method === 'OPTIONS') {
            res.writeHead(204);
            res.end();
            return;
        }

        // llama.cpp compatible /props endpoint
        if (req.method === 'GET' && (pathname === '/props' || pathname === '/v1/props')) {
            requests.push({ method: req.method, pathname });
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(
                JSON.stringify({
                    default_generation_settings: {
                        n_ctx: 131072,
                        params: { n_predict: 4096 },
                    },
                    total_slots: 1,
                    model_alias: models[0],
                }),
            );
            return;
        }

        // OpenAI compatible model discovery
        if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
            requests.push({ method: req.method, pathname });
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(
                JSON.stringify({
                    object: 'list',
                    data: models.map(id => ({
                        id,
                        object: 'model',
                        created: 1700000000,
                        owned_by: 'mock-nola',
                    })),
                }),
            );
            return;
        }

        // OpenAI compatible chat completions (streaming SSE + non-streaming JSON)
        if (req.method === 'POST' && (pathname === '/v1/chat/completions' || pathname === '/chat/completions')) {
            let body;
            try {
                body = await readJsonBody(req);
            } catch (err) {
                res.writeHead(400, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: { message: `Invalid JSON: ${err.message}` } }));
                return;
            }

            const model = typeof body.model === 'string' && body.model ? body.model : models[0];
            const stream = body.stream !== false;
            const messages = Array.isArray(body.messages) ? body.messages : [];
            const systemTexts = messages
                .filter(m => m?.role === 'system')
                .map(m => extractMessageText(m.content))
                .join('\n');

            // Locate the last user message and any tool messages emitted after it.
            let lastUserIdx = -1;
            for (let i = messages.length - 1; i >= 0; i -= 1) {
                if (messages[i]?.role === 'user') {
                    lastUserIdx = i;
                    break;
                }
            }
            const lastUserText = lastUserIdx >= 0 ? extractMessageText(messages[lastUserIdx].content) : '';
            const toolMessagesAfterUser = lastUserIdx >= 0
                ? messages.slice(lastUserIdx + 1).filter(m => m?.role === 'tool')
                : [];

            requests.push({
                method: req.method,
                pathname,
                model,
                stream,
                lastUserText: lastUserText.slice(0, 200),
                toolCountAfterUser: toolMessagesAfterUser.length,
            });

            const completionId = `chatcmpl-mock-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

            // Auxiliary title/summary/compaction prompts
            if (
                /generate a concise.*title|conversation title|short title/i.test(systemTexts) ||
                /summarize the conversation|compact/i.test(systemTexts)
            ) {
                const reply = /title/i.test(systemTexts) ? 'Mock E2E Session' : 'Compacted E2E summary.';
                if (!stream) {
                    res.writeHead(200, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify(makeNonStreamResponse(completionId, model, reply)));
                    return;
                }
                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                writeSseChunk(res, makeDeltaChunk(completionId, model, { role: 'assistant', content: reply }));
                writeSseChunk(res, makeDeltaChunk(completionId, model, {}, 'stop'));
                res.write('data: [DONE]\n\n');
                res.end();
                return;
            }

            // Scenario A: Slow streaming response for mid-stream Stop/Cancel verification
            if (lastUserText.includes('[E2E_SLOW_STREAM]')) {
                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                res.flushHeaders?.();
                writeSseChunk(
                    res,
                    makeDeltaChunk(completionId, model, {
                        role: 'assistant',
                        reasoning_content: 'Planning a long incremental stream that can be cancelled mid-flight...',
                    }),
                );
                writeSseChunk(
                    res,
                    makeDeltaChunk(completionId, model, {
                        content: 'SLOW_STREAM_STARTED: chunk-0 ',
                    }),
                );

                let idx = 1;
                let closed = false;
                const cleanup = () => {
                    closed = true;
                    clearInterval(timer);
                    activeTimers.delete(timer);
                };
                const timer = setInterval(() => {
                    if (closed || res.writableEnded || res.destroyed) {
                        cleanup();
                        return;
                    }
                    if (idx > 60) {
                        writeSseChunk(res, makeDeltaChunk(completionId, model, {}, 'stop'));
                        res.write('data: [DONE]\n\n');
                        res.end();
                        cleanup();
                        return;
                    }
                    writeSseChunk(
                        res,
                        makeDeltaChunk(completionId, model, {
                            content: `chunk-${idx} `,
                        }),
                    );
                    idx += 1;
                }, 180);
                activeTimers.add(timer);
                req.on('close', cleanup);
                res.on('close', cleanup);
                return;
            }

            // Scenario B: Tool call requiring approval under cautious mode
            if (lastUserText.includes('[E2E_APPROVAL_TOOL]')) {
                const pathMatch = lastUserText.match(/TARGET_PATH=([^\s]+)/);
                const targetPath = pathMatch ? pathMatch[1] : 'approval_probe.txt';

                if (toolMessagesAfterUser.length === 0) {
                    // Emit a real write_file tool call
                    const toolArgs = JSON.stringify({
                        path: targetPath,
                        content: 'APPROVED_WRITE_OK\n',
                    });
                    const callId = `call_e2e_${Date.now()}`;

                    if (!stream) {
                        res.writeHead(200, { 'Content-Type': 'application/json' });
                        res.end(
                            JSON.stringify({
                                id: completionId,
                                object: 'chat.completion',
                                created: Math.floor(Date.now() / 1000),
                                model,
                                choices: [
                                    {
                                        index: 0,
                                        message: {
                                            role: 'assistant',
                                            content: null,
                                            reasoning_content: 'Requesting write_file to verify cautious permission approval.',
                                            tool_calls: [
                                                {
                                                    id: callId,
                                                    type: 'function',
                                                    function: { name: 'write_file', arguments: toolArgs },
                                                },
                                            ],
                                        },
                                        finish_reason: 'tool_calls',
                                    },
                                ],
                            }),
                        );
                        return;
                    }

                    res.writeHead(200, {
                        'Content-Type': 'text/event-stream; charset=utf-8',
                        'Cache-Control': 'no-cache',
                        Connection: 'keep-alive',
                    });
                    writeSseChunk(
                        res,
                        makeDeltaChunk(completionId, model, {
                            role: 'assistant',
                            reasoning_content: 'Requesting write_file to verify cautious permission approval.',
                        }),
                    );
                    writeSseChunk(
                        res,
                        makeDeltaChunk(completionId, model, {
                            tool_calls: [
                                {
                                    index: 0,
                                    id: callId,
                                    type: 'function',
                                    function: { name: 'write_file', arguments: '' },
                                },
                            ],
                        }),
                    );
                    writeSseChunk(
                        res,
                        makeDeltaChunk(completionId, model, {
                            tool_calls: [
                                {
                                    index: 0,
                                    function: { arguments: toolArgs },
                                },
                            ],
                        }),
                    );
                    writeSseChunk(res, makeDeltaChunk(completionId, model, {}, 'tool_calls'));
                    res.write('data: [DONE]\n\n');
                    res.end();
                    return;
                }

                // Follow-up turn after tool execution or denial
                const lastToolOutput = extractMessageText(
                    toolMessagesAfterUser[toolMessagesAfterUser.length - 1].content,
                );
                const wasDenied = /denied|rejected|not allowed|permission/i.test(lastToolOutput);
                const replyText = wasDenied
                    ? `Permission was denied as requested. E2E_APPROVAL_DENIED_OK`
                    : `Write tool succeeded after approval for \`${targetPath}\`. E2E_APPROVAL_ALLOWED_OK`;

                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                writeSseChunk(res, makeDeltaChunk(completionId, model, { role: 'assistant', content: replyText }));
                writeSseChunk(
                    res,
                    makeDeltaChunk(completionId, model, {}, 'stop', {
                        prompt_tokens: 110,
                        completion_tokens: 32,
                        total_tokens: 142,
                    }),
                );
                res.write('data: [DONE]\n\n');
                res.end();
                return;
            }

            // Scenario C: Rich math, markdown, code block, table, and reasoning
            let reasoning = '';
            let content = '';
            if (lastUserText.includes('[E2E_RICH_MATH]')) {
                reasoning =
                    'Step 1: Verify parameter count relation p = n_s + n_t.\n' +
                    'Step 2: Render low-rank decomposition equation S_s = lambda S_t + U V^T.\n' +
                    'Step 3: Emit structured Markdown table and TypeScript code block.';
                content = [
                    '## Rich Streaming Verification',
                    '',
                    'The total parameter count satisfies $p = n_s + n_t$ for the source and target components.',
                    '',
                    '$$S_s = \\lambda S_t + UV^\\top$$',
                    '',
                    '| Metric | Value | Status |',
                    '| --- | --- | --- |',
                    '| Inline Math | $p = n_s + n_t$ | Verified |',
                    '| Display Math | Low-rank update | Verified |',
                    '',
                    '```ts',
                    'export function totalParams(ns: number, nt: number): number {',
                    '    return ns + nt;',
                    '}',
                    '```',
                    '',
                    'Sentinel: **E2E_RICH_STREAM_OK**',
                ].join('\n');
            } else if (lastUserText.includes('[E2E_TURN_2]')) {
                reasoning = 'Checking previous context for multi-turn continuity.';
                content = 'Second turn confirmed in the same conversation. **E2E_TURN_2_OK**';
            } else if (lastUserText.includes('[E2E_CONV_2]')) {
                reasoning = 'Responding inside the second isolated conversation.';
                content = 'Isolated second conversation response. **E2E_CONV_2_OK**';
            } else if (lastUserText.includes('[E2E_FOLLOWUP]')) {
                reasoning = 'Confirming clean recovery after mid-stream cancellation.';
                content = 'Follow-up after cancel succeeded cleanly. **E2E_FOLLOWUP_OK**';
            } else if (lastUserText.includes('[E2E_EXPERT_CHECK]')) {
                reasoning = 'Running expert response verification.';
                content = 'Expert workflow verification complete. **E2E_EXPERT_OK**';
            } else {
                reasoning = 'Processing mock OpenAI completion request.';
                content = `Mock response for model \`${model}\`. E2E_MOCK_REPLY_OK`;
            }

            if (!stream) {
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify(makeNonStreamResponse(completionId, model, content, reasoning)));
                return;
            }

            res.writeHead(200, {
                'Content-Type': 'text/event-stream; charset=utf-8',
                'Cache-Control': 'no-cache',
                Connection: 'keep-alive',
            });
            res.flushHeaders?.();

            const reasoningParts = reasoning ? splitIntoChunks(reasoning, 40) : [];
            const contentParts = splitIntoChunks(content, 36);
            let aborted = false;
            req.on('close', () => {
                aborted = true;
            });

            const sendSequential = async () => {
                for (let i = 0; i < reasoningParts.length; i += 1) {
                    if (aborted || res.writableEnded || res.destroyed) return;
                    const delta = i === 0
                        ? { role: 'assistant', reasoning_content: reasoningParts[i] }
                        : { reasoning_content: reasoningParts[i] };
                    writeSseChunk(res, makeDeltaChunk(completionId, model, delta));
                    await new Promise(r => setTimeout(r, 15));
                }
                for (let i = 0; i < contentParts.length; i += 1) {
                    if (aborted || res.writableEnded || res.destroyed) return;
                    const delta = reasoningParts.length === 0 && i === 0
                        ? { role: 'assistant', content: contentParts[i] }
                        : { content: contentParts[i] };
                    writeSseChunk(res, makeDeltaChunk(completionId, model, delta));
                    await new Promise(r => setTimeout(r, 15));
                }
                if (aborted || res.writableEnded || res.destroyed) return;
                writeSseChunk(
                    res,
                    makeDeltaChunk(completionId, model, {}, 'stop', {
                        prompt_tokens: 96,
                        completion_tokens: 84,
                        total_tokens: 180,
                    }),
                );
                res.write('data: [DONE]\n\n');
                res.end();
            };

            void sendSequential();
            return;
        }

        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: `Unhandled mock route: ${req.method} ${pathname}` } }));
    });

    await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
    });

    const addr = server.address();
    const port = typeof addr === 'object' && addr ? addr.port : 0;
    const baseUrl = `http://127.0.0.1:${port}`;

    return {
        port,
        baseUrl,
        v1Url: `${baseUrl}/v1`,
        models,
        requests,
        async close() {
            for (const timer of activeTimers) clearInterval(timer);
            activeTimers.clear();
            await new Promise(resolve => server.close(() => resolve()));
        },
    };
}
