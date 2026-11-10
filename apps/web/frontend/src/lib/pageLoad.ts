export interface PageLoadFailure { route: string; kind: 'page-module-load-failed' | 'page-module-load-timeout'; resource: string | null }

// Keep diagnostics useful without logging arbitrary exception text, query
// parameters, user data, or responses. Only report bundled resource paths.
export function pageLoadFailure(route: string, cause: unknown): PageLoadFailure {
    let resource: string | null = null;
    if (cause instanceof Error) {
        const candidates = cause.message.match(/https?:\/\/[^\s"'<>]+/g) ?? [];
        for (const candidate of candidates) {
            try {
                const path = new URL(candidate).pathname;
                if (/^\/ui\/assets\/[A-Za-z0-9_.-]+\.(js|css)$/.test(path)) { resource = path; break; }
            } catch { /* An error string cannot influence navigation. */ }
        }
    }
    return { route, kind: cause instanceof PageLoadTimeout ? 'page-module-load-timeout' : 'page-module-load-failed', resource };
}
class PageLoadTimeout extends Error { constructor() { super('Page module loading timed out'); } }

export async function observedPageLoad<T>(route: string, loader: () => Promise<T>, report: (failure: PageLoadFailure) => void): Promise<T> {
    let timer: ReturnType<typeof setTimeout>;
    const deadline = new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new PageLoadTimeout()), 30_000); });
    try { return await Promise.race([loader(), deadline]); }
    catch (cause) {
        try { report(pageLoadFailure(route, cause)); } catch { /* Diagnostics never replace the original failure. */ }
        throw cause;
    } finally { clearTimeout(timer!); }
}
