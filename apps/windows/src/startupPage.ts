// SPDX-License-Identifier: Apache-2.0
import type { DesktopStrings } from "./i18n";

export function startupPageHtml(strings: DesktopStrings, mode: "starting" | "failed", detail = ""): string {
    const failed = mode === "failed";
    const title = failed ? strings.failedTitle : strings.startingTitle;
    const body = failed ? strings.failedBody : strings.startingBody;
    const safeDetail = detail.replace(/[&<>"']/g, (character) => ({
        "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;",
    })[character] ?? character);
    return `<!doctype html><html data-theme="auto"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:"><title>${title}</title><style>
        :root{color-scheme:light dark;font-family:"Segoe UI Variable","Segoe UI",sans-serif;--canvas:#fff;--surface:#fff;--ink:#202020;--muted:#606060;--border:#e4e4e4;--outline:#858585;--accent:#202020;--on-accent:#fff;--panel:#f4f4f4;background:var(--canvas);color:var(--ink)}
        *{box-sizing:border-box}body{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--canvas)}
        .card{width:min(480px,calc(100vw - 40px));padding:32px;border:1px solid var(--border);border-radius:16px;background:var(--surface);text-align:center}
        .mark{width:48px;height:48px;margin:0 auto 24px;display:grid;place-items:center;border-radius:12px;background:var(--accent);color:var(--on-accent);font-size:24px;font-weight:650}
        h1{margin:0;font-size:24px;letter-spacing:-.025em;overflow-wrap:anywhere}p{margin:12px auto 0;max-width:38ch;color:var(--muted);font-size:14px;line-height:1.65}
        .pulse{width:40px;height:4px;margin:24px auto 0;overflow:hidden;border-radius:99px;background:var(--panel)}
        .pulse::after{display:block;width:100%;height:100%;border-radius:inherit;background:var(--accent);content:"";animation:breathe 2.4s ease-in-out infinite}
        @keyframes breathe{0%,100%{opacity:.35}50%{opacity:.8}}
        .detail{max-height:min(28vh,220px);overflow:auto;margin-top:20px;padding:12px;border-radius:8px;background:var(--panel);color:var(--muted);font:12px/1.6 Consolas,monospace;overflow-wrap:anywhere;text-align:start}
        .actions{display:flex;justify-content:center;gap:8px;margin-top:24px;flex-wrap:wrap}button{min-height:44px;padding:10px 16px;border:1px solid var(--outline);border-radius:10px;background:var(--surface);color:var(--ink);font:inherit;font-size:14px;font-weight:600;cursor:pointer;transition:background-color 120ms ease,opacity 120ms ease}
        button.primary{border-color:var(--accent);background:var(--accent);color:var(--on-accent)}button:not(.primary):hover{background:var(--panel)}button.primary:hover{opacity:.88}button:active{opacity:.8}button:focus-visible,.detail:focus-visible{outline:2px solid currentColor;outline-offset:3px}
        @media(prefers-color-scheme:dark){:root{--canvas:#000;--surface:#111;--ink:#eee;--muted:#adadad;--border:#292929;--outline:#777;--accent:#eee;--on-accent:#111;--panel:#191919}}
        @media(max-width:500px){.card{padding:24px}}
        @media(prefers-reduced-motion:reduce){.pulse::after{animation:none;opacity:1}button{transition:none}}
        @media(forced-colors:active){button:focus-visible{outline-color:Highlight}.card,button{border:1px solid ButtonText}}
    </style></head><body><main class="card"><div class="mark">N</div><h1>${title}</h1><p>${body}</p>${failed && safeDetail ? `<div class="detail" tabindex="0">${safeDetail}</div>` : ""}${failed ? `<div class="actions"><button id="nola-open-logs">${strings.openLogs}</button><button class="primary" id="nola-retry-host">${strings.retry}</button></div>` : `<div class="pulse" role="status" aria-label="${strings.startingTitle}"></div>`}</main></body></html>`;
}
