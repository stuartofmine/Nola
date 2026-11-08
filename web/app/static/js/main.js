// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements main logic in the web application.
// - Interfaces:
//   - apiCall
//   - ICONS
//   - UI_TEXT
//   - uiText
//   - icon
//   - ensureToastStack
//   - toast
//   - confirmDialog
// - APIs:
//   - none
//
/**
 * Nola Web · shared UI utilities
 *  - apiCall(): JSON fetch helper (preserves existing API)
 *  - Nola.toast(): floating notifications
 *  - Nola.confirm(): styled confirm dialog (replaces window.confirm)
 *  - Nola.icon(): inline SVG icon helper (lucide-style strokes)
 *  - Nola.skeleton(): shimmer placeholder
 *  - Mobile nav toggle (#navToggle)
 */

// ---------------- API helper (preserved) ----------------
async function apiCall(endpoint, method = 'GET', data = null) {
    const options = { method, headers: { 'Content-Type': 'application/json' } };
    if (data) options.body = JSON.stringify(data);
    const response = await fetch(`/api${endpoint}`, options);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.json();
}

// ---------------- Icons ----------------
const ICONS = {
    sidebar: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M9 4v16"/></svg>',
    home: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M4 11.5 12 5l8 6.5"/><path d="M7 10.5V19h10v-8.5"/><path d="M10 19v-4h4v4"/></svg>',
    chat: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h7A2.5 2.5 0 0 1 16 6.5v4A2.5 2.5 0 0 1 13.5 13H9l-4 3v-3H6.5A2.5 2.5 0 0 1 4 10.5z"/><path d="M14 9h3.5A2.5 2.5 0 0 1 20 11.5v3A2.5 2.5 0 0 1 17.5 17H16l-3 3v-3"/></svg>',
    scheduled: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="13" r="7"/><path d="M11 10v3.5l2.5 1.5"/><path d="M16.5 5.5h3v3"/><path d="M20 8.5a9 9 0 0 0-3.3-3"/></svg>',
    results: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3h6l5 5v11a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M14 3v5h5"/><path d="M9 13h6"/><path d="M9 17h6"/></svg>',
    cpu: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><rect x="7" y="7" width="10" height="10" rx="2"/><path d="M10 1v4M14 1v4M10 19v4M14 19v4M19 10h4M19 14h4M1 10h4M1 14h4"/></svg>',
    folder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5z"/></svg>',
    hand: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M8 11V5a1.5 1.5 0 0 1 3 0v4"/><path d="M11 9V4.5a1.5 1.5 0 0 1 3 0V9"/><path d="M14 9V6a1.5 1.5 0 0 1 3 0v6.5a5.5 5.5 0 0 1-11 0V9.5A1.5 1.5 0 0 1 8.5 8L8 11z"/></svg>',
    doc: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3h6l5 5v11a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M14 3v5h5"/><path d="M9 13h6"/><path d="M9 17h6"/></svg>',
    key: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"><circle cx="8.5" cy="15.5" r="3.5"/><path d="M12 15.5h8"/><path d="M17 12.5v6"/><path d="M20 13.5v4"/></svg>',
    check: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>',
    x: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>',
    info: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>',
    warning: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>',
    plus: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>',
    refresh: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2"/></svg>',
    play: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>',
    send: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>',
    sparkles: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.5 4.5L18 9l-4.5 1.5L12 15l-1.5-4.5L6 9l4.5-1.5z"/><path d="M19 14l.8 2.2L22 17l-2.2.8L19 20l-.8-2.2L16 17l2.2-.8z"/></svg>',
    bell: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>',
    clock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>',
    list: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>',
    chart: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="20" x2="12" y2="10"/><line x1="18" y1="20" x2="18" y2="4"/><line x1="6" y1="20" x2="6" y2="16"/></svg>',
    user: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>',
    settings: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
    moon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>',
    sun: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><line x1="12" y1="2" x2="12" y2="4"/><line x1="12" y1="20" x2="12" y2="22"/><line x1="4.93" y1="4.93" x2="6.34" y2="6.34"/><line x1="17.66" y1="17.66" x2="19.07" y2="19.07"/><line x1="2" y1="12" x2="4" y2="12"/><line x1="20" y1="12" x2="22" y2="12"/><line x1="4.93" y1="19.07" x2="6.34" y2="17.66"/><line x1="17.66" y1="6.34" x2="19.07" y2="4.93"/></svg>',
    menu: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/></svg>',
    arrow: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="5" y1="12" x2="19" y2="12"/><polyline points="12 5 19 12 12 19"/></svg>',
    inbox: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>',
};

const UI_TEXT = (window.NOLA_UI_TEXT && typeof window.NOLA_UI_TEXT === 'object') ? window.NOLA_UI_TEXT : {};
function uiText(key, fallback) {
    const value = UI_TEXT[key];
    return (typeof value === 'string' && value.length > 0) ? value : fallback;
}

function icon(name, attrs = {}) {
    const svg = ICONS[name];
    if (!svg) return '';
    const attrStr = Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(' ');
    return svg.replace('<svg ', `<svg ${attrStr} `);
}

// ---------------- Toast ----------------
function ensureToastStack() {
    let stack = document.querySelector('.toast-stack');
    if (!stack) {
        stack = document.createElement('div');
        stack.className = 'toast-stack';
        document.body.appendChild(stack);
    }
    return stack;
}

function toast(message, type = 'info', opts = {}) {
    const duration = opts.duration ?? (type === 'error' ? 0 : type === 'warning' ? 6000 : 3500);
    const stack = ensureToastStack();
    const toastKey = String(opts.key ?? `${type}:${message}`);
    const existing = Array.from(stack.children).find(item =>
        item.dataset.toastKey === toastKey && !item.classList.contains('leaving')
    );
    if (existing) {
        existing.classList.remove('repeated');
        requestAnimationFrame(() => existing.classList.add('repeated'));
        return existing._nolaCloseToast || (() => existing.remove());
    }
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.dataset.toastKey = toastKey;
    el.setAttribute('role', type === 'error' ? 'alert' : 'status');
    el.setAttribute('aria-live', type === 'error' ? 'assertive' : 'polite');
    el.setAttribute('aria-atomic', 'true');
    const iconName = type === 'success' ? 'check' : type === 'error' ? 'x' : type === 'warning' ? 'warning' : 'info';
    el.innerHTML = `
        <span class="t-icon">${icon(iconName)}</span>
        <div class="t-body">${escapeHtml(message)}</div>
        <button class="t-close" aria-label="${escapeHtml(uiText('close', 'Close'))}">${icon('x')}</button>
    `;
    stack.appendChild(el);
    while (stack.children.length > 4) stack.firstElementChild.remove();
    let timeoutId = null;
    const close = () => {
        if (el.classList.contains('leaving')) return;
        if (timeoutId) clearTimeout(timeoutId);
        el.classList.add('leaving');
        setTimeout(() => el.remove(), 220);
    };
    el._nolaCloseToast = close;
    el.querySelector('.t-close').addEventListener('click', close);
    if (duration > 0) timeoutId = setTimeout(close, duration);
    return close;
}

// ---------------- Confirm dialog ----------------
let confirmDialogSequence = 0;
function confirmDialog({
    title = uiText('confirmTitle', 'Confirm action'),
    message = '',
    confirmText = uiText('confirm', 'Confirm'),
    cancelText = uiText('cancel', 'Cancel'),
    danger = true,
} = {}) {
    return new Promise(resolve => {
        confirmDialogSequence += 1;
        const titleId = `nolaConfirmTitle${confirmDialogSequence}`;
        const messageId = `nolaConfirmMessage${confirmDialogSequence}`;
        const previouslyFocused = document.activeElement;
        const previousBodyOverflow = document.body.style.overflow;
        const wrap = document.createElement('div');
        wrap.className = 'modal';
        wrap.setAttribute('role', 'dialog');
        wrap.setAttribute('aria-modal', 'true');
        wrap.setAttribute('aria-labelledby', titleId);
        wrap.setAttribute('aria-describedby', messageId);
        wrap.innerHTML = `
            <div class="modal-content confirm-modal-content">
                <div class="confirm-card">
                    <div class="confirm-icon">${icon(danger ? 'warning' : 'info')}</div>
                    <h3 id="${titleId}">${escapeHtml(title)}</h3>
                    <p id="${messageId}">${escapeHtml(message)}</p>
                    <div class="confirm-actions">
                        <button class="btn btn-secondary" data-act="cancel">${escapeHtml(cancelText)}</button>
                        <button class="btn ${danger ? 'btn-danger' : 'btn-primary'}" data-act="ok">${escapeHtml(confirmText)}</button>
                    </div>
                </div>
            </div>
        `;
        document.body.style.overflow = 'hidden';
        document.body.appendChild(wrap);
        let settled = false;
        const cleanup = (val) => {
            if (settled) return;
            settled = true;
            wrap.remove();
            document.removeEventListener('keydown', onKey);
            document.body.style.overflow = previousBodyOverflow;
            if (previouslyFocused instanceof HTMLElement) previouslyFocused.focus();
            resolve(val);
        };
        const onKey = (e) => {
            if (e.key === 'Escape') cleanup(false);
            if (e.key === 'Tab') {
                const controls = Array.from(wrap.querySelectorAll('button:not([disabled])'));
                const first = controls[0];
                const last = controls[controls.length - 1];
                if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
                else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
            }
        };
        wrap.addEventListener('click', e => {
            if (e.target === wrap) cleanup(false);
            const act = e.target.closest('[data-act]')?.dataset.act;
            if (act === 'ok') cleanup(true);
            if (act === 'cancel') cleanup(false);
        });
        document.addEventListener('keydown', onKey);
        setTimeout(() => wrap.querySelector('[data-act="cancel"]').focus(), 0);
    });
}

// ---------------- Skeleton ----------------
function skeleton(rows = 3) {
    return Array.from({ length: rows }, () => '<div class="skeleton skeleton-card"></div>').join('');
}

// ---------------- Helpers ----------------
function escapeHtml(s) {
    return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;')
        .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// Expose
window.Nola = { toast, confirm: confirmDialog, icon, skeleton, escapeHtml, ICONS };

// ---------------- Boot ----------------
document.addEventListener('DOMContentLoaded', () => {
    // Mobile nav toggle
    const navToggle = document.getElementById('navToggle');
    const navLinks = document.getElementById('navLinks');
    if (navToggle && navLinks) {
        const setNavOpen = (open, restoreFocus = false) => {
            navLinks.classList.toggle('open', open);
            navToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
            document.body.classList.toggle('nav-open', open);
            if (!open && restoreFocus) navToggle.focus();
        };
        navToggle.addEventListener('click', () => setNavOpen(!navLinks.classList.contains('open')));
        // Close on link click (mobile)
        navLinks.addEventListener('click', e => {
            if (e.target.closest('a') || e.target.closest('button')) setNavOpen(false);
        });
        document.addEventListener('click', e => {
            if (!navLinks.classList.contains('open')) return;
            if (navLinks.contains(e.target) || navToggle.contains(e.target)) return;
            setNavOpen(false);
        });
        document.addEventListener('keydown', e => {
            if (e.key === 'Escape' && navLinks.classList.contains('open')) setNavOpen(false, true);
        });
        window.addEventListener('resize', () => {
            if (window.innerWidth > 900 && navLinks.classList.contains('open')) setNavOpen(false);
        });
    }

    // Convert flashed alerts into toasts (progressive enhancement)
    document.querySelectorAll('[data-flash]').forEach(el => {
        const t = el.dataset.flash || 'info';
        const msg = el.textContent.trim();
        if (msg) toast(msg, t);
        el.remove();
    });
});
// Versioned first-run guide. Copy is intentionally client-side so the CLI-hosted
// Web UI follows the browser language even when no server locale session exists.
(function () {
    'use strict';

    var root = document.getElementById('nolaGuide');
    if (!root) return;

    var STORAGE_KEY = 'nola.onboarding.web.v2';
    var locale = (window.NOLA_UI_LOCALE || (navigator.languages && navigator.languages[0]) || navigator.language || 'en')
        .toLowerCase();
    var language = locale.indexOf('zh') === 0 ? 'zh'
        : locale.indexOf('de') === 0 ? 'de'
        : locale.indexOf('es') === 0 ? 'es'
        : locale.indexOf('fr') === 0 ? 'fr'
        : locale.indexOf('ja') === 0 ? 'ja'
        : locale.indexOf('ru') === 0 ? 'ru'
        : 'en';

    var copies = {
        en: {
            brand: 'Context that improves with you', count: '{0} of 4', back: 'Back',
            next: 'Continue', start: 'Start a conversation',
            steps: [
                ['NOT JUST ANOTHER CHAT', 'Turn context into useful intelligence', 'Nola connects what you choose, shows which sources it used, and keeps durable memory editable.', [['Connect', 'Files, data sources, and tools'], ['Understand', 'Grounded answers with visible activity'], ['Improve', 'Reviewable memory and preferences']], 'Nothing is connected without your choice.'],
                ['CHOOSE THE RUNTIME', 'Keep execution understandable', 'The Web workspace talks to the CLI host running on your machine. Models can still be local or remote.', [['CLI host', 'Orchestrates tools and permissions'], ['Model', 'Uses the provider selected in Models'], ['Browser', 'Stays a transparent control surface']], 'Sensitive actions continue to ask for confirmation.'],
                ['START WITH REAL CONTEXT', 'Connect only what the task needs', 'Begin with a workspace or document. Add Web sources and automations when they become useful.', [['Workspace', 'Files used by the current project'], ['Documents', 'Attached directly to a message'], ['Web sources', 'Refreshable sources with visible history']], 'Access is requested at the moment of use, not during this guide.'],
                ['READY FOR A REAL TASK', 'Ask for an outcome, not a feature', 'Give Nola a concrete objective and inspect the sources, thinking, and tool activity as it works.', [['Review a project', 'Find risks and propose a plan'], ['Work with a document', 'Extract, compare, or summarize'], ['Research a topic', 'Search the Web and cite the results']], 'Open this guide again from the Guide button in the top navigation.']
            ]
        },
        zh: {
            brand: '与你一起进化的上下文', count: '第 {0} 步，共 4 步', back: '返回',
            next: '继续', start: '开始对话',
            steps: [
                ['不只是另一个聊天应用', '让上下文变成真正有用的智能', 'Nola 连接你选择的内容，展示回答使用了哪些来源，并让长期记忆始终可编辑。', [['连接', '文件、数据源与工具'], ['理解', '有依据、过程可见的回答'], ['进化', '可检查的记忆与偏好']], '未经你选择，不会连接任何内容。'],
                ['选择运行方式', '让执行位置始终清楚', 'Web 工作台连接运行在本机的 CLI Host；模型本身仍可来自本地或远程提供方。', [['CLI Host', '编排工具与权限'], ['模型', '使用“模型”页面中的当前配置'], ['浏览器', '只做透明的控制界面']], '敏感操作仍会逐项请求确认。'],
                ['从真实上下文开始', '只连接当前任务需要的内容', '先选择工作区或上传文档，需要时再添加网页来源和自动化。', [['工作区', '当前项目实际使用的文件'], ['文档', '作为附件挂在消息下方'], ['网页来源', '可刷新且有历史记录的来源']], '权限在真正使用时申请，不会在本引导里集中弹出。'],
                ['准备完成真实任务', '描述结果，而不是寻找功能', '给 Nola 一个明确目标，并在执行时检查来源、思考过程和工具活动。', [['审查项目', '发现风险并给出计划'], ['处理文档', '提取、比较或总结'], ['研究主题', '联网检索并给出引用']], '之后可随时点击顶部“使用引导”重新打开。']
            ]
        },
        de: {
            brand: 'Kontext, der mit dir besser wird', count: '{0} von 4', back: 'Zurück', next: 'Weiter', start: 'Unterhaltung starten',
            steps: [
                ['MEHR ALS EIN CHAT', 'Kontext wird zu nützlicher Intelligenz', 'Nola verbindet ausgewählte Inhalte, zeigt verwendete Quellen und hält Langzeitwissen bearbeitbar.', [['Verbinden', 'Dateien, Quellen und Werkzeuge'], ['Verstehen', 'Belegte Antworten mit sichtbarem Ablauf'], ['Verbessern', 'Prüfbare Erinnerungen und Vorlieben']], 'Ohne deine Auswahl wird nichts verbunden.'],
                ['AUSFÜHRUNG WÄHLEN', 'Der Ausführungsort bleibt verständlich', 'Der Web-Arbeitsbereich verbindet sich mit dem lokalen CLI-Host; Modelle können lokal oder entfernt sein.', [['CLI-Host', 'Koordiniert Werkzeuge und Rechte'], ['Modell', 'Nutzt die Auswahl unter Modelle'], ['Browser', 'Bleibt eine transparente Oberfläche']], 'Sensible Aktionen verlangen weiterhin eine Bestätigung.'],
                ['MIT ECHTEM KONTEXT STARTEN', 'Nur verbinden, was die Aufgabe braucht', 'Beginne mit Arbeitsbereich oder Dokument und füge Web-Quellen und Automationen später hinzu.', [['Arbeitsbereich', 'Dateien des aktuellen Projekts'], ['Dokumente', 'Direkt an Nachrichten angehängt'], ['Web-Quellen', 'Aktualisierbar mit sichtbarem Verlauf']], 'Zugriff wird erst bei der Nutzung angefragt.'],
                ['BEREIT FÜR EINE AUFGABE', 'Beschreibe das Ergebnis', 'Gib ein klares Ziel vor und prüfe Quellen, Denken und Werkzeugaktivität.', [['Projekt prüfen', 'Risiken finden und planen'], ['Dokument bearbeiten', 'Extrahieren, vergleichen, zusammenfassen'], ['Thema recherchieren', 'Websuche mit Quellen']], 'Die Einführung lässt sich oben über Hilfe erneut öffnen.']
            ]
        },
        es: {
            brand: 'Contexto que mejora contigo', count: '{0} de 4', back: 'Atrás', next: 'Continuar', start: 'Iniciar conversación',
            steps: [
                ['MÁS QUE OTRO CHAT', 'Convierte el contexto en inteligencia útil', 'Nola conecta lo que eliges, muestra las fuentes usadas y mantiene editable la memoria duradera.', [['Conectar', 'Archivos, fuentes y herramientas'], ['Comprender', 'Respuestas fundamentadas y actividad visible'], ['Mejorar', 'Memoria y preferencias revisables']], 'Nada se conecta sin tu elección.'],
                ['ELIGE LA EJECUCIÓN', 'Mantén claro dónde se ejecuta', 'El espacio Web se conecta al host CLI de tu equipo; el modelo puede ser local o remoto.', [['Host CLI', 'Coordina herramientas y permisos'], ['Modelo', 'Usa la selección de Modelos'], ['Navegador', 'Es una superficie de control transparente']], 'Las acciones sensibles siguen pidiendo confirmación.'],
                ['EMPIEZA CON CONTEXTO REAL', 'Conecta solo lo necesario', 'Empieza con un espacio de trabajo o documento y añade fuentes Web y automatizaciones después.', [['Espacio de trabajo', 'Archivos del proyecto actual'], ['Documentos', 'Adjuntos a un mensaje'], ['Fuentes Web', 'Actualizables y con historial']], 'El acceso se solicita al usarlo, no en esta guía.'],
                ['LISTO PARA UNA TAREA REAL', 'Pide un resultado', 'Da un objetivo concreto y revisa fuentes, razonamiento y herramientas durante la ejecución.', [['Revisar un proyecto', 'Detectar riesgos y planear'], ['Trabajar con un documento', 'Extraer, comparar o resumir'], ['Investigar un tema', 'Buscar en la Web y citar']], 'Puedes abrir de nuevo esta guía desde la navegación superior.']
            ]
        },
        fr: {
            brand: 'Un contexte qui progresse avec vous', count: '{0} sur 4', back: 'Retour', next: 'Continuer', start: 'Démarrer une conversation',
            steps: [
                ['PLUS QU’UN CHAT', 'Transformer le contexte en intelligence utile', 'Nola relie ce que vous choisissez, affiche les sources utilisées et garde la mémoire durable modifiable.', [['Connecter', 'Fichiers, sources et outils'], ['Comprendre', 'Réponses fondées et activité visible'], ['Progresser', 'Mémoire et préférences vérifiables']], 'Rien n’est connecté sans votre choix.'],
                ['CHOISIR L’EXÉCUTION', 'Toujours savoir où le travail s’effectue', 'L’espace Web se connecte à l’hôte CLI local ; le modèle peut rester local ou distant.', [['Hôte CLI', 'Orchestre outils et autorisations'], ['Modèle', 'Utilise le choix de la page Modèles'], ['Navigateur', 'Reste une surface de contrôle transparente']], 'Les actions sensibles demandent toujours confirmation.'],
                ['PARTIR DU VRAI CONTEXTE', 'Ne connecter que le nécessaire', 'Commencez par un espace de travail ou un document, puis ajoutez sources Web et automatisations.', [['Espace de travail', 'Fichiers du projet actuel'], ['Documents', 'Joints directement au message'], ['Sources Web', 'Actualisables avec historique']], 'L’accès est demandé au moment de l’utilisation.'],
                ['PRÊT POUR UNE VRAIE TÂCHE', 'Demandez un résultat', 'Donnez un objectif concret et inspectez sources, raisonnement et outils pendant l’exécution.', [['Auditer un projet', 'Trouver les risques et planifier'], ['Traiter un document', 'Extraire, comparer ou résumer'], ['Rechercher un sujet', 'Chercher sur le Web et citer']], 'Rouvrez ce guide depuis le bouton de la navigation supérieure.']
            ]
        },
        ja: {
            brand: '使うほど育つコンテキスト', count: '4 ステップ中 {0}', back: '戻る', next: '続ける', start: '会話を始める',
            steps: [
                ['単なるチャットではありません', 'コンテキストを役立つ知能へ', '選んだ情報を接続し、使った情報源を示し、長期記憶をいつでも編集できます。', [['接続', 'ファイル、情報源、ツール'], ['理解', '根拠と動作が見える回答'], ['改善', '確認できる記憶と好み']], '選択なしに接続されるものはありません。'],
                ['実行方法を選ぶ', 'どこで動くかを明確に', 'Web ワークスペースはこの端末の CLI Host に接続し、モデルはローカルまたはリモートを選べます。', [['CLI Host', 'ツールと権限を調整'], ['モデル', 'モデル画面の設定を使用'], ['ブラウザ', '透明な操作画面として動作']], '機密操作は引き続き個別に確認します。'],
                ['実際のコンテキストから開始', '必要なものだけ接続', 'まずワークスペースか文書を選び、必要になったら Web 情報源や自動化を追加します。', [['ワークスペース', '現在のプロジェクトのファイル'], ['文書', 'メッセージに直接添付'], ['Web 情報源', '更新履歴を確認可能']], 'アクセス権は利用時にだけ要求します。'],
                ['実際のタスクを開始', '機能ではなく成果を伝える', '具体的な目的を伝え、実行中の情報源、思考、ツール操作を確認してください。', [['プロジェクトを確認', 'リスクを見つけて計画'], ['文書を処理', '抽出、比較、要約'], ['テーマを調査', 'Web 検索と引用']], '上部ナビゲーションのガイドから再表示できます。']
            ]
        },
        ru: {
            brand: 'Контекст, который развивается вместе с вами', count: '{0} из 4', back: 'Назад', next: 'Продолжить', start: 'Начать диалог',
            steps: [
                ['БОЛЬШЕ, ЧЕМ ЧАТ', 'Превратите контекст в полезный интеллект', 'Nola подключает выбранное, показывает использованные источники и оставляет долгую память редактируемой.', [['Подключить', 'Файлы, источники и инструменты'], ['Понять', 'Обоснованные ответы и видимые действия'], ['Улучшать', 'Проверяемая память и предпочтения']], 'Без вашего выбора ничего не подключается.'],
                ['ВЫБЕРИТЕ ВЫПОЛНЕНИЕ', 'Всегда понятно, где идёт работа', 'Web-пространство подключается к локальному CLI Host, а модель может быть локальной или удалённой.', [['CLI Host', 'Управляет инструментами и правами'], ['Модель', 'Использует выбор на странице моделей'], ['Браузер', 'Остаётся прозрачной панелью управления']], 'Чувствительные действия по-прежнему требуют подтверждения.'],
                ['НАЧНИТЕ С РЕАЛЬНОГО КОНТЕКСТА', 'Подключайте только необходимое', 'Начните с рабочей папки или документа, затем добавляйте Web-источники и автоматизации.', [['Рабочая папка', 'Файлы текущего проекта'], ['Документы', 'Вложения к сообщению'], ['Web-источники', 'Обновление с историей']], 'Доступ запрашивается во время использования.'],
                ['ГОТОВО К РЕАЛЬНОЙ ЗАДАЧЕ', 'Опишите результат', 'Задайте конкретную цель и следите за источниками, рассуждением и инструментами.', [['Проверить проект', 'Найти риски и составить план'], ['Обработать документ', 'Извлечь, сравнить или суммировать'], ['Исследовать тему', 'Поиск в Web со ссылками']], 'Руководство можно снова открыть из верхней навигации.']
            ]
        }
    };

    var copy = copies[language] || copies.en;
    var step = 0;
    var brand = document.getElementById('nolaGuideBrand');
    var count = document.getElementById('nolaGuideCount');
    var progress = document.getElementById('nolaGuideProgress');
    var eyebrow = document.getElementById('nolaGuideEyebrow');
    var title = document.getElementById('nolaGuideTitle');
    var body = document.getElementById('nolaGuideBody');
    var cards = document.getElementById('nolaGuideCards');
    var note = document.getElementById('nolaGuideNote');
    var back = document.getElementById('nolaGuideBack');
    var next = document.getElementById('nolaGuideNext');
    var opener = document.getElementById('openNolaGuide');
    var dialog = root.querySelector('.nola-guide-dialog');
    var previousGuideFocus = null;
    var previousBodyOverflow = '';

    function render() {
        var data = copy.steps[step];
        brand.textContent = copy.brand;
        count.textContent = copy.count.replace('{0}', String(step + 1));
        progress.setAttribute('aria-valuenow', String(step + 1));
        progress.setAttribute('aria-valuetext', count.textContent);
        eyebrow.textContent = data[0];
        title.textContent = data[1];
        body.textContent = data[2];
        note.textContent = data[4];
        back.textContent = copy.back;
        back.hidden = step === 0;
        next.textContent = step === copy.steps.length - 1 ? copy.start : copy.next;
        progress.replaceChildren();
        copy.steps.forEach(function (_, index) {
            var segment = document.createElement('span');
            if (index <= step) segment.className = 'is-active';
            progress.appendChild(segment);
        });
        cards.replaceChildren();
        data[3].forEach(function (item, index) {
            var card = document.createElement('div');
            card.className = 'nola-guide-card';
            var badge = document.createElement('span');
            badge.className = 'nola-guide-card-index';
            badge.textContent = String(index + 1);
            var text = document.createElement('span');
            var strong = document.createElement('strong');
            strong.textContent = item[0];
            var small = document.createElement('small');
            small.textContent = item[1];
            text.append(strong, small);
            card.append(badge, text);
            cards.appendChild(card);
        });
        if (dialog) {
            dialog.classList.remove('is-step-entering');
            requestAnimationFrame(function () {
                dialog.classList.add('is-step-entering');
            });
        }
    }

    function openGuide(force) {
        if (!force && localStorage.getItem(STORAGE_KEY) === 'complete') return;
        step = 0;
        render();
        previousGuideFocus = document.activeElement;
        root.hidden = false;
        previousBodyOverflow = document.body.style.overflow;
        document.body.style.overflow = 'hidden';
        next.focus();
    }

    function closeGuide(completed) {
        if (completed) localStorage.setItem(STORAGE_KEY, 'complete');
        root.hidden = true;
        document.body.style.overflow = previousBodyOverflow;
        if (previousGuideFocus instanceof HTMLElement) previousGuideFocus.focus();
        else if (opener) opener.focus();
        previousGuideFocus = null;
    }

    back.addEventListener('click', function () {
        if (step > 0) step -= 1;
        render();
    });
    next.addEventListener('click', function () {
        if (step < copy.steps.length - 1) {
            step += 1;
            render();
        } else {
            closeGuide(true);
            if (location.pathname !== '/chat') location.assign('/chat');
        }
    });
    root.querySelectorAll('[data-guide-close]').forEach(function (button) {
        button.addEventListener('click', function () { closeGuide(true); });
    });
    if (opener) opener.addEventListener('click', function () { openGuide(true); });
    document.addEventListener('keydown', function (event) {
        if (!root.hidden && event.key === 'Escape') closeGuide(true);
        if (!root.hidden && event.key === 'Tab') {
            var controls = Array.from(root.querySelectorAll('button:not([disabled]):not([hidden]), a[href]'));
            var first = controls[0];
            var last = controls[controls.length - 1];
            if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
            else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
        }
    });

    openGuide(false);
})();
