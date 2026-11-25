export function safeLoginNext(value: string | null): string {
    const raw = value ?? '';
    const next = raw.trim();
    if (!next.startsWith('/') || next.startsWith('//') || /[\\\u0000-\u001f\u007f]/.test(raw)) return '/dashboard';
    try {
        const url = new URL(next, 'https://nola.invalid');
        if (url.origin !== 'https://nola.invalid' || url.pathname.startsWith('//') || url.pathname.startsWith('/login') || url.pathname === '/api' || url.pathname.startsWith('/api/')) return '/dashboard';
        return url.pathname + url.search + url.hash;
    } catch { return '/dashboard'; }
}

export function isLoopbackHostname(hostname?: string): boolean {
    const resolved = (hostname ?? (typeof globalThis !== 'undefined' && 'location' in globalThis ? globalThis.location?.hostname : '') ?? '')
        .trim()
        .toLowerCase();
    if (!resolved) return true;
    if (resolved === 'localhost' || resolved === '127.0.0.1' || resolved === '::1' || resolved === '[::1]') return true;
    if (resolved.endsWith('.localhost')) return true;
    if (/^127\.(?:25[0-5]|2[0-4]\d|1?\d?\d)\.(?:25[0-5]|2[0-4]\d|1?\d?\d)\.(?:25[0-5]|2[0-4]\d|1?\d?\d)$/.test(resolved)) return true;
    return false;
}

export function validHostToken(value: string, options?: { hostname?: string }): boolean {
    const trimmed = value.trim();
    const length = trimmed.length;
    if (length > 256 || /[\s\u0000-\u001f\u007f]/.test(trimmed)) return false;
    const minLength = isLoopbackHostname(options?.hostname) ? 8 : 16;
    return length >= minLength;
}

export function loginLanguage(languages: readonly string[]): 'zh' | 'en' {
    for (const value of languages) {
        if (/^zh(?:-|$)/i.test(value)) return 'zh';
        if (/^en(?:-|$)/i.test(value)) return 'en';
    }
    return 'en';
}

export function loginErrorKind(code: string): 'invalid' | 'limited' | 'registration' | 'session' | 'handoff' | 'network' {
    if (['invalid_token', 'invalid_host_token'].includes(code)) return 'invalid';
    if (['rate_limited', 'too_many_login_attempts'].includes(code)) return 'limited';
    if (code === 'registration_unavailable') return 'registration';
    if (code === 'session_creation_failed') return 'session';
    if (['invalid_handoff', 'expired_handoff'].includes(code)) return 'handoff';
    return 'network';
}
