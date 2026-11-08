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

export function validHostToken(value: string): boolean {
    const length = value.trim().length;
    return length >= 32 && length <= 256;
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
