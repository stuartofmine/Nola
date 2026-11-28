import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import http from 'node:http';
import { createServer as createNetServer } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';
import { chromium } from '../../windows/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../..');
const WEB_ROOT = path.join(REPO_ROOT, 'apps/web');
const NOLA_BIN = process.env.NOLA_BIN || path.join(REPO_ROOT, 'target/debug/nola');
const CHROME_PATH = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const SCREENSHOT_DIR = path.resolve(
    process.env.NOLA_WEB_GEOMETRY_SCREENSHOT_DIR ||
        '/Users/gaojc/.gemini/antigravity-cli/brain/0758e831-b598-4682-bd58-904f47af48c3/scratch',
);
const HEADLESS = process.env.NOLA_E2E_HEADED !== '1';
const NOTES_MARKER = 'NOLA_GEOMETRY_MARKER_7742';

// ---------------------------------------------------------------------------
// Utility Helpers (Ports, PNG/PDF fixtures, Color/Contrast math)
// ---------------------------------------------------------------------------

function findFreePort() {
    return new Promise((resolve, reject) => {
        const srv = createNetServer();
        srv.unref();
        srv.on('error', reject);
        srv.listen(0, '127.0.0.1', () => {
            const addr = srv.address();
            const port = typeof addr === 'object' && addr ? addr.port : 0;
            srv.close(() => resolve(port));
        });
    });
}

async function waitForServer(url, timeoutMs = 20_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const res = await fetch(url);
            if (res.ok) return;
        } catch {
            // Server still booting.
        }
        await new Promise(r => setTimeout(r, 200));
    }
    throw new Error(`Timed out waiting for nola web server at ${url}`);
}

const CRC_TABLE = (() => {
    const table = new Uint32Array(256);
    for (let n = 0; n < 256; n += 1) {
        let c = n;
        for (let k = 0; k < 8; k += 1) {
            c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
        }
        table[n] = c >>> 0;
    }
    return table;
})();

function crc32(buf) {
    let crc = 0xffffffff;
    for (let i = 0; i < buf.length; i += 1) {
        crc = CRC_TABLE[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
    }
    return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length, 0);
    const typeBuf = Buffer.from(type, 'ascii');
    const crcBuf = Buffer.alloc(4);
    crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
    return Buffer.concat([len, typeBuf, data, crcBuf]);
}

/** Builds a valid RGB PNG image (default 24x24 gradient) so naturalWidth/naturalHeight > 0. */
function makeSamplePng(width = 24, height = 24) {
    const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
    const ihdr = Buffer.alloc(13);
    ihdr.writeUInt32BE(width, 0);
    ihdr.writeUInt32BE(height, 4);
    ihdr[8] = 8; // bit depth
    ihdr[9] = 2; // RGB
    ihdr[10] = 0;
    ihdr[11] = 0;
    ihdr[12] = 0;

    const stride = width * 3 + 1;
    const raw = Buffer.alloc(stride * height);
    for (let y = 0; y < height; y += 1) {
        const rowOffset = y * stride;
        raw[rowOffset] = 0; // filter None
        for (let x = 0; x < width; x += 1) {
            const px = rowOffset + 1 + x * 3;
            raw[px] = Math.round((x / Math.max(1, width - 1)) * 220) + 20;
            raw[px + 1] = Math.round((y / Math.max(1, height - 1)) * 180) + 40;
            raw[px + 2] = 200;
        }
    }
    const idat = zlib.deflateSync(raw);
    return Buffer.concat([
        signature,
        pngChunk('IHDR', ihdr),
        pngChunk('IDAT', idat),
        pngChunk('IEND', Buffer.alloc(0)),
    ]);
}

/** Builds a valid single-page PDF document for inline right-sidebar PDF preview verification. */
function makeMinimalPdf(text = 'Nola Geometry E2E PDF Preview') {
    const objects = [
        '1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n',
        '2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n',
        '3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 360 240] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n',
        `4 0 obj\n<< /Length ${38 + text.length} >>\nstream\nBT /F1 14 Tf 28 180 Td (${text}) Tj ET\nendstream\nendobj\n`,
        '5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n',
    ];
    let pdf = '%PDF-1.4\n';
    const offsets = [0];
    for (const obj of objects) {
        offsets.push(Buffer.byteLength(pdf, 'utf8'));
        pdf += obj;
    }
    const xrefOffset = Buffer.byteLength(pdf, 'utf8');
    pdf += `xref\n0 ${objects.length + 1}\n`;
    pdf += '0000000000 65535 f \n';
    for (let i = 1; i < offsets.length; i += 1) {
        pdf += `${String(offsets[i]).padStart(10, '0')} 00000 n \n`;
    }
    pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
    return Buffer.from(pdf, 'utf8');
}

function parseRgb(colorStr) {
    const m = String(colorStr || '').match(/rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)/i);
    if (!m) return null;
    return [Number(m[1]), Number(m[2]), Number(m[3])];
}

function relativeLuminance([r, g, b]) {
    const toLinear = c => {
        const s = c / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
}

function contrastRatio(rgbA, rgbB) {
    const l1 = relativeLuminance(rgbA);
    const l2 = relativeLuminance(rgbB);
    const hi = Math.max(l1, l2);
    const lo = Math.min(l1, l2);
    return (hi + 0.05) / (lo + 0.05);
}

// ---------------------------------------------------------------------------
// Deterministic Local OpenAI-Compatible Mock Server (Zero External Network)
// ---------------------------------------------------------------------------

function readJsonBody(req) {
    return new Promise((resolve, reject) => {
        const chunks = [];
        req.on('data', c => chunks.push(c));
        req.on('end', () => {
            const raw = Buffer.concat(chunks).toString('utf8').trim();
            if (!raw) return resolve({});
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

function writeSse(res, payload) {
    if (res.writableEnded || res.destroyed) return false;
    res.write(`data: ${JSON.stringify(payload)}\n\n`);
    return true;
}

function deltaChunk(id, model, delta, finishReason = null, usage = undefined) {
    const chunk = {
        id,
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [{ index: 0, delta, finish_reason: finishReason }],
    };
    if (usage) chunk.usage = usage;
    return chunk;
}

async function startDeterministicMockLlm({ workspaceDir, modelId = 'mock-geo-model' }) {
    const requests = [];
    const server = http.createServer(async (req, res) => {
        const url = new URL(req.url || '/', 'http://127.0.0.1');
        const pathname = url.pathname;

        res.setHeader('Access-Control-Allow-Origin', '*');
        res.setHeader('Access-Control-Allow-Headers', '*');
        if (req.method === 'OPTIONS') {
            res.writeHead(204);
            res.end();
            return;
        }

        if (req.method === 'GET' && (pathname === '/props' || pathname === '/v1/props')) {
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(
                JSON.stringify({
                    default_generation_settings: { n_ctx: 131072, params: { n_predict: 4096 } },
                    total_slots: 1,
                    model_alias: modelId,
                }),
            );
            return;
        }

        if (req.method === 'GET' && (pathname === '/v1/models' || pathname === '/models')) {
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(
                JSON.stringify({
                    object: 'list',
                    data: [{ id: modelId, object: 'model', created: 1700000000, owned_by: 'nola-e2e-mock' }],
                }),
            );
            return;
        }

        if (req.method === 'POST' && (pathname === '/v1/chat/completions' || pathname === '/chat/completions')) {
            let body;
            try {
                body = await readJsonBody(req);
            } catch (err) {
                res.writeHead(400, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: { message: `Invalid JSON: ${err.message}` } }));
                return;
            }

            const model = typeof body.model === 'string' && body.model ? body.model : modelId;
            const stream = body.stream !== false;
            const messages = Array.isArray(body.messages) ? body.messages : [];
            const systemTexts = messages
                .filter(m => m?.role === 'system')
                .map(m => extractMessageText(m.content))
                .join('\n');

            let lastUserIdx = -1;
            for (let i = messages.length - 1; i >= 0; i -= 1) {
                if (messages[i]?.role === 'user') {
                    lastUserIdx = i;
                    break;
                }
            }
            const lastUserText = lastUserIdx >= 0 ? extractMessageText(messages[lastUserIdx].content) : '';
            const toolMessagesAfterUser =
                lastUserIdx >= 0 ? messages.slice(lastUserIdx + 1).filter(m => m?.role === 'tool') : [];

            requests.push({
                model,
                stream,
                lastUserText: lastUserText.slice(0, 160),
                toolCountAfterUser: toolMessagesAfterUser.length,
            });

            const cid = `chatcmpl-geo-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

            // Auxiliary title / compaction requests
            if (
                /generate a concise.*title|conversation title|short title/i.test(systemTexts) ||
                /summarize the conversation|compact/i.test(systemTexts)
            ) {
                const reply = /title/i.test(systemTexts) ? '几何与工作流验证会话' : 'Compacted geometry summary.';
                if (!stream) {
                    res.writeHead(200, { 'Content-Type': 'application/json' });
                    res.end(
                        JSON.stringify({
                            id: cid,
                            object: 'chat.completion',
                            created: Math.floor(Date.now() / 1000),
                            model,
                            choices: [{ index: 0, message: { role: 'assistant', content: reply }, finish_reason: 'stop' }],
                            usage: { prompt_tokens: 20, completion_tokens: 10, total_tokens: 30 },
                        }),
                    );
                    return;
                }
                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                writeSse(res, deltaChunk(cid, model, { role: 'assistant', content: reply }));
                writeSse(res, deltaChunk(cid, model, {}, 'stop'));
                res.write('data: [DONE]\n\n');
                res.end();
                return;
            }

            // Scenario 4: Deterministic Upstream HTTP 400 error
            if (lastUserText.includes('[GEO_UPSTREAM_400]')) {
                res.writeHead(400, { 'Content-Type': 'application/json' });
                res.end(
                    JSON.stringify({
                        error: {
                            message: 'Simulated upstream HTTP 400 invalid_request_error: context length exceeded',
                            type: 'invalid_request_error',
                            code: 'context_length_exceeded',
                        },
                    }),
                );
                return;
            }

            // Scenario 1: Long streaming mixed CJK + English reasoning (90 lines) for live bounded viewport test
            if (lastUserText.includes('[GEO_LONG_REASONING]')) {
                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                res.flushHeaders?.();

                let aborted = false;
                req.on('close', () => {
                    aborted = true;
                });

                const sendLongReasoning = async () => {
                    const totalLines = 90;
                    for (let i = 1; i <= totalLines; i += 1) {
                        if (aborted || res.writableEnded || res.destroyed) return;
                        const line =
                            `Step ${String(i).padStart(2, '0')}: 正在分析布局约束与滚动视口边界 — ` +
                            `verifying bounded reasoning viewport stability (line ${i}/${totalLines}).\n`;
                        const delta =
                            i === 1 ? { role: 'assistant', reasoning_content: line } : { reasoning_content: line };
                        writeSse(res, deltaChunk(cid, model, delta));
                        // Stream steadily (~2.2s total) so Playwright reliably samples mid-stream DOM geometry
                        await new Promise(r => setTimeout(r, 24));
                    }
                    if (aborted || res.writableEnded || res.destroyed) return;
                    const finalReply =
                        '深度推理已完成，思考框保持有界滚动窗口，无垂直跳动。**LONG_REASONING_BOUNDED_OK**';
                    writeSse(res, deltaChunk(cid, model, { content: finalReply }));
                    writeSse(
                        res,
                        deltaChunk(cid, model, {}, 'stop', {
                            prompt_tokens: 80,
                            completion_tokens: 120,
                            total_tokens: 200,
                        }),
                    );
                    res.write('data: [DONE]\n\n');
                    res.end();
                };
                void sendLongReasoning();
                return;
            }

            // Scenario 2: Interleaved read_file (OK) + failing bash (exit 128) -> rich CJK Markdown + Table + Code + Math
            if (lastUserText.includes('[GEO_TOOLS_AND_RICH_MD]')) {
                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                res.flushHeaders?.();

                if (toolMessagesAfterUser.length === 0) {
                    const notesPath = path.join(workspaceDir, 'notes.txt');
                    const readArgs = JSON.stringify({ path: notesPath });
                    const bashArgs = JSON.stringify({
                        command: "echo 'STDERR_PROBE_FAILURE: simulated non-zero exit 128' >&2; exit 128",
                    });
                    writeSse(
                        res,
                        deltaChunk(cid, model, {
                            role: 'assistant',
                            reasoning_content: '1. 调用 read_file 读取 notes.txt\n2. 调用 bash 运行诊断命令验证失败态渲染\n',
                        }),
                    );
                    writeSse(
                        res,
                        deltaChunk(cid, model, {
                            tool_calls: [
                                {
                                    index: 0,
                                    id: 'call_geo_read_file_1',
                                    type: 'function',
                                    function: { name: 'read_file', arguments: readArgs },
                                },
                                {
                                    index: 1,
                                    id: 'call_geo_bash_fail_2',
                                    type: 'function',
                                    function: { name: 'bash', arguments: bashArgs },
                                },
                            ],
                        }),
                    );
                    writeSse(res, deltaChunk(cid, model, {}, 'tool_calls'));
                    res.write('data: [DONE]\n\n');
                    res.end();
                    return;
                }

                const allToolOutputs = toolMessagesAfterUser.map(m => extractMessageText(m.content)).join('\n');
                const observedMarker = allToolOutputs.includes(NOTES_MARKER) ? NOTES_MARKER : 'MISSING_MARKER';
                const observedStderr = allToolOutputs.includes('STDERR_PROBE_FAILURE')
                    ? 'STDERR_CAPTURED'
                    : 'MISSING_STDERR';
                const richReply = [
                    '## 渲染几何与多模态工具验证报告',
                    '',
                    `- 已成功读取工作区标记：\`${observedMarker}\``,
                    `- 诊断 Shell 命令按预期返回非零退出码（exit 128，\`${observedStderr}\`），工具卡片已展示失败状态。`,
                    '- 以下验证行内公式 $p = n_s + n_t$ 与 $E = mc^2$ 的 2D 排版。',
                    '',
                    '$$\\mathbf{S}_s = \\lambda \\mathbf{S}_t + \\mathbf{U}\\mathbf{V}^\\top$$',
                    '',
                    '| 检查项 | 验证范围 | 目标阈值 | 状态 |',
                    '| --- | --- | --- | --- |',
                    '| 思考视口 | 长推理滚动窗口 | <= 180px | 通过 |',
                    '| 工具失败态 | bash exit 128 | 红色失败标签 | 通过 |',
                    '| 数学与表格 | CJK + LaTeX + Table | 无水平溢出 | 通过 |',
                    '',
                    '```ts',
                    'export function verifyLongUnbrokenHorizontalLineGeometryWithoutPageOverflow(viewportWidthPx: number, containerRightEdgePx: number, tolerancePx = 2): boolean {',
                    '    const diagnosticBanner = "LONG_CODE_LINE_0123456789_ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz_0123456789_ABCDEFGHIJKLMNOPQRSTUVWXYZ_END_OF_LONG_LINE_TOKEN";',
                    '    return diagnosticBanner.length > 100 && containerRightEdgePx <= viewportWidthPx + tolerancePx;',
                    '}',
                    '```',
                    '',
                    '验证完成标记：**RICH_TOOLS_MD_OK**',
                ].join('\n');

                writeSse(res, deltaChunk(cid, model, { role: 'assistant', content: richReply }));
                writeSse(
                    res,
                    deltaChunk(cid, model, {}, 'stop', {
                        prompt_tokens: 180,
                        completion_tokens: 160,
                        total_tokens: 340,
                    }),
                );
                res.write('data: [DONE]\n\n');
                res.end();
                return;
            }

            // Scenario 3: Permission approval (cautious write_file -> Allow or Deny)
            if (lastUserText.includes('[GEO_PERMISSION_WRITE]')) {
                const targetMatch = lastUserText.match(/TARGET_PATH=([^\s]+)/);
                const targetPath = targetMatch ? targetMatch[1] : path.join(workspaceDir, 'perm_probe.txt');

                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                res.flushHeaders?.();

                if (toolMessagesAfterUser.length === 0) {
                    const writeArgs = JSON.stringify({
                        path: targetPath,
                        content: 'PERMISSION_WRITE_VERIFIED\n',
                    });
                    writeSse(
                        res,
                        deltaChunk(cid, model, {
                            role: 'assistant',
                            reasoning_content: `准备在 cautious 模式下调用 write_file 写入 ${targetPath}。`,
                        }),
                    );
                    writeSse(
                        res,
                        deltaChunk(cid, model, {
                            tool_calls: [
                                {
                                    index: 0,
                                    id: `call_geo_write_${Date.now()}`,
                                    type: 'function',
                                    function: { name: 'write_file', arguments: writeArgs },
                                },
                            ],
                        }),
                    );
                    writeSse(res, deltaChunk(cid, model, {}, 'tool_calls'));
                    res.write('data: [DONE]\n\n');
                    res.end();
                    return;
                }

                const lastToolOut = extractMessageText(toolMessagesAfterUser[toolMessagesAfterUser.length - 1].content);
                const denied = /denied|rejected|not allowed|permission/i.test(lastToolOut);
                const reply = denied
                    ? `写入权限已按用户选择拒绝。**PERMISSION_WRITE_DENIED_OK**`
                    : `已获得写入授权并完成落盘：\`${path.basename(targetPath)}\`。**PERMISSION_WRITE_ALLOWED_OK**`;
                writeSse(res, deltaChunk(cid, model, { role: 'assistant', content: reply }));
                writeSse(
                    res,
                    deltaChunk(cid, model, {}, 'stop', {
                        prompt_tokens: 90,
                        completion_tokens: 30,
                        total_tokens: 120,
                    }),
                );
                res.write('data: [DONE]\n\n');
                res.end();
                return;
            }

            // Scenario 5: Image attachment ack
            if (lastUserText.includes('[GEO_IMAGE_ATTACH]')) {
                res.writeHead(200, {
                    'Content-Type': 'text/event-stream; charset=utf-8',
                    'Cache-Control': 'no-cache',
                    Connection: 'keep-alive',
                });
                writeSse(
                    res,
                    deltaChunk(cid, model, {
                        role: 'assistant',
                        content: '已收到上传的 PNG 图片附件，可在右侧栏内联预览。**IMAGE_ATTACHMENT_RECEIVED_OK**',
                    }),
                );
                writeSse(res, deltaChunk(cid, model, {}, 'stop', { prompt_tokens: 50, completion_tokens: 24, total_tokens: 74 }));
                res.write('data: [DONE]\n\n');
                res.end();
                return;
            }

            // Fallback deterministic reply
            res.writeHead(200, {
                'Content-Type': 'text/event-stream; charset=utf-8',
                'Cache-Control': 'no-cache',
                Connection: 'keep-alive',
            });
            writeSse(res, deltaChunk(cid, model, { role: 'assistant', content: `Mock OK (${model})` }));
            writeSse(res, deltaChunk(cid, model, {}, 'stop'));
            res.write('data: [DONE]\n\n');
            res.end();
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
    return {
        port,
        baseUrl: `http://127.0.0.1:${port}`,
        v1Url: `http://127.0.0.1:${port}/v1`,
        modelId,
        requests,
        async close() {
            await new Promise(resolve => server.close(() => resolve()));
        },
    };
}

// ---------------------------------------------------------------------------
// Playwright Geometry & Workflow Assertion Helpers
// ---------------------------------------------------------------------------

async function assertNoHorizontalOverflow(page, label) {
    const metrics = await page.evaluate(() => ({
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
        bodyScrollWidth: document.body.scrollWidth,
    }));
    assert.ok(
        metrics.scrollWidth <= metrics.innerWidth + 2,
        `[${label}] Horizontal document overflow detected: scrollWidth=${metrics.scrollWidth} > innerWidth=${metrics.innerWidth} + 2`,
    );
}

async function waitForStreamComplete(page, { timeoutMs = 30_000 } = {}) {
    const stopBtn = page.locator('.composer-actions button.danger');
    const permCard = page.locator('.permission-card');
    const startDeadline = Date.now() + 5_000;
    while (Date.now() < startDeadline) {
        if (await stopBtn.isVisible().catch(() => false)) break;
        if (await permCard.isVisible().catch(() => false)) break;
        await page.waitForTimeout(60);
    }

    const deadline = Date.now() + timeoutMs;
    let idleTicks = 0;
    while (Date.now() < deadline) {
        const streamingNow = await stopBtn.isVisible().catch(() => false);
        const permNow = await permCard.isVisible().catch(() => false);
        if (!streamingNow && !permNow) {
            idleTicks += 1;
            if (idleTicks >= 3) return;
        } else {
            idleTicks = 0;
        }
        await page.waitForTimeout(120);
    }
    throw new Error('Timed out waiting for streaming turn to complete');
}

async function sendChatMessage(page, text) {
    const composer = page.locator('#chat-composer');
    await composer.waitFor({ state: 'visible' });
    await composer.fill(text);
    const sendBtn = page.locator('.composer-actions button[type="submit"]');
    await sendBtn.waitFor({ state: 'visible' });
    await sendBtn.click();
}

async function measureComposerRects(page) {
    return page.evaluate(() => {
        const wrap = document.querySelector('.composer-wrap');
        const box = document.querySelector('.composer-box');
        const toolbar = document.querySelector('.composer-toolbar');
        return {
            wrapHeight: wrap ? wrap.getBoundingClientRect().height : 0,
            boxHeight: box ? box.getBoundingClientRect().height : 0,
            toolbarHeight: toolbar ? toolbar.getBoundingClientRect().height : 0,
            scrollWidth: document.documentElement.scrollWidth,
            innerWidth: window.innerWidth,
        };
    });
}

async function assertComposerStableAcrossModeSwitches(page, label) {
    const permissionSelect = page.locator('.composer-toolbar select.toolbar-select').nth(1);
    const reasoningSelect = page.locator('.composer-toolbar select.toolbar-select').nth(2);
    await permissionSelect.waitFor({ state: 'visible' });
    await reasoningSelect.waitFor({ state: 'visible' });

    const baseline = await measureComposerRects(page);
    assert.ok(baseline.wrapHeight > 60 && baseline.boxHeight > 40, `[${label}] Invalid baseline composer rects: ${JSON.stringify(baseline)}`);
    assert.ok(
        baseline.scrollWidth <= baseline.innerWidth + 2,
        `[${label}] Horizontal overflow at baseline: ${JSON.stringify(baseline)}`,
    );

    const reasoningModes = ['none', 'low', 'medium', 'high', 'automatic'];
    for (const effort of reasoningModes) {
        await reasoningSelect.selectOption(effort);
        await page.waitForTimeout(120);
        const cur = await measureComposerRects(page);
        assert.ok(
            Math.abs(cur.wrapHeight - baseline.wrapHeight) <= 1.5,
            `[${label}] Composer wrap jumped vertically switching reasoning=${effort}: baseline=${baseline.wrapHeight}px vs current=${cur.wrapHeight}px (delta=${(cur.wrapHeight - baseline.wrapHeight).toFixed(2)}px)`,
        );
        assert.ok(
            Math.abs(cur.boxHeight - baseline.boxHeight) <= 1.5,
            `[${label}] Composer box jumped vertically switching reasoning=${effort}: baseline=${baseline.boxHeight}px vs current=${cur.boxHeight}px`,
        );
        assert.ok(
            cur.scrollWidth <= cur.innerWidth + 2,
            `[${label}] Horizontal overflow after reasoning=${effort}: ${JSON.stringify(cur)}`,
        );
    }

    const permissionModes = ['allow_all', 'cautious', 'allow_all'];
    for (const perm of permissionModes) {
        await permissionSelect.selectOption(perm);
        await page.waitForTimeout(180);
        const cur = await measureComposerRects(page);
        assert.ok(
            Math.abs(cur.wrapHeight - baseline.wrapHeight) <= 1.5,
            `[${label}] Composer wrap jumped vertically switching permission=${perm}: baseline=${baseline.wrapHeight}px vs current=${cur.wrapHeight}px (delta=${(cur.wrapHeight - baseline.wrapHeight).toFixed(2)}px)`,
        );
        assert.ok(
            Math.abs(cur.boxHeight - baseline.boxHeight) <= 1.5,
            `[${label}] Composer box jumped vertically switching permission=${perm}: baseline=${baseline.boxHeight}px vs current=${cur.boxHeight}px`,
        );
        assert.ok(
            cur.scrollWidth <= cur.innerWidth + 2,
            `[${label}] Horizontal overflow after permission=${perm}: ${JSON.stringify(cur)}`,
        );
    }
    return baseline;
}

async function assertRightSidebarInlinePreview(page, extraPages, label) {
    assert.equal(extraPages.length, 0, `[${label}] Expected ZERO new tabs/popups, but ${extraPages.length} opened`);
    const openDialogs = await page.locator('dialog[open]').count();
    assert.equal(openDialogs, 0, `[${label}] Expected ZERO modal <dialog open> overlays, found ${openDialogs}`);

    const inspector = page.locator('[data-testid="conversation-inspector"]');
    const inlinePreview = page.locator('[data-testid="inspector-inline-preview"]');
    await inspector.waitFor({ state: 'visible' });
    await inlinePreview.waitFor({ state: 'visible' });

    const geo = await page.evaluate(() => {
        const insp = document.querySelector('[data-testid="conversation-inspector"]');
        const prev = document.querySelector('[data-testid="inspector-inline-preview"]');
        return {
            innerWidth: window.innerWidth,
            innerHeight: window.innerHeight,
            scrollWidth: document.documentElement.scrollWidth,
            insp: insp ? insp.getBoundingClientRect().toJSON() : null,
            prev: prev ? prev.getBoundingClientRect().toJSON() : null,
        };
    });

    assert.ok(geo.insp && geo.prev, `[${label}] Missing inspector or preview rect`);
    assert.ok(geo.insp.width >= 260, `[${label}] Inspector width too narrow: ${geo.insp.width}px`);
    assert.ok(
        geo.insp.right <= geo.innerWidth + 2,
        `[${label}] Inspector overflows right viewport edge: right=${geo.insp.right}, innerWidth=${geo.innerWidth}`,
    );
    assert.ok(
        geo.insp.left >= geo.innerWidth / 2 - 40,
        `[${label}] Inspector is not docked on the right side: left=${geo.insp.left}, innerWidth=${geo.innerWidth}`,
    );
    assert.ok(
        geo.prev.left >= geo.insp.left - 2 && geo.prev.right <= geo.insp.right + 2,
        `[${label}] Inline preview escaped inspector bounds: prev=[${geo.prev.left},${geo.prev.right}] vs insp=[${geo.insp.left},${geo.insp.right}]`,
    );
    assert.ok(
        geo.scrollWidth <= geo.innerWidth + 2,
        `[${label}] Horizontal overflow while inspector inline preview open: scrollWidth=${geo.scrollWidth} > innerWidth=${geo.innerWidth}`,
    );
}

// ---------------------------------------------------------------------------
// Main Deterministic E2E Runner
// ---------------------------------------------------------------------------

async function run() {
    const startedAt = Date.now();
    await fs.mkdir(SCREENSHOT_DIR, { recursive: true });

    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'nola-web-geometry-e2e-'));
    const tempHome = path.join(tempRoot, 'home');
    const tempDataDir = path.join(tempRoot, 'data');
    const tempDbPath = path.join(tempDataDir, 'nola.db');
    const tempWorkspace = path.join(tempRoot, 'workspace');
    await fs.mkdir(tempHome, { recursive: true });
    await fs.mkdir(tempDataDir, { recursive: true });
    await fs.mkdir(tempWorkspace, { recursive: true });

    const notesPath = path.join(tempWorkspace, 'notes.txt');
    await fs.writeFile(
        notesPath,
        `${NOTES_MARKER}\n架构验证文档：确保 2D 渲染几何、工具失败态、权限审批与右侧栏内联预览均满足严格不变量。\n`,
        'utf8',
    );
    const samplePngPath = path.join(tempRoot, 'sample-diagram.png');
    await fs.writeFile(samplePngPath, makeSamplePng(24, 24));
    const sampleCodePath = path.join(tempRoot, 'architecture-notes.ts');
    await fs.writeFile(
        sampleCodePath,
        [
            `// ${NOTES_MARKER}`,
            'export interface InspectorPreviewContract {',
            '    readonly inlineRightSidebarOnly: true;',
            '    readonly zeroModalDialogs: true;',
            '    readonly zeroExtraBrowserTabs: true;',
            '}',
            '',
        ].join('\n'),
        'utf8',
    );
    const samplePdfBytes = makeMinimalPdf('Nola Geometry E2E PDF Preview');

    const mockLlm = await startDeterministicMockLlm({
        workspaceDir: tempWorkspace,
        modelId: 'mock-geo-model',
    });

    const webPort = await findFreePort();
    const baseUrl = `http://127.0.0.1:${webPort}`;
    const serverLogs = [];
    const serverProc = spawn(
        NOLA_BIN,
        [
            'web',
            '--host',
            '127.0.0.1',
            '--port',
            String(webPort),
            '--web-root',
            WEB_ROOT,
            '--db',
            tempDbPath,
        ],
        {
            cwd: tempWorkspace,
            env: {
                ...process.env,
                HOME: tempHome,
                NOLA_DATA_DIR: tempDataDir,
                NOLA_DB_PATH: tempDbPath,
            },
            stdio: ['ignore', 'pipe', 'pipe'],
        },
    );
    serverProc.stdout.on('data', c => serverLogs.push(c.toString('utf8')));
    serverProc.stderr.on('data', c => serverLogs.push(c.toString('utf8')));

    let browser;
    let context;
    const pageErrors = [];
    const consoleErrors = [];
    const extraPages = [];
    const screenshots = [];
    const results = {};
    let allowExpectedUpstreamError = false;

    const snap = async (page, filename) => {
        const dest = path.join(SCREENSHOT_DIR, filename);
        await page.screenshot({ path: dest, fullPage: false });
        screenshots.push(dest);
        return dest;
    };

    try {
        await waitForServer(`${baseUrl}/api/web/session`, 20_000);
        console.log(`[geometry-e2e] nola web ready at ${baseUrl}, mock LLM at ${mockLlm.v1Url}`);

        browser = await chromium.launch({
            executablePath: CHROME_PATH,
            headless: HEADLESS,
            args: ['--no-first-run', '--no-default-browser-check'],
        });

        context = await browser.newContext({
            viewport: { width: 1280, height: 800 },
            locale: 'zh-CN',
        });
        await context.addCookies([{ name: 'nola_ui_locale', value: 'zh-Hans', url: baseUrl }]);

        const page = await context.newPage();
        page.setDefaultTimeout(15_000);

        context.on('page', newPage => {
            extraPages.push(newPage.url());
        });
        page.on('pageerror', err => {
            pageErrors.push(String(err));
        });
        page.on('console', msg => {
            if (msg.type() === 'error') {
                const text = msg.text();
                if (allowExpectedUpstreamError && /400|Bad Request|Failed to load resource/i.test(text)) {
                    return;
                }
                consoleErrors.push({ text, location: msg.location() });
            }
        });

        // ------------------------------------------------------------------
        // Setup: Configure deterministic mock model in /models UI (reasoning enabled)
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/models`, { waitUntil: 'networkidle' });
        await page.locator('.models-toolbar button.primary').click();
        const createDialog = page.locator('dialog[open]').last();
        await createDialog.waitFor({ state: 'visible' });
        const inputs = createDialog.locator('.fields input:not([type="checkbox"]):not([type="number"])');
        await inputs.nth(0).fill('Deterministic Mock LLM');
        await inputs.nth(1).fill(mockLlm.v1Url);
        await inputs.nth(2).fill(mockLlm.modelId);
        await inputs.nth(3).fill('Mock Geo Model');
        // Enable reasoning capability checkbox
        await createDialog.locator('.uses').nth(1).locator('input[type="checkbox"]').nth(2).check();
        await createDialog.locator('button.primary').click();
        await createDialog.waitFor({ state: 'hidden' });

        const saveCatalogBtn = page.locator('.catalog-actions button.primary');
        await saveCatalogBtn.waitFor({ state: 'visible' });
        await saveCatalogBtn.click();
        await saveCatalogBtn.waitFor({ state: 'hidden' });

        // Navigate to /chat at desktop 1280x800
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        await page.locator('#chat-composer').waitFor({ state: 'visible' });
        await page.locator('.composer-toolbar select.toolbar-select').first().waitFor({ state: 'visible' });

        // Create an active conversation so permission/reasoning PATCH requests actually fire too
        await page.locator('.conversation-manager button.new-conversation').click();
        await page.waitForFunction(() => new URL(window.location.href).searchParams.has('conversation'));

        // Verify localized default title ("新对话" in zh-Hans, never raw "New Conversation") + custom select chevron
        const initialUiPolish = await page.evaluate(() => {
            const topbarTitle = document.querySelector('.content-tabs [role="tab"]')?.textContent?.trim() || '';
            const sidebarTitle = document.querySelector('.conversation-list .conversation-heading > span')?.textContent?.trim() || '';
            const firstSelect = document.querySelector('.composer-toolbar select.toolbar-select');
            const selectBgImage = firstSelect ? getComputedStyle(firstSelect).backgroundImage : 'none';
            return { topbarTitle, sidebarTitle, selectBgImage };
        });
        assert.equal(initialUiPolish.topbarTitle, '新对话', `Expected localized topbar title "新对话", got "${initialUiPolish.topbarTitle}"`);
        assert.equal(initialUiPolish.sidebarTitle, '新对话', `Expected localized sidebar title "新对话", got "${initialUiPolish.sidebarTitle}"`);
        assert.notEqual(initialUiPolish.selectBgImage, 'none', 'Expected custom SVG dropdown chevron on .toolbar-select');

        // ==================================================================
        // Suite 1: Composer Geometry Stability (Desktop 1280x800 & Narrow 720x800)
        // ==================================================================
        const desktopBaseline = await assertComposerStableAcrossModeSwitches(page, 'desktop-1280x800');

        // Verify toast (triggered by permission switch) does NOT overlap .composer-box on desktop
        const toastGeo = await page.evaluate(() => {
            const toastHost = document.querySelector('.toast-host');
            const box = document.querySelector('.composer-box');
            if (!toastHost || !box) return null;
            const tr = toastHost.getBoundingClientRect();
            const br = box.getBoundingClientRect();
            const overlaps = !(tr.right <= br.left || tr.left >= br.right || tr.bottom <= br.top || tr.top >= br.bottom);
            return { toastTop: tr.top, toastBottom: tr.bottom, composerTop: br.top, overlaps };
        });
        if (toastGeo) {
            assert.equal(toastGeo.overlaps, false, `Toast must NOT overlap .composer-box: ${JSON.stringify(toastGeo)}`);
            assert.ok(toastGeo.toastBottom < toastGeo.composerTop, `Toast should stay anchored above composer: ${JSON.stringify(toastGeo)}`);
        }

        await page.setViewportSize({ width: 720, height: 800 });
        await page.waitForTimeout(150);
        const narrowBaseline = await assertComposerStableAcrossModeSwitches(page, 'narrow-720x800');
        await snap(page, 'web-rewrite-01-composer-narrow.png');

        await page.setViewportSize({ width: 1280, height: 800 });
        await page.waitForTimeout(150);
        results.suite1ComposerStability = {
            ok: true,
            desktopWrapHeight: desktopBaseline.wrapHeight,
            desktopBoxHeight: desktopBaseline.boxHeight,
            narrowWrapHeight: narrowBaseline.wrapHeight,
            narrowBoxHeight: narrowBaseline.boxHeight,
        };
        console.log('[geometry-e2e] Suite 1 PASSED (Composer Geometry Stability)');

        // ==================================================================
        // Suite 2: Live Streaming Long Reasoning Bounded Viewport
        // ==================================================================
        await sendChatMessage(page, '[GEO_LONG_REASONING] 请进行深度架构分析并给出结论。');

        const reasoningPre = page.locator('.reasoning-box[open] pre.reasoning-scroll').last();
        await reasoningPre.waitFor({ state: 'visible', timeout: 10_000 });

        // Wait until live streaming reasoning genuinely overflows its 180px max-height viewport
        let liveReasoningMetrics = null;
        const sampleDeadline = Date.now() + 8_000;
        while (Date.now() < sampleDeadline) {
            const metrics = await page.evaluate(() => {
                const pre = document.querySelector('.reasoning-box[open] pre.reasoning-scroll');
                const stopBtn = document.querySelector('.composer-actions button.danger');
                if (!pre) return null;
                const rect = pre.getBoundingClientRect();
                return {
                    isStreaming: Boolean(stopBtn),
                    height: rect.height,
                    width: rect.width,
                    scrollHeight: pre.scrollHeight,
                    clientHeight: pre.clientHeight,
                    scrollTop: pre.scrollTop,
                    textLen: (pre.textContent || '').length,
                    scrollWidth: document.documentElement.scrollWidth,
                    innerWidth: window.innerWidth,
                };
            });
            if (metrics && metrics.scrollHeight > metrics.clientHeight + 40) {
                liveReasoningMetrics = metrics;
                break;
            }
            await page.waitForTimeout(80);
        }

        assert.ok(
            liveReasoningMetrics,
            'Expected live streaming reasoning box to overflow internally (scrollHeight > clientHeight + 40)',
        );
        assert.ok(
            liveReasoningMetrics.height <= 181.5,
            `Live reasoning box expanded beyond bounded 180px max-height: got ${liveReasoningMetrics.height}px`,
        );
        assert.ok(
            liveReasoningMetrics.height >= 80,
            `Live reasoning box collapsed unexpectedly: got ${liveReasoningMetrics.height}px`,
        );
        assert.ok(
            liveReasoningMetrics.scrollHeight > liveReasoningMetrics.clientHeight,
            `Expected internal scrolling in reasoning box: scrollHeight=${liveReasoningMetrics.scrollHeight} <= clientHeight=${liveReasoningMetrics.clientHeight}`,
        );
        assert.ok(
            liveReasoningMetrics.scrollWidth <= liveReasoningMetrics.innerWidth + 2,
            `Horizontal overflow during live reasoning stream: ${liveReasoningMetrics.scrollWidth} > ${liveReasoningMetrics.innerWidth}`,
        );
        await snap(page, 'web-rewrite-02-live-reasoning-bounded.png');

        await waitForStreamComplete(page);
        const reasoningReplyText = await page.locator('.message.assistant .message-body').last().innerText();
        assert.match(reasoningReplyText, /LONG_REASONING_BOUNDED_OK/);
        results.suite2LiveReasoning = { ok: true, ...liveReasoningMetrics };
        console.log('[geometry-e2e] Suite 2 PASSED (Live Streaming Long Reasoning Bounded Viewport)');

        // ==================================================================
        // Suite 3: Multi-Turn Interleaved Tools (read_file OK + bash Fail Red)
        //          + CJK + Markdown Table + Code Block + Math
        // ==================================================================
        const permissionSelect = page.locator('.composer-toolbar select.toolbar-select').nth(1);
        await permissionSelect.selectOption('allow_all');
        await page.waitForTimeout(100);

        await sendChatMessage(
            page,
            '[GEO_TOOLS_AND_RICH_MD] 读取 notes.txt 并运行诊断命令，然后输出中文结构化报告。',
        );
        await waitForStreamComplete(page);

        const lastAssistantMsg = page.locator('article.message.assistant').last();
        const toolCards = lastAssistantMsg.locator('details.tool-card');
        const toolCardCount = await toolCards.count();
        assert.ok(toolCardCount >= 2, `Expected >= 2 tool cards (read_file + bash), got ${toolCardCount}`);

        const readFileCard = lastAssistantMsg.locator('details.tool-card', { hasText: 'read_file' }).first();
        const bashCard = lastAssistantMsg.locator('details.tool-card', { hasText: 'bash' }).first();
        await readFileCard.waitFor({ state: 'visible' });
        await bashCard.waitFor({ state: 'visible' });

        assert.equal(await readFileCard.getAttribute('data-status'), 'completed', 'read_file card must be completed');
        assert.equal(
            await bashCard.getAttribute('data-status'),
            'failed',
            'failing bash (exit 128) card must have data-status="failed", not false green completed!',
        );

        // Expand failing bash card to inspect its error styling & error text
        await bashCard.locator('summary').click();
        await readFileCard.locator('summary').click();
        await page.waitForTimeout(180);

        const toolVisualCheck = await lastAssistantMsg.evaluate(el => {
            const cards = Array.from(el.querySelectorAll('details.tool-card'));
            const findCard = name => cards.find(c => c.querySelector('.tool-name')?.textContent?.trim() === name);
            const okCard = findCard('read_file');
            const failCard = findCard('bash');
            const okBadge = okCard?.querySelector('.tool-status');
            const failBadge = failCard?.querySelector('.tool-status');
            return {
                okColor: okBadge ? getComputedStyle(okBadge).color : '',
                okLabel: okBadge?.textContent?.trim() || '',
                okSummary: okCard?.querySelector('.tool-summary')?.textContent || '',
                okInlinePreview: okCard?.querySelector('.tool-inline-preview')?.textContent?.trim() || '',
                failColor: failBadge ? getComputedStyle(failBadge).color : '',
                failLabel: failBadge?.textContent?.trim() || '',
                failSummary: failCard?.querySelector('.tool-summary')?.textContent || '',
                failInlinePreview: failCard?.querySelector('.tool-inline-preview')?.textContent?.trim() || '',
            };
        });

        const failRgb = parseRgb(toolVisualCheck.failColor);
        const okRgb = parseRgb(toolVisualCheck.okColor);
        assert.ok(failRgb && okRgb, `Could not parse tool badge colors: ${JSON.stringify(toolVisualCheck)}`);
        assert.notEqual(toolVisualCheck.failColor, toolVisualCheck.okColor, 'Failed tool badge must differ from succeeded badge');
        assert.ok(
            failRgb[0] > failRgb[1] + 40,
            `Expected failing bash badge to have red error color, got ${toolVisualCheck.failColor}`,
        );
        assert.ok(
            toolVisualCheck.okSummary.includes('notes.txt'),
            `Expected read_file summary to mention notes.txt, got: ${toolVisualCheck.okSummary.slice(0, 200)}`,
        );
        assert.ok(
            toolVisualCheck.okInlinePreview.includes('notes.txt'),
            `Expected read_file collapsed header preview to include notes.txt, got: ${toolVisualCheck.okInlinePreview}`,
        );
        assert.ok(
            /128|STDERR_PROBE_FAILURE|exit|error|failed/i.test(toolVisualCheck.failSummary),
            `Expected failing bash summary to report error/stderr, got: ${toolVisualCheck.failSummary.slice(0, 200)}`,
        );
        assert.ok(
            /128|STDERR_PROBE_FAILURE/i.test(toolVisualCheck.failInlinePreview),
            `Expected bash collapsed header preview to include exit/stderr snippet, got: ${toolVisualCheck.failInlinePreview}`,
        );

        // Verify rich Markdown (CJK, code block bounds + internal horizontal scroll, table cells, inline + display math)
        const richMdGeo = await lastAssistantMsg.evaluate(el => {
            const msgRect = el.getBoundingClientRect();
            const heading = el.querySelector('.message-body h2');
            const listItems = el.querySelectorAll('.message-body ul li');
            const codePre = el.querySelector('.message-body .code-block pre');
            const tableScroll = el.querySelector('.message-body .table-scroll');
            const tableCells = Array.from(el.querySelectorAll('.message-body table th, .message-body table td'));
            const inlineMath = el.querySelector('.message-body .math-inline');
            const displayMath = el.querySelector('.message-body .math-block');

            return {
                msgRight: msgRect.right,
                headingText: heading?.textContent?.trim() || '',
                listItemCount: listItems.length,
                codeRect: codePre ? codePre.getBoundingClientRect().toJSON() : null,
                codeScrollWidth: codePre?.scrollWidth || 0,
                codeClientWidth: codePre?.clientWidth || 0,
                tableWrapRect: tableScroll ? tableScroll.getBoundingClientRect().toJSON() : null,
                cellCount: tableCells.length,
                minCellWidth: tableCells.length
                    ? Math.min(...tableCells.map(c => c.getBoundingClientRect().width))
                    : 0,
                minCellHeight: tableCells.length
                    ? Math.min(...tableCells.map(c => c.getBoundingClientRect().height))
                    : 0,
                inlineMathRect: inlineMath ? inlineMath.getBoundingClientRect().toJSON() : null,
                displayMathRect: displayMath ? displayMath.getBoundingClientRect().toJSON() : null,
                bodyText: el.querySelector('.message-body')?.textContent || '',
            };
        });

        assert.ok(richMdGeo.headingText.includes('渲染几何'), `Missing CJK heading: ${richMdGeo.headingText}`);
        assert.ok(richMdGeo.listItemCount >= 3, `Expected >= 3 bullet list items, got ${richMdGeo.listItemCount}`);
        assert.ok(richMdGeo.bodyText.includes('RICH_TOOLS_MD_OK'), 'Missing RICH_TOOLS_MD_OK sentinel');
        assert.ok(richMdGeo.bodyText.includes(NOTES_MARKER), `Assistant reply did not echo real read_file content (${NOTES_MARKER})`);
        assert.ok(richMdGeo.bodyText.includes('STDERR_CAPTURED'), 'Assistant reply did not confirm real failing bash stderr');
        assert.ok(richMdGeo.codeRect, 'Missing fenced code block');
        assert.ok(
            richMdGeo.codeRect.right <= richMdGeo.msgRight + 2,
            `Code block overflowed message card: codeRight=${richMdGeo.codeRect.right} > msgRight=${richMdGeo.msgRight}`,
        );
        assert.ok(
            richMdGeo.codeScrollWidth > richMdGeo.codeClientWidth,
            `Expected long code line to scroll inside <pre>: scrollWidth=${richMdGeo.codeScrollWidth} <= clientWidth=${richMdGeo.codeClientWidth}`,
        );
        assert.ok(richMdGeo.tableWrapRect && richMdGeo.cellCount >= 12, `Missing Markdown table cells: ${richMdGeo.cellCount}`);
        assert.ok(
            richMdGeo.minCellWidth > 20 && richMdGeo.minCellHeight > 16,
            `Collapsed table cell detected: minWidth=${richMdGeo.minCellWidth}, minHeight=${richMdGeo.minCellHeight}`,
        );
        assert.ok(
            richMdGeo.tableWrapRect.right <= richMdGeo.msgRight + 2,
            `Table container overflowed message bounds: ${richMdGeo.tableWrapRect.right} > ${richMdGeo.msgRight}`,
        );
        assert.ok(
            richMdGeo.inlineMathRect && richMdGeo.inlineMathRect.width > 10 && richMdGeo.inlineMathRect.height > 10,
            `Invalid inline math rect: ${JSON.stringify(richMdGeo.inlineMathRect)}`,
        );
        assert.ok(
            richMdGeo.displayMathRect && richMdGeo.displayMathRect.width > 20 && richMdGeo.displayMathRect.height > 16,
            `Invalid display math rect: ${JSON.stringify(richMdGeo.displayMathRect)}`,
        );
        await assertNoHorizontalOverflow(page, 'suite3-rich-markdown');
        await snap(page, 'web-rewrite-03-tools-and-rich-markdown.png');
        results.suite3ToolsAndRichMarkdown = {
            ok: true,
            toolCardCount,
            failBadgeColor: toolVisualCheck.failColor,
            okBadgeColor: toolVisualCheck.okColor,
            tableCells: richMdGeo.cellCount,
            codeScrollsInternally: richMdGeo.codeScrollWidth > richMdGeo.codeClientWidth,
        };
        console.log('[geometry-e2e] Suite 3 PASSED (Tools Red/Green States + Rich CJK Markdown/Table/Code/Math)');

        // ==================================================================
        // Suite 4: Right-Sidebar Inline Preview (ZERO Modal Popups, ZERO New Tabs)
        // ==================================================================

        // 4a. Stage PNG + code file in composer -> click staged PNG chip -> verify right-sidebar inline preview -> Esc x2
        await page.locator('.composer-toolbar input[type="file"]').setInputFiles([samplePngPath, sampleCodePath]);
        const stagedPreviewBtn = page
            .locator('.composer-wrap .attachment-chips .chip-preview-btn', { hasText: 'sample-diagram.png' })
            .first();
        await stagedPreviewBtn.waitFor({ state: 'visible' });
        await stagedPreviewBtn.click();

        await assertRightSidebarInlinePreview(page, extraPages, '4a-staged-png');
        const stagedImg = await page.locator('[data-testid="inspector-inline-preview"] img.inspector-image-full').evaluate(img => ({
            complete: img.complete,
            naturalWidth: img.naturalWidth,
            naturalHeight: img.naturalHeight,
        }));
        assert.ok(stagedImg.complete && stagedImg.naturalWidth === 24 && stagedImg.naturalHeight === 24, `Invalid staged preview image: ${JSON.stringify(stagedImg)}`);
        await snap(page, 'web-rewrite-04a-staged-image-inline-preview.png');

        // Press Escape once -> exits inline preview back to inspector tabs
        await page.keyboard.press('Escape');
        await page.locator('[data-testid="inspector-inline-preview"]').waitFor({ state: 'hidden' });
        await page.locator('#inspector-tab-tasks').waitFor({ state: 'visible' });
        // Press Escape second time -> closes inspector cleanly
        await page.keyboard.press('Escape');
        await page.locator('[data-testid="conversation-inspector"]').waitFor({ state: 'hidden' });

        // 4b. Send staged PNG -> click sent attachment chip -> verify right-sidebar inline preview
        await sendChatMessage(page, '[GEO_IMAGE_ATTACH] 请确认收到图片附件。');
        await waitForStreamComplete(page);
        assert.match(await page.locator('.message.assistant .message-body').last().innerText(), /IMAGE_ATTACHMENT_RECEIVED_OK/);

        const sentImgChip = page
            .locator('.message.user .attachment-chips .attachment-chip-btn', { hasText: 'sample-diagram.png' })
            .last();
        await sentImgChip.waitFor({ state: 'visible' });
        await sentImgChip.click();
        await assertRightSidebarInlinePreview(page, extraPages, '4b-sent-png');
        const sentImg = await page.locator('[data-testid="inspector-inline-preview"] img.inspector-image-full').evaluate(img => ({
            complete: img.complete,
            naturalWidth: img.naturalWidth,
            naturalHeight: img.naturalHeight,
        }));
        assert.ok(sentImg.complete && sentImg.naturalWidth === 24, `Invalid sent preview image: ${JSON.stringify(sentImg)}`);

        // Escape once -> back to inspector tabs
        await page.keyboard.press('Escape');
        await page.locator('[data-testid="inspector-inline-preview"]').waitFor({ state: 'hidden' });

        // 4c. Artifacts tab -> click image artifact & file/text artifact -> both open inside right sidebar
        await page.locator('#inspector-tab-artifacts').click();
        await page.locator('#inspector-panel-artifacts').waitFor({ state: 'visible' });

        const imgArtBtn = page.locator('#inspector-panel-artifacts .art-thumb-btn').first();
        await imgArtBtn.waitFor({ state: 'visible' });
        await imgArtBtn.click();
        await assertRightSidebarInlinePreview(page, extraPages, '4c-artifact-image');
        const artImgWidth = await page
            .locator('[data-testid="inspector-inline-preview"] img.inspector-image-full')
            .evaluate(img => img.naturalWidth);
        assert.ok(artImgWidth > 0, `Artifact preview image naturalWidth=${artImgWidth}`);

        await page.keyboard.press('Escape');
        await page.locator('#inspector-panel-artifacts').waitFor({ state: 'visible' });

        // Click text/code artifact (architecture-notes.ts)
        const fileArtCard = page
            .locator('#inspector-panel-artifacts .art-card', { hasText: 'architecture-notes.ts' })
            .first();
        await fileArtCard.waitFor({ state: 'visible' });
        await fileArtCard.locator('.art-actions button.art-btn').last().click();
        await assertRightSidebarInlinePreview(page, extraPages, '4c-artifact-text');
        const previewPreText = await page
            .locator('[data-testid="inspector-inline-preview"] pre.inspector-text-preview')
            .innerText();
        assert.ok(
            previewPreText.includes(NOTES_MARKER) && previewPreText.includes('InspectorPreviewContract'),
            `Expected text/code artifact preview to display source code, got: ${previewPreText.slice(0, 200)}`,
        );
        await snap(page, 'web-rewrite-04b-artifact-text-inline-preview.png');

        await page.keyboard.press('Escape');
        await page.locator('#inspector-panel-artifacts').waitFor({ state: 'visible' });

        // 4d. Upload valid PDF resource to conversation -> open Browser/PDF tab -> inline PDF iframe preview
        const conversationId = new URL(page.url()).searchParams.get('conversation');
        assert.ok(conversationId, 'Expected active conversation query param in URL');
        const uploadStatus = await page.evaluate(
            async ({ convId, pdfBase64 }) => {
                const res = await fetch(`/api/chat/conversations/${encodeURIComponent(convId)}/resources`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        resourceId: crypto.randomUUID(),
                        displayName: 'architecture-spec.pdf',
                        mimeType: 'application/pdf',
                        dataBase64: pdfBase64,
                    }),
                });
                return { ok: res.ok, status: res.status };
            },
            { convId: conversationId, pdfBase64: samplePdfBytes.toString('base64') },
        );
        assert.ok(uploadStatus.ok, `PDF resource upload failed: ${JSON.stringify(uploadStatus)}`);

        await page.locator('#inspector-tab-browser').click();
        await page.locator('#inspector-panel-browser').waitFor({ state: 'visible' });
        const pdfItem = page.locator('#inspector-panel-browser .bp-item', { hasText: 'architecture-spec.pdf' }).first();
        await pdfItem.waitFor({ state: 'visible', timeout: 10_000 });
        await pdfItem.locator('.bp-actions button.bp-btn').first().click();

        await assertRightSidebarInlinePreview(page, extraPages, '4d-pdf-inline-preview');
        const pdfIframeInfo = await page
            .locator('[data-testid="inspector-inline-preview"] iframe.inspector-pdf-frame')
            .evaluate(frame => ({
                src: frame.getAttribute('src') || '',
                rect: frame.getBoundingClientRect().toJSON(),
            }));
        assert.match(pdfIframeInfo.src, /\/api\/chat\/conversations\/[^/]+\/resources\/[^/]+/);
        assert.ok(pdfIframeInfo.rect.height >= 200, `PDF iframe height too small: ${pdfIframeInfo.rect.height}`);
        await snap(page, 'web-rewrite-04c-pdf-inline-preview.png');

        // 4e. Double-Escape dismissal from PDF inline preview
        await page.keyboard.press('Escape');
        await page.locator('[data-testid="inspector-inline-preview"]').waitFor({ state: 'hidden' });
        await page.locator('#inspector-tab-browser').waitFor({ state: 'visible' });
        await page.keyboard.press('Escape');
        await page.locator('[data-testid="conversation-inspector"]').waitFor({ state: 'hidden' });

        results.suite4InlinePreview = {
            ok: true,
            extraPagesOpened: extraPages.length,
            stagedPngNaturalWidth: stagedImg.naturalWidth,
            sentPngNaturalWidth: sentImg.naturalWidth,
            artifactImageWidth: artImgWidth,
            artifactTextVerified: true,
            pdfIframeHeight: pdfIframeInfo.rect.height,
            doubleEscapeClosedInspector: true,
        };
        console.log('[geometry-e2e] Suite 4 PASSED (Right-Sidebar Inline Preview — ZERO Modals, ZERO New Tabs)');

        // ==================================================================
        // Suite 5: Permission Approval (cautious Allow + Deny) + Upstream HTTP 400 + Reload Persistence
        // ==================================================================
        await permissionSelect.selectOption('cautious');
        await page.waitForTimeout(120);

        // 5a. Allow write_file in cautious mode
        const allowedPath = path.join(tempWorkspace, 'approved_write.txt');
        await sendChatMessage(page, `[GEO_PERMISSION_WRITE] TARGET_PATH=${allowedPath} MODE=ALLOW`);
        const permCard = page.locator('.permission-card');
        await permCard.waitFor({ state: 'visible', timeout: 10_000 });
        assert.match(await permCard.locator('.permission-tool-badge').innerText(), /write_file/);
        const permReasonText = await permCard.locator('.permission-reason').innerText();
        assert.ok(
            !/cautious profile requires approval/i.test(permReasonText),
            `Expected localized permission reason in zh-Hans, got raw English: ${permReasonText}`,
        );

        const permRect = await permCard.evaluate(el => ({
            rect: el.getBoundingClientRect().toJSON(),
            innerWidth: window.innerWidth,
        }));
        assert.ok(
            permRect.rect.left >= 0 && permRect.rect.right <= permRect.innerWidth + 2 && permRect.rect.height > 40,
            `Permission card out of bounds: ${JSON.stringify(permRect)}`,
        );
        await snap(page, 'web-rewrite-05a-permission-card.png');

        await permCard.locator('.interactive-actions button.primary').click();
        await waitForStreamComplete(page);
        assert.equal((await fs.readFile(allowedPath, 'utf8')).trim(), 'PERMISSION_WRITE_VERIFIED');
        assert.match(await page.locator('.message.assistant .message-body').last().innerText(), /PERMISSION_WRITE_ALLOWED_OK/);

        // 5b. Deny write_file in cautious mode
        const deniedPath = path.join(tempWorkspace, 'denied_write.txt');
        await sendChatMessage(page, `[GEO_PERMISSION_WRITE] TARGET_PATH=${deniedPath} MODE=DENY`);
        await permCard.waitFor({ state: 'visible', timeout: 10_000 });
        await permCard.locator('.interactive-actions button.danger').click();
        await waitForStreamComplete(page);
        await assert.rejects(fs.access(deniedPath), { code: 'ENOENT' });
        assert.match(await page.locator('.message.assistant .message-body').last().innerText(), /PERMISSION_WRITE_DENIED_OK/);

        // 5c. Upstream HTTP 400 error recovery (visible error banner, no stuck spinner/stop button, composer enabled)
        allowExpectedUpstreamError = true;
        await sendChatMessage(page, '[GEO_UPSTREAM_400] 触发上游 400 错误恢复测试。');
        const errorBanner = page.locator('.composer-wrap .notice.error[role="alert"]');
        await errorBanner.waitFor({ state: 'visible', timeout: 12_000 });
        await waitForStreamComplete(page, { timeoutMs: 12_000 });
        allowExpectedUpstreamError = false;

        const errorText = await errorBanner.innerText();
        assert.ok(errorText.trim().length > 0, 'Expected non-empty error notice on upstream HTTP 400');
        assert.match(errorText, /400/, `Expected HTTP status 400 preserved in error banner: ${errorText}`);
        assert.ok(!errorText.includes('{"error":'), `Error banner should format JSON cleanly instead of raw JSON dump: ${errorText}`);
        assert.equal(await errorBanner.locator('.notice-dismiss').count(), 1, 'Error banner must provide a dismiss button');
        assert.equal(await page.locator('.composer-actions button.danger').count(), 0, 'Stop button must not remain stuck after HTTP 400');
        assert.equal(await page.locator('#chat-composer').isEnabled(), true, 'Composer textarea must re-enable after HTTP 400');
        await snap(page, 'web-rewrite-05b-upstream-400-recovery.png');
        await errorBanner.locator('.notice-dismiss').click();
        await errorBanner.waitFor({ state: 'hidden' });

        // 5d. Full page.reload() persistence from SQLite
        await page.reload({ waitUntil: 'networkidle' });
        await page.locator('.conversation-list .conversation-row').first().waitFor({ state: 'visible' });
        await page.locator('.message.assistant .message-body', { hasText: 'RICH_TOOLS_MD_OK' }).waitFor({ state: 'visible' });

        const persistedState = await page.evaluate(() => {
            const assistantTexts = Array.from(document.querySelectorAll('.message.assistant .message-body')).map(
                el => el.textContent || '',
            );
            const toolCards = Array.from(document.querySelectorAll('details.tool-card')).map(el => ({
                name: el.querySelector('.tool-name')?.textContent?.trim() || '',
                status: el.getAttribute('data-status') || '',
            }));
            const attImgs = Array.from(document.querySelectorAll('.message.user .attachment-thumb')).map(img => ({
                alt: img.getAttribute('alt') || '',
                naturalWidth: img.naturalWidth,
            }));
            return {
                conversationRows: document.querySelectorAll('.conversation-list .conversation-row').length,
                assistantCount: assistantTexts.length,
                hasRichMd: assistantTexts.some(t => t.includes('RICH_TOOLS_MD_OK')),
                hasAllowed: assistantTexts.some(t => t.includes('PERMISSION_WRITE_ALLOWED_OK')),
                hasDenied: assistantTexts.some(t => t.includes('PERMISSION_WRITE_DENIED_OK')),
                hasTable: Boolean(document.querySelector('.message.assistant table')),
                hasMath: Boolean(document.querySelector('.message.assistant .math-block')),
                toolCards,
                attImgs,
            };
        });

        assert.ok(persistedState.conversationRows >= 1, 'Conversation list must persist after reload');
        assert.ok(persistedState.hasRichMd && persistedState.hasTable && persistedState.hasMath, `Rich turn did not persist: ${JSON.stringify(persistedState)}`);
        assert.ok(persistedState.hasAllowed && persistedState.hasDenied, `Permission turns did not persist: ${JSON.stringify(persistedState)}`);
        assert.ok(
            persistedState.toolCards.some(tc => tc.name === 'read_file' && tc.status === 'completed'),
            `Persisted read_file completed card missing: ${JSON.stringify(persistedState.toolCards)}`,
        );
        assert.ok(
            persistedState.toolCards.some(tc => tc.name === 'bash' && tc.status === 'failed'),
            `Persisted bash failed card missing: ${JSON.stringify(persistedState.toolCards)}`,
        );
        assert.ok(
            persistedState.attImgs.some(img => img.alt === 'sample-diagram.png' && img.naturalWidth > 0),
            `Persisted PNG attachment thumbnail missing: ${JSON.stringify(persistedState.attImgs)}`,
        );

        results.suite5PermissionErrorAndReload = {
            ok: true,
            allowedWritten: true,
            deniedBlocked: true,
            upstream400Banner: errorText.slice(0, 120),
            persistedAssistantCount: persistedState.assistantCount,
        };
        console.log('[geometry-e2e] Suite 5 PASSED (Permission Allow/Deny + Upstream 400 Recovery + Reload Persistence)');

        // ==================================================================
        // Suite 6: Dark & Light Theme Contrast + Narrow Viewport (720x800)
        // ==================================================================
        const themeToggleBtn = page.locator('.sidebar-tools button.icon-btn').nth(1);
        await themeToggleBtn.waitFor({ state: 'visible' });

        const measureTheme = async () =>
            page.evaluate(() => {
                const shell = document.querySelector('.chat-shell') || document.body;
                const style = getComputedStyle(shell);
                return {
                    appearance: document.documentElement.dataset.appearance || 'system',
                    bg: style.backgroundColor,
                    fg: style.color,
                    scrollWidth: document.documentElement.scrollWidth,
                    innerWidth: window.innerWidth,
                };
            });

        // Toggle explicitly to dark and light so both themes are guaranteed verified & screenshotted
        let themeA = await measureTheme();
        await themeToggleBtn.click();
        await page.waitForTimeout(150);
        let themeB = await measureTheme();

        const darkTheme = themeB.appearance === 'dark' ? themeB : themeA;
        if (themeB.appearance !== 'dark') {
            await themeToggleBtn.click();
            await page.waitForTimeout(150);
        }
        const darkMeasured = await measureTheme();
        assert.equal(darkMeasured.appearance, 'dark', `Expected dark appearance, got ${JSON.stringify(darkMeasured)}`);
        const darkBg = parseRgb(darkMeasured.bg);
        const darkFg = parseRgb(darkMeasured.fg);
        assert.ok(darkBg && darkFg, `Unable to parse dark theme colors: ${JSON.stringify(darkMeasured)}`);
        const darkContrast = contrastRatio(darkBg, darkFg);
        assert.ok(relativeLuminance(darkBg) < 0.2, `Dark background luminance too bright: ${darkMeasured.bg}`);
        assert.ok(darkContrast >= 4.5, `Dark theme contrast too low (${darkContrast.toFixed(2)}:1): ${JSON.stringify(darkMeasured)}`);
        await assertNoHorizontalOverflow(page, 'suite6-dark-theme');
        await snap(page, 'web-rewrite-06a-dark-theme.png');

        await themeToggleBtn.click();
        await page.waitForTimeout(150);
        const lightMeasured = await measureTheme();
        assert.equal(lightMeasured.appearance, 'light', `Expected light appearance, got ${JSON.stringify(lightMeasured)}`);
        const lightBg = parseRgb(lightMeasured.bg);
        const lightFg = parseRgb(lightMeasured.fg);
        assert.ok(lightBg && lightFg, `Unable to parse light theme colors: ${JSON.stringify(lightMeasured)}`);
        const lightContrast = contrastRatio(lightBg, lightFg);
        assert.ok(relativeLuminance(lightBg) > 0.75, `Light background luminance too dark: ${lightMeasured.bg}`);
        assert.ok(lightContrast >= 4.5, `Light theme contrast too low (${lightContrast.toFixed(2)}:1): ${JSON.stringify(lightMeasured)}`);
        await assertNoHorizontalOverflow(page, 'suite6-light-theme');
        await snap(page, 'web-rewrite-06b-light-theme.png');

        // Narrow viewport (720x800) final geometry & inspector overlay check
        await page.setViewportSize({ width: 720, height: 800 });
        await page.waitForTimeout(150);
        await assertNoHorizontalOverflow(page, 'suite6-narrow-720x800');
        await page.locator('[data-testid="inspector-toggle"]').click();
        await page.locator('[data-testid="conversation-inspector"]').waitFor({ state: 'visible' });
        const narrowInspectorRect = await page
            .locator('[data-testid="conversation-inspector"]')
            .evaluate(el => ({
                rect: el.getBoundingClientRect().toJSON(),
                innerWidth: window.innerWidth,
            }));
        assert.ok(
            narrowInspectorRect.rect.right <= narrowInspectorRect.innerWidth + 2 && narrowInspectorRect.rect.width >= 260,
            `Narrow viewport inspector out of bounds: ${JSON.stringify(narrowInspectorRect)}`,
        );
        const inspectorBadgeTexts = await page
            .locator('[data-testid="conversation-inspector"] .task-list .status-pill')
            .allInnerTexts();
        assert.ok(inspectorBadgeTexts.length >= 2, `Expected task status badges in inspector, got ${JSON.stringify(inspectorBadgeTexts)}`);
        assert.ok(
            inspectorBadgeTexts.every(txt => !/^(completed|failed|running|pending)$/i.test(txt.trim())),
            `Inspector task badges must be localized in zh-Hans, got: ${JSON.stringify(inspectorBadgeTexts)}`,
        );
        assert.ok(
            inspectorBadgeTexts.some(txt => txt.includes('已完成')) && inspectorBadgeTexts.some(txt => txt.includes('失败')),
            `Expected localized 已完成 and 失败 badges in inspector, got: ${JSON.stringify(inspectorBadgeTexts)}`,
        );
        await assertNoHorizontalOverflow(page, 'suite6-narrow-720x800-inspector');
        await snap(page, 'web-rewrite-06c-narrow-viewport.png');
        await page.keyboard.press('Escape');
        await page.locator('[data-testid="conversation-inspector"]').waitFor({ state: 'hidden' });

        results.suite6ThemeAndNarrow = {
            ok: true,
            darkBg: darkMeasured.bg,
            darkContrast: Number(darkContrast.toFixed(2)),
            lightBg: lightMeasured.bg,
            lightContrast: Number(lightContrast.toFixed(2)),
            narrowInspectorWidth: Math.round(narrowInspectorRect.rect.width),
        };
        console.log('[geometry-e2e] Suite 6 PASSED (Dark & Light Contrast + Narrow 720x800)');

        // Final global cleanliness assertions
        assert.equal(extraPages.length, 0, `Expected ZERO popup tabs/windows, saw: ${JSON.stringify(extraPages)}`);
        assert.deepEqual(pageErrors, [], `Uncaught pageerrors: ${JSON.stringify(pageErrors)}`);
        assert.deepEqual(consoleErrors, [], `Unexpected console errors: ${JSON.stringify(consoleErrors)}`);

        const elapsedMs = Date.now() - startedAt;
        console.log(
            JSON.stringify(
                {
                    ok: true,
                    elapsedMs,
                    mockRequestCount: mockLlm.requests.length,
                    screenshots,
                    results,
                },
                null,
                2,
            ),
        );
    } catch (err) {
        console.error('[geometry-e2e] FAILED:', err);
        if (serverLogs.length) {
            console.error('[geometry-e2e] Recent nola web logs:\n', serverLogs.join('').slice(-3000));
        }
        process.exitCode = 1;
    } finally {
        if (context) await context.close().catch(() => {});
        if (browser) await browser.close().catch(() => {});
        if (serverProc && !serverProc.killed) {
            serverProc.kill('SIGTERM');
            await new Promise(resolve => {
                const t = setTimeout(() => {
                    serverProc.kill('SIGKILL');
                    resolve();
                }, 3_000);
                serverProc.once('exit', () => {
                    clearTimeout(t);
                    resolve();
                });
            });
        }
        await mockLlm.close().catch(() => {});
        await fs.rm(tempRoot, { recursive: true, force: true }).catch(() => {});
    }
}

await run();
