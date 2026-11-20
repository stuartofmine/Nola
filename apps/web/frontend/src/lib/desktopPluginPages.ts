// SPDX-License-Identifier: Apache-2.0
import { readable } from 'svelte/store';
import { api, errorMessage } from './api';

export type DesktopPluginPage = {id: string; title: string; package: string};
export type DesktopPluginDocument = DesktopPluginPage & {html: string};
type Catalog = {pages: DesktopPluginPage[]; ready: boolean; error?: string};

export const desktopPluginsChanged = 'nola:desktop-plugins-changed';
export const desktopPluginPath = (id: string) => '/plugin-page?' + new URLSearchParams({id});
export const desktopPluginCatalog = readable<Catalog>({pages: [], ready: false}, set => {
    let controller: AbortController | undefined;
    async function refresh() {
        if (document.hidden) return;
        controller?.abort();
        const request = new AbortController();
        controller = request;
        try {
            const pages = await api<DesktopPluginPage[]>('/api/desktop-plugin-pages', {
                method: 'POST', body: '{}', signal: request.signal,
            });
            if (!Array.isArray(pages)) throw new Error('Invalid plugin page catalog');
            if (!request.signal.aborted) set({pages, ready: true});
        } catch (error) {
            if (!request.signal.aborted) set({pages: [], ready: false, error: errorMessage(error)});
        }
    }
    void refresh();
    window.addEventListener(desktopPluginsChanged, refresh);
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => {
        controller?.abort();
        window.removeEventListener(desktopPluginsChanged, refresh);
        window.removeEventListener('focus', refresh);
        document.removeEventListener('visibilitychange', refresh);
    };
});

// The outer frame's policy restricts child navigation; the package can only run inline
// scripts in its own opaque document. Neither document receives an app/native bridge.
export function pluginFrameDocument(html: string, title: string): string {
    const escapeAttribute = (text: string) => text.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;');
    const policy = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data:; base-uri 'none'; form-action 'none'; frame-src about:";
    return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policy}"><style>html,body,iframe{margin:0;width:100%;height:100%;border:0;display:block;color-scheme:light dark}</style></head><body><iframe title="${escapeAttribute(title)}" sandbox="allow-scripts" referrerpolicy="no-referrer" srcdoc="${escapeAttribute(html)}"></iframe></body></html>`;
}
